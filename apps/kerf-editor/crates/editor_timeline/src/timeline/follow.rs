//! What the timeline follows of the session and the bridge, as they say
//! it: a snapshot (the lanes built again, what it no longer has let go, a
//! drag whose clip went elsewhere ended), an edit refused, a preview, the
//! connection, Kerf's work and where it looks, the last check's findings;
//! and, as the timeline draws, the keymap, the strip's transport and a
//! change reel.

use super::drag::Gesture;
use super::{TimelineEvent, TimelineView};
use editor_state::model::{self, Timeline};
use editor_state::store::Change;
use gpui::{App, Context};
use kerf_bridge::event::{Check, Severity};
use kerf_bridge::Event;
use std::collections::HashSet;
use std::time::Instant;

impl TimelineView {
    /// What the timeline keeps for itself of the session and the playhead,
    /// as it draws (put away with the panel, nothing): each command's keys,
    /// the strip's transport, a change reel's clips rising or settling.
    pub(super) fn follow(&mut self, cx: &App) {
        self.follow_keymap(cx);
        self.follow_transport(cx);
        self.follow_watch(cx);
    }

    pub(super) fn session_changed(&mut self, change: &Change, cx: &mut Context<Self>) {
        match change {
            Change::Snapshot => self.follow_snapshot(cx),
            // An edit kerfd refused says so: the dropped clip goes back.
            Change::Notice => {
                let refused = |at: Instant, cx: &App| {
                    let notices = &self.session.read(cx).state.notices;
                    notices
                        .last()
                        .is_some_and(|notice| notice.error && notice.at >= at)
                };
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| refused(pending.at, cx))
                {
                    self.pending = None;
                    self.drawn = None;
                    self.held_gain = None;
                    cx.notify();
                }
            }
            Change::Making => self.tick_makings(cx),
            Change::Preview => {
                let preview = &self.session.read(cx).state.preview;
                // Rendering keeps what plays; ready and live play the
                // proposal named; anything else ends it.
                let previewing = match preview.status.as_str() {
                    "running" => self.previewing.clone(),
                    "ready" | "live" => preview.proposal.clone(),
                    _ => None,
                };
                if previewing != self.previewing {
                    self.previewing = previewing;
                    self.rebuild(cx);
                    cx.notify();
                }
            }
            Change::Playback => {
                if self.session.read(cx).state.playing {
                    self.end_skim(cx);
                }
            }
            Change::Connection => {
                // Solo is monitoring kerfd keeps for the session: sent again.
                let connected = self.session.read(cx).connected();
                if connected && !self.solo.is_empty() {
                    self.send_solo(cx);
                }
                // Gone or only connecting: what was in hand is let go
                // unsent, and nothing is pointed at.
                if !connected {
                    self.gesture = None;
                    self.hover = None;
                    self.tip = None;
                    self.end_skim(cx);
                    cx.notify();
                }
            }
            Change::Work | Change::TurnEnded => {
                // A cut Kerf builds shows its newest version while it does.
                let session = self.session.read(cx);
                let building = session
                    .state
                    .work
                    .busy()
                    .then(|| session.live_cut())
                    .flatten()
                    .map(|(proposal, _)| proposal.proposal_id.as_str());
                if building != self.building.as_deref() {
                    self.rebuild(cx);
                    cx.notify();
                }
                if !self.session.read(cx).state.work.busy() && self.kerf_at.is_some() {
                    self.kerf_at = None;
                    self.live_changes.clear();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// A snapshot came: the lanes are built again from it; what it no
    /// longer has is let go, and what was drawn until kerfd answered goes
    /// once it has; a drag whose clip went elsewhere, or under a first cut,
    /// ends; Kerf's cursor moves on.
    fn follow_snapshot(&mut self, cx: &mut Context<Self>) {
        self.prune(cx);
        self.rebuild(cx);
        let revision = self.session.read(cx).revision();
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.revision != revision)
        {
            self.pending = None;
        }
        if self
            .drawn
            .as_ref()
            .is_some_and(|(_, _, at)| *at != revision)
        {
            self.drawn = None;
        }
        if self
            .held_gain
            .as_ref()
            .is_some_and(|(_, at)| *at != revision)
        {
            self.held_gain = None;
        }
        // A drag whose clip went elsewhere, or under a first cut, ends.
        let lanes = self.lanes.clone();
        let ends = match &self.gesture {
            Some(Gesture::Clip(drag)) => {
                let same = lanes.find(&drag.id).is_some_and(|(row, _)| row == drag.row);
                !same || lanes.proposed || drag.target >= lanes.rows.len()
            }
            Some(Gesture::Band(band)) => lanes.find(&band.clip).is_none(),
            Some(Gesture::Signal(draw)) => lanes.find(&draw.clip).is_none(),
            _ => false,
        };
        if ends {
            self.gesture = None;
        }
        if self
            .hover
            .as_ref()
            .is_some_and(|hover| lanes.find(&hover.clip).is_none())
        {
            self.hover = None;
        }
        self.follow_live_proposal(cx);
        cx.notify();
    }

    /// What a snapshot no longer has leaves: chosen clips, targets, solo.
    fn prune(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            let timeline = &session.state.snapshot.timeline;
            let before = session.selection.len();
            session.selection.retain(|id| timeline.clip(id).is_some());
            if session.selection.len() != before {
                cx.notify();
            }
        });
        let tracks: HashSet<String> = self
            .session
            .read(cx)
            .timeline()
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .collect();
        let pruned = self.targets.as_mut().is_some_and(|targets| {
            let before = targets.len();
            targets.retain(|id| tracks.contains(id));
            targets.len() != before
        });
        if pruned {
            cx.emit(TimelineEvent::Targets(self.targets.clone()));
        }
        let before = self.solo.len();
        self.solo.retain(|id| tracks.contains(id));
        if self.solo.len() != before {
            self.send_solo(cx);
        }
    }

    /// What the bridge says that the timeline keeps itself: the last
    /// check's findings, and where Kerf looks.
    pub(super) fn bridge_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::Check(Check::Done { findings, .. }) => {
                let qc: HashSet<String> = findings
                    .iter()
                    .filter(|finding| finding.severity != Severity::Info)
                    .filter_map(|finding| finding.entity.as_deref())
                    .filter(|entity| !entity.is_empty())
                    .map(str::to_owned)
                    .collect();
                if qc != self.qc {
                    self.qc = qc;
                    self.rebuild(cx);
                    cx.notify();
                }
            }
            Event::AgentFocus { times, clips, .. } => {
                let session = self.session.read(cx);
                let snapshot = &session.state.snapshot;
                let timeline = &snapshot.timeline;
                let at = times
                    .first()
                    .and_then(|time| {
                        model::focus_time(time, timeline.rate(), timeline.start.unwrap_or(0.0))
                    })
                    .or_else(|| {
                        let clip = clips.first()?;
                        let named = |timeline: &Timeline| {
                            timeline.clip(clip).map(|(_, clip)| clip.timeline_start)
                        };
                        named(timeline)
                            .or_else(|| snapshot.proposal_timeline.as_ref().and_then(named))
                            .or_else(|| snapshot.proposal_timelines.values().find_map(named))
                    });
                if at.is_some() && at != self.kerf_at {
                    self.kerf_at = at;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// While Kerf works, its cursor moves on to the newest change in its
    /// live proposal: where a trim cuts, else where the clip now starts.
    fn follow_live_proposal(&mut self, cx: &App) {
        let session = self.session.read(cx);
        if !session.state.work.busy() {
            return;
        }
        let snapshot = &session.state.snapshot;
        let Some(proposed) = snapshot
            .pending()
            .and_then(|pending| snapshot.timeline_of(pending, None))
        else {
            return;
        };
        let changes = model::proposal_changes(&snapshot.timeline, proposed);
        let mut keys = HashSet::new();
        let mut newest: Option<(f64, f64, bool)> = None;
        for change in &changes {
            let Some((start, end)) = change.new else {
                continue;
            };
            let key = format!("{}@{end}", change.clip);
            if !self.live_changes.contains(&key) && newest.is_none_or(|(at, _, _)| start >= at) {
                newest = Some((start, end, change.trimmed && !change.moved));
            }
            keys.insert(key);
        }
        self.live_changes = keys;
        if let Some((start, end, trim)) = newest {
            self.kerf_at = Some(if trim { end } else { start });
        }
    }
}
