//! Editing what the picture shows with the pointer and the keys: a press
//! chooses a mask or a path point (else plays or pauses), a drag reshapes
//! it and goes to kerfd as one edit when its button comes up, drawn where
//! it was dropped until kerfd answers; a double-click on the path's curve
//! adds a point; Delete takes what is chosen; Start Motion Path gives a
//! clip its first path.

use super::overlay::{self, Pt};
use super::picture::{apart, to_pt};
use super::shown::{window_to_share, Drag, Shown};
use super::tools::MaskTool;
use super::{ensure_keys, RemoveChosen, StageView};
use editor_session::session::Session;
use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Entity, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, WeakEntity, Window,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Points a press moves before it drags.
pub(super) const DRAG_START: f64 = 3.0;
/// How long an outline or a path let go waits where it was dropped for kerfd's answer.
const HOLD: Duration = Duration::from_millis(1500);

impl StageView {
    pub(super) fn set_cursor(&mut self, cursor: Option<CursorStyle>, cx: &mut Context<Self>) {
        if self.cursor != cursor {
            self.cursor = cursor;
            cx.notify();
        }
    }

    /// The cursor while a drag holds the pointer.
    pub(super) fn drag_cursor(&self) -> Option<CursorStyle> {
        match &self.drag {
            Some(Drag::Mask { handle: None, .. }) => Some(CursorStyle::ClosedHand),
            Some(Drag::Draw { .. }) => Some(CursorStyle::Crosshair),
            Some(Drag::Mask { .. } | Drag::Point { .. }) => Some(CursorStyle::PointingHand),
            Some(Drag::Added { .. }) | None => None,
        }
    }

    /// The mask chosen on the chosen clip, as the picture and the inspector
    /// share it.
    pub(super) fn chosen_mask(&self, cx: &App) -> Option<String> {
        self.session.read(cx).chosen_mask.clone()
    }

    /// Chooses the mask `name` (or none) on the picture and in the inspector.
    pub(super) fn choose_mask(&mut self, name: Option<String>, cx: &mut Context<Self>) {
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
    pub(super) fn follow_selection(&mut self, session: &Entity<Session>, cx: &mut Context<Self>) {
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

    /// A press on the stage: the overlays' if they take it, else on the
    /// picture it plays or pauses; on the bars around it, nothing.
    pub(super) fn press(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let drawn_on = self.shown().filter(|shown| shown.placement.masks_seen());
        if let (Some(tool), Some(shown)) = (self.tool, drawn_on) {
            // A mask tool out has the picture's presses.
            window.focus(&self.focus_handle);
            self.press_tool(tool, &shown, canvas, to_pt(event.position), cx);
            return;
        }
        if let Some(shown) = self.shown() {
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
        // pausing): the picture takes the keys only for what it edits, given
        // above when a tool or what is chosen on it needs them.
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
    pub(super) fn hover(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag.is_some() || event.pressed_button.is_some() {
            return;
        }
        let (Some(picture), Some(canvas)) = (self.picture.get(), self.canvas.get()) else {
            return;
        };
        let at = to_pt(event.position);
        if self.tool.is_some()
            && self
                .shown()
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
        if let Some(shown) = self.shown().filter(|shown| shown.edits()) {
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

    /// A drag follows the pointer anywhere in the window.
    fn drag_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.release(cx);
            return;
        }
        let (Some(picture), Some(shown)) = (self.canvas.get(), self.shown()) else {
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
        if self.drag.is_none() {
            return;
        }
        self.let_go(cx);
    }

    /// What the drag in hand did, sent to kerfd as one edit: a rectangle or
    /// an ellipse dragged out becomes a mask; a point or a mask moved, its
    /// new place.
    fn let_go(&mut self, cx: &mut Context<Self>) {
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
            let (Some(picture), Some(shown)) = (self.canvas.get(), self.shown()) else {
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

    /// Sends `op` to kerfd, drawing what `drag` let go where it was dropped
    /// until kerfd's snapshot says where it is, or it has waited `HOLD`.
    pub(super) fn send_held(&mut self, drag: Drag, op: Value, cx: &mut Context<Self>) {
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

    /// A drop kerfd has not answered goes back by itself once held long
    /// enough (`HOLD`), as the stage draws: drawn again then.
    pub(super) fn follow_held(&mut self, cx: &Context<Self>) {
        if self
            .held
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() > HOLD)
        {
            self.held = None;
        }
        if let Some((_, _, at)) = &self.held {
            self.hold_ends.at(*at + HOLD, cx);
        }
    }

    /// Whether Delete has something here to take: a point of the path
    /// shown (`shown`), or a mask the clip has now. A mask chosen outlives
    /// a snapshot that lacks it (one read before a new mask was added
    /// does), but the key is the window's while it does.
    pub(super) fn takes_delete(&self, shown: Option<&Shown>, cx: &App) -> bool {
        let Some(shown) = shown else {
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
    pub(super) fn follow_chosen_point(&mut self, before: &Value, cx: &mut Context<Self>) {
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
    pub(super) fn remove_chosen(
        &mut self,
        _: &RemoveChosen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.shown();
        if !self.takes_delete(shown.as_deref(), cx) {
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
        let Some(shown) = shown else {
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
}

/// While a drag holds the pointer, it follows it anywhere in the window and
/// ends wherever its button comes up.
pub(super) fn listen(this: &WeakEntity<StageView>, window: &mut Window) {
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
