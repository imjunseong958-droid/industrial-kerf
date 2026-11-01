use crate::mix::{Mixer, Resampler};
use crate::timeline::PlaybackTimeline;
use crate::{ClockSource, PlaybackError};
#[cfg(feature = "native-audio")]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const AUDIO_COMMAND_QUEUE_CAPACITY: usize = 16;
const AUDIO_BUFFER_SECONDS: f64 = 0.35;
/// Room in the device's queue past the buffered time, for the block that
/// tops it up: a queue that overflowed would drop sound not yet played.
#[cfg(feature = "native-audio")]
const AUDIO_HEADROOM_SECONDS: f64 = 0.1;
const AUDIO_TICK: Duration = Duration::from_millis(4);
/// Frames mixed at a time, at the mix's rate.
const MIX_FRAMES: usize = 1024;

#[derive(Clone)]
pub(crate) struct AudioRuntime {
    commands: SyncSender<AudioCommand>,
    clock: AudioClock,
}

impl AudioRuntime {
    pub fn spawn(timeline: PlaybackTimeline, enabled: bool) -> Result<Self, PlaybackError> {
        let (commands, receiver) = mpsc::sync_channel(AUDIO_COMMAND_QUEUE_CAPACITY);
        let clock = AudioClock::new();
        let worker_clock = clock.clone();
        thread::Builder::new()
            .name("kerf-audio".into())
            .spawn(move || {
                let output = make_audio_output(worker_clock.clone(), enabled);
                AudioWorker {
                    timeline,
                    commands: receiver,
                    clock: worker_clock,
                    output,
                    mixer: None,
                    resampler: None,
                    playing: false,
                    position: 0.0,
                    volume: 1.0,
                    muted: false,
                    enabled,
                }
                .run();
            })
            .map_err(|error| {
                PlaybackError::Audio(format!("failed to start audio worker: {error}"))
            })?;
        Ok(Self { commands, clock })
    }

    pub fn open(&self, timeline: PlaybackTimeline) {
        self.clock.set_position(0.0);
        self.clock.set_playing(false);
        self.send(AudioCommand::Open(Box::new(timeline)));
    }

    pub fn play(&self, position: f64) {
        self.clock.play_at(position);
        self.send(AudioCommand::Play(position));
    }

    pub fn pause(&self) {
        self.clock.pause();
        self.send(AudioCommand::Pause);
    }

    pub fn seek(&self, position: f64) {
        self.clock.set_position(position);
        self.send(AudioCommand::Seek(position));
    }

    pub fn set_volume(&self, volume: f32) {
        self.send(AudioCommand::SetVolume(volume));
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(AudioCommand::SetMuted(muted));
    }

    pub fn shutdown(&self) {
        self.send(AudioCommand::Shutdown);
    }

    pub fn clock(&self, duration: f64) -> crate::PlaybackClock {
        crate::PlaybackClock {
            position: self.clock.now().clamp(0.0, duration.max(0.0)),
            duration,
            source: self.clock.source(),
        }
    }

    fn send(&self, command: AudioCommand) {
        let _ = self.commands.send(command);
    }
}

#[derive(Clone)]
pub(crate) struct AudioClock {
    inner: Arc<AudioClockState>,
}

struct AudioClockState {
    played_frames: AtomicU64,
    base_frame: AtomicU64,
    base_position: AtomicU64,
    sample_rate: AtomicU32,
    playing: AtomicBool,
    source: AtomicU8,
    anchor: Mutex<ClockAnchor>,
}

struct ClockAnchor {
    position: f64,
    started: Option<Instant>,
}

impl AudioClock {
    fn new() -> Self {
        Self {
            inner: Arc::new(AudioClockState {
                played_frames: AtomicU64::new(0),
                base_frame: AtomicU64::new(0),
                base_position: AtomicU64::new(0.0_f64.to_bits()),
                sample_rate: AtomicU32::new(48_000),
                playing: AtomicBool::new(false),
                source: AtomicU8::new(ClockSource::MonotonicFallback as u8),
                anchor: Mutex::new(ClockAnchor {
                    position: 0.0,
                    started: None,
                }),
            }),
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_device_clock(&self, sample_rate: u32) {
        let position = self.now();
        self.inner
            .sample_rate
            .store(sample_rate.max(1), Ordering::Release);
        self.inner.base_frame.store(
            self.inner.played_frames.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.inner
            .base_position
            .store(position.to_bits(), Ordering::Release);
        self.inner
            .source
            .store(ClockSource::AudioDevice as u8, Ordering::Release);
    }

    fn set_position(&self, position: f64) {
        let position = position.max(0.0);
        self.inner.base_frame.store(
            self.inner.played_frames.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.inner
            .base_position
            .store(position.to_bits(), Ordering::Release);
        let mut anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anchor.position = position;
        anchor.started = self
            .inner
            .playing
            .load(Ordering::Acquire)
            .then(Instant::now);
    }

    fn play_at(&self, position: f64) {
        self.set_position(position);
        self.inner.playing.store(true, Ordering::Release);
        let mut anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anchor.position = position.max(0.0);
        anchor.started = Some(Instant::now());
    }

    fn pause(&self) {
        let position = self.now();
        self.inner.playing.store(false, Ordering::Release);
        self.set_position(position);
    }

    fn set_playing(&self, playing: bool) {
        if playing {
            self.inner.playing.store(true, Ordering::Release);
            let mut anchor = self
                .inner
                .anchor
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            anchor.started = Some(Instant::now());
        } else {
            self.pause();
        }
    }

    fn is_playing(&self) -> bool {
        self.inner.playing.load(Ordering::Acquire)
    }

    #[cfg(feature = "native-audio")]
    fn record_frames(&self, frames: u64) {
        self.inner
            .played_frames
            .fetch_add(frames, Ordering::Release);
    }

    fn now(&self) -> f64 {
        if self.source() == ClockSource::AudioDevice {
            let played = self.inner.played_frames.load(Ordering::Acquire);
            let base = self.inner.base_frame.load(Ordering::Acquire);
            let position = f64::from_bits(self.inner.base_position.load(Ordering::Acquire));
            let rate = self.inner.sample_rate.load(Ordering::Acquire).max(1) as f64;
            return position + played.saturating_sub(base) as f64 / rate;
        }
        let anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match anchor.started {
            Some(started) if self.is_playing() => anchor.position + started.elapsed().as_secs_f64(),
            _ => anchor.position,
        }
    }

    fn source(&self) -> ClockSource {
        if self.inner.source.load(Ordering::Acquire) == ClockSource::AudioDevice as u8 {
            ClockSource::AudioDevice
        } else {
            ClockSource::MonotonicFallback
        }
    }
}

enum AudioCommand {
    Open(Box<PlaybackTimeline>),
    Play(f64),
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    Shutdown,
}

trait AudioOutput {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    fn push(&self, samples: &[f32]);
    fn queued_frames(&self) -> usize;
    fn clear(&self);
    fn set_volume(&self, volume: f32);
    fn set_muted(&self, muted: bool);
}

struct AudioWorker {
    timeline: PlaybackTimeline,
    commands: Receiver<AudioCommand>,
    clock: AudioClock,
    output: Box<dyn AudioOutput>,
    /// The mix from `position` on, while playing.
    mixer: Option<Mixer>,
    resampler: Option<Resampler>,
    playing: bool,
    position: f64,
    volume: f32,
    muted: bool,
    enabled: bool,
}

impl AudioWorker {
    fn run(mut self) {
        loop {
            let result = if self.playing {
                self.commands.try_recv().map_err(|error| match error {
                    TryRecvError::Empty => RecvTimeoutError::Timeout,
                    TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                })
            } else {
                self.commands.recv_timeout(Duration::from_millis(100))
            };
            match result {
                Ok(command) => {
                    if !self.handle_command(command) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.playing {
                self.decode_until_buffered();
                thread::sleep(AUDIO_TICK);
            }
        }
        self.stop_decoder();
    }

    fn handle_command(&mut self, command: AudioCommand) -> bool {
        match command {
            AudioCommand::Open(timeline) => {
                self.stop_decoder();
                self.timeline = *timeline;
                self.position = 0.0;
                self.playing = false;
                self.output.clear();
                self.clock.set_position(0.0);
                self.clock.set_playing(false);
            }
            AudioCommand::Play(position) => {
                self.stop_decoder();
                self.position = position;
                self.playing = true;
                self.output.clear();
                self.output.set_volume(self.volume);
                self.output.set_muted(self.muted);
                self.clock.play_at(position);
            }
            AudioCommand::Pause => {
                self.position = self.clock.now();
                self.playing = false;
                self.stop_decoder();
                self.output.clear();
                self.clock.pause();
            }
            AudioCommand::Seek(position) => {
                self.position = position;
                self.stop_decoder();
                self.output.clear();
                self.clock.set_position(position);
            }
            AudioCommand::SetVolume(volume) => {
                self.volume = volume;
                self.output.set_volume(volume);
            }
            AudioCommand::SetMuted(muted) => {
                self.muted = muted;
                self.output.set_muted(muted);
            }
            AudioCommand::Shutdown => return false,
        }
        true
    }

    fn decode_until_buffered(&mut self) {
        let target = (self.output.sample_rate() as f64 * AUDIO_BUFFER_SECONDS) as usize;
        if !self.enabled || self.output.queued_frames() >= target {
            return;
        }
        if self.mixer.is_none() {
            let Ok(resampler) = Resampler::new(self.output.sample_rate(), self.output.channels())
            else {
                return;
            };
            self.resampler = resampler;
            self.mixer = Some(Mixer::new(self.timeline.sounds(), self.position));
        }
        let Some(mixer) = self.mixer.as_mut() else {
            return;
        };
        while self.output.queued_frames() < target {
            let mixed = mixer.mix(MIX_FRAMES);
            let samples = match self.resampler.as_mut() {
                Some(resampler) => match resampler.convert(&mixed) {
                    Ok(samples) => samples,
                    Err(_) => return,
                },
                None => mixed,
            };
            if samples.is_empty() {
                return;
            }
            self.output.push(&samples);
        }
    }

    fn stop_decoder(&mut self) {
        self.mixer = None;
        self.resampler = None;
    }
}

#[cfg(feature = "native-audio")]
struct AudioBuffer {
    samples: Mutex<VecDeque<f32>>,
    max_samples: usize,
    volume: AtomicU32,
    muted: AtomicBool,
}

#[cfg(feature = "native-audio")]
impl AudioBuffer {
    fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            max_samples: ((sample_rate as f64 * (AUDIO_BUFFER_SECONDS + AUDIO_HEADROOM_SECONDS))
                as usize)
                .saturating_mul(channels.max(1)),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
        }
    }

    fn push(&self, samples: &[f32]) {
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if samples.len() >= self.max_samples {
            queue.clear();
            queue.extend(samples[samples.len() - self.max_samples..].iter().copied());
            return;
        }
        let available = self.max_samples.saturating_sub(samples.len());
        while queue.len() > available {
            queue.pop_front();
        }
        queue.extend(samples.iter().copied());
    }

    #[allow(dead_code)]
    fn pop(&self) -> Option<f32> {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }

    fn clear(&self) {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    fn queued_frames(&self, channels: usize) -> usize {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
            / channels.max(1)
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Acquire))
    }
}

struct NullAudioOutput {
    sample_rate: u32,
    channels: usize,
}

impl AudioOutput for NullAudioOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn push(&self, _samples: &[f32]) {}

    fn queued_frames(&self) -> usize {
        // There is no sink to fill. Reporting a full bounded queue keeps the
        // monotonic-clock fallback from decoding an entire long source into
        // nowhere while still allowing play/seek/mute controls to work.
        usize::MAX
    }

    fn clear(&self) {}

    fn set_volume(&self, _volume: f32) {}

    fn set_muted(&self, _muted: bool) {}
}

#[cfg(feature = "native-audio")]
struct CpalAudioOutput {
    buffer: Arc<AudioBuffer>,
    sample_rate: u32,
    channels: usize,
    _stream: cpal::Stream,
}

#[cfg(feature = "native-audio")]
impl AudioOutput for CpalAudioOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn push(&self, samples: &[f32]) {
        self.buffer.push(samples);
    }

    fn queued_frames(&self) -> usize {
        self.buffer.queued_frames(self.channels)
    }

    fn clear(&self) {
        self.buffer.clear();
    }

    fn set_volume(&self, volume: f32) {
        self.buffer
            .volume
            .store(volume.to_bits(), Ordering::Release);
    }

    fn set_muted(&self, muted: bool) {
        self.buffer.muted.store(muted, Ordering::Release);
    }
}

fn make_audio_output(_clock: AudioClock, _enabled: bool) -> Box<dyn AudioOutput> {
    #[cfg(feature = "native-audio")]
    if _enabled {
        if let Some(output) = CpalAudioOutput::try_new(_clock.clone()) {
            return Box::new(output);
        }
    }
    Box::new(NullAudioOutput {
        sample_rate: 48_000,
        channels: 2,
    })
}

#[cfg(feature = "native-audio")]
impl CpalAudioOutput {
    fn try_new(clock: AudioClock) -> Option<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        #[cfg(windows)]
        keep_com_loaded();
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let sample_rate = supported.sample_rate().0.max(1);
        let channels = supported.channels().max(1) as usize;
        let config = supported.config();
        let buffer = Arc::new(AudioBuffer::new(sample_rate, channels));
        let callback_buffer = Arc::clone(&buffer);
        let callback_clock = clock.clone();
        let error_callback = |error| eprintln!("kerf playback audio stream: {error}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [f32], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            cpal::SampleFormat::I16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            cpal::SampleFormat::U16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            _ => return None,
        };
        stream.play().ok()?;
        clock.set_device_clock(sample_rate);
        Some(Self {
            buffer,
            sample_rate,
            channels,
            _stream: stream,
        })
    }
}

/// cpal makes one device enumerator for the process, in the first thread
/// that asks, and each audio thread ends COM for itself as it exits. When
/// the last one did, Windows unloaded the audio DLLs under that enumerator
/// and the next session's audio thread read freed code. Holding the
/// multithreaded apartment open keeps COM, and those DLLs, loaded for the
/// life of the process.
#[cfg(all(windows, feature = "native-audio"))]
fn keep_com_loaded() {
    static KEPT: std::sync::Once = std::sync::Once::new();
    KEPT.call_once(|| {
        let mut cookie = std::ptr::null_mut();
        // SAFETY: writes the cookie, which is never handed back, so the
        // apartment stays open.
        unsafe { windows_sys::Win32::System::Com::CoIncrementMTAUsage(&mut cookie) };
    });
}

#[cfg(feature = "native-audio")]
fn write_output<T>(data: &mut [T], channels: usize, buffer: &AudioBuffer, clock: &AudioClock)
where
    T: cpal::Sample + cpal::FromSample<f32>,
{
    if !clock.is_playing() {
        for sample in data {
            *sample = T::from_sample(0.0);
        }
        return;
    }
    let muted = buffer.muted.load(Ordering::Acquire);
    let volume = buffer.volume();
    {
        let mut queue = buffer
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for sample in data.iter_mut() {
            let value = queue.pop_front().unwrap_or(0.0);
            *sample = T::from_sample(if muted { 0.0 } else { value * volume });
        }
    }
    clock.record_frames((data.len() / channels.max(1)) as u64);
}

#[cfg(test)]
mod tests {
    use super::AudioClock;
    use crate::ClockSource;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn monotonic_clock_advances_and_stops_at_pause() {
        let clock = AudioClock::new();
        clock.play_at(2.0);
        thread::sleep(Duration::from_millis(5));
        let playing_position = clock.now();
        assert!(playing_position >= 2.0);
        assert_eq!(clock.source(), ClockSource::MonotonicFallback);

        clock.pause();
        let paused_position = clock.now();
        thread::sleep(Duration::from_millis(5));
        assert!((clock.now() - paused_position).abs() < 0.01);
    }

    #[cfg(feature = "native-audio")]
    #[test]
    fn topping_up_the_queue_keeps_the_sound_not_played_yet() {
        use super::{AudioBuffer, AUDIO_BUFFER_SECONDS, MIX_FRAMES};
        let buffer = AudioBuffer::new(48_000, 2);
        let buffered = (48_000.0 * AUDIO_BUFFER_SECONDS) as usize;
        buffer.push(&vec![1.0; (buffered - 1) * 2]);
        buffer.push(&vec![2.0; MIX_FRAMES * 2]);
        assert_eq!(buffer.queued_frames(2), buffered - 1 + MIX_FRAMES);
        assert_eq!(buffer.pop(), Some(1.0));
    }
}
