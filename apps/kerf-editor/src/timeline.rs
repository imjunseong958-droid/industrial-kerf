//! The timeline: the program's tracks and clips, its ruler and playhead.
//!
//! A strip along the top says where the playhead is and holds snap and
//! zoom; under it the track heads stand at the left, and the ruler and the
//! lanes are drawn on one canvas from [`Lanes`], a compact copy of the
//! snapshot's timeline rebuilt only when a snapshot comes. Subtitle lanes
//! stack on top, then video with the highest track uppermost (V2 over V1),
//! then audio.
//!
//! Clips are chosen by a click, moved by dragging their body (onto another
//! lane of their kind too), and trimmed by dragging an edge. The edit goes
//! to kerfd as one typed operation when the button comes up; until the next
//! snapshot says where the clip is, it is drawn where it was dropped, so it
//! never jumps back. A click or a drag on the ruler or on an empty lane
//! seeks, scrubbing while it drags.
//!
//! While a proposal is pending, the timeline shows the cut it would make,
//! read only, always: the clips it changes in sage and the clips it takes
//! out as dashed outlines. What would be edited is the proposal's to
//! change, and an edit under it would leave it stale; the conversation has
//! its card to accept or reject it. Seeking still works.
//!
//! The program's in and out marks stand on the ruler, the range between
//! them shaded. While the pointer skims the picture a skimmer stands at the
//! time it shows, and the playhead stays where it was.
//!
//! While Kerf works, its cursor stands at the first time it looks at and
//! rings the clips it looks at. Clips whose media is being made carry moving
//! amber stripes (still under Reduce Motion), and the view asks for frames
//! only while they show.

use crate::icons::Icon;
use crate::model::{self, Clip, Timeline, Track, TrackKind};
use crate::motion;
use crate::session::Session;
use crate::store::{Change, Making};
use crate::theme::{self, layout, radius, text, TypeStyle};
use crate::ui;
use gpui::{
    canvas, div, fill, linear_color_stop, linear_gradient, pattern_slash, point, prelude::*, px,
    quad, size, App, BorderStyle, Bounds, ContentMask, Context, Corners, CursorStyle,
    DispatchPhase, ElementId, Entity, FocusHandle, Focusable, Font, FontWeight, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba,
    ScrollWheelEvent, ShapedLine, SharedString, Subscription, Task, TextRun, WeakEntity, Window,
};
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Points before time zero, so the playhead's head shows whole at the start.
const PAD: f32 = 10.0;
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

// ------------------------------------------------------------------ lanes

/// The timeline as it is drawn: its tracks in the order they stack, with
/// what each clip needs to be drawn, hit and dragged.
#[derive(Default)]
struct Lanes {
    fps: f64,
    /// Where the program ends, in seconds.
    end: f64,
    /// A pending proposal's cut, shown read only.
    proposed: bool,
    rows: Vec<Row>,
    markers: Vec<Mark>,
    /// Points the lanes take, top to bottom.
    height: f32,
}

struct Row {
    id: String,
    kind: TrackKind,
    label: SharedString,
    enabled: bool,
    locked: bool,
    muted: bool,
    /// Points from the lanes' top.
    top: f32,
    height: f32,
    clips: Vec<ClipBox>,
    /// Transitions, start and end in seconds.
    transitions: Vec<(f64, f64)>,
    /// Clips of this track a proposal takes out, start and end in seconds.
    removed: Vec<(f64, f64)>,
}

impl Row {
    /// Whether what is on it is quieted: switched off, or muted.
    fn quiet(&self) -> bool {
        !self.enabled || self.muted
    }
}

struct ClipBox {
    id: String,
    label: SharedString,
    start: f64,
    duration: f64,
    start_frame: i64,
    duration_frames: i64,
    enabled: bool,
    /// The colour along its top: what kind of clip it is.
    edge: Rgba,
    source: Option<String>,
    /// A pending proposal adds or changes it.
    changed: bool,
    /// Frames its in point can go earlier, and its out point later, before
    /// its media runs out; None when nothing bounds it.
    head: Option<i64>,
    tail: Option<i64>,
}

impl ClipBox {
    fn new(timeline: &Timeline, track: &Track, clip: &Clip, changed: bool) -> Self {
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
        Self {
            id: clip.id.clone(),
            label: SharedString::from(clip.label().to_owned()),
            start: clip.timeline_start,
            duration: clip.duration,
            start_frame: clip.start_frame,
            duration_frames: clip.duration_frames.max(1),
            enabled: clip.enabled,
            edge: edge_colour(track.kind, clip),
            source: clip.source_id.clone(),
            changed,
            head,
            tail,
        }
    }

    fn end(&self) -> f64 {
        self.start + self.duration
    }
}

struct Mark {
    time: f64,
    duration: f64,
    color: Rgba,
}

impl Lanes {
    /// The lanes of `current`, or of `proposal`'s cut over it when one is pending.
    fn build(current: &Timeline, proposal: Option<&Timeline>) -> Self {
        let shown = proposal.unwrap_or(current);
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
        let mut end = shown.duration.max(0.0);
        let mut top = 0.0;
        let mut rows = Vec::with_capacity(tracks.len());
        for track in ordered {
            let height = lane_height(track.kind);
            let clips = track
                .clips
                .iter()
                .map(|clip| {
                    end = end.max(clip.end());
                    let changed = proposal.is_some() && differs(track, clip, current);
                    ClipBox::new(shown, track, clip, changed)
                })
                .collect();
            let removed = match proposal {
                Some(_) => current
                    .tracks
                    .iter()
                    .find(|before| before.id == track.id)
                    .map(|before| {
                        before
                            .clips
                            .iter()
                            .filter(|clip| shown.clip(&clip.id).is_none())
                            .map(|clip| (clip.timeline_start, clip.end()))
                            .collect()
                    })
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            rows.push(Row {
                id: track.id.clone(),
                kind: track.kind,
                label: SharedString::from(track.label().to_owned()),
                enabled: track.enabled,
                locked: track.locked,
                muted: track.muted,
                top,
                height,
                clips,
                transitions: track
                    .transitions
                    .iter()
                    .filter(|transition| transition.end > transition.start)
                    .map(|transition| (transition.start, transition.end))
                    .collect(),
                removed,
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
                    color: marker_colour(marker.color.as_deref()),
                }
            })
            .collect();
        Self {
            fps: shown.rate(),
            end,
            proposed: proposal.is_some(),
            rows,
            markers,
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
}

fn lane_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => layout::VIDEO_LANE,
        TrackKind::Audio => layout::AUDIO_LANE,
        TrackKind::Subtitle => layout::SUBTITLE_LANE,
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

fn marker_colour(name: Option<&str>) -> Rgba {
    match name.map(str::to_ascii_lowercase).as_deref() {
        Some("red") => theme::neg(),
        Some("green") => theme::pos(),
        Some("blue" | "cyan") => theme::accent_ink(),
        Some("purple" | "violet" | "magenta") => theme::playhead(),
        _ => theme::warn(),
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
}

/// A clip being dragged: moved by its body or trimmed by an edge.
struct ClipDrag {
    id: String,
    edit: Edit,
    /// The row it is on, and the row it would land on.
    row: usize,
    target: usize,
    press_time: f64,
    press_y: f32,
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
    fn new(
        lanes: &Lanes,
        row: usize,
        index: usize,
        zone: Zone,
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
            row,
            target: row,
            press_time,
            press_y,
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

    /// Follows the pointer at `x`, `y` on the canvas.
    fn follow(&mut self, lanes: &Lanes, view: &Viewport, x: f32, y: f32, snap: bool) {
        let fps = lanes.fps;
        let time = view.time(x);
        let delta = time - self.press_time;
        if (delta * view.pps as f64).abs() >= DRAG_START as f64
            || (y - self.press_y).abs() >= DRAG_START
        {
            self.moved = true;
        }
        if !self.moved {
            return;
        }
        let start = self.start_frame as f64 / fps;
        let end = (self.start_frame + self.duration_frames) as f64 / fps;
        let threshold = (SNAP / view.pps.max(f32::EPSILON)) as f64;
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
        if self.edit == Edit::Move {
            // Onto another lane of its kind, unless that one is locked.
            let kind = lanes.rows[self.row].kind;
            self.target = lanes
                .row_at(y - layout::RULER + view.scroll_y)
                .filter(|&row| lanes.rows[row].kind == kind && !lanes.rows[row].locked)
                .unwrap_or(self.row);
        }
    }

    /// Where it is drawn now: start and length, in seconds.
    fn span(&self, fps: f64) -> (f64, f64) {
        let (start, length) = match self.edit {
            Edit::Move => (self.start_frame + self.frames, self.duration_frames),
            Edit::TrimIn => (
                self.start_frame + self.frames,
                self.duration_frames - self.frames,
            ),
            Edit::TrimOut => (self.start_frame, self.duration_frames + self.frames),
        };
        (start as f64 / fps, length.max(1) as f64 / fps)
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
}

/// A dropped clip, until kerfd answers.
struct Pending {
    ghost: Ghost,
    revision: u64,
    at: Instant,
}

/// A media being made, as its clips show it.
struct Made {
    media: String,
    phase: f32,
    label: SharedString,
}

fn making_label(making: &Making) -> SharedString {
    let elapsed = making.since.elapsed().as_secs();
    let clock = format!("{}:{:02}", elapsed / 60, elapsed % 60);
    if making.take > 1 {
        format!("Making take {} · {clock}", making.take).into()
    } else {
        format!("Making · {clock}").into()
    }
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
    /// The lanes as last drawn: what a press hits.
    view: Viewport,
    /// Where the canvas was last laid out, in the window.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    gesture: Option<Gesture>,
    hover: Option<Hover>,
    pending: Option<Pending>,
    /// The playhead as last drawn, to follow it when it moves.
    last_position: f64,
    /// Ticks a making's clock once a second while one runs.
    clock: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TimelineView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                this.session_changed(change, cx)
            }),
        ];
        let lanes = Rc::new(Self::lanes_of(session.read(cx)));
        let mut view = Self {
            session,
            focus_handle: cx.focus_handle(),
            lanes,
            zoom: None,
            left: 0.0,
            scroll_y: 0.0,
            snap: true,
            view: Viewport::default(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            gesture: None,
            hover: None,
            pending: None,
            last_position: 0.0,
            clock: None,
            _subscriptions: subscriptions,
        };
        view.tick_makings(cx);
        view
    }

    fn lanes_of(session: &Session) -> Lanes {
        let snapshot = &session.state.snapshot;
        Lanes::build(&snapshot.timeline, snapshot.proposal_timeline.as_ref())
    }

    fn session_changed(&mut self, change: &Change, cx: &mut Context<Self>) {
        match change {
            Change::Snapshot => {
                let session = self.session.read(cx);
                let revision = session.revision();
                self.lanes = Rc::new(Self::lanes_of(session));
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.revision != revision)
                {
                    self.pending = None;
                }
                // A drag whose clip went elsewhere, or under a proposal, ends.
                if let Some(Gesture::Clip(drag)) = &self.gesture {
                    let same = self
                        .lanes
                        .find(&drag.id)
                        .is_some_and(|(row, _)| row == drag.row);
                    if !same || self.lanes.proposed || drag.target >= self.lanes.rows.len() {
                        self.gesture = None;
                    }
                }
                if self
                    .hover
                    .as_ref()
                    .is_some_and(|hover| self.lanes.find(&hover.clip).is_none())
                {
                    self.hover = None;
                }
                cx.notify();
            }
            // An edit kerfd refused says so: the dropped clip goes back.
            Change::Notice => {
                let refused = self.pending.as_ref().is_some_and(|pending| {
                    let notices = &self.session.read(cx).state.notices;
                    notices
                        .last()
                        .is_some_and(|notice| notice.error && notice.at >= pending.at)
                });
                if refused {
                    self.pending = None;
                    cx.notify();
                }
            }
            Change::Making => self.tick_makings(cx),
            _ => {}
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
        view.scroll_y = view
            .scroll_y
            .clamp(0.0, (self.lanes.height - view.lanes_height()).max(0.0));
        self.left = view.left;
        self.scroll_y = view.scroll_y;
        view
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
        if lane_y < lane.top + CLIP_INSET || lane_y >= lane.top + lane.height - CLIP_INSET {
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

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if self.gesture.is_some() {
            // Its button came up where it was not seen.
            self.release(event.position, cx);
        }
        let (x, y) = self.local(event.position);
        let lanes = self.lanes.clone();
        let time = self.view.time(x).max(0.0);
        let modifiers = event.modifiers;
        let extending = modifiers.shift || modifiers.secondary();
        match self.hit(x, y) {
            Hit::Clip { row, index, zone } if !lanes.proposed => {
                let id = lanes.rows[row].clips[index].id.clone();
                if extending {
                    // ⌘/Ctrl-click toggles a clip in the selection, ⇧-click adds it; neither drags.
                    let toggle = modifiers.secondary();
                    self.session.update(cx, |session, cx| {
                        match session.selection.iter().position(|chosen| *chosen == id) {
                            Some(at) if toggle => {
                                session.selection.remove(at);
                            }
                            Some(_) => {}
                            None => session.selection.push(id),
                        }
                        cx.notify();
                    });
                } else {
                    self.session.update(cx, |session, cx| {
                        session.selection = vec![id];
                        cx.notify();
                    });
                    // A locked track's clips are chosen, never dragged.
                    if !lanes.rows[row].locked {
                        let playhead = self.last_position;
                        self.gesture = Some(Gesture::Clip(ClipDrag::new(
                            &lanes, row, index, zone, time, y, playhead,
                        )));
                    }
                }
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
            if self.hover.is_some() && !self.bounds.get().contains(&event.position) {
                self.hover = None;
                cx.notify();
            }
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.release(event.position, cx);
            return;
        }
        let (x, y) = self.local(event.position);
        self.edge_pan(x);
        let time = self.view.time(x).max(0.0);
        let lanes = self.lanes.clone();
        let view = self.view;
        let snap = self.snap;
        match &mut self.gesture {
            Some(Gesture::Scrub) => self.seek(time, true, cx),
            Some(Gesture::Clip(drag)) => drag.follow(&lanes, &view, x, y, snap),
            None => {}
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
        }
        cx.notify();
    }

    /// Sends what a drag did as one edit.
    fn drop_clip(&mut self, drag: ClipDrag, cx: &mut Context<Self>) {
        let lanes = self.lanes.clone();
        let other_track = drag.edit == Edit::Move && drag.target != drag.row;
        if !drag.moved || (drag.frames == 0 && !other_track) {
            return;
        }
        let op = match drag.edit {
            Edit::Move => {
                let mut op = json!({"op": "move", "clip": drag.id, "frames": drag.frames});
                if other_track {
                    op["track"] = json!(lanes.rows[drag.target].id);
                }
                op
            }
            // A positive amount moves the edge later: the in point shortens, the out point lengthens.
            Edit::TrimIn => {
                json!({"op": "trim", "clip": drag.id, "edge": "in", "frames": drag.frames})
            }
            Edit::TrimOut => {
                json!({"op": "trim", "clip": drag.id, "edge": "out", "frames": drag.frames})
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
            self.pending = Some(Pending {
                ghost: Ghost {
                    id: drag.id,
                    row: drag.target,
                    start,
                    duration,
                    edge: None,
                    dragging: false,
                },
                revision,
                at: Instant::now(),
            });
        }
    }

    fn hover_at(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.gesture.is_some() {
            return;
        }
        let (x, y) = self.local(event.position);
        let hover = match self.hit(x, y) {
            Hit::Clip { row, index, zone } if !self.lanes.proposed => {
                let lane = &self.lanes.rows[row];
                Some(Hover {
                    clip: lane.clips[index].id.clone(),
                    zone: if lane.locked { Zone::Body } else { zone },
                })
            }
            _ => None,
        };
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(text::BODY.line));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        let overflow = (self.lanes.height - self.view.lanes_height()).max(0.0);
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

    /// Pans by `points`, later when positive.
    fn pan(&mut self, points: f32) {
        if self.zoom.is_none() {
            return;
        }
        let left = self.left + (points / self.view.pps.max(f32::EPSILON)) as f64;
        self.left = left.clamp(0.0, max_left(&self.view, self.lanes.end));
    }

    /// Zooms by `factor` around `anchor` points across the lanes, else
    /// around the playhead when it shows, else the middle.
    fn zoom_by(&mut self, factor: f32, anchor: Option<f32>) {
        let view = self.view;
        if view.width <= 0.0 {
            return;
        }
        let fit = fit_pps(view.width, self.lanes.end);
        let most = (self.lanes.fps as f32 * MOST_FRAME).max(fit);
        let next = (view.pps * factor).clamp(fit, most);
        let anchor = anchor.unwrap_or_else(|| {
            let x = view.x(self.last_position);
            if (0.0..=view.width).contains(&x) {
                x
            } else {
                view.width / 2.0
            }
        });
        let time = view.time(anchor);
        if next <= fit * 1.001 {
            self.zoom = None;
            self.left = 0.0;
        } else {
            self.zoom = Some(next);
            self.left = (time - ((anchor - PAD) / next) as f64).max(0.0);
        }
    }

    /// Switches a track's `switch` (`locked`, `muted`, `enabled`) to `value`.
    fn set_track(&mut self, track: &str, switch: &str, value: bool, cx: &mut Context<Self>) {
        if self.lanes.proposed {
            return;
        }
        let mut op = json!({"op": "set-track", "track": track});
        op[switch] = json!(value);
        self.session.update(cx, |session, cx| {
            session.edit(vec![op]);
            cx.notify();
        });
    }

    fn strip(&self, position: f64, cx: &mut Context<Self>) -> impl IntoElement {
        let lanes = &self.lanes;
        div()
            .h(px(layout::STRIP))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::line())
            .child(ui::label(
                model::timecode(position, lanes.fps),
                text::TIMECODE_STRONG,
                theme::ink(),
            ))
            .child(ui::label(
                model::timecode(lanes.end, lanes.fps),
                text::TIMECODE,
                theme::ink3(),
            ))
            .when(lanes.proposed, |strip| {
                strip.child(ui::chip("Proposed cut", theme::pos()))
            })
            .child(div().flex_1())
            // Snap and zoom only where there is a cut to snap and zoom.
            .when(lanes.has_clips(), |strip| {
                strip
                    .child(
                        strip_button("timeline-snap", "Snap", self.snap).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.snap = !this.snap;
                                cx.notify();
                            },
                        )),
                    )
                    .child(ui::hairline(false).h(px(14.0)))
                    .child(
                        strip_button("timeline-zoom-out", "\u{2212}", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.zoom_by(1.0 / ZOOM_STEP, None);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        strip_button("timeline-zoom-in", "+", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.zoom_by(ZOOM_STEP, None);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        strip_button("timeline-fit", "Fit", self.zoom.is_none()).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.zoom = None;
                                this.left = 0.0;
                                cx.notify();
                            }),
                        ),
                    )
            })
    }

    fn heads(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lanes = self.lanes.clone();
        let scroll_y = self.view.scroll_y;
        let rate = lanes.fps;
        let rate = if (rate - rate.round()).abs() < 0.005 {
            format!("{} fps", rate.round())
        } else {
            format!("{rate:.2} fps")
        };
        let rows: Vec<_> = lanes
            .rows
            .iter()
            .map(|row| self.head(row, scroll_y, lanes.proposed, cx))
            .collect();
        div()
            .w(px(layout::TRACK_HEADER))
            .flex_none()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(theme::chrome())
            .border_r_1()
            .border_color(theme::line())
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
                    .children(rows),
            )
    }

    fn head(
        &self,
        row: &Row,
        scroll_y: f32,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label_ink = if row.quiet() {
            theme::ink3()
        } else {
            theme::ink2()
        };
        let lock = {
            let (track, locked) = (row.id.clone(), row.locked);
            ui::icon_button(
                ElementId::Name(format!("track-lock-{}", row.id).into()),
                Icon::Lock,
            )
            .size(22.0)
            .on(locked)
            .disabled(read_only)
            .tooltip(if locked { "Unlock track" } else { "Lock track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "locked", !locked, cx)),
            )
        };
        let switch = if row.kind == TrackKind::Audio {
            let (track, muted) = (row.id.clone(), row.muted);
            ui::icon_button(
                ElementId::Name(format!("track-mute-{}", row.id).into()),
                Icon::Mute,
            )
            .size(22.0)
            .on(muted)
            .disabled(read_only)
            .tooltip(if muted { "Unmute track" } else { "Mute track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "muted", !muted, cx)),
            )
        } else {
            let (track, enabled) = (row.id.clone(), row.enabled);
            ui::icon_button(
                ElementId::Name(format!("track-eye-{}", row.id).into()),
                Icon::Eye,
            )
            .size(22.0)
            .color(if enabled { theme::ink3() } else { theme::dis() })
            .disabled(read_only)
            .tooltip(if enabled { "Hide track" } else { "Show track" })
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_track(&track, "enabled", !enabled, cx)),
            )
        };
        div()
            .absolute()
            .top(px(row.top - scroll_y))
            .left_0()
            .right_0()
            .h(px(row.height))
            .flex()
            .items_center()
            .gap(px(2.0))
            .pl(px(8.0))
            .pr(px(4.0))
            .border_b_1()
            .border_color(theme::line())
            .child(
                div()
                    .flex_none()
                    .w(px(3.0))
                    .h(px((row.height - 16.0).max(8.0)))
                    .mr(px(5.0))
                    .rounded(px(1.5))
                    .bg(theme::with_alpha(
                        kind_colour(row.kind),
                        if row.quiet() { 0.3 } else { 0.7 },
                    )),
            )
            .child(
                ui::line(row.label.clone(), text::CONTROL_STRONG, label_ink)
                    .flex_1()
                    .min_w_0(),
            )
            .child(switch)
            .child(lock)
    }
}

/// A small button of words on the strip, held down when `on`.
fn strip_button(id: &'static str, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
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
        // Skimming the picture moves what it shows, not the playhead: the
        // picture keeps the window's playhead where it was.
        let skim = crate::stage::skim(&self.session, cx);
        let position = state.position;
        let (mark_in, mark_out, _) = session.marks();
        let selection = if lanes.proposed {
            Vec::new()
        } else {
            session.selection.clone()
        };
        let kerf = state.work.busy().then(|| {
            (
                state.agent_focus.first().copied(),
                state.agent_clips.clone(),
            )
        });
        let makings: Vec<Made> = state
            .makings
            .iter()
            .map(|(media, making)| Made {
                media: media.clone(),
                phase: motion::phase(making.since, theme::motion::STRIPE_CYCLE),
                label: making_label(making),
            })
            .collect();
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
                })
            }
            _ => self.pending.as_ref().map(|pending| pending.ghost.clone()),
        };
        let snapped = match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.moved => drag.snapped,
            _ => None,
        };
        let cursor = match &self.gesture {
            Some(Gesture::Clip(drag)) if drag.edit == Edit::Move => Some(CursorStyle::ClosedHand),
            Some(Gesture::Clip(_)) => Some(CursorStyle::ResizeLeftRight),
            _ => None,
        };
        let edge_hover = self
            .hover
            .as_ref()
            .is_some_and(|hover| hover.zone != Zone::Body);
        // The stripes move only while a clip shows them.
        let making_shown = !makings.is_empty()
            && lanes.rows.iter().flat_map(|row| &row.clips).any(|clip| {
                clip.source
                    .as_ref()
                    .is_some_and(|media| makings.iter().any(|made| made.media == *media))
            });
        if making_shown && !motion::reduced() {
            window.request_animation_frame();
        }

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
            makings,
            cursor,
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

        let body = if lanes.has_clips() {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .on_scroll_wheel(cx.listener(Self::wheel))
                .child(self.heads(cx))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .relative()
                        .overflow_hidden()
                        .when(edge_hover, |lanes| {
                            lanes.cursor(CursorStyle::ResizeLeftRight)
                        })
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
                        .on_mouse_move(cx.listener(Self::hover_at))
                        .child(canvas),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::label(
                    "No cut yet \u{2014} ask Kerf, or import media",
                    text::BODY,
                    theme::ink3(),
                ))
                .into_any_element()
        };

        div()
            .key_context(crate::commands::TIMELINE)
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::well())
            .child(self.strip(skim.unwrap_or(position), cx))
            .child(body)
    }
}

// ----------------------------------------------------------------- drawing

/// What one frame of the canvas draws.
struct Frame {
    lanes: Rc<Lanes>,
    view: Viewport,
    playhead: f64,
    /// Where the pointer skims the picture, while it does.
    skimmer: Option<f64>,
    /// The program's in and out marks.
    marks: (Option<f64>, Option<f64>),
    selection: Vec<String>,
    hover: Option<Hover>,
    ghost: Option<Ghost>,
    snapped: Option<f64>,
    /// While Kerf works: the time it looks at, and the clips.
    kerf: Option<(Option<f64>, Vec<String>)>,
    makings: Vec<Made>,
    /// The window's cursor while a drag holds it.
    cursor: Option<CursorStyle>,
    this: WeakEntity<TimelineView>,
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
        for clip in &row.clips {
            match ghost.filter(|ghost| ghost.id == clip.id) {
                // Where a dragged clip was shows as an outline; a dropped one is drawn where it went.
                Some(ghost) if ghost.dragging => {
                    outline_clip(view, pen, width, row, y, clip.start, clip.end(), window)
                }
                Some(_) => {}
                None => paint_clip(
                    frame,
                    pen,
                    width,
                    row,
                    y,
                    clip,
                    clip.start,
                    clip.duration,
                    None,
                    fonts,
                    window,
                    cx,
                ),
            }
        }
        for &(start, end) in &row.removed {
            removed_clip(view, pen, width, row, y, start, end, window);
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
                ghost.start,
                ghost.duration,
                ghost.edge,
                fonts,
                window,
                cx,
            );
        }
    }
    if let Some(time) = frame.snapped {
        let x = view.x(time).floor();
        window.paint_quad(fill(
            pen.rect(x, layout::RULER, 1.0, height - layout::RULER),
            theme::accent_ink(),
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

fn paint_clip(
    frame: &Frame,
    pen: Pen,
    width: f32,
    row: &Row,
    y: f32,
    clip: &ClipBox,
    start: f64,
    duration: f64,
    trimming: Option<Zone>,
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
    let selected = frame.selection.contains(&clip.id);
    let hover = frame.hover.as_ref().filter(|hover| hover.clip == clip.id);
    let making = clip
        .source
        .as_ref()
        .and_then(|media| frame.makings.iter().find(|made| made.media == *media));
    let ground = if clip.changed {
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
    window.paint_quad(quad(
        body,
        corner,
        edge,
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    let under_edge = pen.rect(left, top + CLIP_EDGE, across, tall - CLIP_EDGE);
    let bottom = Corners {
        top_left: px(0.0),
        top_right: px(0.0),
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
    if let Some(made) = making {
        stripes(
            window,
            under_edge,
            made.phase,
            theme::activity::gpu_stripe(),
        );
    }
    // Its name, kept in sight while its start is scrolled off, and clear
    // of a transition coming into it.
    let into = row
        .transitions
        .iter()
        .filter(|(from, to)| *from <= start + 1e-6 && *to > start + 1e-6)
        .map(|(_, to)| view.x(*to) + 4.0)
        .fold(0.0, f32::max);
    let label_x = (x0.max(0.0) + 6.0).max(into);
    let room = x1.min(width) - label_x - 4.0;
    if room > 10.0 {
        let ink = if quiet { theme::dis() } else { theme::ink() };
        let label_y = if tall < 30.0 {
            top + (tall - text::META.line) / 2.0 + 1.0
        } else {
            top + CLIP_EDGE + 3.0
        };
        let line = shape(window, clip.label.clone(), text::META, &fonts.ui, ink);
        let length = (tall >= 36.0 && room > f32::from(line.width) + 8.0).then(|| {
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
            let _ = line.paint(pen.at(label_x, label_y), px(text::META.line), window, cx);
            if let Some(length) = length {
                let _ = length.paint(
                    pen.at(label_x, label_y + 15.0),
                    px(text::MONO_SMALL.line),
                    window,
                    cx,
                );
            }
        });
    }
    if let Some(made) = making {
        let line = shape(
            window,
            made.label.clone(),
            text::META,
            &fonts.ui,
            theme::activity::gpu(),
        );
        let right = x1.min(width) - 6.0;
        if right - f32::from(line.width) > label_x + 40.0 && tall >= 30.0 {
            let at = pen.at(
                right - f32::from(line.width),
                top + tall - text::META.line - 1.0,
            );
            let _ = line.paint(at, px(text::META.line), window, cx);
        }
    }
    // Its outline: the accent chosen, amber being made, sage when a
    // proposal changes it, brighter under the pointer.
    let (outline, outline_width) = if selected {
        (theme::accent(), 1.5)
    } else if making.is_some() {
        (theme::activity::gpu(), 1.0)
    } else if clip.changed {
        (theme::pos(), 1.0)
    } else if hover.is_some() {
        (theme::lineh(), 1.0)
    } else {
        (theme::line2(), 1.0)
    };
    window.paint_quad(quad(
        body,
        corner,
        gpui::transparent_black(),
        px(outline_width),
        outline,
        BorderStyle::Solid,
    ));
    // The edge under the pointer, or the edge being trimmed.
    let edge_bar = trimming
        .or(hover.map(|hover| hover.zone))
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

/// Where a dragged clip was.
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

/// A clip a proposal takes out.
fn removed_clip(
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
        let outline = theme::with_alpha(theme::neg(), 0.7);
        window.paint_quad(quad(
            body,
            px(radius::SMALL),
            gpui::transparent_black(),
            px(1.0),
            outline,
            BorderStyle::Dashed,
        ));
    }
}

/// A transition: a small hatched mark over its cut.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(value: serde_json::Value) -> Timeline {
        Timeline::from_value(&value)
    }

    fn sample() -> Timeline {
        timeline(json!({
            "fps": 24.0,
            "duration": 10.0,
            "tracks": [
                {"id": "V1", "type": "video", "enabled": true, "clips": [
                    {"id": "a", "timelineStart": 0.0, "duration": 4.0, "startFrame": 0, "durationFrames": 96, "enabled": true, "kind": "media"},
                    {"id": "b", "timelineStart": 4.0, "duration": 6.0, "startFrame": 96, "durationFrames": 144, "enabled": true, "kind": "media"}
                ]},
                {"id": "A1", "type": "audio", "enabled": true, "clips": []},
                {"id": "V2", "type": "video", "enabled": true, "clips": [
                    {"id": "t", "timelineStart": 1.0, "duration": 2.0, "startFrame": 24, "durationFrames": 48, "enabled": true, "kind": "generator"}
                ]},
                {"id": "S1", "type": "subtitle", "enabled": true, "clips": []}
            ]
        }))
    }

    #[test]
    fn tracks_stack_subtitles_then_video_high_to_low_then_audio() {
        let lanes = Lanes::build(&sample(), None);
        let order: Vec<_> = lanes.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(order, ["S1", "V2", "V1", "A1"]);
        let tops: Vec<_> = lanes.rows.iter().map(|row| row.top).collect();
        assert_eq!(tops[1], layout::SUBTITLE_LANE);
        assert_eq!(
            lanes.height,
            layout::SUBTITLE_LANE + 2.0 * layout::VIDEO_LANE + layout::AUDIO_LANE
        );
    }

    #[test]
    fn a_proposal_marks_what_it_changes_and_takes_out() {
        let current = sample();
        let mut value = serde_json::to_value(json!({
            "fps": 24.0,
            "tracks": [
                {"id": "V1", "type": "video", "enabled": true, "clips": [
                    {"id": "a", "timelineStart": 0.0, "duration": 4.0, "startFrame": 0, "durationFrames": 96, "enabled": true},
                    {"id": "c", "timelineStart": 4.0, "duration": 2.0, "startFrame": 96, "durationFrames": 48, "enabled": true}
                ]}
            ]
        }))
        .unwrap();
        value["duration"] = json!(6.0);
        let proposal = timeline(value);
        let lanes = Lanes::build(&current, Some(&proposal));
        assert!(lanes.proposed);
        let v1 = &lanes.rows[0];
        let changed: Vec<_> = v1
            .clips
            .iter()
            .map(|clip| (clip.id.as_str(), clip.changed))
            .collect();
        assert_eq!(changed, [("a", false), ("c", true)]);
        assert_eq!(v1.removed, [(4.0, 10.0)]);
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
        let lanes = Lanes::build(&current, None);
        let clip = &lanes.rows[0].clips[0];
        assert_eq!((clip.head, clip.tail), (Some(24), Some(120)));
        let trim_in = ClipDrag::new(&lanes, 0, 0, Zone::In, 2.0, 30.0, 0.0);
        assert_eq!((trim_in.least, trim_in.most), (-24, 95));
        let trim_out = ClipDrag::new(&lanes, 0, 0, Zone::Out, 6.0, 30.0, 0.0);
        assert_eq!((trim_out.least, trim_out.most), (-95, 120));
        let moving = ClipDrag::new(&lanes, 0, 0, Zone::Body, 3.0, 30.0, 0.0);
        assert_eq!(moving.least, -48);
    }

    #[test]
    fn a_move_snaps_to_a_neighbour_within_reach() {
        let lanes = Lanes::build(&sample(), None);
        let view = Viewport {
            width: 1000.0,
            height: 200.0,
            pps: 100.0,
            left: 0.0,
            scroll_y: 0.0,
        };
        // "t" (1–3 s on V2) dragged so its start lands 3 pt from b's start at 4 s.
        let (row, index) = lanes.find("t").unwrap();
        let mut drag = ClipDrag::new(&lanes, row, index, Zone::Body, 2.0, 50.0, 0.0);
        let x = view.x(2.0 + 2.97);
        drag.follow(&lanes, &view, x, 50.0, true);
        assert_eq!(drag.snapped, Some(4.0));
        assert_eq!(drag.frames, 72);
        drag.follow(&lanes, &view, x, 50.0, false);
        assert_eq!(drag.snapped, None);
        assert_eq!(drag.frames, 71);
    }

    #[test]
    fn ruler_steps_divide_evenly_and_leave_room_for_labels() {
        for pps in [0.05f32, 1.0, 12.0, 80.0, 400.0, 1200.0] {
            let (major, minor) = steps(pps, 24.0);
            assert!(
                major * pps as f64 >= LABEL_SPACE as f64 || major == 7200.0,
                "{pps}: {major}"
            );
            let parts = major / minor;
            assert!(
                (parts - parts.round()).abs() < 1e-6,
                "{pps}: {major}/{minor}"
            );
        }
        assert_eq!(ruler_label(65.5, 24.0, 0.5), "01:05:12");
        assert_eq!(ruler_label(3725.0, 24.0, 5.0), "1:02:05");
    }
}
