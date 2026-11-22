//! The fades over a list that scrolls with no bar: at its top while a row is
//! cut there, at its foot while there is more below.

use crate::theme::{self, layout};
use gpui::{
    canvas, linear_color_stop, linear_gradient, point, prelude::*, px, size, Bounds, Rgba,
    ScrollHandle,
};

/// Whether a list that scrolls with no bar shows its fades: at its top
/// while a row is cut by the top edge (any scroll, for painted content
/// with no rows), at its foot while there is more below. `scrolled` is how
/// far it is scrolled down, `most` the furthest it can be, `top` where the
/// list's top is, and `rows` each row's top and bottom as laid out
/// unscrolled.
pub fn scroll_fades(scrolled: f32, most: f32, top: f32, rows: &[(f32, f32)]) -> (bool, bool) {
    let edge = top + scrolled;
    let cut = scrolled > 0.0
        && (rows.is_empty()
            || rows
                .iter()
                .any(|(row_top, row_bottom)| *row_top < edge - 1.0 && *row_bottom > edge + 1.0));
    let more = scrolled < most - 1.0;
    (cut, more)
}

/// The fades over a list scrolled by `handle` that shows no bar: `TOP_FADE`
/// of `ground` at its top while a row is cut there, `BOTTOM_FADE` at its
/// foot while there is more ([`scroll_fades`]). Put it after the list, in
/// a box of the list's place (`relative`); it takes no press.
pub fn scroll_fade(handle: &ScrollHandle, ground: Rgba) -> impl IntoElement + use<> {
    let handle = handle.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let scrolled = -f32::from(handle.offset().y);
            let most = f32::from(handle.max_offset().height);
            let rows: Vec<(f32, f32)> = (0..handle.children_count())
                .filter_map(|at| handle.bounds_for_item(at))
                .map(|row| (f32::from(row.top()), f32::from(row.bottom())))
                .collect();
            let (cut, more) = scroll_fades(scrolled, most, f32::from(view.top()), &rows);
            let solid = ground;
            let clear = theme::with_alpha(ground, 0.0);
            if cut {
                let top = Bounds::new(
                    view.origin,
                    size(view.size.width, px(layout::TOP_FADE).min(view.size.height)),
                );
                window.paint_quad(gpui::fill(
                    top,
                    linear_gradient(
                        180.0,
                        linear_color_stop(solid, 0.0),
                        linear_color_stop(clear, 1.0),
                    ),
                ));
            }
            if more {
                let tall = px(layout::BOTTOM_FADE).min(view.size.height);
                let bottom = Bounds::new(
                    point(view.left(), view.bottom() - tall),
                    size(view.size.width, tall),
                );
                window.paint_quad(gpui::fill(
                    bottom,
                    linear_gradient(
                        180.0,
                        linear_color_stop(clear, 0.0),
                        linear_color_stop(solid, 1.0),
                    ),
                ));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
