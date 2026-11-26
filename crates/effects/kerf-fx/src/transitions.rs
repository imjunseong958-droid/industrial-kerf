//! Video transitions, carried from FilmCraft's renderer (`transitions`): combine the outgoing (A)
//! and incoming (B) layers at progress `p` ∈ \[0,1\]. Kerf's catalog lists them
//! (`kerf-model/src/transitions.tsv`, engine `kerf`), and [`Fx`] gives them a transition's
//! parameters as a document sets them.
//!
//! Every transition is procedural maths written for FilmCraft (no third-party artwork, no masks
//! or textures from other products). Shared building blocks:
//!
//! - [`Tx`]: the inputs plus typed parameter access (defaults come from the definition, so older
//!   projects that lack a newer parameter still render).
//! - [`paint`]: evaluate a per-pixel function in parallel.
//! - [`wipe::field_wipe`]: shape wipes from a scalar field (iris, clock, star, linear…) with
//!   feather, border width/colour and anti-aliasing.
//! - [`cards`]: textured 3D quads under a perspective camera (flips, spins, cubes, folds).
//!
//! Contract (tested for every transition): `p = 0` is exactly the outgoing frame, `p = 1` exactly
//! the incoming frame, the output is finite and deterministic, and it changes continuously near
//! both ends.
//!
//! Morph Cut morphs with dense optical flow ([`flow`]) but has no face tracking.

use kerf_draw::Image;
use kerf_model::{FxDef, ParamType, Params, Rgba, Time, Value};
use rayon::prelude::*;

pub mod cards;
mod dissolve;
mod flow;
pub mod gpu;
mod grunge;
mod legacy;
mod lights;
mod motion;
mod special;
pub mod wipe;

#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod tests;

/// A transition as a document gives it: its definition, the parameters set (the rest take the
/// definition's defaults), and the FilmCraft transition that draws it.
pub struct Fx<'a> {
    /// FilmCraft's id for it.
    pub id: String,
    def: &'a FxDef,
    params: &'a Params,
}

impl<'a> Fx<'a> {
    /// Transition `def` with `params`, if Kerf's renderer draws it.
    pub fn new(def: &'a FxDef, params: &'a Params) -> Option<Self> {
        let id = filmcraft_id(def.id.as_str())?;
        Some(Self { id, def, params })
    }

    fn value(&self, name: &str) -> Option<Value> {
        match self.params.get(name) {
            Some(param) => Some(param.value_at(Time::ZERO)),
            None => self.def.find(name).map(|def| def.default.clone()),
        }
    }

    /// A number parameter (0 when it is not one).
    pub fn num(&self, name: &str) -> f32 {
        let value = self
            .value(name)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0) as f32;
        if value.is_finite() {
            value
        } else {
            0.0
        }
    }

    /// A choice parameter, as its place among the choices.
    pub fn choice(&self, name: &str) -> Option<u32> {
        let chosen = match self.value(name)? {
            Value::Enum(word) | Value::Text(word) => word,
            _ => return None,
        };
        match &self.def.find(name)?.ty {
            ParamType::Enum(options) => options
                .iter()
                .position(|option| *option == chosen)
                .map(|at| at as u32),
            _ => None,
        }
    }

    /// Whether the transition has parameter `name`.
    pub fn has(&self, name: &str) -> bool {
        self.def.find(name).is_some()
    }

    /// A switch parameter.
    pub fn flag(&self, name: &str) -> bool {
        matches!(self.value(name), Some(Value::Bool(true)))
    }

    /// A color parameter.
    pub fn color(&self, name: &str) -> Option<Rgba> {
        match self.value(name)? {
            Value::Color(color) => Some(color),
            _ => None,
        }
    }

    /// Whether the transition takes `side`'s picture as a share of it: what its mix shows of
    /// that picture is the picture as it is, moved, warped, or blurred, by as much of a pixel
    /// in each channel and in alpha, and how much is set by where the two pictures are and by
    /// what the other one shows, not by what this one shows. Where a model films that clip, the
    /// take laid under it can then show in its place by exactly that share ([`split_filmed`]).
    pub fn shares(&self, side: Side) -> bool {
        match self.id.as_str() {
            // These go by how bright the outgoing picture is: where it burns first, or glows.
            "burn_alpha" | "phosphor" => side == Side::Incoming,
            // This by how bright the picture `source` names is.
            "luma_fade" => match self.choice("source") {
                Some(1) => side == Side::Outgoing,
                _ => side == Side::Incoming,
            },
            // These go by how bright both pictures are (rays, a glow, solarizing, a morph's
            // flow), add or blend them in a curve (additive, film), brighten or tint them
            // (flicker, glass's facets, burn chroma), or move their channels apart (chaos,
            // glitch, VHS damage).
            "additive_dissolve" | "burn_chroma" | "chaos" | "film_dissolve" | "flicker"
            | "glass" | "glitch" | "glow" | "morph_cut" | "ray" | "solarize" | "vhs_damage" => {
                false
            }
            _ => true,
        }
    }
}

/// One of the two pictures a transition mixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The clip it leaves (A).
    Outgoing,
    /// The clip it enters (B).
    Incoming,
}

/// The FilmCraft transition that draws Kerf's transition `id`, if Kerf's renderer draws it.
pub fn filmcraft_id(id: &str) -> Option<String> {
    // Kerf's renderer draws `stack` itself.
    let known = id != kerf_model::look::STACK
        && kerf_model::Catalog::builtin().fx(id).is_some_and(|def| {
            def.engine == kerf_model::Engine::Kerf && def.kind == kerf_model::FxKind::Transition
        });
    if !known {
        return None;
    }
    let id = id.replace('-', "_");
    Some(match id.as_str() {
        // FilmCraft names these apart from the effects of the same name.
        "directional_blur" | "mirror" | "mosaic" => format!("{id}_transition"),
        _ => id,
    })
}

#[cfg(test)]
/// Render transition `e` at progress `p` (full-resolution parameters).
pub fn apply(e: &Fx, a: &Image, b: &Image, p: f32) -> Image {
    apply_scaled(e, a, b, p, 1.0)
}

/// Render transition `e` at progress `p`; `scale` converts pixel parameters (border width, blur
/// length, centre) from canvas pixels to working-image pixels (½ for half-resolution playback).
pub fn apply_scaled(e: &Fx, a: &Image, b: &Image, p: f32, scale: f32) -> Image {
    let p = if p.is_finite() {
        p.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if a.w == 0 || a.h == 0 || (a.w, a.h) != (b.w, b.h) {
        return if p < 0.5 { a.clone() } else { b.clone() };
    }
    if p <= 0.0 {
        return a.clone();
    }
    if p >= 1.0 {
        return b.clone();
    }
    if let Some(img) = legacy::apply(e, a, b, p) {
        return img;
    }
    let t = Tx {
        e,
        a,
        b,
        p,
        w: a.w,
        h: a.h,
        wf: a.w as f32,
        hf: a.h as f32,
        scale: if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        },
    };
    let id = e.id.as_str();
    dissolve::apply(id, &t)
        .or_else(|| wipe::apply(id, &t))
        .or_else(|| motion::apply(id, &t))
        .or_else(|| lights::apply(id, &t))
        .or_else(|| grunge::apply(id, &t))
        .or_else(|| special::apply(id, &t))
        .unwrap_or_else(|| paint(t.w, t.h, |x, y| mix(t.sa(x, y), t.sb(x, y), p)))
}

// ---------------------------------------------------------------------------------------------
// Beside a clip a model films

/// [`apply_scaled`] where `filmed` is the picture of a clip a model films, whose take, laid
/// under it, is to show in its place. A transition that does not take that picture as a share
/// of it ([`Fx::shares`]) goes by what the picture shows, and the take is not that: such a mix
/// is a plain cross dissolve of the two pictures here, which does take each as a share.
pub fn apply_filmed(e: &Fx, a: &Image, b: &Image, p: f32, scale: f32, filmed: Side) -> Image {
    if e.shares(filmed) {
        return apply_scaled(e, a, b, p, scale);
    }
    let p = if p.is_finite() {
        p.clamp(0.0, 1.0)
    } else {
        0.0
    };
    a.clone().lerp(b, p)
}

/// [`apply_filmed`] split into what Kerf draws of it and what it leaves to the take.
pub struct Split {
    /// What the mix shows of the picture beside the filmed one, premultiplied, its alpha that
    /// picture's share of each pixel.
    pub drawn: Image,
    /// How much of each pixel the whole mix covers, both pictures' shares.
    pub covered: Vec<f32>,
}

impl Split {
    /// What the mix covers of `under` made clear by as much, and what Kerf draws of it laid
    /// there: the filmed picture's share of each pixel is left clear, for the take to show.
    pub fn lay(&self, under: &mut Image) {
        under
            .px
            .par_chunks_mut(4)
            .zip(self.drawn.px.par_chunks(4))
            .zip(self.covered.par_iter())
            .for_each(|((to, drawn), &covered)| {
                if covered <= 0.0 {
                    return;
                }
                for channel in 0..4 {
                    to[channel] = to[channel] * (1.0 - covered) + drawn[channel];
                }
            });
    }
}

/// [`apply_filmed`] of `a` and `b`, where `filmed`'s is the picture of a clip a model films,
/// split ([`Split`]). It is made from the mix with that picture black where it is, which leaves
/// the other's part, and white, which says how much of each pixel is the filmed one's: what the
/// filmed picture shows does not count, as the take shows in its place.
pub fn split_filmed(e: &Fx, a: &Image, b: &Image, p: f32, scale: f32, filmed: Side) -> Split {
    split(|a, b| apply_filmed(e, a, b, p, scale, filmed), a, b, filmed)
}

/// `mix` of `a` and `b` split as [`split_filmed`] does, exact for a mix that takes `filmed`'s
/// picture as a share of it.
fn split(mix: impl Fn(&Image, &Image) -> Image, a: &Image, b: &Image, filmed: Side) -> Split {
    let (dark, lit) = match filmed {
        Side::Outgoing => (mix(&in_grey(a, 0.0), b), mix(&in_grey(a, 1.0), b)),
        Side::Incoming => (mix(a, &in_grey(b, 0.0)), mix(a, &in_grey(b, 1.0))),
    };
    let mut drawn = dark;
    let mut covered = vec![0.0; drawn.px.len() / 4];
    drawn
        .px
        .par_chunks_mut(4)
        .zip(lit.px.par_chunks(4))
        .zip(covered.par_iter_mut())
        .for_each(|((pixel, lit), covered)| {
            let whole = pixel[3].clamp(0.0, 1.0);
            let filmed = (lit[0] - pixel[0]).clamp(0.0, whole);
            *covered = whole;
            pixel[3] = whole - filmed;
            // Light the transition lays where the pixel is all the filmed picture's, a burn's
            // edge or a leak, adds to the take there, as it adds to the picture in its own
            // sequence. A pixel with no share of its own is laid as nothing
            // (`kerf_draw::blend::mix`), so it keeps one too small to dim the take.
            if pixel[3] <= 0.0 && pixel[..3].iter().any(|channel| *channel > 0.0) {
                pixel[3] = LIGHT_ALONE;
            }
        });
    Split { drawn, covered }
}

/// The share of a pixel that light Kerf draws keeps where it is alone over the take
/// ([`split_filmed`]).
const LIGHT_ALONE: f32 = 1e-6;

/// `picture` with every pixel of it `grey`, as much as it is there.
fn in_grey(picture: &Image, grey: f32) -> Image {
    let mut out = picture.clone();
    out.px.par_chunks_mut(4).for_each(|pixel| {
        let value = grey * pixel[3];
        pixel[..3].fill(value);
    });
    out
}

// ---------------------------------------------------------------------------------------------
// Inputs and parameters

/// One transition evaluation: the effect, both layers, progress and the pixel scale.
pub(crate) struct Tx<'a> {
    pub e: &'a Fx<'a>,
    pub a: &'a Image,
    pub b: &'a Image,
    pub p: f32,
    pub w: usize,
    pub h: usize,
    pub wf: f32,
    pub hf: f32,
    pub scale: f32,
}

impl Tx<'_> {
    /// A float parameter (definition default if the document leaves it out).
    pub fn num(&self, id: &str) -> f32 {
        self.e.num(id)
    }
    /// A percentage parameter as 0..1.
    pub fn frac(&self, id: &str) -> f32 {
        self.num(id) / 100.0
    }
    /// A pixel-size parameter in working-image pixels.
    pub fn px(&self, id: &str) -> f32 {
        self.num(id).max(0.0) * self.scale
    }
    pub fn choice(&self, id: &str) -> u32 {
        self.e.choice(id).unwrap_or(0)
    }
    pub fn flag(&self, id: &str) -> bool {
        self.e.flag(id)
    }
    /// A colour parameter as opaque linear-light RGBA (colour params are display-encoded).
    pub fn color(&self, id: &str) -> [f32; 4] {
        let decode = kerf_draw::color::decode_table();
        match self.e.color(id) {
            Some(c) => [
                decode[usize::from(c.r)],
                decode[usize::from(c.g)],
                decode[usize::from(c.b)],
                1.0,
            ],
            None => [1.0; 4],
        }
    }
    /// The `center` point (`center_x`, `center_y`, fractions of the frame) in working pixels; the
    /// frame's center for a transition without one.
    pub fn center(&self) -> (f32, f32) {
        if !self.e.has("center_x") {
            return (self.wf / 2.0, self.hf / 2.0);
        }
        (
            self.e.num("center_x") * self.wf,
            self.e.num("center_y") * self.hf,
        )
    }
    pub fn seed(&self) -> u32 {
        self.num("seed").max(0.0) as u32
    }
    /// Unit motion vector of the `direction` param: the incoming clip enters *from* that side, so
    /// "From West" moves left → right (+x).
    pub fn dir(&self) -> (f32, f32) {
        match self.choice("direction") {
            0 => (0.0, 1.0),
            1 => (-1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (1.0, 0.0),
        }
    }
    /// Frame extent along the direction of travel.
    pub fn travel(&self) -> f32 {
        let (dx, dy) = self.dir();
        (dx.abs() * self.wf + dy.abs() * self.hf).max(1.0)
    }
    /// Reference size for resolution-independent defaults (1 at 1080p height).
    pub fn unit(&self) -> f32 {
        self.hf / 1080.0
    }
    #[inline]
    pub fn sa(&self, x: f32, y: f32) -> [f32; 4] {
        self.a.sample_bilinear(x, y)
    }
    #[inline]
    pub fn sb(&self, x: f32, y: f32) -> [f32; 4] {
        self.b.sample_bilinear(x, y)
    }
    #[inline]
    pub fn sac(&self, x: f32, y: f32) -> [f32; 4] {
        self.a.sample_bilinear_clamped(x, y)
    }
    #[inline]
    pub fn sbc(&self, x: f32, y: f32) -> [f32; 4] {
        self.b.sample_bilinear_clamped(x, y)
    }
    /// Pixel of A / B at integer coordinates (fast path for unwarped reads).
    #[inline]
    pub fn pa(&self, x: f32, y: f32) -> [f32; 4] {
        self.a.get_clamped(x as isize, y as isize)
    }
    #[inline]
    pub fn pb(&self, x: f32, y: f32) -> [f32; 4] {
        self.b.get_clamped(x as isize, y as isize)
    }
}

// ---------------------------------------------------------------------------------------------
// Pixel helpers

/// Evaluate `f(x, y)` at every pixel centre (x, y in pixels, +0.5 centres), in parallel.
pub(crate) fn paint(w: usize, h: usize, f: impl Fn(f32, f32) -> [f32; 4] + Sync) -> Image {
    let mut out = Image::new(w, h);
    out.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let fy = y as f32 + 0.5;
            for x in 0..w {
                let c = f(x as f32 + 0.5, fy);
                row[x * 4..x * 4 + 4].copy_from_slice(&c);
            }
        });
    out
}

#[inline]
pub(crate) fn mix(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
        a[3] + (b[3] - a[3]) * k,
    ]
}
/// Premultiplied `src` over `dst`.
#[inline]
pub(crate) fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let k = 1.0 - src[3];
    [
        src[0] + dst[0] * k,
        src[1] + dst[1] * k,
        src[2] + dst[2] * k,
        src[3] + dst[3] * k,
    ]
}
#[inline]
pub(crate) fn scale4(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, c[3] * k]
}
/// Add light `rgb × k` (keeps alpha at least as opaque as the light).
#[inline]
pub(crate) fn add_light(c: [f32; 4], rgb: [f32; 4], k: f32) -> [f32; 4] {
    [
        c[0] + rgb[0] * k,
        c[1] + rgb[1] * k,
        c[2] + rgb[2] * k,
        (c[3] + k.max(0.0)).min(1.0),
    ]
}
#[inline]
pub(crate) fn luma(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// 0 at both ends, 1 at the midpoint (smooth).
#[inline]
pub(crate) fn bell(p: f32) -> f32 {
    let t = 4.0 * p * (1.0 - p);
    t * t * (3.0 - 2.0 * t)
}
/// The A→B mix weight for "distort out, distort in" transitions: switches around the midpoint.
#[inline]
pub(crate) fn mid_mix(p: f32) -> f32 {
    smoothstep(0.3, 0.7, p)
}
#[inline]
pub(crate) fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}
#[inline]
pub(crate) fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}
#[inline]
pub(crate) fn ease_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

/// Integer hash → [0, 1).
#[inline]
pub(crate) fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ s.wrapping_mul(0xCB1A_B31F)
        ^ 0x9E37_79B9;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}
/// Smooth value noise in [0, 1) with unit cell size.
pub(crate) fn vnoise(x: f32, y: f32, s: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (xi as i32, yi as i32);
    let a = hash(i, j, s);
    let b = hash(i + 1, j, s);
    let c = hash(i, j + 1, s);
    let d = hash(i + 1, j + 1, s);
    let top = a + (b - a) * ux;
    let bot = c + (d - c) * ux;
    top + (bot - top) * uy
}
/// Fractal (4-octave) value noise in ~[0, 1).
pub(crate) fn fbm(x: f32, y: f32, s: u32) -> f32 {
    let (mut v, mut amp, mut f, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..4 {
        v += vnoise(x * f, y * f, s.wrapping_add(o * 101)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    v / norm
}

/// Average of `n` samples of `img` along the segment centred on (x, y) with half-extent (dx, dy).
pub(crate) fn line_blur(img: &Image, x: f32, y: f32, dx: f32, dy: f32, n: usize) -> [f32; 4] {
    if n <= 1 || (dx.abs() + dy.abs()) < 0.5 {
        return img.sample_bilinear_clamped(x, y);
    }
    let mut acc = [0.0; 4];
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
        let s = img.sample_bilinear_clamped(x + dx * t, y + dy * t);
        for k in 0..4 {
            acc[k] += s[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}
/// Zoom (radial streak) blur towards (cx, cy): samples between the pixel and `k` of the way to
/// the centre.
pub(crate) fn zoom_blur(
    img: &Image,
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    k: f32,
    n: usize,
) -> [f32; 4] {
    let (dx, dy) = ((x - cx) * k * 0.5, (y - cy) * k * 0.5);
    line_blur(img, x - dx, y - dy, dx, dy, n)
}
/// Spin blur about (cx, cy) over `ang` radians (centred on the pixel).
pub(crate) fn spin_blur(
    img: &Image,
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    ang: f32,
    n: usize,
) -> [f32; 4] {
    if n <= 1 || ang.abs() < 1e-4 {
        return img.sample_bilinear_clamped(x, y);
    }
    let (rx, ry) = (x - cx, y - cy);
    let mut acc = [0.0; 4];
    for i in 0..n {
        let a = (i as f32 / (n - 1) as f32 - 0.5) * ang;
        let (s, c) = a.sin_cos();
        let p = img.sample_bilinear_clamped(cx + rx * c - ry * s, cy + rx * s + ry * c);
        for k in 0..4 {
            acc[k] += p[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}
/// The sample offsets of a disc (bokeh-like) blur of radius `r` with `n` golden-angle samples
/// (empty: no blur), for [`disc_blur_with`]. They depend
/// only on `r` and `n`, so a frame works them out once instead of for every pixel (#1871: the
/// square root, sine and cosine of every sample of every pixel were most of a Lens Blur frame).
pub(crate) fn disc_offsets(r: f32, n: usize) -> Vec<(f32, f32)> {
    if r < 0.5 || n <= 1 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let rr = r * ((i as f32 + 0.5) / n as f32).sqrt();
            let a = i as f32 * 2.399_963;
            (rr * a.cos(), rr * a.sin())
        })
        .collect()
}

/// Disc (bokeh-like) blur at `(x, y)` with `n` golden-angle samples, their offsets from
/// [`disc_offsets`].
pub(crate) fn disc_blur_with(img: &Image, x: f32, y: f32, offsets: &[(f32, f32)]) -> [f32; 4] {
    if offsets.is_empty() {
        return img.sample_bilinear_clamped(x, y);
    }
    let mut acc = [0.0; 4];
    for &(dx, dy) in offsets {
        let p = img.sample_bilinear_clamped(x + dx, y + dy);
        for k in 0..4 {
            acc[k] += p[k];
        }
    }
    scale4(acc, 1.0 / offsets.len() as f32)
}
/// Sample count for a blur of extent `len` pixels.
#[inline]
pub(crate) fn taps(len: f32) -> usize {
    ((len.abs() * 0.5).ceil() as usize).clamp(1, 40)
}
/// Gaussian-blurred copy (σ in pixels).
pub(crate) fn blurred(img: &Image, sigma: f32) -> Image {
    let mut out = img.clone();
    if sigma > 0.3 {
        kerf_draw::blur::gaussian(&mut out, sigma, sigma, true);
    }
    out
}
/// Horizontal-wrap bilinear sample (equirectangular frames).
pub(crate) fn sample_wrap(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let w = img.w as f32;
    let xw = (x - 0.5).rem_euclid(w);
    let x0 = xw.floor();
    let tx = xw - x0;
    let x0 = x0 as usize % img.w;
    let x1 = (x0 + 1) % img.w;
    let fy = (y - 0.5).clamp(0.0, img.h as f32 - 1.0);
    let y0 = fy.floor() as usize;
    let y1 = (y0 + 1).min(img.h - 1);
    let ty = fy - y0 as f32;
    let (a, b, c, d) = (
        img.get(x0, y0),
        img.get(x1, y0),
        img.get(x0, y1),
        img.get(x1, y1),
    );
    let mut o = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bot = c[k] + (d[k] - c[k]) * tx;
        o[k] = top + (bot - top) * ty;
    }
    o
}

/// Average a per-pixel scene over `n` time samples spread over `shutter` (in progress units)
/// around `p` (motion blur). `n = 1` or `shutter = 0` evaluates once at `p`.
pub(crate) fn temporal(
    x: f32,
    y: f32,
    p: f32,
    shutter: f32,
    n: usize,
    f: &(impl Fn(f32, f32, f32) -> [f32; 4] + Sync),
) -> [f32; 4] {
    if n <= 1 || shutter <= 1e-5 {
        return f(x, y, p);
    }
    let mut acc = [0.0; 4];
    for i in 0..n {
        let q = (p + (i as f32 / (n - 1) as f32 - 0.5) * shutter).clamp(0.0, 1.0);
        let c = f(x, y, q);
        for k in 0..4 {
            acc[k] += c[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}
/// How far apart on the screen, in pixels of the picture, a pixel's motion blur sees what moves at
/// most from one of its samples to the next. Each sample is sharp, so copies much further apart than
/// this read as copies and not as a smear.
pub(crate) const BLUR_STEP: f32 = 4.0;

/// The most moments a pixel's motion blur takes of a card scene: at 1080p, with the motion blur as it
/// comes, the cards of any of them are then under 6 pixels apart in its fastest frame. Three cards at
/// each of these are about what the GPU's scene kernel has room for.
pub(crate) const MOST_MOMENTS: usize = 80;

/// The most samples a pixel's motion blur takes of a transition that works out where the picture is
/// at each, for what drawing it costs: at 1080p, with the motion blur as it comes, Block Motion's
/// blocks, which move fastest, are then under 5 pixels apart.
pub(crate) const MOST_SAMPLES: usize = 120;

/// How many stretches of the shutter [`samples_over`] follows what moves over to see how fast it goes.
const PROBES: usize = 8;

/// How long the shutter is open from the `motion_blur` param, in progress units: `max` at 100 %, 0 for
/// no motion blur.
pub(crate) fn exposure(t: &Tx, max: f32) -> f32 {
    max * t.frac("motion_blur").clamp(0.0, 1.0)
}

/// How many samples a shutter open for `open` (progress units) around `p` takes: as many as keep
/// `points` within [`BLUR_STEP`] pixels of where they were at the sample before, where they move
/// fastest over any of [`PROBES`] stretches of the shutter, and `most` at most. `points` are places on
/// the screen that move with the picture, as they are at a moment, `None` for one that is not on it
/// then. 1 with the shutter closed.
pub(crate) fn samples_over(
    p: f32,
    open: f32,
    most: usize,
    points: impl Fn(f32) -> Vec<Option<[f32; 2]>>,
) -> usize {
    if open <= 0.0 {
        return 1;
    }
    let (first, last) = ((p - open / 2.0).max(0.0), (p + open / 2.0).min(1.0));
    if last <= first {
        return 1;
    }
    let stretch = (last - first) / PROBES as f32;
    let mut farthest = 0f32;
    let mut before = points(first);
    for i in 1..=PROBES {
        let after = points(first + stretch * i as f32);
        for (a, b) in before.iter().zip(&after) {
            if let (Some(a), Some(b)) = (a, b) {
                farthest = farthest.max((b[0] - a[0]).hypot(b[1] - a[1]));
            }
        }
        before = after;
    }
    // Pixels over the whole shutter at that speed; NaN (from no picture to speak of) takes one sample.
    let across = farthest / stretch * open;
    let steps = (across / BLUR_STEP)
        .ceil()
        .clamp(1.0, (most.max(2) - 1) as f32);
    steps as usize + 1
}

/// Motion blur from the `motion_blur` param that follows how far the picture moves: the shutter, as
/// [`exposure`] gives it, and the samples [`samples_over`] gives `points` over it, [`MOST_SAMPLES`] at
/// most. [`temporal`] takes them.
pub(crate) fn following(
    t: &Tx,
    max: f32,
    points: impl Fn(f32) -> Vec<Option<[f32; 2]>>,
) -> (f32, usize) {
    let open = exposure(t, max);
    (open, samples_over(t.p, open, MOST_SAMPLES, points))
}

/// The moments [`temporal`] evaluates a pixel at, for what is the same at every pixel to be worked out
/// once a frame; [`averaged`] then takes a pixel at each.
pub(crate) fn sample_times(p: f32, shutter: f32, n: usize) -> Vec<f32> {
    if n <= 1 || shutter <= 1e-5 {
        return vec![p];
    }
    (0..n)
        .map(|i| (p + (i as f32 / (n - 1) as f32 - 0.5) * shutter).clamp(0.0, 1.0))
        .collect()
}

/// A pixel at each of `n` of [`sample_times`] (`f` takes which), averaged as [`temporal`] averages them.
pub(crate) fn averaged(n: usize, f: impl Fn(usize) -> [f32; 4]) -> [f32; 4] {
    if n <= 1 {
        return f(0);
    }
    let mut acc = [0.0; 4];
    for i in 0..n {
        let c = f(i);
        for k in 0..4 {
            acc[k] += c[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}
/// Motion-blur settings from the `motion_blur` param: (shutter in progress units, samples), a number
/// of samples that does not follow how far the picture moves.
pub(crate) fn shutter(t: &Tx, max: f32) -> (f32, usize) {
    let mb = t.frac("motion_blur").clamp(0.0, 1.0);
    if mb <= 0.0 {
        (0.0, 1)
    } else {
        (exposure(t, max), ((mb * 12.0).ceil() as usize).clamp(2, 12))
    }
}
