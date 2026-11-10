//! Blend modes (Premiere's 27), composited on premultiplied linear-light
//! images. Carried from FilmCraft's renderer (`blend.rs`).
//!
//! Normal is computed in linear light. The other modes are defined on display-encoded values (as
//! artists expect from every editing app), so their math runs on sRGB-encoded straight colour and
//! the result is converted back to linear. Formulas follow the W3C Compositing and Blending spec.

use kerf_color::{linear_to_srgb, srgb_to_linear};
use rayon::prelude::*;

use crate::image::Image;
use kerf_geom::{Affine, Rect};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Blend {
    pub const ALL: [Blend; 27] = [
        Blend::Normal,
        Blend::Dissolve,
        Blend::Darken,
        Blend::Multiply,
        Blend::ColorBurn,
        Blend::LinearBurn,
        Blend::DarkerColor,
        Blend::Lighten,
        Blend::Screen,
        Blend::ColorDodge,
        Blend::LinearDodge,
        Blend::LighterColor,
        Blend::Overlay,
        Blend::SoftLight,
        Blend::HardLight,
        Blend::VividLight,
        Blend::LinearLight,
        Blend::PinLight,
        Blend::HardMix,
        Blend::Difference,
        Blend::Exclusion,
        Blend::Subtract,
        Blend::Divide,
        Blend::Hue,
        Blend::Saturation,
        Blend::Color,
        Blend::Luminosity,
    ];

    /// The name a clip's `blend` property gives the mode.
    pub fn name(self) -> &'static str {
        match self {
            Blend::Normal => "normal",
            Blend::Dissolve => "dissolve",
            Blend::Darken => "darken",
            Blend::Multiply => "multiply",
            Blend::ColorBurn => "color-burn",
            Blend::LinearBurn => "linear-burn",
            Blend::DarkerColor => "darker-color",
            Blend::Lighten => "lighten",
            Blend::Screen => "screen",
            Blend::ColorDodge => "color-dodge",
            Blend::LinearDodge => "add",
            Blend::LighterColor => "lighter-color",
            Blend::Overlay => "overlay",
            Blend::SoftLight => "soft-light",
            Blend::HardLight => "hard-light",
            Blend::VividLight => "vivid-light",
            Blend::LinearLight => "linear-light",
            Blend::PinLight => "pin-light",
            Blend::HardMix => "hard-mix",
            Blend::Difference => "difference",
            Blend::Exclusion => "exclusion",
            Blend::Subtract => "subtract",
            Blend::Divide => "divide",
            Blend::Hue => "hue",
            Blend::Saturation => "saturation",
            Blend::Color => "color",
            Blend::Luminosity => "luminosity",
        }
    }
}

impl FromStr for Blend {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name() == name)
            .ok_or_else(|| format!("no blend mode `{name}`"))
    }
}

#[inline]
fn sep(mode: Blend, b: f32, s: f32) -> f32 {
    match mode {
        Blend::Darken => b.min(s),
        Blend::Multiply => b * s,
        Blend::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        Blend::LinearBurn => (b + s - 1.0).max(0.0),
        Blend::Lighten => b.max(s),
        Blend::Screen => b + s - b * s,
        Blend::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        Blend::LinearDodge => (b + s).min(1.0),
        Blend::Overlay => sep(Blend::HardLight, s, b),
        Blend::HardLight => {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                sep(Blend::Screen, b, 2.0 * s - 1.0)
            }
        }
        Blend::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        Blend::VividLight => {
            if s <= 0.5 {
                sep(Blend::ColorBurn, b, 2.0 * s)
            } else {
                sep(Blend::ColorDodge, b, 2.0 * s - 1.0)
            }
        }
        Blend::LinearLight => (b + 2.0 * s - 1.0).clamp(0.0, 1.0),
        Blend::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        Blend::HardMix => {
            if sep(Blend::VividLight, b, s) >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        Blend::Difference => (b - s).abs(),
        Blend::Exclusion => b + s - 2.0 * b * s,
        Blend::Subtract => (b - s).max(0.0),
        Blend::Divide => {
            if s <= 0.0 {
                1.0
            } else {
                (b / s).min(1.0)
            }
        }
        _ => s,
    }
}

#[inline]
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut o = c;
    if n < 0.0 {
        for v in &mut o {
            *v = l + (*v - l) * l / (l - n).max(1e-6);
        }
    }
    if x > 1.0 {
        for v in &mut o {
            *v = l + (*v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
    }
    o
}
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx - mn <= 1e-6 {
        return [0.0; 3];
    }
    let mut o = [0.0; 3];
    for k in 0..3 {
        o[k] = (c[k] - mn) * s / (mx - mn);
    }
    o
}

/// Blend function B(Cb, Cs) on straight, display-encoded colour.
pub fn blend_rgb(mode: Blend, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        Blend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        Blend::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        Blend::Color => set_lum(s, lum(b)),
        Blend::Luminosity => set_lum(b, lum(s)),
        Blend::DarkerColor => {
            if lum(s) < lum(b) {
                s
            } else {
                b
            }
        }
        Blend::LighterColor => {
            if lum(s) > lum(b) {
                s
            } else {
                b
            }
        }
        m => [sep(m, b[0], s[0]), sep(m, b[1], s[1]), sep(m, b[2], s[2])],
    }
}

fn hash2(x: usize, y: usize) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Composite `src` over `dst` (same size) with `opacity` and `mode`.
pub fn composite(dst: &mut Image, src: &Image, opacity: f32, mode: Blend) {
    debug_assert_eq!((dst.w, dst.h), (src.w, src.h));
    composite_at(dst, src, 0, 0, opacity, mode);
}

/// [`composite`] for a `src` that is only a rectangle of its layer, with its top-left corner at
/// (`x0`, `y0`) of `dst`. The rest of the layer is transparent, which leaves `dst` as it is, so only
/// the rectangle is visited; its pixels are mixed exactly as [`composite`] mixes them (the dissolve
/// pattern follows the pixel's place in `dst`). The part of the rectangle outside `dst` is ignored.
pub fn composite_at(dst: &mut Image, src: &Image, x0: usize, y0: usize, opacity: f32, mode: Blend) {
    let (cw, ch) = (
        src.w.min(dst.w.saturating_sub(x0)),
        src.h.min(dst.h.saturating_sub(y0)),
    );
    if cw == 0 || ch == 0 || src.px.len() != src.w * src.h * 4 {
        return;
    }
    let dw = dst.w;
    dst.px
        .par_chunks_mut(dw * 4)
        .skip(y0)
        .take(ch)
        .zip(src.px.par_chunks(src.w * 4))
        .enumerate()
        .for_each(|(ry, (d, s))| {
            let y = y0 + ry;
            for rx in 0..cw {
                let x = x0 + rx;
                let j = rx * 4;
                let sp = [
                    s[j] * opacity,
                    s[j + 1] * opacity,
                    s[j + 2] * opacity,
                    s[j + 3] * opacity,
                ];
                mix(&mut d[x * 4..x * 4 + 4], sp, x, y, mode);
            }
        });
}

/// `src` placed through `m` (its pixels to `dst`'s), cropped to `rect` of it, composited over `dst`
/// with `opacity` and `mode` in one pass: [`Image::transformed_within`] then [`composite`], without
/// the picture between them.
pub fn composite_placed(
    dst: &mut Image,
    src: &Image,
    m: &Affine,
    rect: Rect,
    opacity: f32,
    mode: Blend,
) {
    let (w, h) = (dst.w, dst.h);
    let Some(placement) = src.placement(w, h, m, rect) else {
        return;
    };
    dst.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .skip(placement.at.rows.start)
        .take(placement.at.rows.len())
        .for_each(|(y, row)| {
            for x in placement.at.columns.clone() {
                if let Some(sample) = placement.sample(x, y) {
                    mix(
                        &mut row[x * 4..x * 4 + 4],
                        sample.map(|value| value * opacity),
                        x,
                        y,
                        mode,
                    );
                }
            }
        });
}

/// Mix the premultiplied color `sp` (its opacity applied) into pixel `d` at (`x`, `y`).
#[inline]
pub fn mix(d: &mut [f32], mut sp: [f32; 4], x: usize, y: usize, mode: Blend) {
    let sa = sp[3];
    if sa <= 0.0 {
        return;
    }
    if mode == Blend::Dissolve {
        if hash2(x, y) >= sa {
            return;
        }
        sp = [sp[0] / sa, sp[1] / sa, sp[2] / sa, 1.0];
    }
    let da = d[3];
    if mode == Blend::Normal || mode == Blend::Dissolve || da <= 0.0 {
        let k = 1.0 - sp[3];
        for c in 0..4 {
            d[c] = sp[c] + d[c] * k;
        }
        return;
    }
    let sa = sp[3];
    let cs = [
        linear_to_srgb(sp[0] / sa),
        linear_to_srgb(sp[1] / sa),
        linear_to_srgb(sp[2] / sa),
    ];
    let cb = [
        linear_to_srgb(d[0] / da),
        linear_to_srgb(d[1] / da),
        linear_to_srgb(d[2] / da),
    ];
    let bl = blend_rgb(mode, cb, cs);
    for c in 0..3 {
        let mixed = srgb_to_linear(bl[c].clamp(0.0, 1.0));
        d[c] = sp[c] * (1.0 - da) + d[c] * (1.0 - sa) + sa * da * mixed;
    }
    d[3] = sa + da - sa * da;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiply_white_is_identity() {
        let base = [0.2, 0.4, 0.6, 1.0];
        let mut d = Image::filled(1, 1, base);
        let s = Image::filled(1, 1, [1.0, 1.0, 1.0, 1.0]);
        composite(&mut d, &s, 1.0, Blend::Multiply);
        let p = d.get(0, 0);
        for k in 0..3 {
            assert!((p[k] - base[k]).abs() < 1e-4);
        }
    }

    #[test]
    fn placed_composites_match_transforming_first() {
        let mut src = Image::new(6, 4);
        for (i, v) in src.px.iter_mut().enumerate() {
            *v = (i % 7) as f32 / 8.0;
        }
        let m = Affine::motion(
            kerf_geom::Vec2::new(5.0, 4.0),
            kerf_geom::Vec2::new(1.3, 0.9),
            17.0,
            kerf_geom::Vec2::new(3.0, 2.0),
        );
        let rect = Rect::new(0.5, 0.0, 5.0, 4.0);
        for mode in [Blend::Normal, Blend::Screen] {
            let mut a = Image::filled(10, 8, [0.2, 0.1, 0.4, 1.0]);
            let mut b = a.clone();
            composite(&mut a, &src.transformed_within(10, 8, &m, rect), 0.7, mode);
            composite_placed(&mut b, &src, &m, rect, 0.7, mode);
            for (x, y) in a.px.iter().zip(&b.px) {
                assert!((x - y).abs() < 1e-6, "{mode:?}");
            }
        }
    }

    #[test]
    fn names_read_back() {
        for mode in Blend::ALL {
            assert_eq!(mode.name().parse::<Blend>(), Ok(mode));
        }
    }

    #[test]
    fn all_modes_stay_in_range() {
        for m in Blend::ALL {
            let mut d = Image::filled(4, 4, [0.3, 0.1, 0.7, 1.0]);
            let s = Image::filled(4, 4, [0.4, 0.4, 0.1, 0.8]);
            composite(&mut d, &s, 0.9, m);
            for v in &d.px {
                assert!(v.is_finite() && *v >= -1e-5 && *v <= 1.0001, "{m:?} {v}");
            }
        }
    }
}
