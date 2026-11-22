//! Words in the editor's type: a label, a line cut with "…" where its room
//! ends, and the element that cuts it.

use crate::theme::{self, TypeStyle};
use gpui::{
    div, point, prelude::*, px, size, App, Bounds, ElementId, Pixels, Rgba, SharedString, TextRun,
    Window,
};

/// Words in a type style and colour.
pub fn label(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    theme::type_style(div(), style)
        .text_color(color)
        .child(words.into())
}

/// Words that keep to one line, cut with an ellipsis where the room they
/// are given ends ([`cut_line`]).
pub fn line(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    theme::type_style(div(), style)
        .text_color(color)
        .whitespace_nowrap()
        .overflow_hidden()
        .child(cut_line(words))
}

/// One line of words in the type and colour of what holds it, as wide as
/// they are, cut with "…" where the room it is laid out in ends (and set
/// in it as its `text_align` says). GPUI's own `text_ellipsis` keeps the
/// size it measured first, with no room given, and so never cuts.
pub fn cut_line(words: impl Into<SharedString>) -> CutLine {
    let words: SharedString = words.into();
    CutLine(if words.contains('\n') {
        words.replace('\n', " ").into()
    } else {
        words
    })
}

/// See [`cut_line`].
pub struct CutLine(SharedString);

impl IntoElement for CutLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The type of what holds a [`CutLine`]: its size, its line's height and
/// the run of its words.
fn cut_line_type(words: &str, window: &Window) -> (Pixels, Pixels, TextRun, gpui::Font) {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    (
        font_size,
        line_height,
        style.to_run(words.len()),
        style.font(),
    )
}

impl gpui::Element for CutLine {
    type RequestLayoutState = ();
    type PrepaintState = Option<(gpui::ShapedLine, Pixels)>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let (font_size, line_height, run, _) = cut_line_type(&self.0, window);
        let width = window
            .text_system()
            .shape_line(self.0.clone(), font_size, &[run], None)
            .width
            .ceil();
        // As wide as its words unless given less; a flex item can be
        // given none.
        let mut style = gpui::Style::default();
        style.min_size.width = px(0.0).into();
        let layout = window.request_measured_layout(style, move |known, _, _, _| {
            size(
                known.width.unwrap_or(width),
                known.height.unwrap_or(line_height),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let (font_size, line_height, run, font) = cut_line_type(&self.0, window);
        let mut runs = vec![run];
        let words = cx
            .text_system()
            .line_wrapper(font, font_size)
            .truncate_line(self.0.clone(), bounds.size.width, "…", &mut runs);
        let shaped = window
            .text_system()
            .shape_line(words, font_size, &runs, None);
        Some((shaped, line_height))
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        shaped: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((shaped, line_height)) = shaped.as_ref() else {
            return;
        };
        let room = (bounds.size.width - shaped.width).max(Pixels::ZERO);
        let left = match window.text_style().text_align {
            gpui::TextAlign::Left => Pixels::ZERO,
            gpui::TextAlign::Center => room / 2.0,
            gpui::TextAlign::Right => room,
        };
        if let Err(error) = shaped.paint(
            point(bounds.origin.x + left, bounds.origin.y),
            *line_height,
            window,
            cx,
        ) {
            log::warn!("a line of text did not draw: {error}");
        }
    }
}
