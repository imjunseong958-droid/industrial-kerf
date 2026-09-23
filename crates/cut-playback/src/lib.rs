//! Native playback boundary for the Cut editor.
//!
//! The playback crate owns no project state. It receives a validated typed
//! project, maps timeline time to source time, and emits bounded frame/clock
//! events from a worker thread. Video and audio decode run in-process through
//! libav; export stays on the ffmpeg CLI in `cut-engine`. Native clients own
//! the adapter that presents delivered RGBA frames.

mod audio;
mod decoder;
mod libav;
mod timeline;

pub use timeline::{CaptionCue, PlaybackClip, PlaybackSource, PlaybackTimeline, ResolvedClip};

use audio::AudioRuntime;
use decoder::{DecodedVideoFrame, VideoDecoder};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Maximum number of decoded frames kept ahead of the UI.
pub const DEFAULT_DECODE_QUEUE_CAPACITY: usize = 4;
/// Maximum number of events waiting for the UI adapter.
pub const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 64;
const COMMAND_QUEUE_CAPACITY: usize = 32;
const CLOCK_EVENT_INTERVAL: Duration = Duration::from_millis(50);
const WORKER_TICK: Duration = Duration::from_millis(2);
/// Forward seeks inside this window keep the live decoder and drop frames.
const SMALL_FORWARD_SEEK_SECONDS: f64 = 1.0;

/// Playback failures are kept independent from the runtime protocol. The app
/// maps them to its UI/runtime error type at the adapter boundary.
#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error("invalid playback project: {0}")]
    InvalidProject(String),
    #[error("invalid playback value: {0}")]
    InvalidValue(String),
    #[error("playback command queue is full")]
    CommandQueueFull,
    #[error("playback worker is unavailable")]
    WorkerUnavailable,
    #[error("ffmpeg error: {0}")]
    Ffmpeg(String),
    #[error("audio output unavailable: {0}")]
    Audio(String),
}

/// The source used for the playback clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockSource {
    /// The platform audio callback advances the clock.
    AudioDevice,
    /// A monotonic fallback is used when no output device is available.
    MonotonicFallback,
}

/// Playback state visible to the editor adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Ready,
    Playing,
    Paused,
    Ended,
}

/// A clock update emitted while a session is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaybackClock {
    pub position: f64,
    pub duration: f64,
    pub source: ClockSource,
}

/// One decoded RGBA video frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackFrame {
    pub sequence: u64,
    pub timeline_time: f64,
    pub source_time: f64,
    pub clip_id: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub captions: Vec<CaptionCue>,
}

impl PlaybackFrame {
    /// Return the expected byte length for an RGBA frame.
    pub fn byte_len(&self) -> Option<usize> {
        usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
    }

    /// Whether the frame contains exactly one RGBA byte per channel per pixel.
    pub fn has_valid_pixels(&self) -> bool {
        self.byte_len() == Some(self.pixels.len())
    }
}

/// Events delivered to the UI adapter. Frame events may be dropped when the
/// bounded event queue is full; control events are retained in a small bounded
/// pending queue so a slow UI can never stall decoding or lose terminal state.
#[derive(Debug)]
pub enum PlaybackEvent {
    Opened {
        duration: f64,
        width: u32,
        height: u32,
    },
    StateChanged(PlaybackState),
    Clock(PlaybackClock),
    Frame(PlaybackFrame),
    Ended,
    Error(PlaybackError),
}

/// Commands accepted by the stable playback interface.
#[derive(Clone, Debug)]
pub enum PlaybackCommand {
    Open(Box<cut_domain::Project>),
    Play,
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    Shutdown,
}

#[derive(Debug)]
enum WorkerCommand {
    Open {
        project: Box<cut_domain::Project>,
        request_id: u64,
    },
    Play,
    Pause,
    Seek(SeekRequest),
    SeekWake,
    SetVolume(f32),
    SetMuted(bool),
    Shutdown,
}

#[derive(Debug)]
struct SeekRequest {
    position: f64,
    request_id: u64,
    /// False while the pointer is still moving. The worker then shows the
    /// nearest picture it can decode immediately and leaves the exact frame
    /// for the seek that follows once the pointer settles.
    exact: bool,
}

#[derive(Debug)]
struct ControlState {
    next_request_id: u64,
    visible_epoch: u64,
    latest_seek: Option<SeekRequest>,
    /// Bumped on every open and seek so an in-flight GOP decode can stop
    /// before the next pointer position arrives.
    epoch: Arc<AtomicU64>,
}

impl ControlState {
    fn new() -> Self {
        Self {
            // Epoch zero belongs to the initial project events. Start
            // requests at one so the first seek/open can invalidate them.
            next_request_id: 1,
            visible_epoch: 0,
            latest_seek: None,
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    fn allocate_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.epoch.store(request_id, Ordering::Release);
        request_id
    }
}

/// Cloneable control handle for a playback worker.
#[derive(Clone, Debug)]
pub struct PlaybackHandle {
    commands: SyncSender<WorkerCommand>,
    control: Arc<Mutex<ControlState>>,
}

impl PlaybackHandle {
    /// Replace the project without touching the editor's canonical runtime.
    pub fn open(&self, project: cut_domain::Project) -> Result<(), PlaybackError> {
        let mut control = self.lock_control();
        let request_id = control.allocate_request_id();
        let result = self
            .commands
            .try_send(WorkerCommand::Open {
                project: Box::new(project),
                request_id,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => PlaybackError::CommandQueueFull,
                TrySendError::Disconnected(_) => PlaybackError::WorkerUnavailable,
            });
        if result.is_ok() {
            // Publish the boundary when the command is accepted so events
            // from an earlier open cannot escape while this one is queued.
            control.visible_epoch = control.visible_epoch.max(request_id);
        }
        result
    }

    pub fn open_json(&self, project: &serde_json::Value) -> Result<(), PlaybackError> {
        let project = cut_domain::parse_project(project)
            .map_err(|error| PlaybackError::InvalidProject(error.to_string()))?;
        self.open(project)
    }

    pub fn play(&self) -> Result<(), PlaybackError> {
        self.send(PlaybackCommand::Play)
    }

    pub fn pause(&self) -> Result<(), PlaybackError> {
        self.send(PlaybackCommand::Pause)
    }

    pub fn seek(&self, position: f64) -> Result<(), PlaybackError> {
        self.enqueue_seek(position, true)
    }

    /// Show the nearest decodable picture for `position` without walking a
    /// long GOP. A later [`Self::seek`] fills in the exact frame.
    pub fn scrub(&self, position: f64) -> Result<(), PlaybackError> {
        self.enqueue_seek(position, false)
    }

    fn enqueue_seek(&self, position: f64, exact: bool) -> Result<(), PlaybackError> {
        if !position.is_finite() || position < 0.0 {
            return Err(PlaybackError::InvalidValue(
                "seek position must be finite and nonnegative".into(),
            ));
        }
        let mut control = self.lock_control();
        let request_id = control.allocate_request_id();
        let result = self.commands.try_send(WorkerCommand::SeekWake);
        match result {
            Ok(()) | Err(TrySendError::Full(_)) => {
                control.latest_seek = Some(SeekRequest {
                    position,
                    request_id,
                    exact,
                });
                control.visible_epoch = request_id;
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(PlaybackError::WorkerUnavailable),
        }
    }

    pub fn set_volume(&self, volume: f32) -> Result<(), PlaybackError> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(PlaybackError::InvalidValue(
                "volume must be finite and between 0 and 1".into(),
            ));
        }
        self.send(PlaybackCommand::SetVolume(volume))
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), PlaybackError> {
        self.send(PlaybackCommand::SetMuted(muted))
    }

    pub fn shutdown(&self) -> Result<(), PlaybackError> {
        self.send(PlaybackCommand::Shutdown)
    }

    fn send(&self, command: PlaybackCommand) -> Result<(), PlaybackError> {
        let command = match command {
            PlaybackCommand::Open(project) => {
                return self.open(*project);
            }
            PlaybackCommand::Play => WorkerCommand::Play,
            PlaybackCommand::Pause => WorkerCommand::Pause,
            PlaybackCommand::Seek(position) => {
                return self.seek(position);
            }
            PlaybackCommand::SetVolume(volume) => WorkerCommand::SetVolume(volume),
            PlaybackCommand::SetMuted(muted) => WorkerCommand::SetMuted(muted),
            PlaybackCommand::Shutdown => WorkerCommand::Shutdown,
        };
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => PlaybackError::CommandQueueFull,
                TrySendError::Disconnected(_) => PlaybackError::WorkerUnavailable,
            })
    }

    fn lock_control(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Stable controller/event boundary used by native clients. Implementations
/// must keep media work off the caller's UI thread.
pub trait PlaybackInterface {
    fn handle(&self) -> PlaybackHandle;
    fn try_next_event(&self) -> Option<PlaybackEvent>;
    fn next_event_timeout(&self, timeout: Duration) -> Option<PlaybackEvent>;
}

/// A worker-backed playback session.
pub struct PlaybackSession {
    handle: PlaybackHandle,
    events: Receiver<QueuedEvent>,
    control: Arc<Mutex<ControlState>>,
}

struct QueuedEvent {
    epoch: u64,
    event: PlaybackEvent,
}

impl PlaybackSession {
    /// Open a typed project and start its non-UI playback worker.
    pub fn open(
        project: cut_domain::Project,
        config: PlaybackConfig,
    ) -> Result<Self, PlaybackError> {
        let timeline = PlaybackTimeline::from_project(&project)?;
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let event_queue_capacity = config.event_queue_capacity.max(8);
        let (event_tx, event_rx) = mpsc::sync_channel::<QueuedEvent>(event_queue_capacity);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let epoch = Arc::clone(
            &control
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .epoch,
        );
        let audio = AudioRuntime::spawn(
            timeline.clone(),
            &config.ffmpeg_binary,
            config.audio_enabled,
        )?;
        let worker = Worker {
            timeline,
            config,
            commands: command_rx,
            events: event_tx,
            audio,
            state: PlaybackState::Ready,
            position: 0.0,
            decoder: None,
            decode_queue: VecDeque::new(),
            decode_cursor: 0.0,
            sequence: 0,
            preview_requested: true,
            last_clock_event: Instant::now(),
            volume: 1.0,
            muted: false,
            pending_events: VecDeque::new(),
            pending_event_capacity: event_queue_capacity,
            control: Arc::clone(&control),
            event_epoch: 0,
            deferred_command: None,
            preview_rough: false,
            needs_retarget: false,
            retarget_early: false,
            serve_epoch: 0,
            epoch,
        };
        thread::Builder::new()
            .name("cut-playback".into())
            .spawn(move || worker.run())
            .map_err(|error| PlaybackError::Ffmpeg(format!("failed to start worker: {error}")))?;
        Ok(Self {
            handle: PlaybackHandle {
                commands: command_tx,
                control: Arc::clone(&control),
            },
            events: event_rx,
            control,
        })
    }

    /// Parse a canonical v2 wire project at the app adapter boundary.
    pub fn open_json(
        project: &serde_json::Value,
        config: PlaybackConfig,
    ) -> Result<Self, PlaybackError> {
        let project = cut_domain::parse_project(project)
            .map_err(|error| PlaybackError::InvalidProject(error.to_string()))?;
        Self::open(project, config)
    }

    /// Open with the default decoder/audio configuration.
    pub fn open_default(project: cut_domain::Project) -> Result<Self, PlaybackError> {
        Self::open(project, PlaybackConfig::default())
    }

    pub fn handle(&self) -> PlaybackHandle {
        self.handle.clone()
    }

    pub fn play(&self) -> Result<(), PlaybackError> {
        self.handle.play()
    }

    pub fn pause(&self) -> Result<(), PlaybackError> {
        self.handle.pause()
    }

    pub fn seek(&self, position: f64) -> Result<(), PlaybackError> {
        self.handle.seek(position)
    }

    pub fn scrub(&self, position: f64) -> Result<(), PlaybackError> {
        self.handle.scrub(position)
    }

    pub fn set_volume(&self, volume: f32) -> Result<(), PlaybackError> {
        self.handle.set_volume(volume)
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), PlaybackError> {
        self.handle.set_muted(muted)
    }

    pub fn try_next_event(&self) -> Option<PlaybackEvent> {
        loop {
            let event = self.events.try_recv().ok()?;
            if event.epoch >= self.current_epoch() {
                return Some(event.event);
            }
        }
    }

    pub fn next_event_timeout(&self, timeout: Duration) -> Option<PlaybackEvent> {
        let started = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            let event = self.events.recv_timeout(remaining).ok()?;
            if event.epoch >= self.current_epoch() {
                return Some(event.event);
            }
        }
    }

    pub fn drain_events(&self) -> impl Iterator<Item = PlaybackEvent> + '_ {
        std::iter::from_fn(move || self.try_next_event())
    }

    fn current_epoch(&self) -> u64 {
        self.control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .visible_epoch
    }
}

impl PlaybackInterface for PlaybackSession {
    fn handle(&self) -> PlaybackHandle {
        self.handle()
    }

    fn try_next_event(&self) -> Option<PlaybackEvent> {
        self.try_next_event()
    }

    fn next_event_timeout(&self, timeout: Duration) -> Option<PlaybackEvent> {
        self.next_event_timeout(timeout)
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}

/// Configuration for the in-process libav decoder and bounded worker queues.
/// `ffmpeg_binary` is unused for decode; tests still pass it when they spawn
/// ffmpeg only to generate fixtures.
#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    pub ffmpeg_binary: PathBuf,
    pub decode_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub frame_width: Option<u32>,
    pub frame_height: Option<u32>,
    pub audio_enabled: bool,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            ffmpeg_binary: resolve_ffmpeg_binary(),
            decode_queue_capacity: DEFAULT_DECODE_QUEUE_CAPACITY,
            event_queue_capacity: DEFAULT_EVENT_QUEUE_CAPACITY,
            frame_width: None,
            frame_height: None,
            audio_enabled: true,
        }
    }
}

impl PlaybackConfig {
    /// Bound the number of decoded video frames kept ahead of the UI.
    pub fn with_decode_queue_capacity(mut self, capacity: usize) -> Self {
        self.decode_queue_capacity = capacity.max(1);
        self
    }

    /// Bound the number of events waiting for the UI adapter.
    pub fn with_event_queue_capacity(mut self, capacity: usize) -> Self {
        self.event_queue_capacity = capacity.max(8);
        self
    }

    pub fn with_ffmpeg_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.ffmpeg_binary = binary.into();
        self
    }

    pub fn with_audio_enabled(mut self, enabled: bool) -> Self {
        self.audio_enabled = enabled;
        self
    }

    pub fn with_frame_size(mut self, width: u32, height: u32) -> Self {
        self.frame_width = Some(width.max(2));
        self.frame_height = Some(height.max(2));
        self
    }
}

struct Worker {
    timeline: PlaybackTimeline,
    config: PlaybackConfig,
    commands: Receiver<WorkerCommand>,
    events: SyncSender<QueuedEvent>,
    audio: AudioRuntime,
    state: PlaybackState,
    position: f64,
    decoder: Option<VideoDecoder>,
    decode_queue: VecDeque<PlaybackFrame>,
    decode_cursor: f64,
    sequence: u64,
    preview_requested: bool,
    last_clock_event: Instant,
    volume: f32,
    muted: bool,
    pending_events: VecDeque<QueuedEvent>,
    pending_event_capacity: usize,
    control: Arc<Mutex<ControlState>>,
    event_epoch: u64,
    deferred_command: Option<WorkerCommand>,
    preview_rough: bool,
    needs_retarget: bool,
    retarget_early: bool,
    serve_epoch: u64,
    epoch: Arc<AtomicU64>,
}

impl Worker {
    fn run(mut self) {
        self.emit(PlaybackEvent::Opened {
            duration: self.timeline.duration,
            width: self.timeline.width,
            height: self.timeline.height,
        });
        self.emit(PlaybackEvent::StateChanged(self.state));
        self.emit_clock(true);

        loop {
            self.flush_pending_events();
            let timeout = if self.state == PlaybackState::Playing || self.preview_requested {
                WORKER_TICK
            } else {
                Duration::from_millis(100)
            };
            let command = self.next_command(timeout);
            if let Some(command) = command {
                if !self.handle_command(command) {
                    break;
                }
            }
            if self.pending_events.is_empty() {
                if self.state == PlaybackState::Playing {
                    self.tick_playing();
                } else if self.preview_requested {
                    self.tick_preview();
                }
            }
            if self.last_clock_event.elapsed() >= CLOCK_EVENT_INTERVAL {
                self.emit_clock(false);
            }
            self.flush_pending_events();
        }
        self.stop_decoder();
        self.audio.shutdown();
    }

    fn next_command(&mut self, timeout: Duration) -> Option<WorkerCommand> {
        if let Some(command) = self.deferred_command.take() {
            return Some(command);
        }
        let command = match self.commands.try_recv() {
            Ok(command) => Some(command),
            Err(TryRecvError::Disconnected) => return None,
            Err(TryRecvError::Empty) => {
                self.take_seek_request()
                    .map(WorkerCommand::Seek)
                    .or_else(|| match self.commands.recv_timeout(timeout) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Disconnected) => None,
                        Err(RecvTimeoutError::Timeout) => None,
                    })
            }
        }?;
        if let WorkerCommand::Open { request_id, .. } = &command {
            if let Some(request) = self.take_seek_request_before(*request_id) {
                self.deferred_command = Some(command);
                return Some(WorkerCommand::Seek(request));
            }
        }
        Some(command)
    }

    fn take_seek_request(&mut self) -> Option<SeekRequest> {
        self.control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .latest_seek
            .take()
    }

    fn take_seek_request_before(&mut self, request_id: u64) -> Option<SeekRequest> {
        let mut control = self
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if control
            .latest_seek
            .as_ref()
            .is_some_and(|request| request.request_id < request_id)
        {
            control.latest_seek.take()
        } else {
            None
        }
    }

    fn accept_open(&mut self, request_id: u64) {
        self.event_epoch = request_id;
        self.pending_events.clear();
        let mut control = self
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        control.visible_epoch = control.visible_epoch.max(request_id);
    }

    fn handle_command(&mut self, command: WorkerCommand) -> bool {
        match command {
            WorkerCommand::Open {
                project,
                request_id,
            } => match PlaybackTimeline::from_project(&project) {
                Ok(timeline) => {
                    self.accept_open(request_id);
                    self.stop_decoder();
                    self.timeline = timeline.clone();
                    self.audio.open(timeline);
                    self.position = 0.0;
                    self.decode_cursor = 0.0;
                    self.decode_queue.clear();
                    self.preview_requested = true;
                    self.set_state(PlaybackState::Ready);
                    self.emit(PlaybackEvent::Opened {
                        duration: self.timeline.duration,
                        width: self.timeline.width,
                        height: self.timeline.height,
                    });
                    self.emit_clock(true);
                }
                Err(error) => {
                    self.accept_open(request_id);
                    self.emit(PlaybackEvent::Error(error));
                }
            },
            WorkerCommand::Play => self.start_playing(),
            WorkerCommand::Pause => self.pause_playing(),
            WorkerCommand::SeekWake => {
                if let Some(request) = self.take_seek_request() {
                    self.seek_to(request.position, request.request_id, request.exact);
                }
            }
            WorkerCommand::Seek(request) => {
                self.seek_to(request.position, request.request_id, request.exact)
            }
            WorkerCommand::SetVolume(volume) => {
                self.volume = volume;
                self.audio.set_volume(volume);
            }
            WorkerCommand::SetMuted(muted) => {
                self.muted = muted;
                self.audio.set_muted(muted);
            }
            WorkerCommand::Shutdown => return false,
        }
        true
    }

    fn start_playing(&mut self) {
        if self.timeline.duration <= 0.0 {
            return;
        }
        if self.position >= self.timeline.duration {
            self.position = 0.0;
        }
        self.audio.play(self.position);
        self.decode_cursor = self.position;
        self.decode_queue.clear();
        self.stop_decoder();
        self.preview_requested = false;
        self.set_state(PlaybackState::Playing);
        self.emit_clock(true);
    }

    fn pause_playing(&mut self) {
        if self.state == PlaybackState::Playing {
            self.position = self.clock().position;
            self.audio.pause();
            self.set_state(PlaybackState::Paused);
            self.preview_requested = false;
            self.emit_clock(true);
        }
    }

    fn seek_to(&mut self, position: f64, request_id: u64, exact: bool) {
        // A newer pointer position already replaced this one.
        if self.seek_waiting() {
            return;
        }
        self.event_epoch = request_id;
        self.serve_epoch = request_id;
        self.pending_events.clear();
        let next = position.min(self.timeline.duration.max(0.0));
        let was_playing = self.state == PlaybackState::Playing;
        self.audio.seek(next);
        self.position = next;
        let scrub = !exact && !was_playing;
        self.preview_rough = scrub;
        if scrub {
            self.decode_queue.clear();
            self.decode_cursor = next;
            self.needs_retarget = true;
            self.retarget_early = true;
            self.preview_requested = true;
        } else if self.keep_decoder_for_seek(next) {
            self.needs_retarget = false;
            self.retarget_early = false;
            self.preview_requested = !was_playing;
        } else {
            self.decode_queue.clear();
            self.decode_cursor = next;
            self.needs_retarget = true;
            self.retarget_early = false;
            self.preview_requested = !was_playing;
        }
        if was_playing {
            self.audio.play(next);
            self.preview_requested = false;
            self.preview_rough = false;
        }
        self.emit_clock(true);
        // A seek advances the visible event epoch. Repeat the current state
        // so a state transition queued before the seek cannot be filtered
        // away without leaving clients with an out-of-date state.
        self.emit(PlaybackEvent::StateChanged(self.state));
    }

    fn seek_waiting(&self) -> bool {
        self.control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .latest_seek
            .is_some()
    }

    fn keep_decoder_for_seek(&mut self, target: f64) -> bool {
        if self
            .decoder
            .as_ref()
            .is_some_and(|decoder| decoder.is_preview_graph())
        {
            return false;
        }
        let Some((clip_id, clip_end)) = self
            .timeline
            .video_at_ref(target)
            .map(|(clip, _)| (clip.id.clone(), clip.timeline_end()))
        else {
            return false;
        };
        if !self
            .decoder
            .as_ref()
            .is_some_and(|decoder| decoder.clip_id() == clip_id.as_str())
        {
            return false;
        }
        let interval = frame_interval(self.timeline.fps);
        while self
            .decode_queue
            .front()
            .is_some_and(|frame| frame.timeline_time < target - interval * 0.5)
        {
            self.decode_queue.pop_front();
        }
        if self
            .decode_queue
            .front()
            .is_some_and(|frame| frame.timeline_time > target + interval * 0.75)
        {
            return false;
        }
        if self
            .decode_queue
            .front()
            .is_some_and(|frame| (frame.timeline_time - target).abs() <= interval * 0.75)
        {
            return true;
        }
        if !can_reuse_ffmpeg_decoder(
            clip_id.as_str(),
            self.decode_cursor,
            target,
            clip_id.as_str(),
            clip_end,
            self.timeline.fps,
        ) {
            return false;
        }
        while self.decode_cursor < target - interval * 0.5 {
            if self.decoder.as_ref().is_some_and(|decoder| decoder.is_cancelled()) {
                return false;
            }
            match self
                .decoder
                .as_mut()
                .and_then(|decoder| decoder.skip_frame().ok())
            {
                Some(true) => self.decode_cursor += interval,
                None => return false,
                Some(false) => return false,
            }
        }
        true
    }

    fn tick_playing(&mut self) {
        if self.needs_retarget {
            let early = self.retarget_early;
            self.needs_retarget = false;
            if let Err(error) = self.install_decoder(early) {
                self.emit(PlaybackEvent::Error(error));
            }
        }
        let position = self.clock().position.min(self.timeline.duration.max(0.0));
        self.position = position;
        if position >= self.timeline.duration && self.timeline.duration > 0.0 {
            self.audio.pause();
            self.set_state(PlaybackState::Ended);
            self.stop_decoder();
            self.decode_queue.clear();
            self.emit(PlaybackEvent::Ended);
            self.emit_clock(true);
            return;
        }

        self.fill_decode_queue();
        while self.decode_queue.front().is_some_and(|frame| {
            frame.timeline_time <= position + frame_interval(self.timeline.fps) * 0.5
        }) {
            if let Some(frame) = self.decode_queue.pop_front() {
                self.emit(PlaybackEvent::Frame(frame));
            }
        }
    }

    fn tick_preview(&mut self) {
        if self.seek_waiting() {
            self.preview_requested = false;
            return;
        }
        if self.needs_retarget {
            let early = self.retarget_early;
            self.needs_retarget = false;
            if let Err(error) = self.install_decoder(early) {
                self.emit(PlaybackEvent::Error(error));
                self.preview_requested = false;
                self.preview_rough = false;
                return;
            }
        }
        if self.preview_rough {
            self.preview_rough = false;
            self.emit_rough_frame();
            self.preview_requested = false;
            return;
        }
        if self.decode_queue.is_empty() {
            self.fill_decode_queue_limited(1);
        }
        if let Some(frame) = self.decode_queue.pop_front() {
            self.emit(PlaybackEvent::Frame(frame));
            self.preview_requested = false;
        } else if self
            .decoder
            .as_ref()
            .is_some_and(|decoder| decoder.is_cancelled())
        {
            self.preview_requested = false;
        } else if self.timeline.video_at_ref(self.position).is_none() {
            self.stop_decoder();
            self.preview_requested = false;
        } else if self.decoder.is_none() {
            self.preview_requested = false;
        }
    }

    fn emit_rough_frame(&mut self) {
        let decoded = match self.decoder.as_mut() {
            Some(decoder) => decoder.next_frame(),
            None => return,
        };
        match decoded {
            Ok(Some(decoded)) => {
                let Some((clip_index, _)) = self.timeline.video_at_indices(self.position) else {
                    return;
                };
                let timeline_time = decoded
                    .source_time
                    .map(|source_time| {
                        self.timeline.clips()[clip_index].timeline_time_for_source(source_time)
                    })
                    .unwrap_or(self.position);
                let frame = self.make_frame(clip_index, decoded, timeline_time);
                self.emit(PlaybackEvent::Frame(frame));
            }
            Ok(None) => {}
            Err(error) => self.emit(PlaybackEvent::Error(error)),
        }
    }

    fn install_decoder(&mut self, release_early: bool) -> Result<(), PlaybackError> {
        let Some((clip_index, source_index)) = self.timeline.video_at_indices(self.position) else {
            self.stop_decoder();
            return Ok(());
        };
        let clip = self.timeline.clips()[clip_index].clone();
        let source = self.timeline.sources()[source_index].clone();
        let serve = self.serve_epoch;
        let epoch = Arc::clone(&self.epoch);
        let timeline_time = self.position;
        if let Some(mut decoder) = self.decoder.take() {
            if decoder.same_source(&source.path)
                && decoder
                    .retarget(
                        &self.timeline,
                        &clip,
                        &source,
                        timeline_time,
                        release_early,
                        serve,
                    )
                    .is_ok()
            {
                self.decoder = Some(decoder);
                return Ok(());
            }
        }
        self.decoder = Some(VideoDecoder::open(
            &self.config.ffmpeg_binary,
            &self.timeline,
            &clip,
            &source,
            timeline_time,
            self.config.frame_width,
            self.config.frame_height,
            release_early,
            epoch,
            serve,
        )?);
        Ok(())
    }

    fn fill_decode_queue(&mut self) {
        self.fill_decode_queue_limited(self.config.decode_queue_capacity.max(1));
    }

    fn fill_decode_queue_limited(&mut self, capacity: usize) {
        let capacity = capacity.max(1);
        while self.decode_queue.len() < capacity {
            let Some((clip_index, source_index)) =
                self.timeline.video_at_indices(self.decode_cursor)
            else {
                let Some(next_start) = self.timeline.next_video_start_after(self.decode_cursor)
                else {
                    self.stop_decoder();
                    break;
                };
                self.decode_cursor = next_start;
                continue;
            };
            let clip_end = self.timeline.clips()[clip_index].timeline_end();
            if self.decode_cursor >= clip_end {
                self.decode_cursor = clip_end;
                self.stop_decoder();
                continue;
            }
            let decoder_matches = self.decoder.as_ref().is_some_and(|decoder| {
                decoder.clip_id() == self.timeline.clips()[clip_index].id
                    && !decoder.is_preview_graph()
            });
            if !decoder_matches {
                self.stop_decoder();
                let decoder = {
                    let clip = &self.timeline.clips()[clip_index];
                    let source = &self.timeline.sources()[source_index];
                    VideoDecoder::open(
                        &self.config.ffmpeg_binary,
                        &self.timeline,
                        clip,
                        source,
                        self.decode_cursor,
                        self.config.frame_width,
                        self.config.frame_height,
                        false,
                        Arc::clone(&self.epoch),
                        self.serve_epoch,
                    )
                };
                match decoder {
                    Ok(decoder) => self.decoder = Some(decoder),
                    Err(error) => {
                        self.emit(PlaybackEvent::Error(error));
                        self.decode_cursor = clip_end;
                        break;
                    }
                }
            }
            let Some(decoder) = self.decoder.as_mut() else {
                break;
            };
            match decoder.next_frame() {
                Ok(Some(decoded)) => {
                    let timeline_time = self.decode_cursor;
                    if timeline_time >= clip_end {
                        self.stop_decoder();
                        continue;
                    }
                    let frame = self.make_frame(clip_index, decoded, timeline_time);
                    self.decode_queue.push_back(frame);
                    self.decode_cursor += frame_interval(self.timeline.fps);
                }
                Ok(None) => {
                    if self.decoder.as_ref().is_some_and(|decoder| decoder.is_cancelled()) {
                        break;
                    }
                    self.stop_decoder();
                    self.decode_cursor = clip_end;
                }
                Err(error) => {
                    self.emit(PlaybackEvent::Error(error));
                    self.decode_cursor = clip_end;
                    self.stop_decoder();
                    break;
                }
            }
        }
    }

    fn make_frame(
        &mut self,
        clip_index: usize,
        decoded: DecodedVideoFrame,
        timeline_time: f64,
    ) -> PlaybackFrame {
        let clip = &self.timeline.clips()[clip_index];
        let frame = PlaybackFrame {
            sequence: self.sequence,
            timeline_time,
            source_time: clip.source_time_at(timeline_time),
            clip_id: clip.id.clone(),
            width: decoded.width,
            height: decoded.height,
            pixels: decoded.pixels,
            captions: self.timeline.captions_at(timeline_time),
        };
        self.sequence += 1;
        frame
    }

    fn stop_decoder(&mut self) {
        if let Some(mut decoder) = self.decoder.take() {
            decoder.stop();
        }
    }

    fn set_state(&mut self, state: PlaybackState) {
        if self.state != state {
            self.state = state;
            self.emit(PlaybackEvent::StateChanged(state));
        }
    }

    fn clock(&self) -> PlaybackClock {
        self.audio.clock(self.timeline.duration)
    }

    fn emit_clock(&mut self, force: bool) {
        if force || self.last_clock_event.elapsed() >= CLOCK_EVENT_INTERVAL {
            self.last_clock_event = Instant::now();
            self.emit(PlaybackEvent::Clock(self.clock()));
        }
    }

    fn emit(&mut self, event: PlaybackEvent) {
        self.flush_pending_events();
        let event = QueuedEvent {
            epoch: self.event_epoch,
            event,
        };
        match self.events.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(event)) => {
                if matches!(&event.event, PlaybackEvent::Frame(_)) {
                    return;
                }
                if matches!(&event.event, PlaybackEvent::Clock(_)) {
                    if let Some(existing) = self
                        .pending_events
                        .iter_mut()
                        .find(|pending| matches!(&pending.event, PlaybackEvent::Clock(_)))
                    {
                        *existing = event;
                    } else {
                        self.queue_pending(event);
                    }
                } else {
                    self.queue_pending(event);
                }
            }
        }
    }

    fn flush_pending_events(&mut self) {
        while let Some(event) = self.pending_events.pop_front() {
            match self.events.try_send(event) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                Err(TrySendError::Full(event)) => {
                    self.pending_events.push_front(event);
                    break;
                }
            }
        }
    }

    fn queue_pending(&mut self, event: QueuedEvent) {
        if self.pending_events.len() >= self.pending_event_capacity {
            if let Some(index) = self
                .pending_events
                .iter()
                .position(|pending| matches!(&pending.event, PlaybackEvent::Clock(_)))
            {
                self.pending_events.remove(index);
            } else {
                self.pending_events.pop_front();
            }
        }
        self.pending_events.push_back(event);
    }
}

fn frame_interval(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        1.0 / fps
    } else {
        1.0 / 30.0
    }
}

fn can_reuse_ffmpeg_decoder(
    decoder_clip_id: &str,
    decode_cursor: f64,
    target: f64,
    clip_id: &str,
    clip_end: f64,
    fps: f64,
) -> bool {
    if decoder_clip_id != clip_id {
        return false;
    }
    if !target.is_finite() || target < 0.0 || target >= clip_end {
        return false;
    }
    let interval = frame_interval(fps);
    if target + interval * 1.5 < decode_cursor {
        return false;
    }
    target - decode_cursor <= SMALL_FORWARD_SEEK_SECONDS
}

#[cfg(test)]
mod reuse_tests {
    use super::{can_reuse_ffmpeg_decoder, frame_interval, SMALL_FORWARD_SEEK_SECONDS};

    #[test]
    fn reuses_small_forward_seeks_on_the_same_clip() {
        assert!(can_reuse_ffmpeg_decoder(
            "clip-a", 0.0, 0.2, "clip-a", 4.0, 30.0
        ));
        assert!(can_reuse_ffmpeg_decoder(
            "clip-a",
            0.5,
            0.5 + SMALL_FORWARD_SEEK_SECONDS,
            "clip-a",
            4.0,
            30.0
        ));
    }

    #[test]
    fn restarts_backward_far_forward_or_clip_changes() {
        let interval = frame_interval(30.0);
        assert!(!can_reuse_ffmpeg_decoder(
            "clip-a",
            1.0,
            1.0 - interval * 3.0,
            "clip-a",
            4.0,
            30.0
        ));
        assert!(!can_reuse_ffmpeg_decoder(
            "clip-a",
            0.0,
            SMALL_FORWARD_SEEK_SECONDS + 0.05,
            "clip-a",
            4.0,
            30.0
        ));
        assert!(!can_reuse_ffmpeg_decoder(
            "clip-a", 0.0, 0.1, "clip-b", 4.0, 30.0
        ));
        assert!(!can_reuse_ffmpeg_decoder(
            "clip-a", 0.0, 4.0, "clip-a", 4.0, 30.0
        ));
    }
}

fn resolve_ffmpeg_binary() -> PathBuf {
    std::env::var_os("CUT_FFMPEG")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project_with_media() -> cut_domain::Project {
        cut_domain::parse_project(&json!({
            "version": 2,
            "name": "Playback",
            "fps": 30,
            "width": 320,
            "height": 180,
            "sources": [{"id":"source-a","path":"/tmp/a.mp4","duration":8,"hasAudio":false}],
            "tracks": [{"id":"video-1","type":"video","clips":[
                {"id":"clip-a","sourceId":"source-a","in":2,"out":6,"timelineStart":1,"speed":2}
            ]}],
            "overlays": [], "gaps": [], "transitions": [], "effects": [], "markers": []
        }))
        .expect("valid project")
    }

    #[test]
    fn public_session_accepts_controls_without_starting_ffmpeg() {
        let session = PlaybackSession::open(
            cut_domain::empty_project(),
            PlaybackConfig::default().with_audio_enabled(false),
        )
        .expect("session");
        assert!(session.handle().seek(0.25).is_ok());
        assert!(session.handle().set_volume(0.5).is_ok());
        assert!(session.handle().set_muted(true).is_ok());
        assert!(session.handle().pause().is_ok());
    }

    #[test]
    fn frame_byte_validation_rejects_truncated_rgba() {
        let frame = PlaybackFrame {
            sequence: 0,
            timeline_time: 0.0,
            source_time: 0.0,
            clip_id: "clip".into(),
            width: 2,
            height: 2,
            pixels: Arc::from(vec![0_u8; 3].into_boxed_slice()),
            captions: Vec::new(),
        };
        assert!(!frame.has_valid_pixels());
    }

    #[test]
    fn invalid_controls_are_rejected_at_the_interface() {
        let session = PlaybackSession::open(
            cut_domain::empty_project(),
            PlaybackConfig::default().with_audio_enabled(false),
        )
        .expect("session");
        assert!(matches!(
            session.seek(-1.0),
            Err(PlaybackError::InvalidValue(_))
        ));
        assert!(matches!(
            session.set_volume(2.0),
            Err(PlaybackError::InvalidValue(_))
        ));
    }

    #[test]
    fn media_project_is_accepted_even_when_source_is_not_opened_yet() {
        let timeline = PlaybackTimeline::from_project(&project_with_media()).expect("timeline");
        assert_eq!(timeline.duration, 3.0);
        assert_eq!(timeline.width, 320);
        assert_eq!(timeline.height, 180);
    }

    fn worker_for_state_test() -> (Worker, Receiver<QueuedEvent>, Arc<Mutex<ControlState>>) {
        let timeline = PlaybackTimeline::from_project(&project_with_media()).expect("timeline");
        let (commands, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (events, event_receiver) = mpsc::sync_channel(16);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let epoch = Arc::clone(&control.lock().expect("control").epoch);
        let audio = AudioRuntime::spawn(timeline.clone(), PathBuf::from("ffmpeg").as_path(), false)
            .expect("audio worker");
        let worker = Worker {
            timeline,
            config: PlaybackConfig::default().with_audio_enabled(false),
            commands: command_receiver,
            events,
            audio,
            state: PlaybackState::Ready,
            position: 0.0,
            decoder: None,
            decode_queue: VecDeque::new(),
            decode_cursor: 0.0,
            sequence: 0,
            preview_requested: false,
            last_clock_event: Instant::now(),
            volume: 1.0,
            muted: false,
            pending_events: VecDeque::new(),
            pending_event_capacity: 16,
            control: Arc::clone(&control),
            event_epoch: 0,
            deferred_command: None,
            preview_rough: false,
            needs_retarget: false,
            retarget_early: false,
            serve_epoch: 0,
            epoch,
        };
        drop(commands);
        (worker, event_receiver, control)
    }

    #[test]
    fn rapid_seek_requests_are_coalesced_without_filling_the_command_queue() {
        let session = PlaybackSession::open(
            cut_domain::empty_project(),
            PlaybackConfig::default().with_audio_enabled(false),
        )
        .expect("session");
        for position in 0..(COMMAND_QUEUE_CAPACITY * 4) {
            session.seek(position as f64).expect("coalesced seek");
        }
    }

    #[test]
    fn latest_seek_slot_keeps_only_the_newest_target() {
        let (commands, _receiver) = mpsc::sync_channel(1);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let handle = PlaybackHandle {
            commands,
            control: Arc::clone(&control),
        };
        handle.seek(1.0).expect("first seek");
        handle.seek(2.0).expect("second seek");
        assert_eq!(
            control
                .lock()
                .expect("control")
                .latest_seek
                .as_ref()
                .map(|request| request.position),
            Some(2.0)
        );
        assert_eq!(
            control
                .lock()
                .expect("control")
                .latest_seek
                .as_ref()
                .map(|request| request.request_id),
            Some(2)
        );
        assert!(control.lock().expect("control").latest_seek.as_ref().unwrap().exact);
        assert_eq!(
            control.lock().expect("control").epoch.load(Ordering::Acquire),
            2
        );
    }

    #[test]
    fn scrub_keeps_only_the_latest_approximate_target() {
        let (commands, _receiver) = mpsc::sync_channel(1);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let handle = PlaybackHandle {
            commands,
            control: Arc::clone(&control),
        };
        handle.seek(1.0).expect("exact seek");
        handle.scrub(4.0).expect("approximate scrub");
        let guard = control.lock().expect("control");
        let request = guard.latest_seek.as_ref().expect("latest seek");
        assert_eq!(request.position, 4.0);
        assert!(!request.exact);
        assert!(guard.epoch.load(Ordering::Acquire) >= request.request_id);
    }

    #[test]
    fn seek_repeats_current_state_after_advancing_event_epoch() {
        let (mut worker, events, control) = worker_for_state_test();
        assert!(worker.handle_command(WorkerCommand::Play));
        let _: Vec<_> = events.try_iter().collect();
        control.lock().expect("control").visible_epoch = 1;
        assert!(worker.handle_command(WorkerCommand::Seek(SeekRequest {
            position: 1.5,
            request_id: 1,
            exact: true,
        })));
        let events: Vec<_> = events.try_iter().collect();
        assert!(events.iter().any(|event| {
            event.epoch == 1
                && matches!(
                    event.event,
                    PlaybackEvent::StateChanged(PlaybackState::Playing)
                )
        }));
    }

    #[test]
    fn accepted_open_publishes_its_event_epoch_before_worker_handles_it() {
        let (commands, _receiver) = mpsc::sync_channel(1);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let handle = PlaybackHandle {
            commands,
            control: Arc::clone(&control),
        };
        handle
            .open(cut_domain::empty_project())
            .expect("accepted open");
        assert_eq!(control.lock().expect("control").visible_epoch, 1);
    }

    #[test]
    fn rapid_opens_hide_events_from_superseded_projects() {
        let (command_tx, _command_rx) = mpsc::sync_channel(4);
        let (event_tx, event_rx) = mpsc::sync_channel(4);
        let control = Arc::new(Mutex::new(ControlState::new()));
        let handle = PlaybackHandle {
            commands: command_tx,
            control: Arc::clone(&control),
        };
        handle
            .open(cut_domain::empty_project())
            .expect("first open");
        handle
            .open(cut_domain::empty_project())
            .expect("second open");
        event_tx
            .send(QueuedEvent {
                epoch: 1,
                event: PlaybackEvent::Opened {
                    duration: 1.0,
                    width: 2,
                    height: 2,
                },
            })
            .expect("first event");
        event_tx
            .send(QueuedEvent {
                epoch: 2,
                event: PlaybackEvent::Opened {
                    duration: 2.0,
                    width: 2,
                    height: 2,
                },
            })
            .expect("second event");
        let session = PlaybackSession {
            handle,
            events: event_rx,
            control,
        };
        assert!(matches!(
            session.try_next_event(),
            Some(PlaybackEvent::Opened { duration, .. }) if duration == 2.0
        ));
    }

    #[test]
    fn events_from_before_a_seek_are_filtered_at_the_session_boundary() {
        let (command_tx, _command_rx) = mpsc::sync_channel(1);
        let (event_tx, event_rx) = mpsc::sync_channel(2);
        let control = Arc::new(Mutex::new(ControlState::new()));
        event_tx
            .send(QueuedEvent {
                epoch: 0,
                event: PlaybackEvent::Frame(PlaybackFrame {
                    sequence: 0,
                    timeline_time: 0.0,
                    source_time: 0.0,
                    clip_id: "clip".into(),
                    width: 2,
                    height: 2,
                    pixels: Arc::from(vec![0_u8; 16].into_boxed_slice()),
                    captions: Vec::new(),
                }),
            })
            .expect("queued event");
        control.lock().expect("control").visible_epoch = 1;
        let session = PlaybackSession {
            handle: PlaybackHandle {
                commands: command_tx,
                control: Arc::clone(&control),
            },
            events: event_rx,
            control,
        };
        assert!(session.try_next_event().is_none());
    }
}
