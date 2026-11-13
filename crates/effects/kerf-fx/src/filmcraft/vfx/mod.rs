//! CPU implementations of what Kerf carries of the Premiere 26 video-effect catalogue added in
//! M5.11 (see `crate::filmcraft::project::effect::vfx` for the definitions), plus the rebuilt versions
//! of a few core effects (Ultra Key, Noise, Mosaic, Simple Text, Track Matte Key).
//!
//! Conventions match [`crate::filmcraft::effects`]: images are premultiplied linear f32; spatial parameters
//! are full-resolution clip pixels scaled by `FxCtx::px_scale`; colour maths that artists expect
//! in display space (keys, limiters, CDL) runs on sRGB-encoded straight colour.
//!
//! Effects that need more than the current frame — Posterize Time and Echo (other frames), Track
//! Matte Key / Compound Blur (another track), Auto Reframe and Warp Stabilizer
//! (frames for analysis, sequence geometry) — read it through [`FxEnv`]; without an environment
//! (isolated tests, adjustment layers) they leave the image unchanged.

mod blur;
mod color;
pub(crate) use color::cdl;
mod distort;
pub mod gpu;
mod immersive;
mod stylize;
mod temporal;
mod text;

#[cfg(test)]
mod tests;

use crate::filmcraft::project::{EffectInstance, ParamValue};
use crate::filmcraft::tick::Tick;
use kerf_geom::{Affine, Vec2};
use rayon::prelude::*;

use crate::filmcraft::effects::FxCtx;
use kerf_draw::Image;

#[cfg(test)]
pub use temporal::clear_stabilizer_cache;
pub use temporal::{stabilizer_path, StabilizerPath};

/// What an effect may know about the clip beyond its own pixels.
pub trait FxEnv: Sync {
    /// The clip's layer `dt` timeline seconds from now, with the effects before `upto` applied,
    /// at the current working size. Temporal effects among those are skipped (no recursion).
    fn frame(&self, dt: f64, upto: &EffectInstance) -> Option<Image>;
    /// The clip's raw picture `dt` seconds from now at `scale` × its source size (analysis).
    fn source_frame(&self, dt: f64, scale: f32) -> Option<Image>;
    /// Video track `index` (0 = V1) rendered alone at this time, output-sized.
    fn track(&self, index: usize) -> Option<Image>;
    /// Maps working-layer pixels to output pixels (Motion included).
    fn layer_to_output(&self) -> Affine;
    /// Clip duration on the timeline, in seconds.
    fn clip_seconds(&self) -> f64;
    /// Seconds from the clip's start to now.
    fn clip_offset(&self) -> f64;
    fn frame_rate(&self) -> f64;
    fn media_timecode(&self) -> String;
    fn file_name(&self) -> String;
    fn sequence_name(&self) -> String;
    /// Full-resolution sequence frame size.
    fn sequence_size(&self) -> (u32, u32);
    /// Full-resolution clip source size.
    fn source_size(&self) -> (u32, u32);
    /// Identity of the clip and its source range (cache key for analyses).
    fn clip_key(&self) -> u64;
}

/// Effects that read other frames (skipped when rendering another frame for one of them).
pub fn is_temporal(id: &str) -> bool {
    matches!(
        id,
        "posterize_time" | "warp_stabilizer" | "auto_reframe"
    )
}

// ------------------------------------------------------------------ parameters

pub(crate) fn def_value(e: &EffectInstance, id: &str) -> Option<ParamValue> {
    e.def().and_then(|d| d.param(id)).map(|p| p.default.clone())
}
pub(crate) fn fv(e: &EffectInstance, id: &str, cx: &FxCtx) -> f32 {
    e.f64_at(id, cx.t) as f32
}
pub(crate) fn bv(e: &EffectInstance, id: &str) -> bool {
    e.param(id)
        .and_then(|p| p.value.as_bool())
        .or_else(|| def_value(e, id).and_then(|v| v.as_bool()))
        .unwrap_or(false)
}
pub(crate) fn chv(e: &EffectInstance, id: &str) -> u32 {
    match e
        .param(id)
        .map(|p| p.value.clone())
        .or_else(|| def_value(e, id))
    {
        Some(ParamValue::Choice(c)) => c,
        _ => 0,
    }
}
pub(crate) fn cv(e: &EffectInstance, id: &str, cx: &FxCtx) -> [f32; 4] {
    e.param(id)
        .and_then(|p| p.value_at(cx.t).as_color())
        .or_else(|| def_value(e, id).and_then(|v| v.as_color()))
        .unwrap_or([1.0; 4])
}
pub(crate) fn tv(e: &EffectInstance, id: &str) -> String {
    match e
        .param(id)
        .map(|p| p.value.clone())
        .or_else(|| def_value(e, id))
    {
        Some(ParamValue::Text(s)) => s,
        _ => String::new(),
    }
}
/// A point parameter in working pixels. Auto (NaN) components use the effect's auto position
/// (`crate::filmcraft::project::effect::auto_point`) or the layer centre.
pub(crate) fn pv(e: &EffectInstance, id: &str, cx: &FxCtx, img: &Image) -> Vec2 {
    let v = e
        .param(id)
        .map(|p| p.vec2_at(cx.t))
        .or_else(|| def_value(e, id).and_then(|v| v.as_vec2()))
        .unwrap_or(Vec2::new(f64::NAN, f64::NAN));
    let (fx, fy) =
        crate::filmcraft::project::effect::auto_point(&e.effect, id).unwrap_or((0.5, 0.5));
    Vec2::new(
        if v.x.is_nan() {
            img.w as f64 * fx
        } else {
            v.x * cx.px_scale as f64
        },
        if v.y.is_nan() {
            img.h as f64 * fy
        } else {
            v.y * cx.px_scale as f64
        },
    )
}
/// A point offset parameter (no auto position) in working pixels.
pub(crate) fn offv(e: &EffectInstance, id: &str, cx: &FxCtx) -> Vec2 {
    let v = e
        .param(id)
        .map(|p| p.vec2_at(cx.t))
        .or_else(|| def_value(e, id).and_then(|v| v.as_vec2()))
        .unwrap_or_default();
    let k = cx.px_scale as f64;
    Vec2::new(
        if v.x.is_nan() { 0.0 } else { v.x * k },
        if v.y.is_nan() { 0.0 } else { v.y * k },
    )
}

// ------------------------------------------------------------------ colour helpers

pub(crate) use crate::filmcraft::effects::{dec, enc};

#[inline]
pub(crate) fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
#[inline]
pub(crate) fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// sRGB-encoded straight colour param → linear RGB.
#[inline]
pub(crate) fn lin(c: [f32; 4]) -> [f32; 3] {
    dec([c[0], c[1], c[2]])
}

/// Blend `src` (straight linear RGB) onto `base` (straight linear RGB) with one of
/// [`crate::filmcraft::project::effect::SIMPLE_BLEND`] (Normal, Add, Screen, Multiply, Overlay).
#[inline]
pub(crate) fn blend_simple(mode: u32, base: [f32; 3], src: [f32; 3]) -> [f32; 3] {
    let mut o = [0.0; 3];
    for k in 0..3 {
        let (b, s) = (base[k], src[k]);
        o[k] = match mode {
            1 => b + s,
            2 => 1.0 - (1.0 - b.min(1.0)) * (1.0 - s.min(1.0)),
            3 => b * s,
            4 => {
                if b < 0.5 {
                    2.0 * b * s
                } else {
                    1.0 - 2.0 * (1.0 - b.min(1.0)) * (1.0 - s.min(1.0))
                }
            }
            _ => s,
        };
    }
    o
}

/// Premultiplied `src` over premultiplied `dst`.
#[inline]
pub(crate) fn over(dst: &mut [f32], src: [f32; 4]) {
    let k = 1.0 - src[3];
    for c in 0..4 {
        dst[c] = src[c] + dst[c] * k;
    }
}


// ------------------------------------------------------------------ noise

#[inline]
pub(crate) fn hash_u(x: i64, y: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed.wrapping_mul(0x27D4_EB2F_1656_67C5);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 32)
}
/// Uniform 0..1 from integer coordinates.
#[inline]
pub(crate) fn hash01(x: i64, y: i64, z: i64, seed: u64) -> f32 {
    (hash_u(x, y, z, seed) >> 40) as f32 / (1u64 << 24) as f32
}

// ------------------------------------------------------------------ image helpers

/// Gaussian-blur a single-channel buffer (via a temporary image).
pub(crate) fn blur_plane(plane: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.3 {
        return plane.to_vec();
    }
    let mut im = Image {
        w,
        h,
        px: plane.iter().flat_map(|&v| [v, 0.0, 0.0, 0.0]).collect(),
    };
    crate::filmcraft::effects::gaussian(&mut im, sigma, sigma, false);
    im.px.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
}


/// Fill an image from a per-pixel premultiplied colour function (x, y at pixel centres).
pub(crate) fn generate(img: &mut Image, f: impl Fn(f32, f32, [f32; 4]) -> [f32; 4] + Sync) {
    let w = img.w;
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let p = &mut row[x * 4..x * 4 + 4];
                let o = f(x as f32 + 0.5, y as f32 + 0.5, [p[0], p[1], p[2], p[3]]);
                p.copy_from_slice(&o);
            }
        });
}

/// Inverse-map warp with an affine (destination → source sampling).
pub(crate) fn affine_warp(img: &mut Image, m: &Affine) {
    if m.is_identity() {
        return;
    }
    *img = img.transformed(img.w, img.h, m);
}

// ------------------------------------------------------------------ dispatch

/// Apply `e` if it is one of the effects implemented here; returns `false` for anything else.
pub fn apply(img: &mut Image, e: &EffectInstance, cx: &FxCtx) -> bool {
    match e.effect.as_str() {
        // Adjust / Color / Image Control / Keying / Utility colour
        "logo_cutout" => color::logo_cutout(img, e, cx),
        "ultra_key" => color::ultra_key(img, e, cx),
        "track_matte" => color::track_matte(img, e, cx),
        "cineon_converter" => color::cineon(img, e, cx),
        "noise" => color::noise(img, e, cx),
        // Blur & Sharpen
        "reduce_interlace_flicker" => blur::interlace_flicker(img, e, cx),
        // Distort / Transform / Utility geometry
        "corner_pin" => distort::corner_pin(img, e, cx),
        "magnify" => distort::magnify(img, e, cx),
        "transform" if bv(e, "shutter_override") && fv(e, "shutter_angle", cx) > 0.5 => {
            distort::transform_fx(img, e, cx)
        }
        "spacer" => distort::spacer(img, e, cx),
        "auto_align" => distort::auto_align(img, e, cx),
        "rounded_crop" => distort::rounded_crop(img, e, cx),
        "mosaic" => distort::mosaic(img, e, cx),
        // Stylize / Perspective / Generate / Legacy & obsolete generators
        "stroke" => stylize::stroke(img, e, cx),
        "gradient" => stylize::gradient(img, e, cx),
        "checkerboard" => stylize::checkerboard(img, e, cx),
        "paint_bucket" => stylize::paint_bucket(img, e, cx),
        // Time / analysis
        "posterize_time" => temporal::posterize_time(img, e, cx),
        "warp_stabilizer" => temporal::warp_stabilizer(img, e, cx),
        "auto_reframe" => temporal::auto_reframe(img, e, cx),
        // Text
        "simple_text" => text::simple_text(img, e, cx),
        "metadata_burnin" => text::metadata_burnin(img, e, cx),
        id if id.starts_with("vr_") => immersive::apply(img, e, cx),
        _ => return false,
    }
    true
}
