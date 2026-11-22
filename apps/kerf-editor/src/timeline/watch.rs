//! Watching changes on the timeline: while a change reel plays, the clips
//! its proposal changes ([`watched_clips`]) rise off their lanes ringed and
//! every other clip falls back ([`watch_look`]); as it ends they settle
//! back ([`ClipWatch`]), followed as the session says.

use super::TimelineView;
use crate::model;
use crate::motion;
use crate::theme;
use gpui::App;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The clips a change reel watches: every clip proposal `id` (or the
/// version of it `id` names, as the reel plays it) adds, takes out or
/// changes against the program, as its card lists them. None when the
/// snapshot has no timeline of it.
pub fn watched_clips(snapshot: &model::Snapshot, id: &str) -> HashSet<String> {
    let proposed = snapshot
        .proposals
        .iter()
        .find(|proposal| {
            proposal.proposal_id == id
                || proposal
                    .versions
                    .iter()
                    .any(|version| version.proposal_id == id)
        })
        .and_then(|proposal| snapshot.timeline_of(proposal, Some(id)))
        .or_else(|| snapshot.proposal_timelines.get(id));
    proposed
        .map(|proposed| {
            crate::changes::clip_changes(&snapshot.timeline, proposed)
                .into_iter()
                .filter(|change| !change.transition)
                .map(|change| change.clip)
                .collect()
        })
        .unwrap_or_default()
}

/// How a clip is drawn while a change reel watches: how far it has risen
/// off its lane (points), and how strong its ring, the glow under it and
/// the cover over it are (0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WatchLook {
    pub lift: f32,
    pub ring: f32,
    pub glow: f32,
    pub dim: f32,
}

/// A clip `amount` of the way into a change reel (0 at rest, 1 watching):
/// one the reel `watched` rises `CLIP_LIFT` off its lane over a soft glow,
/// ringed in the accent's ink (a sound clip stays on its lane, ringed);
/// every other clip falls back under a cover, so what the proposal changes
/// stands out from what it leaves alone.
pub fn watch_look(amount: f32, watched: bool, audio: bool) -> WatchLook {
    let amount = amount.clamp(0.0, 1.0);
    match (watched, audio) {
        (true, false) => WatchLook {
            lift: theme::motion::CLIP_LIFT * amount,
            ring: amount,
            glow: amount,
            dim: 0.0,
        },
        (true, true) => WatchLook {
            ring: amount,
            ..WatchLook::default()
        },
        (false, _) => WatchLook {
            dim: amount,
            ..WatchLook::default()
        },
    }
}

/// The clips a change reel watches as the timeline shows them: rising off
/// their lanes as it starts, settling back as it ends.
#[derive(Clone, Debug)]
pub struct ClipWatch {
    /// The reel's proposal and the revision its clips were found at.
    key: Option<(String, u64)>,
    clips: Rc<HashSet<String>>,
    /// How far into watching, 0..1.
    amount: motion::Tween,
}

impl Default for ClipWatch {
    fn default() -> Self {
        Self {
            key: None,
            clips: Rc::default(),
            amount: motion::Tween::at(0.0),
        }
    }
}

impl ClipWatch {
    /// Follows the reel at `now`: `reel` is its proposal and the snapshot's
    /// revision while one plays, None once it has ended; `find` gives the
    /// clips it watches ([`watched_clips`]), asked only when either changes.
    /// Clips the timeline was not lifting rise from their lanes over
    /// `STAGE`, eased; as the reel ends they settle back the same way, and
    /// are let go once they have. Under Reduce Motion (`reduced`) at once.
    pub fn follow_at(
        &mut self,
        reel: Option<(&str, u64)>,
        find: impl FnOnce() -> HashSet<String>,
        reduced: bool,
        now: Instant,
    ) {
        let scene = if reduced {
            Duration::ZERO
        } else {
            theme::motion::STAGE
        };
        let Some((proposal, revision)) = reel else {
            if self.key.take().is_some() {
                self.amount.go_at(0.0, scene, now);
            }
            if !self.amount.running_at(now) && self.amount.target() == 0.0 {
                self.clips = Rc::default();
            }
            return;
        };
        let same = self
            .key
            .as_ref()
            .is_some_and(|(known, at)| known == proposal && *at == revision);
        if same {
            return;
        }
        self.key = Some((proposal.to_owned(), revision));
        let clips = find();
        if *self.clips != clips {
            // Other clips rise from their lanes anew.
            self.clips = Rc::new(clips);
            self.amount.set(0.0);
        }
        self.amount.go_at(1.0, scene, now);
    }

    /// How far into watching it is at `now`, 0..1.
    pub fn amount_at(&self, now: Instant) -> f32 {
        self.amount.value_at(now)
    }

    /// Whether it is still rising or settling at `now`.
    pub fn running_at(&self, now: Instant) -> bool {
        self.amount.running_at(now)
    }

    /// The clips it lifts (still, while they settle back).
    pub fn clips(&self) -> &HashSet<String> {
        &self.clips
    }

    /// What the canvas draws at `now`: the clips and how far into watching,
    /// while anything shows.
    pub(super) fn shown_at(&self, now: Instant) -> Option<(Rc<HashSet<String>>, f32)> {
        let amount = self.amount_at(now);
        (amount > 0.0 && !self.clips.is_empty()).then(|| (self.clips.clone(), amount))
    }
}

impl TimelineView {
    /// While a change reel plays, what it watches rises off the lanes; as
    /// it ends, it settles back. Followed as the session (its reel, the
    /// snapshot's revision) and the playhead say, before the lanes draw.
    pub(super) fn follow_watch(&mut self, cx: &App) {
        let session = self.session.read(cx);
        let reel = session.reel().map(|reel| reel.proposal.clone());
        let snapshot = &session.state.snapshot;
        self.watch.follow_at(
            reel.as_deref()
                .map(|proposal| (proposal, snapshot.revision)),
            || {
                reel.as_deref()
                    .map(|proposal| watched_clips(snapshot, proposal))
                    .unwrap_or_default()
            },
            motion::reduced(),
            Instant::now(),
        );
    }
}
