use crate::loudness::{peak_and_rms, LoudnessMeter, LEVEL_FLOOR_DB};
use crate::mix::{Mixer, Resampler, CHANNELS};
use crate::timeline::PlaybackTimeline;
use crate::{ClockSource, Levels, PlaybackError};
use kerf_core::program::{ProgramSound, SAMPLE_RATE};
use kerf_model::Id;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
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
/// How long a device may go without asking for sound, while the program
/// plays, before the clock stops waiting for it. A device asks every few
/// milliseconds; one that has not for this long is not playing, whatever it
/// says, and the clock its frames advance would hold the picture and the
/// meter still with it.
const DEVICE_QUIET_AFTER: Duration = Duration::from_millis(400);
/// How long a device that went quiet has to take sound at its own rate
/// before it keeps the time again: long enough that one that only stutters
/// is left alone.
const DEVICE_STEADY_FOR: Duration = Duration::from_secs(1);
/// The share of its own rate a device has to take sound at to count as
/// steady.
const DEVICE_STEADY_SHARE: f64 = 0.9;

/// How often a playing output says what it is doing, with `KERF_PLAYBACK_STATS`
/// set.
const STATS_PERIOD: Duration = Duration::from_millis(250);

/// With `KERF_PLAYBACK_STATS` set, each output says on stderr which device it
/// opened and how long that took, what it was told to do, and four times a
/// second while playing where its clock is, what keeps it, and how often the
/// device has asked for sound.
fn stats_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KERF_PLAYBACK_STATS").is_some())
}

/// The time since this process first asked, which every stats line carries
/// so the lines of outputs open side by side can be put in order.
fn since_start() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// Which output a stats line is about: outputs are numbered as they open.
fn next_output_number() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

macro_rules! stats {
    ($number:expr, $($line:tt)*) => {
        if stats_on() {
            eprintln!(
                "playback audio[{}] +{:.3}s: {}",
                $number,
                since_start().as_secs_f64(),
                format_args!($($line)*)
            );
        }
    };
}

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
                let number = next_output_number();
                stats!(number, "opening (enabled {enabled})");
                let asked = Instant::now();
                let output = make_audio_output(worker_clock.clone(), enabled);
                stats!(
                    number,
                    "opened in {:.3}s: {}",
                    asked.elapsed().as_secs_f64(),
                    output.describe()
                );
                worker_clock.opened();
                let mut worker = AudioWorker::new(
                    timeline,
                    receiver,
                    worker_clock,
                    output,
                    worker_levels,
                    enabled,
                );
                worker.number = number;
                worker.run();
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
    /// How often the device has asked for sound, playing or not, and when it
    /// last did, in nanoseconds of [`since_start`] (0 before the first).
    device_calls: AtomicU64,
    last_device_call: AtomicU64,
    /// The frames the device has asked for, playing or not.
    device_frames: AtomicU64,
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
                device_calls: AtomicU64::new(0),
                last_device_call: AtomicU64::new(0),
                device_frames: AtomicU64::new(0),
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

    /// The device asked for `frames` of sound.
    #[cfg(feature = "native-audio")]
    fn record_device_call(&self, frames: u64) {
        self.inner.device_calls.fetch_add(1, Ordering::Release);
        self.inner
            .device_frames
            .fetch_add(frames, Ordering::Release);
        self.device_heard_from();
    }

    /// The device opened or asked for sound just now.
    #[cfg(feature = "native-audio")]
    fn device_heard_from(&self) {
        let now = since_start().as_nanos().max(1) as u64;
        self.inner.last_device_call.store(now, Ordering::Release);
    }

    /// How often the device has asked for sound, and how long ago it last
    /// did (or opened, before it has).
    fn device_calls(&self) -> (u64, Option<Duration>) {
        let calls = self.inner.device_calls.load(Ordering::Acquire);
        let last = self.inner.last_device_call.load(Ordering::Acquire);
        let ago = (last > 0).then(|| since_start().saturating_sub(Duration::from_nanos(last)));
        (calls, ago)
    }

    /// The frames the device has asked for, playing or not.
    fn device_frames(&self) -> u64 {
        self.inner.device_frames.load(Ordering::Acquire)
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
    /// Whether a device takes the sound, and its frames advance the clock.
    fn is_device(&self) -> bool {
        false
    }
    /// What the sound goes to, for the stats.
    fn describe(&self) -> String;
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
    /// The device, set aside while it is quiet: the clock runs without it
    /// and `output` only paces the mix, until the device takes sound
    /// steadily again.
    quiet_device: Option<Box<dyn AudioOutput>>,
    /// Since when the quiet device has been asking for sound again, and the
    /// frames it had asked for by then.
    asking_since: Option<(Instant, u64)>,
    /// Which output the stats lines name, and when the last one was written.
    number: u32,
    stats_written: Instant,
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
            quiet_device: None,
            asking_since: None,
            number: 0,
            stats_written: Instant::now(),
        }
    }

    /// A device that stops asking for sound, and says nothing of it, would
    /// stop the clock its frames advance, and with it the picture and the
    /// meter. Once it has been quiet for [`DEVICE_QUIET_AFTER`] while the
    /// program plays, the clock keeps monotonic time from where the device
    /// left it and the mix is paced by that clock; the device stays open,
    /// playing nothing, and keeps the time again once it has taken sound at
    /// its own rate for [`DEVICE_STEADY_FOR`].
    fn watch_device(&mut self) {
        let (_, ago) = self.clock.device_calls();
        let Some(ago) = ago else {
            return;
        };
        if self.quiet_device.is_none() {
            if self.playing
                && self.output.is_device()
                && self.clock.source() == ClockSource::AudioDevice
                && ago > DEVICE_QUIET_AFTER
            {
                self.set_device_aside(ago);
            }
            return;
        }
        let frames = self.clock.device_frames();
        match self.asking_since {
            _ if ago > DEVICE_QUIET_AFTER => self.asking_since = None,
            None => self.asking_since = Some((Instant::now(), frames)),
            Some((since, before)) if since.elapsed() >= DEVICE_STEADY_FOR => {
                let wanted = since.elapsed().as_secs_f64()
                    * f64::from(self.output.sample_rate())
                    * DEVICE_STEADY_SHARE;
                if frames.saturating_sub(before) as f64 >= wanted {
                    self.take_device_back();
                } else {
                    self.asking_since = Some((Instant::now(), frames));
                }
            }
            Some(_) => {}
        }
    }

    /// Carry on from where the quiet device left the clock, without it.
    fn set_device_aside(&mut self, quiet_for: Duration) {
        eprintln!(
            "kerf playback: the sound device has not asked for sound for {} ms; keeping time without it",
            quiet_for.as_millis()
        );
        let pacer = NullAudioOutput::new(
            self.output.sample_rate(),
            self.output.channels(),
            self.clock.clone(),
        )
        .because("the device went quiet".into());
        let device = std::mem::replace(&mut self.output, Box::new(pacer));
        self.clock.set_monotonic_clock();
        self.position = self.clock.now();
        self.stop_decoder();
        // What it did not take is not played late when it asks again.
        device.clear();
        self.quiet_device = Some(device);
        self.asking_since = None;
        stats!(self.number, "device set aside at {:.4}", self.position);
    }

    /// The device takes sound again: play through it from where the clock
    /// is, and let its frames advance the clock.
    fn take_device_back(&mut self) {
        let Some(device) = self.quiet_device.take() else {
            return;
        };
        eprintln!("kerf playback: the sound device asks for sound again; keeping time by it");
        self.asking_since = None;
        self.position = self.clock.now();
        self.stop_decoder();
        device.clear();
        device.set_volume(self.volume);
        device.set_muted(self.muted);
        self.output = device;
        #[cfg(feature = "native-audio")]
        self.clock.set_device_clock(self.output.sample_rate());
        stats!(self.number, "device taken back at {:.4}", self.position);
    }

    /// Where the clock is and what the device has done, a few times a second
    /// while playing.
    fn write_stats(&mut self) {
        if !stats_on() || self.stats_written.elapsed() < STATS_PERIOD {
            return;
        }
        self.stats_written = Instant::now();
        let (calls, ago) = self.clock.device_calls();
        stats!(
            self.number,
            "playing at {:.4} by {:?}, device calls {calls}, last {}, frames played {}, queued {}",
            self.clock.now(),
            self.clock.source(),
            ago.map_or("never".into(), |ago| format!(
                "{:.3}s ago",
                ago.as_secs_f64()
            )),
            self.clock.inner.played_frames.load(Ordering::Acquire),
            self.output.queued_frames(),
        );
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
            if self
                .quiet_device
                .as_ref()
                .is_some_and(|device| !device.is_alive())
            {
                // Gone while it was set aside: try the default device now.
                self.quiet_device = None;
                self.replace_lost_output();
            }
            self.watch_device();
            if self.playing {
                self.decode_until_buffered();
                self.write_stats();
                thread::sleep(AUDIO_TICK);
            }
        }
        self.stop_decoder();
        stats!(
            self.number,
            "closing, device calls {}",
            self.clock.device_calls().0
        );
    }

    /// The device stopped calling back, so its frames no longer advance the
    /// clock. Carry on from the same position on the default device now, or
    /// on the monotonic clock when there is none, so playback still reaches
    /// its end.
    fn replace_lost_output(&mut self) {
        stats!(
            self.number,
            "the device is gone, at {:.4}",
            self.clock.now()
        );
        self.clock.set_monotonic_clock();
        self.position = self.clock.now();
        self.stop_decoder();
        self.output = make_audio_output(self.clock.clone(), self.enabled);
        self.output.set_volume(self.volume);
        self.output.set_muted(self.muted);
        stats!(self.number, "carrying on with {}", self.output.describe());
    }

    fn handle_command(&mut self, command: AudioCommand) -> bool {
        match &command {
            AudioCommand::Open(_) => stats!(self.number, "open"),
            AudioCommand::Play(position) => stats!(self.number, "play from {position:.4}"),
            AudioCommand::Pause => stats!(self.number, "pause at {:.4}", self.clock.now()),
            AudioCommand::Seek(position) => stats!(self.number, "seek to {position:.4}"),
            AudioCommand::SetSolo(tracks) => stats!(self.number, "solo {}", tracks.len()),
            AudioCommand::SetVolume(_) | AudioCommand::SetMuted(_) | AudioCommand::Shutdown => {}
        }
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
    /// Why there is no device, for the stats.
    why: String,
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
            why: "none asked for".into(),
        }
    }

    fn because(mut self, why: String) -> Self {
        self.why = why;
        self
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

    fn describe(&self) -> String {
        format!("no device ({})", self.why)
    }
}

#[cfg(feature = "native-audio")]
struct CpalAudioOutput {
    buffer: Arc<AudioBuffer>,
    /// The device's name and the format it is given.
    opened: String,
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

    fn is_device(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        self.opened.clone()
    }
}

fn make_audio_output(_clock: AudioClock, _enabled: bool) -> Box<dyn AudioOutput> {
    let output = NullAudioOutput::new(48_000, 2, _clock.clone());
    #[cfg(feature = "native-audio")]
    if _enabled {
        return match CpalAudioOutput::try_new(_clock) {
            Ok(output) => Box::new(output),
            Err(why) => Box::new(output.because(why)),
        };
    }
    Box::new(output)
}

#[cfg(feature = "native-audio")]
impl CpalAudioOutput {
    /// The default device playing the queue, or why there is none.
    fn try_new(clock: AudioClock) -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        #[cfg(windows)]
        keep_com_loaded();
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no default output device")?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("no output format: {error}"))?;
        let sample_rate = supported.sample_rate().0.max(1);
        let channels = supported.channels().max(1) as usize;
        let config = supported.config();
        let opened = format!(
            "device {:?} of {:?}, {:?} {sample_rate} Hz {channels} ch, buffer {:?}",
            device.name().unwrap_or_default(),
            host.id(),
            supported.sample_format(),
            supported.buffer_size(),
        );
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
                .map_err(|error| format!("{opened} did not open: {error}"))?,
            cpal::SampleFormat::I16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .map_err(|error| format!("{opened} did not open: {error}"))?,
            cpal::SampleFormat::U16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .map_err(|error| format!("{opened} did not open: {error}"))?,
            format => return Err(format!("{opened}: no output in {format:?}")),
        };
        stream
            .play()
            .map_err(|error| format!("{opened} did not start: {error}"))?;
        clock.device_heard_from();
        clock.set_device_clock(sample_rate);
        Ok(Self {
            buffer,
            opened,
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
    clock.record_device_call((data.len() / channels.max(1)) as u64);
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

        fn describe(&self) -> String {
            "a device that went away".into()
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
}
