//! Native playback boundary for the Cut editor.
//!
//! The playback crate owns no project state. It receives a validated typed
//! project, maps timeline time to source time, and emits bounded frame/clock
//! events from a worker thread. The editor owns the UI adapter and uploads
//! delivered RGBA frames to its wgpu-backed texture system.

mod audio;
mod decoder;
mod timeline;

pub use timeline::{CaptionCue, PlaybackClip, PlaybackSource, PlaybackTimeline, ResolvedClip};

use audio::AudioRuntime;
use decoder::{DecodedVideoFrame, VideoDecoder};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Maximum number of decoded frames kept ahead of the UI.
pub const DEFAULT_DECODE_QUEUE_CAPACITY: usize = 4;
/// Maximum number of events waiting for the UI adapter.
pub const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 64;
const COMMAND_QUEUE_CAPACITY: usize = 32;
const CLOCK_EVENT_INTERVAL: Duration = Duration::from_millis(50);
const WORKER_TICK: Duration = Duration::from_millis(2);

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

/// Cloneable control handle for a playback worker.
#[derive(Clone, Debug)]
pub struct PlaybackHandle {
    commands: SyncSender<PlaybackCommand>,
}

impl PlaybackHandle {
    /// Replace the project without touching the editor's canonical runtime.
    pub fn open(&self, project: cut_domain::Project) -> Result<(), PlaybackError> {
        self.send(PlaybackCommand::Open(Box::new(project)))
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
        if !position.is_finite() || position < 0.0 {
            return Err(PlaybackError::InvalidValue(
                "seek position must be finite and nonnegative".into(),
            ));
        }
        self.send(PlaybackCommand::Seek(position))
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
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => PlaybackError::CommandQueueFull,
                TrySendError::Disconnected(_) => PlaybackError::WorkerUnavailable,
            })
    }
}

/// Stable controller/event boundary used by `cut-app` and future native
/// clients. Implementations must keep media work off the caller's UI thread.
pub trait PlaybackInterface {
    fn handle(&self) -> PlaybackHandle;
    fn try_next_event(&self) -> Option<PlaybackEvent>;
    fn next_event_timeout(&self, timeout: Duration) -> Option<PlaybackEvent>;
}

/// A worker-backed playback session.
pub struct PlaybackSession {
    handle: PlaybackHandle,
    events: Receiver<PlaybackEvent>,
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
        let (event_tx, event_rx) = mpsc::sync_channel(event_queue_capacity);
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
        };
        thread::Builder::new()
            .name("cut-playback".into())
            .spawn(move || worker.run())
            .map_err(|error| PlaybackError::Ffmpeg(format!("failed to start worker: {error}")))?;
        Ok(Self {
            handle: PlaybackHandle {
                commands: command_tx,
            },
            events: event_rx,
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

    pub fn set_volume(&self, volume: f32) -> Result<(), PlaybackError> {
        self.handle.set_volume(volume)
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), PlaybackError> {
        self.handle.set_muted(muted)
    }

    pub fn try_next_event(&self) -> Option<PlaybackEvent> {
        self.events.try_recv().ok()
    }

    pub fn next_event_timeout(&self, timeout: Duration) -> Option<PlaybackEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    pub fn drain_events(&self) -> impl Iterator<Item = PlaybackEvent> + '_ {
        self.events.try_iter()
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

/// Configuration for the FFmpeg decoder and bounded worker queues.
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
    commands: Receiver<PlaybackCommand>,
    events: SyncSender<PlaybackEvent>,
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
    pending_events: VecDeque<PlaybackEvent>,
    pending_event_capacity: usize,
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
            match self.commands.recv_timeout(timeout) {
                Ok(command) => {
                    if !self.handle_command(command) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
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

    fn handle_command(&mut self, command: PlaybackCommand) -> bool {
        match command {
            PlaybackCommand::Open(project) => match PlaybackTimeline::from_project(&project) {
                Ok(timeline) => {
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
                Err(error) => self.emit(PlaybackEvent::Error(error)),
            },
            PlaybackCommand::Play => self.start_playing(),
            PlaybackCommand::Pause => self.pause_playing(),
            PlaybackCommand::Seek(position) => self.seek_to(position),
            PlaybackCommand::SetVolume(volume) => {
                self.volume = volume;
                self.audio.set_volume(volume);
            }
            PlaybackCommand::SetMuted(muted) => {
                self.muted = muted;
                self.audio.set_muted(muted);
            }
            PlaybackCommand::Shutdown => return false,
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

    fn seek_to(&mut self, position: f64) {
        let next = position.min(self.timeline.duration.max(0.0));
        let was_playing = self.state == PlaybackState::Playing;
        self.audio.seek(next);
        self.position = next;
        self.decode_cursor = next;
        self.decode_queue.clear();
        self.stop_decoder();
        self.preview_requested = !was_playing;
        if was_playing {
            self.audio.play(next);
        }
        self.emit_clock(true);
    }

    fn tick_playing(&mut self) {
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
        self.fill_decode_queue();
        if let Some(frame) = self.decode_queue.pop_front() {
            self.emit(PlaybackEvent::Frame(frame));
            self.preview_requested = false;
            self.stop_decoder();
        } else if self.decoder.is_none() || self.timeline.video_at(self.position).is_none() {
            self.preview_requested = false;
        }
    }

    fn fill_decode_queue(&mut self) {
        let capacity = self.config.decode_queue_capacity.max(1);
        while self.decode_queue.len() < capacity {
            let Some(clip) = self.timeline.video_at(self.decode_cursor) else {
                let Some(next_start) = self.timeline.next_video_start_after(self.decode_cursor)
                else {
                    self.stop_decoder();
                    break;
                };
                self.decode_cursor = next_start;
                continue;
            };
            if self.decode_cursor >= clip.timeline_end() {
                self.decode_cursor = clip.timeline_end();
                self.stop_decoder();
                continue;
            }
            let decoder_matches = self
                .decoder
                .as_ref()
                .is_some_and(|decoder| decoder.clip_id() == clip.clip.id);
            if !decoder_matches {
                self.stop_decoder();
                match VideoDecoder::open(
                    &self.config.ffmpeg_binary,
                    &self.timeline,
                    &clip,
                    self.decode_cursor,
                    self.config.frame_width,
                    self.config.frame_height,
                ) {
                    Ok(decoder) => self.decoder = Some(decoder),
                    Err(error) => {
                        self.emit(PlaybackEvent::Error(error));
                        self.decode_cursor = clip.timeline_end();
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
                    if timeline_time >= clip.timeline_end() {
                        self.stop_decoder();
                        continue;
                    }
                    let frame = self.make_frame(&clip, decoded, timeline_time);
                    self.decode_queue.push_back(frame);
                    self.decode_cursor += frame_interval(self.timeline.fps);
                }
                Ok(None) => {
                    self.stop_decoder();
                    self.decode_cursor = clip.timeline_end();
                }
                Err(error) => {
                    self.emit(PlaybackEvent::Error(error));
                    self.decode_cursor = clip.timeline_end();
                    self.stop_decoder();
                    break;
                }
            }
        }
    }

    fn make_frame(
        &mut self,
        clip: &ResolvedClip,
        decoded: DecodedVideoFrame,
        timeline_time: f64,
    ) -> PlaybackFrame {
        let frame = PlaybackFrame {
            sequence: self.sequence,
            timeline_time,
            source_time: clip.source_time_at(timeline_time),
            clip_id: clip.clip.id.clone(),
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
        match self.events.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(event)) => {
                if matches!(&event, PlaybackEvent::Frame(_)) {
                    return;
                }
                if matches!(&event, PlaybackEvent::Clock(_)) {
                    if let Some(existing) = self
                        .pending_events
                        .iter_mut()
                        .find(|pending| matches!(pending, PlaybackEvent::Clock(_)))
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

    fn queue_pending(&mut self, event: PlaybackEvent) {
        if self.pending_events.len() >= self.pending_event_capacity {
            if let Some(index) = self
                .pending_events
                .iter()
                .position(|pending| matches!(pending, PlaybackEvent::Clock(_)))
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
}
