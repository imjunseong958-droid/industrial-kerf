//! OpenType features asked for by tag, over what a font has on by itself. The numbers are those of
//! the bundled Inter, which has tabular figures, a slashed zero, fractions and contextual arrows,
//! and no small capitals.

use kerf_text::fonts::{self, Feature};
use kerf_text::{layout, ParagraphStyle, TextStyle};

const TNUM: [u8; 4] = *b"tnum";
const ZERO: [u8; 4] = *b"zero";
const FRAC: [u8; 4] = *b"frac";
const CALT: [u8; 4] = *b"calt";
const KERN: [u8; 4] = *b"kern";
const SMCP: [u8; 4] = *b"smcp";

fn styled(features: &[Feature]) -> TextStyle {
    TextStyle { size: 100.0, features: features.to_vec(), ..TextStyle::default() }
}

fn glyphs(text: &str, style: &TextStyle) -> Vec<u32> {
    layout(text, style, &ParagraphStyle::default()).glyphs.iter().map(|glyph| glyph.id).collect()
}

fn width(text: &str, style: &TextStyle) -> f32 {
    let [left, _, right, _] = layout(text, style, &ParagraphStyle::default()).bounds;
    right - left
}

#[test]
fn a_face_without_a_feature_takes_no_notice() {
    let plain = styled(&[]);
    for tag in [SMCP, *b"zzzz"] {
        let asked = styled(&[(tag, 1)]);
        assert_eq!(glyphs("Hello 0123", &plain), glyphs("Hello 0123", &asked));
        assert_eq!(width("Hello 0123", &plain), width("Hello 0123", &asked));
    }
}

#[test]
fn a_face_names_the_features_it_has() {
    let inter = fonts::faces_of("Inter");
    let has = inter.first().expect("Inter is bundled").features();
    for tag in [TNUM, ZERO, FRAC, CALT, KERN] {
        assert!(has.contains(&tag), "{} in {:?}", String::from_utf8_lossy(&tag), has.len());
    }
    assert!(!has.contains(&SMCP));
    // Each once, though substitutions and positioning both list some, and in order.
    assert!(has.windows(2).all(|pair| pair[0] < pair[1]));
    let serif = fonts::faces_of("Noto Serif");
    assert!(serif.first().expect("Noto Serif is bundled").features().contains(&SMCP));
}
