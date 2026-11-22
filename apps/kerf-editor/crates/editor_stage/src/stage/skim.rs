//! Skimming: the pointer across the picture (or the timeline's lanes)
//! shows the program at its time without moving the playhead, which the
//! picture keeps ([`Kept`]) while kerfd may still answer the skim's seeks;
//! off the picture, on a press or on Play, the playhead's frame is back.

use super::overlay::Pt;
use super::picture::{to_point, to_pt};
use super::StageView;
use editor_session::session::Session;
use gpui::{App, Bounds, Context, Entity, EntityId, Global, Pixels};
use std::collections::HashMap;

/// The least move across the picture, as a share of it, that skims again.
const SKIM_STEP: f64 = 0.0005;

/// What each window's picture is skimmed to, by its session.
#[derive(Default)]
struct Skims(HashMap<EntityId, f64>);

impl Global for Skims {}

/// The time the pointer shows across `session`'s picture, while it skims
/// it: the timeline's skimmer. The playhead stays where it was.
pub fn skim(session: &Entity<Session>, cx: &App) -> Option<f64> {
    cx.try_global::<Skims>()?
        .0
        .get(&session.entity_id())
        .copied()
}

/// Seconds within which a clock's time is the one that was sent.
const SAME_TIME: f64 = 1e-4;

/// The most times shown that kerfd has yet to answer, kept.
const SHOWN_KEPT: usize = 256;

fn same_time(a: f64, b: f64) -> bool {
    (a - b).abs() < SAME_TIME
}

/// The playhead a skimming picture keeps. kerfd answers each seek with a
/// clock, and those that answer the skim's say where the pointer was, not
/// where the playhead is: from the skim's start until kerfd is back at the
/// playhead, a clock does not move it.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    playhead: f64,
    /// The times the pointer showed, oldest first, that kerfd may yet answer.
    shown: Vec<f64>,
}

/// What one of kerfd's clocks is to a picture that keeps the playhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// The playhead's own time.
    Playhead,
    /// A time the pointer showed.
    Shown,
    /// Neither: something else sent kerfd there.
    Elsewhere,
}

impl Kept {
    pub fn new(playhead: f64) -> Self {
        Self {
            playhead,
            shown: Vec::new(),
        }
    }

    pub fn playhead(&self) -> f64 {
        self.playhead
    }

    /// Whether the window's playhead is still the one kept; if not,
    /// something else (a key, a click) moved it.
    pub fn holds(&self, position: f64) -> bool {
        same_time(position, self.playhead)
    }

    /// Something else moved the playhead to `at`: it is kept there now, and
    /// the times shown before still may not move it.
    pub fn moved_to(&mut self, at: f64) {
        self.playhead = at;
    }

    /// The pointer showed `at`.
    pub fn show(&mut self, at: f64) {
        self.shown.retain(|time| !same_time(*time, at));
        self.shown.push(at);
        if self.shown.len() > SHOWN_KEPT {
            self.shown.remove(0);
        }
    }

    /// What kerfd's clock at `at` is. kerfd answers seeks in the order they
    /// were sent, so none shown before the one it answers is answered after.
    pub fn hear(&mut self, at: f64) -> Heard {
        if same_time(at, self.playhead) {
            return Heard::Playhead;
        }
        match self.shown.iter().position(|time| same_time(*time, at)) {
            Some(index) => {
                self.shown.drain(..index);
                Heard::Shown
            }
            None => Heard::Elsewhere,
        }
    }
}

/// `session`'s picture skims no more: the timeline's skimmer goes.
pub(super) fn forget_skim(session: EntityId, cx: &mut App) {
    if cx.has_global::<Skims>() {
        cx.global_mut::<Skims>().0.remove(&session);
    }
}

impl StageView {
    /// Shows the program at the time `at` gives across the picture, without
    /// moving the playhead; off the picture, the playhead's frame again.
    pub(super) fn skim_to(&mut self, picture: Bounds<Pixels>, at: Pt, cx: &mut Context<Self>) {
        self.follow_playhead(cx);
        let (playing, length, held, position, connected) = {
            let session = self.session.read(cx);
            let state = &session.state;
            (
                state.playing,
                state.program_length(),
                state.preview_required,
                state.position,
                session.connected(),
            )
        };
        if playing || length <= 0.0 || held || !connected || !picture.contains(&to_point(at)) {
            self.end_skim(cx);
            return;
        }
        let (left, _) = to_pt(picture.origin);
        let width = f64::from(f32::from(picture.size.width)).max(1.0);
        let share = ((at.0 - left) / width).clamp(0.0, 1.0);
        if self
            .skim
            .is_some_and(|time| (time / length - share).abs() < SKIM_STEP)
        {
            return;
        }
        let time = share * length;
        self.skim = Some(time);
        self.kept
            .get_or_insert_with(|| Kept::new(position))
            .show(time);
        cx.default_global::<Skims>()
            .0
            .insert(self.session.entity_id(), time);
        // Rough while the pointer moves, exact once it rests.
        self.session.update(cx, |session, _| session.skim(time));
        cx.notify();
    }

    /// The timeline's skim (`TimelineEvent::Skim`, which the window passes
    /// on): the picture shows the program at `time` without moving the
    /// playhead, as skimming the picture itself does; None shows the
    /// playhead's frame again.
    pub fn skim_at(&mut self, time: Option<f64>, cx: &mut Context<Self>) {
        self.follow_playhead(cx);
        let Some(time) = time else {
            self.end_skim(cx);
            return;
        };
        let (playing, length, held, position, connected) = {
            let session = self.session.read(cx);
            let state = &session.state;
            (
                state.playing,
                state.program_length(),
                state.preview_required,
                state.position,
                session.connected(),
            )
        };
        if playing || length <= 0.0 || held || !connected || !time.is_finite() {
            self.end_skim(cx);
            return;
        }
        let time = time.clamp(0.0, length);
        if self.skim.is_some_and(|known| (known - time).abs() < 1e-6) {
            return;
        }
        self.skim = Some(time);
        self.kept
            .get_or_insert_with(|| Kept::new(position))
            .show(time);
        cx.default_global::<Skims>()
            .0
            .insert(self.session.entity_id(), time);
        self.session.update(cx, |session, _| session.skim(time));
        cx.notify();
    }

    /// The pointer left the picture (or a key is about to act from the
    /// playhead): the playhead's frame again.
    pub fn end_skim(&mut self, cx: &mut Context<Self>) {
        self.follow_playhead(cx);
        if self.skim.take().is_none() {
            return;
        }
        forget_skim(self.session.entity_id(), cx);
        if let Some(playhead) = self.kept.as_ref().map(Kept::playhead) {
            self.session.update(cx, |session, cx| {
                session.seek(playhead, false);
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Something other than the skim moved the playhead (a key, a command):
    /// the skim ends there, and the picture shows where it went.
    pub(super) fn follow_playhead(&mut self, cx: &mut Context<Self>) {
        let position = self.session.read(cx).state.position;
        let Some(kept) = self.kept.as_mut() else {
            return;
        };
        if kept.holds(position) {
            return;
        }
        kept.moved_to(position);
        if self.skim.take().is_some() {
            forget_skim(self.session.entity_id(), cx);
        }
        cx.notify();
    }

    /// One of kerfd's clocks, which the window has taken for the playhead's
    /// time. One that answers the skim is not: the playhead stays, and the
    /// picture follows where it is.
    pub(super) fn hear_clock(&mut self, at: f64, cx: &mut Context<Self>) {
        // Playing, its clocks are the playhead's: the start of playback lets
        // the skim go, from the playhead.
        if self.session.read(cx).state.playing {
            return;
        }
        let skimming = self.skim.is_some();
        let Some(kept) = self.kept.as_mut() else {
            return;
        };
        let playhead = kept.playhead();
        let position = match kept.hear(at) {
            Heard::Playhead => {
                // Back at the playhead, with the pointer off the picture.
                if !skimming {
                    self.kept = None;
                }
                at
            }
            Heard::Shown => playhead,
            // While the pointer skims, the picture is its own; off it,
            // something else sent kerfd there, and the playhead follows.
            Heard::Elsewhere if skimming => playhead,
            Heard::Elsewhere => {
                self.kept = None;
                at
            }
        };
        self.session.update(cx, |session, _| {
            session.state.position = position;
        });
        cx.notify();
    }
}
