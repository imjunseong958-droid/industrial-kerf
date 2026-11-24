//! The graph's canvas as it is drawn: its dots, the edges under the cards
//! and their ends over them, the cards where the camera puts them, the
//! ports an edge is drawn from, the names a step reads its edges by, what
//! it says before there is a graph, and the legend and the zoom at its
//! foot; and the canvas's own keys.

use super::cards::gives_color;
use super::drag::Drag;
use super::layout::{
    card_rect, in_port, out_port, Camera, Toward, LEGEND, PAD, ZOOM_READ, ZOOM_STEP,
};
use super::paint::paint_ring;
use super::ShotGraph;
use crate::making::{gives_word, reads_port, GraphEdge, Recipe};
use editor_state::store::Making;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds, Context,
    CursorStyle, ElementId, KeyBinding, MouseButton, MouseDownEvent, PathBuilder, Pixels, Rgba,
    ScrollWheelEvent, Window,
};
use kerf_bridge::event::graph::{Gives, Kind};
use std::collections::HashMap;

actions!(
    shot_graph,
    [FitGraph, ZoomIn, ZoomOut, NodeLeft, NodeRight, NodeUp, NodeDown]
);

/// The key context the canvas answers in, while it has the keys.
const CANVAS: &str = "ShotGraph";

/// The canvas's keys: F or 0 fit the whole graph, + or = and - zoom about
/// its middle, the arrows go to the nearest card that way. Bound again
/// whenever a graph opens and finds them missing (a keymap bound clears them).
pub(crate) fn bind_graph_keys(cx: &mut App) {
    let bound = gpui::Keystroke::parse("f").is_ok_and(|key| {
        cx.all_bindings_for_input(&[key])
            .iter()
            .any(|binding| binding.action().partial_eq(&FitGraph))
    });
    if bound {
        return;
    }
    let at = Some(CANVAS);
    cx.bind_keys([
        KeyBinding::new("f", FitGraph, at),
        KeyBinding::new("0", FitGraph, at),
        KeyBinding::new("=", ZoomIn, at),
        KeyBinding::new("shift-=", ZoomIn, at),
        KeyBinding::new("+", ZoomIn, at),
        KeyBinding::new("-", ZoomOut, at),
        KeyBinding::new("left", NodeLeft, at),
        KeyBinding::new("right", NodeRight, at),
        KeyBinding::new("up", NodeUp, at),
        KeyBinding::new("down", NodeDown, at),
    ]);
}

/// The canvas's dots, drawn while they are this far apart at least.
const GRID: f32 = 24.0;
const GRID_LEAST: f32 = 10.0;
/// How near the pointer has to be to a port to be on it, on the screen.
const PORT_REACH: f32 = 9.0;

/// An edge as it is drawn on the canvas.
struct EdgeLine {
    start: (f32, f32),
    end: (f32, f32),
    color: Rgba,
    dashed: bool,
}

/// Where an edge leaves and where it arrives, in the view, and its color:
/// the rings drawn there over the cards' edges.
type EdgeEnds = ((f32, f32), (f32, f32), Rgba);

/// The port lit, in the view: in what it gives, and whether an edge is
/// being drawn from it.
type LitPort = ((f32, f32), Rgba, bool);

/// The edge being drawn, in the view: from its port to the pointer, in
/// what it would carry, and whether it is only held (over nothing that
/// could read it).
type DrawnWire = ((f32, f32), (f32, f32), Rgba, bool);

impl ShotGraph {
    /// The canvas: its dots, the edges and the cards, panned and zoomed as
    /// a whole, the legend and the zoom's buttons at its foot.
    pub(super) fn canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let room = self.canvas_room(cx);
        let bounds = self.canvas_bounds.clone();
        let keep_bounds = canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
            .absolute()
            .inset_0();
        let Some(graph) = self.shown().cloned() else {
            return self.waiting(room.child(keep_bounds), cx).into_any_element();
        };
        if graph.nodes.is_empty() {
            let why = graph
                .error
                .clone()
                .unwrap_or_else(|| "There are no steps to show.".into());
            let said = if graph.document {
                format!("These steps do not read: {why}")
            } else {
                why
            };
            return room
                .child(keep_bounds)
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(ui::label(said, text::BODY, theme::neg()).max_w(px(480.0)))
                .into_any_element();
        }
        let places = self.places();
        let camera = self.camera;
        let zoom = camera.zoom;
        let selected = self.selected.as_deref();
        // Edges under the cards; where each leaves and arrives, over them;
        // the names they are read by.
        let edges = edge_lines(&graph, &places, camera, selected);
        let ends = edge_ends(&graph, &places, camera);
        let labels = port_labels(&graph, &places, camera, selected);
        let grid = GRID * zoom;
        let dots = (
            camera.offset.0.rem_euclid(grid),
            camera.offset.1.rem_euclid(grid),
        );
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        if making.is_some() && !motion::reduced() && self.workflow.is_none() {
            window.request_animation_frame();
        }
        let cards = self.placed_cards(&graph, &places, making.as_ref(), cx);
        let ports = self.ports(&graph, &places, cx);
        let lit = self.lit_port(&graph, &places);
        let wire = self.drawn_wire(&graph, &places);
        let card_fill = theme::card();
        room.child(
            canvas(
                {
                    let bounds = self.canvas_bounds.clone();
                    move |area, _, _| bounds.set(area)
                },
                move |bounds, _, window, _| {
                    paint_grid_and_edges(window, bounds, grid, dots, &edges, zoom)
                },
            )
            .absolute()
            .inset_0(),
        )
        .children(cards)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    paint_over_cards(window, bounds, &ends, lit, wire, zoom, card_fill)
                },
            )
            .absolute()
            .inset_0(),
        )
        .children(labels)
        .children(ports)
        .child(self.legend(&graph, cx))
        .into_any_element()
    }

    /// The canvas's own box: it has the keys for fitting, zooming and the
    /// arrows, a press on it pans or chooses the shot, a middle press pans
    /// from anywhere on it, and the wheel moves or zooms it.
    fn canvas_room(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("graph-canvas")
            .key_context(CANVAS)
            .track_focus(&self.canvas_focus)
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(theme::well())
            .on_action(cx.listener(|this, _: &FitGraph, _, cx| this.fit(cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                let middle = this.middle();
                this.zoom_by(ZOOM_STEP, middle, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                let middle = this.middle();
                this.zoom_by(1.0 / ZOOM_STEP, middle, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NodeLeft, window, cx| this.arrow(Toward::Left, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NodeRight, window, cx| {
                    this.arrow(Toward::Right, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &NodeUp, window, cx| this.arrow(Toward::Up, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NodeDown, window, cx| this.arrow(Toward::Down, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.press_canvas(event, window, cx)
                }),
            )
            // Ahead of the cards: a middle drag pans from over one too.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.press_middle(event, window, cx)
            }))
            .on_scroll_wheel(
                cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.wheel(event, cx)),
            )
    }

    /// Each card where the camera puts it; the one an edge being drawn is
    /// over is outlined in the accent's ink.
    fn placed_cards(
        &self,
        graph: &Recipe,
        places: &HashMap<String, (f32, f32)>,
        making: Option<&Making>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let camera = self.camera;
        let zoom = camera.zoom;
        let wire_over = match &self.drag {
            Drag::Wire { over, .. } => over.clone(),
            _ => None,
        };
        graph
            .nodes
            .iter()
            .filter_map(|node| {
                let [x, y, width, height] = card_rect(graph, places, &node.id)?;
                let at = camera.to_view((x, y));
                let size = (width * zoom, height * zoom);
                let over = wire_over.as_deref() == Some(node.id.as_str());
                Some(if node.is_part() {
                    self.part_card(node, graph, at, size, zoom, cx)
                } else {
                    self.card(node, at, size, making, over, cx)
                })
            })
            .collect()
    }

    /// The ports, a little larger under the pointer: an edge is drawn from one.
    fn ports(
        &self,
        graph: &Recipe,
        places: &HashMap<String, (f32, f32)>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let camera = self.camera;
        graph
            .nodes
            .iter()
            .filter(|node| !node.is(Kind::Takes))
            .filter_map(|node| {
                let at = camera.to_view(out_port(graph, places, &node.id)?);
                let id = node.id.clone();
                let hover_id = node.id.clone();
                Some(
                    div()
                        .id(ElementId::Name(format!("port-{}", node.id).into()))
                        .absolute()
                        .left(px(at.0 - PORT_REACH))
                        .top(px(at.1 - PORT_REACH))
                        .size(px(2.0 * PORT_REACH))
                        .rounded_full()
                        .cursor(CursorStyle::Crosshair)
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            let now = hovered.then(|| hover_id.clone());
                            if this.hovered_port != now {
                                this.hovered_port = now;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.press_port(&id, event, window, cx);
                            }),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }

    /// The port lit: the one an edge is being drawn from, else the one
    /// under the pointer.
    fn lit_port(&self, graph: &Recipe, places: &HashMap<String, (f32, f32)>) -> Option<LitPort> {
        let lit_port = match &self.drag {
            Drag::Wire { from, .. } => Some((from.clone(), true)),
            _ => self.hovered_port.clone().map(|id| (id, false)),
        };
        lit_port.and_then(|(id, held)| {
            let node = graph.node(&id)?;
            Some((
                self.camera.to_view(out_port(graph, places, &id)?),
                gives_color(node.gives),
                held,
            ))
        })
    }

    /// The edge being drawn, from its port to the pointer, in what it would carry.
    fn drawn_wire(
        &self,
        graph: &Recipe,
        places: &HashMap<String, (f32, f32)>,
    ) -> Option<DrawnWire> {
        match &self.drag {
            Drag::Wire { from, at, over } => graph.node(from).and_then(|node| {
                Some((
                    self.camera.to_view(out_port(graph, places, from)?),
                    *at,
                    gives_color(node.gives),
                    over.is_none(),
                ))
            }),
            _ => None,
        }
    }

    /// The canvas before there is a graph: what it waits on, or why there is none.
    fn waiting(
        &self,
        room: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let reading = if self.workflow_line > 0 {
            "Reading the model's workflow…".to_owned()
        } else {
            format!("Reading how {} is made…", self.media)
        };
        let said = match &self.refused {
            Some(why) => ui::label(why.clone(), text::BODY, theme::neg()),
            None => ui::label(reading, text::BODY, theme::ink3()),
        };
        let mut column = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(theme::space::S12))
            .max_w(px(480.0))
            // A press on what it says, or its button, is not one on the canvas.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(said);
        if self.refused.is_some() && self.uncarried {
            column = column.child(
                ui::button("graph-carry", "Carry Steps into the Document…")
                    .on_click(cx.listener(|this, _, window, cx| this.open_plan(1, window, cx))),
            );
        }
        room.flex()
            .items_center()
            .justify_center()
            .p(px(24.0))
            .child(column)
    }

    /// The legend of what the edges carry, or of what a workflow's rows
    /// are, and beside it the zoom's buttons. They sit on the canvas, but a
    /// press on them is theirs: it neither pans the canvas, nor chooses the
    /// shot, nor counts towards a double-click that fits the graph.
    fn legend(&self, graph: &Recipe, cx: &mut Context<Self>) -> AnyElement {
        let card = || {
            div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .px(px(10.0))
                .h(px(LEGEND))
                .rounded(px(radius::CARD))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::line2())
        };
        let sample = |color: Rgba| div().w(px(16.0)).h(px(1.5)).rounded_full().bg(color);
        let legend = if graph.parts {
            let fills = if self.recipe.as_ref().is_some_and(|recipe| !recipe.document) {
                "the recipe fills"
            } else {
                "the steps fill"
            };
            card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S6))
                        .child(sample(gives_color(Gives::File)))
                        .child(ui::label("link", text::META, theme::ink2())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S6))
                        .child(
                            div()
                                .px(px(5.0))
                                .rounded(px(radius::SMALL))
                                .bg(theme::accent_wash())
                                .child(ui::label("{blank}", text::MONO_SMALL, theme::accent_ink())),
                        )
                        .child(ui::label(fills, text::META, theme::ink2())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S6))
                        .child(ui::label("value", text::MONO_SMALL, theme::ink2()))
                        .child(ui::label("set here", text::META, theme::ink2())),
                )
        } else {
            // The four a shot is told by, always; words and other files where the graph has them.
            let mut shown = vec![Gives::Video, Gives::Image, Gives::Path, Gives::Signal];
            for more in [Gives::Words, Gives::File] {
                if graph.edges.iter().any(|edge| edge.gives == more) {
                    shown.push(more);
                }
            }
            card().children(shown.into_iter().map(|gives| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S6))
                    .child(sample(gives_color(gives)))
                    .child(ui::label(gives_word(gives), text::META, theme::ink2()))
            }))
        };
        let zoom = card()
            .gap(px(0.0))
            .px(px(theme::space::S4))
            .child(
                ui::icon_button("graph-zoom-out", Icon::ZoomOut)
                    .focusable(false)
                    .tooltip("Zoom out")
                    .key("-")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let middle = this.middle();
                        this.zoom_by(1.0 / ZOOM_STEP, middle, cx)
                    })),
            )
            .child(
                ui::icon_button("graph-zoom-in", Icon::ZoomIn)
                    .focusable(false)
                    .tooltip("Zoom in")
                    .key("+")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let middle = this.middle();
                        this.zoom_by(ZOOM_STEP, middle, cx)
                    })),
            )
            .child(
                ui::button("graph-fit", "Fit")
                    .ghost()
                    .small()
                    .focusable(false)
                    .tooltip("The whole graph in view")
                    .key("F")
                    .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
            );
        div()
            .absolute()
            .left(px(PAD))
            .bottom(px(PAD))
            .flex()
            .gap(px(theme::space::S8))
            .child(legend)
            .child(zoom)
            .into_any_element()
    }
}

/// Whether `edge` touches the node chosen: every edge does while none is.
fn touches(selected: Option<&str>, edge: &GraphEdge) -> bool {
    selected.is_none_or(|id| edge.from == id || edge.to == id)
}

/// The edges as they are drawn under the cards, in the view, in what each
/// carries. With a node chosen, what does not touch it falls back; what a
/// model has said it has no use for goes to it dashed.
fn edge_lines(
    graph: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    camera: Camera,
    selected: Option<&str>,
) -> Vec<EdgeLine> {
    graph
        .edges
        .iter()
        .filter_map(|edge| {
            let start = camera.to_view(out_port(graph, places, &edge.from)?);
            let end = camera.to_view(in_port(graph, places, edge)?);
            let color = gives_color(edge.gives);
            let dashed = graph
                .node(&edge.to)
                .is_some_and(|to| !reads_port(to, &edge.port));
            Some(EdgeLine {
                start,
                end,
                color: if touches(selected, edge) {
                    color
                } else {
                    theme::with_alpha(color, 0.35)
                },
                dashed,
            })
        })
        .collect()
}

/// Where each edge leaves and arrives, in the view, to be ringed over the
/// cards' edges.
fn edge_ends(
    graph: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    camera: Camera,
) -> Vec<EdgeEnds> {
    graph
        .edges
        .iter()
        .filter_map(|edge| {
            Some((
                camera.to_view(out_port(graph, places, &edge.from)?),
                camera.to_view(in_port(graph, places, edge)?),
                gives_color(edge.gives),
            ))
        })
        .collect()
}

/// By what name a step reads each edge, beside where it comes in; too
/// small to read, none is.
fn port_labels(
    graph: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    camera: Camera,
    selected: Option<&str>,
) -> Vec<AnyElement> {
    if camera.zoom < ZOOM_READ {
        return Vec::new();
    }
    graph
        .edges
        .iter()
        .filter(|edge| {
            graph
                .node(&edge.to)
                .is_some_and(|to| matches!(to.kind(), Some(Kind::Model | Kind::Join)))
        })
        .filter_map(|edge| {
            let end = camera.to_view(in_port(graph, places, edge)?);
            Some(port_label(&edge.port, end, touches(selected, edge)))
        })
        .collect()
}

/// Under the cards: the canvas's grid, a dot a step while they are far
/// enough apart to be dots, from `dots`, and the edges.
fn paint_grid_and_edges(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    grid: f32,
    dots: (f32, f32),
    edges: &[EdgeLine],
    zoom: f32,
) {
    let origin = bounds.origin;
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    // The canvas's grid: a dot a step, while they are far enough apart to be dots.
    if grid >= GRID_LEAST {
        let dot = theme::with_alpha(theme::ink3(), 0.22);
        let mut y = dots.1;
        while y < height {
            let mut x = dots.0;
            while x < width {
                window.paint_quad(fill(
                    Bounds::new(
                        origin + point(px(x - 0.75), px(y - 0.75)),
                        size(px(1.5), px(1.5)),
                    ),
                    dot,
                ));
                x += grid;
            }
            y += grid;
        }
    }
    for edge in edges {
        // It leaves and arrives level, and turns in the room between the columns.
        let reach = (48.0 * zoom).max((edge.end.0 - edge.start.0).abs() * 0.6);
        let at = |(x, y): (f32, f32)| origin + point(px(x), px(y));
        let mut path = PathBuilder::stroke(px(1.5));
        if edge.dashed {
            path = path.dash_array(&[px(3.0), px(3.0)]);
        }
        path.move_to(at(edge.start));
        path.cubic_bezier_to(
            at(edge.end),
            at((edge.start.0 + reach, edge.start.1)),
            at((edge.end.0 - reach, edge.end.1)),
        );
        if let Ok(path) = path.build() {
            window.paint_path(path, edge.color);
        }
    }
}

/// Over the cards: where each edge leaves and arrives, the port lit, and
/// the edge being drawn.
fn paint_over_cards(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    ends: &[EdgeEnds],
    lit: Option<LitPort>,
    wire: Option<DrawnWire>,
    zoom: f32,
    card_fill: Rgba,
) {
    let origin = bounds.origin;
    let at = |(x, y): (f32, f32)| origin + point(px(x), px(y));
    for (start, end, color) in ends {
        paint_ring(window, at(*start), 3.5 * zoom, card_fill, Some(*color));
        paint_ring(window, at(*end), 3.5 * zoom, *color, None);
    }
    if let Some((port, color, held)) = lit {
        let fill = if held { color } else { card_fill };
        paint_ring(window, at(port), 5.0 * zoom.max(0.8), fill, Some(color));
    }
    if let Some((start, end, color, held)) = wire {
        let reach = 32f32.max((end.0 - start.0).abs() * 0.5);
        let mut path = PathBuilder::stroke(px(1.5));
        // Until it is over something that could read it, it is only being held.
        if held {
            path = path.dash_array(&[px(3.0), px(3.0)]);
        }
        path.move_to(at(start));
        path.cubic_bezier_to(
            at(end),
            at((start.0 + reach, start.1)),
            at((end.0 - reach, end.1)),
        );
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// An edge's name beside where it comes into its card, on the canvas's
/// own colour, so an edge that passes behind it does not strike it through.
fn port_label(port: &str, end: (f32, f32), touches: bool) -> AnyElement {
    const MOST: f32 = 160.0;
    div()
        .absolute()
        .left(px(end.0 - 9.0 - MOST))
        .top(px(end.1 - 19.0))
        .w(px(MOST))
        .h(px(14.0))
        .flex()
        .justify_end()
        .child(
            div()
                .px(px(theme::space::S4))
                .h(px(14.0))
                .flex()
                .items_center()
                .rounded(px(theme::radius::SMALL))
                .bg(theme::with_alpha(theme::well(), 0.86))
                .child(
                    theme::type_style(div(), text::META)
                        .line_height(px(14.0))
                        .text_color(if touches { theme::ink3() } else { theme::dis() })
                        .whitespace_nowrap()
                        .child(port.to_owned()),
                ),
        )
        .into_any_element()
}
