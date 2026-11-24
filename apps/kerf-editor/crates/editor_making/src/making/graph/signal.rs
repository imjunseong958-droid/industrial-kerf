//! A signal drawn by hand in a shot's graph: its values as a line, the
//! frame and value under the pointer, a drawing filled in between where the
//! pointer was and kept as keys, the column's curve drawn and drawn on, and
//! its moments read from a sound of its sequence.

use super::drag::Drag;
use super::inspector::note;
use super::paint::{paint_line, paint_ring, paint_under};
use super::{MenuKind, ShotGraph};
use crate::making::{GraphNode, Holds};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, radius};
use editor_ui::ui;
use gpui::{
    canvas, div, fill, point, prelude::*, px, size, Bounds, Context, CursorStyle, MouseButton,
    MouseDownEvent, Pixels, Point, Window,
};
use kerf_bridge::event::graph::What;
use kerf_bridge::Command;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A signal's curve, drawn to be drawn on.
const CURVE_HEIGHT: f32 = 112.0;
const CURVE_INSET: f32 = 8.0;
/// How far a drawn signal's line may miss a frame between its keys.
const CURVE_WITHIN: f64 = 0.02;

/// A signal's curve being drawn, or drawn and not yet heard back.
pub(super) struct Drawing {
    clip: String,
    frames: usize,
    drawn: BTreeMap<usize, f64>,
    last: Option<usize>,
    /// The pointer is still down.
    pub(super) held: bool,
}

/// A signal's values as a line across `rect` (left, top, width, height):
/// 0 at its foot and 1 at its top, the first value at its left and the last
/// at its right; one value is a level across the whole of it.
pub fn signal_line(values: &[f64], rect: [f32; 4]) -> Vec<(f32, f32)> {
    let [left, top, width, height] = rect;
    if values.is_empty() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let last = values.len() - 1;
    let mut line: Vec<(f32, f32)> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let along = if last == 0 {
                0.0
            } else {
                index as f32 / last as f32
            };
            let value = if value.is_finite() {
                value.clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            (left + along * width, top + height - value * height)
        })
        .collect();
    if last == 0 {
        line.push((left + width, line[0].1));
    }
    line
}

/// The frame and the value under `at` on a curve of `frames` frames drawn
/// across `rect`: the nearest frame, and 0 at the foot to 1 at the top, each
/// held to the curve.
pub fn signal_under(rect: [f32; 4], frames: usize, at: (f32, f32)) -> (usize, f64) {
    let [left, top, width, height] = rect;
    let last = frames.max(1) - 1;
    let along = if width > 0.0 {
        ((at.0 - left) / width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let frame = ((along * last as f32).round() as usize).min(last);
    let value = if height > 0.0 {
        (1.0 - (at.1 - top) / height).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (frame, f64::from(value))
}

/// A drawing goes on to `value` at `frame`: every frame the pointer passed
/// since the `last` it was at lies on the line between the two.
pub fn draw_signal(
    drawn: &mut BTreeMap<usize, f64>,
    last: Option<usize>,
    frame: usize,
    value: f64,
) {
    if let Some(last) = last.filter(|last| *last != frame) {
        let start = drawn.get(&last).copied().unwrap_or(value);
        let span = frame as f64 - last as f64;
        let (low, high) = (last.min(frame), last.max(frame));
        for at in low + 1..high {
            let share = (at as f64 - last as f64) / span;
            drawn.insert(at, start + (value - start) * share);
        }
    }
    drawn.insert(frame, value);
}

/// The keys a drawing is kept as: from its first frame on, a frame is a
/// key where a straight line from the last key to the next frame would miss
/// a frame between by more than `within`; and its last frame. Each is
/// linear, its value to a thousandth.
pub fn signal_keys(drawn: &BTreeMap<usize, f64>, within: f64) -> Vec<Value> {
    let frames: Vec<(usize, f64)> = drawn.iter().map(|(at, value)| (*at, *value)).collect();
    let key = |(at, value): (usize, f64)| json!({"at": at, "value": (value * 1000.0).round() / 1000.0, "interp": "linear"});
    let Some(first) = frames.first() else {
        return Vec::new();
    };
    let mut keys = vec![key(*first)];
    let mut kept = 0;
    for next in 2..frames.len() {
        let (a, b) = (frames[kept], frames[next]);
        let span = (b.0 - a.0) as f64;
        let misses = (kept + 1..next).any(|between| {
            let on = a.1 + (b.1 - a.1) * (frames[between].0 - a.0) as f64 / span;
            (on - frames[between].1).abs() > within
        });
        if misses {
            kept = next - 1;
            keys.push(key(frames[kept]));
        }
    }
    if frames.len() > 1 {
        keys.push(key(frames[frames.len() - 1]));
    }
    keys
}

impl ShotGraph {
    fn start_curve(
        &mut self,
        clip: String,
        frames: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.canvas_focus);
        self.drawing = Some(Drawing {
            clip,
            frames: frames.max(1),
            drawn: BTreeMap::new(),
            last: None,
            held: true,
        });
        self.drag = Drag::Curve;
        self.draw_curve(at);
        cx.notify();
    }

    pub(super) fn draw_curve(&mut self, at: Point<Pixels>) {
        let bounds = self.curve_bounds.get();
        let rect = curve_plot(bounds);
        let at = (f32::from(at.x), f32::from(at.y));
        let Some(drawing) = self.drawing.as_mut() else {
            return;
        };
        let (frame, value) = signal_under(rect, drawing.frames, at);
        draw_signal(&mut drawing.drawn, drawing.last, frame, value);
        drawing.last = Some(frame);
    }

    /// Let go: what was drawn is the clip's value, keyed, in one edit; a
    /// single press is not a drawing. What was drawn stays shown until the
    /// project has it and says its curve again.
    pub(super) fn drawn(&mut self, cx: &mut Context<Self>) {
        let Some(drawing) = self.drawing.as_mut() else {
            return;
        };
        drawing.held = false;
        if drawing.drawn.len() < 2 {
            self.drawing = None;
            return;
        }
        let keys = signal_keys(&drawing.drawn, CURVE_WITHIN);
        let op = json!({"op": "set", "clip": drawing.clip, "props": {"value": {"keys": keys}}});
        self.session.update(cx, |session, cx| {
            session.edit(vec![op]);
            cx.notify();
        });
    }

    /// A signal's curve, drawn across to draw it anew, and its moments read
    /// from a sound of its sequence.
    pub(super) fn signal_column(&self, node: &GraphNode, cx: &mut Context<Self>) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        let Holds::Shot(What::Signal(signal)) = &node.holds else {
            return column;
        };
        if node.is_missing() {
            return column.child(note(node.detail.clone(), theme::neg()));
        }
        column = column.child(note(
            "A number from 0 to 1 over the shot's time. The prompt says what it is of; this says when, and how much.".into(),
            theme::ink2(),
        ));
        let values = signal.values.clone().unwrap_or_default();
        let frames = signal.frames.unwrap_or(1).max(1) as usize;
        let clip = signal.clip.clone();
        let drawn: Option<Vec<(usize, f64)>> = self
            .drawing
            .as_ref()
            .filter(|drawing| drawing.clip == clip)
            .map(|drawing| {
                drawing
                    .drawn
                    .iter()
                    .map(|(at, value)| (*at, *value))
                    .collect()
            });
        let bounds = self.curve_bounds.clone();
        let press_clip = clip.clone();
        column = column.child(ui::tip_on(
            div()
                .id("graph-signal-curve")
                .h(px(CURVE_HEIGHT))
                .w_full()
                .rounded(px(radius::CONTROL))
                .bg(theme::well())
                .border_1()
                .border_color(theme::line2())
                .cursor(CursorStyle::Crosshair)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.start_curve(press_clip.clone(), frames, event.position, window, cx);
                    }),
                )
                .child(
                    canvas(
                        move |area, _, _| bounds.set(area),
                        move |area, _, window, _| {
                            paint_signal(window, area, &values, frames, drawn.as_deref())
                        },
                    )
                    .size_full(),
                ),
            Some(ui::Tip::new(
                "Drag across to draw the signal: 0 at the foot, 1 at the top",
            )),
        ));
        column = column.child(note(
            format!(
                "{}. Drag across it to draw it anew; it is the clip's value in the project, and is undone as any edit.",
                node.detail
            ),
            theme::ink3(),
        ));
        // Its moments read from a sound of its sequence: 1 for a few frames
        // from where each sound starts.
        let sounds = signal.sounds.clone().unwrap_or_default();
        if sounds.is_empty() {
            return column;
        }
        let sound_open = self
            .menu
            .as_ref()
            .is_some_and(|open| open.kind == MenuKind::Sound);
        let trigger = ui::trigger(
            "graph-sound-trigger",
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if this.close_menu(MenuKind::Sound, window, cx) {
                    return;
                }
                let this_entity = cx.weak_entity();
                let items = sounds
                    .iter()
                    .map(|sound| {
                        let of = sound.clip.clone();
                        let name = if sound.name.is_empty() {
                            of.clone()
                        } else {
                            sound.name.clone()
                        };
                        let (this, clip) = (this_entity.clone(), clip.clone());
                        ui::MenuItem::new(name).on_choose(move |_, cx| {
                            let command = Command::SignalFromSound {
                                clip: clip.clone(),
                                of: of.clone(),
                            };
                            // A send refused is said by the session itself.
                            let _ = this.update(cx, |graph, cx| {
                                graph.session.update(cx, |session, _| session.send(command))
                            });
                        })
                    })
                    .collect();
                let at = event.position + point(px(0.0), px(16.0));
                this.open_menu(
                    MenuKind::Sound,
                    items,
                    Some("Where this sound's sounds start".to_owned()),
                    at,
                    window,
                    cx,
                );
            }),
        )
        .child(
            ui::button("graph-signal-sound", "From a sound")
                .ghost()
                .icon(Icon::ChevronDown)
                .when(!sound_open, |button| {
                    button.tooltip("Set it to 1 for three frames from each place a sound of its sequence starts, the beats of a track or a ball on a floor, and 0 between")
                }),
        );
        column.child(div().flex().child(trigger))
    }
}

/// The plot of a curve drawn `bounds` big: inside its edge.
fn curve_plot(bounds: Bounds<Pixels>) -> [f32; 4] {
    [
        f32::from(bounds.origin.x) + CURVE_INSET,
        f32::from(bounds.origin.y) + CURVE_INSET,
        f32::from(bounds.size.width) - 2.0 * CURVE_INSET,
        f32::from(bounds.size.height) - 2.0 * CURVE_INSET,
    ]
}

/// A signal's curve: its floor, middle and top as hairlines, its values
/// washed underneath, and a curve being drawn over it in ink, the one it
/// was falling back to 45%.
fn paint_signal(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    values: &[f64],
    frames: usize,
    drawn: Option<&[(usize, f64)]>,
) {
    let rect = curve_plot(bounds);
    let [left, top, width, height] = rect;
    for share in [0.0f32, 0.5, 1.0] {
        let y = (top + height - height * share).round();
        window.paint_quad(fill(
            Bounds::new(point(px(left), px(y)), size(px(width), px(1.0))),
            theme::line(),
        ));
    }
    let ink = theme::warn();
    let line = signal_line(values, rect);
    let drawing = drawn.is_some_and(|drawn| !drawn.is_empty());
    if !line.is_empty() {
        paint_under(window, &line, top + height, theme::with_alpha(ink, 0.16));
        let was = if drawing {
            theme::with_alpha(ink, 0.45)
        } else {
            ink
        };
        paint_line(window, &line, was, 1.5);
    }
    let Some(drawn) = drawn.filter(|drawn| !drawn.is_empty()) else {
        return;
    };
    let last = frames.max(2) - 1;
    let points: Vec<(f32, f32)> = drawn
        .iter()
        .map(|(at, value)| {
            (
                left + width * *at as f32 / last as f32,
                top + height - height * *value as f32,
            )
        })
        .collect();
    if points.len() == 1 {
        paint_ring(
            window,
            point(px(points[0].0), px(points[0].1)),
            1.5,
            theme::ink(),
            None,
        );
    } else {
        paint_line(window, &points, theme::ink(), 1.5);
    }
}
