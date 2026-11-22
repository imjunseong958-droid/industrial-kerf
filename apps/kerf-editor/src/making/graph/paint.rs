//! The marks the graph paints with: a line through points and the room
//! under it washed, a dot ringed in a colour, a ring, and a cross.

use gpui::{fill, point, px, size, Bounds, PathBuilder, Pixels, Point, Rgba, Window};

/// A line through `points`.
pub(super) fn paint_line(window: &mut Window, points: &[(f32, f32)], color: Rgba, width: f32) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(point(px(points[0].0), px(points[0].1)));
    for (x, y) in &points[1..] {
        path.line_to(point(px(*x), px(*y)));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The room under a line down to `floor`, washed.
pub(super) fn paint_under(window: &mut Window, points: &[(f32, f32)], floor: f32, color: Rgba) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    let mut path = PathBuilder::fill();
    path.move_to(point(px(first.0), px(floor)));
    for (x, y) in points {
        path.line_to(point(px(*x), px(*y)));
    }
    path.line_to(point(px(last.0), px(floor)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A dot `radius` round at `at`, filled, with an edge in `edge` if any.
pub(super) fn paint_ring(
    window: &mut Window,
    at: Point<Pixels>,
    radius: f32,
    fill_color: Rgba,
    edge: Option<Rgba>,
) {
    let bounds = Bounds::new(
        at - point(px(radius), px(radius)),
        size(px(2.0 * radius), px(2.0 * radius)),
    );
    let mut quad = fill(bounds, fill_color).corner_radii(px(radius));
    if let Some(edge) = edge {
        quad = quad.border_widths(px(1.5)).border_color(edge);
    }
    window.paint_quad(quad);
}

/// A ring `radius` round at `at`, its line `width` wide.
pub(super) fn paint_ring_line(
    window: &mut Window,
    at: Point<Pixels>,
    radius: f32,
    color: Rgba,
    width: f32,
) {
    let outer = radius + width / 2.0;
    let bounds = Bounds::new(
        at - point(px(outer), px(outer)),
        size(px(2.0 * outer), px(2.0 * outer)),
    );
    window.paint_quad(
        fill(bounds, gpui::transparent_black())
            .corner_radii(px(outer))
            .border_widths(px(width))
            .border_color(color),
    );
}

/// A cross at `at`, its arms 4 points out.
pub(super) fn paint_cross(window: &mut Window, at: Point<Pixels>, color: Rgba, width: f32) {
    for (from, to) in [((-4.0, -4.0), (4.0, 4.0)), ((-4.0, 4.0), (4.0, -4.0))] {
        let mut path = PathBuilder::stroke(px(width));
        path.move_to(at + point(px(from.0), px(from.1)));
        path.line_to(at + point(px(to.0), px(to.1)));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}
