//! Small marks in the editor's tokens: a hairline, a chip of words, a filled
//! dot, a thin progress bar, and a colour as GPUI's Hsla.

use crate::theme::{self, text};
use gpui::{div, prelude::*, px, Hsla, Rgba, SharedString};

/// A hairline, across or down.
pub fn hairline(across: bool) -> gpui::Div {
    let line = div().flex_none().bg(theme::line());
    if across {
        line.h(px(1.0)).w_full()
    } else {
        line.w(px(1.0)).h_full()
    }
}

/// A small rounded chip of words.
pub fn chip(words: impl Into<SharedString>, color: Rgba) -> gpui::Div {
    theme::type_style(div(), text::META)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::space::S4))
        .h(px(20.0))
        .px(px(theme::space::S8))
        .rounded(px(10.0))
        .bg(theme::raised())
        .text_color(color)
        .child(words.into())
}

/// A filled dot, as a status says.
pub fn dot(color: Rgba, size: f32) -> gpui::Div {
    div().flex_none().size(px(size)).rounded_full().bg(color)
}

/// A thin progress bar, 0..1.
pub fn progress(amount: f64, color: Rgba) -> gpui::Div {
    let amount = amount.clamp(0.0, 1.0) as f32;
    div()
        .h(px(3.0))
        .w_full()
        .rounded_full()
        .bg(theme::track())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(gpui::relative(amount)),
        )
}

/// A colour as GPUI's Hsla.
pub fn hsla(color: Rgba) -> Hsla {
    color.into()
}
