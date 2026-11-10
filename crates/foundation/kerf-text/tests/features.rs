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
fn tabular_figures_all_take_the_same_room() {
    let plain = styled(&[]);
    assert!(width("1111", &plain) < width("0000", &plain) - 20.0, "Inter's 1 is narrower than its 0");
    let tabular = styled(&[(TNUM, 1)]);
    assert_eq!(width("1111", &tabular), width("0000", &tabular));
    assert_eq!(width("1234", &tabular), width("5678", &tabular));
}

#[test]
fn a_feature_takes_the_fonts_other_glyphs() {
    let plain = styled(&[]);
    let slashed = styled(&[(ZERO, 1)]);
    assert_ne!(glyphs("0", &plain), glyphs("0", &slashed));
    assert_eq!(width("0", &plain), width("0", &slashed), "the slashed zero is as wide");
    // Only the zero: the rest of the text is shaped as before.
    assert_eq!(glyphs("abc", &plain), glyphs("abc", &slashed));
    // Three glyphs become a fraction, and a narrower one.
    let fraction = styled(&[(FRAC, 1)]);
    assert_ne!(glyphs("1/2", &plain), glyphs("1/2", &fraction));
    assert!(width("1/2", &fraction) < width("1/2", &plain) - 20.0);
}

#[test]
fn zero_turns_off_what_the_font_has_on() {
    // Inter joins a hyphen and a greater-than into an arrow by itself.
    let plain = styled(&[]);
    assert_eq!(glyphs("->", &plain).len(), 1);
    assert_eq!(glyphs("->", &styled(&[(CALT, 0)])).len(), 2);
    assert!(width("AV", &styled(&[(KERN, 0)])) > width("AV", &plain) + 3.0, "unkerned, A and V stand apart");
    // On is what it was already.
    assert_eq!(width("AV", &styled(&[(KERN, 1)])), width("AV", &plain));
}

#[test]
fn a_feature_named_holds_over_the_styles_own_switch() {
    let unkerned = TextStyle { kerning: false, ..styled(&[]) };
    let named = TextStyle { kerning: false, ..styled(&[(KERN, 1)]) };
    assert!(width("AV", &named) < width("AV", &unkerned) - 3.0);
    assert_eq!(width("AV", &named), width("AV", &styled(&[])));
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
