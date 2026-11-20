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
//! which a drag reshapes. A click on the picture plays or pauses; moving the
//! pointer across it skims the program, the timeline showing a skimmer, and
//! leaving it brings the playhead's frame back. Nothing here moves by
//! itself: frames are asked for only while the light runs.

use crate::icons::Icon;
use crate::model;
use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    canvas, div, fill, point, prelude::*, px, quad, size, App, BorderStyle, Bounds, ContentMask,
    Context, Corners, CursorStyle, DispatchPhase, Entity, EntityId, FocusHandle, FontWeight,
    Global, KeyBinding, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Pixels, Point, Rgba, SharedString, Subscription, TextRun, WeakEntity, Window,
};
use overlay::{Affine, MotionPath, Outline, Pt};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Action safe: a line this share of the picture in from each edge.
const ACTION_SAFE: f32 = 0.035;
/// Title safe, shown by ticks at its corners: this share in.
const TITLE_SAFE: f32 = 0.05;
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

gpui::actions!(stage, [RemoveChosen]);

/// The picture's key context, there while a mask or a path point is chosen:
/// Delete and Backspace take it ahead of the window's own keys.
const CONTEXT: &str = "Stage";

/// Binds Delete and Backspace to the chosen mask or point. Binding a keymap
/// clears this, so it is bound again whenever it is missing.
fn bind_keys(cx: &mut App) {
    let bound = Keystroke::parse("delete").is_ok_and(|delete| {
        cx.all_bindings_for_input(&[delete])
            .iter()
            .any(|binding| binding.action().partial_eq(&RemoveChosen))
    });
    if !bound {
        cx.bind_keys([
            KeyBinding::new("delete", RemoveChosen, Some(CONTEXT)),
            KeyBinding::new("backspace", RemoveChosen, Some(CONTEXT)),
        ]);
    }
}

/// Over the picture, whatever the theme: white over a dark rule reads on any
/// picture, in the accent's own hue too.
fn white(alpha: f32) -> Rgba {
    theme::with_alpha(gpui::rgb(0xffffff), alpha)
}

fn black(alpha: f32) -> Rgba {
    theme::with_alpha(gpui::rgb(0x000000), alpha)
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
}

// --------------------------------------------------------------------- view

pub struct StageView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    area: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The picture as last placed, in the window: what a press and the pointer hit.
    picture: Rc<Cell<Option<Bounds<Pixels>>>>,
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
    /// The clip whose mask or path point is chosen, and which.
    chosen_clip: Option<String>,
    chosen_mask: Option<String>,
    chosen_point: Option<usize>,
    drag: Option<Drag>,
    /// What a drag let go, drawn where it was dropped until kerfd's
    /// snapshot (the revision it was sent at) says where it is.
    held: Option<(Drag, u64, Instant)>,
    cursor: Option<CursorStyle>,
    _subscriptions: Vec<Subscription>,
}

impl StageView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = session.entity_id();
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                match event.0["type"].as_str() {
                    Some("state") => {
                        let before = std::mem::replace(
                            &mut this.program,
                            Rc::new(event.0["snapshot"]["timeline"].clone()),
                        );
                        this.follow_chosen_point(&before, cx);
                    }
                    Some("clock") => {
                        if let Some(at) = event.0["position"].as_f64() {
                            this.hear_clock(at, cx);
                        }
                    }
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
            cx.on_release(move |_, cx| forget_skim(id, cx)),
        ];
        Self {
            session,
            focus_handle: cx.focus_handle(),
            area: Rc::new(Cell::new(None)),
            picture: Rc::new(Cell::new(None)),
            hue: None,
            last_hue: Hue::Kerf,
            edge: Tween::at(0.0),
            edge_since: Instant::now(),
            program: Rc::new(Value::Null),
            shown: None,
            skim: None,
            kept: None,
            chosen_clip: None,
            chosen_mask: None,
            chosen_point: None,
            drag: None,
            held: None,
            cursor: None,
            _subscriptions: subscriptions,
        }
    }

    /// What runs, and the light that says so.
    fn activity(&self, cx: &App) -> (Option<Hue>, Option<String>) {
        let state = &self.session.read(cx).state;
        if let Some(rendering) = state.rendering() {
            return (Some(Hue::Computer), Some(rendering));
        }
        if let Some(making) = state.making_label() {
            return (Some(Hue::Computer), Some(making));
        }
        if state.work.busy() {
            let words = if state.agent_clips.is_empty() {
                "Kerf is looking"
            } else {
                "Kerf is cutting"
            };
            return (Some(Hue::Kerf), Some(words.to_owned()));
        }
        (None, None)
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

    /// A press on the stage: the overlays' if they take it, else on the
    /// picture it plays or pauses; on the bars around it, nothing.
    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picture) = self.picture.get() else {
            return;
        };
        if let Some(shown) = self.shown.clone() {
            if self.press_overlays(
                &shown,
                picture,
                to_pt(event.position),
                event.click_count,
                cx,
            ) {
                // What is chosen takes Delete.
                if self.chosen_mask.is_some() || self.chosen_point.is_some() {
                    bind_keys(cx);
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
            self.chosen_mask = None;
            self.chosen_point = None;
        }
        if let Some(points) = &shown.path {
            if let Some(index) = shown.point_at(picture, at) {
                // One thing chosen at a time.
                self.chosen_mask = None;
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
            let handle = shown.handle_at(picture, self.chosen_mask.as_deref(), at);
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
        let handle = shown.handle_at(picture, self.chosen_mask.as_deref(), at);
        let name = match handle {
            Some(_) => self.chosen_mask.clone(),
            None => shown.mask_at(picture, self.chosen_mask.as_deref(), at),
        };
        self.chosen_mask = name.clone();
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
        let Some(picture) = self.picture.get() else {
            return;
        };
        let at = to_pt(event.position);
        if let Some(shown) = self.shown.clone().filter(|shown| shown.edits()) {
            // The picture holds still while its masks or its path can be dragged.
            let chosen = self.chosen_mask.as_deref();
            let cursor = if shown.point_at(picture, at).is_some()
                || shown.handle_at(picture, chosen, at).is_some()
            {
                Some(CursorStyle::PointingHand)
            } else if shown.curve_at(picture, at).is_some() {
                Some(CursorStyle::Crosshair)
            } else if shown.mask_at(picture, chosen, at).is_some() {
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

    /// The pointer left the picture: the playhead's frame again.
    fn end_skim(&mut self, cx: &mut Context<Self>) {
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
        let (Some(picture), Some(shown)) = (self.picture.get(), self.shown.clone()) else {
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
            None => return,
        }
        cx.notify();
    }

    /// The button came up: what the drag did goes to kerfd as one edit.
    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        cx.notify();
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
    fn takes_delete(&self) -> bool {
        let Some(shown) = &self.shown else {
            return false;
        };
        let point = self
            .chosen_point
            .is_some_and(|index| shown.path.as_ref().is_some_and(|path| index < path.len()));
        let mask = self.chosen_mask.as_ref().is_some_and(|name| {
            overlay::clip_in(&self.program, &shown.clip).is_some_and(|(clip, _)| {
                overlay::masks_of(clip)
                    .iter()
                    .any(|mask| mask.name == *name)
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
        if !self.takes_delete() {
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
        if let Some(name) = self.chosen_mask.take() {
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

    fn transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let position = model::timecode(session.state.position, fps);
        let length = model::timecode(session.state.program_length(), fps);
        let playing = session.state.playing;
        let (mark_in, mark_out, looping) = session.marks();
        let marked = mark_in.is_some() || mark_out.is_some();
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
            "running" => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::chip(
                        format!("Rendering {:.0}%", preview.progress.unwrap_or(0.0) * 100.0),
                        theme::activity::gpu(),
                    ))
                    .child(ui::button("stop-preview", "Stop").small().ghost().on_click(
                        cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(json!({"type": "preview_stop"}));
                            })
                        }),
                    ))
                    .into_any_element(),
            ),
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
            .child(
                icon_button("previous-frame", Icon::StepBack)
                    .tooltip("Previous frame")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::RunCommand::new("playback.previousFrame")),
                            cx,
                        );
                    })),
            )
            .child(
                icon_button("play", if playing { Icon::Pause } else { Icon::Play })
                    .tooltip(if playing { "Pause" } else { "Play" })
                    .color(theme::ink())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session
                            .update(cx, |session, _| session.toggle_playback())
                    })),
            )
            .child(
                icon_button("next-frame", Icon::StepForward)
                    .tooltip("Next frame")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::RunCommand::new("playback.nextFrame")),
                            cx,
                        );
                    })),
            )
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
                    toggle_words(
                        "loop-playback",
                        if looping && marked {
                            "Loop in to out"
                        } else {
                            "Loop"
                        },
                        looping,
                    )
                    .tooltip(|_, cx| cx.new(|_| ui::Tooltip("Loop playback".into())).into())
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::RunCommand::new("playback.loop")),
                            cx,
                        );
                    })),
                ),
            )
            .child(div().flex_1())
            .children(preview_control)
    }
}

/// A small button of words, held down while what it switches is on.
fn toggle_words(id: &'static str, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    theme::type_style(div(), text::CONTROL)
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.0))
        .px(px(7.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .whitespace_nowrap()
        .text_color(if on { theme::ink() } else { theme::ink3() })
        .when(on, |button| button.bg(theme::on()))
        .when(!on, |button| {
            button.hover(|style| style.bg(theme::raised()).text_color(theme::ink()))
        })
        .child(words)
}

impl Render for StageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (hue, chip) = self.activity(cx);
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
        // The format of what the picture shows: a first cut proposed on an
        // empty program brings its own.
        let proposed = session
            .state
            .snapshot
            .proposal_timeline
            .as_ref()
            .filter(|proposal| !timeline.has_clips() && proposal.width > 0 && proposal.height > 0);
        let aspect = match (
            proposed,
            &picture,
            timeline.width > 0 && timeline.height > 0,
        ) {
            (Some(proposal), _, _) => proposal.aspect(),
            (None, _, true) => timeline.aspect(),
            (None, Some((_, width, height)), false) if *height > 0 => {
                *width as f32 / *height as f32
            }
            _ => 16.0 / 9.0,
        };
        let safe_zones = session.prefs.safe_zones;
        let caption = session.state.caption.clone();
        let stand_in: Option<SharedString> = if picture.is_some() {
            None
        } else if !session.state.has_snapshot {
            Some("Connecting to kerfd…".into())
        } else if !timeline.has_clips() {
            Some("No picture yet".into())
        } else {
            Some(session.project_name().into())
        };
        // What was chosen on another clip is let go.
        if shown.as_ref().map(|shown| &shown.clip) != self.chosen_clip.as_ref() {
            self.chosen_clip = shown.as_ref().map(|shown| shown.clip.clone());
            self.chosen_mask = None;
            self.chosen_point = None;
        }
        if shown.is_none() {
            self.drag = None;
        }
        self.shown = shown.clone();
        if self.shown.as_ref().is_none_or(|shown| !shown.edits()) {
            self.cursor = None;
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
        let chosen_mask = self.chosen_mask.clone();
        let chosen_point = self.chosen_point;
        let dragging = match &self.drag {
            Some(Drag::Mask { handle: None, .. }) => Some(CursorStyle::ClosedHand),
            Some(_) => Some(CursorStyle::PointingHand),
            None => None,
        };
        let this = cx.entity().downgrade();

        let stage = canvas(
            move |bounds, window, _| {
                if area_cell.get() != Some(bounds) {
                    area_cell.set(Some(bounds));
                    window.refresh();
                }
                let placement = place(bounds, aspect);
                picture_cell.set(Some(placement.picture));
                placement
            },
            move |bounds, placement: Placement, window, cx| {
                // The device: a bezel, a hairline round it.
                window.paint_quad(quad(
                    placement.device,
                    Corners::all(placement.device_radius),
                    gpui::rgb(0x0a0a0a),
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
                if let Some((image, width, height)) = frame {
                    // Whole, at its own shape.
                    let _ = window.paint_image(
                        contain(placement.picture, width, height),
                        Corners::all(placement.picture_radius),
                        image,
                        0,
                        false,
                    );
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
                        gpui::rgb(0x000000),
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
                        |window| paint_safe_zones(window, placement.picture),
                    );
                }
                if let Some(shown) = &shown {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        paint_overlays(
                            window,
                            cx,
                            placement.picture,
                            shown,
                            chosen_mask.as_deref(),
                            chosen_point,
                        )
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

        let mut overlays = div().absolute().inset_0();
        if let Some(placed) = placed {
            let picture = placed.picture;
            let left = picture.origin.x;
            let top = picture.origin.y;
            if let Some(chip) = chip.clone() {
                let color = self.last_hue.color();
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
                                .child(ui::dot(color, 6.0))
                                .child(chip),
                        )
                        .opacity(edge.max(0.0)),
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
            if !caption.trim().is_empty() {
                overlays = overlays.child(
                    div()
                        .absolute()
                        .left(left)
                        .w(picture.size.width)
                        .top(top + picture.size.height * 0.82)
                        .flex()
                        .justify_center()
                        .child(
                            theme::type_style(div(), text::CAPTION)
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(theme::radius::SMALL))
                                .bg(theme::with_alpha(gpui::rgb(0x000000), 0.72))
                                .text_color(gpui::rgb(0xffffff))
                                .child(caption),
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
                    .when(self.takes_delete(), |area| area.key_context(CONTEXT))
                    .on_action(cx.listener(Self::remove_chosen))
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
    let tick = white(0.45);
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
            white(1.0),
            px(1.0),
            theme::accent(),
            BorderStyle::Solid,
        ));
    }
}

fn top_left(line: &[Point<Pixels>]) -> Option<Point<Pixels>> {
    let first = *line.first()?;
    Some(line.iter().fold(first, |least, at| {
        point(least.x.min(at.x), least.y.min(at.y))
    }))
}

/// An outline over the picture: a dark rule, then a white line on it, which
/// reads on any picture; dashed for a path only.
fn stroke_over(
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
        window.paint_path(path, black(0.55));
    }
    if let Some(path) = build(1.5, dashed) {
        window.paint_path(path, white(if chosen { 1.0 } else { 0.7 }));
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
        color: white(1.0).into(),
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
        if chosen { theme::accent() } else { black(0.6) },
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
        (white(1.0), theme::accent())
    } else {
        (theme::accent(), white(1.0))
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
    window.paint_quad(quad(
        ring,
        Corners::all(placement.picture_radius + out),
        theme::with_alpha(color, 0.0),
        px(scene::EDGE_WIDTH),
        theme::with_alpha(color, scene::EDGE_REST * opacity),
        BorderStyle::Solid,
    ));
    if motion::reduced() {
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
