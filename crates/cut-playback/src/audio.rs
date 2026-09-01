use crate::timeline::{PlaybackTimeline, ResolvedClip};
use crate::{ClockSource, PlaybackError};
#[cfg(feature = "native-audio")]
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const AUDIO_COMMAND_QUEUE_CAPACITY: usize = 16;
const AUDIO_BUFFER_SECONDS: f64 = 0.35;
const AUDIO_TICK: Duration = Duration::from_millis(4);

#[derive(Clone)]
pub(crate) struct AudioRuntime {
    commands: SyncSender<AudioCommand>,
    clock: AudioClock,
}

impl AudioRuntime {
    pub fn spawn(
        timeline: PlaybackTimeline,
        ffmpeg_binary: &Path,
        enabled: bool,
    ) -> Result<Self, PlaybackError> {
        let (commands, receiver) = mpsc::sync_channel(AUDIO_COMMAND_QUEUE_CAPACITY);
        let clock = AudioClock::new();
        let worker_clock = clock.clone();
        let binary = ffmpeg_binary.to_owned();
        thread::Builder::new()
            .name("cut-audio".into())
            .spawn(move || {
                let output = make_audio_output(worker_clock.clone(), enabled);
                AudioWorker {
                    timeline,
                    ffmpeg_binary: binary,
                    commands: receiver,
                    clock: worker_clock,
                    output,
                    decoder: None,
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
        self.send(AudioCommand::Open(timeline));
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
    Open(PlaybackTimeline),
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
    ffmpeg_binary: std::path::PathBuf,
    commands: Receiver<AudioCommand>,
    clock: AudioClock,
    output: Box<dyn AudioOutput>,
    decoder: Option<AudioDecoder>,
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
                self.timeline = timeline;
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
        if !self.enabled
            || self.output.queued_frames()
                >= (self.output.sample_rate() as f64 * AUDIO_BUFFER_SECONDS) as usize
        {
            return;
        }
        let channels = self.output.channels();
        let sample_rate = self.output.sample_rate();
        loop {
            let Some(clip) = self.timeline.audio_at(self.position).into_iter().next() else {
                let Some(next_start) = self.next_audio_start_after(self.position) else {
                    self.stop_decoder();
                    return;
                };
                self.position = next_start;
                continue;
            };
            let decoder_matches = self
                .decoder
                .as_ref()
                .is_some_and(|decoder| decoder.clip_id() == clip.clip.id);
            if !decoder_matches {
                self.stop_decoder();
                self.decoder = AudioDecoder::open(
                    &self.ffmpeg_binary,
                    &clip,
                    self.position,
                    sample_rate,
                    channels,
                )
                .ok();
                if self.decoder.is_none() {
                    self.position = clip.timeline_end();
                    continue;
                }
            }
            let Some(decoder) = self.decoder.as_mut() else {
                return;
            };
            match decoder.next_samples() {
                Ok(Some(samples)) if !samples.is_empty() => {
                    let frames = samples.len() / channels.max(1);
                    self.output.push(&samples);
                    self.position += frames as f64 / sample_rate as f64;
                    if self.position >= clip.timeline_end() {
                        self.position = clip.timeline_end();
                        self.stop_decoder();
                    }
                }
                Ok(Some(_)) => return,
                Ok(None) | Err(_) => {
                    self.stop_decoder();
                    self.position = clip.timeline_end();
                }
            }
            if self.output.queued_frames()
                >= (self.output.sample_rate() as f64 * AUDIO_BUFFER_SECONDS) as usize
            {
                return;
            }
        }
    }

    fn next_audio_start_after(&self, position: f64) -> Option<f64> {
        self.timeline
            .clips
            .iter()
            .filter(|clip| {
                matches!(
                    clip.track_kind,
                    cut_domain::TrackType::Audio | cut_domain::TrackType::Video
                ) && clip.timeline_start > position
            })
            .map(|clip| clip.timeline_start)
            .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn stop_decoder(&mut self) {
        if let Some(mut decoder) = self.decoder.take() {
            decoder.stop();
        }
    }
}

struct AudioDecoder {
    child: Child,
    stdout: ChildStdout,
    clip_id: String,
    pending: Vec<u8>,
}

impl AudioDecoder {
    fn open(
        ffmpeg_binary: &Path,
        clip: &ResolvedClip,
        timeline_time: f64,
        sample_rate: u32,
        channels: usize,
    ) -> Result<Self, PlaybackError> {
        let source_time = clip.source_time_at(timeline_time);
        let remaining = (clip.clip.source_out - source_time).max(0.0);
        if remaining <= 0.0 {
            return Err(PlaybackError::Audio(format!(
                "audio clip {} has no source time remaining",
                clip.clip.id
            )));
        }
        // As with the video decoder, normalize timestamps after an accurate
        // post-input seek before applying the duration trim.
        let mut filters = vec![
            "asetpts=PTS-STARTPTS".into(),
            format!("atrim=duration={}", format_number(remaining)),
        ];
        filters.extend(atempo_filters(clip.clip.speed));
        if (clip.clip.volume - 1.0).abs() > f32::EPSILON {
            filters.push(format!("volume={}", clip.clip.volume));
        }
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-nostdin".into(),
            "-ss".into(),
            format_number(source_time),
            "-i".into(),
            clip.source.path.to_string_lossy().into_owned(),
            "-vn".into(),
            "-ac".into(),
            channels.max(1).to_string(),
            "-ar".into(),
            sample_rate.max(1).to_string(),
        ];
        if !filters.is_empty() {
            args.extend(["-af".into(), filters.join(",")]);
        }
        args.extend([
            "-f".into(),
            "f32le".into(),
            "-acodec".into(),
            "pcm_f32le".into(),
            "pipe:1".into(),
        ]);
        let binary = ffmpeg_binary.to_string_lossy();
        let mut child = Command::new(binary.as_ref())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                PlaybackError::Audio(format!("failed to start {}: {}", binary, error))
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PlaybackError::Audio("audio decoder did not expose stdout".into()))?;
        Ok(Self {
            child,
            stdout,
            clip_id: clip.clip.id.clone(),
            pending: Vec::new(),
        })
    }

    fn clip_id(&self) -> &str {
        &self.clip_id
    }

    fn next_samples(&mut self) -> Result<Option<Vec<f32>>, PlaybackError> {
        let mut bytes = [0_u8; 16 * 1024];
        loop {
            let read = self
                .stdout
                .read(&mut bytes)
                .map_err(|error| PlaybackError::Audio(format!("reading audio failed: {error}")))?;
            if read == 0 {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let usable = self.pending.len() / 4 * 4;
                let samples = self.pending[..usable]
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
                self.pending.drain(..usable);
                return Ok(Some(samples));
            }
            self.pending.extend_from_slice(&bytes[..read]);
            let usable = self.pending.len() / 4 * 4;
            if usable == 0 {
                continue;
            }
            let samples = self.pending[..usable]
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            self.pending.drain(..usable);
            return Ok(Some(samples));
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn atempo_filters(speed: f64) -> Vec<String> {
    if !speed.is_finite() || speed <= 0.0 {
        return Vec::new();
    }
    let mut remaining = speed;
    let mut filters = Vec::new();
    while remaining > 100.0 {
        filters.push("atempo=100".into());
        remaining /= 100.0;
    }
    while remaining < 0.5 {
        filters.push("atempo=0.5".into());
        remaining /= 0.5;
    }
    if (remaining - 1.0).abs() > 1e-6 {
        filters.push(format!("atempo={}", format_number(remaining)));
    }
    filters
}

fn format_number(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    format!("{value:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
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
            max_samples: ((sample_rate as f64 * AUDIO_BUFFER_SECONDS) as usize)
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

        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let sample_rate = supported.sample_rate().0.max(1);
        let channels = supported.channels().max(1) as usize;
        let config = supported.config();
        let buffer = Arc::new(AudioBuffer::new(sample_rate, channels));
        let callback_buffer = Arc::clone(&buffer);
        let callback_clock = clock.clone();
        let error_callback = |error| eprintln!("cut playback audio stream: {error}");
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
    for sample in data.iter_mut() {
        let value = buffer.pop().unwrap_or(0.0);
        *sample = T::from_sample(if muted { 0.0 } else { value * volume });
    }
    clock.record_frames((data.len() / channels.max(1)) as u64);
}

#[cfg(test)]
mod tests {
    use super::{atempo_filters, AudioClock};
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

    #[test]
    fn atempo_chain_keeps_each_filter_inside_ffmpeg_limits() {
        assert_eq!(atempo_filters(2.0), vec!["atempo=2"]);
        assert_eq!(atempo_filters(0.25), vec!["atempo=0.5", "atempo=0.5"]);
        assert_eq!(atempo_filters(250.0), vec!["atempo=100", "atempo=2.5"]);
    }
}
