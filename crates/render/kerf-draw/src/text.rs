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

/// How a piece of text looks at a moment, against how it would rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// Moved by this many pixels.
    pub offset: [f32; 2],
    /// Scaled and turned about its own middle, clockwise, in radians.
    pub scale: f32,
    pub rotation: f32,
    /// 0 to 1.
    pub opacity: f32,
}

/// Draw `text` over `image` as [`draw`] does, with each glyph looking as `look` says. `look` gets which
/// piece the glyph is of how many, counting characters, or words when `by_word`.
pub fn draw_pieces(
    image: &mut Image,
    text: &Text,
    at: [f32; 2],
    place: f32,
    by_word: bool,
    look: &dyn Fn(usize, usize) -> Piece,
) {
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
    // Which piece each glyph is of: the words or characters that came before where it sets.
    let piece_of = |cluster: usize| -> usize {
        let before = &text.text[..cluster.min(text.text.len())];
        if by_word {
            before.split_whitespace().count()
                - usize::from(!before.is_empty() && !before.ends_with(char::is_whitespace))
        } else {
            before.chars().filter(|c| !c.is_whitespace()).count()
        }
    };
    let pieces = if by_word {
        text.text.split_whitespace().count()
    } else {
        text.text.chars().filter(|c| !c.is_whitespace()).count()
    }
    .max(1);
    for glyph in &laid.glyphs {
        let piece = look(piece_of(glyph.cluster), pieces);
        if piece.opacity <= 0.0 || piece.scale == 0.0 {
            continue;
        }
        // Rasterizing reads the room to leave from the bounds: an em and a half around its pen.
        let em = glyph.size;
        let single = kerf_text::layout::Layout {
            glyphs: vec![*glyph],
            bounds: [
                glyph.x - em * 0.5,
                glyph.y - em * 1.2,
                glyph.x + em * 1.5,
                glyph.y + em * 0.5,
            ],
            ..Default::default()
        };
        // About the glyph's own middle: half its size along the line, and half its height up.
        let (cx, cy) = (glyph.x + glyph.size * 0.25, glyph.y - glyph.size * 0.35);
        let (sin, cos) = piece.rotation.sin_cos();
        let (a, b, c, d) = (
            cos * piece.scale,
            sin * piece.scale,
            -sin * piece.scale,
            cos * piece.scale,
        );
        let m: kerf_text::raster::Xform = [
            a,
            b,
            c,
            d,
            at[0] + piece.offset[0] + cx - (a * cx + c * cy),
            baseline + piece.offset[1] + cy - (b * cx + d * cy),
        ];
        let (mask, x, y) = render::rasterize(&single, &m);
        let color = Rgba {
            a: (f32::from(text.color.a) * piece.opacity.min(1.0)).round() as u8,
            ..text.color
        };
        cover(image, &mask, x, y, color);
    }
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

    fn rest(_: usize, _: usize) -> Piece {
        Piece {
            offset: [0.0, 0.0],
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
        }
    }

    fn sample<'a>() -> Text<'a> {
        Text {
            text: "Kerf it up",
            font: "sans-serif",
            size: 40.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
        }
    }

    fn ink(image: &Image) -> f32 {
        image.px.chunks_exact(4).map(|pixel| pixel[3]).sum()
    }

    #[test]
    fn pieces_at_rest_are_what_plain_text_is() {
        let mut plain = Image::new(400, 100);
        draw(&mut plain, &sample(), [200.0, 50.0], 0.5);
        let mut pieces = Image::new(400, 100);
        draw_pieces(&mut pieces, &sample(), [200.0, 50.0], 0.5, false, &rest);
        let off: f32 = plain
            .px
            .iter()
            .zip(&pieces.px)
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(off < ink(&plain) * 0.02, "{off} of {}", ink(&plain));
    }

    #[test]
    fn pieces_count_characters_or_words_without_the_spaces() {
        let seen = std::sync::Mutex::new(Vec::new());
        let look = |piece: usize, pieces: usize| {
            seen.lock().unwrap().push((piece, pieces));
            rest(piece, pieces)
        };
        let mut image = Image::new(400, 100);
        draw_pieces(&mut image, &sample(), [200.0, 50.0], 0.5, false, &look);
        let by_char = std::mem::take(&mut *seen.lock().unwrap());
        assert_eq!(by_char.first(), Some(&(0, 8)));
        assert_eq!(by_char.iter().map(|(piece, _)| *piece).max(), Some(7));
        draw_pieces(&mut image, &sample(), [200.0, 50.0], 0.5, true, &look);
        let by_word = std::mem::take(&mut *seen.lock().unwrap());
        // K e r f and the space after, then i t and its space, then u p.
        let words: Vec<usize> = by_word.iter().map(|(piece, _)| *piece).collect();
        assert_eq!(words, [0, 0, 0, 0, 0, 1, 1, 1, 2, 2]);
        assert_eq!(by_word.first().map(|(_, pieces)| *pieces), Some(3));
    }

    #[test]
    fn a_piece_moves_fades_and_is_left_out_when_it_cannot_be_seen() {
        let moved = |piece: Piece| {
            let mut image = Image::new(400, 200);
            draw_pieces(&mut image, &sample(), [200.0, 100.0], 0.5, false, &|_, _| piece);
            image
        };
        let base = rest(0, 1);
        let at_rest = moved(base);
        let shifted = moved(Piece {
            offset: [0.0, 40.0],
            ..base
        });
        let (_, y0, _, y1) = covered(&at_rest);
        let (_, sy0, _, sy1) = covered(&shifted);
        assert_eq!((sy0 as i32 - y0 as i32, sy1 as i32 - y1 as i32), (40, 40));
        let half = moved(Piece {
            opacity: 0.5,
            ..base
        });
        assert!((ink(&half) / ink(&at_rest) - 0.5).abs() < 0.02);
        assert_eq!(
            ink(&moved(Piece {
                opacity: 0.0,
                ..base
            })),
            0.0
        );
        let small = moved(Piece { scale: 0.5, ..base });
        assert!((ink(&small) / ink(&at_rest) - 0.25).abs() < 0.05);
        let turned = moved(Piece {
            rotation: 0.3,
            ..base
        });
        assert_ne!(turned, at_rest);
        assert!((ink(&turned) / ink(&at_rest) - 1.0).abs() < 0.05);
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
