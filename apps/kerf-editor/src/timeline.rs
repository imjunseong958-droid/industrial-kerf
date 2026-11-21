//! The timeline: the program's tracks and clips, its ruler and playhead.
//!
//! A strip along the top says where the playhead is (or, in ink2, where the
//! pointer skims), and holds the editing tools, snap and zoom (− slider +),
//! and, while the window shows no picture, the transport first;
//! under it the track heads stand at the left, and the ruler and the lanes
//! are drawn on one canvas from [`Lanes`], a compact copy of the snapshot's
//! timeline rebuilt only when a snapshot comes. Subtitle lanes stack on
//! top, then video with the highest track uppermost (V2 over V1), then
//! audio; under the last an add-track row. An empty sequence still shows
//! its heads and a ruler of at least 30 s.
//!
//! What a press or a drag on a clip does is the tool's ([`Tool`]):
//! Selection chooses, moves (onto another lane of its kind too) and trims;
//! Razor splits where it is pressed; Ripple trims and closes or opens the
//! gap; Roll moves the cut between two clips; Slip shows other media in the
//! same span; Slide moves a clip between its neighbours. The edit goes to
//! kerfd as one typed operation when the button comes up; until the next
//! snapshot says where the clip is, it is drawn where it was dropped, so it
//! never jumps back. A click or a drag on the ruler or on an empty lane
//! seeks, scrubbing while it drags; moving over the lanes skims.
//!
//! A sound clip carries its waveform and its volume rubber band; a picture
//! clip its filmstrip; a signal its number as a curve, which an Option-drag
//! draws anew. A right-click on a clip opens its menu, on a head the
//! track's (Sync lock, heights); a double-click on a clip of a shot made
//! from steps opens its graph.
//!
//! The program stays editable while proposals are open: each open proposal
//! has a band on the ruler ("Proposal · author"), a veil over the lanes it
//! changes, and the program's clips it changes are outlined in sage. While
//! one plays as a preview, its tracks show the clips where it puts them,
//! with dashed ghosts where the clips it moves, trims or takes out are now.
//! Only a first cut on an empty program shows as the proposal's own cut,
//! read only.
//!
//! The program's in and out marks stand on the ruler, the range between
//! them shaded. While the pointer skims the picture or the lanes a skimmer
//! stands at the time it shows, and the playhead stays where it was.
//!
//! While Kerf works, its cursor stands where it is looking (the time it
//! names, else the first clip it names, else the newest change in its live
//! proposal) and rings the clips it looks at. Clips whose media is being
//! made carry moving amber stripes (still under Reduce Motion); a making
//! waiting its turn a dashed amber outline and "Queued"; a shot not made
//! yet, or a file that is gone, a dashed hole saying "Not made" or
//! "Offline". The view asks for frames only while stripes show.

use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::model::{self, Clip, GainKey, ProposalBand, Timeline, Track, TrackKind};
use crate::motion;
use crate::session::{BridgeEvent, Session};
use crate::stage::{self, MaskTool};
use crate::store::{Change, MediaActivity};
use crate::theme::{self, layout, radius, text, TypeStyle};
use crate::ui;
use gpui::{
    anchored, canvas, deferred, div, fill, linear_color_stop, linear_gradient, pattern_slash,
    point, prelude::*, px, quad, size, AnimationExt, App, BorderStyle, Bounds, ContentMask,
    Context, Corners, CursorStyle, DismissEvent, DispatchPhase, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Font, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba, ScrollDelta, ScrollWheelEvent,
    ShapedLine, SharedString, Subscription, Task, TextRun, TouchPhase, WeakEntity, Window,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Points before time zero, so the playhead's head shows whole at the start.
const PAD: f32 = 10.0;
/// Points between what the strip holds.
const STRIP_GAP: f32 = 8.0;
/// Points before the strip's tools, past the cut's length.
const TOOLS_SPACE: f32 = 8.0;
/// Points from the strip's last control to its edge, and to its first
/// without the transport.
const STRIP_END: f32 = 6.0;
const STRIP_START: f32 = 12.0;
/// Points inside a clip's edge that take a trim.
const GRAB: f32 = 6.0;
/// Points within which a drag snaps to an edge, a marker or the playhead.
const SNAP: f32 = 6.0;
/// Points a press moves before it drags.
const DRAG_START: f32 = 3.0;
/// Points the playhead keeps from the right edge before the view pages on.
const FOLLOW_MARGIN: f32 = 24.0;
/// Points from either end of the lanes where a drag pans them.
const EDGE_PAN: f32 = 16.0;
/// The widest a frame is drawn, in points.
const MOST_FRAME: f32 = 48.0;
/// What one press of − or + zooms by.
const ZOOM_STEP: f32 = 1.5;
/// − or + held down zooms again after this, and then every
/// [`ZOOM_REPEAT`], until it is let go or the pointer leaves it (Qt's
/// QToolButton autoRepeat).
pub use crate::theme::motion::{ZOOM_REPEAT, ZOOM_REPEAT_DELAY};
/// Points between a lane's edge and its clips.
const CLIP_INSET: f32 = 2.0;
/// Points of the colour edge along a clip's top.
const CLIP_EDGE: f32 = 2.0;
/// Points between ruler labels, at the least.
const LABEL_SPACE: f32 = 84.0;
/// Points between minor ticks, at the least.
const TICK_SPACE: f32 = 7.0;
/// How long a dropped clip waits where it was dropped for kerfd's answer.
const PENDING: Duration = Duration::from_millis(1500);
/// Points of the dark bar a name stands on over a filmstrip or a waveform.
const NAME_BAR: f32 = 16.0;
/// The lowest and highest a sound clip's rubber band reads, in dB.
pub const GAIN_FLOOR: f64 = -60.0;
pub const GAIN_CEILING: f64 = 12.0;
/// Points from a gain key's diamond, and from the band's line, that take a press.
const KEY_REACH: f32 = 5.0;
const LINE_REACH: f32 = 3.0;
/// The shortest and tallest a track is, in points.
pub const TRACK_LEAST: f32 = 20.0;
pub const TRACK_MOST: f32 = 160.0;
/// The heights Option-scroll and Option-Up/Down step every track through.
pub const TRACK_STEPS: [f32; 14] = [
    20.0, 24.0, 28.0, 32.0, 36.0, 42.0, 48.0, 56.0, 64.0, 72.0, 86.0, 104.0, 128.0, 160.0,
];
/// Points either side of the line under a head that take a resize.
const GRIP: f32 = 3.0;
/// The add-track row under the last track.
const ADD_ROW: f32 = 28.0;
/// What an empty sequence's ruler spans, in seconds, so it has labels to read.
const EMPTY_SECONDS: f64 = 30.0;
/// How far a hand-drawn signal may stray from the line between two keys.
const SIGNAL_WITHIN: f64 = 0.02;
/// Points of a proposal's band along the ruler's top.
const BAND_ROW: f32 = 5.0;
/// The zoom slider's width on the strip.
const SLIDER: f32 = 72.0;
/// Points of the bar under the lanes that shows where a zoomed view is.
const SCROLLER: f32 = 5.0;
/// Pixels of a trackpad's scroll that make one step of Option-scroll.
const WHEEL_STEP: f32 = 40.0;

// ------------------------------------------------------------------- tools

/// What a press or a drag on a clip does (the registry's `tool.*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Chooses, moves and trims.
    #[default]
    Selection,
    /// Splits the clip where it is pressed.
    Razor,
    /// Trims, closing or opening the gap after.
    Ripple,
    /// Moves the cut between two clips.
    Roll,
    /// Shows other media in the same span.
    Slip,
    /// Moves a clip between its neighbours.
    Slide,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Selection,
        Tool::Razor,
        Tool::Ripple,
        Tool::Roll,
        Tool::Slip,
        Tool::Slide,
    ];

    /// Its command in the registry, whose keys choose it.
    pub fn command(self) -> &'static str {
        match self {
            Tool::Selection => "tool.selection",
            Tool::Razor => "tool.razor",
            Tool::Ripple => "tool.ripple",
            Tool::Roll => "tool.roll",
            Tool::Slip => "tool.slip",
            Tool::Slide => "tool.slide",
        }
    }

    /// The tool command `id` chooses.
    pub fn from_command(id: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.command() == id)
    }

    /// What it is called on the strip and in its tip.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Selection => "Selection",
            Tool::Razor => "Razor",
            Tool::Ripple => "Ripple Edit",
            Tool::Roll => "Rolling Edit",
            Tool::Slip => "Slip",
            Tool::Slide => "Slide",
        }
    }

    /// A clip's edges take a drag.
    pub fn trims_edges(self) -> bool {
        matches!(self, Tool::Selection | Tool::Ripple | Tool::Roll)
    }

    /// A clip's body takes a drag.
    pub fn drags_bodies(self) -> bool {
        matches!(self, Tool::Selection | Tool::Slip | Tool::Slide)
    }

    /// The pointer over a clip, on one of its edges or not (Qt's
    /// `setToolCursor`).
    pub fn cursor(self, edge: bool) -> CursorStyle {
        if self == Tool::Razor {
            CursorStyle::Crosshair
        } else if edge && self.trims_edges() {
            if self == Tool::Roll {
                CursorStyle::ResizeColumn
            } else {
                CursorStyle::ResizeLeftRight
            }
        } else if self.drags_bodies() {
            if self == Tool::Selection {
                CursorStyle::OpenHand
            } else {
                CursorStyle::ResizeLeftRight
            }
        } else {
            CursorStyle::Arrow
        }
    }
}

/// What the timeline tells the window, which subscribes to it: the
/// timeline cannot reach the picture, the session's command context or
/// the project's settings itself.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEvent {
    /// The pointer skims the lanes at this time (a whole frame's), or has
    /// stopped skimming (None, also on a press or as playback starts). The
    /// window has the picture show that frame without moving the playhead,
    /// as skimming the picture itself does, and show the playhead's again
    /// on None.
    Skim(Option<f64>),
    /// The tracks a source's picture and sound go on now (Insert,
    /// Overwrite), as the person targeted them, at most one of each kind:
    /// the window keeps them for `Session::command_context`'s
    /// `sourceTargets`, which [`model::source_targets`] works out from
    /// them. None follows the first unlocked track of each kind.
    Targets(Option<Vec<String>>),
    /// The tracks' heights changed: the window keeps them for the project
    /// (with the editor's settings, by its path) and gives them back with
    /// [`TimelineView::set_track_heights`] when it opens.
    Heights(TrackHeights),
}

// ---------------------------------------------------------- track heights

/// How tall each track is drawn: one height for every track when set,
/// a track's own over that, else each kind's own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackHeights {
    pub all: Option<f32>,
    pub tracks: BTreeMap<String, f32>,
}

impl TrackHeights {
    /// The height of track `id` of `kind`.
    pub fn height(&self, id: &str, kind: TrackKind) -> f32 {
        self.tracks
            .get(id)
            .copied()
            .or(self.all)
            .unwrap_or_else(|| default_height(kind))
            .clamp(TRACK_LEAST, TRACK_MOST)
    }

    /// Track `id` to `height`, or back to the height of the rest (None).
    pub fn set(&mut self, id: &str, height: Option<f32>) {
        match height {
            Some(height) => {
                self.tracks
                    .insert(id.to_owned(), height.clamp(TRACK_LEAST, TRACK_MOST));
            }
            None => {
                self.tracks.remove(id);
            }
        }
    }

    /// Every track to `height`, none of its own; None gives each kind its own.
    pub fn set_all(&mut self, height: Option<f32>) {
        self.tracks.clear();
        self.all = height.map(|height| height.clamp(TRACK_LEAST, TRACK_MOST));
    }

    /// Every track of `rows` (id and kind) `steps` along [`TRACK_STEPS`],
    /// each from its own height. Tracks added later take the height a
    /// picture track without one of its own steps to; only the tracks that
    /// end up apart from it keep one of their own.
    pub fn step(&mut self, rows: &[(String, TrackKind)], steps: i32) {
        if steps == 0 || rows.is_empty() {
            return;
        }
        let all = stepped_height(
            self.all.unwrap_or_else(|| default_height(TrackKind::Video)),
            steps,
        );
        let tracks = rows
            .iter()
            .map(|(id, kind)| (id.clone(), stepped_height(self.height(id, *kind), steps)))
            .filter(|(_, height)| (height - all).abs() > 0.01)
            .collect();
        self.all = Some(all);
        self.tracks = tracks;
    }
}

/// A track's height when nothing else is set: one roomy height for every
/// kind ([`layout::TRACK_LANE`]), none larger than another for what it
/// is, as the Qt editor's window drew them (Medium and Reset go back to it).
pub fn default_height(_kind: TrackKind) -> f32 {
    layout::TRACK_LANE
}

/// `height` moved `steps` along [`TRACK_STEPS`] (up when positive), from
/// the next height of the ladder on that side; held at its ends.
pub fn stepped_height(height: f32, steps: i32) -> f32 {
    let mut height = height.clamp(TRACK_LEAST, TRACK_MOST);
    for _ in 0..steps.max(0) {
        match TRACK_STEPS.iter().find(|step| **step > height + 0.01) {
            Some(next) => height = *next,
            None => break,
        }
    }
    for _ in 0..(-steps).max(0) {
        match TRACK_STEPS.iter().rev().find(|step| **step < height - 0.01) {
            Some(next) => height = *next,
            None => break,
        }
    }
    height
}

// ------------------------------------------------------- watching changes

/// The clips a change reel watches (Qt's watchChanges, as it gave them to
/// setFocusClips): every clip proposal `id` (or the version of it `id`
/// names, as the reel plays it) adds, takes out or changes against the
/// program, as its card lists them. None when the snapshot has no timeline
/// of it.
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
            crate::chat::clip_changes(&snapshot.timeline, proposed)
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

/// A clip `amount` of the way into a change reel (0 at rest, 1 watching),
/// as Qt's timeline drew it: one the reel `watched` rises `CLIP_LIFT` off
/// its lane over a soft glow, ringed in the accent's ink (a sound clip
/// stays on its lane, ringed); every other clip falls back under a cover.
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
            // Other clips rise from their lanes anew, as Qt's setFocusClips.
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
    fn shown_at(&self, now: Instant) -> Option<(Rc<HashSet<String>>, f32)> {
        let amount = self.amount_at(now);
        (amount > 0.0 && !self.clips.is_empty()).then(|| (self.clips.clone(), amount))
    }
}

/// Where a sound clip's rubber band stands for `db` in a clip `tall`
/// points high from `top` (Qt's `gainY`): −60 dB at its foot, +12 at its
/// head, 2 points in from either.
pub fn gain_y(db: f64, top: f32, tall: f32) -> f32 {
    let fraction =
        ((db.clamp(GAIN_FLOOR, GAIN_CEILING) - GAIN_FLOOR) / (GAIN_CEILING - GAIN_FLOOR)) as f32;
    top + 2.0 + (1.0 - fraction) * (tall - 4.0).max(1.0)
}

/// The gain the band reads at `y` in such a clip, to a tenth of a dB.
pub fn gain_at(y: f32, top: f32, tall: f32) -> f64 {
    let fraction = 1.0 - (y - top - 2.0) / (tall - 4.0).max(1.0);
    let db = (GAIN_FLOOR + fraction as f64 * (GAIN_CEILING - GAIN_FLOOR))
        .clamp(GAIN_FLOOR, GAIN_CEILING);
    (db * 10.0).round() / 10.0
}

/// Where the zoom slider stands, 0..1, for `pps` points a second between
/// the fit and the most: on a log scale, so each step zooms alike.
pub fn zoom_share(pps: f32, fit: f32, most: f32) -> f32 {
    if most <= fit * 1.001 || pps <= fit {
        return 0.0;
    }
    ((pps / fit).ln() / (most / fit).ln()).clamp(0.0, 1.0)
}

/// The points a second the slider at `share` gives; within a 50th of its
/// travel from the start it holds at the fit (the detent).
pub fn zoom_of_share(share: f32, fit: f32, most: f32) -> f32 {
    if share <= 0.02 || most <= fit {
        return fit;
    }
    fit * (most / fit).powf(share.clamp(0.0, 1.0))
}

// ------------------------------------------------------------------ lanes

/// What kind of clip a clip is, as the timeline draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipKind {
    Media,
    /// A generated picture: a title, a solid, a signal.
    Generator,
    Caption,
}

/// The timeline as it is drawn: its tracks in the order they stack, with
/// what each clip needs to be drawn, hit and dragged.
#[derive(Clone, Default)]
struct Lanes {
    fps: f64,
    /// Where the program ends, in seconds.
    end: f64,
    /// A first cut on an empty program, shown as the proposal's own, read only.
    proposed: bool,
    rows: Vec<Row>,
    markers: Vec<Mark>,
    /// Each open proposal over the program.
    bands: Vec<ProposalBand>,
    /// Points the lanes take, top to bottom, without the add-track row.
    height: f32,
}

#[derive(Clone)]
struct Row {
    id: String,
    kind: TrackKind,
    label: SharedString,
    /// "V1", "A2": its kind and number.
    badge: SharedString,
    enabled: bool,
    locked: bool,
    muted: bool,
    sync_locked: bool,
    /// Room tone: its waveforms darker.
    room: bool,
    /// Points from the lanes' top.
    top: f32,
    height: f32,
    clips: Vec<ClipBox>,
    /// Transitions, start and end in seconds.
    transitions: Vec<(f64, f64)>,
    /// Spans of it an open proposal changes, veiled.
    veils: Vec<(f64, f64)>,
    /// Shows a proposal playing as a preview: its clips are the proposal's,
    /// read only, and these are where the clips it changes are now.
    previewed: bool,
    ghosts: Vec<(f64, f64)>,
}

impl Row {
    /// Whether what is on it is quieted: switched off, or muted.
    fn quiet(&self) -> bool {
        !self.enabled || self.muted
    }

    fn audio(&self) -> bool {
        self.kind == TrackKind::Audio
    }
}

#[derive(Clone)]
struct ClipBox {
    id: String,
    label: SharedString,
    /// What its name bar says after its name: its speed, a title's text…
    secondary: SharedString,
    start: f64,
    duration: f64,
    start_frame: i64,
    duration_frames: i64,
    enabled: bool,
    kind: ClipKind,
    /// The colour along its top: what kind of clip it is.
    edge: Rgba,
    source: Option<String>,
    /// In the first cut's own view: the proposal adds or changes it.
    changed: bool,
    /// An open proposal changes it: outlined in sage.
    proposed: bool,
    /// It has effects (an fx mark), or a check found a problem in it (a dot).
    fx: bool,
    qc: bool,
    /// Frames its in point can go earlier, and its out point later, before
    /// its media runs out; None when nothing bounds it.
    head: Option<i64>,
    tail: Option<i64>,
    /// The clip as the snapshot has it: its source range, gain and signal.
    clip: Clip,
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

    fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// A title, a solid or a caption: drawn as a card, with no media.
    fn card(&self) -> bool {
        self.kind != ClipKind::Media
    }
}

#[derive(Clone)]
struct Mark {
    time: f64,
    duration: f64,
    color: Rgba,
    label: Option<String>,
}

/// What the lanes are built from.
struct Build<'a> {
    current: &'a Timeline,
    /// A first cut over an empty program: shown in its place, read only.
    first_cut: Option<&'a Timeline>,
    bands: Vec<ProposalBand>,
    /// A proposal playing as a preview: its id and the timeline it leaves.
    preview: Option<(&'a str, &'a Timeline)>,
    heights: &'a TrackHeights,
    qc: &'a HashSet<String>,
}

impl Lanes {
    /// The lanes of the program with its open proposals over it; of a first
    /// cut alone when the program has no clips.
    fn build(input: Build) -> Self {
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

    fn has_clips(&self) -> bool {
        self.rows.iter().any(|row| !row.clips.is_empty())
    }

    /// The row and index of the clip `id`.
    fn find(&self, id: &str) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(row, lane)| {
            lane.clips
                .iter()
                .position(|clip| clip.id == id)
                .map(|index| (row, index))
        })
    }

    /// The row at `lane_y` points from the lanes' top.
    fn row_at(&self, lane_y: f32) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| lane_y >= row.top && lane_y < row.top + row.height)
    }

    /// The rows' heights from `heights`, laid out again top to bottom.
    fn relayout(&mut self, heights: &TrackHeights) {
        let mut top = 0.0;
        for row in &mut self.rows {
            row.height = heights.height(&row.id, row.kind);
            row.top = top;
            top += row.height;
        }
        self.height = top;
    }

    /// Each row's id and kind, top to bottom.
    fn kinds(&self) -> Vec<(String, TrackKind)> {
        self.rows
            .iter()
            .map(|row| (row.id.clone(), row.kind))
            .collect()
    }
}

fn kind_colour(kind: TrackKind) -> Rgba {
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

/// What a lane draws over its clips: Qt's waveform and room-tone greys
/// (`theme::waveform`), the rubber band and its keys and the bar a name
/// stands on (`theme::lane`).
mod lane_ink {
    use crate::theme::{lane, waveform};
    use gpui::{rgba, Rgba};
    pub fn peak(room: bool) -> Rgba {
        if room {
            waveform::room_peak()
        } else {
            waveform::peak()
        }
    }
    pub fn rms(room: bool) -> Rgba {
        if room {
            waveform::room_rms()
        } else {
            waveform::rms()
        }
    }
    pub use lane::{keyframe as key, name_bar, rubber_band};
    /// What shades a waveform's name, so it reads over the wave.
    pub fn shadow() -> Rgba {
        rgba(0x000000b3)
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

// --------------------------------------------------------------- viewport

/// Where the lanes look: their size, the zoom, and how far they are scrolled.
#[derive(Clone, Copy, Debug, Default)]
struct Viewport {
    width: f32,
    height: f32,
    /// Points a second.
    pps: f32,
    /// The time at the left edge, in seconds.
    left: f64,
    scroll_y: f32,
}

impl Viewport {
    fn x(&self, time: f64) -> f32 {
        PAD + ((time - self.left) * self.pps as f64) as f32
    }

    fn time(&self, x: f32) -> f64 {
        self.left + ((x - PAD) / self.pps.max(f32::EPSILON)) as f64
    }

    /// Seconds the lanes show across.
    fn seconds(&self) -> f64 {
        ((self.width - PAD).max(0.0) / self.pps.max(f32::EPSILON)) as f64
    }

    /// Points the lanes show down, under the ruler.
    fn lanes_height(&self) -> f32 {
        (self.height - layout::RULER).max(0.0)
    }

    /// Where `row` is drawn, from the canvas's top.
    fn row_y(&self, row: &Row) -> f32 {
        layout::RULER + row.top - self.scroll_y
    }
}

/// Points a second that show `end` seconds across `width`.
fn fit_pps(width: f32, end: f64) -> f32 {
    (width - 2.0 * PAD).max(1.0) / end.max(1.0) as f32
}

/// The latest the left edge goes: the program's end halfway across.
fn max_left(view: &Viewport, end: f64) -> f64 {
    (end - view.seconds() * 0.5).max(0.0)
}

/// The span a zoomed view's scroll bar stands for, in seconds: the
/// program and the half view after it that the left edge can reach.
fn scroll_span(view: &Viewport, end: f64) -> f64 {
    (max_left(view, end) + view.seconds())
        .max(view.seconds())
        .max(1e-6)
}

/// The scroll bar's thumb, left and width in points across the lanes.
fn thumb(view: &Viewport, end: f64) -> (f32, f32) {
    let span = scroll_span(view, end);
    let track = (view.width - 2.0 * PAD).max(1.0);
    let width = ((view.seconds() / span) as f32 * track).clamp(24.0_f32.min(track), track);
    let left = PAD + (view.left / span) as f32 * track;
    (left.min(PAD + track - width), width)
}

// ---------------------------------------------------------------- gestures

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Body,
    In,
    Out,
}

enum Hit {
    Ruler,
    Clip {
        row: usize,
        index: usize,
        zone: Zone,
    },
    Lane,
}

#[derive(Clone, Debug, PartialEq)]
struct Hover {
    clip: String,
    zone: Zone,
    /// On the rubber band: its line, or a key.
    band: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Move,
    TrimIn,
    TrimOut,
}

enum Gesture {
    Scrub,
    Clip(ClipDrag),
    Band(BandDrag),
    Signal(SignalDraw),
    Resize(ResizeDrag),
    /// The zoom slider's knob, dragged.
    Slider,
    /// The scroll bar's thumb, held this far from its left.
    Thumb(f32),
}

/// A clip being dragged: moved, slipped or slid by its body, or trimmed,
/// rippled or rolled by an edge.
struct ClipDrag {
    id: String,
    edit: Edit,
    tool: Tool,
    /// The row it is on, and the row it would land on.
    row: usize,
    target: usize,
    press_time: f64,
    /// Where it was pressed: down the lanes, and down the window. It
    /// follows the pointer by how far it has moved in the window, so a
    /// layout that shifts while it drags (the Inspector coming in) does
    /// not move it.
    press_y: f32,
    press_window: f32,
    moved: bool,
    start_frame: i64,
    duration_frames: i64,
    /// The amount, in frames: of the move, or of the edge.
    frames: i64,
    least: i64,
    most: i64,
    /// What it snaps to, in seconds, and what it snapped to last.
    targets: Vec<f64>,
    snapped: Option<f64>,
}

impl ClipDrag {
    // A drag starts from what its press found: the clip, where and when, the tool, the playhead.
    #[allow(clippy::too_many_arguments)]
    fn new(
        lanes: &Lanes,
        row: usize,
        index: usize,
        zone: Zone,
        tool: Tool,
        press_time: f64,
        press_y: f32,
        playhead: f64,
    ) -> Self {
        let clip = &lanes.rows[row].clips[index];
        let edit = match zone {
            Zone::Body => Edit::Move,
            Zone::In => Edit::TrimIn,
            Zone::Out => Edit::TrimOut,
        };
        // What it can snap to, gathered once rather than on every move.
        let mut targets = vec![0.0, playhead];
        for other in lanes.rows.iter().flat_map(|row| &row.clips) {
            if other.id != clip.id {
                targets.push(other.start);
                targets.push(other.end());
            }
        }
        for mark in &lanes.markers {
            targets.push(mark.time);
            if mark.duration > 0.0 {
                targets.push(mark.time + mark.duration);
            }
        }
        // Never before the sequence, never to no frames, never past its media.
        let (least, most) = match edit {
            // A slip shows other media in the same span: as far as its
            // media reaches either way (later media is a drag to the left).
            Edit::Move if tool == Tool::Slip => (
                -clip.tail.unwrap_or(i64::MAX),
                clip.head.unwrap_or(i64::MAX),
            ),
            Edit::Move => (-clip.start_frame, i64::MAX),
            Edit::TrimIn => (
                -clip.head.unwrap_or(i64::MAX).min(clip.start_frame),
                clip.duration_frames - 1,
            ),
            Edit::TrimOut => (-(clip.duration_frames - 1), clip.tail.unwrap_or(i64::MAX)),
        };
        Self {
            id: clip.id.clone(),
            edit,
            tool,
            row,
            target: row,
            press_time,
            press_y,
            press_window: 0.0,
            moved: false,
            start_frame: clip.start_frame,
            duration_frames: clip.duration_frames,
            frames: 0,
            least,
            most: most.max(least),
            targets,
            snapped: None,
        }
    }

    /// Follows the pointer at `x` across the canvas, `dy` points down the
    /// window from where it was pressed.
    fn follow(&mut self, lanes: &Lanes, view: &Viewport, x: f32, dy: f32, snap: bool) {
        let fps = lanes.fps;
        let time = view.time(x);
        let delta = time - self.press_time;
        if (delta * view.pps as f64).abs() >= DRAG_START as f64 || dy.abs() >= DRAG_START {
            self.moved = true;
        }
        if !self.moved {
            return;
        }
        let start = self.start_frame as f64 / fps;
        let end = (self.start_frame + self.duration_frames) as f64 / fps;
        let threshold = (SNAP / view.pps.max(f32::EPSILON)) as f64;
        // A slip leaves the clip where it is: nothing to snap.
        let snap = snap && self.tool != Tool::Slip;
        self.snapped = None;
        let frames = match self.edit {
            Edit::Move => {
                let mut at = start + delta;
                if snap {
                    if let Some((target, shift)) =
                        nearest(&self.targets, &[at, at + (end - start)], threshold)
                    {
                        at += shift;
                        self.snapped = Some(target);
                    }
                }
                (at * fps).round() as i64 - self.start_frame
            }
            Edit::TrimIn | Edit::TrimOut => {
                let edge = if self.edit == Edit::TrimIn {
                    start
                } else {
                    end
                };
                let mut at = edge + delta;
                if snap {
                    if let Some((target, shift)) = nearest(&self.targets, &[at], threshold) {
                        at += shift;
                        self.snapped = Some(target);
                    }
                }
                (at * fps).round() as i64 - (edge * fps).round() as i64
            }
        };
        self.frames = frames.clamp(self.least, self.most);
        if self.edit == Edit::Move && self.tool == Tool::Selection {
            // Onto another lane of its kind, unless that one is locked.
            let kind = lanes.rows[self.row].kind;
            self.target = lanes
                .row_at(self.press_y + dy)
                .filter(|&row| lanes.rows[row].kind == kind && !lanes.rows[row].locked)
                .unwrap_or(self.row);
        }
    }

    /// Where it is drawn now: start and length, in seconds. A slip leaves
    /// it in place.
    fn span(&self, fps: f64) -> (f64, f64) {
        let (start, length) = match self.edit {
            Edit::Move if self.tool == Tool::Slip => (self.start_frame, self.duration_frames),
            Edit::Move => (self.start_frame + self.frames, self.duration_frames),
            Edit::TrimIn => (
                self.start_frame + self.frames,
                self.duration_frames - self.frames,
            ),
            Edit::TrimOut => (self.start_frame, self.duration_frames + self.frames),
        };
        (start as f64 / fps, length.max(1) as f64 / fps)
    }

    /// The time of the edge it moves, and how far it has moved, in seconds:
    /// what the readout by it says.
    fn readout(&self, fps: f64) -> (f64, f64) {
        let delta = self.frames as f64 / fps;
        let edge = match self.edit {
            Edit::Move | Edit::TrimIn => self.start_frame as f64 / fps + delta,
            Edit::TrimOut => (self.start_frame + self.duration_frames) as f64 / fps + delta,
        };
        (edge, delta)
    }

    /// The edit it makes, as kerfd takes it; None when it moved nothing.
    /// A roll's leading edge is the cut after the clip before it, found
    /// in `lanes`; with none there it says why.
    fn op(&self, lanes: &Lanes) -> Result<Option<Value>, &'static str> {
        let other_track = self.edit == Edit::Move && self.target != self.row;
        if !self.moved || (self.frames == 0 && !other_track) {
            return Ok(None);
        }
        let id = &self.id;
        let frames = self.frames;
        let edge = if self.edit == Edit::TrimIn {
            "in"
        } else {
            "out"
        };
        Ok(Some(match (self.tool, self.edit) {
            (Tool::Slip, Edit::Move) => json!({"op": "slip", "clip": id, "frames": -frames}),
            (Tool::Slide, Edit::Move) => json!({"op": "slide", "clip": id, "frames": frames}),
            (_, Edit::Move) => {
                let mut op = json!({"op": "move", "clip": id, "frames": frames});
                if other_track {
                    op["track"] = json!(lanes.rows[self.target].id);
                }
                op
            }
            (Tool::Roll, Edit::TrimOut) => json!({"op": "roll", "clip": id, "frames": frames}),
            (Tool::Roll, Edit::TrimIn) => {
                let before = lanes.rows[self.row]
                    .clips
                    .iter()
                    .find(|clip| clip.start_frame + clip.duration_frames == self.start_frame)
                    .ok_or("A roll needs a clip on the other side of the cut")?;
                json!({"op": "roll", "clip": before.id, "frames": frames})
            }
            // A positive amount moves the edge later: the in point shortens, the out point lengthens.
            (tool, _) => json!({
                "op": "trim",
                "clip": id,
                "edge": edge,
                "frames": frames,
                "ripple": tool == Tool::Ripple,
            }),
        }))
    }
}

/// The target nearest one of `points` within `threshold`, and how far to
/// shift to meet it.
fn nearest(targets: &[f64], points: &[f64], threshold: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for &point in points {
        for &target in targets {
            let shift = target - point;
            if shift.abs() <= threshold && best.is_none_or(|(_, known)| shift.abs() < known.abs()) {
                best = Some((target, shift));
            }
        }
    }
    best
}

/// A sound clip's rubber band being dragged: the whole line (a level, or
/// every key), or one key.
struct BandDrag {
    clip: String,
    /// The keys as they will be written; one level when not keyed.
    keys: Vec<GainKey>,
    keyed: bool,
    key: Option<usize>,
    /// The press's height in the window.
    press_y: f32,
    /// The clip's box as drawn at the press: top and height on the canvas.
    top: f32,
    tall: f32,
    from: Vec<f64>,
    values: Vec<f64>,
    moved: bool,
}

impl BandDrag {
    fn follow(&mut self, y: f32) {
        let dy = y - self.press_y;
        for (index, value) in self.values.iter_mut().enumerate() {
            if self.key.is_some_and(|key| key != index) {
                continue;
            }
            *value = gain_at(
                gain_y(self.from[index], self.top, self.tall) + dy,
                self.top,
                self.tall,
            );
        }
        self.moved = self.moved || dy.abs() >= 1.0;
    }

    /// The band as it reads now: its keys (a key Option added among them)
    /// with the values dragged to.
    fn shown(&self) -> GainShown {
        GainShown {
            clip: self.clip.clone(),
            keys: self.keys.clone(),
            values: self.values.clone(),
            keyed: self.keyed,
        }
    }

    /// What its release sets: the level, or the keys with their new values.
    fn gain(&self) -> Value {
        if !self.keyed {
            return json!(self.values.first().copied().unwrap_or(0.0));
        }
        let keys: Vec<Value> = self
            .keys
            .iter()
            .zip(&self.values)
            .map(|(key, value)| json!({"at": key.frame, "value": value, "interp": key.interp}))
            .collect();
        json!({ "keys": keys })
    }
}

/// A sound clip's gain drawn other than as the snapshot has it: while its
/// band is dragged, and once let go until kerfd answers. Its own keys, as
/// a key added by Option-click is among them before the project has it.
#[derive(Clone, Debug, PartialEq)]
struct GainShown {
    clip: String,
    keys: Vec<GainKey>,
    values: Vec<f64>,
    keyed: bool,
}

/// Clip `clip`'s gain as it is drawn: `over` it when that is its own,
/// else the snapshot's keys and their values; and whether they are keys.
fn gain_drawn(
    clip: &ClipBox,
    fps: f64,
    over: Option<&GainShown>,
) -> (Vec<GainKey>, Vec<f64>, bool) {
    match over.filter(|over| over.clip == clip.id) {
        Some(over) => (over.keys.clone(), over.values.clone(), over.keyed),
        None => {
            let (keys, keyed) = clip.clip.gain_keys(fps);
            let values = keys.iter().map(|key| key.value).collect();
            (keys, values, keyed)
        }
    }
}

/// A signal being drawn by hand across its clip: a value 0..1 at each frame.
/// It reads the pointer against the clip's box as it is drawn at each move,
/// never as it was at the press: choosing the clip brings the Inspector in,
/// which moves the timeline while the drag goes on.
struct SignalDraw {
    clip: String,
    frames: i64,
    drawn: BTreeMap<i64, f64>,
    last: Option<i64>,
}

/// The frame of a signal clip `frames` long, and the value 0..1, that the
/// pointer at `x`, `y` draws over the clip's box `(left, top, width,
/// height)` (its curve stands 2 points in from its top and its foot, as
/// it is drawn); None when the box has no width.
pub fn signal_at(
    x: f32,
    y: f32,
    (left, top, across, tall): (f32, f32, f32, f32),
    frames: i64,
) -> Option<(i64, f64)> {
    if across <= 0.0 {
        return None;
    }
    let (top, tall) = (top + 2.0, (tall - 4.0).max(1.0));
    let last = (frames - 1).max(0);
    let frame = (((x - left) / across) as f64 * last as f64)
        .round()
        .clamp(0.0, last as f64) as i64;
    let value = (1.0 - ((y - top) / tall) as f64).clamp(0.0, 1.0);
    Some((frame, value))
}

impl SignalDraw {
    /// The pointer at `x`, `y` over the clip's box `rect` as drawn now:
    /// every frame it passed since the last is on the line between the two.
    fn draw_to(&mut self, x: f32, y: f32, rect: (f32, f32, f32, f32)) {
        let Some((frame, value)) = signal_at(x, y, rect, self.frames) else {
            return;
        };
        if let Some(from) = self.last.filter(|from| *from != frame) {
            let start = self.drawn.get(&from).copied().unwrap_or(value);
            let step = if frame > from { 1 } else { -1 };
            let mut at = from + step;
            while at != frame {
                let share = (at - from) as f64 / (frame - from) as f64;
                self.drawn.insert(at, start + (value - start) * share);
                at += step;
            }
        }
        self.drawn.insert(frame, value);
        self.last = Some(frame);
    }
}

/// The line under a head being dragged: one track's height, or every track
/// of its kind (Option).
struct ResizeDrag {
    track: String,
    kind: TrackKind,
    every: bool,
    /// The pointer's window y at the press, and the track's height then.
    anchor: f32,
    from: f32,
}

/// A clip drawn somewhere other than where the snapshot has it.
#[derive(Clone, Debug)]
struct Ghost {
    id: String,
    row: usize,
    start: f64,
    duration: f64,
    /// The edge being trimmed.
    edge: Option<Zone>,
    /// Still being dragged: where it was shows as an outline.
    dragging: bool,
    /// Its media's head moves with a leading trim: seconds of it.
    head_shift: f64,
}

/// A dropped clip, until kerfd answers.
struct Pending {
    ghost: Ghost,
    revision: u64,
    at: Instant,
}

/// What a media's clips say it is doing, when it is doing more than
/// standing ready.
#[derive(Clone)]
struct Activity {
    media: String,
    state: MediaActivity,
    /// The stripes' phase, while it is being made.
    phase: f32,
    label: SharedString,
}

impl Activity {
    fn making(&self) -> bool {
        matches!(self.state, MediaActivity::Making { .. })
    }

    /// Dashed: waiting its turn, or with no file to show.
    fn dashed(&self) -> bool {
        self.state.absent() || matches!(self.state, MediaActivity::Queued { .. })
    }
}

/// What a clip says of its media's making, holding still or not: "Making
/// take 2 · 0:41", "Queued · 2 ahead", "Not made", "Offline".
pub fn activity_label(activity: &MediaActivity) -> Option<String> {
    Some(match activity {
        MediaActivity::Ready => return None,
        MediaActivity::Making { take, since } => {
            let elapsed = since.elapsed().as_secs();
            let clock = format!("{}:{:02}", elapsed / 60, elapsed % 60);
            if *take > 1 {
                format!("Making take {take} · {clock}")
            } else {
                format!("Making · {clock}")
            }
        }
        MediaActivity::Queued { ahead: Some(ahead) } if *ahead > 0 => {
            format!("Queued · {ahead} ahead")
        }
        MediaActivity::Queued { .. } => "Queued".into(),
        MediaActivity::NotMade => "Not made".into(),
        MediaActivity::Offline => "Offline".into(),
    })
}

/// A menu open over the timeline, where it opened.
struct OpenMenu {
    menu: Entity<ui::Menu>,
    at: Point<Pixels>,
    _dismiss: Subscription,
}

/// What the pointer rests on, for its tip: where it is on the canvas.
#[derive(Clone, Debug, PartialEq)]
struct CanvasTip {
    key: String,
    tip: ui::Tip,
    bounds: (f32, f32, f32, f32),
}

// -------------------------------------------------------------------- view

pub struct TimelineView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    lanes: Rc<Lanes>,
    /// Points a second; None fits the program to the width.
    zoom: Option<f32>,
    /// The time at the lanes' left edge, in seconds.
    left: f64,
    scroll_y: f32,
    snap: bool,
    tool: Tool,
    /// The lanes as last drawn: what a press hits.
    view: Viewport,
    /// Where the canvas was last laid out, in the window.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where the zoom slider was last laid out, in the window.
    slider: Rc<Cell<Bounds<Pixels>>>,
    gesture: Option<Gesture>,
    hover: Option<Hover>,
    pending: Option<Pending>,
    /// The playhead as last drawn, to follow it when it moves.
    last_position: f64,
    /// Ticks a making's clock once a second while one runs.
    clock: Option<Task<()>>,
    /// − or + held down: zooming again while it is ([`ZOOM_REPEAT`]).
    zoom_held: Option<Task<()>>,
    heights: TrackHeights,
    /// The tracks a source's picture and sound go on; None: the first
    /// unlocked track of each kind.
    targets: Option<Vec<String>>,
    /// The sound tracks soloed, for this session's monitoring only.
    solo: Vec<String>,
    /// Clips the last finished check found a warning or an error in.
    qc: HashSet<String>,
    /// The proposal playing as a preview, as the preview's events say.
    previewing: Option<String>,
    /// The cut Kerf builds, while it does, as the lanes were last built:
    /// they show its newest version.
    building: Option<String>,
    /// The version of its proposal the pinned card shows, as the window
    /// last said ([`TimelineView::show_version`]).
    shown_version: Option<String>,
    /// Where Kerf's cursor stands while it works, and the changes of its
    /// live proposal it has stood on.
    kerf_at: Option<f64>,
    live_changes: HashSet<String>,
    /// The clips a change reel watches, risen off their lanes while it plays.
    watch: ClipWatch,
    /// Where the pointer skims the lanes.
    skim: Option<f64>,
    /// A signal drawn, or a band's values let go, shown until kerfd answers.
    drawn: Option<(String, BTreeMap<i64, f64>, u64)>,
    held_gain: Option<(GainShown, u64)>,
    menu: Option<OpenMenu>,
    tip: Option<CanvasTip>,
    /// Each command's keys in the keymap in use, for tips and menus.
    keys: (String, HashMap<String, Vec<String>>),
    /// Option-scroll over the heads, gathering a trackpad's small steps.
    wheel: f32,
    /// The pointer over the add-track row.
    add_hover: bool,
    /// Why each of the strip's transport controls cannot run, for the
    /// command context it was asked of (the playhead left out).
    transport_asked: Option<(Value, HashMap<&'static str, String>)>,
    /// Whether the strip carried the transport when last drawn, and how
    /// many times it has come: each time it fades in anew.
    strip_transport: (bool, u64),
    /// What the strip showed when last drawn, and how wide it was laid out.
    strip_fit: StripFit,
    strip_widths: StripWidths,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TimelineEvent> for TimelineView {}

impl TimelineView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                this.session_changed(change, cx)
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
        ];
        let mut view = Self {
            session,
            focus_handle: cx.focus_handle(),
            lanes: Rc::default(),
            zoom: None,
            left: 0.0,
            scroll_y: 0.0,
            snap: true,
            tool: Tool::Selection,
            view: Viewport::default(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            slider: Rc::new(Cell::new(Bounds::default())),
            gesture: None,
            hover: None,
            pending: None,
            last_position: 0.0,
            clock: None,
            zoom_held: None,
            heights: TrackHeights::default(),
            targets: None,
            solo: Vec::new(),
            qc: HashSet::new(),
            previewing: None,
            building: None,
            shown_version: None,
            kerf_at: None,
            live_changes: HashSet::new(),
            watch: ClipWatch::default(),
            skim: None,
            drawn: None,
            held_gain: None,
            menu: None,
            tip: None,
            keys: (String::new(), HashMap::new()),
            wheel: 0.0,
            add_hover: false,
            transport_asked: None,
            strip_transport: (false, 0),
            strip_fit: StripFit::ALL,
            strip_widths: StripWidths::default(),
            _subscriptions: subscriptions,
        };
        view.rebuild(cx);
        view.tick_makings(cx);
        view
    }

    // ------------------------------------------------------------- the api

    /// Gives the timeline the keys: Space, Home and End, the arrows, S,
    /// Delete, M and the tool keys work at once. The window's
    /// `settle_focus` calls it when the panel shows the timeline: on first
    /// activation, when a sheet or the Media band closes, and when nothing
    /// else has the keys.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    /// The tool a press or a drag on a clip uses.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Chooses the tool (the registry's `tool.*`: its keys while the
    /// timeline has them, and the window's menus and palette).
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.tool != tool {
            self.tool = tool;
            self.hover = None;
            cx.notify();
        }
    }

    /// Whether a drag snaps to clip edges, the playhead and markers
    /// (Sequence ▸ Snap, the strip's Snap).
    pub fn snapping(&self) -> bool {
        self.snap
    }

    /// Turns snapping on or off (`sequence.snap`).
    pub fn set_snapping(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.snap != on {
            self.snap = on;
            cx.notify();
        }
    }

    /// How tall each track is, as the person left them.
    pub fn track_heights(&self) -> &TrackHeights {
        &self.heights
    }

    /// The heights kept for the project (with its settings), as it opens.
    pub fn set_track_heights(&mut self, heights: TrackHeights, cx: &mut Context<Self>) {
        if self.heights != heights {
            self.heights = heights;
            self.relayout(cx);
        }
    }

    /// View ▸ Fit Tracks to Panel: every track one height, sharing the
    /// panel's height under the ruler with the add-track row in view.
    pub fn fit_tracks(&mut self, cx: &mut Context<Self>) {
        let rows = self.lanes.rows.len();
        if rows == 0 {
            return;
        }
        let room = self.view.lanes_height() - ADD_ROW;
        self.heights.set_all(Some(room / rows as f32));
        self.heights_changed(cx);
    }

    /// View ▸ Reset Track Heights: each kind its own height again.
    pub fn reset_track_heights(&mut self, cx: &mut Context<Self>) {
        self.heights = TrackHeights::default();
        self.heights_changed(cx);
    }

    /// Every track `steps` along the ladder (Option-Up and Option-Down).
    pub fn step_track_heights(&mut self, steps: i32, cx: &mut Context<Self>) {
        let first = self.first_row_shown();
        self.step_keeping(steps, first, cx);
    }

    /// The tracks a source's picture and sound go on, as the person
    /// targeted them; None follows the first unlocked track of each kind.
    pub fn targets(&self) -> Option<&[String]> {
        self.targets.as_deref()
    }

    /// The sound tracks soloed for monitoring.
    pub fn solo_tracks(&self) -> &[String] {
        &self.solo
    }

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

    /// Clip `id`'s box in the window as the lanes last drew it, cut to the
    /// lanes; None while it is scrolled out of view or the lanes are not
    /// laid out. The window stands the Ask popover over it (`ai.ask`).
    pub fn clip_bounds(&self, id: &str) -> Option<Bounds<Pixels>> {
        let canvas = self.bounds.get();
        let (width, height) = (f32::from(canvas.size.width), f32::from(canvas.size.height));
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let (x, y, across, tall) = self.clip_rect_of(id)?;
        let (left, right) = (x.max(0.0), (x + across).min(width));
        let (top, bottom) = (y.max(layout::RULER), (y + tall).min(height));
        (right > left && bottom > top).then(|| {
            Bounds::new(
                canvas.origin + point(px(left), px(top)),
                size(px(right - left), px(bottom - top)),
            )
        })
    }

    /// A point at the playhead on the lanes' top, in the window (the lanes'
    /// middle while the playhead is out of view); None while the lanes are
    /// not laid out. The Ask popover stands there with no clip chosen.
    pub fn playhead_anchor(&self, cx: &App) -> Option<Bounds<Pixels>> {
        let canvas = self.bounds.get();
        let width = f32::from(canvas.size.width);
        if width <= 0.0 || f32::from(canvas.size.height) <= 0.0 {
            return None;
        }
        let x = self.view.x(self.session.read(cx).state.position);
        let x = if (0.0..=width).contains(&x) {
            x
        } else {
            width / 2.0
        };
        Some(Bounds::new(
            canvas.origin + point(px(x), px(layout::RULER)),
            size(px(1.0), px(1.0)),
        ))
    }

    /// Sequence ▸ Add Track… (`timeline.addTrack`): its two choices open
    /// under the tracks, from the keys.
    pub fn open_add_track(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let y = (layout::RULER + self.lanes.height - self.scroll_y + ADD_ROW)
            .min(f32::from(bounds.size.height));
        let at = point(
            bounds.origin.x - px(layout::TRACK_HEADER) + px(8.0),
            bounds.origin.y + px(y),
        );
        self.add_track_menu(at, true, window, cx);
    }

    // ------------------------------------------------------------ the lanes

    fn rebuild(&mut self, cx: &App) {
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

    /// The rows laid out again at the heights now.
    fn relayout(&mut self, cx: &mut Context<Self>) {
        let heights = self.heights.clone();
        Rc::make_mut(&mut self.lanes).relayout(&heights);
        cx.notify();
    }

    fn heights_changed(&mut self, cx: &mut Context<Self>) {
        self.relayout(cx);
        cx.emit(TimelineEvent::Heights(self.heights.clone()));
    }

    /// The first row whose lane shows under the ruler.
    fn first_row_shown(&self) -> Option<usize> {
        self.lanes
            .row_at(self.scroll_y)
            .or_else(|| (!self.lanes.rows.is_empty()).then_some(0))
    }

    /// Every track `steps` along the ladder, keeping `row` where it shows.
    fn step_keeping(&mut self, steps: i32, row: Option<usize>, cx: &mut Context<Self>) {
        if steps == 0 || self.lanes.rows.is_empty() {
            return;
        }
        let before = row.map(|row| self.lanes.rows[row].top - self.scroll_y);
        let kinds = self.lanes.kinds();
        self.heights.step(&kinds, steps);
        self.heights_changed(cx);
        if let (Some(row), Some(shown)) = (row, before) {
            self.scroll_y = (self.lanes.rows[row].top - shown).max(0.0);
        }
    }

    fn session_changed(&mut self, change: &Change, cx: &mut Context<Self>) {
        match change {
            Change::Snapshot => {
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
    fn bridge_event(&mut self, event: &Value, cx: &mut Context<Self>) {
        match event["type"].as_str().unwrap_or_default() {
            "check" if event["status"] == "done" => {
                let qc: HashSet<String> = event["findings"]
                    .as_array()
                    .map(|findings| {
                        findings
                            .iter()
                            .filter(|finding| finding["severity"] != "info")
                            .filter_map(|finding| finding["entity"].as_str())
                            .filter(|entity| !entity.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if qc != self.qc {
                    self.qc = qc;
                    self.rebuild(cx);
                    cx.notify();
                }
            }
            "agent_focus" => {
                let session = self.session.read(cx);
                let snapshot = &session.state.snapshot;
                let timeline = &snapshot.timeline;
                let at = event["times"]
                    .as_array()
                    .and_then(|times| times.first())
                    .and_then(|time| model::focus_seconds(time, timeline.rate(), timeline.start))
                    .or_else(|| {
                        let clip = event["clips"].as_array()?.first()?.as_str()?;
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

    /// Keeps a making's clock going once a second, which Reduce Motion
    /// leaves as all that moves, while any media is being made.
    fn tick_makings(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() || self.session.read(cx).state.makings.is_empty() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(theme::motion::CLOCK_FRAME)
                .await;
            let going = this.update(cx, |this, cx| {
                let going = !this.session.read(cx).state.makings.is_empty();
                if !going {
                    this.clock = None;
                }
                cx.notify();
                going
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        }));
    }

    /// The zoom, scroll and follow for lanes `width` × `height` with the
    /// playhead at `position`.
    fn layout(&mut self, width: f32, height: f32, position: f64) -> Viewport {
        let end = self.lanes.end;
        let fit = fit_pps(width, end);
        let most = (self.lanes.fps as f32 * MOST_FRAME).max(fit);
        let pps = match self.zoom {
            Some(zoom) if width <= 0.0 || zoom > fit => zoom.min(most),
            _ => {
                self.zoom = None;
                fit
            }
        };
        let mut view = Viewport {
            width,
            height,
            pps,
            left: if self.zoom.is_some() { self.left } else { 0.0 },
            scroll_y: self.scroll_y,
        };
        // The view pages on when the playhead moves out of it: playing, or sent there.
        if position != self.last_position && self.gesture.is_none() && width > 0.0 {
            let x = view.x(position);
            if x < PAD || x > width - FOLLOW_MARGIN {
                view.left = position - view.seconds() * 0.1;
            }
        }
        self.last_position = position;
        view.left = view.left.clamp(0.0, max_left(&view, end));
        view.scroll_y = view.scroll_y.clamp(
            0.0,
            (self.lanes.height + ADD_ROW - view.lanes_height()).max(0.0),
        );
        self.left = view.left;
        self.scroll_y = view.scroll_y;
        view
    }

    /// The fit and the most points a second at the lanes' width.
    fn zoom_range(&self) -> (f32, f32) {
        let fit = fit_pps(self.view.width, self.lanes.end);
        (fit, (self.lanes.fps as f32 * MOST_FRAME).max(fit))
    }

    /// A window position on the canvas.
    fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        let origin = self.bounds.get().origin;
        (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        if y < layout::RULER {
            return Hit::Ruler;
        }
        let view = &self.view;
        let lane_y = y - layout::RULER + view.scroll_y;
        let Some(row) = self.lanes.row_at(lane_y) else {
            return Hit::Lane;
        };
        let lane = &self.lanes.rows[row];
        // A previewed proposal's clips are its own, not the program's.
        if lane.previewed
            || lane_y < lane.top + CLIP_INSET
            || lane_y >= lane.top + lane.height - CLIP_INSET
        {
            return Hit::Lane;
        }
        // The last drawn is on top.
        for (index, clip) in lane.clips.iter().enumerate().rev() {
            let x0 = view.x(clip.start);
            let x1 = view.x(clip.end()).max(x0 + 2.0);
            if x < x0 || x >= x1 {
                continue;
            }
            let grab = GRAB.min((x1 - x0) / 3.0);
            let zone = if x - x0 < grab {
                Zone::In
            } else if x1 - x <= grab {
                Zone::Out
            } else {
                Zone::Body
            };
            return Hit::Clip { row, index, zone };
        }
        Hit::Lane
    }

    /// The clip `index` of `row`'s box on the canvas: left, top, width, height.
    fn clip_rect(&self, row: usize, index: usize) -> (f32, f32, f32, f32) {
        let lane = &self.lanes.rows[row];
        let clip = &lane.clips[index];
        let x0 = self.view.x(clip.start);
        let x1 = self.view.x(clip.end()).max(x0 + 2.0);
        let top = self.view.row_y(lane) + CLIP_INSET;
        (x0, top, x1 - x0, lane.height - 2.0 * CLIP_INSET)
    }

    /// Clip `id`'s box on the canvas, as [`Self::clip_rect`], where the
    /// lanes last drew it.
    fn clip_rect_of(&self, id: &str) -> Option<(f32, f32, f32, f32)> {
        let (row, index) = self.lanes.find(id)?;
        Some(self.clip_rect(row, index))
    }

    /// The rubber band's points across clip `index` of `row` as it reads
    /// now (a drag's values over the clip's own), and whether it is keyed.
    fn band_points(&self, row: usize, index: usize) -> (Vec<(f32, f32)>, bool) {
        let clip = &self.lanes.rows[row].clips[index];
        let (x0, top, across, tall) = self.clip_rect(row, index);
        let over = self.gain_over();
        let (keys, values, keyed) = gain_drawn(clip, self.lanes.fps, over.as_ref());
        band_line(&keys, &values, keyed, x0, across, top, tall, self.view.pps)
    }

    /// A band drawn other than as the snapshot has it: the one being
    /// dragged, else the one just let go.
    fn gain_over(&self) -> Option<GainShown> {
        match &self.gesture {
            Some(Gesture::Band(band)) => Some(band.shown()),
            _ => self.held_gain.as_ref().map(|(held, _)| held.clone()),
        }
    }

    /// The band's key under (`x`, `y`) on clip `index` of `row`, or its line
    /// (Some(None)); None off it.
    fn band_at(&self, row: usize, index: usize, x: f32, y: f32) -> Option<Option<usize>> {
        let lane = &self.lanes.rows[row];
        if !lane.audio() || lane.clips[index].card() {
            return None;
        }
        let (points, keyed) = self.band_points(row, index);
        if keyed {
            // The ends are the line's, not keys.
            for (key, (kx, ky)) in points.iter().enumerate().skip(1).take(points.len() - 2) {
                if (kx - x).hypot(ky - y) <= KEY_REACH {
                    return Some(Some(key - 1));
                }
            }
        }
        let line = line_y(&points, x)?;
        ((line - y).abs() <= LINE_REACH).then_some(None)
    }
}

/// Option-click on a sound clip's rubber band `frame` frames into it adds
/// a key there at `level` dB among `keys`, in frame order (a band not
/// `keyed` is one level, which the key takes the place of); a key already
/// on that frame stays as it is. Gives the index of the key on that frame,
/// which the drag from the click then moves.
pub fn add_gain_key(
    keys: &mut Vec<GainKey>,
    keyed: bool,
    frame: i64,
    fps: f64,
    level: f64,
) -> usize {
    if !keyed {
        keys.clear();
    }
    let at = keys
        .iter()
        .position(|known| known.frame >= frame)
        .unwrap_or(keys.len());
    if keys.get(at).is_none_or(|known| known.frame != frame) {
        keys.insert(
            at,
            GainKey {
                at: frame as f64 / fps,
                frame,
                value: level,
                interp: "linear".into(),
            },
        );
    }
    at
}

/// The points of a rubber band across a clip at `x0` `across` wide, `top`
/// and `tall` on the canvas: flat from its start at the first key's level,
/// through each key (`at` seconds in, at `pps`) at its value in `values`,
/// flat to its end at the last; and whether it is keyed (a diamond on each
/// point but the ends).
// The band takes its keys, their values and each measure of the clip it crosses.
#[allow(clippy::too_many_arguments)]
pub fn band_line(
    keys: &[GainKey],
    values: &[f64],
    keyed: bool,
    x0: f32,
    across: f32,
    top: f32,
    tall: f32,
    pps: f32,
) -> (Vec<(f32, f32)>, bool) {
    let value = |index: usize| values.get(index).copied().unwrap_or(0.0);
    let mut points = vec![(x0, gain_y(value(0), top, tall))];
    if keyed {
        for (index, key) in keys.iter().enumerate() {
            points.push((x0 + key.at as f32 * pps, gain_y(value(index), top, tall)));
        }
    }
    let last = if keyed {
        keys.len().saturating_sub(1)
    } else {
        0
    };
    points.push((x0 + across, gain_y(value(last), top, tall)));
    (points, keyed)
}

/// The line's height at `x`: flat before its first point and after its last.
fn line_y(points: &[(f32, f32)], x: f32) -> Option<f32> {
    let first = points.first()?;
    let last = points.last()?;
    if x <= first.0 {
        return Some(first.1);
    }
    if x >= last.0 {
        return Some(last.1);
    }
    points.windows(2).find_map(|pair| {
        let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
        (x >= ax && x <= bx).then(|| {
            if bx > ax {
                ay + (by - ay) * (x - ax) / (bx - ax)
            } else {
                ay
            }
        })
    })
}

// ----------------------------------------------------------------- pointer

impl TimelineView {
    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The skim ends here, so the picture goes back to the playhead's frame.
        self.end_skim(cx);
        self.tip = None;
        // kerfd connecting or gone: the lanes take nothing (Qt's timeline
        // was disabled then).
        if self.offline(cx) {
            self.gesture = None;
            return;
        }
        window.focus(&self.focus_handle);
        if self.gesture.is_some() {
            // Its button came up where it was not seen.
            self.release(event.position, cx);
        }
        let (x, y) = self.local(event.position);
        if let Some(grab) = self.thumb_at(x, y) {
            self.gesture = Some(Gesture::Thumb(grab));
            cx.notify();
            return;
        }
        let lanes = self.lanes.clone();
        let time = self.view.time(x).max(0.0);
        let modifiers = event.modifiers;
        let extending = modifiers.shift || modifiers.secondary();
        let hit = match self.hit(x, y) {
            // The first cut is the proposal's: a press there only seeks.
            Hit::Clip { .. } if lanes.proposed => Hit::Lane,
            hit => hit,
        };
        match hit {
            Hit::Clip { row, index, zone } => {
                self.press_clip(event, row, index, zone, x, y, time, window, cx)
            }
            hit => {
                if !lanes.proposed && matches!(hit, Hit::Lane) && !extending {
                    self.session.update(cx, |session, cx| {
                        if !session.selection.is_empty() {
                            session.selection.clear();
                            cx.notify();
                        }
                    });
                }
                self.gesture = Some(Gesture::Scrub);
                self.seek(time, true, cx);
            }
        }
        self.hover = None;
        cx.notify();
    }

    /// A press on clip `index` of `row`: what the tool and the keys held make of it.
    // Each argument is a distinct part of the press: its event, which clip, where and when.
    #[allow(clippy::too_many_arguments)]
    fn press_clip(
        &mut self,
        event: &MouseDownEvent,
        row: usize,
        index: usize,
        zone: Zone,
        x: f32,
        y: f32,
        time: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lanes = self.lanes.clone();
        let lane = &lanes.rows[row];
        let clip = &lane.clips[index];
        let id = clip.id.clone();
        let modifiers = event.modifiers;
        if modifiers.shift || modifiers.secondary() {
            // ⌘/Ctrl-click toggles a clip in the selection, ⇧-click adds it;
            // neither drags. The one clicked last leads.
            self.extend_selection(&id, modifiers.secondary(), cx);
            return;
        }
        if event.click_count >= 2 {
            self.choose(&id, cx);
            if modifiers.alt {
                window.dispatch_action(Box::new(RunCommand::new("ai.ask")), cx);
            } else if let Some(media) = self.made_media(clip, cx) {
                window.dispatch_action(
                    Box::new(crate::making::OpenShotGraph {
                        media: media.into(),
                    }),
                    cx,
                );
            }
            return;
        }
        if self.tool == Tool::Razor {
            self.choose(&id, cx);
            // Split where it was pressed, never at either end.
            let at = (time * lanes.fps).round() as i64;
            let at = at.clamp(
                clip.start_frame + 1,
                clip.start_frame + clip.duration_frames - 1,
            );
            if !lane.locked && clip.duration_frames > 1 {
                self.session.update(cx, |session, cx| {
                    session.edit(vec![json!({"op": "split", "clip": id, "at": at})]);
                    cx.notify();
                });
            }
            return;
        }
        self.choose(&id, cx);
        if lane.locked {
            // A locked track's clips are chosen, never dragged.
            return;
        }
        let (x0, top, across, tall) = self.clip_rect(row, index);
        // Option-drag across a signal clip draws its number.
        if self.tool == Tool::Selection && modifiers.alt && !clip.clip.signal.is_empty() {
            let mut draw = SignalDraw {
                clip: id,
                frames: clip.duration_frames.max(1),
                drawn: BTreeMap::new(),
                last: None,
            };
            draw.draw_to(x, y, (x0, top, across, tall));
            self.gesture = Some(Gesture::Signal(draw));
            return;
        }
        // A sound clip's rubber band: its keys and its line take the press.
        if self.tool == Tool::Selection {
            if let Some(key) = self.band_at(row, index, x, y) {
                let (mut keys, mut keyed) = clip.clip.gain_keys(lanes.fps);
                let mut key = key;
                if modifiers.alt && key.is_none() {
                    // Option-click on the line adds a key there, at the level it reads.
                    let offset = ((x - x0) / self.view.pps.max(f32::EPSILON)) as f64;
                    let frame = (offset.clamp(0.0, clip.duration) * lanes.fps).round() as i64;
                    let level = gain_at(y, top, tall);
                    key = Some(add_gain_key(&mut keys, keyed, frame, lanes.fps, level));
                    keyed = true;
                }
                let from: Vec<f64> = keys.iter().map(|key| key.value).collect();
                self.gesture = Some(Gesture::Band(BandDrag {
                    clip: id,
                    keyed,
                    key: if keyed { key } else { None },
                    press_y: f32::from(event.position.y),
                    top,
                    tall,
                    values: from.clone(),
                    from,
                    moved: modifiers.alt,
                    keys,
                }));
                return;
            }
        }
        let can = if zone == Zone::Body {
            self.tool.drags_bodies()
        } else {
            self.tool.trims_edges()
        };
        if can {
            let playhead = self.last_position;
            let lane_y = y - layout::RULER + self.view.scroll_y;
            let mut drag =
                ClipDrag::new(&lanes, row, index, zone, self.tool, time, lane_y, playhead);
            drag.press_window = f32::from(event.position.y);
            self.gesture = Some(Gesture::Clip(drag));
        }
    }

    /// The media a clip plays when it is a shot made from steps: a
    /// double-click opens its graph.
    fn made_media(&self, clip: &ClipBox, cx: &App) -> Option<String> {
        let media = clip.source.as_deref()?;
        self.session
            .read(cx)
            .timeline()
            .source(media)
            .filter(|source| source.is_made())
            .map(|source| source.id.clone())
    }

    /// Clip `id` alone is chosen.
    fn choose(&self, id: &str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            if session.selection != [id] {
                session.selection = vec![id.to_owned()];
                cx.notify();
            }
        });
    }

    /// ⇧ or ⌘ on clip `id`: it joins the chosen and leads them (what one
    /// clip's commands act on); ⇧ on a chosen one makes it lead; ⌘
    /// (`toggle`) on a chosen one lets it go.
    fn extend_selection(&self, id: &str, toggle: bool, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            extend(&mut session.selection, id, toggle);
            cx.notify();
        });
    }

    /// Whether kerfd is connecting or gone: the lanes and the heads take
    /// no presses, drags or switches then, and show none of what a press
    /// would take, as Qt's timeline was disabled (`setEnabled(false)`).
    pub fn offline(&self, cx: &App) -> bool {
        !self.session.read(cx).connected()
    }

    fn seek(&mut self, time: f64, scrub: bool, cx: &mut Context<Self>) {
        self.last_position = time;
        self.session.update(cx, |session, cx| {
            session.seek(time, scrub);
            cx.notify();
        });
    }

    /// The pointer moved anywhere in the window: a gesture follows it.
    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.gesture.is_none() {
            let inside = self.bounds.get().contains(&event.position);
            if !inside && (self.hover.is_some() || self.skim.is_some() || self.tip.is_some()) {
                self.hover = None;
                self.tip = None;
                self.end_skim(cx);
                cx.notify();
            }
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.release(event.position, cx);
            return;
        }
        let (x, y) = self.local(event.position);
        // A signal's clip where it is drawn now, the timeline perhaps moved.
        let signal_box = match &self.gesture {
            Some(Gesture::Signal(draw)) => self.clip_rect_of(&draw.clip),
            _ => None,
        };
        match &mut self.gesture {
            Some(Gesture::Band(band)) => {
                band.follow(f32::from(event.position.y));
                cx.notify();
                return;
            }
            Some(Gesture::Signal(draw)) => {
                if let Some(rect) = signal_box {
                    draw.draw_to(x, y, rect);
                }
                cx.notify();
                return;
            }
            Some(Gesture::Resize(resize)) => {
                let height = resize.from + f32::from(event.position.y) - resize.anchor;
                let (track, kind, every) = (resize.track.clone(), resize.kind, resize.every);
                self.resize_to(&track, kind, every, height, cx);
                return;
            }
            Some(Gesture::Slider) => {
                self.slide_zoom(event.position, cx);
                return;
            }
            Some(Gesture::Thumb(grab)) => {
                let grab = *grab;
                self.drag_thumb(x - grab, cx);
                return;
            }
            _ => {}
        }
        self.edge_pan(x);
        let time = self.view.time(x).max(0.0);
        let lanes = self.lanes.clone();
        let view = self.view;
        let snap = self.snap;
        match &mut self.gesture {
            Some(Gesture::Scrub) => self.seek(time, true, cx),
            Some(Gesture::Clip(drag)) => {
                let dy = f32::from(event.position.y) - drag.press_window;
                drag.follow(&lanes, &view, x, dy, snap)
            }
            _ => {}
        }
        cx.notify();
    }

    /// Pans the lanes while a drag holds the pointer near either end.
    fn edge_pan(&mut self, x: f32) {
        if self.zoom.is_none() {
            return;
        }
        let width = self.view.width;
        let shift = if x > width - EDGE_PAN {
            x - (width - EDGE_PAN)
        } else if x < EDGE_PAN {
            x - EDGE_PAN
        } else {
            return;
        };
        let left = self.view.left + (shift * 0.5 / self.view.pps) as f64;
        self.view.left = left.clamp(0.0, max_left(&self.view, self.lanes.end));
        self.left = self.view.left;
    }

    fn release(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        match gesture {
            Gesture::Scrub => {
                let (x, _) = self.local(position);
                let time = self.view.time(x).max(0.0);
                self.seek(time, false, cx);
            }
            Gesture::Clip(drag) => self.drop_clip(drag, cx),
            Gesture::Band(band) => self.drop_band(band, cx),
            Gesture::Signal(draw) => self.drop_signal(draw, cx),
            Gesture::Resize(_) => cx.emit(TimelineEvent::Heights(self.heights.clone())),
            Gesture::Slider | Gesture::Thumb(_) => {}
        }
        cx.notify();
    }

    /// Sends what a drag did as one edit.
    fn drop_clip(&mut self, drag: ClipDrag, cx: &mut Context<Self>) {
        let lanes = self.lanes.clone();
        let op = match drag.op(&lanes) {
            Ok(Some(op)) => op,
            Ok(None) => return,
            Err(why) => {
                self.session.update(cx, |session, cx| {
                    session.state.notify(why, true);
                    cx.notify();
                });
                return;
            }
        };
        let (start, duration) = drag.span(lanes.fps);
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        });
        if sent {
            let head_shift = if drag.edit == Edit::TrimIn {
                drag.frames as f64 / lanes.fps
            } else {
                0.0
            };
            self.pending = Some(Pending {
                ghost: Ghost {
                    id: drag.id,
                    row: drag.target,
                    start,
                    duration,
                    edge: None,
                    dragging: false,
                    head_shift,
                },
                revision,
                at: Instant::now(),
            });
        }
    }

    /// Sets the gain the band was dragged to: a level, or its keys.
    fn drop_band(&mut self, band: BandDrag, cx: &mut Context<Self>) {
        if !band.moved {
            return;
        }
        let gain = band.gain();
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![
                json!({"op": "set", "clip": band.clip, "props": {"gain": gain}}),
            ]);
            cx.notify();
            sent
        });
        if sent {
            self.held_gain = Some((band.shown(), revision));
            self.pending_marker(revision);
        }
    }

    /// Sets the signal drawn: its keys, thinned. A click draws nothing.
    fn drop_signal(&mut self, draw: SignalDraw, cx: &mut Context<Self>) {
        if draw.drawn.len() < 2 {
            return;
        }
        let keys = model::signal_keys(&draw.drawn, SIGNAL_WITHIN);
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let op = json!({"op": "set", "clip": draw.clip, "props": {"value": {"keys": keys}}});
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        });
        if sent {
            self.drawn = Some((draw.clip, draw.drawn, revision));
            self.pending_marker(revision);
        }
    }

    /// An edit with no clip to hold in place went: a refusal still puts
    /// back what it showed.
    fn pending_marker(&mut self, revision: u64) {
        self.pending = Some(Pending {
            ghost: Ghost {
                id: String::new(),
                row: usize::MAX,
                start: 0.0,
                duration: 0.0,
                edge: None,
                dragging: false,
                head_shift: 0.0,
            },
            revision,
            at: Instant::now(),
        });
    }

    fn hover_at(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.gesture.is_some() {
            return;
        }
        // Nothing to point at while kerfd is not connected: no trim edges,
        // no tips, no skim.
        if self.offline(cx) {
            if self.hover.is_some() || self.tip.is_some() {
                self.hover = None;
                self.tip = None;
                cx.notify();
            }
            self.end_skim(cx);
            return;
        }
        let (x, y) = self.local(event.position);
        let lanes = self.lanes.clone();
        let hover = match self.hit(x, y) {
            Hit::Clip { row, index, zone } if !lanes.proposed => {
                let lane = &lanes.rows[row];
                let band = self.tool == Tool::Selection
                    && !lane.locked
                    && self.band_at(row, index, x, y).is_some();
                Some(Hover {
                    clip: lane.clips[index].id.clone(),
                    zone: if lane.locked { Zone::Body } else { zone },
                    band,
                })
            }
            _ => None,
        };
        let tip = self.tip_at(x, y, cx);
        if hover != self.hover || tip != self.tip {
            self.hover = hover;
            self.tip = tip;
            cx.notify();
        }
        self.skim_to(x, y, cx);
    }

    /// The pointer over the lanes skims: the picture shows the frame under
    /// it while nothing plays, and the playhead stays where it was.
    fn skim_to(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let state = &self.session.read(cx).state;
        let inside = x >= 0.0 && y >= 0.0 && x < self.view.width && y < self.view.height;
        if state.playing || !inside || self.thumb_at(x, y).is_some() {
            self.end_skim(cx);
            return;
        }
        let fps = self.lanes.fps;
        let end = self.lanes.end.max(state.program_length());
        let frame = (self.view.time(x).clamp(0.0, end) * fps).round() / fps;
        if self
            .skim
            .is_some_and(|known| (known - frame).abs() < 0.5 / fps)
        {
            return;
        }
        self.skim = Some(frame);
        cx.emit(TimelineEvent::Skim(Some(frame)));
        cx.notify();
    }

    fn end_skim(&mut self, cx: &mut Context<Self>) {
        if self.skim.take().is_some() {
            cx.emit(TimelineEvent::Skim(None));
            cx.notify();
        }
    }

    /// What the pointer rests on, for its tip: a clip (its name and its
    /// in and out), a sequence marker's flag, a proposal's band.
    fn tip_at(&self, x: f32, y: f32, cx: &App) -> Option<CanvasTip> {
        let lanes = &self.lanes;
        let view = &self.view;
        let fps = lanes.fps;
        if y < layout::RULER {
            for (index, mark) in lanes.markers.iter().enumerate() {
                let mx = view.x(mark.time);
                if x >= mx - 5.0 && x <= mx + 5.0 && (9.0..=layout::RULER - 2.0).contains(&y) {
                    let words = match &mark.label {
                        Some(name) => format!("Sequence marker · {name}"),
                        None => "Sequence marker".to_owned(),
                    };
                    return Some(CanvasTip {
                        key: format!("marker-{index}"),
                        tip: ui::Tip::new(words).key(model::timecode(mark.time, fps)),
                        bounds: (mx - 5.0, 9.0, 10.0, 12.0),
                    });
                }
            }
            for band in &lanes.bands {
                let (left, right) = (
                    view.x(band.start),
                    view.x(band.end).max(view.x(band.start) + 3.0),
                );
                if x >= left && x <= right && y <= BAND_ROW + 3.0 {
                    let words = if band.author.is_empty() {
                        "Proposal".to_owned()
                    } else {
                        format!("Proposal · {}", band.author)
                    };
                    return Some(CanvasTip {
                        key: format!("band-{}", band.id),
                        tip: ui::Tip::new(words),
                        bounds: (left, 0.0, right - left, BAND_ROW + 3.0),
                    });
                }
            }
            return None;
        }
        let Hit::Clip { row, index, .. } = self.hit(x, y) else {
            return None;
        };
        let clip = &lanes.rows[row].clips[index];
        let (x0, top, across, tall) = self.clip_rect(row, index);
        let _ = cx;
        Some(CanvasTip {
            key: format!("clip-{}", clip.id),
            tip: ui::Tip::new(clip.label.clone()).key(format!(
                "{}–{}",
                model::timecode(clip.start, fps),
                model::timecode(clip.end(), fps)
            )),
            bounds: (
                x0.max(0.0),
                top,
                (x0 + across).min(view.width) - x0.max(0.0),
                tall,
            ),
        })
    }

    /// A right-click on a clip chooses it and opens its menu.
    fn context_press(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_skim(cx);
        self.tip = None;
        if self.offline(cx) {
            return;
        }
        window.focus(&self.focus_handle);
        let (x, y) = self.local(event.position);
        if self.lanes.proposed {
            return;
        }
        let Hit::Clip { row, index, .. } = self.hit(x, y) else {
            return;
        };
        let id = self.lanes.rows[row].clips[index].id.clone();
        self.session.update(cx, |session, cx| {
            if session.selection.first() != Some(&id) {
                if session.selection.contains(&id) {
                    extend(&mut session.selection, &id, false);
                } else {
                    session.selection = vec![id.clone()];
                }
                cx.notify();
            }
        });
        self.clip_menu(row, index, event.position, window, cx);
        // The menu has the keys: the timeline does not take them back.
        window.prevent_default();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(text::BODY.line));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let overflow = (self.lanes.height + ADD_ROW - self.view.lanes_height()).max(0.0);
        if event.modifiers.secondary() {
            // Zooms around the pointer.
            let (x, _) = self.local(event.position);
            self.zoom_by((dy * 0.01).exp(), Some(x));
        } else if event.modifiers.shift || dx.abs() > dy.abs() {
            self.pan(-if dx.abs() > dy.abs() { dx } else { dy });
        } else if overflow > 0.0 {
            self.scroll_y = (self.scroll_y - dy).clamp(0.0, overflow);
        } else {
            self.pan(-dy);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Option-scroll over the heads steps every track, a step a notch (a
    /// trackpad's small deltas add up to one), keeping the track under the
    /// pointer where it is; otherwise the heads scroll as the lanes do.
    fn heads_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.alt {
            self.wheel = 0.0;
            self.wheel(event, window, cx);
            return;
        }
        cx.stop_propagation();
        if matches!(event.touch_phase, TouchPhase::Started | TouchPhase::Ended) {
            self.wheel = 0.0;
        }
        let steps = match event.delta {
            // A notch is one step, however many lines the system says it
            // is (three on Linux); a high-resolution wheel's parts add up.
            ScrollDelta::Lines(lines) => {
                let lines = if lines.y != 0.0 { lines.y } else { lines.x };
                self.wheel += lines;
                if self.wheel.abs() >= 1.0 {
                    let step = self.wheel.signum() as i32;
                    self.wheel = 0.0;
                    step
                } else {
                    0
                }
            }
            ScrollDelta::Pixels(pixels) => {
                let pixels = if pixels.y != px(0.0) {
                    pixels.y
                } else {
                    pixels.x
                };
                self.wheel += f32::from(pixels);
                let steps = (self.wheel / WHEEL_STEP).trunc();
                self.wheel -= steps * WHEEL_STEP;
                steps as i32
            }
        };
        if steps == 0 {
            return;
        }
        let (_, y) = self.local(event.position);
        let row = self
            .lanes
            .row_at(y - layout::RULER + self.scroll_y)
            .or_else(|| self.lanes.rows.len().checked_sub(1));
        self.step_keeping(steps, row, cx);
    }

    /// Pans by `points`, later when positive.
    fn pan(&mut self, points: f32) {
        if self.zoom.is_none() {
            return;
        }
        let left = self.left + (points / self.view.pps.max(f32::EPSILON)) as f64;
        self.left = left.clamp(0.0, max_left(&self.view, self.lanes.end));
    }

    /// Zooms by `factor` around `anchor` points across the lanes, else
    /// around the playhead when it shows, else the middle. Zooming out
    /// stops at the fit.
    fn zoom_by(&mut self, factor: f32, anchor: Option<f32>) {
        let view = self.view;
        if view.width <= 0.0 {
            return;
        }
        let (fit, most) = self.zoom_range();
        self.zoom_to((view.pps * factor).clamp(fit, most), anchor);
    }

    fn zoom_to(&mut self, pps: f32, anchor: Option<f32>) {
        let view = self.view;
        let (fit, _) = self.zoom_range();
        let anchor = anchor.unwrap_or_else(|| {
            let x = view.x(self.last_position);
            if (0.0..=view.width).contains(&x) {
                x
            } else {
                view.width / 2.0
            }
        });
        let time = view.time(anchor);
        if pps <= fit * 1.001 {
            self.zoom = None;
            self.left = 0.0;
        } else {
            self.zoom = Some(pps);
            self.left = (time - ((anchor - PAD) / pps) as f64).max(0.0);
        }
        self.view.pps = pps.max(fit);
    }

    /// The zoom slider's knob follows the pointer.
    fn slide_zoom(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let bounds = self.slider.get();
        let width = f32::from(bounds.size.width).max(1.0);
        let share = (f32::from(position.x - bounds.origin.x) - 5.0) / (width - 10.0).max(1.0);
        let (fit, most) = self.zoom_range();
        self.zoom_to(zoom_of_share(share.clamp(0.0, 1.0), fit, most), None);
        cx.notify();
    }

    /// The scroll bar's thumb under (`x`, `y`): how far into it.
    fn thumb_at(&self, x: f32, y: f32) -> Option<f32> {
        if self.zoom.is_none() || self.view.width <= 0.0 {
            return None;
        }
        let top = self.view.height - SCROLLER - 2.0;
        if y < top - 2.0 || y > self.view.height {
            return None;
        }
        let (left, width) = thumb(&self.view, self.lanes.end);
        (x >= left && x <= left + width).then_some(x - left)
    }

    /// The thumb dragged to `left` points across.
    fn drag_thumb(&mut self, left: f32, cx: &mut Context<Self>) {
        let span = scroll_span(&self.view, self.lanes.end);
        let track = (self.view.width - 2.0 * PAD).max(1.0);
        let time = ((left - PAD) / track) as f64 * span;
        self.left = time.clamp(0.0, max_left(&self.view, self.lanes.end));
        cx.notify();
    }

    /// Track `track` (or every track of `kind`, `every`) to `height`.
    fn resize_to(
        &mut self,
        track: &str,
        kind: TrackKind,
        every: bool,
        height: f32,
        cx: &mut Context<Self>,
    ) {
        if every {
            let ids: Vec<String> = self
                .lanes
                .rows
                .iter()
                .filter(|row| row.kind == kind)
                .map(|row| row.id.clone())
                .collect();
            for id in ids {
                self.heights.set(&id, Some(height));
            }
        } else {
            self.heights.set(track, Some(height));
        }
        self.relayout(cx);
    }

    /// Switches a track's `switch` (`locked`, `muted`, `enabled`,
    /// `syncLocked`) to `value`.
    fn set_track(&mut self, track: &str, switch: &str, value: bool, cx: &mut Context<Self>) {
        if self.lanes.proposed || self.offline(cx) {
            return;
        }
        let mut op = json!({"op": "set-track", "track": track});
        op[switch] = json!(value);
        self.session.update(cx, |session, cx| {
            session.edit(vec![op]);
            cx.notify();
        });
    }

    /// The targets as they stand: the person's, else the first unlocked
    /// track of each kind.
    fn targets_now(&self, cx: &App) -> Vec<String> {
        match &self.targets {
            Some(targets) => targets.clone(),
            None => model::default_targets(self.session.read(cx).timeline()),
        }
    }

    /// Track `id` targeted in place of its kind's other, or not targeted.
    fn toggle_target(&mut self, id: &str, cx: &mut Context<Self>) {
        let targets =
            model::toggle_target(self.session.read(cx).timeline(), &self.targets_now(cx), id);
        self.targets = Some(targets);
        cx.emit(TimelineEvent::Targets(self.targets.clone()));
        cx.notify();
    }

    fn toggle_solo(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.solo.iter().position(|known| known == id) {
            Some(at) => {
                self.solo.remove(at);
            }
            None => self.solo.push(id.to_owned()),
        }
        self.send_solo(cx);
        cx.notify();
    }

    /// Tells kerfd which sound tracks playback hears alone (none: all).
    fn send_solo(&mut self, cx: &mut Context<Self>) {
        let tracks = self.solo.clone();
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "solo", "tracks": tracks}));
        });
    }

    /// Each command's keys in the keymap in use.
    fn keys_of(&mut self, id: &str, cx: &App) -> Option<String> {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 != keymap {
            let keys = crate::commands::catalog(keymap)
                .into_iter()
                .map(|command| (command.id, command.keys))
                .collect();
            self.keys = (keymap.to_owned(), keys);
        }
        self.keys
            .1
            .get(id)
            .and_then(|keys| keys.first())
            .map(|key| crate::commands::shown_keys(key))
    }

    // --------------------------------------------------------------- menus

    fn open_menu(
        &mut self,
        items: Vec<ui::MenuItem>,
        at: Point<Pixels>,
        width: f32,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(width);
            if from_keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            at,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// A row of a menu that runs command `id`, greyed with why when it
    /// cannot run now.
    fn command_item(
        &mut self,
        id: &'static str,
        words: &'static str,
        states: &Value,
        cx: &App,
    ) -> ui::MenuItem {
        let mut item = ui::MenuItem::new(words)
            .on_choose(move |window, cx| window.dispatch_action(Box::new(RunCommand::new(id)), cx));
        if let Some(key) = self.keys_of(id, cx) {
            item = item.key(key);
        }
        if let Some(reason) = crate::palette::unavailable(states, id) {
            item = item.disabled(true).tip(reason.to_owned());
        }
        item
    }

    /// A clip's menu (Qt's): Ask Kerf…, the edits on it, Focus Region…,
    /// Start Motion Path where it can take one, and for a shot made from
    /// steps Open Graph, or Carry Steps into the Document… while its
    /// steps are still in a recipe file.
    fn clip_menu(
        &mut self,
        row: usize,
        index: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lanes = self.lanes.clone();
        let lane = &lanes.rows[row];
        let clip = &lane.clips[index];
        // Ask's state with Kerf's access too (no key, signed out), as the
        // window's menus have it: greyed with why, its key shown.
        let states = self.session.read(cx).command_states(crate::app::cloud(cx));
        let mut items = vec![self
            .command_item("ai.ask", "Ask Kerf…", &states, cx)
            .icon(Icon::Sparkle)];
        items.push(
            self.command_item("clip.split", "Split at Playhead", &states, cx)
                .separated(),
        );
        items.push(self.command_item("clip.delete", "Delete", &states, cx));
        items.push(self.command_item("clip.rippleDelete", "Ripple Delete", &states, cx));
        items.push(self.command_item("clip.extract", "Extract", &states, cx));
        items.push(
            self.command_item("clip.focusRegion", "Focus Region…", &states, cx)
                .separated(),
        );
        let path = clip.clip.props["path"]
            .as_str()
            .is_some_and(|path| !path.trim().is_empty());
        if lane.kind == TrackKind::Video && clip.kind != ClipKind::Caption && !path {
            items.push(self.command_item("clip.startPath", "Start Motion Path", &states, cx));
        }
        let source = clip
            .source
            .as_deref()
            .and_then(|id| self.session.read(cx).timeline().source(id).cloned());
        if let Some(source) = source {
            if source.uncarried() {
                let media: SharedString = source.id.clone().into();
                let recipe = source.recipe.clone().unwrap_or_default();
                items.push(
                    ui::MenuItem::new("Carry Steps into the Document…")
                        .separated()
                        .tip(format!(
                            "Write the steps of {recipe} under {}'s line in the document; what would be written is shown first",
                            source.id
                        ))
                        .on_choose(move |window, cx| {
                            window.dispatch_action(
                                Box::new(crate::making::MakeMedia {
                                    media: media.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            )
                        }),
                );
            } else if source.is_made() {
                let media: SharedString = source.id.clone().into();
                items.push(ui::MenuItem::new("Open Graph").separated().on_choose(
                    move |window, cx| {
                        window.dispatch_action(
                            Box::new(crate::making::OpenShotGraph {
                                media: media.clone(),
                            }),
                            cx,
                        )
                    },
                ));
            }
        }
        self.open_menu(items, at, 232.0, false, window, cx);
    }

    /// A head's menu (Qt's): Sync lock, its height, every track's, Fit
    /// tracks to panel and Reset track heights.
    fn head_menu(
        &mut self,
        row: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.offline(cx) {
            return;
        }
        let lanes = self.lanes.clone();
        let Some(lane) = lanes.rows.get(row) else {
            return;
        };
        let this = cx.entity().downgrade();
        let (track, synced) = (lane.id.clone(), lane.sync_locked);
        let mut items = Vec::new();
        {
            let this = this.clone();
            let track = track.clone();
            items.push(
                ui::MenuItem::new("Sync lock")
                    .checked(synced)
                    .disabled(lanes.proposed)
                    .tip("Ripple edits move this track")
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| {
                            view.set_track(&track, "syncLocked", !synced, cx)
                        });
                    }),
            );
        }
        let presets = [
            ("Small", Some(layout::TRACK_SMALL)),
            ("Medium", None),
            ("Large", Some(layout::TRACK_LARGE)),
        ];
        let own: Vec<ui::MenuItem> = presets
            .iter()
            .map(|(name, height)| {
                let wanted = height.unwrap_or_else(|| {
                    self.heights
                        .all
                        .unwrap_or_else(|| default_height(lane.kind))
                });
                let this = this.clone();
                let track = track.clone();
                let height = *height;
                ui::MenuItem::new(*name)
                    .checked((lane.height - wanted).abs() < 0.5)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| {
                            view.heights.set(&track, height);
                            view.heights_changed(cx);
                        });
                    })
            })
            .collect();
        items.push(ui::MenuItem::new("Track height").separated().submenu(own));
        let all: Vec<ui::MenuItem> = presets
            .iter()
            .map(|(name, height)| {
                let every = lanes.rows.iter().all(|row| {
                    let wanted = height.unwrap_or_else(|| default_height(row.kind));
                    (row.height - wanted).abs() < 0.5
                });
                let this = this.clone();
                let height = *height;
                ui::MenuItem::new(*name)
                    .checked(every)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| match height {
                            Some(height) => {
                                view.heights.set_all(Some(height));
                                view.heights_changed(cx);
                            }
                            None => view.reset_track_heights(cx),
                        });
                    })
            })
            .collect();
        items.push(ui::MenuItem::new("All tracks").submenu(all));
        {
            let this = this.clone();
            items.push(
                ui::MenuItem::new("Fit tracks to panel")
                    .separated()
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| view.fit_tracks(cx));
                    }),
            );
        }
        items.push(
            ui::MenuItem::new("Reset track heights").on_choose(move |_, cx| {
                let _ = this.update(cx, |view, cx| view.reset_track_heights(cx));
            }),
        );
        self.open_menu(items, at, 220.0, false, window, cx);
    }

    /// The add-track row's choices: a picture track or a sound track.
    fn add_track_menu(
        &mut self,
        at: Point<Pixels>,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
        let items = vec![
            self.command_item("sequence.addVideoTrack", "Add video track", &states, cx),
            self.command_item("sequence.addAudioTrack", "Add audio track", &states, cx),
        ];
        self.open_menu(items, at, 200.0, from_keys, window, cx);
    }

    // ---------------------------------------------------------------- keys

    /// The registry's commands the timeline answers itself while it has the
    /// keys: the tools, Esc letting a chosen clip go when there is no
    /// proposal to reject, the track heights and Add Track….
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        if let Some(tool) = Tool::from_command(id) {
            self.set_tool(tool, cx);
            return;
        }
        match id {
            "proposal.reject" => {
                let session = self.session.read(cx);
                let none_open = session.state.snapshot.pending().is_none();
                if none_open && !session.selection.is_empty() {
                    self.session.update(cx, |session, cx| {
                        session.selection.clear();
                        cx.notify();
                    });
                } else {
                    cx.propagate();
                }
            }
            "view.fitTracks" => self.fit_tracks(cx),
            "view.resetTrackHeights" => self.reset_track_heights(cx),
            "timeline.addTrack" => self.open_add_track(window, cx),
            "sequence.snap" => {
                let on = !self.snap;
                self.set_snapping(on, cx);
            }
            _ => cx.propagate(),
        }
    }

    /// Option-Up and Option-Down step every track; ⌘= and ⌘− zoom (and
    /// repeat while held).
    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let only_alt = modifiers.alt && !modifiers.shift && !modifiers.secondary();
        match keystroke.key.as_str() {
            "up" | "down" if only_alt => {
                let steps = if keystroke.key == "up" { 1 } else { -1 };
                self.step_track_heights(steps, cx);
                cx.stop_propagation();
            }
            "=" | "+" if modifiers.secondary() && !modifiers.alt => {
                self.zoom_by(ZOOM_STEP, None);
                cx.stop_propagation();
                cx.notify();
            }
            "-" if modifiers.secondary() && !modifiers.alt => {
                self.zoom_by(1.0 / ZOOM_STEP, None);
                cx.stop_propagation();
                cx.notify();
            }
            _ => {}
        }
    }
}

/// `id` joins `selection` and leads it (what one clip's commands act on);
/// ⇧ on one already chosen makes it lead; `toggle` (⌘) on one already
/// chosen lets it go.
pub fn extend(selection: &mut Vec<String>, id: &str, toggle: bool) {
    match selection.iter().position(|chosen| chosen == id) {
        Some(at) if toggle => {
            selection.remove(at);
        }
        Some(at) => {
            let chosen = selection.remove(at);
            selection.insert(0, chosen);
        }
        None => selection.insert(0, id.to_owned()),
    }
}

// ------------------------------------------------------------ strip, heads

/// Whether the timeline's strip carries the transport: while the window
/// shows no picture (View ▸ Picture off, `Session::picture_open` false),
/// as Qt's strip carried it while its viewer was hidden. Until the window
/// says (None) the stage has it.
pub fn strip_has_transport(picture_open: Option<bool>) -> bool {
    picture_open == Some(false)
}

/// What the timeline's strip shows of what it can let go. Too narrow for
/// all of it (a narrow window, the transport in it), it lets go first of
/// the zoom slider (− and + stay), then of the transport's Go to start and
/// Go to end (Home and End still reach them) and the transport's wide gap,
/// then of the cut's length and the space before the tools. What still
/// does not fit is cut off at the tools' end: Snap, − and +, and Fit stay
/// whole, as in Qt's strip, which held its window wide enough for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripFit {
    pub slider: bool,
    pub transport_ends: bool,
    pub length: bool,
}

impl StripFit {
    pub const ALL: StripFit = StripFit {
        slider: true,
        transport_ends: true,
        length: true,
    };

    /// Each step, from all of it, as they are let go.
    pub const STEPS: [StripFit; 4] = [
        StripFit::ALL,
        StripFit {
            slider: false,
            ..StripFit::ALL
        },
        StripFit {
            slider: false,
            transport_ends: false,
            length: true,
        },
        StripFit {
            slider: false,
            transport_ends: false,
            length: false,
        },
    ];

    /// How much wider the strip is with this shown than with all of it let
    /// go: `transport`, whether it carries the transport (three buttons
    /// always, five with its ends); `length`, how wide the cut's length is
    /// set.
    pub fn width(self, transport: bool, length: f32) -> f32 {
        let button = theme::height::H24 + layout::STRIP_TRANSPORT_SPACING;
        let mut width = 0.0;
        if transport {
            width += 3.0 * button - layout::STRIP_TRANSPORT_SPACING + STRIP_GAP;
            if self.transport_ends {
                width += 2.0 * button + layout::STRIP_TRANSPORT_GAP - STRIP_GAP;
            }
        }
        if self.slider {
            width += SLIDER + STRIP_GAP;
        }
        if self.length {
            width += length + STRIP_GAP + TOOLS_SPACE + STRIP_GAP;
        }
        width
    }

    /// What fits in `room`, from what it shows now: the most of it whose
    /// width, over `rest` (what the strip needs with all of it let go),
    /// fits; showing more than now only with [`layout::STRIP_REGROW`] to
    /// spare. With none fitting, all of it let go.
    pub fn within(self, room: f32, rest: f32, transport: bool, length: f32) -> StripFit {
        let now = Self::STEPS
            .iter()
            .position(|step| *step == self)
            .unwrap_or(0);
        Self::STEPS
            .into_iter()
            .enumerate()
            .find(|(step, fit)| {
                let spare = if *step < now {
                    layout::STRIP_REGROW
                } else {
                    0.0
                };
                rest + fit.width(transport, length) + spare <= room
            })
            .map_or(Self::STEPS[3], |(_, fit)| fit)
    }
}

/// How wide the strip, what it holds on its left and on its right (each
/// as wide as its pieces would be, cut off or not), and the cut's length
/// were last laid out.
#[derive(Clone, Default)]
struct StripWidths {
    room: Rc<Cell<f32>>,
    left: Rc<Cell<f32>>,
    right: Rc<Cell<f32>>,
    length: Rc<Cell<f32>>,
}

/// Over its parent, a canvas that keeps the parent's width in `cell`; a
/// new width draws the strip again with it.
fn width_canvas(cell: Rc<Cell<f32>>) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let width = f32::from(bounds.size.width);
            if (cell.get() - width).abs() > 0.1 {
                cell.set(width);
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl TimelineView {
    /// The transport, in the strip while the picture pane is closed: Go to
    /// start, Previous frame, Play (Pause while playing), Next frame, Go to
    /// end, each its command ([`stage::TRANSPORT`]), its key in its tip,
    /// greyed with the registry's reason while it cannot run
    /// ([`stage::transport_reasons`], [`stage::transport_tip`]), as the
    /// stage's are. It fades in over `FADE` each time it comes. Without
    /// `ends` (a narrow strip, [`StripFit`]) it has no Go to start and Go
    /// to end, and the strip's own gap to the timecode.
    fn strip_transport(&mut self, ends: bool, cx: &mut Context<Self>) -> impl IntoElement {
        // Asked of the registry again only when what it is asked about
        // changes, not on every frame played.
        let mut context = self.session.read(cx).command_context();
        if let Some(context) = context.as_object_mut() {
            context.remove("playheadFrame");
        }
        if self
            .transport_asked
            .as_ref()
            .is_none_or(|(asked, _)| *asked != context)
        {
            let reasons = stage::transport_reasons(&context);
            self.transport_asked = Some((context, reasons));
        }
        let reasons = self
            .transport_asked
            .as_ref()
            .map(|(_, reasons)| reasons.clone())
            .unwrap_or_default();
        let playing = self.session.read(cx).state.playing;
        let buttons: Vec<_> = stage::TRANSPORT
            .into_iter()
            .filter(|(id, _, _)| ends || !matches!(*id, "playback.goToStart" | "playback.goToEnd"))
            .map(|(id, name, icon)| {
                let play = id == "playback.toggle";
                let (name, icon) = if play && playing {
                    ("Pause", Icon::Pause)
                } else {
                    (name, icon)
                };
                let reason = reasons.get(id).cloned();
                let mut button = ui::icon_button(
                    ElementId::Name(format!("timeline-transport-{id}").into()),
                    icon,
                )
                .size(theme::height::H24)
                .tooltip(name)
                .disabled(reason.is_some())
                .focusable(false)
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx);
                });
                // Play in ink at rest, as Qt's was.
                if play {
                    button = button.color(theme::ink());
                }
                if let Some(key) = self.keys_of(id, cx) {
                    button = button.key(key);
                }
                if let Some(reason) = reason {
                    button = button.reason(stage::transport_tip(name, Some(&reason)));
                }
                button
            })
            .collect();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(layout::STRIP_TRANSPORT_SPACING))
            // To the timecode, Qt's gap, the strip's own gap counted in.
            .when(ends, |transport| {
                transport.pr(px(layout::STRIP_TRANSPORT_GAP - STRIP_GAP))
            })
            .children(buttons)
            .with_animation(
                ("timeline-transport", self.strip_transport.1),
                motion::animation(theme::motion::FADE),
                |transport, t| transport.opacity(t),
            )
    }

    /// The strip over the lanes: the transport (while the picture pane is
    /// closed), the timecode where the playhead is and the cut's length,
    /// the tools; at its right Snap and the zoom. As much shows as fits
    /// ([`StripFit`]), from how wide it and its pieces were last laid out.
    fn strip(&mut self, shown: f64, skimming: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let has_transport = strip_has_transport(self.session.read(cx).picture_open);
        let widths = self.strip_widths.clone();
        let length_width = widths.length.get();
        let room = widths.room.get();
        // Its pieces as last laid out, less those that come and go then.
        let rest = widths.left.get() + widths.right.get()
            - self.strip_fit.width(self.strip_transport.0, length_width);
        if room > 0.0 {
            let start = if has_transport {
                layout::STRIP_EDGE
            } else {
                STRIP_START
            };
            let room = room - start - STRIP_END - 2.0 * STRIP_GAP;
            self.strip_fit = self
                .strip_fit
                .within(room, rest, has_transport, length_width);
        }
        let shows = self.strip_fit;
        if has_transport != self.strip_transport.0 {
            self.strip_transport.0 = has_transport;
            if has_transport {
                self.strip_transport.1 += 1;
            }
        }
        let transport = has_transport.then(|| self.strip_transport(shows.transport_ends, cx));
        let lanes = self.lanes.clone();
        let this = cx.entity().downgrade();
        let tips: Vec<String> = [Tool::Selection, Tool::Razor]
            .into_iter()
            .map(|tool| match self.keys_of(tool.command(), cx) {
                Some(key) => format!("{}  ·  {key}", tool.name()),
                None => tool.name().to_owned(),
            })
            .collect();
        let tips: Vec<&str> = tips.iter().map(String::as_str).collect();
        let tools = ui::segmented(
            "timeline-tools",
            &[(Tool::Selection, "Selection"), (Tool::Razor, "Razor")],
            self.tool,
            move |tool, _, cx| {
                let _ = this.update(cx, |view, cx| view.set_tool(tool, cx));
            },
        )
        .tips(&tips)
        .focusable(false);
        let other_tool = !matches!(self.tool, Tool::Selection | Tool::Razor);
        let tool_key = self.keys_of(self.tool.command(), cx);
        let snap_key = self.keys_of("sequence.snap", cx);
        let zoom_in = crate::commands::shown_keys("Ctrl+=");
        let zoom_out = crate::commands::shown_keys("Ctrl+-");
        let (fit, most) = self.zoom_range();
        let share = zoom_share(self.view.pps, fit, most);
        // With the picture pane closed, the transport first; then where
        // the playhead is (in ink2 where the pointer skims), the cut's
        // length, the tools. Cut off at the tools' end where it must be.
        let left = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(STRIP_GAP))
            .child(width_canvas(widths.left.clone()))
            .children(transport)
            .child(ui::label(
                model::timecode(shown, lanes.fps),
                text::TIMECODE_STRONG,
                if skimming {
                    theme::ink2()
                } else {
                    theme::ink()
                },
            ))
            .when(shows.length, |left| {
                left.child(
                    ui::label(
                        model::timecode(lanes.end, lanes.fps),
                        text::TIMECODE,
                        theme::ink3(),
                    )
                    .relative()
                    .flex_none()
                    .child(width_canvas(widths.length.clone())),
                )
            })
            .when(lanes.proposed, |left| {
                left.child(ui::chip("Proposed cut", theme::pos()))
            })
            .when(shows.length, |left| {
                left.child(div().w(px(TOOLS_SPACE)).flex_none())
            })
            .child(tools)
            .when(other_tool, |left| {
                left.child(ui::tipped(
                    "timeline-tool-other",
                    ui::Tip::new(format!("{} tool", self.tool.name())).maybe_key(tool_key),
                    ui::chip(self.tool.name(), theme::accent_ink()),
                ))
            });
        let right = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(STRIP_GAP))
            .child(width_canvas(widths.right.clone()))
            // Its name and key in a kit tip, as Qt's strip said them.
            .child(ui::tipped(
                "timeline-snap-tip",
                ui::Tip::new("Snap").maybe_key(snap_key),
                strip_button("timeline-snap", "Snap", self.snap).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.snap = !this.snap;
                        cx.notify();
                    },
                )),
            ))
            .child(ui::hairline(false).h(px(14.0)))
            .child(ui::tipped(
                "timeline-zoom-out-tip",
                ui::Tip::new("Zoom out").key(zoom_out),
                self.zoom_button("timeline-zoom-out", "\u{2212}", 1.0 / ZOOM_STEP, cx),
            ))
            .when(shows.slider, |right| {
                right.child(ui::tipped(
                    "timeline-zoom-slider-tip",
                    ui::Tip::new("Timeline zoom"),
                    self.slider(share, cx),
                ))
            })
            .child(ui::tipped(
                "timeline-zoom-in-tip",
                ui::Tip::new("Zoom in").key(zoom_in),
                self.zoom_button("timeline-zoom-in", "+", ZOOM_STEP, cx),
            ))
            .child(
                strip_button("timeline-fit", "Fit", self.zoom.is_none()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = None;
                        this.left = 0.0;
                        cx.notify();
                    },
                )),
            );
        div()
            .h(px(layout::STRIP))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .gap(px(STRIP_GAP))
            .pl(px(if has_transport {
                layout::STRIP_EDGE
            } else {
                STRIP_START
            }))
            .pr(px(STRIP_END))
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::line())
            .child(width_canvas(widths.room.clone()))
            .child(
                div()
                    .flex()
                    .flex_shrink()
                    .min_w_0()
                    .overflow_hidden()
                    .child(left),
            )
            .child(div().flex_1())
            .child(right)
    }

    /// The strip's − or +: one step of zoom by `factor` a click (on its
    /// release), and held down, a step after [`ZOOM_REPEAT_DELAY`] and
    /// every [`ZOOM_REPEAT`] after it until it is let go or the pointer
    /// leaves it, as Qt's autoRepeat buttons did.
    fn zoom_button(
        &self,
        id: &'static str,
        words: &'static str,
        factor: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        strip_button(id, words, false)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| this.hold_zoom(factor, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.zoom_held = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.zoom_held = None),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !*hovered {
                    this.zoom_held = None;
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_held = None;
                this.zoom_by(factor, None);
                cx.notify();
            }))
    }

    /// − or + pressed: while it is held, a step of zoom after
    /// [`ZOOM_REPEAT_DELAY`] and every [`ZOOM_REPEAT`] after that. Letting
    /// go (or leaving it) drops the task, which stops it.
    fn hold_zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.zoom_held = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ZOOM_REPEAT_DELAY).await;
            loop {
                let held = this.update(cx, |this, cx| {
                    this.zoom_by(factor, None);
                    cx.notify();
                });
                if held.is_err() {
                    break;
                }
                cx.background_executor().timer(ZOOM_REPEAT).await;
            }
        }));
    }

    /// The zoom's slider: a rail, its fill to the knob, the knob; pressed
    /// or dragged, the zoom follows it on a log scale.
    fn slider(&self, share: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = self.slider.clone();
        let knob = 10.0;
        let travel = SLIDER - knob;
        let at = share * travel;
        div()
            .id("timeline-zoom-slider")
            .relative()
            .flex_none()
            .w(px(SLIDER))
            .h(px(22.0))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.gesture = Some(Gesture::Slider);
                    this.slide_zoom(event.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(knob / 2.0))
                    .right(px(knob / 2.0))
                    .top(px(10.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(theme::track()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(knob / 2.0))
                    .w(px(at))
                    .top(px(10.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(theme::ink3()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(at))
                    .top(px(6.0))
                    .size(px(knob))
                    .rounded_full()
                    .bg(theme::ink())
                    .border_1()
                    .border_color(theme::line2()),
            )
    }

    fn heads(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let lanes = self.lanes.clone();
        let scroll_y = self.view.scroll_y;
        let rate = lanes.fps;
        let rate = if (rate - rate.round()).abs() < 0.005 {
            format!("{} fps", rate.round())
        } else {
            format!("{rate:.2} fps")
        };
        let targets = self.targets_now(cx);
        let read_only = lanes.proposed || self.offline(cx);
        let rows: Vec<_> = lanes
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.head(index, row, scroll_y, read_only, &targets, cx))
            .collect();
        let add_top = lanes.height - scroll_y;
        let add_hover = self.add_hover;
        div()
            .w(px(layout::TRACK_HEADER))
            .flex_none()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(theme::chrome())
            .border_r_1()
            .border_color(theme::line())
            .on_scroll_wheel(cx.listener(Self::heads_wheel))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(layout::RULER))
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(ui::label(rate, text::META, theme::ink3())),
            )
            .child(
                div()
                    .absolute()
                    .top(px(layout::RULER))
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .overflow_hidden()
                    .children(rows)
                    // Under the last track: a row whose + adds one.
                    .when(!read_only, |heads| {
                        heads.child(
                            div()
                                .id("timeline-add-row")
                                .absolute()
                                .top(px(add_top))
                                .left_0()
                                .right_0()
                                .h(px(ADD_ROW))
                                .flex()
                                .items_center()
                                .pl(px(6.0))
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    this.add_hover = *hovered;
                                    cx.notify();
                                }))
                                .child(ui::tipped(
                                    "timeline-add-track-tip",
                                    ui::Tip::new("Add track"),
                                    div()
                                        .id("timeline-add-track")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(26.0))
                                        .h(px(18.0))
                                        .rounded(px(radius::SMALL))
                                        .border_1()
                                        .border_color(if add_hover {
                                            theme::lineh()
                                        } else {
                                            theme::line()
                                        })
                                        .cursor_pointer()
                                        .child(ui::label(
                                            "+",
                                            text::CONTROL_STRONG,
                                            if add_hover {
                                                theme::ink()
                                            } else {
                                                theme::ink3()
                                            },
                                        ))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(
                                                |this, event: &MouseDownEvent, window, cx| {
                                                    let at = point(
                                                        event.position.x,
                                                        event.position.y + px(10.0),
                                                    );
                                                    this.add_track_menu(at, false, window, cx);
                                                    window.prevent_default();
                                                },
                                            ),
                                        ),
                                )),
                        )
                    }),
            )
    }

    fn head(
        &self,
        index: usize,
        row: &Row,
        scroll_y: f32,
        read_only: bool,
        targets: &[String],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label_ink = if row.quiet() {
            theme::ink3()
        } else {
            theme::ink2()
        };
        let control = (row.height - 6.0).clamp(16.0, 22.0);
        let lock = {
            let (track, locked) = (row.id.clone(), row.locked);
            ui::icon_button(
                ElementId::Name(format!("track-lock-{}", row.id).into()),
                Icon::Lock,
            )
            .size(control)
            .on(locked)
            .disabled(read_only)
            .tooltip(if locked { "Unlock track" } else { "Lock track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "locked", !locked, cx)),
            )
        };
        let switch = if row.audio() {
            let (track, muted) = (row.id.clone(), row.muted);
            ui::icon_button(
                ElementId::Name(format!("track-mute-{}", row.id).into()),
                Icon::Mute,
            )
            .size(control)
            .on(muted)
            .disabled(read_only)
            .tooltip(if muted { "Unmute" } else { "Mute" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "muted", !muted, cx)),
            )
        } else {
            let (track, enabled) = (row.id.clone(), row.enabled);
            ui::icon_button(
                ElementId::Name(format!("track-eye-{}", row.id).into()),
                Icon::Eye,
            )
            .size(control)
            .color(if enabled { theme::ink3() } else { theme::dis() })
            .disabled(read_only)
            .tooltip(if enabled { "Hide track" } else { "Show track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "enabled", !enabled, cx)),
            )
        };
        // Solo, for sound: what playback hears alone, for this session;
        // nothing to hear while kerfd is not connected.
        let offline = self.offline(cx);
        let solo = row.audio().then(|| {
            let track = row.id.clone();
            let on = self.solo.contains(&row.id);
            ui::tipped(
                ElementId::Name(format!("track-solo-tip-{}", row.id).into()),
                ui::Tip::new("Solo (monitoring only)"),
                if offline {
                    // Greyed, with no hover and nothing to press.
                    theme::type_style(div(), text::CONTROL)
                        .id(ElementId::Name(format!("track-solo-{}", row.id).into()))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .h(px(control))
                        .min_w(px(control))
                        .rounded(px(radius::CONTROL))
                        .text_color(theme::dis())
                        .child("S")
                } else {
                    strip_button_owned(
                        ElementId::Name(format!("track-solo-{}", row.id).into()),
                        "S",
                        on,
                    )
                    .h(px(control))
                    .min_w(px(control))
                    .px(px(0.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_solo(&track, cx)))
                },
            )
        });
        // The kind chip, a target for picture and sound: where source edits land.
        let targetable = row.kind != TrackKind::Subtitle && !read_only;
        let targeted = targetable && targets.contains(&row.id);
        let colour = kind_colour(row.kind);
        let chip = {
            let track = row.id.clone();
            div()
                .id(ElementId::Name(format!("track-target-{}", row.id).into()))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(26.0))
                .h(px((row.height - 8.0).clamp(14.0, 18.0)))
                .rounded(px(radius::SMALL))
                .border_1()
                .border_color(if targeted {
                    colour
                } else {
                    theme::with_alpha(colour, if row.quiet() { 0.3 } else { 0.6 })
                })
                .when(targeted, |chip| chip.bg(theme::with_alpha(colour, 0.85)))
                .child(ui::label(
                    row.badge.clone(),
                    text::BADGE,
                    if targeted {
                        theme::surround()
                    } else {
                        label_ink
                    },
                ))
                .when(targetable, |chip| {
                    chip.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_target(&track, cx)))
                })
        };
        let chip_tip = if !targetable {
            format!("{} · {}", row.badge, row.label)
        } else if targeted {
            format!("{} · Targeted", row.badge)
        } else {
            format!("{} · Target track", row.badge)
        };
        let grip = {
            let (track, kind, height) = (row.id.clone(), row.kind, row.height);
            let words = format!(
                "Drag to resize {}. Option-drag resizes every {} track; Option-scroll over the heads resizes every track.",
                row.badge,
                if row.audio() { "sound" } else { "picture" }
            );
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(GRIP + 1.0))
                .child(ui::tipped(
                    ElementId::Name(format!("track-grip-tip-{}", row.id).into()),
                    ui::Tip::new(words),
                    div()
                        .id(ElementId::Name(format!("track-grip-{}", row.id).into()))
                        .w(px(layout::TRACK_HEADER))
                        .h(px(GRIP + 1.0))
                        .cursor(CursorStyle::ResizeUpDown)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, _| {
                                this.gesture = Some(Gesture::Resize(ResizeDrag {
                                    track: track.clone(),
                                    kind,
                                    every: event.modifiers.alt,
                                    anchor: f32::from(event.position.y),
                                    from: height,
                                }));
                            }),
                        ),
                ))
        };
        div()
            .id(ElementId::Name(format!("track-head-{}", row.id).into()))
            .absolute()
            .top(px(row.top - scroll_y))
            .left_0()
            .right_0()
            .h(px(row.height))
            .flex()
            .items_center()
            .gap(px(2.0))
            .pl(px(6.0))
            .pr(px(4.0))
            .border_b_1()
            .border_color(theme::line())
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.head_menu(index, event.position, window, cx);
                    window.prevent_default();
                }),
            )
            .child(ui::tipped(
                ElementId::Name(format!("track-target-tip-{}", row.id).into()),
                ui::Tip::new(chip_tip),
                chip,
            ))
            .child(div().w(px(4.0)).flex_none())
            // Its name, when it is more than its badge says.
            .child(
                ui::line(
                    if row.label == row.badge {
                        SharedString::default()
                    } else {
                        row.label.clone()
                    },
                    text::META,
                    label_ink,
                )
                .flex_1()
                .min_w_0(),
            )
            .children(solo)
            .child(switch)
            .child(lock)
            .when(!read_only, |head| head.child(grip))
    }
}

/// A small button of words on the strip, held down when `on`.
fn strip_button(id: &'static str, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    strip_button_owned(id.into(), words, on)
}

fn strip_button_owned(id: ElementId, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    theme::type_style(div(), text::CONTROL)
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.0))
        .min_w(px(22.0))
        .px(px(7.0))
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .text_color(if on { theme::ink() } else { theme::ink2() })
        .when(on, |button| button.bg(theme::on()))
        .when(!on, |button| {
            button.hover(|style| style.bg(theme::raised()).text_color(theme::ink()))
        })
        .child(words)
}

impl Focusable for TimelineView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.get();
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.at.elapsed() > PENDING)
        {
            self.pending = None;
        }
        let lanes = self.lanes.clone();
        let session = self.session.read(cx);
        let state = &session.state;
        // Skimming the picture or the lanes moves what it shows, not the
        // playhead: the picture keeps the window's playhead where it was.
        let skim = self.skim.or_else(|| crate::stage::skim(&self.session, cx));
        let position = state.position;
        let (mark_in, mark_out, _) = session.marks();
        let selection = if lanes.proposed {
            Vec::new()
        } else {
            session.selection.clone()
        };
        let kerf = state.work.busy().then(|| {
            (
                self.kerf_at.or_else(|| state.agent_focus.first().copied()),
                state.agent_clips.clone(),
            )
        });
        let mut activities: Vec<Activity> = Vec::new();
        for clip in lanes.rows.iter().flat_map(|row| &row.clips) {
            let Some(media) = clip.source.as_ref().filter(|_| !clip.card()) else {
                continue;
            };
            if activities.iter().any(|known| known.media == *media) {
                continue;
            }
            let activity = state.media_activity(media);
            if let Some(label) = activity_label(&activity) {
                let phase = match &activity {
                    MediaActivity::Making { since, .. } => {
                        motion::phase(*since, theme::motion::STRIPE_CYCLE)
                    }
                    _ => 0.0,
                };
                activities.push(Activity {
                    media: media.clone(),
                    state: activity,
                    phase,
                    label: label.into(),
                });
            }
        }
        self.view = self.layout(width, height, position);
        let view = self.view;

        let ghost = match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.moved => {
                let (start, duration) = drag.span(lanes.fps);
                Some(Ghost {
                    id: drag.id.clone(),
                    row: drag.target,
                    start,
                    duration,
                    edge: match drag.edit {
                        Edit::Move => None,
                        Edit::TrimIn => Some(Zone::In),
                        Edit::TrimOut => Some(Zone::Out),
                    },
                    dragging: true,
                    head_shift: if drag.edit == Edit::TrimIn {
                        drag.frames as f64 / lanes.fps
                    } else if drag.tool == Tool::Slip {
                        -(drag.frames as f64) / lanes.fps
                    } else {
                        0.0
                    },
                })
            }
            _ => self
                .pending
                .as_ref()
                .filter(|pending| !pending.ghost.id.is_empty())
                .map(|pending| pending.ghost.clone()),
        };
        let (snapped, readout) = match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.moved => {
                let (edge, delta) = drag.readout(lanes.fps);
                (drag.snapped, Some((edge, delta, drag.target)))
            }
            _ => (None, None),
        };
        let cursor = match &self.gesture {
            Some(Gesture::Clip(drag)) => Some(match (drag.edit, drag.tool) {
                (Edit::Move, Tool::Selection) => CursorStyle::ClosedHand,
                (Edit::Move, _) => CursorStyle::ResizeLeftRight,
                (_, Tool::Roll) => CursorStyle::ResizeColumn,
                _ => CursorStyle::ResizeLeftRight,
            }),
            Some(Gesture::Band(_)) | Some(Gesture::Resize(_)) => Some(CursorStyle::ResizeUpDown),
            Some(Gesture::Signal(_)) => Some(CursorStyle::Crosshair),
            _ => None,
        };
        let hover_cursor = self.hover.as_ref().map(|hover| {
            let locked = lanes
                .find(&hover.clip)
                .is_some_and(|(row, _)| lanes.rows[row].locked);
            if hover.band {
                CursorStyle::ResizeUpDown
            } else if locked {
                CursorStyle::Arrow
            } else {
                self.tool.cursor(hover.zone != Zone::Body)
            }
        });
        let gain = self.gain_over();
        let signal = match &self.gesture {
            Some(Gesture::Signal(draw)) => Some((draw.clip.clone(), draw.drawn.clone())),
            _ => self
                .drawn
                .as_ref()
                .map(|(clip, drawn, _)| (clip.clone(), drawn.clone())),
        };
        // The stripes move only while a clip shows them.
        if activities.iter().any(Activity::making) && !motion::reduced() {
            window.request_animation_frame();
        }
        // While a change reel plays, what it watches rises off the lanes.
        let now = Instant::now();
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
            now,
        );
        if self.watch.running_at(now) {
            window.request_animation_frame();
        }
        let watch = self.watch.shown_at(now);

        let frame = Frame {
            lanes: lanes.clone(),
            view,
            playhead: position,
            skimmer: skim,
            marks: (mark_in, mark_out),
            selection,
            hover: self.hover.clone(),
            ghost,
            snapped,
            kerf,
            activities,
            cursor,
            tool: self.tool,
            zoomed: self.zoom.is_some(),
            readout,
            gain,
            signal,
            watch,
            session: self.session.clone(),
            this: cx.entity().downgrade(),
        };
        let cell = self.bounds.clone();
        let canvas = canvas(
            move |bounds, window, _| {
                // A new size lays the lanes out again on the next frame.
                if cell.get().size != bounds.size {
                    window.request_animation_frame();
                }
                cell.set(bounds);
            },
            move |bounds, (), window, cx| paint(&frame, bounds, window, cx),
        )
        .size_full();

        let tip = self.tip.clone().map(|tip| {
            let (x, y, w, h) = tip.bounds;
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w.max(1.0)))
                .h(px(h.max(1.0)))
                .child(ui::tipped(
                    ElementId::Name(format!("timeline-tip-{}", tip.key).into()),
                    tip.tip,
                    div().w(px(w.max(1.0))).h(px(h.max(1.0))),
                ))
        });
        let empty = (!lanes.has_clips()).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(layout::RULER))
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::label(
                    "No cut yet \u{2014} ask Kerf, or import media",
                    text::BODY,
                    theme::ink3(),
                ))
        });
        let lanes_area = div()
            .flex_1()
            .h_full()
            .relative()
            .overflow_hidden()
            .when_some(hover_cursor, |area, cursor| area.cursor(cursor))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::context_press))
            .on_mouse_move(cx.listener(Self::hover_at))
            .child(canvas)
            .children(empty)
            .children(tip);
        let skimming = self.skim.is_some() || crate::stage::skim(&self.session, cx).is_some();
        let strip = self.strip(skim.unwrap_or(position), skimming, cx);
        let heads = self.heads(cx);
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(heads)
            .child(lanes_area);
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(open.menu.clone()),
            )
            .with_priority(2)
        });

        div()
            .key_context(crate::commands::TIMELINE)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::run_command))
            .on_key_down(cx.listener(Self::key_down))
            // ⇧R ⇧E ⇧P: the picture's mask tools, for the clip chosen here.
            .on_action(cx.listener(|this, _: &stage::RectangleMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Rectangle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &stage::EllipseMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Ellipse, window, cx);
            }))
            .on_action(cx.listener(|this, _: &stage::PenMask, window, cx| {
                stage::toggle_mask_tool(&this.session, MaskTool::Pen, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::well())
            .child(strip)
            .child(body)
            .children(menu)
    }
}

// ----------------------------------------------------------------- drawing

/// What one frame of the canvas draws.
struct Frame {
    lanes: Rc<Lanes>,
    view: Viewport,
    playhead: f64,
    /// Where the pointer skims the picture or the lanes, while it does.
    skimmer: Option<f64>,
    /// The program's in and out marks.
    marks: (Option<f64>, Option<f64>),
    selection: Vec<String>,
    hover: Option<Hover>,
    ghost: Option<Ghost>,
    snapped: Option<f64>,
    /// While Kerf works: where its cursor stands, and the clips it looks at.
    kerf: Option<(Option<f64>, Vec<String>)>,
    /// What the media on the lanes are doing, where it is more than ready.
    activities: Vec<Activity>,
    /// The window's cursor while a drag holds it.
    cursor: Option<CursorStyle>,
    tool: Tool,
    /// Zoomed in: a bar under the lanes shows where the view is.
    zoomed: bool,
    /// A drag's readout: the edge's time, how far it moved, its row.
    readout: Option<(f64, f64, usize)>,
    /// A rubber band as dragged, or as just let go: its keys and values.
    gain: Option<GainShown>,
    /// A signal as it is drawn, or as it was drawn until kerfd has it.
    signal: Option<(String, BTreeMap<i64, f64>)>,
    /// While a change reel plays (and as it settles back): the clips it
    /// watches, and how far into watching, 0..1.
    watch: Option<(Rc<HashSet<String>>, f32)>,
    session: Entity<Session>,
    this: WeakEntity<TimelineView>,
}

impl Frame {
    /// How `clip` on `row` is drawn while a change reel watches.
    fn watch_look(&self, row: &Row, clip: &ClipBox) -> WatchLook {
        self.watch
            .as_ref()
            .map(|(clips, amount)| watch_look(*amount, clips.contains(&clip.id), row.audio()))
            .unwrap_or_default()
    }

    fn activity(&self, clip: &ClipBox) -> Option<&Activity> {
        if clip.card() {
            return None;
        }
        let media = clip.source.as_ref()?;
        self.activities
            .iter()
            .find(|activity| activity.media == *media)
    }
}

/// Points on the canvas, as the window places them.
#[derive(Clone, Copy)]
struct Pen {
    x: f32,
    y: f32,
}

impl Pen {
    fn rect(&self, x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.x + x), px(self.y + y)),
            size(px(width.max(0.0)), px(height.max(0.0))),
        )
    }

    fn at(&self, x: f32, y: f32) -> Point<Pixels> {
        point(px(self.x + x), px(self.y + y))
    }
}

struct Fonts {
    ui: Font,
    strong: Font,
    mono: Font,
}

impl Fonts {
    fn new() -> Self {
        Self {
            ui: theme::ui_font(FontWeight::NORMAL),
            strong: theme::ui_font(FontWeight::MEDIUM),
            mono: theme::mono_font(FontWeight::NORMAL),
        }
    }
}

fn shape(
    window: &Window,
    words: SharedString,
    style: TypeStyle,
    font: &Font,
    color: Rgba,
) -> ShapedLine {
    let run = TextRun {
        len: words.len(),
        font: font.clone(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(words, px(style.px), &[run], None)
}

fn masked<R>(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    draw: impl FnOnce(&mut Window) -> R,
) -> R {
    window.with_content_mask(Some(ContentMask { bounds }), draw)
}

fn paint(frame: &Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    listen(frame, window);
    if let Some(cursor) = frame.cursor {
        window.set_window_cursor_style(cursor);
    }
    let view = &frame.view;
    if view.width <= 0.0 {
        // Laid out for the first time: drawn on the next frame, at its size.
        return;
    }
    let pen = Pen {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
    };
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let fonts = Fonts::new();
    let lanes_box = pen.rect(0.0, layout::RULER, width, height - layout::RULER);
    masked(window, lanes_box, |window| {
        paint_lanes(frame, pen, width, height, &fonts, window, cx)
    });
    paint_ruler(frame, pen, width, &fonts, window, cx);
    if let Some((Some(time), _)) = &frame.kerf {
        paint_kerf(view, *time, pen, width, height, &fonts, window, cx);
    }
    paint_playhead(view, frame.playhead, pen, width, height, window);
    if let Some(time) = frame.skimmer {
        let x = view.x(time).floor();
        if (0.0..width).contains(&x) {
            window.paint_quad(fill(pen.rect(x, 0.0, 1.0, height), theme::ink2()));
        }
    }
    if frame.zoomed {
        // Where the view is along the program.
        let (left, across) = thumb(view, frame.lanes.end);
        let bar = pen.rect(left, height - SCROLLER - 2.0, across, SCROLLER);
        window.paint_quad(quad(
            bar,
            px(SCROLLER / 2.0),
            theme::with_alpha(theme::ink(), 0.22),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// While the canvas shows, a drag follows the pointer anywhere in the
/// window and ends wherever its button comes up.
fn listen(frame: &Frame, window: &mut Window) {
    let this = frame.this.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture {
            this.update(cx, |view, cx| view.pointer_moved(event, cx))
                .ok();
        }
    });
    let this = frame.this.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
            this.update(cx, |view, cx| view.release(event.position, cx))
                .ok();
        }
    });
}

fn paint_lanes(
    frame: &Frame,
    pen: Pen,
    width: f32,
    height: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    let visible = |row: &Row| {
        let y = view.row_y(row);
        y < height && y + row.height > layout::RULER
    };
    for row in lanes.rows.iter().filter(|row| visible(row)) {
        let y = view.row_y(row);
        if row.locked {
            let hatch = pattern_slash(theme::with_alpha(theme::ink(), 0.05).into(), 1.0, 7.0);
            window.paint_quad(fill(pen.rect(0.0, y, width, row.height), hatch));
        }
        // A veil over what each open proposal changes on it.
        for &(start, end) in &row.veils {
            let (x0, x1) = (view.x(start).max(-1.0), view.x(end).min(width + 1.0));
            if x1 > x0 {
                window.paint_quad(fill(
                    pen.rect(x0, y, x1 - x0, row.height),
                    theme::with_alpha(theme::ink(), 0.035),
                ));
            }
        }
        window.paint_quad(fill(
            pen.rect(0.0, y + row.height - 1.0, width, 1.0),
            theme::line(),
        ));
    }
    // After the program's end the lanes are quieter.
    let end_x = view.x(lanes.end).max(0.0);
    if end_x < width {
        let after = pen.rect(end_x, layout::RULER, width - end_x, height - layout::RULER);
        window.paint_quad(fill(after, theme::with_alpha(theme::surround_dim(), 0.45)));
    }
    for mark in &lanes.markers {
        let x = view.x(mark.time).floor();
        if (0.0..width).contains(&x) {
            let line = pen.rect(x, layout::RULER, 1.0, height - layout::RULER);
            window.paint_quad(fill(line, theme::with_alpha(mark.color, 0.22)));
        }
    }
    let ghost = frame.ghost.as_ref();
    for row in lanes.rows.iter().filter(|row| visible(row)) {
        let y = view.row_y(row);
        // Where a previewed proposal's clips are now, dashed, under its own.
        for &(start, end) in &row.ghosts {
            if let Some((left, top, across, tall)) = clip_box(view, width, row, y, start, end) {
                window.paint_quad(quad(
                    pen.rect(left, top, across, tall),
                    px(radius::SMALL),
                    theme::with_alpha(theme::ink(), 0.03),
                    px(1.0),
                    theme::with_alpha(theme::ink(), 0.3),
                    BorderStyle::Dashed,
                ));
            }
        }
        for clip in &row.clips {
            match ghost.filter(|ghost| ghost.id == clip.id && !row.previewed) {
                // Where a dragged clip was shows as an outline; a dropped one is drawn where it went.
                Some(ghost) if ghost.dragging => {
                    outline_clip(view, pen, width, row, y, clip.start, clip.end(), window)
                }
                Some(_) => {}
                None => {
                    let look = frame.watch_look(row, clip);
                    paint_watched(view, pen, width, row, y, clip, look, false, window);
                    paint_clip(
                        frame,
                        pen,
                        width,
                        row,
                        y - look.lift,
                        clip,
                        (clip.start, clip.duration),
                        None,
                        0.0,
                        fonts,
                        window,
                        cx,
                    );
                    paint_watched(view, pen, width, row, y, clip, look, true, window);
                }
            }
        }
        for &(start, end) in &row.transitions {
            paint_transition(view, pen, width, row, y, start, end, window);
        }
    }
    if let Some(ghost) = ghost {
        let found = lanes.find(&ghost.id);
        if let (Some((from, index)), Some(row)) = (found, lanes.rows.get(ghost.row)) {
            let clip = &lanes.rows[from].clips[index];
            let y = view.row_y(row);
            paint_clip(
                frame,
                pen,
                width,
                row,
                y,
                clip,
                (ghost.start, ghost.duration),
                ghost.edge,
                ghost.head_shift,
                fonts,
                window,
                cx,
            );
            // Where a moved clip will overwrite a neighbour, a dashed box inside it.
            if ghost.dragging && ghost.edge.is_none() && frame.tool != Tool::Slip {
                let (g0, g1) = (view.x(ghost.start), view.x(ghost.start + ghost.duration));
                for other in row.clips.iter().filter(|other| other.id != ghost.id) {
                    let (o0, o1) = (view.x(other.start), view.x(other.end()));
                    let (left, right) = (g0.max(o0).max(-4.0), g1.min(o1).min(width + 4.0));
                    if right - left < 6.0 {
                        continue;
                    }
                    let top = y + CLIP_INSET + 2.5;
                    let tall = row.height - 2.0 * CLIP_INSET - 5.0;
                    window.paint_quad(quad(
                        pen.rect(left + 2.5, top, right - left - 5.0, tall),
                        px(0.0),
                        gpui::transparent_black(),
                        px(1.0),
                        theme::ink(),
                        BorderStyle::Dashed,
                    ));
                }
            }
        }
    }
    if let Some(time) = frame.snapped {
        let x = view.x(time).floor();
        window.paint_quad(fill(
            pen.rect(x, layout::RULER, 1.0, height - layout::RULER),
            theme::accent_ink(),
        ));
    }
    if let Some((edge, delta, row)) = frame.readout {
        if let Some(row) = lanes.rows.get(row) {
            paint_readout(
                view,
                edge,
                delta,
                view.row_y(row),
                lanes.fps,
                pen,
                width,
                fonts,
                window,
                cx,
            );
        }
    }
}

/// By a dragged edge: its new timecode and how far it moved,
/// "00:00:12:04  +00:00:01:02".
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_readout(
    view: &Viewport,
    edge: f64,
    delta: f64,
    row_y: f32,
    fps: f64,
    pen: Pen,
    width: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let words = format!(
        "{}  {}{}",
        model::timecode(edge, fps),
        if delta < 0.0 { "-" } else { "+" },
        model::timecode(delta.abs(), fps)
    );
    let line = shape(
        window,
        words.into(),
        text::MONO_SMALL,
        &fonts.mono,
        theme::ink(),
    );
    let across = f32::from(line.width) + 16.0;
    let x = view.x(edge);
    let mut left = x + 6.0;
    if left + across > width - 4.0 {
        left = x - 6.0 - across;
    }
    let top = (row_y - 22.0).max(layout::RULER + 2.0);
    window.paint_quad(quad(
        pen.rect(left, top, across, 20.0),
        px(radius::CONTROL),
        theme::card(),
        px(1.0),
        theme::line2(),
        BorderStyle::Solid,
    ));
    let _ = line.paint(
        pen.at(left + 8.0, top + (20.0 - text::MONO_SMALL.line) / 2.0),
        px(text::MONO_SMALL.line),
        window,
        cx,
    );
}

/// What a change reel draws round `clip` on `row` (at `y`), as `look`
/// says: under it (`over` false), the glow it rises over; over it, the
/// cover on a clip it does not watch, or the accent's ring round one it
/// does, where it has risen to (Qt's timeline, while watching changes).
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_watched(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    clip: &ClipBox,
    look: WatchLook,
    over: bool,
    window: &mut Window,
) {
    if look == WatchLook::default() {
        return;
    }
    let Some((left, top, across, tall)) = clip_box(view, width, row, y, clip.start, clip.end())
    else {
        return;
    };
    let top = top - look.lift;
    if !over {
        if look.glow > 0.0 {
            let glow = theme::lane::watch_glow();
            window.paint_quad(quad(
                pen.rect(left - 4.0, top - 2.0, across + 8.0, tall + 10.0),
                px(radius::SMALL + 2.0),
                theme::with_alpha(glow, glow.a * look.glow),
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
        return;
    }
    if look.dim > 0.0 {
        let dim = theme::lane::watch_dim();
        window.paint_quad(fill(
            pen.rect(left, top, across, tall),
            theme::with_alpha(dim, dim.a * look.dim),
        ));
    }
    if look.ring > 0.0 {
        // Just outside it, as Qt's 2-point line round it was.
        window.paint_quad(quad(
            pen.rect(left - 2.0, top - 2.0, across + 4.0, tall + 4.0),
            px(radius::SMALL + 2.0),
            gpui::transparent_black(),
            px(2.0),
            theme::with_alpha(theme::accent_ink(), look.ring),
            BorderStyle::Solid,
        ));
    }
}

/// The box a clip from `start` to `end` takes on `row` drawn at `y`, kept
/// within a little of the canvas so a long clip makes no huge quad.
fn clip_box(
    view: &Viewport,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
) -> Option<(f32, f32, f32, f32)> {
    let x0 = view.x(start);
    let x1 = view.x(end).max(x0 + 2.0);
    if x1 < -8.0 || x0 > width + 8.0 {
        return None;
    }
    let left = x0.max(-8.0);
    let right = x1.min(width + 8.0);
    Some((
        left,
        y + CLIP_INSET,
        right - left,
        row.height - 2.0 * CLIP_INSET,
    ))
}

/// What a clip shows of its media under its name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Art {
    None,
    Strip,
    Wave,
}

// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_clip(
    frame: &Frame,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    clip: &ClipBox,
    (start, duration): (f64, f64),
    trimming: Option<Zone>,
    head_shift: f64,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let Some((left, top, across, tall)) = clip_box(view, width, row, y, start, start + duration)
    else {
        return;
    };
    let x0 = view.x(start);
    let x1 = view.x(start + duration).max(x0 + 2.0);
    let body = pen.rect(left, top, across, tall);
    let quiet = !clip.enabled || row.quiet();
    let selected = !row.previewed && frame.selection.contains(&clip.id);
    let hover = frame
        .hover
        .as_ref()
        .filter(|hover| hover.clip == clip.id && !row.previewed);
    let activity = frame.activity(clip);
    let absent = activity.is_some_and(|activity| activity.state.absent());
    let ground = if absent {
        theme::well()
    } else if clip.changed {
        theme::mix(theme::card(), theme::pos(), 0.16)
    } else {
        theme::card()
    };
    let (ground, edge) = if quiet {
        (
            theme::mix(theme::well(), ground, 0.55),
            theme::mix(theme::well(), clip.edge, 0.35),
        )
    } else {
        (ground, clip.edge)
    };
    let corner = px(radius::SMALL);
    // The edge colour fills it, and its ground covers all but its top.
    if !absent {
        window.paint_quad(quad(
            body,
            corner,
            edge,
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
    let under_edge = if absent {
        body
    } else {
        pen.rect(left, top + CLIP_EDGE, across, tall - CLIP_EDGE)
    };
    let bottom = Corners {
        top_left: px(if absent { radius::SMALL } else { 0.0 }),
        top_right: px(if absent { radius::SMALL } else { 0.0 }),
        bottom_right: corner,
        bottom_left: corner,
    };
    window.paint_quad(quad(
        under_edge,
        bottom,
        ground,
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    // What it holds: frames along a picture, a sound's waveform. A leading
    // trim being dragged moves the in point with it.
    let mut art = Art::None;
    if let (false, false, Some(source)) = (absent, clip.card(), clip.source.as_deref()) {
        let source_at = |offset: f64| clip.clip.source_at(offset + head_shift).unwrap_or(0.0);
        let inside = pen.rect(x0, top + CLIP_EDGE, x1 - x0, tall - CLIP_EDGE);
        if row.audio() {
            let peaks = frame.session.read(cx).artwork.peaks(source);
            if let Some(peaks) = peaks.filter(|peaks| !peaks.is_empty()) {
                let wave = pen.rect(x0, top + CLIP_EDGE + 1.0, x1 - x0, tall - CLIP_EDGE - 2.0);
                let gain = 10f64.powf(clip.clip.gain_db() / 20.0);
                masked(window, under_edge, |window| {
                    crate::artwork::paint_waveform(
                        window,
                        wave,
                        &peaks,
                        source_at(0.0),
                        source_at(duration),
                        lane_ink::peak(row.room),
                        lane_ink::rms(row.room),
                        gain,
                    )
                });
                art = Art::Wave;
            }
        } else {
            let artwork = &frame.session.read(cx).artwork;
            let painted = masked(window, under_edge, |window| {
                crate::artwork::paint_filmstrip(
                    window, inside, artwork, source, view.pps, source_at,
                )
            });
            if painted {
                art = Art::Strip;
            }
        }
        if art != Art::None && quiet {
            window.paint_quad(fill(under_edge, theme::with_alpha(theme::well(), 0.55)));
        }
    }
    if let Some(activity) = activity.filter(|activity| activity.making()) {
        stripes(
            window,
            under_edge,
            activity.phase,
            theme::activity::gpu_stripe(),
        );
    }
    // A signal's number over its time, filled under as a sound's level is,
    // and what is being drawn over it until the document has it.
    if clip.kind == ClipKind::Generator && !clip.clip.signal.is_empty() && x1 - x0 >= 4.0 {
        let inside = (x0, top + 2.0, x1 - x0, tall - 4.0);
        masked(window, under_edge, |window| {
            paint_signal(&clip.clip.signal, inside, pen, window);
            if let Some((_, drawn)) = frame.signal.as_ref().filter(|(id, _)| *id == clip.id) {
                paint_drawn(drawn, clip.duration_frames, inside, pen, window);
            }
        });
    }
    paint_name(
        frame,
        pen,
        width,
        row,
        clip,
        (x0, x1),
        (top, tall),
        art,
        quiet,
        duration,
        fonts,
        window,
        cx,
    );
    if let Some(activity) = activity {
        paint_activity(
            activity,
            pen,
            width,
            (x0, x1),
            (top, tall),
            fonts,
            window,
            cx,
        );
    }
    // A sound clip's volume rubber band.
    if row.audio() && !clip.card() && !row.previewed {
        let (keys, values, keyed) = gain_drawn(clip, frame.lanes.fps, frame.gain.as_ref());
        let dragged = frame.gain.as_ref().is_some_and(|gain| gain.clip == clip.id);
        let level = values.first().copied().unwrap_or(0.0);
        // At 0 dB with no keys it shows only where it can be dragged.
        let shown = keyed || level.abs() > 0.05 || hover.is_some() || selected || dragged;
        if shown {
            let (points, _) = band_line(&keys, &values, keyed, x0, x1 - x0, top, tall, view.pps);
            masked(window, body, |window| {
                paint_band(&points, keyed, pen, window)
            });
        }
    }
    // Its outline: ink chosen (as Qt drew it, read over frames); dashed while it waits its turn
    // (amber) or has no file (a hole); amber being made; sage when an open
    // proposal changes it; brighter under the pointer.
    let dashed = activity.is_some_and(Activity::dashed);
    let (outline, outline_width, style) = if selected {
        (theme::ink(), 1.5, BorderStyle::Solid)
    } else if dashed {
        let queued = activity.is_some_and(|activity| !activity.state.absent());
        (
            if queued {
                theme::activity::gpu()
            } else {
                theme::lineh()
            },
            1.0,
            BorderStyle::Dashed,
        )
    } else if activity.is_some_and(Activity::making) {
        (theme::activity::gpu(), 1.0, BorderStyle::Solid)
    } else if clip.changed || clip.proposed {
        (theme::pos(), 1.0, BorderStyle::Solid)
    } else if hover.is_some() {
        (theme::lineh(), 1.0, BorderStyle::Solid)
    } else {
        (theme::line2(), 1.0, BorderStyle::Solid)
    };
    window.paint_quad(quad(
        body,
        corner,
        gpui::transparent_black(),
        px(outline_width),
        outline,
        style,
    ));
    // The edge under the pointer, or the edge being trimmed.
    let edge_bar = trimming
        .or(hover
            .filter(|hover| !hover.band && frame.tool.trims_edges())
            .map(|hover| hover.zone))
        .filter(|zone| *zone != Zone::Body);
    if let Some(zone) = edge_bar {
        let x = if zone == Zone::In { x0 } else { x1 - 2.0 };
        masked(window, body, |window| {
            window.paint_quad(fill(pen.rect(x, top, 2.0, tall), theme::ink()));
        });
    }
    // Kerf looking at it rings it.
    let looked_at = frame
        .kerf
        .as_ref()
        .is_some_and(|(_, clips)| clips.contains(&clip.id));
    if looked_at {
        let ring = pen.rect(left - 2.0, top - 2.0, across + 4.0, tall + 4.0);
        window.paint_quad(quad(
            ring,
            px(radius::SMALL + 2.0),
            gpui::transparent_black(),
            px(1.5),
            theme::activity::kerf(),
            BorderStyle::Solid,
        ));
    }
}

/// A clip's name and what goes with it: a red dot when a check found a
/// problem in it, its speed or a title's text after it, an fx mark when it
/// has effects. Over a filmstrip it stands on a dark bar; over a waveform
/// it is shadowed; a caption's is in its middle; with nothing under it,
/// its length goes under it in a tall clip.
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_name(
    frame: &Frame,
    pen: Pen,
    width: f32,
    row: &Row,
    clip: &ClipBox,
    (x0, x1): (f32, f32),
    (top, tall): (f32, f32),
    art: Art,
    quiet: bool,
    duration: f64,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let start = view.time(x0);
    // Kept in sight while its start is scrolled off, and clear of a
    // transition coming into it.
    let into = row
        .transitions
        .iter()
        .filter(|(from, to)| *from <= start + 1e-6 && *to > start + 1e-6)
        .map(|(_, to)| view.x(*to) + 4.0)
        .fold(0.0, f32::max);
    let label_x = (x0.max(0.0) + 6.0).max(into);
    let room = x1.min(width) - label_x - 4.0;
    if room <= 10.0 {
        return;
    }
    let bar_top = top + CLIP_EDGE;
    let bar_tall = NAME_BAR.min(tall - CLIP_EDGE);
    if art == Art::Strip {
        let bar = pen.rect(
            x0.max(-2.0),
            bar_top,
            x1.min(width + 2.0) - x0.max(-2.0),
            bar_tall,
        );
        window.paint_quad(fill(bar, lane_ink::name_bar()));
    }
    let ink = if quiet { theme::dis() } else { theme::ink() };
    let label_y = if clip.kind == ClipKind::Caption || (art == Art::None && tall < 30.0) {
        top + (tall - text::META.line) / 2.0 + 1.0
    } else {
        bar_top + (bar_tall - text::META.line) / 2.0 + 1.0
    };
    let name = shape(window, clip.label.clone(), text::META, &fonts.ui, ink);
    let shadow = (art == Art::Wave).then(|| {
        shape(
            window,
            clip.label.clone(),
            text::META,
            &fonts.ui,
            lane_ink::shadow(),
        )
    });
    let secondary = (!clip.secondary.is_empty()).then(|| {
        shape(
            window,
            clip.secondary.clone(),
            text::META,
            &fonts.ui,
            theme::ink3(),
        )
    });
    let fx = clip.fx.then(|| {
        shape(
            window,
            "fx".into(),
            text::MONO_SMALL,
            &fonts.mono,
            theme::ink3(),
        )
    });
    let length = (art == Art::None
        && clip.kind != ClipKind::Caption
        && tall >= 36.0
        && room > f32::from(name.width) + 8.0)
        .then(|| {
            let ink = if quiet { theme::dis() } else { theme::ink3() };
            shape(
                window,
                model::length(duration).into(),
                text::MONO_SMALL,
                &fonts.mono,
                ink,
            )
        });
    masked(window, pen.rect(label_x, top, room, tall), |window| {
        let mut x = label_x;
        if clip.qc {
            let dot = pen.rect(x, label_y + (text::META.line - 6.0) / 2.0, 6.0, 6.0);
            window.paint_quad(quad(
                dot,
                px(3.0),
                theme::neg(),
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
            x += 10.0;
        }
        if let Some(shadow) = &shadow {
            let _ = shadow.paint(
                pen.at(x + 1.0, label_y + 1.0),
                px(text::META.line),
                window,
                cx,
            );
        }
        let _ = name.paint(pen.at(x, label_y), px(text::META.line), window, cx);
        x += f32::from(name.width) + 6.0;
        if let Some(secondary) = &secondary {
            let _ = secondary.paint(pen.at(x, label_y), px(text::META.line), window, cx);
            x += f32::from(secondary.width) + 6.0;
        }
        if let Some(fx) = &fx {
            let _ = fx.paint(pen.at(x, label_y), px(text::MONO_SMALL.line), window, cx);
        }
        if let Some(length) = &length {
            let _ = length.paint(
                pen.at(label_x, label_y + 15.0),
                px(text::MONO_SMALL.line),
                window,
                cx,
            );
        }
    });
}

/// What a clip's media is doing, in a pill at its foot: the making's
/// clock, "Queued", "Not made" (ink2), "Offline" (brick).
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_activity(
    activity: &Activity,
    pen: Pen,
    width: f32,
    (x0, x1): (f32, f32),
    (top, tall): (f32, f32),
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let ink = match activity.state {
        MediaActivity::Offline => theme::neg(),
        MediaActivity::NotMade => theme::ink2(),
        _ => theme::activity::gpu(),
    };
    let style = if activity.making() {
        text::MONO_SMALL
    } else {
        text::META
    };
    let font = if activity.making() {
        &fonts.mono
    } else {
        &fonts.ui
    };
    let line = shape(window, activity.label.clone(), style, font, ink);
    let across = f32::from(line.width) + 8.0;
    let right = x1.min(width) - 4.0;
    let left = right - across;
    if left < x0.max(0.0) + 4.0 || x1 - x0 < 24.0 {
        return;
    }
    // Under the name in a clip tall enough, else at its middle.
    let pill_top = if tall >= 30.0 {
        top + tall - 18.0
    } else {
        top + (tall - 16.0) / 2.0
    };
    let pill = pen.rect(left, pill_top, across, 16.0);
    masked(window, pen.rect(x0, top, x1 - x0, tall), |window| {
        window.paint_quad(quad(
            pill,
            px(radius::SMALL),
            theme::activity::chip(),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
        let _ = line.paint(
            pen.at(left + 4.0, pill_top + (16.0 - style.line) / 2.0),
            px(style.line),
            window,
            cx,
        );
    });
}

/// A rubber band: the line, and a diamond on each key.
fn paint_band(points: &[(f32, f32)], keyed: bool, pen: Pen, window: &mut Window) {
    let mut line = PathBuilder::stroke(px(1.0));
    for (index, (x, y)) in points.iter().enumerate() {
        if index == 0 {
            line.move_to(pen.at(*x, *y));
        } else {
            line.line_to(pen.at(*x, *y));
        }
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, lane_ink::rubber_band());
    }
    if keyed && points.len() > 2 {
        for (x, y) in &points[1..points.len() - 1] {
            let mut diamond = PathBuilder::fill();
            diamond.move_to(pen.at(*x, y - 4.0));
            diamond.line_to(pen.at(x + 4.0, *y));
            diamond.line_to(pen.at(*x, y + 4.0));
            diamond.line_to(pen.at(x - 4.0, *y));
            diamond.close();
            if let Ok(path) = diamond.build() {
                window.paint_path(path, lane_ink::key());
            }
        }
    }
}

/// A signal's values 0..1 spread across `(x, y, w, h)`, filled under.
fn paint_signal(values: &[f64], (x, y, w, h): (f32, f32, f32, f32), pen: Pen, window: &mut Window) {
    let points = signal_points(values, (x, y, w, h));
    if points.len() < 2 {
        return;
    }
    let mut under = PathBuilder::fill();
    under.move_to(pen.at(points[0].0, y + h));
    for (px_, py) in &points {
        under.line_to(pen.at(*px_, *py));
    }
    under.line_to(pen.at(points[points.len() - 1].0, y + h));
    under.close();
    if let Ok(path) = under.build() {
        window.paint_path(path, lane_ink::peak(false));
    }
    let mut line = PathBuilder::stroke(px(1.0));
    line.move_to(pen.at(points[0].0, points[0].1));
    for (px_, py) in &points[1..] {
        line.line_to(pen.at(*px_, *py));
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, lane_ink::rms(false));
    }
}

/// The points a signal's curve passes through across a box (Qt's
/// `signalcurve::curve`): one value is a level across the whole of it.
pub fn signal_points(values: &[f64], (x, y, w, h): (f32, f32, f32, f32)) -> Vec<(f32, f32)> {
    if values.is_empty() || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let last = values.len() - 1;
    let mut points: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let along = if last == 0 {
                0.0
            } else {
                index as f32 / last as f32
            };
            (x + along * w, y + h - value.clamp(0.0, 1.0) as f32 * h)
        })
        .collect();
    if last == 0 {
        points.push((x + w, points[0].1));
    }
    points
}

/// A signal being drawn by hand, over the clip's own, in ink.
fn paint_drawn(
    drawn: &BTreeMap<i64, f64>,
    frames: i64,
    (x, y, w, h): (f32, f32, f32, f32),
    pen: Pen,
    window: &mut Window,
) {
    let last = (frames - 1).max(1) as f32;
    let mut line = PathBuilder::stroke(px(1.5));
    for (index, (frame, value)) in drawn.iter().enumerate() {
        let at = pen.at(x + w * *frame as f32 / last, y + h - h * *value as f32);
        if index == 0 {
            line.move_to(at);
        } else {
            line.line_to(at);
        }
    }
    if drawn.len() == 1 {
        if let Some((frame, value)) = drawn.iter().next() {
            let (cx_, cy) = (x + w * *frame as f32 / last, y + h - h * *value as f32);
            window.paint_quad(fill(pen.rect(cx_ - 1.0, cy - 1.0, 2.0, 2.0), theme::ink()));
        }
        return;
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, theme::ink());
    }
}

/// Where a dragged clip was.
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn outline_clip(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
    window: &mut Window,
) {
    if let Some((left, top, across, tall)) = clip_box(view, width, row, y, start, end) {
        let body = pen.rect(left, top, across, tall);
        let ground = theme::with_alpha(theme::card(), 0.5);
        window.paint_quad(quad(
            body,
            px(radius::SMALL),
            ground,
            px(1.0),
            theme::line2(),
            BorderStyle::Dashed,
        ));
    }
}

/// A transition: a small hatched mark over its cut.
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_transition(
    view: &Viewport,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    start: f64,
    end: f64,
    window: &mut Window,
) {
    let (mut x0, mut x1) = (view.x(start), view.x(end));
    if x1 - x0 < 8.0 {
        let middle = (x0 + x1) / 2.0;
        (x0, x1) = (middle - 4.0, middle + 4.0);
    }
    if x1 < 0.0 || x0 > width {
        return;
    }
    let tall = row.height - 2.0 * CLIP_INSET;
    let mark = pen.rect(x0, y + CLIP_INSET + tall * 0.25, x1 - x0, tall * 0.5);
    let hatch = pattern_slash(theme::with_alpha(theme::ink(), 0.55).into(), 1.0, 3.0);
    let outline = theme::with_alpha(theme::ink2(), 0.7);
    window.paint_quad(quad(
        mark,
        px(3.0),
        hatch,
        px(1.0),
        outline,
        BorderStyle::Solid,
    ));
}

/// Bands leaning right, half a space wide, a space apart, moved on by
/// `phase` of a space, inside `area`.
fn stripes(window: &mut Window, area: Bounds<Pixels>, phase: f32, color: Rgba) {
    let space = theme::motion::STRIPE_SPACE;
    let (left, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
    let (right, bottom) = (
        left + f32::from(area.size.width),
        top + f32::from(area.size.height),
    );
    let tall = bottom - top;
    if tall <= 0.0 || right <= left {
        return;
    }
    let mut builder = PathBuilder::fill();
    let mut x = left - tall - space + phase.fract() * space;
    while x < right + space {
        builder.move_to(point(px(x), px(bottom)));
        builder.line_to(point(px(x + space / 2.0), px(bottom)));
        builder.line_to(point(px(x + space / 2.0 + tall), px(top)));
        builder.line_to(point(px(x + tall), px(top)));
        builder.close();
        x += space;
    }
    if let Ok(path) = builder.build() {
        masked(window, area, |window| window.paint_path(path, color));
    }
}

/// The interval between labelled ticks and between ticks, in seconds, at
/// `pps` points a second.
fn steps(pps: f32, fps: f64) -> (f64, f64) {
    let frame = 1.0 / fps.max(1.0);
    let ladder = [
        frame,
        2.0 * frame,
        5.0 * frame,
        10.0 * frame,
        0.5,
        1.0,
        2.0,
        5.0,
        10.0,
        15.0,
        30.0,
        60.0,
        120.0,
        300.0,
        600.0,
        900.0,
        1800.0,
        3600.0,
        7200.0,
    ];
    let pps = pps.max(f32::EPSILON) as f64;
    let mut rungs = ladder.iter().copied().scan(0.0, |last, step: f64| {
        let rising = step > *last + 1e-9;
        if rising {
            *last = step;
        }
        Some(rising.then_some(step))
    });
    let mut major = 7200.0;
    let mut below = Vec::with_capacity(ladder.len());
    for step in rungs.by_ref().flatten() {
        if step * pps >= LABEL_SPACE as f64 {
            major = step;
            break;
        }
        below.push(step);
    }
    let minor = below
        .iter()
        .rev()
        .copied()
        .find(|&step| {
            let parts = major / step;
            step * pps >= TICK_SPACE as f64 && (parts - parts.round()).abs() < 1e-6
        })
        .unwrap_or(major);
    (major, minor)
}

/// A ruler's label at `time`: minutes and seconds, with frames when the
/// labels are under a second apart, and hours once there are any.
fn ruler_label(time: f64, fps: f64, major: f64) -> String {
    let whole = fps.round().max(1.0) as i64;
    let total = (time.max(0.0) * fps).round() as i64;
    let (seconds, frames) = (total / whole, total % whole);
    let (hours, minutes, seconds) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    let clock = if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    };
    if major < 1.0 {
        format!("{clock}:{frames:02}")
    } else {
        clock
    }
}

fn paint_ruler(
    frame: &Frame,
    pen: Pen,
    width: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let lanes = &*frame.lanes;
    let ruler = pen.rect(0.0, 0.0, width, layout::RULER);
    window.paint_quad(fill(ruler, theme::chrome()));
    window.paint_quad(fill(
        pen.rect(0.0, layout::RULER - 1.0, width, 1.0),
        theme::line(),
    ));
    let (major, minor) = steps(view.pps, lanes.fps);
    masked(window, ruler, |window| {
        // Each open proposal's band, under everything.
        for band in &lanes.bands {
            let x0 = view.x(band.start);
            let x1 = view.x(band.end).max(x0 + 3.0);
            let (left, right) = (x0.max(-2.0), x1.min(width + 2.0));
            if right <= left {
                continue;
            }
            window.paint_quad(fill(
                pen.rect(left, 0.0, right - left, layout::RULER - 1.0),
                theme::with_alpha(theme::ink(), 0.05),
            ));
            window.paint_quad(quad(
                pen.rect(left, 1.0, right - left, BAND_ROW),
                px(BAND_ROW / 2.0),
                theme::with_alpha(theme::pos(), 0.6),
                px(0.0),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
        let mut index = (view.left / minor).floor() as i64;
        loop {
            let time = index as f64 * minor;
            index += 1;
            let x = view.x(time).floor();
            if x > width + 1.0 {
                break;
            }
            if time < -1e-9 {
                continue;
            }
            let parts = time / major;
            let labelled = (parts - parts.round()).abs() * major < minor * 0.25;
            let (tall, color) = if labelled {
                (9.0, theme::track2())
            } else {
                (4.0, theme::track())
            };
            window.paint_quad(fill(
                pen.rect(x, layout::RULER - 1.0 - tall, 1.0, tall),
                color,
            ));
            if labelled {
                let words: SharedString = ruler_label(time, lanes.fps, major).into();
                let line = shape(window, words, text::MONO_SMALL, &fonts.mono, theme::ink3());
                let _ = line.paint(pen.at(x + 4.0, 2.0), px(text::MONO_SMALL.line), window, cx);
            }
        }
        for mark in &lanes.markers {
            let x = view.x(mark.time);
            if mark.duration > 0.0 {
                let band = pen.rect(x, layout::RULER - 5.0, mark.duration as f32 * view.pps, 3.0);
                window.paint_quad(fill(band, theme::with_alpha(mark.color, 0.5)));
            }
            if (-6.0..width + 6.0).contains(&x) {
                let mut pin = PathBuilder::fill();
                pin.move_to(pen.at(x - 4.0, 11.0));
                pin.line_to(pen.at(x + 4.0, 11.0));
                pin.line_to(pen.at(x + 4.0, 17.0));
                pin.line_to(pen.at(x, 21.0));
                pin.line_to(pen.at(x - 4.0, 17.0));
                pin.close();
                if let Ok(path) = pin.build() {
                    window.paint_path(path, mark.color);
                }
            }
        }
        paint_marks(view, frame.marks, lanes.end, pen, width, window);
    });
}

/// The program's in and out marks: the range between them along the
/// ruler's foot, and each mark's line.
fn paint_marks(
    view: &Viewport,
    (mark_in, mark_out): (Option<f64>, Option<f64>),
    end: f64,
    pen: Pen,
    width: f32,
    window: &mut Window,
) {
    if mark_in.is_none() && mark_out.is_none() {
        return;
    }
    let from = view.x(mark_in.unwrap_or(0.0)).round();
    let to = view.x(mark_out.unwrap_or(end)).round();
    let (left, right) = (from.max(-1.0), to.min(width + 1.0));
    if right > left {
        let range = pen.rect(left, 0.0, right - left, layout::RULER - 1.0);
        window.paint_quad(fill(range, theme::with_alpha(theme::ink(), 0.06)));
        let foot = pen.rect(left, layout::RULER - 4.0, right - left, 3.0);
        window.paint_quad(fill(foot, theme::with_alpha(theme::ink(), 0.2)));
    }
    for (mark, x) in [(mark_in, from), (mark_out, to)] {
        if mark.is_some() && (-1.0..width + 1.0).contains(&x) {
            let line = pen.rect(x, layout::RULER - 11.0, 1.0, 10.0);
            window.paint_quad(fill(line, theme::ink()));
        }
    }
}

/// Kerf's cursor: a line in its blue with its light either side, and its name.
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
fn paint_kerf(
    view: &Viewport,
    time: f64,
    pen: Pen,
    width: f32,
    height: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let x = view.x(time).floor();
    if !(-12.0..width + 12.0).contains(&x) {
        return;
    }
    let kerf = theme::activity::kerf();
    let (lit, unlit) = (theme::with_alpha(kerf, 0.3), theme::with_alpha(kerf, 0.0));
    let rising = linear_gradient(
        90.0,
        linear_color_stop(unlit, 0.0),
        linear_color_stop(lit, 1.0),
    );
    let falling = linear_gradient(
        90.0,
        linear_color_stop(lit, 0.0),
        linear_color_stop(unlit, 1.0),
    );
    window.paint_quad(fill(pen.rect(x - 10.0, 0.0, 10.0, height), rising));
    window.paint_quad(fill(pen.rect(x + 2.0, 0.0, 10.0, height), falling));
    window.paint_quad(fill(pen.rect(x, 0.0, 2.0, height), kerf));
    let name = shape(
        window,
        "Kerf".into(),
        text::META_STRONG,
        &fonts.strong,
        theme::surround(),
    );
    let tag = pen.rect(
        x + 2.0,
        layout::RULER,
        f32::from(name.width) + 8.0,
        text::META.line,
    );
    let corners = Corners {
        top_left: px(0.0),
        top_right: px(radius::SMALL),
        bottom_right: px(radius::SMALL),
        bottom_left: px(0.0),
    };
    window.paint_quad(quad(
        tag,
        corners,
        kerf,
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    let _ = name.paint(
        pen.at(x + 6.0, layout::RULER),
        px(text::META.line),
        window,
        cx,
    );
}

/// The playhead: a line down the ruler and the lanes, with its head on the ruler.
fn paint_playhead(
    view: &Viewport,
    time: f64,
    pen: Pen,
    width: f32,
    height: f32,
    window: &mut Window,
) {
    let x = view.x(time).floor();
    if !(-8.0..width + 8.0).contains(&x) {
        return;
    }
    let color = theme::playhead();
    window.paint_quad(fill(pen.rect(x, 0.0, 1.0, height), color));
    let middle = x + 0.5;
    let mut head = PathBuilder::fill();
    head.move_to(pen.at(middle - 6.0, 0.0));
    head.line_to(pen.at(middle + 6.0, 0.0));
    head.line_to(pen.at(middle + 6.0, 8.0));
    head.line_to(pen.at(middle, 13.0));
    head.line_to(pen.at(middle - 6.0, 8.0));
    head.close();
    if let Ok(path) = head.build() {
        window.paint_path(path, color);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(value: serde_json::Value) -> Timeline {
        Timeline::from_value(&value)
    }

    fn lanes_of(
        current: &Timeline,
        first_cut: Option<&Timeline>,
        bands: Vec<ProposalBand>,
    ) -> Lanes {
        Lanes::build(Build {
            current,
            first_cut,
            bands,
            preview: None,
            heights: &TrackHeights::default(),
            qc: &HashSet::new(),
        })
    }

    #[test]
    fn a_trim_cannot_reach_past_the_media_or_to_no_frames() {
        let current = timeline(json!({
            "fps": 24.0,
            "sources": [{"id": "m", "path": "m.mp4", "duration": 10.0}],
            "tracks": [{"id": "V1", "type": "video", "enabled": true, "clips": [
                {"id": "a", "timelineStart": 2.0, "duration": 4.0, "startFrame": 48, "durationFrames": 96,
                 "enabled": true, "sourceId": "m", "in": 1.0, "out": 5.0}
            ]}]
        }));
        let lanes = lanes_of(&current, None, Vec::new());
        let clip = &lanes.rows[0].clips[0];
        assert_eq!((clip.head, clip.tail), (Some(24), Some(120)));
        let drag = |zone, tool, at| ClipDrag::new(&lanes, 0, 0, zone, tool, at, 30.0, 0.0);
        let trim_in = drag(Zone::In, Tool::Selection, 2.0);
        assert_eq!((trim_in.least, trim_in.most), (-24, 95));
        let trim_out = drag(Zone::Out, Tool::Selection, 6.0);
        assert_eq!((trim_out.least, trim_out.most), (-95, 120));
        let moving = drag(Zone::Body, Tool::Selection, 3.0);
        assert_eq!(moving.least, -48);
        // A slip reaches as far as the media goes either way.
        let slip = drag(Zone::Body, Tool::Slip, 3.0);
        assert_eq!((slip.least, slip.most), (-120, 24));
    }
}
