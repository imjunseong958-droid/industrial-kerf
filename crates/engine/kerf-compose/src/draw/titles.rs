//! Titles and captions as they are drawn: a title's text in its font and face, where it reaches as
//! its reveal moves its pieces, drawn whole or piece by piece as the reveal brings them in, and a
//! caption burned in; with all that goes into each picture, to keep it by.

use super::*;

/// All that goes into what [`title_on`] draws on `run`, to keep that picture by. Most titles have no
/// keys and come to the same each frame, as a reveal does once it is over; one whose numbers move has
/// a key of its own at each frame, which is never kept.
pub(super) fn title_key(title: &TitlePlan, run: &SourceRun) -> String {
    use std::fmt::Write;
    let ((w, h), (across, down), time) = (run.size, run.beyond, run.at);
    let mut key = format!(
        "title|{w}x{h}+{across}+{down}|{:?}|{:?}|{:?}|{:x}|{:?}|{:?}",
        title.text,
        title.font,
        title.style,
        title.size.to_bits(),
        title.color,
        title.justify
    );
    let mut value = |param: &Param| {
        let _ = write!(key, "|{:x}", number(&param.value_at(time)).to_bits());
    };
    for (_, param) in &title.axes {
        value(param);
    }
    if let Some(reveal) = &title.reveal {
        value(&reveal.reveal);
        value(&reveal.soft);
        value(&reveal.from[0]);
        value(&reveal.from[1]);
        value(&reveal.scale);
        value(&reveal.rotation);
        value(&reveal.opacity);
    }
    for (tag, _) in &title.axes {
        let _ = write!(key, "|{tag:?}");
    }
    let _ = write!(key, "|{:?}", title.features);
    if let Some(reveal) = &title.reveal {
        let _ = write!(key, "|{}", reveal.by_word);
    }
    key
}

/// A title's variable font's axes at `time`. At 0, an axis is left as the font has it.
pub(super) fn title_axes(title: &TitlePlan, time: Time) -> Vec<([u8; 4], f32)> {
    title
        .axes
        .iter()
        .map(|(tag, param)| (*tag, number(&param.value_at(time)) as f32))
        .filter(|(_, value)| *value != 0.0)
        .collect()
}

/// A title's text, on a canvas `height` pixels high.
pub(super) fn title_text<'a>(
    title: &'a TitlePlan,
    axes: &'a [([u8; 4], f32)],
    height: f64,
) -> Text<'a> {
    Text {
        text: &title.text,
        font: &title.font,
        style: &title.style,
        size: (title.size * height).max(1.0) as f32,
        color: title.color,
        justify: title.justify,
        outline: None,
        axes,
        features: &title.features,
    }
}

/// Where a title draws at clip time `time` on a canvas of `(w, h)` pixels: the box of its lines, and
/// as far as a reveal takes each of its pieces. None for one that draws nothing.
pub(super) fn title_extent(title: &TitlePlan, (w, h): (usize, usize), time: Time) -> Option<Rect> {
    let axes = title_axes(title, time);
    let text = title_text(title, &axes, h as f64);
    let [left, top, right, bottom] = text::extent(&text, [w as f32 / 2.0, h as f32 / 2.0], 0.5)?;
    let [mut left, mut top, mut right, mut bottom] = [left, top, right, bottom].map(f64::from);
    let em = f64::from(text.size);
    // A glyph reaches a little past its line, as an italic's does. A reveal moves each piece of it as
    // far as its `from_` numbers say, and grows and turns it about its own middle.
    let mut around = em / 2.0;
    if let Some(reveal) = &title.reveal {
        let read = |param: &Param| number(&param.value_at(time));
        let (x, y) = (
            read(&reveal.from[0]) * h as f64,
            read(&reveal.from[1]) * h as f64,
        );
        (left, right) = (left + x.min(0.0), right + x.max(0.0));
        (top, bottom) = (top + y.min(0.0), bottom + y.max(0.0));
        around += em * read(&reveal.scale).abs().max(1.0);
    }
    Some(Rect::new(
        left - around,
        top - around,
        right - left + 2.0 * around,
        bottom - top + 2.0 * around,
    ))
}

/// A title drawn on a transparent picture of the canvas of `run` and as far past it as that says.
pub(super) fn title_on(title: &TitlePlan, run: &SourceRun) -> Image {
    let (w, h) = run.picture();
    let mut image = Image::new(w, h);
    let time = run.at;
    let axes = title_axes(title, time);
    let text = title_text(title, &axes, run.size.1 as f64);
    let at = run.middle().map(|at| at as f32);
    let Some(reveal) = &title.reveal else {
        text::draw(&mut image, &text, at, 0.5);
        return image;
    };
    let read = |param: &Param| number(&param.value_at(time));
    let (progress, soft) = (read(&reveal.reveal), read(&reveal.soft).max(0.01));
    let down = run.size.1 as f64;
    let (from_x, from_y, from_scale, from_rotation, from_opacity) = (
        read(&reveal.from[0]) * down,
        read(&reveal.from[1]) * down,
        read(&reveal.scale),
        read(&reveal.rotation).to_radians(),
        read(&reveal.opacity),
    );
    let look = |piece: usize, pieces: usize| {
        let coming = ((progress * (pieces as f64 + soft) - piece as f64) / soft).clamp(0.0, 1.0);
        let along = coming * coming * (3.0 - 2.0 * coming);
        let away = 1.0 - along;
        text::Piece {
            offset: [(from_x * away) as f32, (from_y * away) as f32],
            scale: (from_scale + (1.0 - from_scale) * along) as f32,
            rotation: (from_rotation * away) as f32,
            opacity: (from_opacity + (1.0 - from_opacity) * along) as f32,
        }
    };
    text::draw_pieces(&mut image, &text, at, 0.5, reveal.by_word, &look);
    image
}

/// The size a caption's text is drawn at.
pub(super) fn caption_size(sequence: &SequencePlan, scale: f64) -> f64 {
    (f64::from(sequence.height) * 0.045).max(12.0) * scale
}

/// A caption's text, as it is burned in.
pub(super) fn caption_text(caption: &CaptionPlan, size: f64) -> Text<'_> {
    Text {
        text: &caption.text,
        font: "sans-serif",
        style: "",
        size: size as f32,
        color: Rgba::WHITE,
        justify: Justify::Center,
        outline: Some((
            Rgba {
                a: 217,
                ..Rgba::BLACK
            },
            (size * 0.06) as f32,
        )),
        axes: &[],
        features: &[],
    }
}

/// All that goes into what [`caption_on`] draws, to keep that picture by.
pub(super) fn caption_key(
    caption: &CaptionPlan,
    sequence: &SequencePlan,
    scale: f64,
    (w, h): (usize, usize),
) -> String {
    let size = caption_size(sequence, scale);
    let text = caption_text(caption, size);
    format!(
        "caption|{w}x{h}|{:?}|{:?}|{:x}|{:?}|{:?}|{:?}",
        text.text,
        text.font,
        text.size.to_bits(),
        text.color,
        text.justify,
        text.outline
    )
}

/// A caption drawn on a transparent `w` × `h` picture.
pub(super) fn caption_on(
    caption: &CaptionPlan,
    sequence: &SequencePlan,
    scale: f64,
    (w, h): (usize, usize),
) -> Image {
    let mut image = Image::new(w, h);
    let text = caption_text(caption, caption_size(sequence, scale));
    text::draw(&mut image, &text, [w as f32 / 2.0, h as f32 * 0.95], 1.0);
    image
}
