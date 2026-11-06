//! A variable font's axes, set on its way through shaping and drawing. `testdata/make_test_font.py`
//! says what the test font's numbers are.

use kerf_text::fonts::{self, Axis, FaceData, FaceInfo, FontSource};
use kerf_text::{layout, render, ParagraphStyle, TextStyle};

const FAMILY: &str = "Kerf Test Variable";

struct TestFont;

impl FontSource for TestFont {
    fn faces(&self) -> Vec<FaceInfo> {
        vec![FaceInfo {
            family: FAMILY.into(),
            style: "Regular".into(),
            weight: 400,
            italic: false,
            index: 0,
            data: FaceData::Static(include_bytes!("../testdata/KerfTestVariable.ttf")),
            origin: "test",
        }]
    }
}

const WGHT: [u8; 4] = *b"wght";
const WDTH: [u8; 4] = *b"wdth";

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
