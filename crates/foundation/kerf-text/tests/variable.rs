//! A variable font's axes, set on its way through shaping and drawing. `testdata/make_test_font.py`
//! says what the test font's numbers are.

use kerf_text::fonts::{self, Axis, FaceData, FaceInfo, FontSource};
use kerf_text::{layout, render, ParagraphStyle, TextStyle};

const FAMILY: &str = "Kerf Test Variable";

/// The same stem drawn in CFF2 outlines, with `wght` alone.
const CFF2: &str = "Kerf Test CFF2";

struct TestFont;

impl FontSource for TestFont {
    fn faces(&self) -> Vec<FaceInfo> {
        let face = |family: &str, bytes: &'static [u8]| FaceInfo {
            family: family.into(),
            style: "Regular".into(),
            weight: 400,
            italic: false,
            index: 0,
            data: FaceData::Static(bytes),
            origin: "test",
        };
        vec![
            face(FAMILY, include_bytes!("../testdata/KerfTestVariable.ttf")),
            face(CFF2, include_bytes!("../testdata/KerfTestCff2.otf")),
        ]
    }
}

const WGHT: [u8; 4] = *b"wght";
const WDTH: [u8; 4] = *b"wdth";
const OPSZ: [u8; 4] = *b"opsz";
const SLNT: [u8; 4] = *b"slnt";
const ITAL: [u8; 4] = *b"ital";

fn styled(family: &str, axes: &[Axis]) -> TextStyle {
    fonts::add_source(&TestFont);
    TextStyle { family: family.into(), size: 100.0, axes: axes.to_vec(), ..TextStyle::default() }
}

/// How much ink a capital I has, in pixels at 100 pixels to the em.
fn ink(family: &str, text: &str, axes: &[Axis]) -> f32 {
    let laid = layout(text, &styled(family, axes), &ParagraphStyle::default());
    let (mask, _, _) = render::rasterize(&laid, &render::at(200.0, 200.0));
    mask.a.iter().sum()
}

fn advance(family: &str, axes: &[Axis]) -> f32 {
    let laid = layout("II", &styled(family, axes), &ParagraphStyle::default());
    laid.glyphs[1].x - laid.glyphs[0].x
}

fn close(found: f32, wanted: f32) {
    assert!((found - wanted).abs() < wanted * 0.02, "{found} against {wanted}");
}

#[test]
fn weight_changes_the_ink_by_what_the_font_says() {
    // The stem is 100 units wide and 700 high: at 100 pixels to the em, 10 by 70.
    close(ink(FAMILY, "I", &[]), 700.0);
    // At the far ends of the axis it gains 200 and loses 60.
    close(ink(FAMILY, "I", &[(WGHT, 900.0)]), 30.0 * 70.0);
    close(ink(FAMILY, "I", &[(WGHT, 100.0)]), 4.0 * 70.0);
    // Halfway to the far end, half of that.
    close(ink(FAMILY, "I", &[(WGHT, 650.0)]), 20.0 * 70.0);
    // Past the axis's end, it stays there.
    close(ink(FAMILY, "I", &[(WGHT, 2000.0)]), 30.0 * 70.0);
}

#[test]
fn several_axes_add_up() {
    close(ink(FAMILY, "I", &[(WDTH, 200.0)]), 20.0 * 70.0);
    close(ink(FAMILY, "I", &[(WGHT, 900.0), (WDTH, 200.0)]), 40.0 * 70.0);
}

#[test]
fn an_axis_the_face_lacks_or_its_default_is_the_font_as_it_comes() {
    fonts::add_source(&TestFont);
    let face = kerf_text::resolve(FAMILY, "Regular").face;
    assert_eq!(fonts::instance(face, &[]), 0);
    assert_eq!(fonts::instance(face, &[(WGHT, 400.0)]), 0);
    assert_eq!(fonts::instance(face, &[(*b"GRAD", 50.0)]), 0);
    assert_ne!(fonts::instance(face, &[(WGHT, 500.0)]), 0);
    // The same setting is the same instance, and another is another.
    assert_eq!(fonts::instance(face, &[(WGHT, 500.0)]), fonts::instance(face, &[(WGHT, 500.001)]));
    assert_ne!(fonts::instance(face, &[(WGHT, 500.0)]), fonts::instance(face, &[(WGHT, 600.0)]));
    // A face with no axes is drawn as it was, whatever it is told.
    let inter = kerf_text::resolve("Inter", "Regular").face;
    assert_eq!(fonts::instance(inter, &[(WGHT, 900.0)]), 0);
    let plain = layout("Words", &styled("Inter", &[]), &ParagraphStyle::default());
    let told = layout("Words", &styled("Inter", &[(WGHT, 900.0)]), &ParagraphStyle::default());
    assert_eq!(plain.glyphs.len(), told.glyphs.len());
    for (a, b) in plain.glyphs.iter().zip(&told.glyphs) {
        assert_eq!((a.id, a.x, a.instance), (b.id, b.x, b.instance));
    }
    assert_eq!(render::rasterize(&plain, &render::at(0.0, 100.0)).0.a, render::rasterize(&told, &render::at(0.0, 100.0)).0.a);
}

#[test]
fn a_real_font_grows_and_widens_with_its_weight() {
    // On a Mac, one of the system's: its advances come from its own variation tables, as most
    // fonts' do, where the test font's are its outline's.
    let path = "/System/Library/Fonts/Supplemental/Skia.ttf";
    if !std::path::Path::new(path).exists() {
        eprintln!("no Skia here; skipping");
        return;
    }
    fonts::scan_system();
    let (light, heavy) = (ink("Skia", "Weight", &[(WGHT, 0.5)]), ink("Skia", "Weight", &[(WGHT, 2.5)]));
    assert!(heavy > light * 1.3, "{light} {heavy}");
    assert!(advance("Skia", &[(WGHT, 2.5)]) != advance("Skia", &[(WGHT, 0.5)]));
}

#[test]
fn the_advance_goes_with_the_weight_where_the_font_says_so() {
    // 300 units at rest and 500 at the heavy end, by the outline's phantom points.
    close(advance(FAMILY, &[]), 30.0);
    close(advance(FAMILY, &[(WGHT, 900.0)]), 50.0);
}

/// How far the top of an I's stem leans to the right of its foot, in pixels at 100 to the em: from
/// the middle of its ink on each row, two rows in from each end, carried on to its whole height.
fn lean(family: &str, axes: &[Axis]) -> f32 {
    let laid = layout("I", &styled(family, axes), &ParagraphStyle::default());
    let (mask, _, _) = render::rasterize(&laid, &render::at(200.0, 200.0));
    let (width, height) = (mask.w, mask.h);
    let row = |y: usize| &mask.a[y * width..(y + 1) * width];
    let middle = |y: usize| {
        let (sum, total) = row(y).iter().enumerate().fold((0.0, 0.0), |(sum, total), (x, a)| (sum + x as f32 * a, total + a));
        sum / total
    };
    let rows: Vec<usize> = (0..height).filter(|y| row(*y).iter().sum::<f32>() > 1.0).collect();
    let (top, foot) = (rows[2], rows[rows.len() - 3]);
    (middle(top) - middle(foot)) * 70.0 / (foot - top) as f32
}

#[test]
fn optical_size_changes_the_stem_on_its_scale() {
    // 50 units either way at its far ends, from a default that is neither the middle nor an end.
    close(ink(FAMILY, "I", &[(OPSZ, 144.0)]), 5.0 * 70.0);
    close(ink(FAMILY, "I", &[(OPSZ, 8.0)]), 15.0 * 70.0);
    close(advance(FAMILY, &[(OPSZ, 144.0)]), 25.0);
    close(advance(FAMILY, &[(OPSZ, 8.0)]), 35.0);
    assert_eq!(fonts::instance(kerf_text::resolve(FAMILY, "Regular").face, &[(OPSZ, 14.0)]), 0);
}

#[test]
fn slant_leans_the_stem_and_leaves_the_advance() {
    fonts::add_source(&TestFont);
    assert!(lean(FAMILY, &[]).abs() < 1.0, "{}", lean(FAMILY, &[]));
    close(lean(FAMILY, &[(SLNT, -20.0)]), 25.5);
    close(lean(FAMILY, &[(SLNT, -10.0)]), 12.75);
    close(advance(FAMILY, &[(SLNT, -20.0)]), 30.0);
    close(ink(FAMILY, "I", &[(SLNT, -20.0)]), 700.0);
}

#[test]
fn italic_leans_the_stem_and_takes_from_the_advance() {
    fonts::add_source(&TestFont);
    close(lean(FAMILY, &[(ITAL, 1.0)]), 12.0);
    close(advance(FAMILY, &[(ITAL, 1.0)]), 28.0);
    // All five at once add up, as their deltas do.
    let all = [(WGHT, 900.0), (WDTH, 200.0), (OPSZ, 144.0), (SLNT, -20.0), (ITAL, 1.0)];
    close(advance(FAMILY, &all), 30.0 + 20.0 + 10.0 - 5.0 - 2.0);
    close(lean(FAMILY, &all), 25.5 + 12.0);
}

#[test]
fn a_cff2_font_draws_and_shapes_at_its_axes() {
    fonts::add_source(&TestFont);
    let face = kerf_text::resolve(CFF2, "Regular");
    assert!(!face.missing);
    assert_eq!(
        fonts::face(face.face).axes(),
        [fonts::AxisRange { tag: WGHT, min: 100.0, default: 400.0, max: 900.0 }]
    );
    close(ink(CFF2, "I", &[]), 700.0);
    close(ink(CFF2, "I", &[(WGHT, 900.0)]), 30.0 * 70.0);
    close(ink(CFF2, "I", &[(WGHT, 100.0)]), 4.0 * 70.0);
    close(ink(CFF2, "I", &[(WGHT, 650.0)]), 20.0 * 70.0);
    // Its advances are `HVAR`'s, there being no outline phantom points to go by.
    close(advance(CFF2, &[]), 30.0);
    close(advance(CFF2, &[(WGHT, 900.0)]), 50.0);
    close(advance(CFF2, &[(WGHT, 100.0)]), 24.0);
    // An instance is a different picture of the same glyph, cached apart.
    assert_ne!(fonts::instance(face.face, &[(WGHT, 900.0)]), 0);
    assert_eq!(fonts::instance(face.face, &[(WGHT, 400.0)]), 0);
}

#[test]
fn the_test_font_says_what_axes_it_has() {
    fonts::add_source(&TestFont);
    let face = fonts::face(kerf_text::resolve(FAMILY, "Regular").face);
    let axis = |tag: &[u8; 4], min, default, max| fonts::AxisRange { tag: *tag, min, default, max };
    assert_eq!(
        face.axes(),
        [
            axis(b"wght", 100.0, 400.0, 900.0),
            axis(b"wdth", 50.0, 100.0, 200.0),
            axis(b"opsz", 8.0, 14.0, 144.0),
            axis(b"slnt", -20.0, 0.0, 0.0),
            axis(b"ital", 0.0, 0.0, 1.0),
        ]
    );
    assert!(fonts::faces_of("Inter").iter().all(|inter| inter.axes().is_empty()));
    assert!(fonts::faces_of("No Such Font Family 123").is_empty());
}

/// A face of the system's by its file, which it goes by where its family is hidden, with `tag`.
fn system_face(path: &str, tag: [u8; 4], text: &str) -> Option<std::sync::Arc<fonts::Face>> {
    if !std::path::Path::new(path).exists() {
        eprintln!("no {path} here; skipping");
        return None;
    }
    fonts::scan_system();
    let found = fonts::all_faces().into_iter().find(|face| {
        matches!(&face.info.data, FaceData::File(file) if file.as_path() == std::path::Path::new(path))
            && face.axes().iter().any(|axis| axis.tag == tag)
            && text.chars().all(|c| face.has_char(c))
    });
    if found.is_none() {
        eprintln!("{path} has no face with that axis for this text; skipping");
    }
    found
}

fn across(family: &str, text: &str, axes: &[Axis]) -> f32 {
    let laid = layout(text, &styled(family, axes), &ParagraphStyle::default());
    assert!(!laid.missing_font && laid.glyphs.iter().all(|glyph| glyph.id != 0), "{family}");
    laid.bounds[2] - laid.bounds[0]
}

/// A real font with optical sizes, where the system has one: what its outlines and advances do
/// comes from its own tables, at a default that is its far end.
#[test]
fn a_real_font_changes_with_its_optical_size() {
    let Some(face) = system_face("/System/Library/Fonts/NewYork.ttf", OPSZ, "Words") else { return };
    let family = face.info.family.clone();
    let opsz = face.axes().into_iter().find(|axis| axis.tag == OPSZ).unwrap();
    assert!(opsz.min < opsz.max, "{opsz:?}");
    let (small, large) = (ink(&family, "Words", &[(OPSZ, opsz.min)]), ink(&family, "Words", &[(OPSZ, opsz.max)]));
    assert!((small - large).abs() > large * 0.02, "{family}: {small} {large}");
    // Small text is set wider than display text.
    let (wide, narrow) = (across(&family, "Words", &[(OPSZ, opsz.min)]), across(&family, "Words", &[(OPSZ, opsz.max)]));
    assert!(wide > narrow * 1.02, "{family}: {wide} {narrow}");
}

/// A real CFF2 font, where the system has one.
#[test]
fn a_real_cff2_font_draws_and_shapes_at_its_axes() {
    // Devanagari, which one of its faces covers.
    let text = "\u{0915}\u{0916}\u{0917}";
    let Some(face) = system_face("/System/Library/Fonts/SFIndia.ttc", WGHT, text) else { return };
    let family = face.info.family.clone();
    let wght = face.axes().into_iter().find(|axis| axis.tag == WGHT).unwrap();
    let (light, heavy) = (wght.min + 99.0, wght.max);
    let (thin, thick) = (ink(&family, text, &[(WGHT, light)]), ink(&family, text, &[(WGHT, heavy)]));
    assert!(thick > thin * 1.5, "{family}: {thin} {thick}");
    let (narrow, wide) = (across(&family, text, &[(WGHT, light)]), across(&family, text, &[(WGHT, heavy)]));
    assert!(wide > narrow, "{family}: {narrow} {wide}");
    eprintln!("{family}: ink {thin} to {thick}, across {narrow} to {wide}");
}
