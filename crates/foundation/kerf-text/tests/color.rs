//! Colour glyphs, drawn from the small colour fonts in `testdata` (see `make_color_fonts.py`):
//! COLR v0 layers in palette and text colour, a COLR v1 gradient, and a CBDT PNG; and an emoji
//! in an Inter text, which falls back to a colour face.

use kerf_text::color::ColorImage;
use kerf_text::fonts::{self, FaceData, FaceInfo, FontSource};
use kerf_text::{ParagraphStyle, TextStyle, layout, render};

const COLR: &str = "Kerf Test COLR";
const BITMAP: &str = "Kerf Test Bitmap";

struct ColorFonts;

impl FontSource for ColorFonts {
    fn faces(&self) -> Vec<FaceInfo> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let face = |family: &str, file: &str| FaceInfo {
            family: family.into(),
            style: "Regular".into(),
            weight: 400,
            italic: false,
            index: 0,
            data: FaceData::File(dir.join(file)),
            origin: "test",
            names: Vec::new(),
            color: true,
        };
        vec![face(COLR, "KerfTestColr.ttf"), face(BITMAP, "KerfTestBitmap.ttf")]
    }
}

/// `text` in `family` at 100 pixels to the em, its pen at (10, 100): its colours, and its coverage
/// in the same place.
fn drawn(family: &str, text: &str) -> (ColorImage, Vec<f32>, i32, i32) {
    fonts::add_source(&ColorFonts);
    let laid = layout(text, &TextStyle { family: family.into(), size: 100.0, ..TextStyle::default() }, &ParagraphStyle::default());
    assert!(laid.glyphs.iter().all(|g| g.id != 0), "{:?}", laid.glyphs);
    let m = render::at(10.0, 100.0);
    let (mask, mx, my) = render::rasterize(&laid, &m);
    let (image, x, y) = render::rasterize_color(&laid, &m, [1.0, 1.0, 1.0, 1.0]).expect("colour glyphs");
    assert_eq!((mx, my, mask.w, mask.h), (x, y, image.w, image.h));
    (image, mask.a, x, y)
}

fn at(image: &ColorImage, x0: i32, y0: i32, x: i32, y: i32) -> [f32; 4] {
    image.px[(y - y0) as usize * image.w + (x - x0) as usize]
}

fn near(found: [f32; 4], wanted: [f32; 4]) {
    assert!(found.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.02), "{found:?} against {wanted:?}");
}

#[test]
fn colr_layers_draw_in_the_palette_and_the_text_colour() {
    // The box runs from x 20 to 80 and y 40 to 100; the inner box from 40 to 60 and 60 to 80.
    let (image, cover, x0, y0) = drawn(COLR, "\u{10FFFC}");
    near(at(&image, x0, y0, 25, 45), [1.0, 0.0, 0.0, 1.0]);
    near(at(&image, x0, y0, 50, 70), [1.0, 1.0, 1.0, 1.0]);
    near(at(&image, x0, y0, 5, 45), [0.0; 4]);
    // What covers is what the colours do.
    let i = (45 - y0) as usize * image.w + (25 - x0) as usize;
    assert!((cover[i] - 1.0).abs() < 0.01);
    let ink: f32 = cover.iter().sum();
    assert!((ink - 3600.0).abs() < 60.0, "{ink}");
}

#[test]
fn a_colr_v1_gradient_runs_from_red_to_blue() {
    let (image, _, x0, y0) = drawn(COLR, "\u{10FFFB}");
    let (left, middle, right) = (at(&image, x0, y0, 21, 70), at(&image, x0, y0, 50, 70), at(&image, x0, y0, 79, 70));
    assert!(left[0] > 0.95 && left[2] < 0.05, "{left:?}");
    assert!(right[2] > 0.95 && right[0] < 0.05, "{right:?}");
    assert!((middle[0] - 0.5).abs() < 0.05 && (middle[2] - 0.5).abs() < 0.05, "{middle:?}");
}

#[test]
fn a_cbdt_bitmap_is_drawn_from_its_png() {
    // 8 pixels at 8 to the em, so an em of 100 is 12.5 times as large: from x 10 to 110, its top
    // on the ascender (y 0) and its foot on the baseline. The lower right quarter is blue.
    let (image, cover, x0, y0) = drawn(BITMAP, "\u{10FFFD}");
    near(at(&image, x0, y0, 30, 20), [1.0, 0.0, 0.0, 1.0]);
    near(at(&image, x0, y0, 90, 80), [0.0, 0.0, 1.0, 1.0]);
    near(at(&image, x0, y0, 30, 80), [1.0, 0.0, 0.0, 1.0]);
    let ink: f32 = cover.iter().sum();
    assert!((ink - 10_000.0).abs() < 300.0, "{ink}");
}

#[test]
fn a_bitmap_turns_and_shrinks_with_the_transform() {
    fonts::add_source(&ColorFonts);
    let laid = layout("\u{10FFFD}", &TextStyle { family: BITMAP.into(), size: 100.0, ..TextStyle::default() }, &ParagraphStyle::default());
    let (sin, cos) = 0.5f32.sin_cos();
    let turned = [cos * 0.5, sin * 0.5, -sin * 0.5, cos * 0.5, 100.0, 100.0];
    let (image, _, _) = render::rasterize_color(&laid, &turned, [1.0; 4]).expect("colour");
    let alpha: f32 = image.px.iter().map(|p| p[3]).sum();
    // A quarter of the area.
    assert!((alpha - 2500.0).abs() < 150.0, "{alpha}");
}

#[test]
fn an_emoji_in_inter_falls_back_to_a_colour_face() {
    fonts::add_source(&ColorFonts);
    let laid = layout("Hi \u{1F480}", &TextStyle { size: 60.0, ..TextStyle::default() }, &ParagraphStyle::default());
    let skull = laid.glyphs.last().expect("glyphs");
    assert_ne!(skull.id, 0);
    let face = fonts::face(skull.face);
    assert!(face.info.color && face.covers('\u{1F480}'), "{}", face.info.family);
    assert!(render::rasterize_color(&laid, &render::at(0.0, 80.0), [1.0; 4]).is_some());
    // The letters stay in Inter.
    assert_eq!(fonts::face(laid.glyphs[0].face).info.family, "Inter");
}

