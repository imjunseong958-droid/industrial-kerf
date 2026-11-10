use crate::loudness::{peak_and_rms, LoudnessMeter, LEVEL_FLOOR_DB};
use crate::mix::{Mixer, Resampler, CHANNELS};
use crate::timeline::PlaybackTimeline;
use crate::{ClockSource, Levels, PlaybackError};
use kerf_core::program::{ProgramSound, SAMPLE_RATE};
use kerf_model::Id;
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
/// Frames mixed at a time, at the mix's rate. Each block is also one
/// reading of the program level.
const MIX_FRAMES: usize = 1024;
/// Level readings kept for the clock to reach: the buffered time's worth,
/// with room to spare.
const LEVEL_QUEUE_CAPACITY: usize = 64;
/// The longest the clock waits for the output device to open. On macOS the
/// first device a process opens takes longer the more files sit beside the
/// executable: seconds beside the hundreds of thousands in Cargo's `deps`, and
/// more on a busy machine. One that never opens must not hold playback up for
/// good, so the clock then runs without it.
const DEVICE_OPEN_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct AudioRuntime {
    commands: SyncSender<AudioCommand>,
    clock: AudioClock,
    levels: LevelTap,
}

impl AudioRuntime {
    pub fn spawn(timeline: PlaybackTimeline, enabled: bool) -> Result<Self, PlaybackError> {
        let (commands, receiver) = mpsc::sync_channel(AUDIO_COMMAND_QUEUE_CAPACITY);
        let clock = AudioClock::new();
        if enabled {
            clock.set_opening();
        }
        let levels = LevelTap::new(clock.clone());
        let worker_clock = clock.clone();
        let worker_levels = levels.clone();
        thread::Builder::new()
            .name("kerf-audio".into())
            .spawn(move || {
                let output = make_audio_output(worker_clock.clone(), enabled);
                worker_clock.opened();
                AudioWorker::new(
                    timeline,
                    receiver,
                    worker_clock,
                    output,
                    worker_levels,
                    enabled,
                )
                .run();
            })
            .map_err(|error| {
                PlaybackError::Audio(format!("failed to start audio worker: {error}"))
            })?;
        Ok(Self {
            commands,
            clock,
            levels,
        })
    }

    pub fn open(&self, timeline: PlaybackTimeline) {
        self.clock.set_position(0.0);
        self.clock.set_playing(false);
        self.levels.clear();
        self.send(AudioCommand::Open(Box::new(timeline)));
    }

    pub fn play(&self, position: f64) {
        self.clock.play_at(position);
        self.levels.clear();
        self.send(AudioCommand::Play(position));
    }

    pub fn pause(&self) {
        self.clock.pause();
        self.levels.clear();
        self.send(AudioCommand::Pause);
    }

    pub fn seek(&self, position: f64) {
        self.clock.set_position(position);
        self.levels.clear();
        self.send(AudioCommand::Seek(position));
    }

    /// Hear only these audio tracks of the program sequence; none hears
    /// them all.
    pub fn set_solo(&self, tracks: Vec<Id>) {
        self.send(AudioCommand::SetSolo(tracks));
    }

    /// The program level readings, as the sound reaches the output.
    pub fn levels(&self) -> LevelTap {
        self.levels.clone()
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
            dropped: 0,
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
    /// The output is still opening, so nothing can be heard or metered yet:
    /// time waits, and playing starts where it was asked to, with its sound.
    opening: AtomicBool,
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
                opening: AtomicBool::new(false),
                anchor: Mutex::new(ClockAnchor {
                    position: 0.0,
                    started: None,
                }),
            }),
        }
    }

    fn set_opening(&self) {
        self.inner.opening.store(true, Ordering::Release);
    }

    /// The output has opened, or there is none to open: time runs from now,
    /// not from when playing was asked for.
    fn opened(&self) {
        let mut anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.opening.store(false, Ordering::Release);
        if anchor.started.is_some() {
            anchor.started = Some(Instant::now());
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

    /// Keep time from the monotonic clock, from where the device left it,
    /// once the device that advanced it is gone.
    fn set_monotonic_clock(&self) {
        let position = self.now();
        self.set_position(position);
        self.inner
            .source
            .store(ClockSource::MonotonicFallback as u8, Ordering::Release);
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

    /// Where the clock was last started, moved or stopped.
    fn anchor_position(&self) -> f64 {
        self.inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .position
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
            Some(started) if self.is_playing() => {
                let mut elapsed = started.elapsed();
                if self.inner.opening.load(Ordering::Acquire) {
                    elapsed = elapsed.saturating_sub(DEVICE_OPEN_WAIT);
                }
                anchor.position + elapsed.as_secs_f64()
            }
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

/// The program level of each block mixed, held until the clock reaches it,
/// so a reading describes the sound being heard rather than the sound
/// buffered ahead of it.
#[derive(Clone)]
pub(crate) struct LevelTap {
    state: Arc<Mutex<TapState>>,
    clock: AudioClock,
}

#[derive(Default)]
struct TapState {
    /// Blocks mixed and not yet read, oldest first.
    blocks: VecDeque<Levels>,
    /// Blocks heard but not yet read, folded into one reading: a block that
    /// falls out of the queue unread is kept here, so its peak still reaches
    /// the meter.
    unread: Option<Folded>,
    /// What the last read returned, repeated until a newer block is heard.
    last: Option<Levels>,
}

/// Blocks folded into one reading: the per-channel peak of all of them,
/// the RMS as the mean power over them, and the rest from the newest.
#[derive(Clone, Copy)]
struct Folded {
    newest: Levels,
    peak: [f32; 2],
    power: [f64; 2],
    blocks: u32,
}

impl Folded {
    fn new(block: Levels) -> Self {
        Self {
            newest: block,
            peak: block.peak,
            power: block.rms.map(power_of_db),
            blocks: 1,
        }
    }

    fn add(&mut self, block: Levels) {
        for channel in 0..2 {
            self.peak[channel] = self.peak[channel].max(block.peak[channel]);
            self.power[channel] += power_of_db(block.rms[channel]);
        }
        self.blocks += 1;
        self.newest = block;
    }

    fn reading(&self) -> Levels {
        let blocks = f64::from(self.blocks);
        Levels {
            peak: self.peak,
            rms: self
                .power
                .map(|power| ((10.0 * (power / blocks).log10()) as f32).max(LEVEL_FLOOR_DB)),
            ..self.newest
        }
    }
}

fn power_of_db(db: f32) -> f64 {
    10f64.powf(f64::from(db) / 10.0)
}

fn fold(folded: &mut Option<Folded>, block: Levels) {
    match folded {
        Some(folded) => folded.add(block),
        None => *folded = Some(Folded::new(block)),
    }
}

impl LevelTap {
    fn new(clock: AudioClock) -> Self {
        Self {
            state: Arc::new(Mutex::new(TapState {
                blocks: VecDeque::with_capacity(LEVEL_QUEUE_CAPACITY),
                ..TapState::default()
            })),
            clock,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TapState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn push(&self, levels: Levels) {
        let mut state = self.lock();
        if state.blocks.len() >= LEVEL_QUEUE_CAPACITY {
            if let Some(evicted) = state.blocks.pop_front() {
                fold(&mut state.unread, evicted);
            }
        }
        state.blocks.push_back(levels);
    }

    fn clear(&self) {
        let mut state = self.lock();
        state.blocks.clear();
        state.unread = None;
        state.last = None;
    }

    /// The reading of every block heard since the last read: the highest
    /// sample peak among them, so a short transient is never skipped
    /// between polls, their mean RMS, and the newest one's loudness and
    /// time. The last reading is repeated while no newer block has been
    /// heard; `None` while nothing mixed has been heard yet (stopped, or
    /// just started or moved).
    pub fn latest(&self) -> Option<Levels> {
        let now = self.clock.now();
        let mut state = self.lock();
        let TapState {
            blocks,
            unread,
            last,
        } = &mut *state;
        while let Some(block) = blocks.front().copied().filter(|block| block.at <= now) {
            blocks.pop_front();
            fold(unread, block);
        }
        if let Some(folded) = unread.take() {
            *last = Some(folded.reading());
        }
        *last
    }
}

enum AudioCommand {
    Open(Box<PlaybackTimeline>),
    Play(f64),
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    SetSolo(Vec<Id>),
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
    /// False once the device went away and the output plays no more.
    fn is_alive(&self) -> bool {
        true
    }
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
    /// The audio tracks soloed, and the sound they mix (all of it when
    /// none is).
    solo: Vec<Id>,
    sounds: Arc<[ProgramSound]>,
    /// The program's level and loudness, measured as it is mixed.
    levels: LevelTap,
    loudness: Option<LoudnessMeter>,
}

impl AudioWorker {
    fn new(
        timeline: PlaybackTimeline,
        commands: Receiver<AudioCommand>,
        clock: AudioClock,
        output: Box<dyn AudioOutput>,
        levels: LevelTap,
        enabled: bool,
    ) -> Self {
        let sounds = timeline.sounds();
        Self {
            timeline,
            commands,
            clock,
            output,
            mixer: None,
            resampler: None,
            playing: false,
            position: 0.0,
            volume: 1.0,
            muted: false,
            enabled,
            solo: Vec::new(),
            sounds,
            levels,
            loudness: None,
        }
    }

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
            if !self.output.is_alive() {
                self.replace_lost_output();
            }
            if self.playing {
                self.decode_until_buffered();
                thread::sleep(AUDIO_TICK);
            }
        }
        self.stop_decoder();
    }

    /// The device stopped calling back, so its frames no longer advance the
    /// clock. Carry on from the same position on the default device now, or
    /// on the monotonic clock when there is none, so playback still reaches
    /// its end.
    fn replace_lost_output(&mut self) {
        self.clock.set_monotonic_clock();
        self.position = self.clock.now();
        self.stop_decoder();
        self.output = make_audio_output(self.clock.clone(), self.enabled);
        self.output.set_volume(self.volume);
        self.output.set_muted(self.muted);
    }

    fn handle_command(&mut self, command: AudioCommand) -> bool {
        match command {
            AudioCommand::Open(timeline) => {
                self.stop_decoder();
                self.timeline = *timeline;
                self.sounds = self.timeline.soloed_sounds(&self.solo);
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
            AudioCommand::SetSolo(tracks) => {
                self.solo = tracks;
                self.sounds = self.timeline.soloed_sounds(&self.solo);
                if self.playing {
                    // Remix from what is playing now, as a seek to here.
                    self.position = self.clock.now();
                    self.stop_decoder();
                    self.output.clear();
                    self.clock.set_position(self.position);
                }
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
            self.mixer = Some(Mixer::new(Arc::clone(&self.sounds), self.position));
            self.loudness = Some(LoudnessMeter::new(SAMPLE_RATE, CHANNELS));
            self.levels.clear();
        }
        let Some(mixer) = self.mixer.as_mut() else {
            return;
        };
        while self.output.queued_frames() < target {
            let mixed = mixer.mix(MIX_FRAMES);
            if let Some(loudness) = self.loudness.as_mut() {
                loudness.push(&mixed);
                let (peak, rms) = peak_and_rms(&mixed, CHANNELS);
                self.levels.push(Levels {
                    peak,
                    rms,
                    momentary: loudness.momentary(),
                    short_term: loudness.short_term(),
                    at: mixer.position(),
                });
            }
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
        self.loudness = None;
        self.levels.clear();
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

/// The output when there is no device: it takes the sound as the clock
/// passes it, so the program is still mixed (and metered) in real time,
/// level with the clock, and never decoded ahead of it into nowhere.
struct NullAudioOutput {
    sample_rate: u32,
    channels: usize,
    clock: AudioClock,
    pace: Mutex<NullPace>,
}

#[derive(Default)]
struct NullPace {
    /// Frames taken since the first push after a clear.
    pushed: u64,
    /// The program time of the first frame taken: where the clock was last
    /// started or moved, which is where the mix starts.
    origin: Option<f64>,
}

impl NullAudioOutput {
    fn new(sample_rate: u32, channels: usize, clock: AudioClock) -> Self {
        Self {
            sample_rate,
            channels,
            clock,
            pace: Mutex::new(NullPace::default()),
        }
    }

    fn pace(&self) -> std::sync::MutexGuard<'_, NullPace> {
        self.pace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl AudioOutput for NullAudioOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn push(&self, samples: &[f32]) {
        let mut pace = self.pace();
        if pace.origin.is_none() {
            pace.origin = Some(self.clock.anchor_position());
        }
        pace.pushed += (samples.len() / self.channels.max(1)) as u64;
    }

    /// The frames taken that the clock has not reached yet. Time spent
    /// before the first push (opening the mix) counts as played, as a device
    /// would have played silence, so the mix catches up with the clock
    /// rather than trailing it.
    fn queued_frames(&self) -> usize {
        let pace = self.pace();
        let Some(origin) = pace.origin else {
            return 0;
        };
        let rate = f64::from(self.sample_rate);
        let end = origin + pace.pushed as f64 / rate;
        ((end - self.clock.now()) * rate).max(0.0) as usize
    }

    fn clear(&self) {
        *self.pace() = NullPace::default();
    }

    fn set_volume(&self, _volume: f32) {}

    fn set_muted(&self, _muted: bool) {}
}

#[cfg(feature = "native-audio")]
struct CpalAudioOutput {
    buffer: Arc<AudioBuffer>,
    sample_rate: u32,
    channels: usize,
    /// Set when the stream reports its device gone.
    lost: Arc<AtomicBool>,
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

    fn is_alive(&self) -> bool {
        !self.lost.load(Ordering::Acquire)
    }
}

fn make_audio_output(_clock: AudioClock, _enabled: bool) -> Box<dyn AudioOutput> {
    #[cfg(feature = "native-audio")]
    if _enabled {
        if let Some(output) = CpalAudioOutput::try_new(_clock.clone()) {
            return Box::new(output);
        }
    }
    Box::new(NullAudioOutput::new(48_000, 2, _clock))
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
        let lost = Arc::new(AtomicBool::new(false));
        let callback_lost = Arc::clone(&lost);
        let error_callback = move |error| {
            eprintln!("kerf playback audio stream: {error}");
            if matches!(error, cpal::StreamError::DeviceNotAvailable) {
                callback_lost.store(true, Ordering::Release);
            }
        };
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
            lost,
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
    use std::time::{Duration, Instant};

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

    /// A clock playing from 0.5 since `waited` ago, its device still opening.
    fn opening_since(waited: Duration) -> AudioClock {
        let clock = AudioClock::new();
        clock.set_opening();
        clock.play_at(0.5);
        clock.inner.anchor.lock().expect("anchor").started = Instant::now().checked_sub(waited);
        clock
    }

    #[test]
    fn the_clock_waits_while_the_device_opens() {
        let clock = opening_since(Duration::from_secs(5));
        assert_eq!(clock.now(), 0.5, "nothing has been heard yet");

        // The wait is not played: time runs from when the device opened.
        let opened = Instant::now();
        clock.opened();
        let position = clock.now();
        assert!(position >= 0.5);
        assert!(
            position - 0.5 <= opened.elapsed().as_secs_f64(),
            "{position}"
        );
    }

    /// An output whose device went away.
    #[cfg(feature = "native-audio")]
    struct LostOutput;

    #[cfg(feature = "native-audio")]
    impl super::AudioOutput for LostOutput {
        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn channels(&self) -> usize {
            2
        }

        fn push(&self, _samples: &[f32]) {}

        fn queued_frames(&self) -> usize {
            usize::MAX
        }

        fn clear(&self) {}

        fn set_volume(&self, _volume: f32) {}

        fn set_muted(&self, _muted: bool) {}

        fn is_alive(&self) -> bool {
            false
        }
    }

    #[cfg(feature = "native-audio")]
    #[test]
    fn a_lost_device_hands_the_clock_back_to_monotonic_time() {
        use super::{AudioCommand, AudioWorker};
        use crate::PlaybackTimeline;
        use std::sync::mpsc;

        let clock = AudioClock::new();
        clock.set_device_clock(48_000);
        clock.play_at(1.0);
        // No callback advances the device's frames: the clock is stuck.
        thread::sleep(Duration::from_millis(5));
        assert_eq!(clock.now(), 1.0);

        let (commands, receiver) = mpsc::sync_channel(4);
        let timeline =
            PlaybackTimeline::from_project(&kerf_model::Project::default()).expect("timeline");
        let worker_clock = clock.clone();
        let levels = super::LevelTap::new(clock.clone());
        let thread = thread::spawn(move || {
            let mut worker = AudioWorker::new(
                timeline,
                receiver,
                worker_clock,
                Box::new(LostOutput),
                levels,
                false,
            );
            worker.playing = true;
            worker.position = 1.0;
            worker.run()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while clock.source() != ClockSource::MonotonicFallback && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(clock.source(), ClockSource::MonotonicFallback);
        let resumed = clock.now();
        assert!(resumed >= 1.0);
        thread::sleep(Duration::from_millis(10));
        assert!(clock.now() > resumed);
        commands
            .send(AudioCommand::Shutdown)
            .expect("worker running");
        thread.join().expect("worker exits");
    }

    #[test]
    fn a_reading_keeps_the_loudest_peak_heard_since_the_last_one() {
        use super::LevelTap;
        use crate::Levels;
        let clock = AudioClock::new();
        let tap = LevelTap::new(clock.clone());
        let level = |at: f64, peak: f32, rms: f32| Levels {
            peak: [peak, peak - 1.0],
            rms: [rms, rms],
            momentary: Some(-23.0),
            short_term: None,
            at,
        };
        clock.set_position(1.0);
        tap.push(level(0.96, -20.0, -30.0));
        tap.push(level(0.98, -0.5, -30.0));
        tap.push(level(1.0, -18.0, -30.0));
        tap.push(level(1.02, 0.0, -10.0));
        let reading = tap.latest().expect("heard");
        assert_eq!(
            reading.peak,
            [-0.5, -1.5],
            "the loud block is not the newest"
        );
        assert_eq!(reading.rms, [-30.0, -30.0]);
        assert_eq!(reading.at, 1.0);
        assert_eq!(reading.momentary, Some(-23.0));
        assert_eq!(tap.latest(), Some(reading), "nothing newer heard");

        clock.set_position(1.1);
        tap.push(level(1.04, -12.0, -10.0));
        let reading = tap.latest().expect("heard");
        assert_eq!(
            reading.peak,
            [0.0, -1.0],
            "an earlier reading's peak is not kept"
        );
        assert!((reading.rms[0] + 10.0).abs() < 1e-4, "{:?}", reading.rms);
        assert_eq!(reading.at, 1.04);

        // A block that falls out of a full queue unread still counts.
        tap.clear();
        clock.set_position(10.0);
        tap.push(level(5.0, -3.0, -40.0));
        for block in 1..super::LEVEL_QUEUE_CAPACITY {
            tap.push(level(5.0 + block as f64 * 0.02, -60.0, -40.0));
        }
        tap.push(level(9.0, -60.0, -40.0));
        assert_eq!(tap.latest().map(|levels| levels.peak[0]), Some(-3.0));
    }

    fn ffmpeg() -> Option<std::path::PathBuf> {
        let binary = std::env::var_os("KERF_FFMPEG")
            .map_or_else(|| "ffmpeg".into(), std::path::PathBuf::from);
        let output = std::process::Command::new(&binary)
            .arg("-version")
            .output()
            .ok()?;
        output.status.success().then_some(binary)
    }

    /// A 1 kHz tone at `dbfs` on both channels, four seconds long.
    fn tone(
        ffmpeg: &std::path::Path,
        directory: &std::path::Path,
        name: &str,
        dbfs: f64,
    ) -> String {
        let path = directory
            .join(name)
            .to_str()
            .expect("UTF-8 path")
            .to_owned();
        let amplitude = 10f64.powf(dbfs / 20.0);
        let source = format!(
            "aevalsrc={amplitude}*sin(2*PI*1000*t)|{amplitude}*sin(2*PI*1000*t):s=48000:d=4"
        );
        let output = std::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(&source)
            .args(["-c:a", "pcm_f32le", &path])
            .output()
            .expect("run ffmpeg");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    }

    /// The level a worker without a device reads `seconds` into playing
    /// `timeline` from the start, with `solo` soloed.
    fn level_after(
        timeline: crate::PlaybackTimeline,
        solo: &[&str],
        seconds: f64,
    ) -> Option<crate::Levels> {
        use super::{AudioCommand, AudioWorker, LevelTap, NullAudioOutput};
        use std::sync::mpsc;
        use std::time::Instant;

        let clock = AudioClock::new();
        let tap = LevelTap::new(clock.clone());
        let (commands, receiver) = mpsc::sync_channel(4);
        let (worker_clock, worker_tap) = (clock.clone(), tap.clone());
        let output = NullAudioOutput::new(48_000, 2, clock.clone());
        let thread = thread::spawn(move || {
            AudioWorker::new(
                timeline,
                receiver,
                worker_clock,
                Box::new(output),
                worker_tap,
                true,
            )
            .run()
        });
        let solo = solo
            .iter()
            .map(|id| kerf_model::Id::new(*id).expect("id"))
            .collect();
        commands.send(AudioCommand::SetSolo(solo)).expect("solo");
        clock.play_at(0.0);
        commands.send(AudioCommand::Play(0.0)).expect("play");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut levels = None;
        while Instant::now() < deadline {
            levels = tap.latest();
            if levels.is_some_and(|levels| levels.at >= seconds) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        commands.send(AudioCommand::Pause).expect("pause");
        thread::sleep(Duration::from_millis(50));
        assert_eq!(tap.latest(), None, "a pause clears the reading");
        commands.send(AudioCommand::Shutdown).expect("shutdown");
        thread.join().expect("worker exits");
        levels
    }

    #[test]
    fn the_program_level_reads_the_mix_and_follows_a_solo() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping the program level check: ffmpeg is unavailable");
            return;
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        let quiet = tone(&ffmpeg, directory.path(), "quiet.wav", -20.0);
        let loud = tone(&ffmpeg, directory.path(), "loud.wav", -6.0);
        let text = format!(
            "kerf 1\nproject name=\"Levels\"\n\
             media q path=\"{quiet}\" rate=30 dur=4s audio=2ch@48000\n\
             media l path=\"{loud}\" rate=30 dur=4s audio=2ch@48000\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=q src=0s at=0s dur=4s\n\
             \x20 track A2 audio\n\
             \x20   clip b1 media=l src=0s at=0s dur=4s\n\
             \x20     set gain=-60\n"
        );
        let project = kerf_lang::parse_document(&text, kerf_model::Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let timeline = crate::PlaybackTimeline::from_project(&project).expect("timeline");

        // Soloed, the −20 dBFS tone reads as itself.
        let levels = level_after(timeline.clone(), &["A1"], 0.5).expect("a reading");
        for side in 0..2 {
            assert!((levels.peak[side] + 20.0).abs() < 0.1, "{levels:?}");
            assert!((levels.rms[side] + 23.01).abs() < 0.1, "{levels:?}");
        }
        let momentary = levels.momentary.expect("400 ms mixed");
        assert!((momentary + 20.0).abs() < 0.3, "{levels:?}");
        assert_eq!(levels.short_term, None, "3 s not mixed yet");

        // Soloed, the loud tone, 60 dB down by its gain, is all there is.
        let levels = level_after(timeline, &["A2"], 0.2).expect("a reading");
        for side in 0..2 {
            assert!((levels.peak[side] + 66.0).abs() < 0.2, "{levels:?}");
        }
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
