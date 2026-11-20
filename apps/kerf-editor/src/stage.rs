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
use overlay::{Affine, MotionPath, Outline, Overlap, Pt};
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

fn place(area: Bounds<Pixels>, aspect: f32) -> Placement {
    let phone = aspect < 1.2;
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
}

impl Shown {
    /// Whether the picture's clicks are its own: there is something to drag.
    fn edits(&self) -> bool {
        !self.masks.is_empty() || self.path.is_some()
    }

    fn aspect(&self) -> f64 {
        let (width, height) = self.placement.canvas;
        width.max(1.0) / height.max(1.0)
    }

    /// Fractions of the clip's picture to the window, with the canvas drawn
    /// at `picture`.
    fn to_window(&self, picture: Bounds<Pixels>) -> Affine {
        let (width, height) = self.placement.canvas;
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement
            .picture
            .then(Affine::scale(
                across / width.max(1.0),
                down / height.max(1.0),
            ))
            .then(Affine::translate(x, y))
    }

    /// A point of the clip's motion path to the window.
    fn path_to_window(&self, picture: Bounds<Pixels>) -> Affine {
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement
            .path
            .then(Affine::scale(across, down))
            .then(Affine::translate(x, y))
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
        let map = self.to_window(picture);
        square_at(&mask.outline.handles(), map, at)
    }

    /// The mask under `at`: the chosen one first, then the last drawn.
    fn mask_at(&self, picture: Bounds<Pixels>, chosen: Option<&str>, at: Pt) -> Option<String> {
        let map = self.to_window(picture);
        let hit = |mask: &overlay::Mask| {
            let (line, closed) = mask.outline.line();
            let line: Vec<Pt> = line.into_iter().map(|point| map.apply(point)).collect();
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
        square_at(self.path.as_ref()?, self.path_to_window(picture), at)
    }

    /// How far along the path's curve `at` is, in spans, when it is on it.
    fn curve_at(&self, picture: Bounds<Pixels>, at: Pt) -> Option<f64> {
        let points = self.path.as_ref()?;
        let map = self.path_to_window(picture);
        let line: Vec<Pt> = MotionPath::new(points.clone(), self.aspect())
            .samples()
            .into_iter()
            .map(|point| map.apply(point))
            .collect();
        let mut nearest = REACH;
        let mut found = None;
        for (index, pair) in line.windows(2).enumerate() {
            let (from, to) = (pair[0], pair[1]);
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

/// The square of `points`, mapped by `map`, under `at`: the last drawn first.
fn square_at(points: &[Pt], map: Affine, at: Pt) -> Option<usize> {
    points.iter().rposition(|point| {
        let (x, y) = map.apply(*point);
        (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
    })
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
    said: Option<String>,
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

/// What a transport control's tip says: its name while it can be pressed;
/// greyed, its name and why not (Qt's tipWithReason).
pub fn transport_tip(name: &str, reason: Option<&str>) -> String {
    match reason.filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("{name} · {reason}"),
        None => name.to_owned(),
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
                        let running = event.0["status"] == "running";
                        let proposal = event.0["proposalId"].as_str().unwrap_or_default();
                        if !running || this.stopped.as_deref() != Some(proposal) {
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
    fn activity(&self, cx: &App) -> (Option<Hue>, Option<String>) {
        let session = self.session.read(cx);
        let state = &session.state;
        if !state.makings.is_empty() {
            let makings: Vec<(String, String, u64)> = state
                .makings
                .iter()
                .map(|(media, making)| {
                    // As the band names it: no content hash, a shot by its name.
                    let name = Some(session.media_name(media))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| media.clone());
                    (name, making.line.clone(), making.since.elapsed().as_secs())
                })
                .collect();
            return (Some(Hue::Computer), chip::makings(&makings));
        }
        // A change reel's previews and a first cut's are the reel's and the
        // cut card's to say (Qt's quietenCutPreview); an export still is.
        let quiet = state.preview.rendering() && (session.reel().is_some() || session.cutting());
        if let Some(rendering) = state.rendering().filter(|_| !quiet) {
            return (Some(Hue::Computer), Some(rendering));
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
            return (Some(Hue::Kerf), Some(words.to_owned()));
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
    /// program; none while a proposal's cut or its preview stands in for it.
    fn overlays(&self, session: &Session) -> Option<Shown> {
        let state = &session.state;
        if state.snapshot.proposal_timeline.is_some()
            || (state.preview.proposal.is_some()
                && matches!(state.preview.status.as_str(), "running" | "ready" | "live"))
        {
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

    /// Whether there is a picture clip chosen that a mask tool can draw on.
    fn can_draw(&self, cx: &App) -> bool {
        self.overlays(self.session.read(cx)).is_some()
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
                session
                    .state
                    .notify("Choose a picture clip to draw a mask on", false);
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
        self.pen.len() >= 3
            && self.pen.first().is_some_and(|first| {
                let (x, y) = shown.to_window(picture).apply(*first);
                (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
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
        let Some(back) = shown.to_window(picture).inverse() else {
            return;
        };
        let on = back.apply(at);
        match tool {
            MaskTool::Pen => {
                if self.pen_closes(shown, picture, at) {
                    self.finish_pen(true, cx);
                    return;
                }
                // A second press of a double-click is not another corner.
                let again = self.pen.last().is_some_and(|last| {
                    apart(shown.to_window(picture).apply(*last), at) < DRAG_START
                });
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
            // outside it), and does nothing else: it would play.
            cx.notify();
            return;
        }
        if let (Some(tool), Some(shown)) = (self.tool, self.shown.clone()) {
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
            if let Some(index) = shown.point_at(picture, at) {
                // One thing chosen at a time.
                self.choose_mask(None, cx);
                self.chosen_point = Some(index);
                let (x, y) = shown.path_to_window(picture).apply(points[index]);
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
            if self.chosen_point.take().is_some() && shown.masks.is_empty() {
                return true;
            }
        }
        if shown.masks.is_empty() {
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
        let map = shown.to_window(picture);
        let found = name.and_then(|name| {
            let mask = shown.masks.iter().find(|mask| mask.name == name)?;
            Some((name, mask.outline.clone(), map.inverse()?))
        });
        if let Some((name, outline, back)) = found {
            self.drag = Some(Drag::Mask {
                clip,
                name,
                handle,
                from: back.apply(at),
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
        if self.tool.is_some() && self.shown.is_some() {
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
        let (playing, length, held, position) = {
            let state = &self.session.read(cx).state;
            (
                state.playing,
                state.program_length(),
                state.preview.status == "required",
                state.position,
            )
        };
        if playing || length <= 0.0 || held || !picture.contains(&to_point(at)) {
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
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "seek", "position": time, "scrub": true}));
        });
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
        let (playing, length, held, position) = {
            let state = &self.session.read(cx).state;
            (
                state.playing,
                state.program_length(),
                state.preview.status == "required",
                state.position,
            )
        };
        if playing || length <= 0.0 || held || !time.is_finite() {
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
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "seek", "position": time, "scrub": true}));
        });
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
        match &mut self.drag {
            Some(Drag::Point {
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
                if let Some(back) = shown.path_to_window(picture).inverse() {
                    let to = back.apply((at.0 + held.0, at.1 + held.1));
                    if overlay::path_holds(to) {
                        now[*index] = to;
                    }
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
                if let Some(back) = shown.to_window(picture).inverse() {
                    let to = back.apply(at);
                    *now = match handle {
                        Some(handle) => original.with_handle(*handle, to),
                        None => original.moved_by((to.0 - from.0, to.1 - from.1)),
                    };
                }
            }
            Some(Drag::Draw { to, .. }) => {
                if let Some(back) = shown.to_window(picture).inverse() {
                    *to = back.apply(at);
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
            let map = shown.to_window(picture);
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
    /// picture it is one line cut at title safe; on a vertical one it wraps
    /// within title safe, balanced: the narrowest width that keeps the
    /// lines the widest needs.
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

    /// What the preview render's card says while one runs, unless Stop was
    /// pressed on it or Kerf is building a cut (the picture just keeps up):
    /// its title, its line, and how far it has got.
    fn preview_card(&self, cx: &App) -> Option<(String, String, f64)> {
        let session = self.session.read(cx);
        let state = &session.state;
        let preview = &state.preview;
        if !preview.rendering() {
            return None;
        }
        let proposal_id = preview.proposal.clone().unwrap_or_default();
        if self.stopped.as_deref() == Some(proposal_id.as_str()) {
            return None;
        }
        if state.work.busy() && state.snapshot.proposal_timeline.is_some() {
            return None;
        }
        // A change reel renders what it plays: its strip says so.
        if session.reel().is_some() || session.cutting() {
            return None;
        }
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
                .filter(|command| TRANSPORT.iter().any(|(id, _, _)| *id == command.id))
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
        let drawable = self.overlays(session).is_some();
        let preview = session.state.preview.clone();
        let preview_control = match preview.status.as_str() {
            "required" => Some(
                ui::button("render-preview", "Render preview")
                    .small()
                    .icon(Icon::Sparkle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, _| session.preview(None))
                    }))
                    .into_any_element(),
            ),
            // While it renders, its card over the picture says so, with Stop.
            "ready" | "live" if preview.proposal.is_some() => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::chip("Previewing the proposal", theme::pos()))
                    .child(
                        ui::button("exit-preview", "Back to the cut")
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
                    .child(
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
                        // Its menu says all the tip would: none while it is open.
                        .when(self.zoom_menu.is_none(), |button| {
                            button.tooltip(|_, cx| {
                                cx.new(|_| ui::Tooltip("Viewer zoom".into())).into()
                            })
                        })
                        .child(zoom_words(self.zoom)),
                    )
                    .children(self.zoom_menu()),
            )
            .child(ui::hairline(false).h(px(16.0)).mx(px(6.0)))
            .children(MaskTool::ALL.map(|tool| {
                let words = format!(
                    "{} ({})",
                    tool.label(),
                    crate::commands::shown_keys(tool.shortcut())
                );
                icon_button(tool.id(), tool.icon())
                    .on(self.tool == Some(tool))
                    .disabled(!drawable)
                    .tooltip(if drawable {
                        words
                    } else {
                        format!("{words} · Choose a picture clip to draw on")
                    })
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
        // down, and what was in hand is let go.
        if shown.is_none() {
            self.drag = None;
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
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left + px(12.0))
                        .top(top + px(if placed.phone { 36.0 } else { 12.0 }))
                        .child(
                            theme::type_style(div(), text::CONTROL)
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .h(px(24.0))
                                .px(px(10.0))
                                .rounded(px(12.0))
                                .bg(theme::activity::chip())
                                .text_color(theme::ink())
                                .child(ui::dot(color, 6.0).opacity(dot))
                                .child(chip),
                        )
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
                .then(|| self.caption_box(window, picture, aspect < 1.0, words.clone()));
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
                                .when(aspect >= 1.0, |line| {
                                    line.whitespace_nowrap().overflow_hidden().text_ellipsis()
                                })
                                .child(words),
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
                                    ui::button("stop-preview", "Stop").small().ghost().on_click(
                                        cx.listener(|this, _, _, cx| {
                                            let proposal = this
                                                .session
                                                .read(cx)
                                                .state
                                                .preview
                                                .proposal
                                                .clone()
                                                .unwrap_or_default();
                                            this.stopped = Some(proposal);
                                            this.session.update(cx, |session, cx| {
                                                session.send(json!({"type": "preview_stop"}));
                                                cx.notify();
                                            });
                                            cx.notify();
                                        }),
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
    let map = shown.to_window(picture);
    for mask in &shown.masks {
        let chosen = chosen_mask == Some(mask.name.as_str());
        let (line, closed) = mask.outline.line();
        let line: Vec<Point<Pixels>> = line.into_iter().map(|at| to_point(map.apply(at))).collect();
        stroke_over(window, &line, closed, chosen, mask.path_only());
        if let Some(anchor) = top_left(&line) {
            paint_tag(window, cx, anchor, &mask.name, chosen, picture);
        }
        if chosen {
            for handle in mask.outline.handles() {
                paint_handle(window, to_point(map.apply(handle)), false);
            }
        }
    }
    if let Some(points) = &shown.path {
        let map = shown.path_to_window(picture);
        let path = MotionPath::new(points.clone(), shown.aspect());
        let line: Vec<Point<Pixels>> = path
            .samples()
            .into_iter()
            .map(|at| to_point(map.apply(at)))
            .collect();
        stroke_over(window, &line, false, true, false);
        // The points are square, as a mask's handles are; the chosen one white.
        for (index, at) in points.iter().enumerate() {
            paint_handle(
                window,
                to_point(map.apply(*at)),
                chosen_point == Some(index),
            );
        }
        let on = path.at(shown.along);
        let center = to_point(map.apply((on.x, on.y)));
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
    }
}

/// What a mask tool has drawn so far: a rectangle or an ellipse dragged
/// out, or the pen's corners with a dashed side to the pointer, its first
/// corner white when a click there would close the outline.
fn paint_drawn(window: &mut Window, picture: Bounds<Pixels>, shown: &Shown, drawn: &Drawn) {
    let map = shown.to_window(picture);
    if let Some(outline) = &drawn.outline {
        let (line, closed) = outline.line();
        let line: Vec<Point<Pixels>> = line.into_iter().map(|at| to_point(map.apply(at))).collect();
        stroke_over(window, &line, closed, true, false);
    }
    let corners: Vec<Pt> = drawn.pen.iter().map(|at| map.apply(*at)).collect();
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
        let mut chip = format!("Making {name} · ");
        match phase(line) {
            Phase::Waiting { ahead: Some(ahead) } if ahead > 0 => {
                chip.push_str(&format!("queued, {ahead} ahead"))
            }
            other if other.queued() => chip.push_str("queued"),
            _ => chip.push_str(&elapsed(*seconds)),
        }
        if makings.len() > 1 {
            chip.push_str(&format!(" +{}", makings.len() - 1));
        }
        Some(chip)
    }
}

/// What the preview render's card over the picture says: pure, so that it
/// can be held to what Qt's said.
pub mod hud {
    use crate::chat::ClipChange;
    use crate::model;

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
        /// Fractions of the clip's picture to canvas pixels: where its masks are.
        pub picture: Affine,
        /// A point of its motion path to fractions of the canvas.
        pub path: Affine,
        pub canvas: Pt,
        /// Seconds into the clip.
        pub clip_time: f64,
    }

    /// Where the clip `id` of `timeline` is at `position` seconds into the
    /// program; none for a clip that draws no picture, one in depth (which
    /// no flat map shows), and one that cannot be seen.
    pub fn placement(timeline: &Value, id: &str, position: f64) -> Option<Placement> {
        let (clip, track) = clip_in(timeline, id)?;
        let props = &clip["props"];
        if props["three_d"].as_bool() == Some(true) {
            return None;
        }
        let canvas = canvas_of(timeline);
        let (picture, fit) = picture_of(timeline, clip, track, canvas)?;
        let start = clip["timelineStart"].as_f64().unwrap_or(0.0);
        let duration = clip["duration"].as_f64().unwrap_or(0.0).max(0.0);
        let clip_time = (position - start).clamp(0.0, duration);
        let parents = parents_motion(
            timeline,
            clip,
            canvas,
            position.clamp(start, start + duration),
        );
        let source = Affine::scale(picture.0.max(1.0), picture.1.max(1.0));
        let geometry = moved_geometry_at(props, canvas, clip_time);
        let on_canvas = source
            .then(placed(&geometry, picture, fit, canvas))
            .then(parents);
        on_canvas.inverse()?;
        // The path says where the anchor goes.
        let g = geometry_at(props, clip_time);
        let [l, r, t, b] = g.crop;
        let width = picture.0.max(1.0) * (1.0 - l - r).max(1e-6);
        let height = picture.1.max(1.0) * (1.0 - t - b).max(1e-6);
        let (fw, fh) = filled((width, height), fit, canvas);
        let (cw, ch) = (canvas.0.max(1.0), canvas.1.max(1.0));
        let origin = (
            0.5 + g.at.0 + g.anchor.0 * fw / cw,
            0.5 + g.at.1 + g.anchor.1 * fh / ch,
        );
        let path = Affine::translate(origin.0, origin.1)
            .then(Affine::scale(cw, ch))
            .then(parents)
            .then(Affine::scale(1.0 / cw, 1.0 / ch));
        Some(Placement {
            picture: on_canvas,
            path,
            canvas,
            clip_time,
        })
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
    pub fn drawn(ellipse: bool, from: Pt, to: Pt, map: Affine) -> Option<Outline> {
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
