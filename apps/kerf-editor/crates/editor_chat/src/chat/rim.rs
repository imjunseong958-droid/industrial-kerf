//! The light round a cut's picture: points round a rounded rectangle
//! ([`rim_point`]), the light running round it while Kerf makes the cut,
//! and the ring lit once as it is done.

use editor_ui::theme::{self, motion as scene};
use gpui::{point, px, Window};

/// The point `t` (0..1) of the way round a rounded rectangle, clockwise
/// from the middle of its top edge.
pub fn rim_point(
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: f32,
    t: f32,
) -> gpui::Point<gpui::Pixels> {
    let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (straight_w, straight_h) = (w - 2.0 * r, h - 2.0 * r);
    let arc = std::f32::consts::FRAC_PI_2 * r;
    let total = 2.0 * (straight_w + straight_h) + 4.0 * arc;
    if total <= 0.0 {
        return bounds.origin;
    }
    let mut left = t.rem_euclid(1.0) * total;
    // The legs, clockwise from the top's middle: each a line or a quarter arc.
    let legs: [(f32, u8); 9] = [
        (straight_w / 2.0, 0),
        (arc, 1),
        (straight_h, 2),
        (arc, 3),
        (straight_w, 4),
        (arc, 5),
        (straight_h, 6),
        (arc, 7),
        (straight_w / 2.0, 8),
    ];
    for (length, leg) in legs {
        if left > length && leg != 8 {
            left -= length;
            continue;
        }
        let along = if length > 0.0 { left / length } else { 0.0 };
        let corner = |cx: f32, cy: f32, from: f32| {
            let angle = from + along * std::f32::consts::FRAC_PI_2;
            (cx + r * angle.cos(), cy + r * angle.sin())
        };
        let (px_, py_) = match leg {
            0 => (x + w / 2.0 + left, y),
            1 => corner(x + w - r, y + r, -std::f32::consts::FRAC_PI_2),
            2 => (x + w, y + r + left),
            3 => corner(x + w - r, y + h - r, 0.0),
            4 => (x + w - r - left, y + h),
            5 => corner(x + r, y + h - r, std::f32::consts::FRAC_PI_2),
            6 => (x, y + h - r - left),
            7 => corner(x + r, y + r, std::f32::consts::PI),
            _ => (x + r + left, y),
        };
        return point(px(px_), px(py_));
    }
    bounds.origin
}

/// The light running round a cut's picture while Kerf makes it: a head of
/// white going into the accent's blue, `tail` of the way round behind it,
/// at `turn` (0..1). Under Reduce Motion, a still ring.
pub(super) fn paint_rim(
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: f32,
    turn: Option<f32>,
    window: &mut Window,
) {
    const RUNS: usize = 18;
    const TAIL: f32 = scene::CARD_TAIL;
    const POINTS: usize = 6;
    let blue = gpui::Rgba {
        r: 143.0 / 255.0,
        g: 176.0 / 255.0,
        b: 1.0,
        a: 1.0,
    };
    let Some(turn) = turn else {
        let mut ring = gpui::PathBuilder::stroke(px(2.0));
        let points: Vec<_> = (0..=96)
            .map(|step| rim_point(bounds, radius, step as f32 / 96.0))
            .collect();
        ring.add_polygon(&points, true);
        if let Ok(path) = ring.build() {
            window.paint_path(path, theme::with_alpha(blue, 0.6));
        }
        return;
    };
    for run in 0..RUNS {
        let fade = 1.0 - run as f32 / RUNS as f32;
        let mix = (run as f32 / 3.0).min(1.0);
        let shade = gpui::Rgba {
            r: 1.0 + (blue.r - 1.0) * mix,
            g: 1.0 + (blue.g - 1.0) * mix,
            b: 1.0,
            a: 0.95 * fade * fade,
        };
        let head = turn - run as f32 * TAIL / RUNS as f32;
        let points: Vec<_> = (0..=POINTS)
            .map(|step| {
                let at = head - step as f32 * TAIL / RUNS as f32 / POINTS as f32;
                rim_point(bounds, radius, at)
            })
            .collect();
        let mut line = gpui::PathBuilder::stroke(px(2.0));
        line.add_polygon(&points, false);
        if let Ok(path) = line.build() {
            window.paint_path(path, shade);
        }
    }
}

/// A ring round a cut's picture lit once as it is done, going out.
pub(super) fn paint_ring(
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: f32,
    light: f32,
    window: &mut Window,
) {
    if light <= 0.0 {
        return;
    }
    let points: Vec<_> = (0..=96)
        .map(|step| rim_point(bounds, radius, step as f32 / 96.0))
        .collect();
    let mut ring = gpui::PathBuilder::stroke(px(2.0));
    ring.add_polygon(&points, true);
    if let Ok(path) = ring.build() {
        window.paint_path(path, theme::with_alpha(gpui::white().into(), 0.94 * light));
    }
}
