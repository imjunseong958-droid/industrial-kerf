//! Titles and captions, laid out and drawn by FilmCraft's text engine:
//! OpenType shaping, line breaks at `\n`, and antialiased glyphs whose
//! coverage is linear, so they composite correctly in linear light.
//! A character the font lacks is drawn by an installed font that has it,
//! and emoji in their own colours, over the fill (`kerf_text::color`).

use crate::color;
use crate::image::Image;
use kerf_model::Rgba;
use kerf_text::mask::{stroke, StrokeKind};
use kerf_text::color::ColorImage;
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
    /// A family, or `sans-serif`, `serif`, or `monospace`, or a face's own
    /// name ("Inter Black"). One the machine lacks is drawn in Inter.
    pub font: &'a str,
    /// The face of the family: a style name it has (`Bold`, `Black
    /// Italic`), or a weight from 100 to 900 with or without `Italic`.
    /// Empty for the face `font` names, or the family's Regular.
    pub style: &'a str,
    /// The font size, in pixels.
    pub size: f32,
    pub color: Rgba,
    pub justify: Justify,
    /// An edge around the glyphs, under them: its color and how far it
    /// reaches past them, in pixels.
    pub outline: Option<(Rgba, f32)>,
    /// Where to set a variable font's axes.
    pub axes: &'a [kerf_text::fonts::Axis],
    /// The font's OpenType features to turn on or off, over what it has on by itself.
    pub features: &'a [kerf_text::fonts::Feature],
}

/// Draw `text` over `image`, its lines placed against `at`'s x by its
/// justification, and `place` of the block's height above `at`'s y: a half
/// centers the block on it, 1 puts its bottom there.
pub fn draw(image: &mut Image, text: &Text, at: [f32; 2], place: f32) {
    if text.text.trim().is_empty() || text.size <= 0.0 {
        return;
    }
    let (style, paragraph) = styles(text);
    let laid = layout(text.text, &style, &paragraph);
    let [_, top, _, bottom] = laid.bounds;
    let baseline = at[1] - (top + (bottom - top) * place);
    let m = render::at(at[0], baseline);
    let (glyphs, x, y) = render::rasterize(&laid, &m);
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
    fill(image, &laid, &m, glyphs, (x, y), text.color, 1.0);
}

/// The character and paragraph styles `text` is laid out in.
fn styles(text: &Text) -> (TextStyle, ParagraphStyle) {
    let family = GENERIC
        .iter()
        .find(|(generic, _)| generic.eq_ignore_ascii_case(text.font.trim()))
        .map_or(text.font.trim(), |(_, family)| family);
    let style = TextStyle {
        family: if family.is_empty() { "Inter" } else { family }.to_owned(),
        style: text.style.trim().to_owned(),
        size: text.size,
        axes: text.axes.to_vec(),
        features: text.features.to_vec(),
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
    (style, paragraph)
}

/// Fill the glyphs of `laid`, whose coverage `glyphs` is placed at `at`, in `color`, and draw
/// its colour glyphs (emoji) over that in their own colours, all at `opacity`.
fn fill(
    image: &mut Image,
    laid: &kerf_text::Layout,
    m: &kerf_text::Xform,
    mut glyphs: Mask,
    (x, y): (i32, i32),
    color: Rgba,
    opacity: f32,
) {
    let alpha = f32::from(color.a) / 255.0 * opacity.clamp(0.0, 1.0);
    let foreground = [
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        1.0,
    ];
    let colors = render::rasterize_color(laid, m, foreground);
    if let Some((colors, _, _)) = &colors {
        // The fill shows only where no colour glyph covers.
        for (covered, painted) in glyphs.a.iter_mut().zip(&colors.px) {
            *covered = (*covered - painted[3]).max(0.0);
        }
    }
    let shown = Rgba {
        a: (alpha * 255.0).round() as u8,
        ..color
    };
    cover(image, &glyphs, x, y, shown);
    if let Some((colors, cx, cy)) = colors {
        paint(image, &colors, cx, cy, alpha);
    }
}

/// Draw `colors` (premultiplied, sRGB-encoded), placed at `x`, `y`, over `image` in linear light,
/// at `opacity`.
fn paint(image: &mut Image, colors: &ColorImage, x: i32, y: i32, opacity: f32) {
    let w = image.w;
    image
        .px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(row_y, row)| {
            let cy = row_y as i32 - y;
            if cy < 0 || cy >= colors.h as i32 {
                return;
            }
            let from = x.max(0);
            let to = (x + colors.w as i32).min(w as i32);
            for px in from..to {
                let p = colors.px[cy as usize * colors.w + (px - x) as usize];
                if p[3] <= 0.0 {
                    continue;
                }
                let a = (p[3] * opacity).min(1.0);
                let paint = [
                    color::to_linear(p[0] / p[3]) * a,
                    color::to_linear(p[1] / p[3]) * a,
                    color::to_linear(p[2] / p[3]) * a,
                    a,
                ];
                let pixel = &mut row[px as usize * 4..px as usize * 4 + 4];
                for k in 0..4 {
                    pixel[k] = paint[k] + pixel[k] * (1.0 - a);
                }
            }
        });
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
    let (style, paragraph) = styles(text);
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
        fill(image, &single, &m, mask, (x, y), text.color, piece.opacity);
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
            style: "",
            size: 40.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
            axes: &[],
            features: &[],
        }
    }

    fn ink(image: &Image) -> f32 {
        image.px.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).sum()
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
            style: "",
            size: 48.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
            axes: &[],
            features: &[],
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
                style: "",
                size: 30.0,
                color: Rgba::WHITE,
                justify,
                outline: None,
                axes: &[],
                features: &[],
            };
            draw(&mut image, &text, [200.0, 50.0], 0.5);
            covered(&image)
        };
        assert!(draw_at(Justify::Left).0 >= 199);
        assert!(draw_at(Justify::Right).2 <= 201);
    }

    /// The colour test font of `kerf-text`: U+10FFFC is a red box with an inner box in the
    /// text's own colour.
    struct ColorFont;

    impl kerf_text::fonts::FontSource for ColorFont {
        fn faces(&self) -> Vec<kerf_text::fonts::FaceInfo> {
            vec![kerf_text::fonts::FaceInfo {
                family: "Kerf Test COLR".into(),
                style: "Regular".into(),
                weight: 400,
                italic: false,
                index: 0,
                data: kerf_text::fonts::FaceData::Static(include_bytes!(
                    "../../../foundation/kerf-text/testdata/KerfTestColr.ttf"
                )),
                origin: "test",
                names: Vec::new(),
                color: true,
            }]
        }
    }

    #[test]
    fn colour_glyphs_keep_their_colours_over_the_fill() {
        kerf_text::fonts::add_source(&ColorFont);
        let draw_in = |color: Rgba| {
            let mut image = Image::new(300, 120);
            let text = Text {
                text: "H\u{10FFFC}",
                font: "Kerf Test COLR",
                style: "",
                size: 80.0,
                color,
                justify: Justify::Center,
                outline: Some((Rgba::BLACK, 3.0)),
                axes: &[],
                features: &[],
            };
            draw(&mut image, &text, [150.0, 60.0], 0.5);
            image
        };
        let green = Rgba {
            r: 0,
            g: 255,
            b: 0,
            a: 255,
        };
        let image = draw_in(green);
        let count = |test: &dyn Fn(&[f32]) -> bool| {
            image
                .px
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| test(&p[..]))
                .count()
        };
        // The H falls back to Inter and is filled green; the box is red, with a green middle,
        // the text's colour, and nothing of the fill shows through the red.
        let red = count(&|p| p[0] > 0.95 && p[1] < 0.05 && p[3] > 0.99);
        let filled = count(&|p| p[1] > 0.95 && p[0] < 0.05);
        assert!(red > 1000, "{red}");
        assert!(filled > 600, "{filled}");
        assert_eq!(count(&|p| p[0] > 0.2 && p[1] > 0.2), 0);
        // Half the text's alpha, half the emoji's.
        let half = draw_in(Rgba { a: 128, ..green });
        let most = half
            .px
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .fold(0.0f32, f32::max);
        assert!((most - 0.5).abs() < 0.03, "{most}");
    }

    #[test]
    fn outlines_reach_past_the_glyphs() {
        let mut plain = Image::new(300, 100);
        let mut edged = Image::new(300, 100);
        let text = Text {
            text: "Hi",
            font: "sans-serif",
            style: "",
            size: 40.0,
            color: Rgba::WHITE,
            justify: Justify::Center,
            outline: None,
            axes: &[],
            features: &[],
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
