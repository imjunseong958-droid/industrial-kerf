//! Titles and captions, laid out and drawn by FilmCraft's text engine:
//! OpenType shaping, line breaks at `\n`, and antialiased glyphs whose
//! coverage is linear, so they composite correctly in linear light.

use crate::color;
use crate::image::Image;
use kerf_model::Rgba;
use kerf_text::mask::{stroke, StrokeKind};
use kerf_text::raster::Mask;
use kerf_text::{layout, render, Align, ParagraphStyle, TextStyle};
use rayon::prelude::*;

/// The families generic font names stand for, all bundled.
const GENERIC: &[(&str, &str)] = &[
    ("sans-serif", "Inter"),
    ("serif", "Noto Serif"),
    ("monospace", "JetBrains Mono"),
];

/// Where lines sit against the point they are drawn at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    /// Lines begin at the point.
    Left,
    /// Lines are centered on it.
    #[default]
    Center,
    /// Lines end at it.
    Right,
}

/// Text to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Text<'a> {
    pub text: &'a str,
    /// A family, or `sans-serif`, `serif`, or `monospace`. One the machine
    /// lacks is drawn in Inter.
    pub font: &'a str,
    /// The font size, in pixels.
    pub size: f32,
    pub color: Rgba,
    pub justify: Justify,
    /// An edge around the glyphs, under them: its color and how far it
    /// reaches past them, in pixels.
    pub outline: Option<(Rgba, f32)>,
}

/// Draw `text` over `image`, its lines placed against `at`'s x by its
/// justification, and `place` of the block's height above `at`'s y: a half
/// centers the block on it, 1 puts its bottom there.
pub fn draw(image: &mut Image, text: &Text, at: [f32; 2], place: f32) {
    if text.text.trim().is_empty() || text.size <= 0.0 {
        return;
    }
    let family = GENERIC
        .iter()
        .find(|(generic, _)| generic.eq_ignore_ascii_case(text.font.trim()))
        .map_or(text.font.trim(), |(_, family)| family);
    let style = TextStyle {
        family: if family.is_empty() { "Inter" } else { family }.to_owned(),
        size: text.size,
        ..TextStyle::default()
    };
    let paragraph = ParagraphStyle {
        align: match text.justify {
            Justify::Left => Align::Left,
            Justify::Center => Align::Center,
            Justify::Right => Align::Right,
        },
        ..ParagraphStyle::default()
    };
    let laid = layout(text.text, &style, &paragraph);
    let [_, top, _, bottom] = laid.bounds;
    let baseline = at[1] - (top + (bottom - top) * place);
    let (glyphs, x, y) = render::rasterize(&laid, &render::at(at[0], baseline));
    if let Some((edge, width)) = text.outline.filter(|(_, width)| *width > 0.0) {
        let margin = width.ceil() as i32 + 2;
        let mut room = Mask::new(
            glyphs.w + 2 * margin as usize,
            glyphs.h + 2 * margin as usize,
        );
        room.add(&glyphs, margin, margin);
        let outline = stroke(&room, width, StrokeKind::Outer);
        cover(image, &outline, x - margin, y - margin, edge);
    }
    cover(image, &glyphs, x, y, text.color);
}

/// Draw `color` over `image` where `mask`, placed at `x`, `y`, covers.
fn cover(image: &mut Image, mask: &Mask, x: i32, y: i32, color: Rgba) {
    let decode = color::decode_table();
    let alpha = f32::from(color.a) / 255.0;
    let paint = [
        decode[usize::from(color.r)] * alpha,
        decode[usize::from(color.g)] * alpha,
        decode[usize::from(color.b)] * alpha,
        alpha,
    ];
    let w = image.w;
    image
        .px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(row_y, row)| {
            let my = row_y as i32 - y;
            if my < 0 || my >= mask.h as i32 {
                return;
            }
            let from = x.max(0);
            let to = (x + mask.w as i32).min(w as i32);
            for px in from..to {
                let covered = mask.a[my as usize * mask.w + (px - x) as usize];
                if covered <= 0.0 {
                    continue;
                }
                let pixel = &mut row[px as usize * 4..px as usize * 4 + 4];
                let keep = 1.0 - paint[3] * covered;
                for k in 0..4 {
                    pixel[k] = paint[k] * covered + pixel[k] * keep;
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(image: &Image) -> (usize, usize, usize, usize) {
        let mut bounds = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..image.h {
            for x in 0..image.w {
                if image.get(x, y)[3] > 0.5 {
                    bounds = (
                        bounds.0.min(x),
                        bounds.1.min(y),
                        bounds.2.max(x),
                        bounds.3.max(y),
                    );
                }
            }
        }
        bounds
    }

    #[test]
    fn titles_center_on_the_point() {
        let mut image = Image::new(400, 200);
        let text = Text {
            text: "Day One",
            font: "sans-serif",
            size: 48.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
        };
        draw(&mut image, &text, [200.0, 100.0], 0.5);
        let (x0, y0, x1, y1) = covered(&image);
        assert!(((x0 + x1) as f32 / 2.0 - 200.0).abs() < 4.0, "{x0} {x1}");
        assert!(((y0 + y1) as f32 / 2.0 - 100.0).abs() < 12.0, "{y0} {y1}");
    }

    #[test]
    fn left_and_right_hang_from_the_point() {
        let draw_at = |justify| {
            let mut image = Image::new(400, 100);
            let text = Text {
                text: "Edge",
                font: "serif",
                size: 30.0,
                color: Rgba::WHITE,
                justify,
                outline: None,
            };
            draw(&mut image, &text, [200.0, 50.0], 0.5);
            covered(&image)
        };
        assert!(draw_at(Justify::Left).0 >= 199);
        assert!(draw_at(Justify::Right).2 <= 201);
    }

    #[test]
    fn outlines_reach_past_the_glyphs() {
        let mut plain = Image::new(300, 100);
        let mut edged = Image::new(300, 100);
        let text = Text {
            text: "Hi",
            font: "sans-serif",
            size: 40.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
        };
        draw(&mut plain, &text, [150.0, 50.0], 0.5);
        let outlined = Text {
            outline: Some((Rgba::BLACK, 4.0)),
            ..text
        };
        draw(&mut edged, &outlined, [150.0, 50.0], 0.5);
        let (a, b) = (covered(&plain), covered(&edged));
        assert!(b.0 + 3 <= a.0 && b.2 >= a.2 + 3, "{a:?} {b:?}");
    }
}
