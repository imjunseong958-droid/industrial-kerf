//! The graph's cards on the canvas: a step's, or a thing a step reads, by
//! its kind, what it gives and what is wrong with it, with a model's
//! settings and a signal's curve or a clip's path under its lines, and a
//! making's stripes over the takes'; and a node of a model's workflow, its
//! inputs a row each.

use super::fields::one_line;
use super::layout::{
    draws_curve, CARD_CURVE, CARD_H, CARD_MODEL, CARD_W, PART_HEAD, PART_ROW, PART_W,
};
use super::paint::{paint_line, paint_ring, paint_under};
use super::signal::signal_line;
use super::ShotGraph;
use crate::making::{clock, input_kind, shown_value, value_text, GraphNode, InputKind, Recipe};
use editor_state::store::Making;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, radius, text};
use editor_ui::ui;
use gpui::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, Bounds, Context, ElementId,
    MouseButton, MouseDownEvent, Rgba,
};
use serde_json::Value;

/// What an edge carrying `gives` is coloured.
pub(super) fn gives_color(gives: &str) -> Rgba {
    match gives {
        "video" => theme::accent_ink(),
        "image" => theme::pos(),
        "path" => theme::playhead(),
        "signal" => theme::warn(),
        _ => theme::ink3(),
    }
}

pub(super) fn kind_icon(kind: &str) -> Icon {
    match kind {
        "sequence" => Icon::Film,
        "render" => Icon::Eye,
        "model" => Icon::Sparkle,
        "join" => Icon::Scissors,
        "file" => Icon::Folder,
        "signal" => Icon::Clock,
        "moves" => Icon::Pen,
        "cast" | "style" => Icon::Chat,
        "takes" => Icon::Media,
        _ => Icon::Dot,
    }
}

/// A model's seed, length, size and what else its line sets, as its card's
/// third line says them.
fn model_line(node: &GraphNode) -> String {
    let holds = &node.holds;
    let mut said = Vec::new();
    if !holds["seed"].is_null() {
        said.push(format!("seed {}", value_text(&holds["seed"])));
    }
    if !holds["seconds"].is_null() {
        said.push(format!("{} s", value_text(&holds["seconds"])));
    }
    if let Some(size) = holds["size"].as_str() {
        said.push(size.to_owned());
    }
    for knob in holds["knobs"].as_array().into_iter().flatten() {
        if !knob["value"].is_null() {
            said.push(format!(
                "{} {}",
                knob["name"].as_str().unwrap_or_default(),
                value_text(&knob["value"])
            ));
        }
    }
    if said.is_empty() {
        "as the model comes".to_owned()
    } else {
        said.join(" · ")
    }
}

/// What is wrong with a node, by a dot: not there in brick, wanting a look in ochre.
fn node_mark(node: &GraphNode) -> Option<Rgba> {
    let holds = &node.holds;
    if holds["there"] == Value::Bool(false) {
        return Some(theme::neg());
    }
    if node.kind == "model" && holds["setUp"] == Value::Bool(false) {
        return Some(theme::warn());
    }
    if node.kind == "takes" && !holds["wants"].is_null() {
        return Some(theme::warn());
    }
    None
}

impl ShotGraph {
    pub(super) fn card(
        &self,
        node: &GraphNode,
        at: (f32, f32),
        size: (f32, f32),
        making: Option<&Making>,
        wire_over: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.camera.zoom;
        let z = |points: f32| px(points * zoom);
        let color = gives_color(&node.gives);
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        let striped = making.filter(|_| node.kind == "takes");
        let lines = card_lines(node, striped, color, zoom);
        let phase = striped.map(|making| motion::phase(making.since, scene::STRIPE_CYCLE));
        // Under the lines: a signal's number over its clip, or the path a clip is led along.
        let under = if node.kind == "model" {
            CARD_H - 9.0 + CARD_MODEL
        } else {
            CARD_H - 9.0
        };
        let curve = draws_curve(node).then(|| card_curve(node, color, under, zoom));
        div()
            .id(ElementId::Name(format!("node-{}", node.id).into()))
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(px(size.0))
            .h(px(size.1))
            .rounded(z(radius::PANEL))
            .overflow_hidden()
            .bg(theme::card())
            .when(wire_over, |card| {
                card.border(px(1.5)).border_color(theme::accent_ink())
            })
            .when(!wire_over, |card| {
                card.border_1().border_color(if selected {
                    theme::accent()
                } else {
                    theme::line2()
                })
            })
            .when(!selected && !wire_over, |card| {
                card.hover(|card| card.border_color(theme::lineh()))
            })
            .cursor_pointer()
            .when_some(phase, |card, phase| {
                card.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            crate::media::paint_stripes(window, bounds, phase)
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(z(10.0))
                    .p(z(10.0))
                    .child(
                        div()
                            .flex_none()
                            .size(z(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(z(radius::CARD))
                            .bg(theme::with_alpha(color, 0.16))
                            .child(kind_icon(&node.kind).view(16.0 * zoom, color)),
                    )
                    .child(lines),
            )
            .children(curve)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_card(&id, event, window, cx);
                }),
            )
            .into_any_element()
    }

    /// A node of a model's workflow: its name and number, over a row an
    /// input: a link comes in at its row; a blank is the steps', in the
    /// accent's ink on a wash of it; a value is the workflow's own, plain.
    pub(super) fn part_card(
        &self,
        node: &GraphNode,
        _graph: &Recipe,
        at: (f32, f32),
        size: (f32, f32),
        zoom: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let z = |points: f32| px(points * zoom);
        let selected = self.selected.as_deref() == Some(node.id.as_str());
        let id = node.id.clone();
        let number = format!("#{}", value_text(&node.holds["id"]));
        let rows = node.holds["inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|input| {
                let name = input["name"].as_str().unwrap_or_default().to_owned();
                let kind = input_kind(&input);
                let value = shown_value(&input["value"]);
                div()
                    .h(z(PART_ROW))
                    .flex()
                    .items_center()
                    .gap(z(10.0))
                    .px(z(10.0))
                    .child(
                        zoomed(
                            ui::line(
                                name,
                                text::META,
                                if kind == InputKind::Link {
                                    theme::ink2()
                                } else {
                                    theme::ink3()
                                },
                            ),
                            text::META,
                            zoom,
                        )
                        .flex_shrink()
                        .min_w_0()
                        .max_w(z(PART_W * 0.5)),
                    )
                    .child(div().flex_1())
                    .when(kind == InputKind::Blank, |row| {
                        row.child(
                            div()
                                .px(z(5.0))
                                .rounded(z(radius::SMALL))
                                .bg(theme::accent_wash())
                                .min_w_0()
                                .child(zoomed(
                                    ui::line(value.clone(), text::MONO_SMALL, theme::accent_ink()),
                                    text::MONO_SMALL,
                                    zoom,
                                )),
                        )
                    })
                    .when(kind == InputKind::Value, |row| {
                        row.child(
                            zoomed(
                                ui::line(value.clone(), text::MONO_SMALL, theme::ink2()),
                                text::MONO_SMALL,
                                zoom,
                            )
                            .min_w_0(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        div()
            .id(ElementId::Name(format!("node-{}", node.id).into()))
            .absolute()
            .left(px(at.0))
            .top(px(at.1))
            .w(px(size.0))
            .h(px(size.1))
            .rounded(z(radius::CARD))
            .overflow_hidden()
            .bg(theme::card())
            .border_1()
            .border_color(if selected {
                theme::accent()
            } else {
                theme::line2()
            })
            .when(!selected, |card| {
                card.hover(|card| card.border_color(theme::lineh()))
            })
            .cursor_pointer()
            .child(
                div()
                    .h(z(PART_HEAD))
                    .flex()
                    .items_center()
                    .gap(z(8.0))
                    .px(z(10.0))
                    .pt(z(4.0))
                    .child(
                        zoomed(
                            ui::line(node.title.clone(), text::CONTROL_STRONG, theme::ink()),
                            text::CONTROL_STRONG,
                            zoom,
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(zoomed(
                        ui::label(number, text::MONO_SMALL, theme::ink3()),
                        text::MONO_SMALL,
                        zoom,
                    )),
            )
            .child(div().mx(px(1.0)).h(px(1.0)).bg(theme::line()))
            .children(rows)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_card(&id, event, window, cx);
                }),
            )
            .into_any_element()
    }
}

/// A card's lines: its title, by a dot what is wrong with it, and what it
/// gives; what it is, or its making's take and clock over the stripes; and
/// a model's settings.
fn card_lines(node: &GraphNode, striped: Option<&Making>, color: Rgba, zoom: f32) -> gpui::Div {
    let z = |points: f32| px(points * zoom);
    let detail = match striped {
        Some(making) => format!("Making take {} · {}", making.take, clock(making.since)),
        None if node.kind == "model" && node.detail.trim().is_empty() => "no prompt".to_owned(),
        None => one_line(&node.detail),
    };
    let mut lines = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(z(6.0))
                .h(z(20.0))
                .child(
                    zoomed(
                        ui::line(node.title.clone(), text::BODY_NAME, theme::ink()),
                        text::BODY_NAME,
                        zoom,
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .when_some(node_mark(node), |row, mark| {
                    row.child(ui::dot(mark, 6.0 * zoom))
                })
                .child(
                    div()
                        .flex_none()
                        .px(z(6.0))
                        .h(z(16.0))
                        .flex()
                        .items_center()
                        .rounded(z(8.0))
                        .bg(theme::with_alpha(color, 0.14))
                        .child(zoomed(
                            ui::label(node.gives.clone(), text::BADGE, color),
                            text::BADGE,
                            zoom,
                        )),
                ),
        )
        // Over the stripes it is said in ink: amber on amber is not read.
        .child(zoomed(
            ui::line(
                detail,
                text::META,
                if striped.is_some() {
                    theme::ink()
                } else {
                    theme::ink3()
                },
            ),
            text::META,
            zoom,
        ));
    if node.kind == "model" {
        lines = lines.child(zoomed(
            ui::line(model_line(node), text::MONO_SMALL, theme::ink3()),
            text::MONO_SMALL,
            zoom,
        ));
    }
    lines
}

/// A signal's curve or a clip's path drawn under a card's lines, `under`
/// points down it.
fn card_curve(node: &GraphNode, color: Rgba, under: f32, zoom: f32) -> AnyElement {
    let holds = &node.holds;
    let values: Vec<f64> = holds["values"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| value.as_f64().unwrap_or(0.0))
        .collect();
    let aspect = holds["aspect"].as_f64().unwrap_or(16.0 / 9.0).max(0.1);
    let path: Option<Vec<(f64, f64)>> = holds["path"]
        .as_str()
        .and_then(editor_stage::stage::overlay::parse_path)
        .map(|points| {
            if points.len() >= 2 {
                editor_stage::stage::overlay::MotionPath::new(points, aspect).samples()
            } else {
                points
            }
        });
    let signal = node.kind == "signal";
    let ground = theme::with_alpha(theme::surround_dim(), 0.6);
    let node_fill = theme::card();
    div()
        .absolute()
        .left(px(12.0 * zoom))
        .top(px((under + 3.0) * zoom))
        .w(px((CARD_W - 24.0) * zoom))
        .h(px((CARD_CURVE - 12.0) * zoom))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                    let (width, height) =
                        (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(px(left - 4.0 * zoom), px(top - 3.0 * zoom)),
                                size(px(width + 8.0 * zoom), px(height + 6.0 * zoom)),
                            ),
                            ground,
                        )
                        .corner_radii(px(radius::SMALL * zoom)),
                    );
                    if signal {
                        let line = signal_line(&values, [left, top, width, height]);
                        paint_under(window, &line, top + height, theme::with_alpha(color, 0.16));
                        paint_line(window, &line, color, 1.25);
                        return;
                    }
                    let Some(path) = &path else {
                        return;
                    };
                    // In frame widths across and frame heights down, as the clip's x and y are.
                    let drawn: Vec<(f64, f64)> =
                        path.iter().map(|(x, y)| (x * aspect, *y)).collect();
                    let (mut low, mut high) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
                    for (x, y) in &drawn {
                        low = (low.0.min(*x), low.1.min(*y));
                        high = (high.0.max(*x), high.1.max(*y));
                    }
                    let fit = (f64::from(width) / (high.0 - low.0).max(1e-3))
                        .min(f64::from(height) / (high.1 - low.1).max(1e-3));
                    let middle = ((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0);
                    let line: Vec<(f32, f32)> = drawn
                        .iter()
                        .map(|(x, y)| {
                            (
                                left + width / 2.0 + ((x - middle.0) * fit) as f32,
                                top + height / 2.0 + ((y - middle.1) * fit) as f32,
                            )
                        })
                        .collect();
                    paint_line(window, &line, color, 1.25);
                    if let (Some(first), Some(last)) = (line.first(), line.last()) {
                        paint_ring(
                            window,
                            point(px(first.0), px(first.1)),
                            2.5,
                            node_fill,
                            Some(color),
                        );
                        paint_ring(window, point(px(last.0), px(last.1)), 2.5, color, None);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// `element`'s type set in `style` at `zoom`.
fn zoomed(element: gpui::Div, style: theme::TypeStyle, zoom: f32) -> gpui::Div {
    element
        .text_size(px(style.px * zoom))
        .line_height(px(style.line * zoom))
}
