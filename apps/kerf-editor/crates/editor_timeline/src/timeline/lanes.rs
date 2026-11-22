//! The lanes as the timeline draws them ([`Lanes`]): the tracks in the
//! order they stack, each clip with what it needs to be drawn, hit and
//! dragged, the markers and each open proposal's band. Built again from
//! the snapshot (and the version of each proposal shown) only when that
//! changes; laid out again when the heights do.

use super::TimelineView;
use editor_state::model::{
    self, Clip, ClipShow, ProposalBand, ProposalShow, Timeline, TimelineShow, Track, TrackKind,
    TrackShow,
};
use editor_state::prefs::TrackHeights;
use editor_ui::theme;
use gpui::{App, Context, Rgba, SharedString};
use std::collections::HashSet;
use std::rc::Rc;

/// Points between a lane's edge and its clips.
pub(super) const CLIP_INSET: f32 = 2.0;
/// What an empty sequence's ruler spans, in seconds, so it has labels to read.
const EMPTY_SECONDS: f64 = 30.0;

/// What kind of clip a clip is, as the timeline draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipKind {
    Media,
    /// A generated picture: a title, a solid, a signal.
    Generator,
    Caption,
}

/// The timeline as it is drawn: its tracks in the order they stack, with
/// what each clip needs to be drawn, hit and dragged.
#[derive(Clone, Default)]
pub(super) struct Lanes {
    pub(super) fps: f64,
    /// Where the program ends, in seconds.
    pub(super) end: f64,
    /// A first cut on an empty program, shown as the proposal's own, read only.
    pub(super) proposed: bool,
    pub(super) rows: Vec<Row>,
    pub(super) markers: Vec<Mark>,
    /// Each open proposal over the program.
    pub(super) bands: Vec<ProposalBand>,
    /// Points the lanes take, top to bottom, without the add-track row.
    pub(super) height: f32,
}

#[derive(Clone)]
pub(super) struct Row {
    pub(super) id: String,
    pub(super) kind: TrackKind,
    pub(super) label: SharedString,
    /// "V1", "A2": its kind and number.
    pub(super) badge: SharedString,
    pub(super) enabled: bool,
    pub(super) locked: bool,
    pub(super) muted: bool,
    pub(super) sync_locked: bool,
    /// Room tone: its waveforms darker.
    pub(super) room: bool,
    /// Points from the lanes' top.
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) clips: Vec<ClipBox>,
    /// Transitions, start and end in seconds.
    pub(super) transitions: Vec<(f64, f64)>,
    /// Spans of it an open proposal changes, veiled.
    pub(super) veils: Vec<(f64, f64)>,
    /// Shows a proposal playing as a preview: its clips are the proposal's,
    /// read only, and these are where the clips it changes are now.
    pub(super) previewed: bool,
    pub(super) ghosts: Vec<(f64, f64)>,
}

impl Row {
    /// Whether what is on it is quieted: switched off, or muted.
    pub(super) fn quiet(&self) -> bool {
        !self.enabled || self.muted
    }

    pub(super) fn audio(&self) -> bool {
        self.kind == TrackKind::Audio
    }
}

#[derive(Clone)]
pub(super) struct ClipBox {
    pub(super) id: String,
    pub(super) label: SharedString,
    /// What its name bar says after its name: its speed, a title's text…
    pub(super) secondary: SharedString,
    pub(super) start: f64,
    pub(super) duration: f64,
    pub(super) start_frame: i64,
    pub(super) duration_frames: i64,
    pub(super) enabled: bool,
    pub(super) kind: ClipKind,
    /// The colour along its top: what kind of clip it is.
    pub(super) edge: Rgba,
    pub(super) source: Option<String>,
    /// In the first cut's own view: the proposal adds or changes it.
    pub(super) changed: bool,
    /// An open proposal changes it: outlined in sage.
    pub(super) proposed: bool,
    /// It has effects (an fx mark), or a check found a problem in it (a dot).
    pub(super) fx: bool,
    pub(super) qc: bool,
    /// Frames its in point can go earlier, and its out point later, before
    /// its media runs out; None when nothing bounds it.
    pub(super) head: Option<i64>,
    pub(super) tail: Option<i64>,
    /// The clip as the snapshot has it: its source range, gain and signal.
    pub(super) clip: Clip,
}

impl ClipBox {
    fn new(timeline: &Timeline, track: &Track, clip: &Clip) -> Self {
        let fps = timeline.rate();
        let speed = clip.speed.unwrap_or(1.0);
        let source = clip.source_id.as_deref().and_then(|id| timeline.source(id));
        let (head, tail) = match (clip.source_in, clip.source_out, source) {
            (Some(source_in), Some(source_out), Some(source))
                if speed > 0.0 && source.duration > 0.0 =>
            {
                (
                    Some((source_in / speed * fps).floor().max(0.0) as i64),
                    Some(
                        ((source.duration - source_out) / speed * fps)
                            .floor()
                            .max(0.0) as i64,
                    ),
                )
            }
            _ => (None, None),
        };
        let kind = if clip.kind == "caption" || track.kind == TrackKind::Subtitle {
            ClipKind::Caption
        } else if clip.kind == "generator" {
            ClipKind::Generator
        } else {
            ClipKind::Media
        };
        Self {
            id: clip.id.clone(),
            label: SharedString::from(clip.label().to_owned()),
            secondary: clip.secondary().into(),
            start: clip.timeline_start,
            duration: clip.duration,
            start_frame: clip.start_frame,
            duration_frames: clip.duration_frames.max(1),
            enabled: clip.enabled,
            kind,
            edge: edge_colour(track.kind, clip),
            source: clip.source_id.clone().filter(|id| !id.is_empty()),
            changed: false,
            proposed: false,
            fx: !clip.effects.is_empty(),
            qc: false,
            head,
            tail,
            clip: clip.clone(),
        }
    }

    pub(super) fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// A title, a solid or a caption: drawn as a card, with no media.
    pub(super) fn card(&self) -> bool {
        self.kind != ClipKind::Media
    }
}

#[derive(Clone)]
pub(super) struct Mark {
    pub(super) time: f64,
    pub(super) duration: f64,
    pub(super) color: Rgba,
    pub(super) label: Option<String>,
}

/// What the lanes are built from.
pub(super) struct Build<'a> {
    pub(super) current: &'a Timeline,
    /// A first cut over an empty program: shown in its place, read only.
    pub(super) first_cut: Option<&'a Timeline>,
    pub(super) bands: Vec<ProposalBand>,
    /// A proposal playing as a preview: its id and the timeline it leaves.
    pub(super) preview: Option<(&'a str, &'a Timeline)>,
    pub(super) heights: &'a TrackHeights,
    pub(super) qc: &'a HashSet<String>,
}

impl Lanes {
    /// The lanes of the program with its open proposals over it; of a first
    /// cut alone when the program has no clips.
    pub(super) fn build(input: Build) -> Self {
        let shown = input.first_cut.unwrap_or(input.current);
        let tracks = &shown.tracks;
        let ordered = tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Subtitle)
            .chain(
                tracks
                    .iter()
                    .rev()
                    .filter(|track| track.kind == TrackKind::Video),
            )
            .chain(tracks.iter().filter(|track| track.kind == TrackKind::Audio));
        let bands = if input.first_cut.is_some() {
            Vec::new()
        } else {
            input.bands
        };
        let proposed_clips: HashSet<&str> = bands
            .iter()
            .flat_map(|band| band.clips.iter().map(String::as_str))
            .collect();
        let previewed_band = input
            .preview
            .filter(|_| input.first_cut.is_none())
            .and_then(|(id, timeline)| {
                bands
                    .iter()
                    .find(|band| band.id == id)
                    .map(|band| (band, timeline))
            });
        let mut end = shown.duration.max(0.0);
        let mut top = 0.0;
        let mut rows = Vec::with_capacity(tracks.len());
        for track in ordered {
            let height = input.heights.height(&track.id, track.kind);
            // A track the previewed proposal changes shows its clips.
            let preview_track = previewed_band
                .filter(|(band, _)| band.tracks.contains(&track.id))
                .and_then(|(_, timeline)| {
                    timeline
                        .tracks
                        .iter()
                        .find(|other| other.id == track.id)
                        .map(|other| (other, timeline))
                });
            let (from_track, from_timeline) = preview_track.unwrap_or((track, shown));
            let clips = from_track
                .clips
                .iter()
                .map(|clip| {
                    end = end.max(clip.end());
                    let mut clip_box = ClipBox::new(from_timeline, track, clip);
                    clip_box.changed =
                        input.first_cut.is_some() && differs(track, clip, input.current);
                    clip_box.proposed = proposed_clips.contains(clip.id.as_str());
                    clip_box.qc = input.qc.contains(&clip.id);
                    clip_box
                })
                .collect();
            let veils = bands
                .iter()
                .filter(|band| band.tracks.contains(&track.id))
                .map(|band| (band.start, band.end))
                .collect();
            let ghosts = previewed_band
                .map(|(band, _)| {
                    band.ghosts
                        .iter()
                        .filter(|(on, _, _)| *on == track.id)
                        .map(|(_, start, end)| (*start, *end))
                        .collect()
                })
                .unwrap_or_default();
            rows.push(Row {
                id: track.id.clone(),
                kind: track.kind,
                label: SharedString::from(track.label().to_owned()),
                badge: shown.track_badge(&track.id).into(),
                enabled: track.enabled,
                locked: track.locked,
                muted: track.muted,
                sync_locked: track.sync_locked,
                room: track.is_room_tone(),
                top,
                height,
                clips,
                transitions: from_track
                    .transitions
                    .iter()
                    .filter(|transition| transition.end > transition.start)
                    .map(|transition| (transition.start, transition.end))
                    .collect(),
                veils,
                previewed: preview_track.is_some(),
                ghosts,
            });
            top += height;
        }
        let markers = shown
            .markers
            .iter()
            .map(|marker| {
                end = end.max(marker.time + marker.duration.max(0.0));
                Mark {
                    time: marker.time,
                    duration: marker.duration.max(0.0),
                    color: theme::marker::named(marker.color.as_deref()),
                    label: marker.label.clone().filter(|label| !label.is_empty()),
                }
            })
            .collect();
        for band in &bands {
            end = end.max(band.end);
        }
        let has_clips = rows.iter().any(|row: &Row| !row.clips.is_empty());
        if !has_clips {
            end = end.max(EMPTY_SECONDS);
        }
        Self {
            fps: shown.rate(),
            end,
            proposed: input.first_cut.is_some(),
            rows,
            markers,
            bands,
            height: top,
        }
    }

    pub(super) fn has_clips(&self) -> bool {
        self.rows.iter().any(|row| !row.clips.is_empty())
    }

    /// The row and index of the clip `id`.
    pub(super) fn find(&self, id: &str) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(row, lane)| {
            lane.clips
                .iter()
                .position(|clip| clip.id == id)
                .map(|index| (row, index))
        })
    }

    /// The row at `lane_y` points from the lanes' top.
    pub(super) fn row_at(&self, lane_y: f32) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| lane_y >= row.top && lane_y < row.top + row.height)
    }

    /// The rows' heights from `heights`, laid out again top to bottom.
    pub(super) fn relayout(&mut self, heights: &TrackHeights) {
        let mut top = 0.0;
        for row in &mut self.rows {
            row.height = heights.height(&row.id, row.kind);
            row.top = top;
            top += row.height;
        }
        self.height = top;
    }

    /// Each row's id and kind, top to bottom.
    pub(super) fn kinds(&self) -> Vec<(String, TrackKind)> {
        self.rows
            .iter()
            .map(|row| (row.id.clone(), row.kind))
            .collect()
    }
}

pub(super) fn kind_colour(kind: TrackKind) -> Rgba {
    match kind {
        TrackKind::Video => theme::accent_ink(),
        TrackKind::Audio => theme::pos(),
        TrackKind::Subtitle => theme::warn(),
    }
}

/// The colour along a clip's top: generated pictures (titles, solids) in
/// the playhead's violet, else its track's kind.
fn edge_colour(kind: TrackKind, clip: &Clip) -> Rgba {
    match clip.kind.as_str() {
        "generator" => theme::playhead(),
        "caption" => theme::warn(),
        _ => kind_colour(kind),
    }
}

/// Whether a proposal's `clip` on `track` is new or other than it is in `before`.
fn differs(track: &Track, clip: &Clip, before: &Timeline) -> bool {
    let Some((was_on, was)) = before.clip(&clip.id) else {
        return true;
    };
    was_on.id != track.id
        || was.start_frame != clip.start_frame
        || was.duration_frames != clip.duration_frames
        || was.source_in != clip.source_in
        || was.source_out != clip.source_out
        || was.speed != clip.speed
        || was.enabled != clip.enabled
        || was.source_id != clip.source_id
        || was.fx != clip.fx
        || was.label() != clip.label()
        || was.effects != clip.effects
}

impl TimelineView {
    /// The version the pinned proposal's card shows (its Versions menu;
    /// `ChatView::pinned_proposal`), or None for each proposal's own:
    /// that proposal's band, veil and outlines follow it. The window calls
    /// it whenever the conversation changes.
    pub fn show_version(&mut self, version: Option<String>, cx: &mut Context<Self>) {
        if self.shown_version != version {
            self.shown_version = version;
            self.rebuild(cx);
            cx.notify();
        }
    }

    pub(super) fn rebuild(&mut self, cx: &App) {
        let session = self.session.read(cx);
        let snapshot = &session.state.snapshot;
        let current = &snapshot.timeline;
        let pin = self.shown_version.as_deref();
        let previewing = self.previewing.as_deref();
        let is_of = |proposal: &model::Proposal, id: &str| {
            proposal.proposal_id == id
                || proposal
                    .versions
                    .iter()
                    .any(|version| version.proposal_id == id)
        };
        // While Kerf builds a cut, the cut it builds shows its newest
        // version, so what it places is in the lanes as it is placed (and
        // its flight has somewhere to land) before that version's preview
        // plays. Else a proposal shows the version playing as its preview,
        // else the version its pinned card shows, else its own.
        let building = session
            .state
            .work
            .busy()
            .then(|| session.live_cut())
            .flatten()
            .map(|(proposal, _)| proposal.proposal_id.clone());
        let shown_of = |proposal: &model::Proposal| {
            shown_version(proposal, building.as_deref(), previewing, pin)
        };
        let first_cut = if current.has_clips() {
            None
        } else {
            snapshot
                .pending()
                .and_then(|pending| snapshot.timeline_of(pending, shown_of(pending)))
                .or(snapshot.proposal_timeline.as_ref())
                .filter(|timeline| timeline.has_clips())
        };
        let bands = snapshot
            .proposals
            .iter()
            .filter(|proposal| proposal.is_pending())
            .filter_map(|proposal| {
                let timeline = snapshot.timeline_of(proposal, shown_of(proposal))?;
                ProposalBand::of(
                    &proposal.proposal_id,
                    &proposal.author_name(),
                    current,
                    timeline,
                )
            })
            .collect();
        let preview = previewing.and_then(|id| {
            let proposal = snapshot
                .proposals
                .iter()
                .find(|proposal| proposal.is_pending() && is_of(proposal, id))?;
            let timeline = snapshot.timeline_of(proposal, shown_of(proposal).or(Some(id)))?;
            Some((proposal.proposal_id.as_str(), timeline))
        });
        let lanes = Lanes::build(Build {
            current,
            first_cut,
            bands,
            preview,
            heights: &self.heights,
            qc: &self.qc,
        });
        self.lanes = Rc::new(lanes);
        self.building = building;
    }
}

/// Which version of `proposal` the lanes show (None: its own): the cut Kerf
/// is `building`, while it builds one, so what it places is drawn as it is
/// placed; else the version `previewing` (playing as its preview); else the
/// one its `pinned` card shows. An id that is no version of `proposal` is
/// passed over.
pub fn shown_version<'a>(
    proposal: &model::Proposal,
    building: Option<&'a str>,
    previewing: Option<&'a str>,
    pinned: Option<&'a str>,
) -> Option<&'a str> {
    let is_of = |id: &&str| {
        proposal.proposal_id == *id
            || proposal
                .versions
                .iter()
                .any(|version| version.proposal_id == *id)
    };
    building
        .filter(is_of)
        .or(previewing.filter(is_of))
        .or(pinned.filter(is_of))
}
