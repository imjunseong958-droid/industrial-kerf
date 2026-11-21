//! The stage: the picture, in the frame of the device it is made for (a
//! laptop for 16:9 and wider, a phone for vertical and square), with its
//! transport under it.
//!
//! Nothing moves the picture's colours: the room around it stays neutral.
//! While something works a light runs round just outside the picture's
//! edge, blue for Kerf and amber for the computer, amber winning; the chip
//! at its top left says what runs. Both fade in, and out when it ends;
//! under Reduce Motion the ring is still.
//!
//! The picture takes the format of the cut it shows (a first cut Kerf
//! proposes on an empty program brings its own), and a frame is drawn whole
//! at its own shape, never stretched. Over it: the safe zones when they are
//! on (View ▸ Safe Zones), and the chosen clip's masks and motion path,
//! which a drag reshapes. The mask tools (⇧R ⇧E ⇧P, or the transport's)
//! draw a new mask on the chosen picture clip: a rectangle or an ellipse
//! dragged out, or a pen's corners clicked, closed on its first point. A
//! click on the picture plays or pauses; moving the pointer across it skims
//! the program, the timeline showing a skimmer, and leaving it brings the
//! playhead's frame back. Nothing here moves by itself: frames are asked
//! for only while the light runs.

use crate::icons::Icon;
use crate::model;
use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, layout, motion as scene, over_picture, text};
use crate::ui::{self, icon_button};
use gpui::{
    canvas, div, fill, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, ContentMask,
    Context, Corners, CursorStyle, DismissEvent, DispatchPhase, ElementId, Entity, EntityId,
    FocusHandle, FontWeight, Global, KeyBinding, KeyContext, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Rgba, SharedString, Subscription,
    Task, TextRun, WeakEntity, Window,
};
use overlay::{Affine, MotionPath, Outline, Overlap, Projective, Pt};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Action safe: a line this share of the picture in from each edge.
const ACTION_SAFE: f32 = 0.035;
/// Title safe, shown by ticks at its corners: this share in.
const TITLE_SAFE: f32 = 0.05;
/// Points either side of the caption's words, and over and under them.
const CAPTION_PAD: (f32, f32) = (8.0, 2.0);
/// The preview render's card: its widest and its height.
const HUD_SIZE: (f32, f32) = (288.0, 72.0);
/// Points of each title-safe tick.
const SAFE_TICK: f32 = 16.0;
/// Points across the cross at the picture's centre.
const SAFE_CROSS: f32 = 12.0;
/// Points of a handle's side.
const HANDLE: f32 = 7.0;
/// How near the pointer must be to a handle, a point or an outline to take it.
const REACH: f64 = 6.0;
/// Points a press moves before it drags.
const DRAG_START: f64 = 3.0;
/// How long an outline or a path let go waits where it was dropped for kerfd's answer.
const HOLD: Duration = Duration::from_millis(1500);
/// The least move across the picture, as a share of it, that skims again.
const SKIM_STEP: f64 = 0.0005;

// RemoveChosen: Delete or Backspace on what the picture has chosen (a
// mask, a path point) or the pen's last corner. RectangleMask, EllipseMask
// and PenMask take out a mask tool, or put it down again. CancelDrawing:
// Esc lets go of what is being drawn, then puts the tool down. FinishPen:
// Return leaves the pen's outline open.
gpui::actions!(
    stage,
    [
        RemoveChosen,
        RectangleMask,
        EllipseMask,
        PenMask,
        CancelDrawing,
        FinishPen
    ]
);

/// The viewer's zoom chosen from a menu (the app's View ▸ Viewer Zoom): a
/// percent of the frame's own pixels, 0 to fit it. The window carries it
/// out, bringing the picture in for it.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct SetZoom {
    pub percent: u32,
}

/// The picture's key context, there while the stage has the focus.
pub const CONTEXT: &str = "Stage";
/// With it while Delete and Backspace have something on the picture to
/// take: a mask or a path point chosen, or a pen's corners. Otherwise the
/// window's own Delete has the key.
const DELETES: &str = "deletes";
/// With it while a mask tool is out: Esc is the picture's.
const DRAWS: &str = "draws";
/// With it while the pen has two corners or more: Return leaves it open.
const PEN_OPEN: &str = "pen_open";

/// A tool that draws a new mask on the chosen picture clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskTool {
    Rectangle,
    Ellipse,
    Pen,
}

impl MaskTool {
    pub const ALL: [MaskTool; 3] = [MaskTool::Rectangle, MaskTool::Ellipse, MaskTool::Pen];

    /// Its name, as Qt's View menu had it.
    pub fn label(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "Rectangle mask",
            MaskTool::Ellipse => "Ellipse mask",
            MaskTool::Pen => "Pen mask",
        }
    }

    /// What to do with it, while it is out.
    pub fn hint(self) -> &'static str {
        match self {
            MaskTool::Rectangle | MaskTool::Ellipse => "Drag on the picture to draw it",
            MaskTool::Pen => {
                "Click its corners, then the first again to close it (Return leaves it open)"
            }
        }
    }

    /// Its key, as the registry writes keys.
    pub fn shortcut(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "Shift+R",
            MaskTool::Ellipse => "Shift+E",
            MaskTool::Pen => "Shift+P",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            MaskTool::Rectangle => Icon::MaskRectangle,
            MaskTool::Ellipse => Icon::MaskEllipse,
            MaskTool::Pen => Icon::Pen,
        }
    }

    fn id(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "mask-rectangle",
            MaskTool::Ellipse => "mask-ellipse",
            MaskTool::Pen => "mask-pen",
        }
    }
}

/// Binds the picture's keys. ⇧R ⇧E ⇧P take out (or put down) a mask tool
/// wherever the window has the keys, as Qt's View menu did (typing in a
/// field types them); Delete and Backspace, Esc and Return act on what the
/// picture has chosen or is drawing, ahead of the window's own keys, only
/// while it has something to act on.
///
/// `commands::bind` calls it after the registry's keys, since binding a
/// keymap clears these; the stage binds them again whenever it draws and
/// finds them gone.
pub fn bind_keys(cx: &mut App) {
    let mut bindings = Vec::new();
    for context in [
        CONTEXT,
        crate::commands::TIMELINE,
        crate::commands::WORKSPACE,
    ] {
        bindings.push(KeyBinding::new("shift-r", RectangleMask, Some(context)));
        bindings.push(KeyBinding::new("shift-e", EllipseMask, Some(context)));
        bindings.push(KeyBinding::new("shift-p", PenMask, Some(context)));
    }
    for key in ["shift-r", "shift-e", "shift-p"] {
        bindings.push(KeyBinding::new(
            key,
            gpui::NoAction,
            Some(crate::input::CONTEXT),
        ));
    }
    let deletes = format!("{CONTEXT} && {DELETES}");
    bindings.push(KeyBinding::new("delete", RemoveChosen, Some(&deletes)));
    bindings.push(KeyBinding::new("backspace", RemoveChosen, Some(&deletes)));
    let draws = format!("{CONTEXT} && {DRAWS}");
    bindings.push(KeyBinding::new("escape", CancelDrawing, Some(&draws)));
    let open = format!("{CONTEXT} && {PEN_OPEN}");
    bindings.push(KeyBinding::new("enter", FinishPen, Some(&open)));
    cx.bind_keys(bindings);
}

/// Binds the picture's keys again if a keymap bound since cleared them.
fn ensure_keys(cx: &mut App) {
    let bound = cx
        .key_bindings()
        .borrow()
        .bindings_for_action(&RectangleMask)
        .next()
        .is_some();
    if !bound {
        bind_keys(cx);
    }
}

/// Each window's picture, by its session.
#[derive(Default)]
struct Stages(HashMap<EntityId, WeakEntity<StageView>>);

impl Global for Stages {}

/// Takes out the mask tool `tool` on `session`'s picture, or puts it down
/// if it is out, as ⇧R ⇧E ⇧P do: the timeline calls it for its keys, and a
/// menu may. False (and a notice saying why) when no picture clip is chosen
/// to draw on.
pub fn toggle_mask_tool(
    session: &Entity<Session>,
    tool: MaskTool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let stage = cx
        .try_global::<Stages>()
        .and_then(|stages| stages.0.get(&session.entity_id()))
        .and_then(WeakEntity::upgrade);
    stage.is_some_and(|stage| stage.update(cx, |stage, cx| stage.toggle_tool(tool, window, cx)))
}

/// Width over height of what `session`'s picture shows: the sequence's
/// format; a first cut proposed on an empty program brings its own; with
/// neither, the frame's, else 16:9. The workspace splits the panel by it.
pub fn shown_aspect(session: &Session) -> f32 {
    let timeline = session.timeline();
    let proposed = session
        .state
        .snapshot
        .proposal_timeline
        .as_ref()
        .filter(|proposal| !timeline.has_clips() && proposal.width > 0 && proposal.height > 0);
    match (
        proposed,
        session.picture.as_ref(),
        timeline.width > 0 && timeline.height > 0,
    ) {
        (Some(proposal), _, _) => proposal.aspect(),
        (None, _, true) => timeline.aspect(),
        (None, Some(picture), false) if picture.height > 0 => {
            picture.width as f32 / picture.height as f32
        }
        _ => 16.0 / 9.0,
    }
}

/// Which of the two lights runs round the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hue {
    Kerf,
    Computer,
}

impl Hue {
    fn color(self) -> Rgba {
        match self {
            Hue::Kerf => theme::activity::kerf(),
            Hue::Computer => theme::activity::gpu(),
        }
    }
}

// ----------------------------------------------------------------- skimming

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

fn forget_skim(session: EntityId, cx: &mut App) {
    if cx.has_global::<Skims>() {
        cx.global_mut::<Skims>().0.remove(&session);
    }
}

// ---------------------------------------------------------------- placement

/// Where the picture and its device sit within the stage.
#[derive(Clone, Copy, Debug)]
struct Placement {
    device: Bounds<Pixels>,
    picture: Bounds<Pixels>,
    device_radius: Pixels,
    picture_radius: Pixels,
    phone: bool,
}

/// Whether a picture of `aspect` (width over height) is wide: framed as a
/// laptop, its caption one line. Below it (vertical, square and near
/// square) it is framed as a phone and its caption wraps over balanced
/// lines, as Qt's `stage::isWide` drew both.
pub fn wide(aspect: f32) -> bool {
    aspect >= WIDE_FROM
}

/// Where a picture's aspect starts to be wide (Qt's 1.2).
pub const WIDE_FROM: f32 = 1.2;

fn place(area: Bounds<Pixels>, aspect: f32) -> Placement {
    let phone = !wide(aspect);
    let margin = px(24.0);
    let bezel = if phone { px(9.0) } else { px(8.0) };
    let room = size(
        (area.size.width - margin * 2.0).max(px(40.0)),
        (area.size.height - margin * 2.0).max(px(40.0)),
    );
    // The picture fits the room less the bezel, at its own aspect.
    let inner = size(room.width - bezel * 2.0, room.height - bezel * 2.0);
    let (width, height) = if f32::from(inner.width) / f32::from(inner.height) > aspect {
        (inner.height * aspect, inner.height)
    } else {
        (inner.width, inner.width / aspect)
    };
    let picture = Bounds::new(
        point(
            area.origin.x + (area.size.width - width) / 2.0,
            area.origin.y + (area.size.height - height) / 2.0,
        ),
        size(width, height),
    );
    let device = Bounds::new(
        point(picture.origin.x - bezel, picture.origin.y - bezel),
        size(width + bezel * 2.0, height + bezel * 2.0),
    );
    let (device_radius, picture_radius) = if phone {
        let radius = (width.min(height) * 0.14).min(px(46.0));
        (radius + bezel, radius)
    } else {
        (px(14.0), px(6.0))
    };
    Placement {
        device,
        picture,
        device_radius,
        picture_radius,
        phone,
    }
}

/// A frame `width` × `height` drawn whole within `bounds`, centred: never
/// stretched to another shape.
fn contain(bounds: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    if width == 0 || height == 0 {
        return bounds;
    }
    let scale = (f32::from(bounds.size.width) / width as f32)
        .min(f32::from(bounds.size.height) / height as f32);
    let shown = size(px(width as f32 * scale), px(height as f32 * scale));
    Bounds::new(
        point(
            bounds.origin.x + (bounds.size.width - shown.width) / 2.0,
            bounds.origin.y + (bounds.size.height - shown.height) / 2.0,
        ),
        shown,
    )
}

/// The zooms the viewer offers: Fit (0), then shares of the frame's own pixels.
pub const ZOOMS: [f32; 4] = [0.0, 0.5, 1.0, 2.0];

/// A zoom as its menu names it: "Fit", "50%".
pub fn zoom_words(zoom: f32) -> String {
    if zoom <= 0.0 {
        "Fit".to_owned()
    } else {
        format!("{}%", (zoom * 100.0).round() as i64)
    }
}

/// Where a frame `frame` pixels big is drawn over the picture `card` at
/// `zoom`: the card itself at Fit (or with no frame size); else that share
/// of its pixels, one frame pixel a device pixel at 100% (`scale` device
/// pixels to the point), centred on the card.
pub fn zoomed(card: Bounds<Pixels>, frame: (u32, u32), zoom: f32, scale: f32) -> Bounds<Pixels> {
    let (width, height) = frame;
    if zoom <= 0.0 || width == 0 || height == 0 {
        return card;
    }
    let scale = scale.max(1.0);
    let shown = size(
        px(width as f32 * zoom / scale),
        px(height as f32 * zoom / scale),
    );
    Bounds::new(
        point(
            card.center().x - shown.width / 2.0,
            card.center().y - shown.height / 2.0,
        ),
        shown,
    )
}

/// The point `share` (0..1) of the way round `bounds`, clockwise from its top left.
fn round_point(bounds: Bounds<Pixels>, share: f32) -> Point<Pixels> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let perimeter = 2.0 * (width + height);
    let mut along = share.rem_euclid(1.0) * perimeter;
    let origin = bounds.origin;
    if along < width {
        return point(origin.x + px(along), origin.y);
    }
    along -= width;
    if along < height {
        return point(origin.x + px(width), origin.y + px(along));
    }
    along -= height;
    if along < width {
        return point(origin.x + px(width - along), origin.y + px(height));
    }
    along -= width;
    point(origin.x, origin.y + px(height - along))
}

fn to_pt(at: Point<Pixels>) -> Pt {
    (f64::from(f32::from(at.x)), f64::from(f32::from(at.y)))
}

fn to_point((x, y): Pt) -> Point<Pixels> {
    point(px(x as f32), px(y as f32))
}

fn apart(a: Pt, b: Pt) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

// ----------------------------------------------------------------- overlays

/// The chosen clip's masks and motion path, as the picture shows them.
#[derive(Clone, Debug)]
struct Shown {
    clip: String,
    placement: overlay::Placement,
    masks: Vec<overlay::Mask>,
    path: Option<Vec<Pt>>,
    /// How far along its path the clip is now.
    along: f64,
    /// Seconds into the program it is shown at.
    position: f64,
    /// A path in depth that no one map shows, sampled where the clip's
    /// anchor goes (`overlay::sampled`), its points where a drag has them.
    sampled: Option<overlay::Sampled>,
}

impl Shown {
    /// Whether the picture's clicks are its own: there is something to drag.
    /// A path is only here while some of it can be seen (`Stage::overlays`).
    fn edits(&self) -> bool {
        self.edits_masks() || self.path.is_some()
    }

    /// Whether its masks take the picture's clicks: it has some, and its
    /// picture can be seen (Qt's `editsMasks`). Turned edge on or behind
    /// the camera, a click on the picture plays as it would with none.
    fn edits_masks(&self) -> bool {
        !self.masks.is_empty() && self.placement.masks_seen()
    }

    fn aspect(&self) -> f64 {
        let (width, height) = self.placement.canvas;
        width.max(1.0) / height.max(1.0)
    }

    /// Fractions of the clip's picture to the window, with the canvas drawn
    /// at `picture`; none when the clip cannot be seen.
    fn to_window(&self, picture: Bounds<Pixels>) -> Option<Projective> {
        let (width, height) = self.placement.canvas;
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement.picture.shows().then(|| {
            self.placement
                .picture
                .then(Affine::scale(
                    across / width.max(1.0),
                    down / height.max(1.0),
                ))
                .then(Affine::translate(x, y))
        })
    }

    /// A point of the clip's motion path to the window; none when no one
    /// map shows it.
    fn path_to_window(&self, picture: Bounds<Pixels>) -> Option<Projective> {
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement.path.shows().then(|| {
            self.placement
                .path
                .then(Affine::scale(across, down))
                .then(Affine::translate(x, y))
        })
    }

    /// Where the path's point `index` is in the window: on its sampled
    /// curve, or as its map shows it; none out of sight.
    fn point_place(&self, picture: Bounds<Pixels>, index: usize) -> Option<Pt> {
        if let Some(sampled) = &self.sampled {
            return sampled
                .handles
                .get(index)
                .copied()
                .flatten()
                .map(|share| share_to_window(picture, share));
        }
        let at = *self.path.as_ref()?.get(index)?;
        let map = self.path_to_window(picture)?;
        map.in_sight(at).then(|| map.apply(at))
    }

    /// With what a drag holds in place of what the snapshot says.
    fn holding(&mut self, drag: &Drag) {
        match drag {
            Drag::Point { clip, now, .. } if *clip == self.clip => self.path = Some(now.clone()),
            Drag::Mask {
                clip, name, now, ..
            } if *clip == self.clip => {
                if let Some(mask) = self.masks.iter_mut().find(|mask| mask.name == *name) {
                    mask.outline = now.clone();
                }
            }
            Drag::Added { clip, mask }
                if *clip == self.clip && self.masks.iter().all(|has| has.name != mask.name) =>
            {
                self.masks.push(mask.clone());
            }
            _ => {}
        }
    }

    /// The handle of the mask `chosen` under `at`.
    fn handle_at(&self, picture: Bounds<Pixels>, chosen: Option<&str>, at: Pt) -> Option<usize> {
        let mask = self
            .masks
            .iter()
            .find(|mask| Some(mask.name.as_str()) == chosen)?;
        let map = self.to_window(picture)?;
        square_at(&mask.outline.handles(), map, at)
    }

    /// The mask under `at`: the chosen one first, then the last drawn.
    fn mask_at(&self, picture: Bounds<Pixels>, chosen: Option<&str>, at: Pt) -> Option<String> {
        let map = self.to_window(picture)?;
        let hit = |mask: &overlay::Mask| {
            let (line, closed) = mask.outline.line();
            // What is out of sight is not there to press.
            let closed = closed && line.iter().all(|point| map.in_sight(*point));
            let line: Vec<Pt> = line
                .into_iter()
                .filter(|point| map.in_sight(*point))
                .map(|point| map.apply(point))
                .collect();
            (closed && overlay::inside(&line, at))
                || overlay::distance_to_line(&line, closed, at) <= REACH
        };
        if let Some(mask) = self
            .masks
            .iter()
            .find(|mask| Some(mask.name.as_str()) == chosen)
            .filter(|mask| hit(mask))
        {
            return Some(mask.name.clone());
        }
        self.masks
            .iter()
            .rev()
            .find(|mask| hit(mask))
            .map(|mask| mask.name.clone())
    }

    /// The point of the motion path under `at`.
    fn point_at(&self, picture: Bounds<Pixels>, at: Pt) -> Option<usize> {
        let count = self.path.as_ref()?.len();
        (0..count).rev().find(|index| {
            self.point_place(picture, *index)
                .is_some_and(|(x, y)| (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH)
        })
    }

    /// How far along the path's curve `at` is, in spans, when it is on it.
    /// A sampled path's curve takes no point.
    fn curve_at(&self, picture: Bounds<Pixels>, at: Pt) -> Option<f64> {
        if self.sampled.is_some() {
            return None;
        }
        let points = self.path.as_ref()?;
        let map = self.path_to_window(picture)?;
        let samples = MotionPath::new(points.clone(), self.aspect()).samples();
        let mut nearest = REACH;
        let mut found = None;
        for (index, pair) in samples.windows(2).enumerate() {
            // In depth, what is on or behind the camera breaks the curve.
            if !map.in_sight(pair[0]) || !map.in_sight(pair[1]) {
                continue;
            }
            let (from, to) = (map.apply(pair[0]), map.apply(pair[1]));
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = apart((from.0 + dx * t, from.1 + dy * t), at);
            if distance <= nearest {
                nearest = distance;
                found = Some((index as f64 + t) / overlay::SAMPLES_PER_SPAN as f64);
            }
        }
        found
    }
}

/// How wide `words` are, set on one line in `style`.
fn line_width(window: &Window, words: &str, style: theme::TypeStyle) -> f32 {
    let words: SharedString = words.to_owned().into();
    let run = TextRun {
        len: words.len(),
        font: theme::ui_font(style.weight),
        color: theme::ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(words, px(style.px), &[run], None)
            .width,
    )
}

/// `words` cut short with an ellipsis to fit `room` points, set in `style`.
fn cut_to(cx: &App, words: &str, room: f32, style: theme::TypeStyle) -> SharedString {
    let font = theme::ui_font(style.weight);
    let mut runs = vec![TextRun {
        len: words.len(),
        font: font.clone(),
        color: theme::ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    cx.text_system()
        .line_wrapper(font, px(style.px))
        .truncate_line(words.to_owned().into(), px(room.max(0.0)), "…", &mut runs)
}

/// A place in fractions of the canvas, drawn at `picture`, in the window.
fn share_to_window(picture: Bounds<Pixels>, (x, y): Pt) -> Pt {
    let (left, top) = to_pt(picture.origin);
    let (across, down) = to_pt(point(picture.size.width, picture.size.height));
    (left + x * across, top + y * down)
}

/// A place in the window as fractions of the canvas drawn at `picture`.
fn window_to_share(picture: Bounds<Pixels>, (x, y): Pt) -> Pt {
    let (left, top) = to_pt(picture.origin);
    let (across, down) = to_pt(point(picture.size.width, picture.size.height));
    ((x - left) / across.max(1.0), (y - top) / down.max(1.0))
}

/// The square of `points`, mapped by `map`, under `at`: the last drawn
/// first, and none out of sight.
fn square_at(points: &[Pt], map: Projective, at: Pt) -> Option<usize> {
    points.iter().rposition(|point| {
        if !map.in_sight(*point) {
            return false;
        }
        let (x, y) = map.apply(*point);
        (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
    })
}

/// `points` mapped by `map` into the window, in the runs that are in
/// sight: one run, `closed`, when all of a closed outline is.
fn runs_in_sight(points: &[Pt], closed: bool, map: Projective) -> (Vec<Vec<Point<Pixels>>>, bool) {
    let mut runs: Vec<Vec<Point<Pixels>>> = Vec::new();
    let mut going = false;
    for at in points {
        if !map.in_sight(*at) {
            going = false;
            continue;
        }
        let shown = to_point(map.apply(*at));
        match runs.last_mut() {
            Some(run) if going => run.push(shown),
            _ => runs.push(vec![shown]),
        }
        going = true;
    }
    let whole = runs.len() == 1 && runs[0].len() == points.len();
    (runs, closed && whole)
}

/// A press on the picture's overlays, followed until its button comes up.
#[derive(Clone, Debug)]
enum Drag {
    /// A mask by a handle, or moved whole (no handle).
    Mask {
        clip: String,
        name: String,
        handle: Option<usize>,
        /// Where the press was, in fractions of the clip's picture.
        from: Pt,
        original: Outline,
        now: Outline,
        pressed: Pt,
        moved: bool,
    },
    /// A point of the motion path.
    Point {
        clip: String,
        index: usize,
        before: Vec<Pt>,
        now: Vec<Pt>,
        /// From the pointer to the point, so the point does not jump to it.
        held: Pt,
        pressed: Pt,
        moved: bool,
    },
    /// A rectangle or an ellipse being dragged out by its tool, corner to
    /// corner, in fractions of the clip's picture.
    Draw {
        clip: String,
        tool: MaskTool,
        from: Pt,
        to: Pt,
    },
    /// Not a drag: a mask just drawn, shown until kerfd's snapshot has it.
    Added { clip: String, mask: overlay::Mask },
}

/// What a mask tool has drawn so far, as the picture shows it.
#[derive(Clone, Debug, Default)]
struct Drawn {
    /// A rectangle or an ellipse being dragged out.
    outline: Option<Outline>,
    /// The pen's corners, in fractions of the clip's picture.
    pen: Vec<Pt>,
    /// Where the pointer is, in the window, for the pen's next side.
    pointer: Option<Pt>,
}

/// What the chip over the picture says: its words, cut short where it has
/// no room; how a making stands after them, never cut; and its tip, the
/// whole of it, while a making runs.
#[derive(Clone, Debug, PartialEq)]
struct Said {
    words: String,
    stands: Option<String>,
    tip: Option<String>,
}

impl Said {
    fn words(words: String) -> Self {
        Self {
            words,
            stands: None,
            tip: None,
        }
    }
}

// --------------------------------------------------------------------- view

pub struct StageView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    area: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The picture as last placed, in the window: what a press and the pointer hit.
    picture: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The frame as last drawn in it, in the window: the picture itself at
    /// Fit, else the frame at the zoom, centred and cut to the picture.
    /// What the overlays are drawn over and hit on.
    canvas: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The viewer's zoom: 0 fits the frame to the picture; else the share
    /// of its own pixels it is drawn at.
    zoom: f32,
    /// The zoom's menu while it is open (the kit's: its keys, Esc and a
    /// press outside close it), and what drops it as it closes.
    zoom_menu: Option<(Entity<ui::Menu>, Subscription)>,
    hue: Option<Hue>,
    /// The light's hue as it fades out.
    last_hue: Hue,
    edge: Tween,
    edge_since: Instant,
    /// The program's timeline as the last snapshot sent it, whole: the
    /// overlays read what the typed model leaves out (props, masks).
    program: Rc<Value>,
    /// The overlays as last drawn.
    shown: Option<Rc<Shown>>,
    /// The time the pointer shows across the picture, while it skims.
    skim: Option<f64>,
    /// The playhead while kerfd may still answer the skim's seeks.
    kept: Option<Kept>,
    /// The clip whose mask (`Session::chosen_mask`, shared with the
    /// inspector) or path point is chosen, and which point.
    chosen_clip: Option<String>,
    chosen_point: Option<usize>,
    /// The mask tool out, if one is.
    tool: Option<MaskTool>,
    /// The pen's corners so far, in fractions of the clip's picture.
    pen: Vec<Pt>,
    /// Where the pointer is over the stage, in the window, while the pen draws.
    pointer: Option<Pt>,
    /// What the chip over the picture said last, kept while it fades out.
    said: Option<Said>,
    /// Ticks the chip's clock once a second while a making runs and
    /// nothing else asks for frames (Reduce Motion).
    clock: Option<Task<()>>,
    /// The snapshot's proposals, their versions and timelines: what the
    /// preview's card says it renders.
    book: Rc<crate::chat::ProposalBook>,
    /// The proposal whose preview render Stop was pressed on, hidden until
    /// that run ends.
    stopped: Option<String>,
    /// The last finished check (`check` done), for its caption-overlap box.
    check: Option<Rc<Value>>,
    /// The overlap box over the picture, fading in and out, and the last
    /// one shown, kept while it fades out.
    overlap_level: Tween,
    overlap: Option<Overlap>,
    /// The caption's balanced width on a vertical picture, for its words
    /// and the most it may be: words, most, width, lines.
    caption_fit: Option<(SharedString, i32, f32, usize)>,
    drag: Option<Drag>,
    /// What a drag let go, drawn where it was dropped until kerfd's
    /// snapshot (the revision it was sent at) says where it is.
    held: Option<(Drag, u64, Instant)>,
    cursor: Option<CursorStyle>,
    /// The transport's commands' keys in the keymap they were read for, as
    /// the system writes them.
    keys: (String, HashMap<String, String>),
    /// Why each transport control cannot run, and the command context
    /// (less the playhead) it was asked for: asked again only when that
    /// changes, not on every frame played.
    transport_asked: Option<(Value, HashMap<&'static str, String>)>,
    /// The clips of a cut Kerf builds that have been seen placed, and the
    /// media of the newest: the picture stands in as that clip until its
    /// render comes (Cut builds).
    cut_seen: std::collections::HashSet<String>,
    cut_newest: Option<String>,
    /// The poster standing in for the picture, and the one before it as it
    /// crossfades over APPEAR.
    stand_in_poster: Option<(String, Instant)>,
    stand_in_before: Option<String>,
    /// Watching what a proposal changed: the segment whose Before still
    /// is asked for or shown ("proposal/index"), the still and since when
    /// its line sweeps; the side last seen, the frame showing as the side
    /// changed (it wipes away once the next frame comes), the wipe, and
    /// the frame last drawn.
    compare_key: Option<String>,
    compare: Option<(std::sync::Arc<gpui::RenderImage>, Instant)>,
    reel_side: Option<crate::session::ReelSide>,
    wipe_armed: Option<std::sync::Arc<gpui::RenderImage>>,
    wipe: Option<(std::sync::Arc<gpui::RenderImage>, Instant)>,
    last_frame: Option<std::sync::Arc<gpui::RenderImage>>,
    /// Pictures let go of, for the atlas to drop as the stage next draws.
    retired: Vec<std::sync::Arc<gpui::RenderImage>>,
    _subscriptions: Vec<Subscription>,
}

/// The media whose poster stands in for the picture at `at` of
/// `timeline` until a frame comes: the topmost picture clip there with
/// media of its own (a title or a solid has none), else the first clip of
/// footage on the timeline. None with no footage.
pub fn poster_source(timeline: &model::Timeline, at: f64) -> Option<String> {
    let pictures = || {
        timeline
            .tracks
            .iter()
            .rev()
            .filter(|track| track.kind == model::TrackKind::Video)
    };
    pictures()
        .flat_map(|track| track.clips.iter())
        .find(|clip| clip.timeline_start <= at && at < clip.end() && clip.source_id.is_some())
        .or_else(|| {
            pictures()
                .flat_map(|track| track.clips.iter())
                .filter(|clip| clip.source_id.is_some())
                .min_by(|a, b| a.timeline_start.total_cmp(&b.timeline_start))
        })
        .and_then(|clip| clip.source_id.clone())
}

/// Of `timeline`'s picture clips with media, those not in `seen`, in the
/// order they lie (as a cut builds, each newly placed clip): their ids
/// and media.
pub fn placed_clips(
    seen: &std::collections::HashSet<String>,
    timeline: &model::Timeline,
) -> Vec<(String, String)> {
    let mut placed: Vec<(f64, String, String)> = timeline
        .tracks
        .iter()
        .filter(|track| track.kind == model::TrackKind::Video)
        .flat_map(|track| track.clips.iter())
        .filter(|clip| !seen.contains(&clip.id))
        .filter_map(|clip| {
            Some((
                clip.timeline_start,
                clip.id.clone(),
                clip.source_id.clone()?,
            ))
        })
        .collect();
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    placed
        .into_iter()
        .map(|(_, clip, source)| (clip, source))
        .collect()
}

/// Where the Before|After line is across the picture (a share of its
/// width), `elapsed` into the sweep: from `COMPARE_FROM` to `COMPARE_TO`
/// and back, evenly, once each `COMPARE_SWEEP`; held at `COMPARE_FROM`
/// under Reduce Motion (`still`).
pub fn compare_at(elapsed: Duration, still: bool) -> f32 {
    if still {
        return COMPARE_FROM;
    }
    let period = COMPARE_SWEEP.as_secs_f32();
    let phase = (elapsed.as_secs_f32() / period).fract();
    let there = 1.0 - (phase * 2.0 - 1.0).abs();
    COMPARE_FROM + (COMPARE_TO - COMPARE_FROM) * there
}

/// The bytes of standard base64 `text` (padding optional, no line
/// breaks); None when it does not read.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let text = text.trim_end_matches('=').as_bytes();
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        let mut bits = 0u32;
        for (at, byte) in chunk.iter().enumerate() {
            bits |= value(*byte)? << (18 - 6 * at);
        }
        out.push((bits >> 16) as u8);
        if chunk.len() > 2 {
            out.push((bits >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(bits as u8);
        }
    }
    Some(out)
}

/// The Before|After line's sweep, there and back, and how far across the
/// picture it goes (Qt's compareSweep, compareFrom, compareTo); the glow
/// either side of it, the tags at the picture's foot, and the change
/// reel's strip over the picture's top, at most so wide.
use crate::theme::layout::{COMPARE_TAG_IN, REEL_STRIP_IN, REEL_STRIP_MOST};
use crate::theme::motion::{COMPARE_FROM, COMPARE_GLOW, COMPARE_SWEEP, COMPARE_TO};
/// How far into a segment its Before still is taken, and how wide.
const COMPARE_IN: f64 = 0.04;
const COMPARE_WIDE: u32 = 960;

/// Whether a bridge event releases the poster standing in for a cut Kerf
/// builds: a `preview` of `first_cut` (the cut shown on the empty program)
/// that has stopped running, so its render is what the picture shows (Qt
/// let go of the stand-in as that preview ended, `releaseStaying`).
pub fn releases_cover(event: &Value, first_cut: Option<&str>) -> bool {
    event["type"] == "preview"
        && event["status"]
            .as_str()
            .is_some_and(|status| !status.is_empty() && status != "running")
        && first_cut.is_some_and(|cut| event["proposalId"].as_str() == Some(cut))
}

/// Why each transport control cannot run in `context` (the registry's
/// command context, `Session::command_context`), by its command: no
/// kerfd, or a timeline that plays only once a preview is rendered.
pub fn transport_reasons(context: &Value) -> HashMap<&'static str, String> {
    let states = kerf_bridge::commands_states(context);
    TRANSPORT
        .iter()
        .filter_map(|(id, _, _)| {
            let reason = crate::palette::unavailable(&states, id)?;
            Some((*id, reason.to_owned()))
        })
        .collect()
}

/// Whether the picture shows the program, so the clip chosen on it can be
/// drawn over and drawn on (its masks, its path, the mask tools): not while
/// a first cut proposed on an empty program stands in for it, nor while a
/// proposal's preview renders or shows. A proposal waiting over a program
/// with clips leaves the picture the program's, as kerfd plays the
/// document, and Qt's viewer drew on it all the same (`updateViewerMasks`).
pub fn shows_program(state: &crate::store::EditorState, program_has_clips: bool) -> bool {
    let cut_stands_in = state.snapshot.proposal_timeline.is_some() && !program_has_clips;
    let preview_shows = state.preview.proposal.is_some()
        && matches!(state.preview.status.as_str(), "running" | "ready" | "live");
    !cut_stands_in && !preview_shows
}

/// Whether the chip says Kerf is cutting rather than looking (Qt's
/// `work.cutting`): Kerf works (`busy`) on a cut it proposes for an empty
/// program (no clips of its own, a proposed timeline there).
pub fn kerf_cutting(busy: bool, program_has_clips: bool, proposal_timeline: bool) -> bool {
    busy && !program_has_clips && proposal_timeline
}

/// How clear the working chip's dot is `phase` (0..1) of the way through
/// a pulse (`WORK_PULSE`), as Qt drew it: full at either end, down to 35%
/// half way.
pub fn pulse_level(phase: f32) -> f32 {
    let phase = if phase.is_finite() {
        phase.clamp(0.0, 1.0)
    } else {
        0.0
    };
    1.0 - 0.65 * (1.0 - (phase * 2.0 - 1.0).abs())
}

/// The transport's controls: Go to start, Previous frame, Play, Next
/// frame, Go to end; each its command, name and icon.
pub const TRANSPORT: [(&str, &str, Icon); 5] = [
    ("playback.goToStart", "Go to start", Icon::StepBack),
    (
        "playback.previousFrame",
        "Previous frame",
        Icon::ChevronLeft,
    ),
    ("playback.toggle", "Play", Icon::Play),
    ("playback.nextFrame", "Next frame", Icon::ChevronRight),
    ("playback.goToEnd", "Go to end", Icon::StepForward),
];

/// The transport's other commands whose keys its tips say: Loop and the
/// viewer's zoom.
const TRANSPORT_MORE: [&str; 2] = ["playback.loop", "viewer.zoom"];

/// What a transport control's tip says: its name while it can be pressed;
/// greyed, its name and why not (Qt's tipWithReason).
pub fn transport_tip(name: &str, reason: Option<&str>) -> String {
    match reason.filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("{name} · {reason}"),
        None => name.to_owned(),
    }
}

/// The version of the proposal the preview plays (`id`, the preview's
/// proposal) among the snapshot's `proposals`, as Qt's PreviewChip found
/// it: the version's own number when `id` is one of a proposal's versions,
/// else that proposal's. None when no proposal is `id`, or it has no number.
pub fn previewed_version(proposals: &[model::Proposal], id: &str) -> Option<u32> {
    let mut version = 0;
    for proposal in proposals {
        let mut matches = proposal.proposal_id == id;
        for listed in &proposal.versions {
            if listed.proposal_id == id {
                matches = true;
                version = listed.version;
            }
        }
        if matches && version == 0 {
            version = proposal.version;
        }
    }
    (version > 0).then_some(version)
}

/// What the chip over the picture says while a proposal's preview plays,
/// as Qt's did: "Previewing proposal v2", or "Previewing proposal" with no
/// version known.
pub fn preview_chip_words(version: Option<u32>) -> String {
    match version {
        Some(version) => format!("Previewing proposal v{version}"),
        None => "Previewing proposal".to_owned(),
    }
}

impl StageView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = session.entity_id();
        let this = cx.weak_entity();
        cx.default_global::<Stages>().0.insert(id, this);
        ensure_keys(cx);
        let subscriptions = vec![
            cx.observe(&session, |this, session, cx| {
                this.follow_selection(&session, cx);
                cx.notify();
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                match event.0["type"].as_str() {
                    Some("state") => {
                        let before = std::mem::replace(
                            &mut this.program,
                            Rc::new(event.0["snapshot"]["timeline"].clone()),
                        );
                        this.follow_chosen_point(&before, cx);
                        this.book = Rc::new(crate::chat::ProposalBook::read(&event.0["snapshot"]));
                        this.follow_cut(cx);
                    }
                    // The last finished check's findings; one running keeps them.
                    Some("check") if event.0["status"] == "done" => {
                        this.check = Some(Rc::new(event.0.clone()));
                        cx.notify();
                    }
                    // A render stopped here stays hidden until its run ends,
                    // unless another proposal's render takes over.
                    Some("preview") => {
                        if !render_stop_holds(this.stopped.as_deref(), &event.0) {
                            this.stopped = None;
                        }
                        // The render of the cut Kerf builds has come: it is
                        // the picture now, and the poster standing in goes
                        // (until Kerf places another clip).
                        let cut = this
                            .session
                            .read(cx)
                            .first_cut()
                            .map(|(proposal, _)| proposal.proposal_id.clone());
                        if releases_cover(&event.0, cut.as_deref()) && this.cut_newest.is_some() {
                            this.cut_newest = None;
                            cx.notify();
                        }
                    }
                    Some("clock") => {
                        if let Some(at) = event.0["position"].as_f64() {
                            this.hear_clock(at, cx);
                        }
                    }
                    // The Before still of the segment watched.
                    Some("effect_preview") => this.hear_compare(&event.0, cx),
                    _ => {}
                }
            }),
            cx.subscribe(&session, |this, session, change: &Change, cx| {
                let state = &session.read(cx).state;
                match change {
                    // Playback starts from the playhead, never from where the
                    // pointer skimmed, and its clocks are the playhead's.
                    Change::Playback if state.playing && this.kept.is_some() => {
                        let kept = this.kept.take();
                        if this.skim.take().is_some() {
                            forget_skim(session.entity_id(), cx);
                            if let Some(kept) = kept {
                                session.update(cx, |session, cx| {
                                    session.seek(kept.playhead(), false);
                                    cx.notify();
                                });
                            }
                        }
                        cx.notify();
                    }
                    // kerfd gone or only connecting: the picture is cleared,
                    // and the pointer skims nothing until it is back.
                    Change::Connection
                        if state.connection != crate::store::Connection::Connected =>
                    {
                        if this.skim.take().is_some() {
                            forget_skim(session.entity_id(), cx);
                        }
                        this.kept = None;
                        cx.notify();
                    }
                    // An edit kerfd refused says so: what was dropped goes back.
                    Change::Notice => {
                        let refused = this.held.as_ref().is_some_and(|(_, _, at)| {
                            state
                                .notices
                                .last()
                                .is_some_and(|notice| notice.error && notice.at >= *at)
                        });
                        if refused {
                            this.held = None;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }),
            // A key that moves the playhead while the pointer skims (a frame
            // on, the start) ends the skim, from the playhead.
            cx.observe_keystrokes(|this, _, _, cx| this.follow_playhead(cx)),
            cx.on_release(move |_, cx| {
                forget_skim(id, cx);
                if cx.has_global::<Stages>() {
                    cx.global_mut::<Stages>().0.remove(&id);
                }
            }),
        ];
        Self {
            session,
            focus_handle: cx.focus_handle(),
            area: Rc::new(Cell::new(None)),
            picture: Rc::new(Cell::new(None)),
            canvas: Rc::new(Cell::new(None)),
            zoom: 0.0,
            zoom_menu: None,
            hue: None,
            last_hue: Hue::Kerf,
            edge: Tween::at(0.0),
            edge_since: Instant::now(),
            program: Rc::new(Value::Null),
            shown: None,
            skim: None,
            kept: None,
            chosen_clip: None,
            chosen_point: None,
            tool: None,
            pen: Vec::new(),
            pointer: None,
            said: None,
            clock: None,
            book: Rc::default(),
            stopped: None,
            check: None,
            overlap_level: Tween::at(0.0),
            overlap: None,
            caption_fit: None,
            drag: None,
            held: None,
            cursor: None,
            keys: (String::new(), HashMap::new()),
            cut_seen: std::collections::HashSet::new(),
            cut_newest: None,
            transport_asked: None,
            stand_in_poster: None,
            stand_in_before: None,
            compare_key: None,
            compare: None,
            reel_side: None,
            wipe_armed: None,
            wipe: None,
            last_frame: None,
            retired: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// What runs, and the light that says so: a making wins over a render,
    /// which wins over Kerf.
    fn activity(&self, cx: &App) -> (Option<Hue>, Option<Said>) {
        let session = self.session.read(cx);
        let state = &session.state;
        if !state.makings.is_empty() {
            let makings: Vec<(String, crate::store::Making)> = state
                .makings
                .iter()
                .map(|(media, making)| {
                    // As the band names it: no content hash, a shot by its name.
                    let name = Some(session.media_name(media))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| media.clone());
                    (name, making.clone())
                })
                .collect();
            let said = chip::following(&makings).map(|following| Said {
                words: following.said,
                stands: Some(following.stands),
                tip: Some(following.tip),
            });
            return (Some(Hue::Computer), said);
        }
        // A change reel's previews and a first cut's are the reel's and the
        // cut card's to say (Qt's quietenCutPreview); an export still is.
        let quiet = state.preview.rendering() && (session.reel().is_some() || session.cutting());
        if let Some(rendering) = state.rendering().filter(|_| !quiet) {
            return (Some(Hue::Computer), Some(Said::words(rendering)));
        }
        if state.work.busy() {
            let cutting = kerf_cutting(
                true,
                session.timeline().has_clips(),
                state.snapshot.proposal_timeline.is_some(),
            );
            let words = if cutting {
                "Kerf is cutting"
            } else {
                "Kerf is looking"
            };
            return (Some(Hue::Kerf), Some(Said::words(words.to_owned())));
        }
        (None, None)
    }

    /// Follows the change reel (`Session::reel`): watching the After side,
    /// the program as it was at the segment's start is asked for, to
    /// sweep over the picture (Before|After); a change of side wipes the
    /// frame that was showing away once the next comes.
    fn follow_reel(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let frame = session
            .picture
            .as_ref()
            .map(|picture| picture.image.clone());
        let Some(reel) = session.reel().cloned() else {
            self.reel_side = None;
            self.wipe_armed = None;
            self.compare_key = None;
            if let Some((still, _)) = self.compare.take() {
                self.retired.push(still);
            }
            if frame.is_some() {
                self.last_frame = frame;
            }
            return;
        };
        // What was last shown wipes away (the picture may have gone as
        // the side changed: the last frame drawn, not none).
        if self.reel_side.is_some_and(|side| side != reel.side) {
            self.wipe_armed = self.last_frame.clone().or_else(|| frame.clone());
        }
        self.reel_side = Some(reel.side);
        // A new frame after the side changed starts the wipe.
        let new_frame = match (&frame, &self.last_frame) {
            (Some(now), Some(was)) => !std::sync::Arc::ptr_eq(now, was),
            (Some(_), None) => true,
            _ => false,
        };
        if new_frame && !motion::reduced() {
            if let Some(from) = self.wipe_armed.take() {
                if let Some((old, _)) = self.wipe.replace((from, Instant::now())) {
                    self.retired.push(old);
                }
            }
        }
        if frame.is_some() {
            self.last_frame = frame;
        }
        if reel.side != crate::session::ReelSide::After {
            self.compare_key = None;
            if let Some((still, _)) = self.compare.take() {
                self.retired.push(still);
            }
            return;
        }
        let key = format!("{}/{}", reel.proposal, reel.index);
        if self.compare_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.compare_key = Some(key.clone());
        if let Some((still, _)) = self.compare.take() {
            self.retired.push(still);
        }
        // The program's picture clip there; a part that is new has nothing
        // to compare with.
        let Some(segment) = reel.segments.get(reel.index) else {
            return;
        };
        let at = segment.before.max(0.0) + COMPARE_IN;
        let clip = session
            .timeline()
            .tracks
            .iter()
            .filter(|track| track.kind == model::TrackKind::Video)
            .flat_map(|track| track.clips.iter())
            .find(|clip| clip.timeline_start <= at && at < clip.end())
            .map(|clip| clip.id.clone());
        if let Some(clip) = clip {
            self.session.update(cx, |session, _| {
                session.send(json!({
                    "type": "effect_previews",
                    "clip": clip,
                    "at": at,
                    "effects": [{"fx": "color.basic", "key": format!("reel-before:{key}")}],
                    "width": COMPARE_WIDE,
                    "key": "reel",
                }));
            });
        }
    }

    /// An `effect_preview` that is the Before still asked for: it sweeps
    /// over the picture from now.
    fn hear_compare(&mut self, event: &Value, cx: &mut Context<Self>) {
        let Some(key) = self.compare_key.as_deref() else {
            return;
        };
        if event["key"].as_str() != Some(format!("reel-before:{key}").as_str()) {
            return;
        }
        let (Some(width), Some(height)) = (event["width"].as_u64(), event["height"].as_u64())
        else {
            return;
        };
        let Some(rgba) = event["rgba"].as_str().and_then(base64_decode) else {
            return;
        };
        if let Some(still) = crate::session::render_image(&rgba, width as u32, height as u32) {
            if let Some((old, _)) = self.compare.replace((still, Instant::now())) {
                self.retired.push(old);
            }
            cx.notify();
        }
    }

    /// As Kerf builds a cut on an empty program, the newest clip it placed:
    /// the picture stands in as it until the render comes. The clips of a
    /// proposal already there when it is first seen are not new.
    fn follow_cut(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let state = &session.state;
        let proposed = state
            .snapshot
            .proposal_timeline
            .as_ref()
            .filter(|_| !session.timeline().has_clips());
        let Some(proposed) = proposed else {
            self.cut_seen.clear();
            self.cut_newest = None;
            return;
        };
        let placed = placed_clips(&self.cut_seen, proposed);
        let first_sight = self.cut_seen.is_empty();
        for (clip, _) in &placed {
            self.cut_seen.insert(clip.clone());
        }
        if state.work.busy() && !first_sight {
            if let Some((_, source)) = placed.last() {
                self.cut_newest = Some(source.clone());
                cx.notify();
            }
        }
    }

    /// The chip's clock goes on once a second while a making runs, even
    /// when nothing else asks for frames.
    fn tick_makings(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() || self.session.read(cx).state.makings.is_empty() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(scene::CLOCK_FRAME).await;
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

    /// The chosen clip's masks and motion path where the picture shows the
    /// program ([`shows_program`]); none while a proposal's cut or its
    /// preview stands in for it.
    /// Its path only while some of it can be seen: a path in depth no one
    /// map shows is sampled where the clip's anchor goes, its points where a
    /// drag has them; one all on or behind the camera is no path to show
    /// or edit (Qt's `setMotionPath`), unless one of its points is in hand.
    fn overlays(&self, session: &Session) -> Option<Shown> {
        let state = &session.state;
        if !shows_program(state, session.timeline().has_clips()) {
            return None;
        }
        let clip = session.selection.first()?;
        let placement = overlay::placement(&self.program, clip, state.position)?;
        let (data, _) = overlay::clip_in(&self.program, clip)?;
        let props = &data["props"];
        let mut shown = Shown {
            clip: clip.clone(),
            placement,
            masks: overlay::masks_of(data),
            path: props["path"].as_str().and_then(overlay::parse_path),
            along: overlay::property_at(props, "along", placement.clip_time, 0.0),
            position: state.position,
            sampled: None,
        };
        for drag in [
            self.held.as_ref().map(|(drag, _, _)| drag),
            self.drag.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            shown.holding(drag);
        }
        if !shown.placement.path.shows() {
            shown.sampled = overlay::sampled(
                &self.program,
                &shown.clip,
                shown.position,
                shown.path.as_deref(),
            );
        }
        let in_hand = matches!(&self.drag, Some(Drag::Point { clip, .. }) if *clip == shown.clip);
        let seen = shown.sampled.is_some()
            || shown
                .path
                .as_deref()
                .is_some_and(|points| shown.placement.path_seen(points));
        if !seen && !in_hand {
            shown.path = None;
        }
        Some(shown)
    }

    fn set_cursor(&mut self, cursor: Option<CursorStyle>, cx: &mut Context<Self>) {
        if self.cursor != cursor {
            self.cursor = cursor;
            cx.notify();
        }
    }

    /// The mask chosen on the chosen clip, as the picture and the inspector
    /// share it.
    fn chosen_mask(&self, cx: &App) -> Option<String> {
        self.session.read(cx).chosen_mask.clone()
    }

    /// Chooses the mask `name` (or none) on the picture and in the inspector.
    fn choose_mask(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.session.read(cx).chosen_mask == name {
            return;
        }
        self.session.update(cx, |session, cx| {
            session.chosen_mask = name;
            cx.notify();
        });
    }

    /// Another clip chosen: what was chosen on the last one, and what was
    /// being drawn on it, is let go. The tool stays out while there is a
    /// clip to draw on.
    fn follow_selection(&mut self, session: &Entity<Session>, cx: &mut Context<Self>) {
        let first = session.read(cx).selection.first().cloned();
        if first == self.chosen_clip {
            return;
        }
        self.chosen_clip = first;
        self.chosen_point = None;
        self.end_drawing();
        if self.chosen_clip.is_none() {
            self.tool = None;
        }
        if session.read(cx).chosen_mask.is_some() {
            // Read in this same frame by the inspector: no notice needed.
            session.update(cx, |session, _| session.chosen_mask = None);
        }
    }

    /// Whether there is a picture clip chosen that a mask tool can draw on:
    /// one the picture places where it can be seen, not turned edge on or
    /// behind the camera (Qt's `updateViewerMasks`: an invertible
    /// placement), on the program shown rather than a proposal. The mask
    /// tools are greyed in the menus while it is false and none is out
    /// ([`StageView::mask_tool`]).
    pub fn can_draw(&self, cx: &App) -> bool {
        self.overlays(self.session.read(cx))
            .is_some_and(|shown| shown.placement.masks_seen())
    }

    /// The picture as last placed, in the window's coordinates: where the
    /// workspace's export flight leaves from (`Workspace::fly_export_home`).
    pub fn picture_bounds(&self) -> Option<Bounds<Pixels>> {
        self.picture.get()
    }

    /// The mask tool out, if one is.
    pub fn mask_tool(&self) -> Option<MaskTool> {
        self.tool
    }

    /// Takes out `tool`, or puts it down if it is out (⇧R ⇧E ⇧P, the
    /// transport's tools). False, with a notice saying why, when no picture
    /// clip is chosen to draw on.
    pub fn toggle_tool(
        &mut self,
        tool: MaskTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tool == Some(tool) {
            self.set_tool(None, window, cx);
            return true;
        }
        if !self.can_draw(cx) {
            self.session.update(cx, |session, cx| {
                session.state.notify(crate::commands::NO_MASK_CLIP, false);
                cx.notify();
            });
            return false;
        }
        self.set_tool(Some(tool), window, cx);
        true
    }

    /// Takes out `tool` (None puts the one out down): what was being drawn
    /// is let go, and the picture takes the keys and holds still.
    pub fn set_tool(
        &mut self,
        tool: Option<MaskTool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tool = tool.filter(|_| self.can_draw(cx));
        if tool == self.tool {
            return;
        }
        self.end_drawing();
        self.tool = tool;
        if tool.is_some() {
            ensure_keys(cx);
            self.end_skim(cx);
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    /// Lets go of a rectangle or ellipse being dragged out and of the
    /// pen's corners.
    fn end_drawing(&mut self) {
        if matches!(self.drag, Some(Drag::Draw { .. })) {
            self.drag = None;
        }
        self.pen.clear();
        self.pointer = None;
    }

    /// Whether a mask is being drawn: dragged out, or the pen's corners.
    fn drawing(&self) -> bool {
        !self.pen.is_empty() || matches!(self.drag, Some(Drag::Draw { .. }))
    }

    /// Whether a press at `at` (in the window) is on the pen's first corner
    /// with enough corners to close the outline.
    fn pen_closes(&self, shown: &Shown, picture: Bounds<Pixels>, at: Pt) -> bool {
        let Some(map) = shown.to_window(picture) else {
            return false;
        };
        self.pen.len() >= 3
            && self.pen.first().is_some_and(|first| {
                let (x, y) = map.apply(*first);
                map.in_sight(*first) && (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
            })
    }

    /// A press with a mask tool out: a corner of the pen (its first again
    /// closes it), or the start of a rectangle or an ellipse.
    fn press_tool(
        &mut self,
        tool: MaskTool,
        shown: &Shown,
        picture: Bounds<Pixels>,
        at: Pt,
        cx: &mut Context<Self>,
    ) {
        // A press past the horizon, where none of the picture is, draws nothing.
        let Some(map) = shown.to_window(picture) else {
            return;
        };
        let Some(on) = map.point_in_sight(at) else {
            return;
        };
        match tool {
            MaskTool::Pen => {
                if self.pen_closes(shown, picture, at) {
                    self.finish_pen(true, cx);
                    return;
                }
                // A second press of a double-click is not another corner.
                let again = self
                    .pen
                    .last()
                    .is_some_and(|last| apart(map.apply(*last), at) < DRAG_START);
                if !again {
                    self.pen.push(on);
                }
                self.pointer = Some(at);
            }
            MaskTool::Rectangle | MaskTool::Ellipse => {
                self.drag = Some(Drag::Draw {
                    clip: shown.clip.clone(),
                    tool,
                    from: on,
                    to: on,
                });
            }
        }
        cx.notify();
    }

    /// The pen's corners become a mask: closed on its first corner (three
    /// or more), or left open by Return (two or more). False when there are
    /// too few.
    fn finish_pen(&mut self, closed: bool, cx: &mut Context<Self>) -> bool {
        if self.pen.len() < if closed { 3 } else { 2 } {
            return false;
        }
        let Some(clip) = self.shown.as_ref().map(|shown| shown.clip.clone()) else {
            return false;
        };
        let points = std::mem::take(&mut self.pen);
        self.pointer = None;
        self.add_mask(&clip, Outline::Points { points, closed }, cx);
        true
    }

    /// Adds `outline` to `clip` as a new mask, named the first of `k1`,
    /// `k2`, … the clip does not use. It shows, chosen, at once; the tool
    /// is put down.
    fn add_mask(&mut self, clip: &str, outline: Outline, cx: &mut Context<Self>) {
        let mut taken = overlay::clip_in(&self.program, clip)
            .map(|(data, _)| overlay::mask_names(data))
            .unwrap_or_default();
        if let Some((Drag::Added { clip: held, mask }, _, _)) = &self.held {
            if held == clip {
                taken.push(mask.name.clone());
            }
        }
        let name = overlay::new_mask_name(&taken);
        let op = overlay::mask_add(clip, &name, &outline);
        self.tool = None;
        self.chosen_point = None;
        self.choose_mask(Some(name.clone()), cx);
        let mask = overlay::Mask {
            name,
            mode: "add".to_owned(),
            outline,
        };
        self.send_held(
            Drag::Added {
                clip: clip.to_owned(),
                mask,
            },
            op,
            cx,
        );
        cx.notify();
    }

    /// The viewer's zoom: 0 is Fit.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Draws the frame at `zoom` (0 fits it to the picture), and closes the
    /// zoom's menu.
    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom.max(0.0);
        self.zoom_menu = None;
        cx.notify();
    }

    /// Opens the zoom's menu (Fit, 50%, 100%, 200%) over the transport, or
    /// closes it: View ▸ Viewer Zoom… (`viewer.zoom`, ⇧Z) calls it, and the
    /// transport's zoom. Open, the picture takes Esc to close it.
    pub fn toggle_zoom_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoom_menu.take().is_some() {
            cx.notify();
            return;
        }
        let this = cx.entity().downgrade();
        let items = ZOOMS
            .map(|zoom| {
                let this = this.clone();
                let item = ui::MenuItem::new(zoom_words(zoom))
                    .checked((zoom - self.zoom).abs() < f32::EPSILON)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |stage, cx| stage.set_zoom(zoom, cx));
                    });
                if zoom <= 0.0 {
                    item.key(crate::commands::shown_keys("Shift+Z"))
                } else {
                    item
                }
            })
            .to_vec();
        // Its first row is lit at once, so the arrows and Return work from
        // the keys (⇧Z) as from the pointer.
        let menu = cx.new(|cx| ui::Menu::new(items, window, cx).width(168.0).from_keys());
        let dismissed = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.zoom_menu = None;
            cx.notify();
        });
        self.zoom_menu = Some((menu, dismissed));
        cx.notify();
    }

    /// The zoom's menu, over the transport's zoom.
    fn zoom_menu(&self) -> Option<impl IntoElement> {
        let (menu, _) = self.zoom_menu.as_ref()?;
        Some(
            div()
                .absolute()
                .bottom(px(layout::TRANSPORT - 4.0))
                .left(px(0.0))
                .child(menu.clone()),
        )
    }

    /// Esc: lets go of what is being drawn, else puts the tool down; with
    /// neither, the window's Esc has it.
    fn cancel_drawing(&mut self, _: &CancelDrawing, window: &mut Window, cx: &mut Context<Self>) {
        if self.drawing() {
            self.end_drawing();
            cx.notify();
        } else if self.tool.is_some() {
            self.set_tool(None, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Return: the pen's corners become an open outline, as Stroke draws along.
    fn finish_pen_open(&mut self, _: &FinishPen, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.finish_pen(false, cx) {
            cx.propagate();
        }
    }

    /// A press on the stage: the overlays' if they take it, else on the
    /// picture it plays or pauses; on the bars around it, nothing.
    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(picture), Some(canvas)) = (self.picture.get(), self.canvas.get()) else {
            return;
        };
        if self.zoom_menu.take().is_some() {
            // A press on the picture closes the zoom's menu (as a press
            // outside it), and does nothing else: it would play. The keys
            // stay where they were.
            window.prevent_default();
            cx.notify();
            return;
        }
        let drawn_on = self
            .shown
            .clone()
            .filter(|shown| shown.placement.masks_seen());
        if let (Some(tool), Some(shown)) = (self.tool, drawn_on) {
            // A mask tool out has the picture's presses.
            window.focus(&self.focus_handle);
            self.press_tool(tool, &shown, canvas, to_pt(event.position), cx);
            return;
        }
        if let Some(shown) = self.shown.clone() {
            if self.press_overlays(&shown, canvas, to_pt(event.position), event.click_count, cx) {
                // What is chosen takes Delete.
                if self.chosen_mask(cx).is_some() || self.chosen_point.is_some() {
                    ensure_keys(cx);
                    window.focus(&self.focus_handle);
                }
                cx.notify();
                return;
            }
        }
        // A press that plays or pauses, or one on the black bars, leaves the
        // keys where they were (the timeline's Space goes on playing and
        // pausing), as Qt's viewer took focus only for what it edits: the
        // picture's own focus is given above, when a tool or what is chosen
        // on it needs its keys.
        window.prevent_default();
        if !picture.contains(&event.position) {
            return;
        }
        // From the playhead, never from where the pointer skimmed.
        self.end_skim(cx);
        self.session.update(cx, |session, cx| {
            session.toggle_playback();
            cx.notify();
        });
        cx.notify();
    }

    /// Whether the overlays took a press at `at`.
    fn press_overlays(
        &mut self,
        shown: &Shown,
        picture: Bounds<Pixels>,
        at: Pt,
        clicks: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let clip = shown.clip.clone();
        if self.chosen_clip.as_ref() != Some(&clip) {
            self.chosen_clip = Some(clip.clone());
            self.choose_mask(None, cx);
            self.chosen_point = None;
        }
        let chosen = self.chosen_mask(cx);
        if let Some(points) = &shown.path {
            if let Some((index, (x, y))) = shown
                .point_at(picture, at)
                .and_then(|index| Some((index, shown.point_place(picture, index)?)))
            {
                // One thing chosen at a time.
                self.choose_mask(None, cx);
                self.chosen_point = Some(index);
                self.drag = Some(Drag::Point {
                    clip,
                    index,
                    before: points.clone(),
                    now: points.clone(),
                    held: (x - at.0, y - at.1),
                    pressed: at,
                    moved: false,
                });
                return true;
            }
            let handle = shown.handle_at(picture, chosen.as_deref(), at);
            if let Some(u) = handle
                .is_none()
                .then(|| shown.curve_at(picture, at))
                .flatten()
            {
                if clicks >= 2 {
                    // A double-click on the curve adds a point there.
                    let more = overlay::with_point_on(points, shown.aspect(), u);
                    let span = (u.floor().max(0.0) as usize).min(points.len().saturating_sub(2));
                    self.chosen_point = Some(span + 1);
                    self.write_path(&clip, points.clone(), more, cx);
                }
                // A click on the curve is the start of a double-click, not of playing.
                return true;
            }
            // A click off it lets go of the point rather than playing.
            if self.chosen_point.take().is_some() && !shown.edits_masks() {
                return true;
            }
        }
        // Masks that cannot be seen take no click: it plays.
        if !shown.edits_masks() {
            return false;
        }
        // The masks take the picture's clicks: a handle of the chosen one,
        // else the mask under the pointer, else none chosen.
        let handle = shown.handle_at(picture, chosen.as_deref(), at);
        let name = match handle {
            Some(_) => chosen,
            None => shown.mask_at(picture, chosen.as_deref(), at),
        };
        self.choose_mask(name.clone(), cx);
        self.chosen_point = None;
        let found = name.and_then(|name| {
            let mask = shown.masks.iter().find(|mask| mask.name == name)?;
            let from = shown.to_window(picture)?.point_in_sight(at)?;
            Some((name, mask.outline.clone(), from))
        });
        if let Some((name, outline, from)) = found {
            self.drag = Some(Drag::Mask {
                clip,
                name,
                handle,
                from,
                original: outline.clone(),
                now: outline,
                pressed: at,
                moved: false,
            });
        }
        true
    }

    /// The pointer over the stage with no button down: the cursor says what
    /// a press would take, and across the picture it skims.
    fn hover(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_some() || event.pressed_button.is_some() {
            return;
        }
        let (Some(picture), Some(canvas)) = (self.picture.get(), self.canvas.get()) else {
            return;
        };
        let at = to_pt(event.position);
        if self.tool.is_some()
            && self
                .shown
                .as_ref()
                .is_some_and(|shown| shown.placement.masks_seen())
        {
            // A mask tool out: the picture holds still, and the pen's next
            // side follows the pointer.
            self.set_cursor(Some(CursorStyle::Crosshair), cx);
            self.end_skim(cx);
            if !self.pen.is_empty() {
                self.pointer = Some(at);
                cx.notify();
            }
            return;
        }
        if let Some(shown) = self.shown.clone().filter(|shown| shown.edits()) {
            // The picture holds still while its masks or its path can be dragged.
            let chosen = self.chosen_mask(cx);
            let chosen = chosen.as_deref();
            let cursor = if shown.point_at(canvas, at).is_some()
                || shown.handle_at(canvas, chosen, at).is_some()
            {
                Some(CursorStyle::PointingHand)
            } else if shown.curve_at(canvas, at).is_some() {
                Some(CursorStyle::Crosshair)
            } else if shown.mask_at(canvas, chosen, at).is_some() {
                Some(CursorStyle::OpenHand)
            } else {
                None
            };
            self.set_cursor(cursor, cx);
            self.end_skim(cx);
            return;
        }
        self.set_cursor(None, cx);
        self.skim_to(picture, at, cx);
    }

    /// Shows the program at the time `at` gives across the picture, without
    /// moving the playhead; off the picture, the playhead's frame again.
    fn skim_to(&mut self, picture: Bounds<Pixels>, at: Pt, cx: &mut Context<Self>) {
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
    fn follow_playhead(&mut self, cx: &mut Context<Self>) {
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
    /// time. One that answers the skim is not: the playhead stays.
    fn hear_clock(&mut self, at: f64, cx: &mut Context<Self>) {
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

    /// A drag follows the pointer anywhere in the window.
    fn drag_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.release(cx);
            return;
        }
        let (Some(picture), Some(shown)) = (self.canvas.get(), self.shown.clone()) else {
            return;
        };
        let at = to_pt(event.position);
        let program = self.program.clone();
        match &mut self.drag {
            Some(Drag::Point {
                clip,
                index,
                now,
                held,
                pressed,
                moved,
                ..
            }) => {
                // A hand that wobbles while clicking has not moved anything.
                if !*moved && apart(at, *pressed) < DRAG_START {
                    return;
                }
                *moved = true;
                let to = (at.0 + held.0, at.1 + held.1);
                // On a sampled path, the point that puts the clip's anchor
                // there, solved for; else the point the path's map sees there.
                let point = if shown.sampled.is_some() {
                    let seen = window_to_share(picture, to);
                    overlay::sampled_point_for(&program, clip, shown.position, now, *index, seen)
                } else {
                    shown
                        .path_to_window(picture)
                        .and_then(|map| map.point_in_sight(to))
                };
                // What cannot land there, as past a horizon, stays where it last was.
                if let Some(to) = point.filter(|to| overlay::path_holds(*to)) {
                    now[*index] = to;
                }
            }
            Some(Drag::Mask {
                handle,
                from,
                original,
                now,
                pressed,
                moved,
                ..
            }) => {
                if !*moved && apart(at, *pressed) < DRAG_START {
                    return;
                }
                *moved = true;
                if let Some(to) = shown
                    .to_window(picture)
                    .and_then(|map| map.point_in_sight(at))
                {
                    *now = match handle {
                        Some(handle) => original.with_handle(*handle, to),
                        None => original.moved_by((to.0 - from.0, to.1 - from.1)),
                    };
                }
            }
            Some(Drag::Draw { to, .. }) => {
                if let Some(at) = shown
                    .to_window(picture)
                    .and_then(|map| map.point_in_sight(at))
                {
                    *to = at;
                }
            }
            Some(Drag::Added { .. }) | None => return,
        }
        cx.notify();
    }

    /// The button came up: what the drag did goes to kerfd as one edit.
    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        cx.notify();
        if let Drag::Draw {
            clip,
            tool,
            from,
            to,
        } = &drag
        {
            // A click, not a drag, draws nothing; the tool stays out.
            let (Some(picture), Some(shown)) = (self.canvas.get(), self.shown.clone()) else {
                return;
            };
            let Some(map) = shown.to_window(picture) else {
                return;
            };
            if let Some(outline) = overlay::drawn(*tool == MaskTool::Ellipse, *from, *to, map) {
                self.add_mask(clip, outline, cx);
            }
            return;
        }
        let op = match &drag {
            Drag::Point {
                clip,
                before,
                now,
                moved: true,
                ..
            } if now != before => {
                json!({"op": "set", "clip": clip, "props": {"path": overlay::path_text(now)}})
            }
            Drag::Mask {
                clip,
                name,
                original,
                now,
                moved: true,
                ..
            } if now != original => overlay::mask_set(clip, name, now),
            _ => return,
        };
        self.send_held(drag, op, cx);
    }

    /// Writes `points` as the path of `clip`, which was `before`.
    fn write_path(&mut self, clip: &str, before: Vec<Pt>, points: Vec<Pt>, cx: &mut Context<Self>) {
        let op = json!({"op": "set", "clip": clip, "props": {"path": overlay::path_text(&points)}});
        let drag = Drag::Point {
            clip: clip.to_owned(),
            index: 0,
            before,
            now: points,
            held: (0.0, 0.0),
            pressed: (0.0, 0.0),
            moved: true,
        };
        self.send_held(drag, op, cx);
    }

    fn send_held(&mut self, drag: Drag, op: Value, cx: &mut Context<Self>) {
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        });
        if sent {
            self.held = Some((drag, revision, Instant::now()));
        }
    }

    /// Whether Delete has something here to take: a point of the path
    /// shown, or a mask the clip has now. A mask chosen outlives a snapshot
    /// that lacks it (one read before a new mask was added does), but the
    /// key is the window's while it does.
    fn takes_delete(&self, cx: &App) -> bool {
        let Some(shown) = &self.shown else {
            return false;
        };
        if self.drawing() {
            return true;
        }
        let point = self
            .chosen_point
            .is_some_and(|index| shown.path.as_ref().is_some_and(|path| index < path.len()));
        let mask = self.chosen_mask(cx).is_some_and(|name| {
            overlay::clip_in(&self.program, &shown.clip).is_some_and(|(clip, _)| {
                overlay::masks_of(clip).iter().any(|mask| mask.name == name)
            })
        });
        point || mask
    }

    /// A snapshot changed the program: the chosen point goes with its
    /// place, as long as the path still has one there.
    fn follow_chosen_point(&mut self, before: &Value, cx: &mut Context<Self>) {
        // A drag owns its point until it is let go.
        if self.drag.is_some() {
            return;
        }
        let (Some(index), Some(clip)) = (self.chosen_point, self.chosen_clip.as_deref()) else {
            return;
        };
        // What the picture showed: the path as an edit of it was sent, until
        // kerfd answers it, else as it was.
        let revision = self.session.read(cx).revision();
        let held = match &self.held {
            Some((
                Drag::Point {
                    clip: held, now, ..
                },
                sent_at,
                _,
            )) if held == clip => {
                if *sent_at == revision {
                    return;
                }
                Some(now.clone())
            }
            _ => None,
        };
        let Some(shown) = held.or_else(|| overlay::path_of(before, clip)) else {
            self.chosen_point = None;
            return;
        };
        let after = overlay::path_of(&self.program, clip).unwrap_or_default();
        self.chosen_point = overlay::follow_point(&shown, index, &after);
        cx.notify();
    }

    /// Delete or Backspace: the chosen path point goes (a path keeps its
    /// two), else the chosen mask.
    fn remove_chosen(&mut self, _: &RemoveChosen, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.takes_delete(cx) {
            cx.propagate();
            return;
        }
        // While drawing: the pen's last corner goes; a rectangle or an
        // ellipse in hand keeps the key and nothing else.
        if self.drawing() {
            if self.pen.pop().is_some() && self.pen.is_empty() {
                self.pointer = None;
            }
            cx.notify();
            return;
        }
        let Some(shown) = self.shown.clone() else {
            return;
        };
        let point = self.chosen_point.and_then(|index| {
            let points = shown.path.as_ref().filter(|points| index < points.len())?;
            Some((index, points))
        });
        if let Some((index, points)) = point {
            // The key is answered even when the path has only its two.
            if points.len() > 2 {
                let mut fewer = points.clone();
                fewer.remove(index);
                self.chosen_point = None;
                self.write_path(&shown.clip, points.clone(), fewer, cx);
            }
            cx.notify();
            return;
        }
        if let Some(name) = self.chosen_mask(cx) {
            self.choose_mask(None, cx);
            let op = json!({"op": "mask-remove", "clip": shown.clip, "name": name});
            self.session.update(cx, |session, cx| {
                session.edit(vec![op]);
                cx.notify();
            });
            cx.notify();
        }
    }

    /// Start Motion Path (`clip.startPath`): gives the chosen picture clip
    /// a first path from where it is, a quarter of the canvas towards its
    /// middle, and keys that take it along the path over the clip unless
    /// something already says how far along it is. False when there is no
    /// picture clip chosen, or it has a path already.
    pub fn start_path(&mut self, cx: &mut Context<Self>) -> bool {
        let session = self.session.read(cx);
        let op = session
            .selection
            .first()
            .and_then(|clip| overlay::start_path(&self.program, clip, session.state.position));
        let Some(op) = op else {
            return false;
        };
        self.session.update(cx, |session, cx| {
            let sent = session.edit(vec![op]);
            cx.notify();
            sent
        })
    }

    /// Where the caption at the playhead sits over `picture` (in the
    /// stage's own points): centred, its foot on title safe. On a wide
    /// picture it is one line cut at title safe; on a vertical, square or
    /// near-square one ([`wide`]) it wraps within title safe, balanced: the
    /// narrowest width that keeps the lines the widest needs.
    fn caption_box(
        &mut self,
        window: &Window,
        picture: Bounds<Pixels>,
        vertical: bool,
        words: SharedString,
    ) -> Bounds<Pixels> {
        let style = text::CAPTION;
        let (w, h) = (
            f32::from(picture.size.width),
            f32::from(picture.size.height),
        );
        let (tx, ty) = ((w * TITLE_SAFE).round(), (h * TITLE_SAFE).round());
        let runs = [TextRun {
            len: words.len(),
            font: theme::ui_font(style.weight),
            color: over_picture::caption_ink().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let font_size = px(style.px);
        let (across, lines) = if vertical {
            let most = (w - 2.0 * tx - 16.0 - 2.0 * CAPTION_PAD.0).max(1.0);
            let key = (words.clone(), most.round() as i32);
            match &self.caption_fit {
                Some((cached, at, across, lines)) if (cached, at) == (&key.0, &key.1) => {
                    (*across, *lines)
                }
                _ => {
                    let lines_at = |width: f32| {
                        window
                            .text_system()
                            .shape_text(words.clone(), font_size, &runs, Some(px(width)), None)
                            .map(|lines| {
                                lines
                                    .iter()
                                    .map(|line| line.wrap_boundaries().len() + 1)
                                    .sum::<usize>()
                            })
                            .unwrap_or(1)
                            .max(1)
                    };
                    let lines = lines_at(most);
                    let (mut low, mut high) = (1.0_f32, most);
                    for _ in 0..24 {
                        if high - low <= 0.5 {
                            break;
                        }
                        let middle = (low + high) / 2.0;
                        if lines_at(middle) <= lines {
                            high = middle;
                        } else {
                            low = middle;
                        }
                    }
                    let fit = (high.ceil().min(most), lines);
                    self.caption_fit = Some((key.0, key.1, fit.0, fit.1));
                    fit
                }
            }
        } else {
            let most = (w - 2.0 * tx - 2.0 * CAPTION_PAD.0).max(1.0);
            let shaped = window
                .text_system()
                .shape_line(words, font_size, &runs, None);
            (f32::from(shaped.width).ceil().min(most), 1)
        };
        let shown = size(
            px(across + 2.0 * CAPTION_PAD.0),
            px(lines as f32 * style.line + 2.0 * CAPTION_PAD.1),
        );
        Bounds::new(
            point(
                picture.origin.x + px(((w - f32::from(shown.width)) / 2.0).round()),
                picture.origin.y + px(h - ty) - shown.height,
            ),
            shown,
        )
    }

    /// Whether the preview render's card shows ([`render_card_shows`]): a
    /// render runs, Stop was not pressed on it, Kerf is not building a cut
    /// (the picture just keeps up) and no change reel plays. The window
    /// asks before Esc stops the render, and adds whether the picture is in.
    pub fn render_card_shown(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        let state = &session.state;
        let preview = &state.preview;
        render_card_shows(RenderCard {
            rendering: preview.rendering(),
            stopped: self.stopped.as_deref()
                == Some(preview.proposal.as_deref().unwrap_or_default()),
            kerf_building: state.work.busy() && state.snapshot.proposal_timeline.is_some(),
            reel: session.reel().is_some() || session.cutting(),
            picture_shown: true,
        })
    }

    /// Stops the preview rendering (the card's Stop, or Esc while the card
    /// shows): its card goes at once and stays away until that run ends
    /// ([`render_stop_holds`]), as Qt's `AgentHud::stop` hid it.
    pub fn stop_render(&mut self, cx: &mut Context<Self>) {
        let proposal = self
            .session
            .read(cx)
            .state
            .preview
            .proposal
            .clone()
            .unwrap_or_default();
        self.stopped = Some(proposal);
        self.session.update(cx, |session, cx| {
            session.send(json!({"type": "preview_stop"}));
            cx.notify();
        });
        cx.notify();
    }

    /// What the preview render's card says while it shows
    /// ([`StageView::render_card_shown`]): its title, its line, and how far
    /// it has got.
    fn preview_card(&self, cx: &App) -> Option<(String, String, f64)> {
        if !self.render_card_shown(cx) {
            return None;
        }
        let session = self.session.read(cx);
        let state = &session.state;
        let preview = &state.preview;
        let proposal_id = preview.proposal.clone().unwrap_or_default();
        let proposal = self.book.find(&state.snapshot.proposals, &proposal_id);
        let version = proposal.map_or(0, |proposal| {
            self.book
                .versions_of(proposal)
                .iter()
                .find(|version| version.proposal_id == proposal_id)
                .map_or(proposal.version, |version| version.version)
        });
        let timeline = session.timeline();
        let changes = proposal
            .and_then(|proposal| self.book.timeline_of(proposal, &proposal_id))
            .map(|proposed| crate::chat::clip_changes(timeline, proposed))
            .unwrap_or_default();
        let summary = proposal.map(model::Proposal::summary).unwrap_or_default();
        let detail = hud::detail(
            &changes,
            &summary,
            preview.message.as_deref().unwrap_or_default(),
            timeline.rate(),
        );
        Some((hud::title(version), detail, preview.progress.unwrap_or(0.0)))
    }

    fn transport(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // The transport's keys, read again when the keymap has changed.
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 != keymap {
            let keys = crate::commands::catalog(keymap)
                .into_iter()
                .filter(|command| {
                    TRANSPORT.iter().any(|(id, _, _)| *id == command.id)
                        || TRANSPORT_MORE.contains(&command.id.as_str())
                })
                .filter_map(|command| {
                    let key = command
                        .keys
                        .first()
                        .map(|key| crate::commands::shown_keys(key))?;
                    Some((command.id, key))
                })
                .collect();
            self.keys = (keymap.to_owned(), keys);
        }
        let keys = self.keys.1.clone();
        let session = self.session.read(cx);
        // Why each cannot run now (no kerfd; a preview to render first):
        // asked of the registry again only when what it is asked about
        // changes, not on every frame played.
        let mut context = session.command_context();
        if let Some(context) = context.as_object_mut() {
            context.remove("playheadFrame");
        }
        if self
            .transport_asked
            .as_ref()
            .is_none_or(|(asked, _)| *asked != context)
        {
            let reasons = transport_reasons(&context);
            self.transport_asked = Some((context, reasons));
        }
        let reasons = self
            .transport_asked
            .as_ref()
            .map(|(_, reasons)| reasons.clone())
            .unwrap_or_default();
        let fps = session.timeline().rate();
        let position = model::timecode(session.state.position, fps);
        let length = model::timecode(session.state.program_length(), fps);
        let playing = session.state.playing;
        let (mark_in, mark_out, looping) = session.marks();
        let marked = mark_in.is_some() || mark_out.is_some();
        let drawable = self
            .overlays(session)
            .is_some_and(|shown| shown.placement.picture.shows());
        let preview = session.state.preview.clone();
        // The chip keeps quiet while a change reel plays (its strip says
        // what plays) or Kerf is cutting the first cut, as Qt's did.
        let chip_quiet = session.reel().is_some() || session.cutting();
        // Which version of its proposal plays, as the chip says it.
        let previewing = preview
            .proposal
            .as_deref()
            .map(|id| preview_chip_words(previewed_version(&session.state.snapshot.proposals, id)));
        let preview_control = match preview.status.as_str() {
            // Why the picture needs it rendered (what the live picture cannot
            // show yet), cut short before the button and whole in the tips.
            "required" => {
                let reason = preview
                    .reason()
                    .unwrap_or_else(|| crate::store::PREVIEW_REQUIRED.to_owned());
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w_0()
                        .flex_shrink()
                        .child(ui::tip_on(
                            ui::line(reason.clone(), text::CONTROL, theme::ink3())
                                .id("render-preview-reason")
                                .min_w_0()
                                .flex_shrink()
                                .max_w(px(layout::PREVIEW_REASON_MOST)),
                            Some(ui::Tip::new(reason.clone())),
                        ))
                        .child(
                            ui::button("render-preview", "Render preview")
                                .small()
                                .icon(Icon::Sparkle)
                                .tooltip(reason)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |session, _| {
                                        session.preview(None);
                                    })
                                })),
                        )
                        .into_any_element(),
                )
            }
            // While it renders, its card over the picture says so, with Stop.
            "ready" | "live" if preview.proposal.is_some() && !chip_quiet => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::chip(previewing.unwrap_or_default(), theme::pos()))
                    .child(
                        ui::button("exit-preview", "Exit preview")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.update(cx, |session, _| {
                                    session.send(json!({"type": "preview_exit"}));
                                })
                            })),
                    )
                    .into_any_element(),
            ),
            "unavailable" => preview
                .message
                .clone()
                .map(|message| ui::chip(message, theme::ink3()).into_any_element()),
            _ => None,
        };
        div()
            .h(px(layout::TRANSPORT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(12.0))
            // Go to start, Previous frame, Play, Next frame, Go to end: each
            // its command, its key in its tip, greyed with why while it
            // cannot run.
            .children(TRANSPORT.map(|(id, name, icon)| {
                let play = id == "playback.toggle";
                let (name, icon) = match (play, playing) {
                    (true, true) => ("Pause", Icon::Pause),
                    _ => (name, icon),
                };
                let reason = reasons.get(id).cloned();
                let mut button =
                    icon_button(ElementId::Name(format!("transport-{id}").into()), icon)
                        .tooltip(name)
                        .disabled(reason.is_some())
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::commands::RunCommand { id: id.into() }),
                                cx,
                            );
                        });
                if play {
                    button = button.color(theme::ink());
                }
                if let Some(key) = keys.get(id) {
                    button = button.key(key.clone());
                }
                if let Some(reason) = reason {
                    button = button.reason(transport_tip(name, Some(&reason)));
                }
                button
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(8.0))
                    .child(ui::label(position, text::TIMECODE_STRONG, theme::ink()))
                    .child(ui::label("/", text::TIMECODE, theme::ink3()))
                    .child(ui::label(length, text::TIMECODE, theme::ink3())),
            )
            .child(
                div().pl(px(8.0)).child(
                    icon_button("loop-playback", Icon::Loop)
                        .on(looping)
                        .tooltip(if looping && marked {
                            "Loop in to out"
                        } else {
                            "Loop"
                        })
                        .when_some(keys.get("playback.loop").cloned(), |button, key| {
                            button.key(key)
                        })
                        .on_click(cx.listener(|_, _, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::commands::RunCommand::new("playback.loop")),
                                cx,
                            );
                        })),
                ),
            )
            .child(
                div()
                    .relative()
                    .pl(px(4.0))
                    .child(ui::tip_on(
                        theme::type_style(
                            crate::sidebar::trigger("viewer-zoom", {
                                let this = cx.entity().downgrade();
                                move |_, window, cx| {
                                    this.update(cx, |this, cx| this.toggle_zoom_menu(window, cx))
                                        .ok();
                                }
                            }),
                            text::CONTROL,
                        )
                        .flex()
                        .items_center()
                        .h(px(22.0))
                        .px(px(7.0))
                        .rounded(px(theme::radius::CONTROL))
                        .cursor_pointer()
                        .text_color(if self.zoom_menu.is_some() || self.zoom > 0.0 {
                            theme::ink()
                        } else {
                            theme::ink3()
                        })
                        .when(self.zoom_menu.is_some(), |button| button.bg(theme::on()))
                        .hover(|style| style.bg(theme::raised()).text_color(theme::ink()))
                        .child(zoom_words(self.zoom)),
                        // Its menu says all the tip would: none while it is open.
                        self.zoom_menu.is_none().then(|| {
                            ui::Tip::new("Viewer zoom").maybe_key(keys.get("viewer.zoom").cloned())
                        }),
                    ))
                    .children(self.zoom_menu()),
            )
            .child(ui::hairline(false).h(px(16.0)).mx(px(6.0)))
            .children(MaskTool::ALL.map(|tool| {
                icon_button(tool.id(), tool.icon())
                    .on(self.tool == Some(tool))
                    .disabled(!drawable)
                    .tooltip(tool.label())
                    .key(crate::commands::shown_keys(tool.shortcut()))
                    .reason(transport_tip(
                        tool.label(),
                        Some("Choose a picture clip to draw on"),
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_tool(tool, window, cx);
                    }))
            }))
            .children(self.tool.map(|tool| {
                ui::line(tool.hint(), text::CONTROL, theme::ink3())
                    .pl(px(6.0))
                    .min_w_0()
                    .flex_shrink()
            }))
            .child(div().flex_1())
            .children(preview_control)
    }
}

impl Render for StageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ensure_keys(cx);
        let session = self.session.clone();
        self.follow_selection(&session, cx);
        self.follow_reel(cx);
        for image in std::mem::take(&mut self.retired) {
            cx.drop_image(image, Some(window));
        }
        // The Before still sweeping, and the wipe of a side changed.
        let still_motion = motion::reduced();
        let compare = self.compare.as_ref().map(|(still, since)| {
            if !still_motion {
                window.request_animation_frame();
            }
            (still.clone(), compare_at(since.elapsed(), still_motion))
        });
        let comparing = compare.is_some();
        let wipe = match &self.wipe {
            Some((from, since)) if *since + scene::STAGE > Instant::now() => {
                window.request_animation_frame();
                let t = since.elapsed().as_secs_f32() / scene::STAGE.as_secs_f32();
                Some((from.clone(), motion::ease(t.clamp(0.0, 1.0))))
            }
            Some(_) => {
                if let Some((from, _)) = self.wipe.take() {
                    let showing = self
                        .session
                        .read(cx)
                        .picture
                        .as_ref()
                        .is_some_and(|picture| std::sync::Arc::ptr_eq(&picture.image, &from));
                    if !showing {
                        cx.drop_image(from, Some(window));
                    }
                }
                None
            }
            None => None,
        };
        let (hue, chip) = self.activity(cx);
        // The chip keeps what it said while it fades out.
        if chip.is_some() {
            self.said = chip.clone();
        }
        self.tick_makings(cx);
        if hue != self.hue {
            if let Some(hue) = hue {
                self.last_hue = hue;
                if self.hue.is_none() {
                    self.edge_since = Instant::now();
                }
            }
            self.hue = hue;
            self.edge
                .go(if hue.is_some() { 1.0 } else { 0.0 }, scene::EDGE_FADE);
        }
        let edge = self.edge.sample(window);
        if edge > 0.0 && !motion::reduced() {
            window.request_animation_frame();
        }
        let revision = self.session.read(cx).revision();
        if self
            .held
            .as_ref()
            .is_some_and(|(_, sent_at, at)| *sent_at != revision || at.elapsed() > HOLD)
        {
            self.held = None;
        }
        let session = self.session.read(cx);
        // Once a frame, for what is drawn and what the pointer hits.
        let shown = self.overlays(session).map(Rc::new);
        let timeline = session.timeline();
        let picture = session
            .picture
            .as_ref()
            .map(|picture| (picture.image.clone(), picture.width, picture.height));
        let aspect = shown_aspect(session);
        // The frame's own pixels, for the zoom: the sequence's, else the picture's.
        let frame_size = if timeline.width > 0 && timeline.height > 0 {
            (timeline.width, timeline.height)
        } else {
            picture
                .as_ref()
                .map_or((0, 0), |(_, width, height)| (*width, *height))
        };
        let safe_zones = session.prefs.safe_zones;
        let caption = session.state.caption.clone();
        // Never a blank while a picture is coming: as Kerf builds a cut the
        // newest clip it placed stands in for it; with no frame yet, the
        // poster of the clip at the playhead; else words.
        let cutting = kerf_cutting(
            session.state.work.busy(),
            timeline.has_clips(),
            session.state.snapshot.proposal_timeline.is_some(),
        );
        let poster_media = if cutting {
            self.cut_newest.clone()
        } else if picture.is_none() {
            let shown = if timeline.has_clips() {
                Some(timeline)
            } else {
                session.state.snapshot.proposal_timeline.as_ref()
            };
            shown.and_then(|shown| poster_source(shown, session.state.position))
        } else {
            None
        };
        let poster_media = poster_media.filter(|media| session.artwork.poster(media).is_some());
        if poster_media.as_deref()
            != self
                .stand_in_poster
                .as_ref()
                .map(|(media, _)| media.as_str())
        {
            self.stand_in_before = self.stand_in_poster.take().map(|(media, _)| media);
            self.stand_in_poster = poster_media.clone().map(|media| (media, Instant::now()));
        }
        // The newest crossfades in over the one before, over APPEAR.
        let poster_in = self.stand_in_poster.as_ref().map_or(1.0, |(_, since)| {
            if motion::reduced() {
                return 1.0;
            }
            let t = (since.elapsed().as_secs_f32() / scene::APPEAR.as_secs_f32()).min(1.0);
            if t < 1.0 {
                window.request_animation_frame();
            }
            motion::ease(t)
        });
        let posters: Vec<(std::sync::Arc<gpui::RenderImage>, f32)> = [
            (poster_in < 1.0)
                .then_some(self.stand_in_before.as_deref())
                .flatten()
                .and_then(|media| session.artwork.poster(media))
                .map(|image| (image, 1.0)),
            self.stand_in_poster
                .as_ref()
                .and_then(|(media, _)| session.artwork.poster(media))
                .map(|image| (image, poster_in)),
        ]
        .into_iter()
        .flatten()
        .collect();
        // The frame shows under the first poster as it fades in; once one
        // covers it whole it is not drawn.
        let covering = posters.iter().any(|(_, level)| *level >= 1.0);
        let stand_in: Option<SharedString> = if picture.is_some() || !posters.is_empty() {
            None
        } else if !session.state.has_snapshot {
            Some("Connecting to kerfd…".into())
        } else if !timeline.has_clips() {
            Some("No picture yet".into())
        } else {
            Some(session.project_name().into())
        };
        let chosen_mask = session.chosen_mask.clone();
        // A picture that cannot be seen cannot be drawn on: the tool goes
        // down, and what was being drawn is let go; with no clip shown at
        // all, whatever was in hand.
        if shown.is_none() {
            self.drag = None;
        }
        if !shown
            .as_ref()
            .is_some_and(|shown| shown.placement.masks_seen())
        {
            self.tool = None;
            self.end_drawing();
        }
        self.shown = shown.clone();
        if self.tool.is_none() && self.shown.as_ref().is_none_or(|shown| !shown.edits()) {
            self.cursor = None;
        }
        let in_hand = Drawn {
            outline: match &self.drag {
                Some(Drag::Draw { tool, from, to, .. }) => {
                    Some(overlay::dragged(*tool == MaskTool::Ellipse, *from, *to))
                }
                _ => None,
            },
            pen: self.pen.clone(),
            pointer: self.pointer,
        };
        let mut context = KeyContext::default();
        context.add(CONTEXT);
        if self.takes_delete(cx) {
            context.add(DELETES);
        }
        if self.tool.is_some() || self.drawing() {
            context.add(DRAWS);
        }
        if self.pen.len() >= 2 {
            context.add(PEN_OPEN);
        }

        // Overlays sit on the picture as it was placed last frame.
        // The canvas is placed in window coordinates; the overlays in the stage's own.
        let placed = self
            .area
            .get()
            .map(|area| place(Bounds::new(point(px(0.0), px(0.0)), area.size), aspect));
        let area_cell = self.area.clone();
        let picture_cell = self.picture.clone();
        let hue_color = self.last_hue.color();
        let since = self.edge_since;
        let frame = picture.clone();
        let framed = frame.is_some();
        let chosen_point = self.chosen_point;
        let dragging = match &self.drag {
            Some(Drag::Mask { handle: None, .. }) => Some(CursorStyle::ClosedHand),
            Some(Drag::Draw { .. }) => Some(CursorStyle::Crosshair),
            Some(Drag::Mask { .. } | Drag::Point { .. }) => Some(CursorStyle::PointingHand),
            Some(Drag::Added { .. }) | None => None,
        };
        let this = cx.entity().downgrade();

        let canvas_cell = self.canvas.clone();
        let zoom = self.zoom;
        let stage = canvas(
            move |bounds, window, _| {
                if area_cell.get() != Some(bounds) {
                    area_cell.set(Some(bounds));
                    window.refresh();
                }
                let placement = place(bounds, aspect);
                picture_cell.set(Some(placement.picture));
                let drawn = zoomed(placement.picture, frame_size, zoom, window.scale_factor());
                canvas_cell.set(Some(drawn));
                (placement, drawn)
            },
            move |bounds, (placement, drawn): (Placement, Bounds<Pixels>), window, cx| {
                // The device: a bezel, a hairline round it.
                window.paint_quad(quad(
                    placement.device,
                    Corners::all(placement.device_radius),
                    over_picture::bezel(),
                    px(1.0),
                    theme::line2(),
                    BorderStyle::Solid,
                ));
                window.paint_quad(quad(
                    placement.picture,
                    Corners::all(placement.picture_radius),
                    theme::overlay_screen(),
                    px(0.0),
                    theme::line(),
                    BorderStyle::Solid,
                ));
                // A poster standing in: under the cut Kerf builds it covers
                // the frame; with none, it is the picture until one comes.
                if let (Some((image, width, height)), false) = (frame.clone(), covering) {
                    // Whole, at its own shape; zoomed, cut to the picture.
                    if zoom > 0.0 {
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds: placement.picture,
                            }),
                            |window| {
                                let _ = window.paint_image(
                                    contain(drawn, width, height),
                                    Corners::default(),
                                    image,
                                    0,
                                    false,
                                );
                            },
                        );
                    } else {
                        let card = contain(placement.picture, width, height);
                        let _ = window.paint_image(
                            card,
                            Corners::all(placement.picture_radius),
                            image,
                            0,
                            false,
                        );
                        // What was showing as the side changed, wiping away
                        // to the right as what shows now comes in.
                        if let Some((from, t)) = &wipe {
                            let edge = card.left() + card.size.width * *t;
                            let kept =
                                Bounds::from_corners(point(edge, card.top()), card.bottom_right());
                            window.with_content_mask(
                                Some(ContentMask { bounds: kept }),
                                |window| {
                                    let _ = window.paint_image(
                                        card,
                                        Corners::all(placement.picture_radius),
                                        from.clone(),
                                        0,
                                        false,
                                    );
                                },
                            );
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(edge - px(1.0), card.top()),
                                    size(px(2.0), card.size.height),
                                ),
                                over_picture::line(),
                            ));
                        }
                        // What it was, left of the moving line; what it is, right of it.
                        if let Some((still, at)) = &compare {
                            paint_compare(
                                window,
                                card,
                                placement.picture_radius,
                                still.clone(),
                                *at,
                            );
                        }
                    }
                }
                if placement.phone {
                    // The phone's island.
                    let island = size(
                        placement.picture.size.width * 0.28,
                        px(18.0).min(placement.picture.size.height * 0.03),
                    );
                    let origin = point(
                        placement.picture.center().x - island.width / 2.0,
                        placement.picture.origin.y + px(8.0),
                    );
                    window.paint_quad(quad(
                        Bounds::new(origin, island),
                        Corners::all(island.height / 2.0),
                        over_picture::island(),
                        px(0.0),
                        theme::line(),
                        BorderStyle::Solid,
                    ));
                }
                // Over a picture, not over the words standing in for one.
                if safe_zones && framed {
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: placement.picture,
                        }),
                        |window| paint_safe_zones(window, drawn),
                    );
                }
                if let Some(shown) = &shown {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        paint_overlays(
                            window,
                            cx,
                            drawn,
                            shown,
                            chosen_mask.as_deref(),
                            chosen_point,
                        );
                        paint_drawn(window, drawn, shown, &in_hand);
                    });
                }
                if edge > 0.001 {
                    paint_edge_light(window, placement, hue_color, edge, since);
                }
                if let Some(cursor) = dragging {
                    window.set_window_cursor_style(cursor);
                    listen(&this, window);
                }
            },
        )
        .absolute()
        .size_full();

        let preview_card = self.preview_card(cx);
        let mut overlays = div().absolute().inset_0();
        if let Some(placed) = placed {
            let picture = placed.picture;
            let left = picture.origin.x;
            let top = picture.origin.y;
            let card_says =
                preview_card.is_some() && self.session.read(cx).state.makings.is_empty();
            if let Some(chip) = chip
                .clone()
                .or_else(|| self.said.clone())
                .filter(|_| edge > 0.001 && !card_says)
            {
                let color = self.last_hue.color();
                // Its dot pulses once each WORK_PULSE; held still under
                // Reduce Motion. The light round the picture asks for the
                // frames meanwhile.
                let dot = if motion::reduced() {
                    1.0
                } else {
                    window.request_animation_frame();
                    pulse_level(motion::phase(self.edge_since, scene::WORK_PULSE))
                };
                // As wide as its words, up to CHIP_MOST and inside the
                // picture: what a making last said is cut short there, and
                // its tip says the whole of it.
                let most = (f32::from(picture.size.width) - 2.0 * layout::CHIP_IN)
                    .clamp(0.0, layout::CHIP_MOST);
                // The chip is as wide as its words, up to `most`; past that
                // its words are cut short with an ellipsis.
                let stands = chip.stands.map(|stands| format!("· {stands}"));
                let after = stands.as_deref().map_or(0.0, |stands| {
                    6.0 + line_width(window, stands, text::CONTROL)
                });
                let words = line_width(window, &chip.words, text::CONTROL);
                let width = (2.0 * 10.0 + 6.0 + 6.0 + words + after).ceil().min(most);
                let shown = cut_to(
                    cx,
                    &chip.words,
                    width - 2.0 * 10.0 - 6.0 - 6.0 - after,
                    text::CONTROL,
                );
                let body = theme::type_style(div(), text::CONTROL)
                    .id("stage-chip")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(24.0))
                    .w(px(width))
                    .px(px(10.0))
                    .rounded(px(12.0))
                    .overflow_hidden()
                    .bg(theme::activity::chip())
                    .text_color(theme::ink())
                    .child(ui::dot(color, 6.0).opacity(dot).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(shown),
                    )
                    .children(stands.map(|stands| {
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_color(theme::ink2())
                            .child(stands)
                    }));
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left + px(layout::CHIP_IN))
                        .top(top + px(if placed.phone { 36.0 } else { layout::CHIP_IN }))
                        .child(ui::tip_on(body, chip.tip.map(ui::Tip::new)))
                        .opacity(edge.max(0.0)),
                );
            }
            // Before and After, at the picture's foot, while the still sweeps.
            if comparing && framed {
                let tag = |words: &'static str, ground: Rgba| {
                    theme::type_style(div(), text::META)
                        .h(px(22.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .rounded(px(6.0))
                        .bg(ground)
                        .text_color(over_picture::tag_ink())
                        .child(words)
                };
                let foot = top + picture.size.height - px(COMPARE_TAG_IN + 22.0);
                overlays = overlays
                    .child(
                        div()
                            .absolute()
                            .left(left + px(COMPARE_TAG_IN))
                            .top(foot)
                            .child(tag("Before", over_picture::tag())),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(f32::from(
                                self.area.get().map_or(Pixels::ZERO, |area| area.size.width)
                                    - (left + picture.size.width),
                            ) + COMPARE_TAG_IN))
                            .top(foot)
                            .child(tag("After", theme::accent())),
                    );
            }
            // The change reel's strip over the picture's top, at its corner.
            if let Some(strip) = crate::chat::reel_strip(&self.session, cx) {
                let room = self
                    .area
                    .get()
                    .map_or(0.0, |area| f32::from(area.size.width));
                let wide = REEL_STRIP_MOST.min(room - 2.0 * REEL_STRIP_IN).max(0.0);
                let shown = f32::from(picture.size.width);
                let at = if shown >= wide + 2.0 * REEL_STRIP_IN {
                    f32::from(left) + REEL_STRIP_IN
                } else {
                    f32::from(left) + shown / 2.0 - wide / 2.0
                };
                let at = at.clamp(
                    REEL_STRIP_IN,
                    (room - REEL_STRIP_IN - wide).max(REEL_STRIP_IN),
                );
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(px(at))
                        .top(top + px(REEL_STRIP_IN))
                        .w(px(wide))
                        .child(strip),
                );
            }
            // The posters standing in, the newest fading in over the last.
            for (index, (poster, level)) in posters.iter().enumerate() {
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left)
                        .top(top)
                        .w(picture.size.width)
                        .h(picture.size.height)
                        .rounded(px(theme::radius::CARD))
                        .overflow_hidden()
                        .opacity(*level)
                        .child(
                            gpui::img(poster.clone())
                                .id(("stand-in", index))
                                .size_full()
                                .object_fit(gpui::ObjectFit::Cover),
                        ),
                );
            }
            if let Some(stand_in) = stand_in {
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left)
                        .top(top)
                        .w(picture.size.width)
                        .h(picture.size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::label(stand_in, text::BODY, theme::ink3())),
                );
            }
            // The caption and the check's box are over the frame as drawn,
            // and kept inside the picture as Qt kept them: zoomed in, the
            // frame is bigger than the picture, and they go with its edges.
            let card = picture;
            let frame_at = zoomed(card, frame_size, self.zoom, window.scale_factor());
            // In the picture's own points, as the layer kept to it places them.
            let picture = Bounds::new(
                point(
                    frame_at.origin.x - card.origin.x,
                    frame_at.origin.y - card.origin.y,
                ),
                frame_at.size,
            );
            let (left, top) = (picture.origin.x, picture.origin.y);
            let mut on_frame = div()
                .absolute()
                .left(card.origin.x)
                .top(card.origin.y)
                .w(card.size.width)
                .h(card.size.height)
                .overflow_hidden();
            let words: SharedString = caption
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .into();
            let caption_box = (!words.is_empty())
                .then(|| self.caption_box(window, picture, !wide(aspect), words.clone()));
            if let Some(shown) = caption_box {
                on_frame = on_frame.child(
                    div()
                        .absolute()
                        .left(shown.origin.x)
                        .top(shown.origin.y)
                        .w(shown.size.width)
                        .h(shown.size.height)
                        .px(px(CAPTION_PAD.0))
                        .py(px(CAPTION_PAD.1))
                        .bg(over_picture::caption_ground())
                        .child(
                            theme::type_style(div(), text::CAPTION)
                                .w_full()
                                .text_center()
                                .text_color(over_picture::caption_ink())
                                .map(|line| {
                                    if wide(aspect) {
                                        line.whitespace_nowrap()
                                            .overflow_hidden()
                                            .child(ui::cut_line(words))
                                    } else {
                                        line.child(words)
                                    }
                                }),
                        ),
                );
            }
            // The last finished check's caption-overlap box at the playhead,
            // fading in and out.
            let (playhead, fps) = {
                let session = self.session.read(cx);
                (session.state.position, session.timeline().rate())
            };
            let found = self
                .check
                .as_ref()
                .and_then(|check| overlay::overlap_at(check, playhead, fps))
                .filter(|_| framed);
            if found.is_some() != (self.overlap_level.target() > 0.5) {
                self.overlap_level
                    .go(if found.is_some() { 1.0 } else { 0.0 }, scene::FADE);
            }
            if found.is_some() {
                self.overlap = found;
            }
            let level = self.overlap_level.sample(window);
            if let Some(overlap) = self.overlap.clone().filter(|_| level > 0.001) {
                let [x, y, w, h] = overlap.rect;
                let (across, down) = (
                    f64::from(f32::from(picture.size.width)),
                    f64::from(f32::from(picture.size.height)),
                );
                let found_box = Bounds::new(
                    point(left + px((x * across) as f32), top + px((y * down) as f32)),
                    size(px((w * across) as f32), px((h * down) as f32)),
                );
                on_frame = on_frame.child(
                    div()
                        .absolute()
                        .left(found_box.origin.x)
                        .top(found_box.origin.y)
                        .w(found_box.size.width)
                        .h(found_box.size.height)
                        .bg(over_picture::overlap())
                        .border_1()
                        .border_color(theme::neg())
                        .opacity(level),
                );
                if let Some(shown) = caption_box {
                    // The caption the note is about, outlined 2 points out.
                    on_frame = on_frame.child(
                        div()
                            .absolute()
                            .left(shown.origin.x - px(3.0))
                            .top(shown.origin.y - px(3.0))
                            .w(shown.size.width + px(6.0))
                            .h(shown.size.height + px(6.0))
                            .border_1()
                            .border_color(theme::neg())
                            .opacity(level),
                    );
                }
                if !overlap.tag.is_empty() {
                    // Over the caption's right end, 4 points up; without a
                    // caption, over the box's.
                    let anchor = caption_box.unwrap_or(found_box);
                    on_frame = on_frame.child(
                        div()
                            .absolute()
                            .right(card.size.width - (anchor.origin.x + anchor.size.width))
                            .top(anchor.origin.y - px(4.0 + 16.0))
                            .child(
                                theme::type_style(div(), text::MONO_SMALL)
                                    .h(px(16.0))
                                    .px(px(4.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(theme::radius::SMALL))
                                    .bg(theme::chrome())
                                    .text_color(theme::neg())
                                    .child(overlap.tag),
                            )
                            .opacity(level),
                    );
                }
            }
            overlays = overlays.child(on_frame);
            // The preview render's card, in title safe at the picture's top right.
            if let Some((title, detail, progress)) = preview_card {
                let (left, top) = (card.origin.x, card.origin.y);
                let (w, h) = (f32::from(card.size.width), f32::from(card.size.height));
                let (tx, ty) = ((w * TITLE_SAFE).round(), (h * TITLE_SAFE).round());
                let across = HUD_SIZE.0.min(w - 2.0 * tx - 16.0).max(0.0);
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left + px(w - tx - 8.0 - across))
                        .top(top + px(ty + 8.0))
                        .w(px(across))
                        .h(px(HUD_SIZE.1))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded(px(theme::radius::CARD))
                        .bg(theme::with_alpha(theme::card(), 0.94))
                        .border_1()
                        .border_color(theme::line2())
                        .shadow_lg()
                        // Its presses are its own, not the picture's (which plays).
                        .occlude()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(px(16.0))
                                        .rounded(px(theme::radius::SMALL))
                                        .bg(theme::on())
                                        .child(Icon::Sparkle.view(12.0, theme::ink())),
                                )
                                .child(
                                    ui::line(title, text::CONTROL_STRONG, theme::ink())
                                        .flex_1()
                                        .min_w_0(),
                                ),
                        )
                        .child(ui::line(detail, text::META, theme::ink3()).pl(px(24.0)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .pl(px(24.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .child(ui::progress(progress, theme::activity::gpu())),
                                )
                                .child(ui::label(
                                    hud::percent(progress),
                                    text::MONO_SMALL,
                                    theme::ink2(),
                                ))
                                .child(
                                    // Its tip says what it stops, and its key:
                                    // Esc stops the render too while the card
                                    // shows (agent_hud.cpp's setTip).
                                    ui::button("stop-preview", "Stop")
                                        .small()
                                        .ghost()
                                        .tooltip(hud::STOP_TIP)
                                        .key(hud::STOP_KEY)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.stop_render(cx)),
                                        ),
                                ),
                        ),
                );
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surround())
            .child(
                div()
                    .id("stage-picture")
                    .track_focus(&self.focus_handle)
                    .key_context(context)
                    .on_action(cx.listener(Self::remove_chosen))
                    .on_action(cx.listener(Self::cancel_drawing))
                    .on_action(cx.listener(Self::finish_pen_open))
                    .on_action(cx.listener(|this, _: &RectangleMask, window, cx| {
                        this.toggle_tool(MaskTool::Rectangle, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &EllipseMask, window, cx| {
                        this.toggle_tool(MaskTool::Ellipse, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &PenMask, window, cx| {
                        this.toggle_tool(MaskTool::Pen, window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when_some(self.cursor, |area, cursor| area.cursor(cursor))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
                    .on_mouse_move(cx.listener(Self::hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered && this.drag.is_none() {
                            this.end_skim(cx);
                            this.set_cursor(None, cx);
                        }
                    }))
                    .child(stage)
                    .child(overlays),
            )
            .child(self.transport(cx))
    }
}

/// The change reel's Before still over the picture `card`, left of the
/// line `at` of its width, with a soft glow either side of the line and
/// the line itself in white.
fn paint_compare(
    window: &mut Window,
    card: Bounds<Pixels>,
    radius: Pixels,
    still: std::sync::Arc<gpui::RenderImage>,
    at: f32,
) {
    let edge = card.left() + card.size.width * at;
    let before = Bounds::from_corners(card.origin, point(edge, card.bottom()));
    let placed = crate::artwork::fitted(card, crate::artwork::image_size(&still), false);
    window.with_content_mask(Some(ContentMask { bounds: before }), |window| {
        let _ = window.paint_image(placed, Corners::all(radius), still, 0, false);
    });
    let glow = px(COMPARE_GLOW);
    let clear = theme::with_alpha(over_picture::line(), 0.0);
    let lit = theme::with_alpha(over_picture::line(), 0.32);
    window.paint_quad(fill(
        Bounds::new(point(edge - glow, card.top()), size(glow, card.size.height)),
        gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(clear, 0.0),
            gpui::linear_color_stop(lit, 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(point(edge, card.top()), size(glow, card.size.height)),
        gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(lit, 0.0),
            gpui::linear_color_stop(clear, 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(edge - px(1.0), card.top()),
            size(px(2.0), card.size.height),
        ),
        over_picture::line(),
    ));
}

/// While a drag holds the pointer, it follows it anywhere in the window and
/// ends wherever its button comes up.
fn listen(this: &WeakEntity<StageView>, window: &mut Window) {
    let moved = this.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture {
            moved.update(cx, |view, cx| view.drag_moved(event, cx)).ok();
        }
    });
    let up = this.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
            up.update(cx, |view, cx| view.release(cx)).ok();
        }
    });
}

/// The broadcast safe zones: action safe, a 1 pt line just inside a rect
/// 3.5% in from every edge; title safe, 5% in, shown by 16 pt ticks at its
/// corners; and a 12 pt cross at the centre.
fn paint_safe_zones(window: &mut Window, picture: Bounds<Pixels>) {
    let (x, y) = (f32::from(picture.origin.x), f32::from(picture.origin.y));
    let (w, h) = (
        f32::from(picture.size.width),
        f32::from(picture.size.height),
    );
    if w < 2.0 * SAFE_TICK || h < 2.0 * SAFE_TICK {
        return;
    }
    let rect = |left: f32, top: f32, across: f32, down: f32| {
        Bounds::new(
            point(px(left.round()), px(top.round())),
            size(px(across.round().max(1.0)), px(down.round().max(1.0))),
        )
    };
    let (ax, ay) = (x + w * ACTION_SAFE, y + h * ACTION_SAFE);
    let (aw, ah) = (w * (1.0 - 2.0 * ACTION_SAFE), h * (1.0 - 2.0 * ACTION_SAFE));
    for line in [
        rect(ax, ay, aw, 1.0),
        rect(ax, ay + ah - 1.0, aw, 1.0),
        rect(ax, ay + 1.0, 1.0, ah - 2.0),
        rect(ax + aw - 1.0, ay + 1.0, 1.0, ah - 2.0),
    ] {
        window.paint_quad(fill(line, theme::lineh()));
    }
    let (tx, ty) = (x + w * TITLE_SAFE, y + h * TITLE_SAFE);
    let (right, bottom) = (x + w * (1.0 - TITLE_SAFE), y + h * (1.0 - TITLE_SAFE));
    let k = SAFE_TICK;
    let tick = over_picture::tick();
    for line in [
        rect(tx, ty, k, 1.0),
        rect(tx, ty + 1.0, 1.0, k - 1.0),
        rect(right - k, ty, k, 1.0),
        rect(right - 1.0, ty + 1.0, 1.0, k - 1.0),
        rect(tx, bottom - 1.0, k, 1.0),
        rect(tx, bottom - k, 1.0, k - 1.0),
        rect(right - k, bottom - 1.0, k, 1.0),
        rect(right - 1.0, bottom - k, 1.0, k - 1.0),
    ] {
        window.paint_quad(fill(line, tick));
    }
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let half = SAFE_CROSS / 2.0;
    window.paint_quad(fill(rect(cx - half, cy, SAFE_CROSS, 1.0), tick));
    window.paint_quad(fill(rect(cx, cy - half, 1.0, SAFE_CROSS), tick));
}

/// The chosen clip's masks (outlines, a name on each, the chosen one's
/// handles) and its motion path (the curve, square points, and a round one
/// where the clip is now).
fn paint_overlays(
    window: &mut Window,
    cx: &mut App,
    picture: Bounds<Pixels>,
    shown: &Shown,
    chosen_mask: Option<&str>,
    chosen_point: Option<usize>,
) {
    // In depth, what is on or behind the camera breaks an outline.
    if let Some(map) = shown.to_window(picture) {
        for mask in &shown.masks {
            let chosen = chosen_mask == Some(mask.name.as_str());
            let (line, closed) = mask.outline.line();
            let (runs, closed) = runs_in_sight(&line, closed, map);
            for run in &runs {
                stroke_over(window, run, closed, chosen, mask.path_only());
            }
            if let Some(anchor) = top_left(&runs.concat()) {
                paint_tag(window, cx, anchor, &mask.name, chosen, picture);
            }
            if chosen {
                for handle in mask.outline.handles() {
                    if map.in_sight(handle) {
                        paint_handle(window, to_point(map.apply(handle)), false);
                    }
                }
            }
        }
    }
    let Some(points) = &shown.path else {
        return;
    };
    let marker = |window: &mut Window, center: Point<Pixels>| {
        let radius = px(HANDLE / 2.0);
        window.paint_quad(quad(
            Bounds::new(
                point(center.x - radius, center.y - radius),
                size(radius * 2.0, radius * 2.0),
            ),
            Corners::all(radius),
            over_picture::line(),
            px(1.0),
            theme::accent(),
            BorderStyle::Solid,
        ));
    };
    // A path no one map shows: the curve where the clip's anchor goes, its
    // points where the anchor is at each.
    if let Some(sampled) = &shown.sampled {
        let mut runs: Vec<Vec<Point<Pixels>>> = Vec::new();
        let mut going = false;
        for place in &sampled.places {
            match place {
                Some(share) => {
                    let at = to_point(share_to_window(picture, *share));
                    match runs.last_mut() {
                        Some(run) if going => run.push(at),
                        _ => runs.push(vec![at]),
                    }
                    going = true;
                }
                None => going = false,
            }
        }
        for run in &runs {
            stroke_over(window, run, false, true, false);
        }
        if let Some(now) = sampled.now {
            marker(window, to_point(share_to_window(picture, now)));
        }
        for (index, handle) in sampled.handles.iter().enumerate() {
            if let Some(share) = handle {
                paint_handle(
                    window,
                    to_point(share_to_window(picture, *share)),
                    chosen_point == Some(index),
                );
            }
        }
        return;
    }
    let Some(map) = shown.path_to_window(picture) else {
        return;
    };
    let path = MotionPath::new(points.clone(), shown.aspect());
    let samples = path.samples();
    // All of it on or behind the camera is no curve to show.
    if !samples.iter().any(|at| map.in_sight(*at)) {
        return;
    }
    let (runs, _) = runs_in_sight(&samples, false, map);
    for run in &runs {
        stroke_over(window, run, false, true, false);
    }
    // The points are square, as a mask's handles are; the chosen one white.
    for (index, at) in points.iter().enumerate() {
        if map.in_sight(*at) {
            paint_handle(
                window,
                to_point(map.apply(*at)),
                chosen_point == Some(index),
            );
        }
    }
    let on = path.at(shown.along);
    if map.in_sight((on.x, on.y)) {
        marker(window, to_point(map.apply((on.x, on.y))));
    }
}

/// What a mask tool has drawn so far: a rectangle or an ellipse dragged
/// out, or the pen's corners with a dashed side to the pointer, its first
/// corner white when a click there would close the outline.
fn paint_drawn(window: &mut Window, picture: Bounds<Pixels>, shown: &Shown, drawn: &Drawn) {
    let Some(map) = shown.to_window(picture) else {
        return;
    };
    if let Some(outline) = &drawn.outline {
        let (line, closed) = outline.line();
        let (runs, closed) = runs_in_sight(&line, closed, map);
        for run in &runs {
            stroke_over(window, run, closed, true, false);
        }
    }
    let corners: Vec<Pt> = drawn
        .pen
        .iter()
        .filter(|at| map.in_sight(**at))
        .map(|at| map.apply(*at))
        .collect();
    let Some((&first, &last)) = corners.first().zip(corners.last()) else {
        return;
    };
    let line: Vec<Point<Pixels>> = corners.iter().map(|at| to_point(*at)).collect();
    stroke_over(window, &line, false, true, false);
    if let Some(pointer) = drawn.pointer {
        stroke_over(
            window,
            &[to_point(last), to_point(pointer)],
            false,
            false,
            true,
        );
    }
    let closes = corners.len() >= 3
        && drawn.pointer.is_some_and(|pointer| {
            (first.0 - pointer.0).abs() <= REACH && (first.1 - pointer.1).abs() <= REACH
        });
    for (index, corner) in corners.iter().enumerate() {
        paint_handle(window, to_point(*corner), index == 0 && closes);
    }
}

fn top_left(line: &[Point<Pixels>]) -> Option<Point<Pixels>> {
    let first = *line.first()?;
    Some(line.iter().fold(first, |least, at| {
        point(least.x.min(at.x), least.y.min(at.y))
    }))
}

/// An outline over the picture: a dark rule, then a white line on it, which
/// reads on any picture; dashed for a path only. Focus Region's box is
/// drawn with it too (`focus::FocusRegion`).
pub fn stroke_over(
    window: &mut Window,
    line: &[Point<Pixels>],
    closed: bool,
    chosen: bool,
    dashed: bool,
) {
    if line.len() < 2 {
        return;
    }
    let build = |width: f32, dash: bool| {
        let mut builder = PathBuilder::stroke(px(width));
        if dash {
            builder = builder.dash_array(&[px(4.0), px(3.0)]);
        }
        builder.move_to(line[0]);
        for at in &line[1..] {
            builder.line_to(*at);
        }
        if closed {
            builder.line_to(line[0]);
        }
        builder.build().ok()
    };
    if let Some(path) = build(3.0, false) {
        window.paint_path(path, over_picture::rule());
    }
    if let Some(path) = build(1.5, dashed) {
        let ink = if chosen {
            over_picture::line()
        } else {
            over_picture::line_idle()
        };
        window.paint_path(path, ink);
    }
}

/// A mask's name on a tag just above `anchor` (its outline's top left), kept
/// inside the picture: 8 points under its top edge and 4 in from its sides.
fn paint_tag(
    window: &mut Window,
    cx: &mut App,
    anchor: Point<Pixels>,
    name: &str,
    chosen: bool,
    picture: Bounds<Pixels>,
) {
    let words: SharedString = name.to_owned().into();
    let run = TextRun {
        len: words.len(),
        font: theme::ui_font(FontWeight::SEMIBOLD),
        color: over_picture::tag_ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words, px(11.0), &[run], None);
    let (across, down) = (f32::from(line.width) + 10.0, 16.0);
    let (left, top) = (f32::from(picture.origin.x), f32::from(picture.origin.y));
    let (right, bottom) = (
        left + f32::from(picture.size.width),
        top + f32::from(picture.size.height),
    );
    // Clear of the top-left handle, by 4 points.
    let x = (f32::from(anchor.x) - HANDLE / 2.0)
        .clamp(left + 4.0, (left + 4.0).max(right - 4.0 - across));
    let y = (f32::from(anchor.y) - HANDLE / 2.0 - 4.0 - down)
        .clamp(top + 8.0, (top + 8.0).max(bottom - 4.0 - down));
    let tag = Bounds::new(point(px(x), px(y)), size(px(across), px(down)));
    window.paint_quad(quad(
        tag,
        Corners::all(px(4.0)),
        if chosen {
            theme::accent()
        } else {
            over_picture::tag()
        },
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    let _ = line.paint(point(px(x + 5.0), px(y)), px(down), window, cx);
}

/// A handle: the accent with a white edge, or white with the accent's when
/// `inverted` (a chosen point).
fn paint_handle(window: &mut Window, center: Point<Pixels>, inverted: bool) {
    let half = px(HANDLE / 2.0);
    let (fill, edge) = if inverted {
        (over_picture::line(), theme::accent())
    } else {
        (theme::accent(), over_picture::line())
    };
    window.paint_quad(quad(
        Bounds::new(
            point(center.x - half, center.y - half),
            size(px(HANDLE), px(HANDLE)),
        ),
        Corners::default(),
        fill,
        px(1.0),
        edge,
        BorderStyle::Solid,
    ));
}

/// The light round the picture: the rest of the ring faint, and a head with
/// a tail running round once each `EDGE_TURN`.
fn paint_edge_light(
    window: &mut Window,
    placement: Placement,
    color: Rgba,
    opacity: f32,
    since: Instant,
) {
    let out = px(scene::EDGE_GAP + scene::EDGE_WIDTH / 2.0);
    let ring = Bounds::new(
        point(
            placement.picture.origin.x - out,
            placement.picture.origin.y - out,
        ),
        size(
            placement.picture.size.width + out * 2.0,
            placement.picture.size.height + out * 2.0,
        ),
    );
    // Quiet under the light; whole and steady under Reduce Motion, where
    // no light runs to say that something works.
    let still = motion::reduced();
    let rest = if still {
        scene::EDGE_STILL
    } else {
        scene::EDGE_REST
    };
    window.paint_quad(quad(
        ring,
        Corners::all(placement.picture_radius + out),
        theme::with_alpha(color, 0.0),
        px(scene::EDGE_WIDTH),
        theme::with_alpha(color, rest * opacity),
        BorderStyle::Solid,
    ));
    if still {
        return;
    }
    let head = motion::phase(since, scene::EDGE_TURN);
    let pieces = 28;
    for piece in 0..pieces {
        let from = head - scene::EDGE_TAIL * (1.0 - piece as f32 / pieces as f32);
        let to = head - scene::EDGE_TAIL * (1.0 - (piece + 1) as f32 / pieces as f32);
        let strength = (piece + 1) as f32 / pieces as f32;
        let mut path = PathBuilder::stroke(px(scene::EDGE_WIDTH));
        path.move_to(round_point(ring, from));
        // A piece that turns a corner goes by it.
        let corner_from = (from.rem_euclid(1.0) * 4.0).floor();
        let corner_to = (to.rem_euclid(1.0) * 4.0).floor();
        if corner_from != corner_to {
            let middle = (from + to) / 2.0;
            path.line_to(round_point(ring, middle));
        }
        path.line_to(round_point(ring, to));
        if let Ok(path) = path.build() {
            window.paint_path(path, theme::with_alpha(color, opacity * strength));
        }
    }
}

/// What the chip over the picture says of what is being made: pure, so
/// that it can be held to what Qt's said.
pub mod chip {
    /// How a making's line says it stands (`asked`, `waiting … 2 ahead`,
    /// `running 12s`, `rendering 40%`, `made 31s`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Phase {
        Asked,
        /// Its turn on the server not yet come, with how many are ahead.
        Waiting {
            ahead: Option<u32>,
        },
        Running,
        Rendering,
        Made,
        Other,
    }

    impl Phase {
        /// Waiting its turn: queued, not yet making.
        pub fn queued(self) -> bool {
            matches!(self, Phase::Asked | Phase::Waiting { .. })
        }
    }

    /// The first word of `line`, taken whole. The line is whatever the
    /// making's program printed, in any script: it is cut only where a
    /// character ends.
    fn starts_with_word(line: &str, word: &str) -> bool {
        let (Some(head), Some(rest)) = (line.get(..word.len()), line.get(word.len()..)) else {
            return false;
        };
        head.eq_ignore_ascii_case(word)
            && !rest
                .chars()
                .next()
                .is_some_and(|next| next.is_alphanumeric() || next == '_')
    }

    /// How the making's `line` says it stands.
    pub fn phase(line: &str) -> Phase {
        let said = line.trim();
        if starts_with_word(said, "asked") {
            return Phase::Asked;
        }
        if starts_with_word(said, "waiting") {
            // The number just before "ahead".
            let words: Vec<&str> = said.split_whitespace().collect();
            let bare = |word: &str| word.trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
            let ahead = words
                .windows(2)
                .find(|pair| bare(pair[1]).eq_ignore_ascii_case("ahead"))
                .and_then(|pair| bare(pair[0]).parse().ok());
            return Phase::Waiting { ahead };
        }
        for (word, phase) in [
            ("running", Phase::Running),
            ("rendering", Phase::Rendering),
            ("made", Phase::Made),
        ] {
            if starts_with_word(said, word) {
                return phase;
            }
        }
        Phase::Other
    }

    /// Seconds as a clock: `1:05`, or `1:02:05` past the hour.
    pub fn elapsed(seconds: u64) -> String {
        let (hours, minutes, rest) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
        if hours > 0 {
            format!("{hours}:{minutes:02}:{rest:02}")
        } else {
            format!("{minutes}:{rest:02}")
        }
    }

    /// A file's name without its extension.
    pub fn stem(file: &str) -> &str {
        match file.rfind('.') {
            Some(at) if at > 0 => &file[..at],
            _ => file,
        }
    }

    /// The chip for the makings, each its media's name, its line and the
    /// seconds since it began, in the window's order: the one making ahead
    /// of those waiting their turn, "queued, N ahead" while it waits, its
    /// clock while it makes, and " +N" for the others. None with none.
    pub fn makings(makings: &[(String, String, u64)]) -> Option<String> {
        let shown = makings
            .iter()
            .find(|(_, line, _)| !phase(line).queued())
            .or_else(|| makings.first())?;
        let (name, line, seconds) = shown;
        let mut chip = format!("Making {name} · {}", stands(line, *seconds));
        if makings.len() > 1 {
            chip.push_str(&format!(" +{}", makings.len() - 1));
        }
        Some(chip)
    }

    /// How a making stands, by its `line` and the `seconds` since it
    /// began: "queued, 2 ahead" or "queued" while it waits its turn, else
    /// its clock.
    fn stands(line: &str, seconds: u64) -> String {
        match phase(line) {
            Phase::Waiting { ahead: Some(ahead) } if ahead > 0 => format!("queued, {ahead} ahead"),
            other if other.queued() => "queued".to_owned(),
            _ => elapsed(seconds),
        }
    }

    /// What the chip says of the makings while they run, followed in words
    /// as Qt's toolbar status followed them.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Following {
        /// What the one shown last said, as `store::making_status` puts it
        /// ("Making m4: <line>", "Making take 2 of m4 from its steps…"):
        /// cut short on the chip where it has no room.
        pub said: String,
        /// After it, never cut: how it stands ("0:41", "queued, 2 ahead")
        /// and " +N" for the others.
        pub stands: String,
        /// Its tip: each making's words whole and how it stands, a line
        /// each, the one shown first.
        pub tip: String,
    }

    /// The chip for the makings, each its media's name, its take, its line
    /// and the seconds since it began, in the window's order: the one
    /// making ahead of those waiting their turn, as `makings` chooses.
    /// None with none.
    pub fn following(makings: &[(String, crate::store::Making)]) -> Option<Following> {
        following_at(makings, std::time::Instant::now())
    }

    /// The chip for the makings as it stands at `now`: `following`, with
    /// each clock counted from when its making began to `now` (none for
    /// one that begins after it).
    pub fn following_at(
        makings: &[(String, crate::store::Making)],
        now: std::time::Instant,
    ) -> Option<Following> {
        let seconds =
            |making: &crate::store::Making| now.saturating_duration_since(making.since).as_secs();
        let first = makings
            .iter()
            .position(|(_, making)| !phase(&making.line).queued())
            .unwrap_or(0);
        let (name, shown) = makings.get(first)?;
        let mut stands = stands(&shown.line, seconds(shown));
        if makings.len() > 1 {
            stands.push_str(&format!(" +{}", makings.len() - 1));
        }
        let line = |(name, making): &(String, crate::store::Making)| {
            format!(
                "{} · {}",
                crate::store::making_status(name, making),
                self::stands(&making.line, seconds(making))
            )
        };
        let tip = std::iter::once(&makings[first])
            .chain(
                makings
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != first)
                    .map(|(_, making)| making),
            )
            .map(line)
            .collect::<Vec<_>>()
            .join("\n");
        Some(Following {
            said: crate::store::making_status(name, shown),
            stands,
            tip,
        })
    }
}

/// What the preview render's card over the picture stands on, for
/// [`render_card_shows`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderCard {
    /// A preview renders.
    pub rendering: bool,
    /// That render was stopped here, and runs on until the bridge says it
    /// ended ([`render_stop_holds`]).
    pub stopped: bool,
    /// Kerf builds its cut: the picture is its live preview, and the card
    /// keeps out of the way (Qt quietened the HUD).
    pub kerf_building: bool,
    /// A change reel plays (it renders what it plays) or Kerf cuts its
    /// first cut: its own strip says so.
    pub reel: bool,
    /// The picture shows: the panel is open with its pane in.
    pub picture_shown: bool,
}

/// Whether the preview render's card shows over the picture
/// ([`StageView::preview_card`]): Esc stops the render only then, as Qt's
/// filter took it only while the HUD was active and visible
/// (agent_hud.cpp).
pub fn render_card_shows(card: RenderCard) -> bool {
    card.rendering && !card.stopped && !card.kerf_building && !card.reel && card.picture_shown
}

/// Whether a render stopped here (the render of proposal `stopped`) is
/// still the one running after bridge `event`, its card still done with:
/// until its run ends (a `preview` of any other status), or another
/// proposal's render takes over, as the stage keeps its own Stop.
pub fn render_stop_holds(stopped: Option<&str>, event: &serde_json::Value) -> bool {
    let Some(stopped) = stopped else {
        return false;
    };
    if event["type"] != "preview" {
        return true;
    }
    event["status"] == "running" && event["proposalId"].as_str().unwrap_or_default() == stopped
}

/// What the preview render's card over the picture says: pure, so that it
/// can be held to what Qt's said.
pub mod hud {
    use crate::chat::ClipChange;
    use crate::model;

    /// Stop's tip and its key, as Qt's had them: Esc stops the render
    /// while the card shows, ahead of what else Esc would do.
    pub const STOP_TIP: &str = "Stop preview render";
    pub const STOP_KEY: &str = "Esc";

    /// Its title: "Rendering preview · v2", with the version when known.
    pub fn title(version: u32) -> String {
        if version > 0 {
            format!("Rendering preview · v{version}")
        } else {
            "Rendering preview".to_owned()
        }
    }

    /// Its second line: the proposal's first change ("Logo → 00:00:26:12 ·
    /// 2 changes", "Logo removed · 1 change"), else the first line of what
    /// it does, else what the render's event said.
    pub fn detail(changes: &[ClipChange], summary: &str, message: &str, fps: f64) -> String {
        if let Some(first) = changes.first() {
            let name = if first.name.is_empty() {
                &first.clip
            } else {
                &first.name
            };
            let count = match changes.len() {
                1 => " · 1 change".to_owned(),
                n => format!(" · {n} changes"),
            };
            if first.removed {
                return format!("{name} removed{count}");
            }
            return format!("{name} → {}{count}", model::timecode(first.new_start, fps));
        }
        if let Some(line) = summary.lines().map(str::trim).find(|line| !line.is_empty()) {
            return line.to_owned();
        }
        message.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// How far it has got, as a whole percent.
    pub fn percent(progress: f64) -> String {
        format!("{}%", (progress.clamp(0.0, 1.0) * 100.0).round() as i64)
    }
}

/// What the picture's overlays are drawn from: where a clip's picture lands on
/// the canvas (kerf-compose's `placed`), its masks' outlines, and the curve of
/// its motion path (kerf_model's `MotionPath`). Pure functions of the
/// snapshot's timeline as the bridge sends it, so that they can be held to
/// the compositor's own numbers.
pub mod overlay {
    use serde_json::{json, Map, Value};

    /// A point: across, then down.
    pub type Pt = (f64, f64);

    /// A map of the plane that keeps lines straight: `(x, y)` goes to
    /// `(a x + c y + e, b x + d y + f)`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Affine {
        pub a: f64,
        pub b: f64,
        pub c: f64,
        pub d: f64,
        pub e: f64,
        pub f: f64,
    }

    impl Affine {
        pub const IDENTITY: Self = Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };

        pub fn translate(x: f64, y: f64) -> Self {
            Self {
                e: x,
                f: y,
                ..Self::IDENTITY
            }
        }

        pub fn scale(x: f64, y: f64) -> Self {
            Self {
                a: x,
                d: y,
                ..Self::IDENTITY
            }
        }

        /// Turned `degrees` clockwise, y running down.
        pub fn rotate(degrees: f64) -> Self {
            let (sin, cos) = degrees.to_radians().sin_cos();
            Self {
                a: cos,
                b: sin,
                c: -sin,
                d: cos,
                e: 0.0,
                f: 0.0,
            }
        }

        /// This map, and then `next`.
        pub fn then(self, next: Self) -> Self {
            Self {
                a: next.a * self.a + next.c * self.b,
                b: next.b * self.a + next.d * self.b,
                c: next.a * self.c + next.c * self.d,
                d: next.b * self.c + next.d * self.d,
                e: next.a * self.e + next.c * self.f + next.e,
                f: next.b * self.e + next.d * self.f + next.f,
            }
        }

        pub fn apply(self, (x, y): Pt) -> Pt {
            (
                self.a * x + self.c * y + self.e,
                self.b * x + self.d * y + self.f,
            )
        }

        /// The map back; none when it folds the plane onto a line.
        pub fn inverse(self) -> Option<Self> {
            let det = self.a * self.d - self.b * self.c;
            if !det.is_finite() || det.abs() < 1e-12 {
                return None;
            }
            let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
            Some(Self {
                a,
                b,
                c,
                d,
                e: -(a * self.e + c * self.f),
                f: -(b * self.e + d * self.f),
            })
        }
    }

    /// The nearest a camera sees, in frame heights (kerf-compose's `NEAR`):
    /// what is nearer than this along its way, or behind it, is out of sight.
    pub const NEAREST: f64 = 0.001;

    /// What `w` is held to at least as a point is mapped, as QTransform's
    /// near clip has it: a point on or behind the camera still maps somewhere.
    const NEAR_CLIP: f64 = 1e-6;

    /// A map of the plane as a camera may see it, in perspective, in
    /// QTransform's order: `(x, y)` goes to `(m11 x + m21 y + m31,
    /// m12 x + m22 y + m32)` over `w = m13 x + m23 y + m33`, `m[r][c]` being
    /// `m(r+1)(c+1)`. For a placement in depth `w` is how far along the
    /// camera's way the point is, in frame heights; for a flat one it is 1.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Projective {
        pub m: [[f64; 3]; 3],
    }

    impl From<Affine> for Projective {
        fn from(map: Affine) -> Self {
            Self {
                m: [
                    [map.a, map.b, 0.0],
                    [map.c, map.d, 0.0],
                    [map.e, map.f, 1.0],
                ],
            }
        }
    }

    impl Projective {
        /// What a placement is when there is nothing to place on: no map back.
        pub const UNSEEN: Self = Self { m: [[0.0; 3]; 3] };

        /// This map, and then `next`.
        pub fn then(self, next: impl Into<Self>) -> Self {
            let next = next.into();
            let mut m = [[0.0; 3]; 3];
            for (row, out) in m.iter_mut().enumerate() {
                for (column, cell) in out.iter_mut().enumerate() {
                    *cell = (0..3).map(|k| self.m[row][k] * next.m[k][column]).sum();
                }
            }
            Self { m }
        }

        /// How far along the camera's way `(x, y)` is: its `w`.
        fn depth(self, (x, y): Pt) -> f64 {
            self.m[0][2] * x + self.m[1][2] * y + self.m[2][2]
        }

        /// Where `(x, y)` goes. One on or behind the camera goes somewhere
        /// far off; [`Self::in_sight`] says which are seen.
        pub fn apply(self, (x, y): Pt) -> Pt {
            let across = self.m[0][0] * x + self.m[1][0] * y + self.m[2][0];
            let down = self.m[0][1] * x + self.m[1][1] * y + self.m[2][1];
            let w = self.depth((x, y));
            if w == 1.0 {
                return (across, down);
            }
            let w = w.max(NEAR_CLIP);
            (across / w, down / w)
        }

        /// Whether `(x, y)` is in front of the camera, where it can be seen.
        pub fn in_sight(self, at: Pt) -> bool {
            self.depth(at) >= NEAREST
        }

        /// The map back; none when it folds the plane onto a line, or
        /// there is nothing to place on ([`Self::UNSEEN`]).
        pub fn inverse(self) -> Option<Self> {
            let m = self.m;
            let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| {
                m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
            };
            let adjugate = [
                [
                    cofactor(1, 2, 1, 2),
                    -cofactor(0, 2, 1, 2),
                    cofactor(0, 1, 1, 2),
                ],
                [
                    -cofactor(1, 2, 0, 2),
                    cofactor(0, 2, 0, 2),
                    -cofactor(0, 1, 0, 2),
                ],
                [
                    cofactor(1, 2, 0, 1),
                    -cofactor(0, 2, 0, 1),
                    cofactor(0, 1, 0, 1),
                ],
            ];
            let det =
                m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
            if !det.is_finite() || det.abs() < 1e-12 {
                return None;
            }
            Some(Self {
                m: adjugate.map(|row| row.map(|cell| cell / det)),
            })
        }

        /// Whether it places anything: it has a map back.
        pub fn shows(self) -> bool {
            self.inverse().is_some()
        }

        /// The point that is seen at `seen`: none past the horizon, where
        /// nothing of the plane is, and none on or behind the camera.
        pub fn point_in_sight(self, seen: Pt) -> Option<Pt> {
            let back = self.inverse()?;
            if back.depth(seen) <= NEAR_CLIP {
                return None;
            }
            let point = back.apply(seen);
            self.in_sight(point).then_some(point)
        }
    }

    /// How far along a span `interp` is, `t` of the way across
    /// (kerf_model's `Interp::curve`).
    pub fn curve(interp: &str, t: f64) -> f64 {
        match interp {
            "linear" => t,
            "hold" => 0.0,
            "ease-in" => t * t,
            "ease-out" => 1.0 - (1.0 - t) * (1.0 - t),
            "ease" => t * t * (3.0 - 2.0 * t),
            other => bezier(other, t).unwrap_or(t),
        }
    }

    /// A curve written as its two handles, `x1,y1,x2,y2`.
    fn bezier(interp: &str, t: f64) -> Option<f64> {
        let numbers: Vec<f64> = interp
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [x1, y1, x2, y2] = numbers[..] else {
            return None;
        };
        let cubic = |a: f64, b: f64, s: f64| {
            let r = 1.0 - s;
            3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
        };
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..40 {
            let middle = (low + high) / 2.0;
            if cubic(x1, x2, middle) < t {
                low = middle;
            } else {
                high = middle;
            }
        }
        Some(cubic(y1, y2, (low + high) / 2.0))
    }

    /// The number `name` of a clip's `props` at `clip_time` seconds into
    /// it: a value, or its keyframes between; `fallback` when it has none.
    pub fn property_at(props: &Value, name: &str, clip_time: f64, fallback: f64) -> f64 {
        match &props[name] {
            Value::Number(number) => number.as_f64().unwrap_or(fallback),
            Value::Array(keys) if !keys.is_empty() => {
                let at = |index: usize| keys[index]["at"].as_f64().unwrap_or(0.0);
                let value = |index: usize| keys[index]["value"].as_f64().unwrap_or(fallback);
                if clip_time <= at(0) {
                    return value(0);
                }
                for index in 1..keys.len() {
                    if clip_time > at(index) {
                        continue;
                    }
                    let span = at(index) - at(index - 1);
                    let t = if span > 0.0 {
                        (clip_time - at(index - 1)) / span
                    } else {
                        1.0
                    };
                    let interp = keys[index - 1]["interp"].as_str().unwrap_or("linear");
                    return value(index - 1) + (value(index) - value(index - 1)) * curve(interp, t);
                }
                value(keys.len() - 1)
            }
            _ => fallback,
        }
    }

    /// A clip's geometry at a moment: the compositor's `GEOMETRY`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Geometry {
        pub crop: [f64; 4],
        pub scale: f64,
        pub scale_x: f64,
        pub scale_y: f64,
        pub rotation: f64,
        pub anchor: Pt,
        pub at: Pt,
    }

    pub fn geometry_at(props: &Value, clip_time: f64) -> Geometry {
        let prop = |name: &str, fallback: f64| property_at(props, name, clip_time, fallback);
        Geometry {
            crop: [
                prop("crop_l", 0.0),
                prop("crop_r", 0.0),
                prop("crop_t", 0.0),
                prop("crop_b", 0.0),
            ],
            scale: prop("scale", 1.0),
            scale_x: prop("scale_x", 1.0),
            scale_y: prop("scale_y", 1.0),
            rotation: prop("rotation", 0.0),
            anchor: (prop("anchor_x", 0.0), prop("anchor_y", 0.0)),
            at: (prop("x", 0.0), prop("y", 0.0)),
        }
    }

    /// The same with the clip's motion path added in: the path adds to
    /// where it would be, and `orient` to how it is turned.
    pub fn moved_geometry_at(props: &Value, canvas: Pt, clip_time: f64) -> Geometry {
        let mut geometry = geometry_at(props, clip_time);
        if let Some(points) = props["path"].as_str().and_then(parse_path) {
            let on = MotionPath::new(points, aspect(canvas))
                .at(property_at(props, "along", clip_time, 0.0));
            geometry.at.0 += on.x;
            geometry.at.1 += on.y;
            if props["orient"].as_bool() == Some(true) {
                geometry.rotation += on.heading;
            }
        }
        geometry
    }

    fn aspect((width, height): Pt) -> f64 {
        width.max(1.0) / height.max(1.0)
    }

    /// What a picture of the cropped size fills on the canvas, before the
    /// clip's own scale.
    fn filled((width, height): Pt, fit: bool, (cw, ch): Pt) -> Pt {
        if !fit {
            return (width, height);
        }
        (
            (ch * width / height).round().min(cw).max(1.0),
            (cw * height / width).round().min(ch).max(1.0),
        )
    }

    /// Where a source of `picture` pixels lands on the canvas, in canvas
    /// pixels (kerf-compose's `placed`).
    pub fn placed(g: &Geometry, picture: Pt, fit: bool, canvas: Pt) -> Affine {
        let (sw, sh) = (picture.0.max(1.0), picture.1.max(1.0));
        let [l, r, t, b] = g.crop;
        let width = sw * (1.0 - l - r).max(1e-6);
        let height = sh * (1.0 - t - b).max(1e-6);
        let (fw, fh) = filled((width, height), fit, canvas);
        let (ax, ay) = g.anchor;
        Affine::translate(
            -(sw * l + width * (0.5 + ax)),
            -(sh * t + height * (0.5 + ay)),
        )
        .then(Affine::scale(
            fw / width * g.scale * g.scale_x,
            fh / height * g.scale * g.scale_y,
        ))
        .then(Affine::rotate(g.rotation))
        .then(Affine::translate(
            canvas.0 / 2.0 + g.at.0 * canvas.0 + ax * fw,
            canvas.1 / 2.0 + g.at.1 * canvas.1 + ay * fh,
        ))
    }

    /// How a parent has moved its children at its clip time: from where it
    /// would sit with no position, scale or rotation of its own, to where it
    /// is (kerf-compose's `parent_motion`).
    pub fn parent_motion(
        props: &Value,
        picture: Pt,
        fit: bool,
        canvas: Pt,
        clip_time: f64,
    ) -> Affine {
        let now = moved_geometry_at(props, canvas, clip_time);
        let rest = Geometry {
            crop: now.crop,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            anchor: now.anchor,
            at: (0.0, 0.0),
        };
        match placed(&rest, picture, fit, canvas).inverse() {
            Some(back) => back.then(placed(&now, picture, fit, canvas)),
            None => Affine::IDENTITY,
        }
    }

    /// The canvas a timeline is drawn on, in pixels: its sequence's size,
    /// else its largest media's, else 1920 × 1080.
    pub fn canvas_of(timeline: &Value) -> Pt {
        let size = |value: &Value| {
            let (width, height) = (value["width"].as_f64()?, value["height"].as_f64()?);
            (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
                .then_some((width.round().max(2.0), height.round().max(2.0)))
        };
        if let Some(size) = size(timeline) {
            return size;
        }
        timeline["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(size)
            .fold(None, |best: Option<Pt>, size| match best {
                Some(best) if best.0 * best.1 >= size.0 * size.1 => Some(best),
                _ => Some(size),
            })
            .unwrap_or((1920.0, 1080.0))
    }

    /// The clip `id` of `timeline`, and the kind of track it is on.
    pub fn clip_in<'a>(timeline: &'a Value, id: &str) -> Option<(&'a Value, &'a str)> {
        timeline["tracks"].as_array()?.iter().find_map(|track| {
            let clip = track["clips"]
                .as_array()?
                .iter()
                .find(|clip| clip["id"].as_str() == Some(id))?;
            Some((clip, track["type"].as_str().unwrap_or_default()))
        })
    }

    /// The size of the picture a clip on a `track` track draws, and whether
    /// it is fitted to the canvas; none for what draws no picture.
    pub fn picture_of(
        timeline: &Value,
        clip: &Value,
        track: &str,
        canvas: Pt,
    ) -> Option<(Pt, bool)> {
        let kind = clip["kind"].as_str().unwrap_or_default();
        if track != "video" || kind == "caption" {
            return None;
        }
        if kind != "media" {
            // Generators fill the canvas; a nested sequence is fitted to it.
            return Some((canvas, kind == "sequence"));
        }
        let media = clip["sourceId"].as_str()?;
        let source = timeline["sources"]
            .as_array()?
            .iter()
            .find(|source| source["id"].as_str() == Some(media))?;
        let (width, height) = (source["width"].as_f64()?, source["height"].as_f64()?);
        (width > 0.0 && height > 0.0).then_some(((width, height), true))
    }

    /// Where a flat clip's parents have moved it, in canvas pixels, at
    /// `position` seconds into the program.
    pub fn parents_motion(timeline: &Value, clip: &Value, canvas: Pt, position: f64) -> Affine {
        let mut moved = Affine::IDENTITY;
        let mut seen: Vec<String> = Vec::new();
        let mut parent = clip["parent"].as_str().map(str::to_owned);
        while let Some(id) = parent.filter(|id| !id.is_empty() && !seen.contains(id)) {
            let Some((rig, track)) = clip_in(timeline, &id) else {
                break;
            };
            let Some((picture, fit)) = picture_of(timeline, rig, track, canvas) else {
                break;
            };
            let start = rig["timelineStart"].as_f64().unwrap_or(0.0);
            moved = moved.then(parent_motion(
                &rig["props"],
                picture,
                fit,
                canvas,
                position - start,
            ));
            seen.push(id);
            parent = rig["parent"].as_str().map(str::to_owned);
        }
        moved
    }

    /// Where a picture clip is on the canvas at a moment.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Placement {
        /// Fractions of the clip's picture to canvas pixels: where its
        /// masks are. In depth, as the camera sees it; [`Projective::UNSEEN`]
        /// when it cannot be seen (turned edge on, behind the camera).
        pub picture: Projective,
        /// A point of its motion path to fractions of the canvas;
        /// [`Projective::UNSEEN`] for a path in depth no one map shows
        /// ([`sampled`] shows that one).
        pub path: Projective,
        pub canvas: Pt,
        /// Seconds into the clip.
        pub clip_time: f64,
    }

    impl Placement {
        /// Whether its masks can be drawn, chosen and dragged on the
        /// picture: its picture can be seen, not turned edge on or behind
        /// the camera (Qt's `editsMasks`: an invertible placement).
        pub fn masks_seen(&self) -> bool {
            self.picture.shows()
        }

        /// Whether its motion path `points` can be seen through its map:
        /// the map shows, and some of its curve is in front of the camera
        /// (Qt's `setMotionPath` drops one all on or behind it). A path no
        /// one map shows is seen by [`sampled`] instead.
        pub fn path_seen(&self, points: &[Pt]) -> bool {
            let (width, height) = self.canvas;
            self.path.shows()
                && MotionPath::new(points.to_vec(), width.max(1.0) / height.max(1.0))
                    .samples()
                    .iter()
                    .any(|at| self.path.in_sight(*at))
        }
    }

    /// What a clip is placed with at a moment: its picture's size, whether
    /// it is fitted to the canvas, the canvas, seconds into it, how its
    /// parents have moved it on the canvas, and where it starts.
    #[derive(Clone, Copy, Debug)]
    struct Scene {
        picture: Pt,
        fit: bool,
        canvas: Pt,
        clip_time: f64,
        parents: Affine,
        start: f64,
    }

    impl Scene {
        /// Seconds into the program.
        fn at(&self) -> f64 {
            self.start + self.clip_time
        }
    }

    /// The clip `id` of `timeline` and what it is placed with at `position`
    /// seconds into the program; none for a clip that draws no picture.
    fn scene_of<'a>(timeline: &'a Value, id: &str, position: f64) -> Option<(&'a Value, Scene)> {
        let (clip, track) = clip_in(timeline, id)?;
        let canvas = canvas_of(timeline);
        let (picture, fit) = picture_of(timeline, clip, track, canvas)?;
        let start = clip["timelineStart"].as_f64().unwrap_or(0.0);
        let duration = clip["duration"].as_f64().unwrap_or(0.0).max(0.0);
        let clip_time = (position - start).clamp(0.0, duration);
        let parents = parents_motion(timeline, clip, canvas, start + clip_time);
        Some((
            clip,
            Scene {
                picture,
                fit,
                canvas,
                clip_time,
                parents,
                start,
            },
        ))
    }

    /// Where the clip `id` of `timeline` is at `position` seconds into the
    /// program, flat or in depth as the camera sees it; none for a clip
    /// that draws no picture, and one of which nothing can be seen.
    pub fn placement(timeline: &Value, id: &str, position: f64) -> Option<Placement> {
        let (clip, scene) = scene_of(timeline, id, position)?;
        let picture = picture_placement(timeline, clip, &scene);
        let path = path_placement(timeline, clip, &scene);
        if !picture.shows() && !path.shows() && sampled(timeline, id, position, None).is_none() {
            return None;
        }
        Some(Placement {
            picture,
            path,
            canvas: scene.canvas,
            clip_time: scene.clip_time,
        })
    }

    // ------------------------------------------------------------- depth

    /// The field of view at which a clip at z = 0 is its flat self, in
    /// degrees (kerf-compose's `DEFAULT_FOV`).
    const RESTING_FOV: f64 = 40.0;

    /// A point or a way in the compositor's world: x right, y down and z
    /// away from the viewer, in canvas pixels, 0, 0, 0 the canvas's middle.
    type World = [f64; 3];

    fn add(a: World, b: World) -> World {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    fn minus(a: World, b: World) -> World {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn times(a: World, by: f64) -> World {
        [a[0] * by, a[1] * by, a[2] * by]
    }

    fn dot(a: World, b: World) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn cross(a: World, b: World) -> World {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Turned about x by `pitch` and then about y by `yaw`, in degrees.
    fn turned(a: World, pitch: f64, yaw: f64) -> World {
        let (sp, cp) = pitch.to_radians().sin_cos();
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (y, z) = (a[1] * cp - a[2] * sp, a[1] * sp + a[2] * cp);
        [a[0] * cy + z * sy, y, -a[0] * sy + z * cy]
    }

    /// Whether a clip is placed in depth (`three_d`).
    fn in_depth(clip: &Value) -> bool {
        clip["props"]["three_d"].as_bool() == Some(true)
    }

    /// Whether a parent's place in depth is its children's (kerf-compose's
    /// `Rig::in_depth`): a null's, and that of a clip in depth, which a mesh
    /// is without saying so.
    fn leads_in_depth(clip: &Value) -> bool {
        matches!(clip["fx"].as_str(), Some("null" | "mesh")) || in_depth(clip)
    }

    /// A clip's own numbers in depth at `clip_time`: how far back, in canvas
    /// pixels, and how it is turned about x and about y, in degrees.
    struct Depth {
        z: f64,
        pitch: f64,
        yaw: f64,
    }

    fn depth_at(props: &Value, canvas: Pt, clip_time: f64) -> Depth {
        Depth {
            z: property_at(props, "z", clip_time, 0.0) * canvas.1.max(1.0),
            pitch: property_at(props, "rotate_x", clip_time, 0.0),
            yaw: property_at(props, "rotate_y", clip_time, 0.0),
        }
    }

    /// How its parents have taken a clip in depth, as one thing with them
    /// (kerf-compose's `Carried`): where the world's x, y and z go, and
    /// where its middle goes.
    #[derive(Clone, Copy, Debug)]
    struct Carried {
        ways: [World; 3],
        shift: World,
    }

    impl Carried {
        const STILL: Self = Self {
            ways: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            shift: [0.0; 3],
        };

        /// This, and then as a parent takes it: turned about `about` by the
        /// parent's `pitch` and `yaw` (degrees), and moved `back`.
        fn then(self, about: World, pitch: f64, yaw: f64, back: f64) -> Self {
            Self {
                ways: self.ways.map(|way| turned(way, pitch, yaw)),
                shift: add(
                    add(about, turned(minus(self.shift, about), pitch, yaw)),
                    [0.0, 0.0, back],
                ),
            }
        }

        fn way(&self, of: World) -> World {
            add(
                add(times(self.ways[0], of[0]), times(self.ways[1], of[1])),
                times(self.ways[2], of[2]),
            )
        }

        fn point(&self, at: World) -> World {
            add(self.way(at), self.shift)
        }
    }

    /// Fractions of a clip's picture to canvas pixels, flat: the
    /// compositor's geometry with its parents'.
    fn flat_placement(clip: &Value, scene: &Scene) -> Affine {
        let (picture, canvas) = (scene.picture, scene.canvas);
        Affine::scale(picture.0.max(1.0), picture.1.max(1.0))
            .then(placed(
                &moved_geometry_at(&clip["props"], canvas, scene.clip_time),
                picture,
                scene.fit,
                canvas,
            ))
            .then(scene.parents)
    }

    /// How the parents of `clip` have taken it in depth at `at` seconds of
    /// `timeline`, the one it names first (kerf-compose's `Scene::at`):
    /// each somewhere in depth turns it about its own middle, where that
    /// is on the canvas, and moves it back.
    fn carried_by(timeline: &Value, clip: &Value, canvas: Pt, at: f64) -> Carried {
        let mut carried = Carried::STILL;
        let mut seen: Vec<String> = Vec::new();
        let mut parent = clip["parent"].as_str().map(str::to_owned);
        while let Some(id) = parent.filter(|id| !id.is_empty() && !seen.contains(id)) {
            seen.push(id.clone());
            let Some((rig, track)) = clip_in(timeline, &id) else {
                break;
            };
            let Some((picture, fit)) = picture_of(timeline, rig, track, canvas) else {
                break;
            };
            parent = rig["parent"].as_str().map(str::to_owned);
            if !leads_in_depth(rig) {
                continue;
            }
            let start = rig["timelineStart"].as_f64().unwrap_or(0.0);
            let depth = depth_at(&rig["props"], canvas, at - start);
            if depth.z == 0.0 && depth.pitch == 0.0 && depth.yaw == 0.0 {
                continue;
            }
            let scene = Scene {
                picture,
                fit,
                canvas,
                clip_time: at - start,
                parents: parents_motion(timeline, rig, canvas, at),
                start,
            };
            let middle = flat_placement(rig, &scene).apply((0.5, 0.5));
            let about = [middle.0 - canvas.0 / 2.0, middle.1 - canvas.1 / 2.0, 0.0];
            carried = carried.then(about, depth.pitch, depth.yaw, depth.z);
        }
        carried
    }

    /// How a sequence is seen at a moment (kerf-compose's `Scene::at`).
    struct Camera {
        eye: World,
        right: World,
        down: World,
        forward: World,
        focal: f64,
        width: f64,
        height: f64,
    }

    impl Camera {
        /// Where `(a, b)` of the plane `origin + a across + b below` lands,
        /// in canvas pixels; its `w` is how far along the camera's way that
        /// is, in frame heights. Edge on, it is a line at most a pixel wide,
        /// which the compositor's rays miss: unseen.
        fn sees(&self, origin: World, across: World, below: World) -> Projective {
            let from = minus(origin, self.eye);
            let perpendicular = cross(across, below);
            let area = dot(perpendicular, perpendicular).sqrt();
            if area <= 0.0 || dot(from, perpendicular).abs() / area < NEAREST * self.height {
                return Projective::UNSEEN;
            }
            let sideways = add(
                times(self.right, self.focal),
                times(self.forward, self.width / 2.0),
            );
            let downwards = add(
                times(self.down, self.focal),
                times(self.forward, self.height / 2.0),
            );
            let row = |of: World| {
                [
                    dot(of, sideways) / self.height,
                    dot(of, downwards) / self.height,
                    dot(of, self.forward) / self.height,
                ]
            };
            Projective {
                m: [row(across), row(below), row(from)],
            }
        }
    }

    /// `timeline`'s topmost camera showing at `at` seconds, or the resting
    /// one, which sees the plane z = 0 as the flat picture.
    fn camera_at(timeline: &Value, canvas: Pt, at: f64) -> Camera {
        // Times come as seconds in doubles, a hair off the frames they name.
        const SAME_MOMENT: f64 = 1e-9;
        let mut shown: Option<&Value> = None;
        for track in timeline["tracks"].as_array().into_iter().flatten() {
            if track["enabled"].as_bool() == Some(false) {
                continue;
            }
            for clip in track["clips"].as_array().into_iter().flatten() {
                if clip["fx"].as_str() != Some("camera") || clip["enabled"].as_bool() == Some(false)
                {
                    continue;
                }
                let start = clip["timelineStart"].as_f64().unwrap_or(0.0);
                let duration = clip["duration"].as_f64().unwrap_or(0.0);
                if start - SAME_MOMENT <= at && at < start + duration - SAME_MOMENT {
                    shown = Some(clip);
                }
            }
        }
        let empty = Value::Null;
        let camera_clip = shown.unwrap_or(&empty);
        let props = &camera_clip["props"];
        let clip_time = at - camera_clip["timelineStart"].as_f64().unwrap_or(0.0);
        let read = |name: &str, otherwise: f64| property_at(props, name, clip_time, otherwise);
        let (width, height) = (canvas.0.max(1.0), canvas.1.max(1.0));
        let distance = |fov: f64| height / 2.0 / (fov.clamp(1.0, 170.0).to_radians() / 2.0).tan();
        let mut camera = Camera {
            eye: [
                read("x", 0.0) * width,
                read("y", 0.0) * height,
                read("z", 0.0) * height - distance(RESTING_FOV),
            ],
            right: [1.0, 0.0, 0.0],
            down: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            focal: distance(read("fov", RESTING_FOV)),
            width,
            height,
        };
        let (pitch, yaw) = (read("tilt", 0.0), read("pan", 0.0));
        let roll = read("roll", 0.0).to_radians();
        camera.forward = turned([0.0, 0.0, 1.0], pitch, yaw);
        let level_right = turned([1.0, 0.0, 0.0], pitch, yaw);
        let level_down = cross(camera.forward, level_right);
        camera.right = add(
            times(level_right, roll.cos()),
            times(level_down, roll.sin()),
        );
        camera.down = minus(
            times(level_down, roll.cos()),
            times(level_right, roll.sin()),
        );
        if camera_clip["parent"].as_str().is_none_or(str::is_empty) {
            return camera;
        }
        // A camera with a parent is where the parent takes it, looking as
        // the parent turns it (kerf-compose's `Rigged`): across the canvas
        // as a clip is moved, and then in depth.
        let moved = parents_motion(timeline, camera_clip, canvas, at);
        let carried = carried_by(timeline, camera_clip, canvas, at);
        let on = moved.apply((width / 2.0 + camera.eye[0], height / 2.0 + camera.eye[1]));
        camera.eye = carried.point([on.0 - width / 2.0, on.1 - height / 2.0, camera.eye[2]]);
        // By as much as its parents turn a clip on the canvas, however they size it.
        let (sin, cos) = moved.b.atan2(moved.a).sin_cos();
        let way =
            |of: World| carried.way([of[0] * cos - of[1] * sin, of[0] * sin + of[1] * cos, of[2]]);
        camera.right = way(camera.right);
        camera.down = way(camera.down);
        camera.forward = way(camera.forward);
        camera
    }

    /// Fractions of `clip`'s picture to canvas pixels: flat, or in depth
    /// as the camera sees it (kerf-compose's `Plane::of`: lifted to its `z`
    /// and turned about its middle, then where its parents in depth take
    /// it). [`Projective::UNSEEN`] when none of its corners is in sight.
    fn picture_placement(timeline: &Value, clip: &Value, scene: &Scene) -> Projective {
        let flat = flat_placement(clip, scene);
        if !in_depth(clip) {
            return flat.into();
        }
        let canvas = scene.canvas;
        let depth = depth_at(&clip["props"], canvas, scene.clip_time);
        let middle = flat.apply((0.5, 0.5));
        let across = turned([flat.a, flat.b, 0.0], depth.pitch, depth.yaw);
        let below = turned([flat.c, flat.d, 0.0], depth.pitch, depth.yaw);
        let origin = minus(
            minus(
                [
                    middle.0 - canvas.0 / 2.0,
                    middle.1 - canvas.1 / 2.0,
                    depth.z,
                ],
                times(across, 0.5),
            ),
            times(below, 0.5),
        );
        let carried = carried_by(timeline, clip, canvas, scene.at());
        let seen = camera_at(timeline, canvas, scene.at()).sees(
            carried.point(origin),
            carried.way(across),
            carried.way(below),
        );
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        if corners.iter().any(|corner| seen.in_sight(*corner)) {
            seen
        } else {
            Projective::UNSEEN
        }
    }

    /// Where the clip's anchor is in its picture, in fractions of it: what
    /// its motion path takes along.
    fn anchor_of(g: &Geometry) -> Pt {
        let [l, r, t, b] = g.crop;
        (
            l + (1.0 - l - r) * (0.5 + g.anchor.0),
            t + (1.0 - t - b) * (0.5 + g.anchor.1),
        )
    }

    /// A point of `clip`'s motion path to fractions of the canvas. In depth
    /// the path is on the plane at the clip's `z`, where the camera sees it;
    /// [`Projective::UNSEEN`] for a clip that faces along its path
    /// (`orient`) from an anchor off its middle, which no one map shows.
    fn path_placement(timeline: &Value, clip: &Value, scene: &Scene) -> Projective {
        let (canvas, picture) = (scene.canvas, scene.picture);
        let g = geometry_at(&clip["props"], scene.clip_time);
        let [l, r, t, b] = g.crop;
        let width = picture.0.max(1.0) * (1.0 - l - r).max(1e-6);
        let height = picture.1.max(1.0) * (1.0 - t - b).max(1e-6);
        let (fw, fh) = filled((width, height), scene.fit, canvas);
        let (cw, ch) = (canvas.0.max(1.0), canvas.1.max(1.0));
        let origin = (
            0.5 + g.at.0 + g.anchor.0 * fw / cw,
            0.5 + g.at.1 + g.anchor.1 * fh / ch,
        );
        let on_canvas = Affine::translate(origin.0, origin.1)
            .then(Affine::scale(cw, ch))
            .then(scene.parents);
        let to_shares = Affine::scale(1.0 / cw, 1.0 / ch);
        if !in_depth(clip) {
            return on_canvas.then(to_shares).into();
        }
        // The path says where the anchor goes, and the clip turns about its middle.
        let flat = flat_placement(clip, scene);
        let (middle, anchor) = (flat.apply((0.5, 0.5)), flat.apply(anchor_of(&g)));
        let to_middle = [middle.0 - anchor.0, middle.1 - anchor.1, 0.0];
        let depth = depth_at(&clip["props"], canvas, scene.clip_time);
        let swung = add(
            turned(times(to_middle, -1.0), depth.pitch, depth.yaw),
            to_middle,
        );
        if dot(swung, swung) > 1e-12 && clip["props"]["orient"].as_bool() == Some(true) {
            return Projective::UNSEEN;
        }
        let plane = add([-cw / 2.0, -ch / 2.0, depth.z], swung);
        // And where the clip's parents in depth take it, the path's plane with it.
        let carried = carried_by(timeline, clip, canvas, scene.at());
        let seen = camera_at(timeline, canvas, scene.at()).sees(
            carried.point(plane),
            carried.way([1.0, 0.0, 0.0]),
            carried.way([0.0, 1.0, 0.0]),
        );
        Projective::from(on_canvas).then(seen).then(to_shares)
    }

    // ------------------------------------------------------- sampled paths

    /// How many steps a path in depth that no one map shows is sampled in.
    pub const SAMPLED_STEPS: usize = 96;

    /// A path in depth that no one map shows (a clip facing along it from
    /// an anchor off its middle), sampled where the clip's anchor goes, in
    /// fractions of the canvas: none where that is out of sight.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Sampled {
        /// The curve, [`SAMPLED_STEPS`] steps along it.
        pub places: Vec<Option<Pt>>,
        /// Where the clip is on it now.
        pub now: Option<Pt>,
        /// Where its anchor is at each of the path's points.
        pub handles: Vec<Option<Pt>>,
    }

    /// `clip` with its path's points `points` (as `path=` would say them
    /// exactly) and, if given, `along`.
    fn with_path(clip: &Value, points: &[Pt], along: Option<f64>) -> Value {
        let mut moved = clip.clone();
        let text = points
            .iter()
            .map(|(x, y)| format!("{x},{y}"))
            .collect::<Vec<_>>()
            .join(" ");
        moved["props"]["path"] = json!(text);
        if let Some(along) = along {
            moved["props"]["along"] = json!(along);
        }
        moved
    }

    /// Where `placement` puts the clip's `anchor`, in fractions of the canvas;
    /// none when that cannot be seen.
    fn share_of(placement: Projective, anchor: Pt, canvas: Pt) -> Option<Pt> {
        if !placement.shows() || !placement.in_sight(anchor) {
            return None;
        }
        let on = placement.apply(anchor);
        Some((on.0 / canvas.0.max(1.0), on.1 / canvas.1.max(1.0)))
    }

    /// Where the anchor of `clip`, with its path's points `points`, is when
    /// at its point `index`.
    fn anchor_at_point(
        timeline: &Value,
        clip: &Value,
        scene: &Scene,
        points: &[Pt],
        index: usize,
    ) -> Option<Pt> {
        let along = MotionPath::new(points.to_vec(), aspect(scene.canvas)).along_of(index);
        let moved = with_path(clip, points, Some(along));
        let anchor = anchor_of(&geometry_at(&clip["props"], scene.clip_time));
        share_of(
            picture_placement(timeline, &moved, scene),
            anchor,
            scene.canvas,
        )
    }

    /// The clip `id` and what it is placed with, when its path is one no
    /// one map shows: in depth, facing along a path it has, from an anchor
    /// off its middle.
    fn sampled_scene<'a>(
        timeline: &'a Value,
        id: &str,
        position: f64,
    ) -> Option<(&'a Value, Scene, Vec<Pt>)> {
        let (clip, scene) = scene_of(timeline, id, position)?;
        let props = &clip["props"];
        let points = props["path"].as_str().and_then(parse_path)?;
        let facing = in_depth(clip) && props["orient"].as_bool() == Some(true);
        (facing && !path_placement(timeline, clip, &scene).shows()).then_some((clip, scene, points))
    }

    /// The path of the clip `id` at `position` seconds into the program,
    /// sampled, when it is one no one map shows; with its points as
    /// `points` say in place of the document's (a point being dragged).
    /// None for any other clip, and for one whose path is all out of sight.
    pub fn sampled(
        timeline: &Value,
        id: &str,
        position: f64,
        points: Option<&[Pt]>,
    ) -> Option<Sampled> {
        let (clip, scene, written) = sampled_scene(timeline, id, position)?;
        let points = points.map_or(written, <[Pt]>::to_vec);
        let moved = with_path(clip, &points, None);
        let anchor = anchor_of(&geometry_at(&clip["props"], scene.clip_time));
        let places: Vec<Option<Pt>> = (0..=SAMPLED_STEPS)
            .map(|step| {
                let there = with_path(clip, &points, Some(step as f64 / SAMPLED_STEPS as f64));
                share_of(
                    picture_placement(timeline, &there, &scene),
                    anchor,
                    scene.canvas,
                )
            })
            .collect();
        if places.iter().all(Option::is_none) {
            return None;
        }
        let along = property_at(&moved["props"], "along", scene.clip_time, 0.0);
        let now = places[(along.clamp(0.0, 1.0) * SAMPLED_STEPS as f64).round() as usize];
        let handles = (0..points.len())
            .map(|index| anchor_at_point(timeline, clip, &scene, &points, index))
            .collect();
        Some(Sampled {
            places,
            now,
            handles,
        })
    }

    /// Where point `index` of the path `points` of the clip `id` must be for
    /// its anchor to be seen at `seen` (fractions of the canvas), found by
    /// solving from the clip's own placement there; none when no point
    /// puts it there, as past a horizon.
    pub fn sampled_point_for(
        timeline: &Value,
        id: &str,
        position: f64,
        points: &[Pt],
        index: usize,
        seen: Pt,
    ) -> Option<Pt> {
        // Far closer than the document can say a point, a ten-thousandth of the frame.
        const CLOSE: f64 = 1e-7;
        // How far a point is moved to see what that does, and the most a step may take it.
        const NUDGE: f64 = 1e-4;
        const MOST_STEP: f64 = 0.5;
        const MOST_STEPS: usize = 12;
        let (clip, scene, _) = sampled_scene(timeline, id, position)?;
        if index >= points.len() {
            return None;
        }
        let mut trying = points.to_vec();
        let mut landing = |point: Pt| {
            trying[index] = point;
            anchor_at_point(timeline, clip, &scene, &trying, index)
        };
        let mut point = points[index];
        for _ in 0..MOST_STEPS {
            let on = landing(point)?;
            let across = landing((point.0 + NUDGE, point.1))?;
            let down = landing((point.0, point.1 + NUDGE))?;
            let off = (on.0 - seen.0, on.1 - seen.1);
            if off.0.hypot(off.1) < CLOSE {
                return Some(point);
            }
            let (a, b) = ((across.0 - on.0) / NUDGE, (down.0 - on.0) / NUDGE);
            let (c, d) = ((across.1 - on.1) / NUDGE, (down.1 - on.1) / NUDGE);
            let det = a * d - b * c;
            if det.abs() < 1e-12 {
                return None;
            }
            let mut by = ((d * off.0 - b * off.1) / det, (a * off.1 - c * off.0) / det);
            let length = by.0.hypot(by.1);
            if length > MOST_STEP {
                by = (by.0 * MOST_STEP / length, by.1 * MOST_STEP / length);
            }
            point = (point.0 - by.0, point.1 - by.1);
        }
        let on = landing(point)?;
        ((on.0 - seen.0).hypot(on.1 - seen.1) < CLOSE).then_some(point)
    }

    // ------------------------------------------------------------- masks

    /// A mask's outline, in fractions of its clip's picture.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Outline {
        /// Left, top, right, bottom.
        Rect([f64; 4]),
        /// Centre across and down, then the radius across and down.
        Ellipse([f64; 4]),
        Points {
            points: Vec<Pt>,
            closed: bool,
        },
    }

    /// Points an ellipse is drawn and hit with.
    const ELLIPSE_STEPS: usize = 72;

    fn four(value: &Value) -> Option<[f64; 4]> {
        let values = value.as_array()?;
        let mut out = [0.0; 4];
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = values.get(index)?.as_f64()?;
        }
        Some(out)
    }

    /// Fractions are kept to four places, under half a pixel of a 4K
    /// picture, so the document stays readable.
    fn fraction(value: f64) -> f64 {
        let rounded = (value * 10000.0).round() / 10000.0;
        if rounded == 0.0 {
            0.0
        } else {
            rounded
        }
    }

    impl Outline {
        /// A mask's outline as the snapshot carries it.
        pub fn of(mask: &Value) -> Option<Self> {
            if let Some(rect) = four(&mask["rect"]) {
                return Some(Self::Rect(rect));
            }
            if let Some(ellipse) = four(&mask["ellipse"]) {
                return Some(Self::Ellipse(ellipse));
            }
            let points = mask["points"]
                .as_array()?
                .iter()
                .map(|point| Some((point[0].as_f64()?, point[1].as_f64()?)))
                .collect::<Option<Vec<_>>>()?;
            Some(Self::Points {
                points,
                closed: mask["closed"].as_bool().unwrap_or(true),
            })
        }

        /// A rectangle's corners clockwise from its top left, an ellipse's
        /// left, right, top and bottom, or a path's points.
        pub fn handles(&self) -> Vec<Pt> {
            match self {
                Self::Rect([l, t, r, b]) => vec![(*l, *t), (*r, *t), (*r, *b), (*l, *b)],
                Self::Ellipse([x, y, rx, ry]) => {
                    vec![(x - rx, *y), (x + rx, *y), (*x, y - ry), (*x, y + ry)]
                }
                Self::Points { points, .. } => points.clone(),
            }
        }

        /// With its handle `index` dragged to `to`: a rectangle keeps the
        /// opposite corner, an ellipse its centre.
        pub fn with_handle(&self, index: usize, to: Pt) -> Self {
            match self {
                Self::Rect(_) => {
                    let handles = self.handles();
                    let (ox, oy) = handles[(index + 2) % 4];
                    Self::Rect([to.0.min(ox), to.1.min(oy), to.0.max(ox), to.1.max(oy)])
                }
                Self::Ellipse([x, y, rx, ry]) => {
                    if index < 2 {
                        Self::Ellipse([*x, *y, (to.0 - x).abs(), *ry])
                    } else {
                        Self::Ellipse([*x, *y, *rx, (to.1 - y).abs()])
                    }
                }
                Self::Points { points, closed } => {
                    let mut points = points.clone();
                    if let Some(point) = points.get_mut(index) {
                        *point = to;
                    }
                    Self::Points {
                        points,
                        closed: *closed,
                    }
                }
            }
        }

        pub fn moved_by(&self, (dx, dy): Pt) -> Self {
            match self {
                Self::Rect([l, t, r, b]) => Self::Rect([l + dx, t + dy, r + dx, b + dy]),
                Self::Ellipse([x, y, rx, ry]) => Self::Ellipse([x + dx, y + dy, *rx, *ry]),
                Self::Points { points, closed } => Self::Points {
                    points: points.iter().map(|(x, y)| (x + dx, y + dy)).collect(),
                    closed: *closed,
                },
            }
        }

        /// The outline as a line through points, and whether it closes.
        pub fn line(&self) -> (Vec<Pt>, bool) {
            match self {
                Self::Rect(_) => (self.handles(), true),
                Self::Ellipse([x, y, rx, ry]) => (
                    (0..ELLIPSE_STEPS)
                        .map(|step| {
                            let angle = step as f64 / ELLIPSE_STEPS as f64 * std::f64::consts::TAU;
                            (x + rx * angle.cos(), y + ry * angle.sin())
                        })
                        .collect(),
                    true,
                ),
                Self::Points { points, closed } => (points.clone(), *closed),
            }
        }

        /// What an operation carries of it, each fraction to four places.
        pub fn fields(&self) -> Map<String, Value> {
            let mut fields = Map::new();
            match self {
                Self::Rect(rect) => {
                    fields.insert("rect".into(), json!(rect.map(fraction)));
                }
                Self::Ellipse(ellipse) => {
                    fields.insert("ellipse".into(), json!(ellipse.map(fraction)));
                }
                Self::Points { points, closed } => {
                    let points: Vec<[f64; 2]> = points
                        .iter()
                        .map(|(x, y)| [fraction(*x), fraction(*y)])
                        .collect();
                    fields.insert("points".into(), json!(points));
                    fields.insert("closed".into(), json!(closed));
                }
            }
            fields
        }
    }

    /// A mask on a clip's picture, as the picture shows it.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Mask {
        pub name: String,
        pub mode: String,
        pub outline: Outline,
    }

    impl Mask {
        /// A path only (mode none): it cuts nothing, and is drawn dashed.
        pub fn path_only(&self) -> bool {
            self.mode == "none"
        }
    }

    /// A clip's masks, in the order they combine.
    pub fn masks_of(clip: &Value) -> Vec<Mask> {
        clip["masks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|mask| {
                Some(Mask {
                    name: mask["name"].as_str()?.to_owned(),
                    mode: mask["mode"].as_str().unwrap_or("add").to_owned(),
                    outline: Outline::of(mask)?,
                })
            })
            .collect()
    }

    /// The edit that gives the mask `name` of `clip` the outline `outline`.
    pub fn mask_set(clip: &str, name: &str, outline: &Outline) -> Value {
        let mut op = outline.fields();
        op.insert("op".into(), json!("mask-set"));
        op.insert("clip".into(), json!(clip));
        op.insert("name".into(), json!(name));
        Value::Object(op)
    }

    /// The names of a clip's masks, whatever their outlines.
    pub fn mask_names(clip: &Value) -> Vec<String> {
        clip["masks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|mask| mask["name"].as_str().map(str::to_owned))
            .collect()
    }

    /// The name a new mask takes: the first of `k1`, `k2`, … not `taken`.
    pub fn new_mask_name(taken: &[String]) -> String {
        (1..)
            .map(|n| format!("k{n}"))
            .find(|name| !taken.contains(name))
            .unwrap_or_default()
    }

    /// The edit that adds the mask `name` with the outline `outline` to
    /// `clip`, after its others.
    pub fn mask_add(clip: &str, name: &str, outline: &Outline) -> Value {
        let mut op = outline.fields();
        op.insert("op".into(), json!("mask-add"));
        op.insert("clip".into(), json!(clip));
        op.insert("name".into(), json!(name));
        Value::Object(op)
    }

    /// A rectangle, or the ellipse inside it, dragged out from `from` to
    /// `to` (fractions of the clip's picture), corner to corner.
    pub fn dragged(ellipse: bool, from: Pt, to: Pt) -> Outline {
        let (left, right) = (from.0.min(to.0), from.0.max(to.0));
        let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
        if ellipse {
            Outline::Ellipse([
                (left + right) / 2.0,
                (top + bottom) / 2.0,
                (right - left) / 2.0,
                (bottom - top) / 2.0,
            ])
        } else {
            Outline::Rect([left, top, right, bottom])
        }
    }

    /// Points a drag must go, across or down on the picture, to draw a
    /// rectangle or an ellipse rather than be a click.
    pub const DRAW_LEAST: f64 = 4.0;

    /// The mask a rectangle or ellipse tool draws when its drag from `from`
    /// to `to` is let go, `map` taking the clip's picture to the window.
    /// None for a click (under [`DRAW_LEAST`] points both across and down),
    /// and for an outline too thin to hold an area as the document keeps it.
    pub fn drawn(ellipse: bool, from: Pt, to: Pt, map: impl Into<Projective>) -> Option<Outline> {
        let map = map.into();
        let (a, b) = (map.apply(from), map.apply(to));
        if (a.0 - b.0).abs() < DRAW_LEAST && (a.1 - b.1).abs() < DRAW_LEAST {
            return None;
        }
        let outline = dragged(ellipse, from, to);
        let holds = match &outline {
            Outline::Rect([l, t, r, b]) => {
                fraction(*r) > fraction(*l) && fraction(*b) > fraction(*t)
            }
            Outline::Ellipse([_, _, rx, ry]) => fraction(*rx) > 0.0 && fraction(*ry) > 0.0,
            Outline::Points { .. } => true,
        };
        holds.then_some(outline)
    }

    /// The caption-overlap box a finished check puts over the picture: where
    /// (fractions of the frame: left, top, width, height) and what it measured.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Overlap {
        pub rect: [f64; 4],
        /// "8 px"; empty when the finding measured nothing.
        pub tag: String,
    }

    /// A number as a person reads it: four figures at most, no trailing zeros.
    fn figures(value: f64) -> String {
        if value.fract() == 0.0 && value.abs() < 1e9 {
            return format!("{value:.0}");
        }
        let digits = (4 - 1 - value.abs().log10().floor() as i32).clamp(0, 6) as usize;
        let text = format!("{value:.digits$}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        text.to_owned()
    }

    /// The `text.overlap` finding of `check` (a `check` event, done) whose
    /// frames the playhead at `playhead` is on, give or take half a frame
    /// at `fps`: the first such with an area.
    pub fn overlap_at(check: &Value, playhead: f64, fps: f64) -> Option<Overlap> {
        let slack = 0.5 / fps.max(1.0);
        check["findings"].as_array()?.iter().find_map(|finding| {
            if finding["code"] != "text.overlap" {
                return None;
            }
            let rect = &finding["rect"];
            let (x, y) = (rect["x"].as_f64()?, rect["y"].as_f64()?);
            let (w, h) = (rect["w"].as_f64()?, rect["h"].as_f64()?);
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            if let Some(at) = finding["at"].as_f64() {
                let end = at + finding["duration"].as_f64().unwrap_or(0.0).max(0.0);
                if playhead < at - slack || playhead > end + slack {
                    return None;
                }
            }
            let tag = match &finding["value"] {
                Value::Number(number) => {
                    let number = figures(number.as_f64().unwrap_or(0.0));
                    match finding["unit"].as_str().filter(|unit| !unit.is_empty()) {
                        Some(unit) => format!("{number} {unit}"),
                        None => number,
                    }
                }
                Value::String(text) => text.clone(),
                _ => String::new(),
            };
            Some(Overlap {
                rect: [x, y, w, h],
                tag,
            })
        })
    }

    /// The distance from `at` to the line through `points`.
    pub fn distance_to_line(points: &[Pt], closed: bool, at: Pt) -> f64 {
        let segment = |from: Pt, to: Pt| {
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (from.0 + dx * t - at.0).hypot(from.1 + dy * t - at.1)
        };
        let mut nearest = f64::INFINITY;
        for pair in points.windows(2) {
            nearest = nearest.min(segment(pair[0], pair[1]));
        }
        if closed && points.len() > 2 {
            nearest = nearest.min(segment(points[points.len() - 1], points[0]));
        }
        if points.len() == 1 {
            nearest = (points[0].0 - at.0).hypot(points[0].1 - at.1);
        }
        nearest
    }

    /// Whether `at` is inside the closed line through `points`.
    pub fn inside(points: &[Pt], at: Pt) -> bool {
        let mut inside = false;
        let mut last = match points.last() {
            Some(last) => *last,
            None => return false,
        };
        for &point in points {
            if (point.1 > at.1) != (last.1 > at.1)
                && at.0 < (last.0 - point.0) * (at.1 - point.1) / (last.1 - point.1) + point.0
            {
                inside = !inside;
            }
            last = point;
        }
        inside
    }

    // ------------------------------------------------------- motion paths

    /// How many samples each span between two points of a path has
    /// (kerf_model's `path::SAMPLES`).
    pub const SAMPLES_PER_SPAN: usize = 32;
    /// How far out a path's point can be, either way, in frames.
    const PATH_LIMIT: f64 = 10.0;

    /// The points of a clip's `path=`, `x,y x,y`; none when it does not
    /// read as the document requires.
    pub fn parse_path(text: &str) -> Option<Vec<Pt>> {
        let points = text
            .split_whitespace()
            .map(|point| {
                let (x, y) = point.split_once(',')?;
                let (x, y): (f64, f64) = (x.parse().ok()?, y.parse().ok()?);
                (x.is_finite() && y.is_finite() && x.abs() <= PATH_LIMIT && y.abs() <= PATH_LIMIT)
                    .then_some((x, y))
            })
            .collect::<Option<Vec<_>>>()?;
        (points.len() >= 2).then_some(points)
    }

    /// `points` as `path=` writes them, each held to what it allows.
    pub fn path_text(points: &[Pt]) -> String {
        let text = |value: f64| {
            let rounded = (value.clamp(-PATH_LIMIT, PATH_LIMIT) * 10000.0).round() / 10000.0;
            format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
        };
        points
            .iter()
            .map(|(x, y)| format!("{},{}", text(*x), text(*y)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Whether `path=` can say `point` as it is.
    pub fn path_holds((x, y): Pt) -> bool {
        x.abs() <= PATH_LIMIT && y.abs() <= PATH_LIMIT
    }

    /// How near two points of a path are to be the same one: more than
    /// `path=` rounds a point by.
    const PATH_SAME: f64 = 0.0002;

    /// The motion path of the clip `id`, if it has one.
    pub fn path_of(timeline: &Value, id: &str) -> Option<Vec<Pt>> {
        let (clip, _) = clip_in(timeline, id)?;
        clip["props"]["path"].as_str().and_then(parse_path)
    }

    /// Where the point at `index` of `before` is in `after`: the one at its
    /// place, as `path=` keeps it. None when `after` has no point there, as
    /// an undo can leave another point at that index.
    pub fn follow_point(before: &[Pt], index: usize, after: &[Pt]) -> Option<usize> {
        let (x, y) = *before.get(index)?;
        after
            .iter()
            .position(|(across, down)| (across - x).hypot(down - y) < PATH_SAME)
    }

    /// The edit that starts a motion path on the clip `id` at `position`
    /// seconds into the program: from where the clip is, a quarter of the
    /// canvas towards its middle, with `along` keyed from 0 to 1 over the
    /// clip unless it says already how far along it is. None for a clip
    /// that draws no picture, or has a path already.
    pub fn start_path(timeline: &Value, id: &str, position: f64) -> Option<Value> {
        let placement = placement(timeline, id, position)?;
        let (clip, _) = clip_in(timeline, id)?;
        let props = &clip["props"];
        if props["path"]
            .as_str()
            .is_some_and(|path| !path.trim().is_empty())
        {
            return None;
        }
        let (from, _) = placement.path.apply((0.0, 0.0));
        let toward = if from > 0.5 { -0.25 } else { 0.25 };
        let mut set = Map::new();
        set.insert(
            "path".into(),
            json!(path_text(&[(0.0, 0.0), (toward, 0.0)])),
        );
        if props.get("along").is_none_or(Value::is_null) {
            let last = (clip["durationFrames"].as_i64().unwrap_or(1) - 1).max(1);
            set.insert(
                "along".into(),
                json!({"keys": [
                    {"at": 0, "value": 0.0, "interp": "linear"},
                    {"at": last, "value": 1.0, "interp": "linear"}
                ]}),
            );
        }
        Some(json!({"op": "set", "clip": id, "props": set}))
    }

    /// `points` with one more, on their curve `u` spans along.
    pub fn with_point_on(points: &[Pt], aspect: f64, u: f64) -> Vec<Pt> {
        let on = MotionPath::new(points.to_vec(), aspect).curve(u);
        let span = (u.floor().max(0.0) as usize).min(points.len().saturating_sub(2));
        let mut points = points.to_vec();
        points.insert(span + 1, on);
        points
    }

    /// Where a clip is on its path: a point, and the way it heads, in
    /// degrees clockwise from the right.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct OnPath {
        pub x: f64,
        pub y: f64,
        pub heading: f64,
    }

    /// The curve through a clip's path points (centripetal Catmull-Rom), on
    /// a canvas `aspect` times as wide as it is high.
    #[derive(Clone, Debug)]
    pub struct MotionPath {
        points: Vec<Pt>,
        aspect: f64,
        distances: Vec<f64>,
    }

    impl MotionPath {
        /// From two points up.
        pub fn new(points: Vec<Pt>, aspect: f64) -> Self {
            let mut path = Self {
                points,
                aspect,
                distances: Vec::new(),
            };
            let total = path.points.len().saturating_sub(1) * SAMPLES_PER_SPAN;
            let mut distances = Vec::with_capacity(total + 1);
            distances.push(0.0);
            let mut last = path.curve(0.0);
            let mut walked = 0.0;
            for sample in 1..=total {
                let at = path.curve(sample as f64 / SAMPLES_PER_SPAN as f64);
                walked += ((at.0 - last.0) * aspect).hypot(at.1 - last.1);
                distances.push(walked);
                last = at;
            }
            path.distances = distances;
            path
        }

        /// On the curve, `u` spans along.
        pub fn curve(&self, u: f64) -> Pt {
            if self.points.len() < 2 {
                return self.points.first().copied().unwrap_or((0.0, 0.0));
            }
            let last = self.points.len() - 1;
            let span = (u.floor().max(0.0) as usize).min(last - 1);
            let t = (u - span as f64).clamp(0.0, 1.0);
            let (p1, p2) = (self.points[span], self.points[span + 1]);
            // Past the ends, a point continuing straight on.
            let p0 = if span == 0 {
                (2.0 * p1.0 - p2.0, 2.0 * p1.1 - p2.1)
            } else {
                self.points[span - 1]
            };
            let p3 = if span + 2 > last {
                (2.0 * p2.0 - p1.0, 2.0 * p2.1 - p1.1)
            } else {
                self.points[span + 2]
            };
            // Knots a square root of the distance apart, on the canvas.
            let gap = |a: Pt, b: Pt| {
                ((a.0 - b.0) * self.aspect)
                    .hypot(a.1 - b.1)
                    .sqrt()
                    .max(1e-9)
            };
            let t0 = 0.0;
            let t1 = t0 + gap(p0, p1);
            let t2 = t1 + gap(p1, p2);
            let t3 = t2 + gap(p2, p3);
            let at = t1 + (t2 - t1) * t;
            let lerp = |a: Pt, b: Pt, low: f64, high: f64| {
                let k = (at - low) / (high - low);
                (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
            };
            // Barry and Goldman's pyramid.
            let a1 = lerp(p0, p1, t0, t1);
            let a2 = lerp(p1, p2, t1, t2);
            let a3 = lerp(p2, p3, t2, t3);
            let b1 = lerp(a1, a2, t0, t2);
            let b2 = lerp(a2, a3, t1, t3);
            lerp(b1, b2, t1, t2)
        }

        fn total(&self) -> f64 {
            self.distances.last().copied().unwrap_or(0.0)
        }

        /// How far along it is at its point `index`, as [`Self::at`] takes it.
        pub fn along_of(&self, index: usize) -> f64 {
            let total = self.total();
            if total <= 0.0 {
                return 0.0;
            }
            let index = index.min(self.points.len().saturating_sub(1)) * SAMPLES_PER_SPAN;
            self.distances.get(index).copied().unwrap_or(total) / total
        }

        fn at_distance(&self, along: f64) -> f64 {
            let total = self.total();
            if total <= 0.0 || self.distances.len() < 2 {
                return 0.0;
            }
            let wanted = along.clamp(0.0, 1.0) * total;
            let found = self
                .distances
                .partition_point(|distance| *distance < wanted);
            let next = found.clamp(1, self.distances.len() - 1);
            let (low, high) = (self.distances[next - 1], self.distances[next]);
            let within = if high > low {
                (wanted - low) / (high - low)
            } else {
                0.0
            };
            ((next - 1) as f64 + within) / SAMPLES_PER_SPAN as f64
        }

        /// Where a clip `along` of the way along is, from 0 to 1 of the distance.
        pub fn at(&self, along: f64) -> OnPath {
            let u = self.at_distance(along);
            let (x, y) = self.curve(u);
            let last = self.points.len().saturating_sub(1) as f64;
            const STEP: f64 = 1e-3;
            let before = self.curve((u - STEP).max(0.0));
            let after = self.curve((u + STEP).min(last));
            let heading = (after.1 - before.1)
                .atan2((after.0 - before.0) * self.aspect)
                .to_degrees();
            OnPath { x, y, heading }
        }

        /// The curve as a line, [`SAMPLES_PER_SPAN`] samples to each span.
        pub fn samples(&self) -> Vec<Pt> {
            let total = self.points.len().saturating_sub(1) * SAMPLES_PER_SPAN;
            (0..=total)
                .map(|sample| self.curve(sample as f64 / SAMPLES_PER_SPAN as f64))
                .collect()
        }
    }
}
