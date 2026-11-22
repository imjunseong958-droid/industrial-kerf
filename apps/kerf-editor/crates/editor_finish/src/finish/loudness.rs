//! Loudness, Finish's second section: the programme measured to EBU R128,
//! the three numbers it delivers on, and its loudness over the programme
//! drawn as a chart that reads out where the pointer is.

use super::{empty, number, said, working, FinishView};
use editor_state::model;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use gpui::{
    canvas, div, point, prelude::*, px, AnyElement, Bounds, Context, MouseMoveEvent, PathBuilder,
    Pixels, Rgba, SharedString, Window,
};
use kerf_bridge::event::LoudnessReady;
use kerf_bridge::Command;
use std::rc::Rc;

/// The loudness chart's height, and the loudness it spans (LUFS).
const CHART_HEIGHT: f32 = 168.0;
const CHART_TOP: f32 = -6.0;
const CHART_BOTTOM: f32 = -54.0;
/// The levels the chart draws a hairline at, beside the target's.
const CHART_LEVELS: [f32; 3] = [-12.0, -36.0, -48.0];
/// The most points the chart draws, whatever the programme's length.
const CHART_POINTS: usize = 480;
/// How far integrated loudness may be from its target, in LU.
pub(super) const TOLERANCE: f64 = 1.0;
/// What R128 asks of a programme, until a measurement says otherwise.
const TARGET: f64 = -23.0;
const PEAK_LIMIT: f64 = -1.0;

/// The last measurement of the programme's loudness (`loudness {status: ready}`).
pub(super) struct Loudness {
    pub(super) integrated: Option<f64>,
    lra: Option<f64>,
    true_peak: Option<f64>,
    short_term_max: Option<f64>,
    duration: f64,
    pub(super) target: f64,
    limit: f64,
    revision: u64,
    points: Rc<Vec<ChartPoint>>,
}

/// One point of the chart: its time, its place across (0..1), and the
/// momentary and short-term loudness there (None for silence).
#[derive(Clone, Copy, Debug)]
struct ChartPoint {
    at: f32,
    x: f32,
    momentary: Option<f32>,
    short: Option<f32>,
}

/// `value` where it is a number to show: silence (-inf), which the bridge
/// writes as `null`, is none.
fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

impl Loudness {
    pub(super) fn of(ready: &LoudnessReady) -> Self {
        let measured = &ready.loudness;
        let series: Vec<(f32, Option<f32>, Option<f32>)> = measured
            .series
            .iter()
            .filter_map(|&(at, momentary, short)| {
                let level = |level: f64| finite(level).map(|level| level as f32);
                Some((finite(at)? as f32, level(momentary), level(short)))
            })
            .collect();
        let last = series
            .last()
            .map(|sample| f64::from(sample.0))
            .unwrap_or(0.0);
        let duration = finite(measured.duration)
            .filter(|d| *d > 0.0)
            .unwrap_or(last);
        Self {
            integrated: finite(measured.integrated),
            lra: finite(measured.lra),
            true_peak: finite(measured.true_peak_max),
            short_term_max: finite(measured.short_term_max),
            duration,
            target: finite(ready.target).unwrap_or(TARGET),
            limit: finite(ready.limit).unwrap_or(PEAK_LIMIT),
            revision: ready.revision,
            points: Rc::new(chart_points(&series, duration as f32)),
        }
    }
}

/// `series` (a sample each 0.1 s) in at most [`CHART_POINTS`] points, each
/// the mean of the loudness its samples heard.
fn chart_points(series: &[(f32, Option<f32>, Option<f32>)], duration: f32) -> Vec<ChartPoint> {
    if series.is_empty() {
        return Vec::new();
    }
    let size = series.len().div_ceil(CHART_POINTS).max(1);
    let mean = |values: &mut dyn Iterator<Item = f32>| {
        let (sum, count) = values.fold((0.0, 0), |(sum, count), value| (sum + value, count + 1));
        (count > 0).then(|| sum / count as f32)
    };
    series
        .chunks(size)
        .map(|chunk| {
            let at = chunk[0].0;
            ChartPoint {
                at,
                x: if duration > 0.0 {
                    (at / duration).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                momentary: mean(&mut chunk.iter().filter_map(|sample| sample.1)),
                short: mean(&mut chunk.iter().filter_map(|sample| sample.2)),
            }
        })
        .collect()
}

impl FinishView {
    /// Measures the programme to R128; unchanged since the last time, the
    /// bridge answers at once.
    fn measure(&mut self, cx: &mut Context<Self>) {
        if self.finishing.read(cx).measuring.is_some() {
            return;
        }
        let sent = self.session.update(cx, |session, _| {
            session.send(Command::Loudness { sequence: None })
        });
        if sent {
            self.finishing.update(cx, |finishing, cx| {
                finishing.measuring = Some(0.0);
                finishing.loudness_failed = None;
                cx.notify();
            });
        }
    }

    /// The section: its title and Measure, how a measurement runs or
    /// failed, and the last one's numbers and chart.
    pub(super) fn loudness(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let finishing = self.finishing.read(cx);
        let running = finishing.measuring;
        let revision = self.session.read(cx).revision();
        let measure = ui::button(
            "finish-measure",
            if finishing.loudness.is_some() {
                "Measure Again"
            } else {
                "Measure"
            },
        )
        .primary()
        .disabled(running.is_some() || !self.connected(cx))
        .on_click(cx.listener(|this, _, _, cx| this.measure(cx)))
        .into_any_element();
        let mut out = vec![self.title(
            "Loudness",
            "The programme measured to EBU R128: integrated loudness on −23 LUFS, true peak at most −1 dBTP.",
            Some(measure),
        )];
        if let Some(progress) = running {
            out.push(working("Measuring…", progress));
        }
        if let Some(failed) = &finishing.loudness_failed {
            out.push(said(failed.clone(), theme::neg()));
        }
        let Some(loudness) = &finishing.loudness else {
            if running.is_none() && finishing.loudness_failed.is_none() {
                out.push(empty("Not measured yet", "Measure listens to the whole programme; the answer is kept until the next edit."));
            }
            return out;
        };
        out.push(loudness_stats(loudness));
        out.push(self.chart(loudness, cx));
        out.push(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S4))
                .child(ui::label(
                    format!("Measured at revision {}", loudness.revision),
                    text::META,
                    theme::ink3(),
                ))
                .when(loudness.revision != revision, |row| {
                    row.child(ui::label(
                        "· the project changed since",
                        text::META,
                        theme::warn(),
                    ))
                })
                .into_any_element(),
        );
        out
    }

    /// The chart over the programme: its legend, the plot, and how long
    /// the programme is under it.
    fn chart(&self, loudness: &Loudness, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .child(chart_legend(loudness.target as f32))
            .child(self.chart_plot(loudness, cx))
            .child(
                div()
                    .flex()
                    .child(ui::label("0:00", text::MONO_SMALL, theme::ink3()))
                    .child(div().flex_1())
                    .child(ui::label(
                        model::length(loudness.duration),
                        text::MONO_SMALL,
                        theme::ink3(),
                    )),
            )
            .into_any_element()
    }

    /// The plot: momentary loudness and short-term over it, the levels and
    /// the target ticked, and the pointer's place read out.
    fn chart_plot(&self, loudness: &Loudness, cx: &Context<Self>) -> impl IntoElement {
        let points = loudness.points.clone();
        let target = loudness.target as f32;
        let hover = self.hover.filter(|_| !points.is_empty());
        let hovered = hover.and_then(|x| {
            let index = ((x * (points.len() - 1) as f32).round() as usize).min(points.len() - 1);
            points.get(index).copied()
        });
        let level_y = |level: f32| (CHART_TOP - level) / (CHART_TOP - CHART_BOTTOM) * CHART_HEIGHT;
        let tick = |level: f32| {
            ui::label(number(f64::from(level), 0), text::MONO_SMALL, theme::ink3())
                .absolute()
                .left(px(theme::space::S6))
                .top(px(level_y(level) - 16.0))
        };
        let readout = hovered.map(|point| {
            let level = |value: Option<f32>| {
                value.map_or("—".to_owned(), |v| {
                    format!("{} LUFS", number(f64::from(v), 1))
                })
            };
            format!(
                "{} · Short-term {} · Momentary {}",
                clock(f64::from(point.at)),
                level(point.short),
                level(point.momentary)
            )
        });
        let bounds = self.chart_bounds.clone();
        let painted = self.chart_bounds.clone();
        let hover_x = hovered.map(|point| point.x);
        div()
            .id("finish-chart")
            .relative()
            .h(px(CHART_HEIGHT))
            .w_full()
            .rounded(px(radius::CARD))
            .bg(theme::well())
            .border_1()
            .border_color(theme::line())
            .overflow_hidden()
            .child(
                canvas(
                    move |area, _, _| painted.set(Some(area)),
                    move |area, _, window, _| paint_chart(area, &points, target, hover_x, window),
                )
                .absolute()
                .inset_0(),
            )
            .children(CHART_LEVELS.iter().map(|level| tick(*level)))
            .child(tick(target))
            .when_some(readout, |chart, readout| {
                chart.child(
                    ui::label(readout, text::MONO_SMALL, theme::ink())
                        .absolute()
                        .top(px(theme::space::S6))
                        .right(px(theme::space::S8))
                        .px(px(theme::space::S6))
                        .rounded(px(radius::SMALL))
                        .bg(theme::with_alpha(theme::card(), 0.9)),
                )
            })
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let Some(area) = bounds.get() else {
                    return;
                };
                let across = (event.position.x - area.origin.x) / area.size.width;
                let inside = area.contains(&event.position);
                let next = inside.then_some(across.clamp(0.0, 1.0));
                if next != this.hover {
                    this.hover = next;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.hover.take().is_some() {
                    cx.notify();
                }
            }))
    }
}

/// The chart's head: what it shows, and its lines' legend.
fn chart_legend(target: f32) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S12))
        .child(ui::label(
            "Over the programme",
            text::META_STRONG,
            theme::ink3(),
        ))
        .child(div().flex_1())
        .child(legend(swatch(momentary_ink(), 1.0, false), "Momentary"))
        .child(legend(
            swatch(theme::accent_ink(), 2.0, false),
            "Short-term",
        ))
        .child(legend(
            swatch(target_ink(), 1.0, true),
            format!("Target {}", number(f64::from(target), 0)),
        ))
}

/// The three numbers R128 delivers on, each with what it is held to.
fn loudness_stats(loudness: &Loudness) -> AnyElement {
    let integrated = loudness.integrated.map(|value| {
        let off = value - loudness.target;
        if off.abs() <= TOLERANCE {
            (
                theme::pos(),
                format!("On target, {} LUFS", number(loudness.target, 0)),
            )
        } else {
            let way = if off > 0.0 { "over" } else { "under" };
            (
                theme::neg(),
                format!(
                    "{} LU {way} {} LUFS",
                    number(off.abs(), 1),
                    number(loudness.target, 0)
                ),
            )
        }
    });
    let peak = loudness.true_peak.map(|value| {
        if value <= loudness.limit {
            (
                theme::pos(),
                format!("Within {} dBTP", number(loudness.limit, 0)),
            )
        } else {
            (
                theme::neg(),
                format!(
                    "{} dB over {} dBTP",
                    number(value - loudness.limit, 1),
                    number(loudness.limit, 0)
                ),
            )
        }
    });
    let range_note = loudness.short_term_max.map(|max| {
        (
            theme::ink3(),
            format!("Short-term at most {} LUFS", number(max, 1)),
        )
    });
    div()
        .flex()
        .gap(px(theme::space::S12))
        .child(stat("Integrated", loudness.integrated, "LUFS", integrated))
        .child(stat("True peak", loudness.true_peak, "dBTP", peak))
        .child(stat("Loudness range", loudness.lra, "LU", range_note))
        .into_any_element()
}

fn stat(
    name: &'static str,
    value: Option<f64>,
    unit: &'static str,
    note: Option<(Rgba, String)>,
) -> AnyElement {
    let shown = value.map_or("—".to_owned(), |value| number(value, 1));
    div()
        .flex_1()
        .min_w_0()
        .px(px(theme::space::S12))
        .py(px(theme::space::S12))
        .flex()
        .flex_col()
        .gap(px(theme::space::S4))
        .rounded(px(radius::CARD))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line())
        .child(ui::label(name, text::META, theme::ink3()))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(theme::space::S4))
                .child(ui::label(shown, text::MONO_LARGE, theme::ink()))
                .when(value.is_some(), |row| {
                    row.child(ui::label(unit, text::META, theme::ink2()))
                }),
        )
        .child(match (value, note) {
            (None, _) => ui::label("Silent", text::META, theme::ink3()).into_any_element(),
            (Some(_), Some((color, words))) => div()
                .flex()
                .items_center()
                .gap(px(theme::space::S6))
                .child(ui::dot(color, 6.0))
                .child(ui::line(words, text::META, theme::ink2()).min_w_0())
                .into_any_element(),
            (Some(_), None) => div().into_any_element(),
        })
        .into_any_element()
}

/// Momentary loudness: short-term's colour, faint, as the finer grain of
/// the same measure.
fn momentary_ink() -> Rgba {
    theme::with_alpha(theme::accent_ink(), 0.6)
}

/// The target: recessive, a reference and not a series.
fn target_ink() -> Rgba {
    theme::ink3()
}

/// A line of the chart's legend: its mark, then its name in ink.
fn legend(mark: impl IntoElement, words: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S6))
        .child(mark)
        .child(ui::label(words, text::META, theme::ink2()))
}

/// A short line in a series' colour and weight, dashed for the target.
fn swatch(color: Rgba, weight: f32, dashed: bool) -> impl IntoElement {
    let piece = move |width: f32| div().w(px(width)).h(px(weight)).rounded_full().bg(color);
    if dashed {
        div()
            .flex()
            .gap(px(theme::space::S2))
            .child(piece(5.0))
            .child(piece(5.0))
            .child(piece(4.0))
    } else {
        div().flex().child(piece(18.0))
    }
}

/// Paints the chart in `area`: hairlines at a few levels, the target
/// dashed, momentary loudness thin and faint, short-term over it, and a
/// line where the pointer is. Silence breaks a line.
fn paint_chart(
    area: Bounds<Pixels>,
    points: &[ChartPoint],
    target: f32,
    hover: Option<f32>,
    window: &mut Window,
) {
    let height = f32::from(area.size.height);
    let width = f32::from(area.size.width);
    let y = |level: f32| {
        let share = ((CHART_TOP - level) / (CHART_TOP - CHART_BOTTOM)).clamp(0.0, 1.0);
        area.origin.y + px(height * share)
    };
    let x = |share: f32| area.origin.x + px(width * share);
    let across = |builder: &mut PathBuilder, level: f32| {
        builder.move_to(point(area.origin.x, y(level)));
        builder.line_to(point(area.origin.x + area.size.width, y(level)));
    };
    let mut grid = PathBuilder::stroke(px(1.0));
    for level in CHART_LEVELS {
        across(&mut grid, level);
    }
    if let Ok(path) = grid.build() {
        window.paint_path(path, theme::line());
    }
    let mut goal = PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(3.0)]);
    across(&mut goal, target);
    if let Ok(path) = goal.build() {
        window.paint_path(path, target_ink());
    }
    let series =
        |pick: fn(&ChartPoint) -> Option<f32>, weight: f32, color: Rgba, window: &mut Window| {
            let mut builder = PathBuilder::stroke(px(weight));
            let mut drawn = false;
            let mut down = false;
            for point_at in points {
                match pick(point_at) {
                    Some(level) => {
                        let at = point(x(point_at.x), y(level));
                        if down {
                            builder.line_to(at);
                            drawn = true;
                        } else {
                            builder.move_to(at);
                        }
                        down = true;
                    }
                    None => down = false,
                }
            }
            if drawn {
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }
        };
    series(|p| p.momentary, 1.0, momentary_ink(), window);
    series(|p| p.short, 2.0, theme::accent_ink(), window);
    if let Some(share) = hover {
        let mut cursor = PathBuilder::stroke(px(1.0));
        cursor.move_to(point(x(share), area.origin.y));
        cursor.line_to(point(x(share), area.origin.y + area.size.height));
        if let Ok(path) = cursor.build() {
            window.paint_path(path, theme::lineh());
        }
    }
}

/// Three bars of a meter, as an icon is drawn: loudness.
pub(super) fn meter_glyph(color: Rgba) -> impl IntoElement {
    let bar = move |tall: f32| div().w(px(2.0)).h(px(tall)).rounded_full().bg(color);
    div()
        .size(px(16.0))
        .flex_none()
        .flex()
        .items_end()
        .justify_center()
        .gap(px(theme::space::S2))
        .pb(px(2.5))
        .child(bar(6.0))
        .child(bar(11.0))
        .child(bar(8.0))
}

/// `m:ss.s`: a moment of the programme to the tenth.
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor();
    format!("{}:{:04.1}", minutes as u64, seconds - minutes * 60.0)
}
