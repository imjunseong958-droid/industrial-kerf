//! The viewer's media worker: the program, or a rendered preview in its place, played as a
//! playback session, its frames put where the editor takes them and its clock, state and
//! levels said as events.

pub(crate) mod preview;

use crate::event::{Preview, PreviewStatus};
use crate::wake::Outbox;
use crate::{emit, error, meter, Command, Event, MediaProject, Shared};
use kerf_model::{Id, Project};
use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use preview::{discard_preview, discard_previews, sweep_previews};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn dimension(value: f64) -> u32 {
    value.round().clamp(2.0, u32::MAX as f64) as u32
}

/// The viewer frame: the sequence canvas, with its longest edge at most
/// 1920 pixels.
fn preview_frame_size(project: &Project) -> (u32, u32) {
    let (width, height) = project.sequences.first().map_or((1920, 1080), |sequence| {
        (sequence.width.max(2), sequence.height.max(2))
    });
    let scale = (1920.0 / f64::from(width.max(height))).min(1.0);
    (
        dimension(f64::from(width) * scale),
        dimension(f64::from(height) * scale),
    )
}

/// With `KERF_PLAYBACK_STATS` set, a line on stderr every 5 seconds of playing: frames shown and
/// dropped, and the size they came at, for measuring playback in the real editor.
struct PlaybackStats {
    on: bool,
    since: Instant,
    shown: u64,
    dropped: u64,
    dropped_before: u64,
    size: (u32, u32),
}

impl PlaybackStats {
    fn new() -> Self {
        Self {
            on: std::env::var_os("KERF_PLAYBACK_STATS").is_some(),
            since: Instant::now(),
            shown: 0,
            dropped: 0,
            dropped_before: 0,
            size: (0, 0),
        }
    }

    fn frame(&mut self, width: u32, height: u32) {
        self.shown += 1;
        self.size = (width, height);
    }

    fn clock(&mut self, dropped: u64) {
        self.dropped = dropped;
        if !self.on || self.since.elapsed() < Duration::from_secs(5) {
            return;
        }
        let spent = self.since.elapsed().as_secs_f64();
        eprintln!(
            "playback: {:.1} shown/s, {:.1} dropped/s, {}x{}",
            self.shown as f64 / spent,
            dropped.saturating_sub(self.dropped_before) as f64 / spent,
            self.size.0,
            self.size.1
        );
        self.shown = 0;
        self.dropped_before = dropped;
        self.since = Instant::now();
    }
}

/// Play what the viewer shows until the bridge closes: the program, or a rendered preview in
/// its place, each as it is put in the media slot.
pub(crate) fn media_worker(rx: Receiver<Command>, tx: Outbox, shared: Arc<Shared>) {
    let mut player = Player {
        stats: PlaybackStats::new(),
        session: None,
        position: 0.0,
        playing: false,
        volume: 1.0,
        solo: Vec::new(),
        native: false,
    };
    let mut swept = Instant::now();
    let mut meter = meter::Meter::default();
    while !shared.closed.load(Ordering::Relaxed) {
        for command in rx.try_iter() {
            player.command(command, &tx);
        }
        // What waits in the slot becomes what is shown with the slot held,
        // so `preview_exit` finds a preview in one place or the other.
        let taken = take_waiting(&shared);
        if let Some((project, rendered, shown_proposal, old)) = taken
            && !player.show(project, rendered, shown_proposal, old, &tx, &shared)
        {
            thread::sleep(Duration::from_millis(4));
            continue;
        }
        player.follow(&tx, &shared);
        meter.tick(
            Instant::now(),
            player.playing && player.session.is_some(),
            || player.session.as_ref().and_then(PlaybackSession::levels),
            &tx,
        );
        if swept.elapsed() >= Duration::from_millis(250) {
            sweep_previews(&shared);
            swept = Instant::now();
        }
        thread::sleep(if player.session.is_some() {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(4)
        });
    }
    drop(player.session);
    discard_previews(&shared);
    // A playback worker that is shutting down may hold its file open a
    // little longer.
    for _ in 0..40 {
        if sweep_previews(&shared) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// What the viewer plays, and what carries over from one playback session to the next.
struct Player {
    stats: PlaybackStats,
    session: Option<PlaybackSession>,
    position: f64,
    playing: bool,
    volume: f32,
    /// The tracks heard alone, kept across sessions; the program's own
    /// sessions only, as a rendered preview has none of its tracks.
    solo: Vec<Id>,
    /// What plays is the program itself, not a rendered preview of it.
    native: bool,
}

/// Take what waits in the media slot and make it what is shown, with the slot held: the
/// project, whether it is a rendered preview, the proposal that preview shows, and the
/// preview file shown until now.
fn take_waiting(shared: &Shared) -> Option<(Project, bool, Option<String>, Option<PathBuf>)> {
    shared.project.lock().ok().and_then(|mut slot| {
        // The proposal a rendered preview shows.
        let (project, file, shown_proposal) = match slot.take()? {
            MediaProject::Native(project) => (project, None, None),
            MediaProject::Rendered(project, file, proposal) => (project, Some(file), proposal),
        };
        let rendered = file.is_some();
        let old = shared
            .shown_preview
            .lock()
            .ok()
            .and_then(|mut shown| std::mem::replace(&mut *shown, file));
        Some((project, rendered, shown_proposal, old))
    })
}

impl Player {
    /// Carry out `solo`, `play`, `pause` or `volume` on the session playing; what it sets
    /// carries over to the next session.
    fn command(&mut self, command: Command, tx: &Outbox) {
        let handle = self.session.as_ref().map(PlaybackSession::handle);
        let result = match command {
            Command::Solo { tracks } => {
                self.solo = tracks
                    .iter()
                    .filter_map(|track| Id::new(track).ok())
                    .collect();
                handle
                    .filter(|_| self.native)
                    .map(|handle| handle.set_solo(self.solo.clone()))
            }
            Command::Play => {
                self.playing = true;
                handle.map(|handle| handle.play())
            }
            Command::Pause => {
                self.playing = false;
                handle.map(|handle| handle.pause())
            }
            Command::Volume { value } => {
                self.volume = value;
                handle.map(|handle| handle.set_volume(value))
            }
            _ => None,
        };
        if let Some(Err(err)) = result {
            error(tx, err);
        }
    }

    /// Show `project` in place of what was shown: the program, or a rendered preview of it
    /// (`rendered`) showing `shown_proposal`. `old`, the preview file shown until now, goes.
    /// `false` when the program cannot be played here, which leaves nothing playing.
    fn show(
        &mut self,
        project: Project,
        rendered: bool,
        shown_proposal: Option<String>,
        old: Option<PathBuf>,
        tx: &Outbox,
        shared: &Shared,
    ) -> bool {
        self.native = !rendered;
        // Close the old session before the preview it played goes.
        drop(self.session.take());
        if let Some(old) = old {
            discard_preview(shared, old);
        }
        if let Ok(mut frame) = shared.frame.lock() {
            *frame = None;
        }
        emit(tx, Event::ViewerReset);
        emit(
            tx,
            Event::Caption {
                text: String::new(),
            },
        );
        let plan = kerf_playback::preview_plan(&project);
        let message = plan
            .reasons
            .iter()
            .map(|reason| reason.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let composited = !rendered && plan.composited();
        if !rendered && !plan.native_exact() && !composited {
            self.session = None;
            self.playing = false;
            emit(tx, Event::Playback { playing: false });
            emit(
                tx,
                Event::Preview(Preview {
                    status: if plan.unavailable() {
                        PreviewStatus::Unavailable
                    } else {
                        PreviewStatus::Required
                    },
                    message: Some(message),
                    ..Preview::default()
                }),
            );
            return false;
        }
        // The compositor draws a program cuts cannot show, live; what it
        // or an export does not match yet comes along as a note.
        emit(
            tx,
            Event::Preview(if composited {
                Preview {
                    status: PreviewStatus::Live,
                    message: Some(message),
                    ..Preview::default()
                }
            } else if let Some(proposal) = &shown_proposal {
                Preview {
                    status: PreviewStatus::Ready,
                    proposal_id: Some(proposal.clone()),
                    ..Preview::default()
                }
            } else {
                Preview {
                    status: PreviewStatus::Ready,
                    ..Preview::default()
                }
            }),
        );
        self.open(project, composited, tx);
        true
    }

    /// Open a playback session of `project` where the last one was, at its volume, with the
    /// tracks heard alone, and playing if it played.
    fn open(&mut self, project: Project, composited: bool, tx: &Outbox) {
        let size = preview_frame_size(&project);
        // A new worker prevents old-project frames from resurfacing after an edit.
        self.session = match PlaybackSession::open(
            project,
            PlaybackConfig::default()
                .with_frame_size(size.0, size.1)
                .with_composited(composited),
        ) {
            Ok(session) => {
                // The new session goes on as the last one was; where it cannot, it plays as it
                // opened, and why is logged.
                let carried = |what: &str, result: Result<(), kerf_playback::PlaybackError>| {
                    if let Err(error) = result {
                        log::warn!("the reopened playback did not take its {what}: {error}");
                    }
                };
                carried("volume", session.handle().set_volume(self.volume));
                if self.native && !self.solo.is_empty() {
                    carried("solo", session.handle().set_solo(self.solo.clone()));
                }
                carried("position", session.handle().seek(self.position));
                if self.playing {
                    carried("playing", session.handle().play());
                }
                Some(session)
            }
            Err(err) => {
                error(tx, err);
                None
            }
        };
    }

    /// Carry out the seek asked for, and pass on what the session playing says: its frames
    /// to the frame slot, and its clock, its state and its errors as events.
    fn follow(&mut self, tx: &Outbox, shared: &Shared) {
        if let Some(session) = &self.session {
            let handle = session.handle();
            let seek = shared.seek.lock().ok().and_then(|mut slot| slot.take());
            if let Some(seek) = seek {
                // Where the viewer was asked to be, before the session says
                // so itself: an edit that replaces this session first opens
                // the next one here, not where the last frame was.
                self.position = seek.position;
                // Keep the picture that is already on screen. The UI drops a
                // frame only when a newer one is closer to the pointer, so
                // clearing this slot just freezes the viewer during a scrub.
                let result = if seek.exact {
                    handle.seek(seek.position)
                } else {
                    handle.scrub(seek.position)
                };
                if let Err(err) = result {
                    error(tx, err);
                }
            }
            for event in session.drain_events() {
                match event {
                    PlaybackEvent::Frame(frame) => {
                        self.stats.frame(frame.width, frame.height);
                        self.position = frame.timeline_time;
                        show_frame(frame, tx, shared);
                    }
                    PlaybackEvent::Clock(clock) => {
                        self.stats.clock(clock.dropped);
                        self.position = clock.position;
                        // Replaceable: the next clock comes soon.
                        let _ = tx.try_send(Event::Clock {
                            position: clock.position,
                            duration: clock.duration,
                            dropped: clock.dropped,
                        });
                    }
                    PlaybackEvent::StateChanged(state) => {
                        self.playing = state == PlaybackState::Playing;
                        emit(
                            tx,
                            Event::Playback {
                                playing: self.playing,
                            },
                        );
                    }
                    PlaybackEvent::Error(err) => error(tx, err),
                    PlaybackEvent::Ended => emit(tx, Event::Playback { playing: false }),
                    PlaybackEvent::Opened { duration, .. } => {
                        self.position = self.position.min(duration);
                        emit(
                            tx,
                            Event::Clock {
                                position: self.position,
                                duration,
                                dropped: 0,
                            },
                        );
                    }
                }
            }
        }
    }
}

/// A frame the session drew: its captions said, and the frame put in the slot the editor
/// takes frames from.
fn show_frame(frame: PlaybackFrame, tx: &Outbox, shared: &Shared) {
    let caption = frame
        .captions
        .iter()
        .map(|cue| cue.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Replaceable: every frame says its captions again.
    let _ = tx.try_send(Event::Caption { text: caption });
    if let Ok(mut slot) = shared.frame.lock() {
        *slot = Some(frame);
    }
    tx.ring();
}
