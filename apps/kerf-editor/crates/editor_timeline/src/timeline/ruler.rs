//! The ruler over the lanes and what stands across it: each open
//! proposal's band, the ticks and their labels at the zoom, the markers'
//! pins, the program's in and out; and down the ruler and the lanes,
//! Kerf's cursor while it works and the playhead.

use super::canvas::{masked, shape, Fonts, Frame, Pen};
use super::viewport::Viewport;
use editor_ui::theme::{self, layout, radius, text};
use gpui::{
    fill, linear_color_stop, linear_gradient, px, quad, App, BorderStyle, Corners, PathBuilder,
    SharedString, Window,
};

/// Points between ruler labels, at the least.
const LABEL_SPACE: f32 = 84.0;
/// Points between minor ticks, at the least.
const TICK_SPACE: f32 = 7.0;
/// Points of a proposal's band along the ruler's top.
pub(super) const BAND_ROW: f32 = 5.0;

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

pub(super) fn paint_ruler(
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
        paint_bands(frame, pen, width, window);
        paint_ticks(frame, (major, minor), pen, width, fonts, window, cx);
        paint_pins(frame, pen, width, window);
        paint_marks(view, frame.marks, lanes.end, pen, width, window);
    });
}

/// Each open proposal's band, under everything on the ruler.
fn paint_bands(frame: &Frame, pen: Pen, width: f32, window: &mut Window) {
    let view = &frame.view;
    for band in &frame.lanes.bands {
        let x0 = view.x(band.start);
        let x1 = view.x(band.end).max(x0 + 3.0);
        let (left, right) = (x0.max(-2.0), x1.min(width + 2.0));
        if right <= left {
            continue;
        }
        window.paint_quad(fill(
            pen.rect(left, 0.0, right - left, layout::RULER - 1.0),
            theme::with_alpha(theme::ink(), 0.05),
        ));
        window.paint_quad(quad(
            pen.rect(left, 1.0, right - left, BAND_ROW),
            px(BAND_ROW / 2.0),
            theme::with_alpha(theme::pos(), 0.6),
            px(0.0),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// The ticks across the ruler, every `minor` seconds, each `major` taller
/// and labelled.
fn paint_ticks(
    frame: &Frame,
    (major, minor): (f64, f64),
    pen: Pen,
    width: f32,
    fonts: &Fonts,
    window: &mut Window,
    cx: &mut App,
) {
    let view = &frame.view;
    let fps = frame.lanes.fps;
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
            let words: SharedString = ruler_label(time, fps, major).into();
            let line = shape(window, words, text::MONO_SMALL, &fonts.mono, theme::ink3());
            if let Err(error) =
                line.paint(pen.at(x + 4.0, 2.0), px(text::MONO_SMALL.line), window, cx)
            {
                log::warn!("a ruler label did not draw: {error}");
            }
        }
    }
}

/// Each sequence marker: its span along the ruler's foot, and its pin.
fn paint_pins(frame: &Frame, pen: Pen, width: f32, window: &mut Window) {
    let view = &frame.view;
    for mark in &frame.lanes.markers {
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
// Painting takes the window, where the canvas is and each measure it draws with.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_kerf(
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
    if let Err(error) = name.paint(
        pen.at(x + 6.0, layout::RULER),
        px(text::META.line),
        window,
        cx,
    ) {
        log::warn!("Kerf's cursor's name did not draw: {error}");
    }
}

/// The playhead: a line down the ruler and the lanes, with its head on the ruler.
pub(super) fn paint_playhead(
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
