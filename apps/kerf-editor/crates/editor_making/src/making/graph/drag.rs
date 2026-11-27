//! What a press, a drag and the wheel do on the canvas, and the arrows
//! between its cards: the canvas panned (by the middle button from over a
//! card too), a card moved and its place kept, an edge drawn from a port
//! and wired into the card it is let go over, and the canvas zoomed.

use super::layout::{card_rect, nearest_toward, save_places, Toward};
use super::{MenuKind, ShotGraph};
use crate::making::{port_says, ports_for, value_of, wire_nth};
use editor_ui::ui;
use gpui::{
    Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, Window,
};
use kerf_bridge::RecipeEdit;

/// A drag under way on the canvas.
pub(super) enum Drag {
    None,
    /// The canvas, panned from where it was; `middle`: by the middle
    /// button, from anywhere, cards too.
    Canvas {
        from: Point<Pixels>,
        offset: (f32, f32),
        moved: bool,
        middle: bool,
    },
    /// A card, moved from where it was.
    Card {
        id: String,
        from: Point<Pixels>,
        place: (f32, f32),
        moved: bool,
    },
    /// An edge drawn from a card's port, to the pointer; over a card that
    /// could read it.
    Wire {
        from: String,
        at: (f32, f32),
        over: Option<String>,
    },
    /// A signal's curve, drawn by hand.
    Curve,
}

/// A press that moves less than this is a click.
const DRAG_FROM: f32 = 3.0;

impl ShotGraph {
    pub(super) fn press_canvas(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        // A double-click on the canvas takes in the whole graph.
        if event.click_count >= 2 {
            self.drag = Drag::None;
            self.fit(cx);
            return;
        }
        self.drag = Drag::Canvas {
            from: event.position,
            offset: self.camera.offset,
            moved: false,
            middle: false,
        };
        cx.notify();
    }

    /// The middle button pressed anywhere on the canvas, over a card too:
    /// a drag pans it; let go, nothing is chosen.
    pub(super) fn press_middle(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Middle || !matches!(self.drag, Drag::None) {
            return;
        }
        window.focus(&self.canvas_focus);
        self.drag = Drag::Canvas {
            from: event.position,
            offset: self.camera.offset,
            moved: false,
            middle: true,
        };
        cx.notify();
    }

    /// The middle button let go: its pan ends.
    pub(super) fn middle_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.drag, Drag::Canvas { middle: true, .. }) {
            self.drag = Drag::None;
            cx.notify();
        }
    }

    pub(super) fn press_card(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        let place = self.places().get(id).copied().unwrap_or((0.0, 0.0));
        self.drag = Drag::Card {
            id: id.to_owned(),
            from: event.position,
            place,
            moved: false,
        };
        self.select(Some(id.to_owned()), window, cx);
    }

    pub(super) fn press_port(
        &mut self,
        id: &str,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        self.drag = Drag::Wire {
            from: id.to_owned(),
            at: self.local(event.position),
            over: None,
        };
        cx.notify();
    }

    /// The card under a point of the canvas, the topmost.
    fn card_at(&self, at: (f32, f32)) -> Option<String> {
        let graph = self.shown()?;
        let places = self.places();
        graph.nodes.iter().rev().find_map(|node| {
            let [x, y, width, height] = card_rect(graph, &places, &node.id)?;
            let (left, top) = self.camera.to_view((x, y));
            let zoom = self.camera.zoom;
            let inside = at.0 >= left
                && at.0 <= left + width * zoom
                && at.1 >= top
                && at.1 <= top + height * zoom;
            inside.then(|| node.id.clone())
        })
    }

    pub(super) fn on_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.drag, Drag::None) {
            return;
        }
        let held = if matches!(self.drag, Drag::Canvas { middle: true, .. }) {
            MouseButton::Middle
        } else {
            MouseButton::Left
        };
        if event.pressed_button != Some(held) {
            self.drag = Drag::None;
            if let Some(drawing) = self.drawing.as_mut() {
                drawing.held = false;
            }
            cx.notify();
            return;
        }
        let local = self.local(event.position);
        let zoom = self.camera.zoom.max(0.01);
        let over_card = match &self.drag {
            Drag::Wire { from, .. } => {
                let from = from.clone();
                self.card_at(local).filter(|over| {
                    let graph = self.shown();
                    let (Some(from), Some(to)) = (
                        graph.and_then(|graph| graph.node(&from)),
                        graph.and_then(|graph| graph.node(over)),
                    ) else {
                        return false;
                    };
                    !ports_for(from, to).is_empty()
                })
            }
            _ => None,
        };
        match &mut self.drag {
            Drag::None => {}
            Drag::Canvas {
                from,
                offset,
                moved,
                ..
            } => {
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                if !*moved && dx.hypot(dy) < DRAG_FROM {
                    return;
                }
                *moved = true;
                self.camera.offset = (offset.0 + dx, offset.1 + dy);
                self.fitted = false;
            }
            Drag::Card {
                id,
                from,
                place,
                moved,
            } => {
                let (dx, dy) = (
                    f32::from(event.position.x - from.x),
                    f32::from(event.position.y - from.y),
                );
                if !*moved && dx.hypot(dy) < DRAG_FROM {
                    return;
                }
                *moved = true;
                // A card goes where it is dragged, on whole points.
                self.placed.insert(
                    id.clone(),
                    ((place.0 + dx / zoom).round(), (place.1 + dy / zoom).round()),
                );
            }
            Drag::Wire { at, over, .. } => {
                *at = local;
                *over = over_card;
            }
            Drag::Curve => self.draw_curve(event.position),
        }
        cx.notify();
    }

    pub(super) fn on_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => return,
            // A press on the canvas that did not pan it chooses nothing: the shot itself.
            Drag::Canvas {
                moved: false,
                middle: false,
                ..
            } => self.select(None, window, cx),
            Drag::Canvas { .. } => {}
            Drag::Card { moved: true, .. } => {
                save_places(&self.session, &self.places_key(), &self.placed, cx)
            }
            Drag::Card { .. } => {}
            Drag::Wire {
                from,
                over: Some(to),
                ..
            } => self.wire(&from, &to, event.position, window, cx),
            Drag::Wire { .. } => {}
            Drag::Curve => self.drawn(cx),
        }
        cx.notify();
    }

    /// An edge drawn from `from` let go over `to`: what it gives is written
    /// on `to`'s line under the name it reads it by, asked first where
    /// several would.
    fn wire(
        &mut self,
        from: &str,
        to: &str,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(graph) = self.shown() else {
            return;
        };
        let (Some(from), Some(to)) = (graph.node(from).cloned(), graph.node(to).cloned()) else {
            return;
        };
        let ports = ports_for(&from, &to);
        if ports.is_empty() {
            return;
        }
        let value = value_of(&from);
        if value.is_empty() {
            return;
        }
        if let [port] = ports.as_slice() {
            self.read_port(&to.id, port, &value, cx);
            return;
        }
        // More than one name would read it: which is the person's to say.
        let this = cx.weak_entity();
        let reads = to.model().and_then(|model| model.reads.clone());
        let items = ports
            .iter()
            .map(|port| {
                // What the model has said it has no use for is still the
                // line's to say, and is said so.
                let says = if reads.as_ref().is_some_and(|reads| !reads.contains(port)) {
                    "the model has no use for it".to_owned()
                } else {
                    port_says(port)
                };
                let (this, to, port, value) =
                    (this.clone(), to.id.clone(), port.clone(), value.clone());
                ui::MenuItem::new(port.clone())
                    .key(says)
                    .on_choose(move |_, cx| {
                        let _ =
                            this.update(cx, |graph, cx| graph.read_port(&to, &port, &value, cx));
                    })
            })
            .collect();
        let header = format!("{} reads {} as", to.title, from.title);
        self.open_menu(MenuKind::Port, items, Some(header), at, window, cx);
    }

    /// `to`'s line given `value` under `port`: replaced, or one more where
    /// the name is one a line may give again.
    fn read_port(&mut self, to: &str, port: &str, value: &str, cx: &mut Context<Self>) {
        let Some(recipe) = &self.recipe else {
            return;
        };
        let Some(line) = recipe.node(to).and_then(|node| node.line) else {
            return;
        };
        let nth = wire_nth(recipe, to, port);
        self.edit(
            RecipeEdit {
                line,
                key: port.to_owned(),
                nth,
                value: Some(value.to_owned()),
                ..RecipeEdit::default()
            },
            cx,
        );
    }

    pub(super) fn arrow(&mut self, toward: Toward, window: &mut Window, cx: &mut Context<Self>) {
        let Some(graph) = self.shown() else {
            return;
        };
        let places = self.places();
        let centres: Vec<(String, (f32, f32))> = graph
            .nodes
            .iter()
            .filter_map(|node| {
                let [x, y, width, height] = card_rect(graph, &places, &node.id)?;
                Some((node.id.clone(), (x + width / 2.0, y + height / 2.0)))
            })
            .collect();
        if let Some(next) = nearest_toward(&centres, self.selected.as_deref(), toward) {
            self.select(Some(next), window, cx);
        }
    }

    pub(super) fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let at = self.local(event.position);
        let zooms = event.modifiers.control || event.modifiers.platform;
        match event.delta {
            // Two fingers on a trackpad move the canvas.
            ScrollDelta::Pixels(delta) if !zooms => {
                self.camera.offset = (
                    self.camera.offset.0 + f32::from(delta.x),
                    self.camera.offset.1 + f32::from(delta.y),
                );
                self.fitted = false;
                cx.notify();
            }
            // A wheel, or either with Ctrl or ⌘, zooms about the pointer.
            ScrollDelta::Pixels(delta) => self.zoom_by(1.0015f32.powf(f32::from(delta.y)), at, cx),
            ScrollDelta::Lines(lines) => self.zoom_by(1.0015f32.powf(lines.y * 120.0), at, cx),
        }
    }
}
