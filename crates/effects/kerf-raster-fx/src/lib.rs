//! The effects library.
//!
//! Every effect is an [`EffectSpec`]: a stable id (`ec.blur.gaussian`), its display name and
//! category (the Effects & Presets tree), typed parameters with defaults and UI hints, and a CPU
//! render function. Effect instances on layers are ordinary property groups (see
//! [`instantiate`]), so parameters animate, take expressions and are addressable like any other
//! property.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod blur2;
mod blur3;
mod color2;
mod color_fx;
mod distort;
mod distort2;
mod distort3;
mod distort4;
mod generate;
mod generate3;
mod keying;
mod keying2;
mod klt;
mod matte;
mod noise;
mod noise2;
mod plane;
mod stylize2;
mod util;
mod value;

pub use value::{ParamUi, Value};

use std::collections::HashMap;
use std::sync::OnceLock;

pub use kerf_raster::Image;

#[cfg(test)]
/// Effect categories in Effects & Presets order.
pub const CATEGORIES: &[&str] = &[
    "Blur & Sharpen",
    "Channel",
    "Color Correction",
    "Distort",
    "Generate",
    "Immersive Video",
    "Keying",
    "Matte",
    "Noise & Grain",
    "Perspective",
    "Simulation",
    "Stylize",
    "Text",
    "Time",
    "Transition",
    "Utility",
];

pub struct ParamSpec {
    /// Point defaults are fractions of the layer size (0.5, 0.5 = layer centre).
    pub id: &'static str,
    pub name: &'static str,
    pub default: Value,
    pub ui: ParamUi,
}

/// Parameter values evaluated at the current time.
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub values: HashMap<String, Value>,
}

impl Params {
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id)
    }
    pub fn f(&self, id: &str) -> f64 {
        self.values.get(id).map(Value::as_f64).unwrap_or(0.0)
    }
    pub fn v2(&self, id: &str) -> [f64; 2] {
        self.values.get(id).map(Value::as_vec2).unwrap_or([0.0; 2])
    }
    pub fn color(&self, id: &str) -> [f32; 4] {
        self.values.get(id).map(Value::as_color).unwrap_or([1.0; 4])
    }
    pub fn b(&self, id: &str) -> bool {
        self.values.get(id).map(Value::as_bool).unwrap_or(false)
    }
    pub fn e(&self, id: &str) -> u32 {
        self.values.get(id).map(Value::as_enum).unwrap_or(0)
    }
    /// String parameter (`Value::Str`), empty when missing or of another kind.
    pub fn s(&self, id: &str) -> &str {
        match self.values.get(id) {
            Some(Value::Str(s)) => s,
            _ => "",
        }
    }
}

/// A layer image in flight: point `p` sits at pixel `p * scale + offset`. Points are in layer
/// space, or in effect space while the layer's effects run (see [`Buf::rebase`]).
#[derive(Clone, Debug, Default)]
pub struct Buf {
    pub img: Image,
    pub offset: [f64; 2],
    pub scale: f64,
}

impl Buf {
    pub fn to_px(&self, p: [f64; 2]) -> (f64, f64) {
        (p[0] * self.scale + self.offset[0], p[1] * self.scale + self.offset[1])
    }
    /// Grow the buffer so it covers a `size`-layer-pixel region starting at its offset (see
    /// [`cover_plan`]): the whole layer rectangle a sized source's buffer already is. Content
    /// keeps its place; the new area is transparent. No-op when the buffer already covers it.
    pub fn cover(&mut self, size: [f64; 2]) {
        let Some((dx, dy, w, h)) = cover_plan(self.offset, self.scale, size, self.img.width, self.img.height) else { return };
        let src = std::mem::replace(&mut self.img, Image::new(w, h));
        let (sw, sh) = (src.width as usize, src.height as usize);
        for y in 0..sh {
            let s = y * sw;
            let d = (y + dy as usize) * w as usize + dx as usize;
            self.img.data[d..d + sw].copy_from_slice(&src.data[s..s + sw]);
        }
        self.offset[0] += dx as f64;
        self.offset[1] += dy as f64;
    }
    /// Grow by `pad` transparent pixels on every side.
    pub fn pad(&mut self, pad: u32) {
        if pad == 0 {
            return;
        }
        self.img = self.img.padded(pad);
        self.offset[0] += pad as f64;
        self.offset[1] += pad as f64;
    }
}

/// Where [`Buf::cover`] would take a buffer: shift its pixels by `(dx, dy)` and resize it to
/// `width` × `height` so it spans the `size`-layer-pixel region that starts at its offset — the
/// whole layer rectangle, which a sized source's buffer already is. `None` when the buffer
/// already covers that region unchanged (footage, solids, the comp below an adjustment).
pub fn cover_plan(offset: [f64; 2], scale: f64, size: [f64; 2], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    if scale <= 0.0 || !scale.is_finite() || !offset[0].is_finite() || !offset[1].is_finite() || !size[0].is_finite() || !size[1].is_finite() {
        return None;
    }
    // Region and content, both as pixel ranges in the buffer's own coordinates.
    let (rw, rh) = (offset[0] + (size[0] * scale).ceil(), offset[1] + (size[1] * scale).ceil());
    let (lo_x, lo_y) = (offset[0].min(0.0).floor(), offset[1].min(0.0).floor());
    let (hi_x, hi_y) = (rw.max(width as f64).ceil(), rh.max(height as f64).ceil());
    let (nw_f, nh_f) = (hi_x - lo_x, hi_y - lo_y);
    if !(nw_f <= 16384.0 && nh_f <= 16384.0) {
        return None;
    }
    let (dx, dy) = ((-lo_x) as u32, (-lo_y) as u32);
    let (nw, nh) = (nw_f.round().max(1.0) as u32, nh_f.round().max(1.0) as u32);
    (dx != 0 || dy != 0 || nw != width || nh != height).then_some((dx, dy, nw, nh))
}

/// A layer mask at the current time, flattened to a polyline in layer coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaskShape {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub inverted: bool,
}

/// Pixels of another layer referenced by a layer parameter: its buffer (layer space, see
/// [`Buf`]) and its source size in layer pixels.
#[derive(Clone, Debug, Default)]
pub struct LayerPixels {
    pub buf: Buf,
    pub size: [f64; 2],
}

/// Services the renderer offers effects that look beyond their own pixels.
pub trait EffectHost: Sync {
    /// Pixels entering the effect stack, in effect space, before any effects. For an
    /// adjustment layer this is the composition below it, in the effect working space.
    fn original(&self) -> Option<&Buf> {
        None
    }

    /// Layer `id` of the same composition at the current comp time. `masks_and_effects`
    /// selects "Effects & Masks" over "Source". `None` for missing/recursive references; own Source reads are supported.
    fn layer(&self, id: u64, masks_and_effects: bool) -> Option<LayerPixels>;
    /// `frames` stereo sample frames (interleaved L R …) of layer `id`'s audio starting at comp
    /// time `start` seconds, at `rate` Hz. `None` when the layer has no audio.
    fn audio(&self, id: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>>;
    /// Layer `id` with its masks but none of its effects (a layer parameter's "Masks" choice).
    /// Defaults to the plain source.
    fn layer_masks(&self, id: u64) -> Option<LayerPixels> {
        self.layer(id, false)
    }
    /// The effect's own layer at another **layer time** `layer_time` (seconds): its source with
    /// masks applied, followed by the first `effects` effects of its stack (0 = none; the host
    /// never goes past the effect being rendered). `None` when unavailable (no host, recursion
    /// limit, adjustment layers). Time effects use this to read neighbouring frames.
    fn self_at(&self, _layer_time: f64, _effects: usize) -> Option<Buf> {
        None
    }
    /// Layer `id` of the same composition at comp time `comp_time` (seconds), like
    /// [`EffectHost::layer`] but at another time.
    fn layer_at(&self, _id: u64, _comp_time: f64, _masks_and_effects: bool) -> Option<LayerPixels> {
        None
    }
    /// The running effect's own parameters evaluated at another **layer time** (Radio Waves'
    /// Parameters Are Set At: Birth). `None` when unavailable.
    fn params_at(&self, _layer_time: f64) -> Option<Params> {
        None
    }
}

/// Extra context the renderer may supply (all optional; `Default` is "nothing known").
#[derive(Clone, Copy, Default)]
pub struct EffectEnv<'a> {
    /// The layer's masks (enabled ones, top to bottom).
    pub masks: &'a [MaskShape],
    pub host: Option<&'a dyn EffectHost>,
    /// Composition time in seconds.
    pub comp_time: f64,
    /// Composition frame rate (0 when unknown).
    pub frame_rate: f64,
    /// Index of the running effect in its layer's stack (for [`EffectHost::self_at`]).
    pub effect_index: usize,
    /// Working-space pixels are linear light (Linearize Working Space).
    pub working_linear: bool,
    /// The composition's shutter (angle and phase in degrees, samples per frame) when both the
    /// comp's and the layer's motion blur switches are on (Transform's Use Composition's
    /// Shutter Angle).
    pub shutter: Option<(f64, f64, u32)>,
}

/// What an effect gets to render with.
pub struct EffectCtx<'a> {
    pub params: &'a Params,
    /// Layer time in seconds.
    pub time: f64,
    /// Layer source size in layer pixels (the effect's "layer bounds").
    pub layer_size: [f64; 2],
    /// Instance seed (stable per effect instance).
    pub seed: u32,
    /// Rendering for an adjustment layer (operating on the comp below).
    pub adjustment: bool,
    /// Masks, other layers, audio, comp timing.
    pub env: EffectEnv<'a>,
}

impl EffectCtx<'_> {
    /// Frame rate for frame-based maths (comp rate, else 30).
    pub fn fps(&self) -> f64 {
        if self.env.frame_rate > 0.0 { self.env.frame_rate } else { 30.0 }
    }
}

pub type RenderFn = fn(&EffectCtx, Buf) -> Buf;

pub struct EffectSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub params: Vec<ParamSpec>,
    pub render: RenderFn,
    /// Implemented on the GPU path too (shown with a badge).
    pub gpu: bool,
    /// Supports 32 bpc float (badge).
    pub float: bool,
}

pub(crate) fn p(id: &'static str, name: &'static str, default: Value, ui: ParamUi) -> ParamSpec {
    ParamSpec { id, name, default, ui }
}
pub(crate) fn slider(min: f64, max: f64, smin: f64, smax: f64, decimals: u8) -> ParamUi {
    ParamUi::Slider { min, max, slider_min: smin, slider_max: smax, decimals }
}
pub(crate) fn popup(opts: &[&str]) -> ParamUi {
    ParamUi::Popup { options: opts.iter().map(|s| s.to_string()).collect() }
}
pub(crate) fn num(v: f64) -> Value {
    Value::Scalar(v)
}
pub(crate) fn col(r: f64, g: f64, b: f64) -> Value {
    Value::Color([r, g, b, 1.0])
}

/// All effects, sorted by category then name.
pub fn registry() -> &'static [EffectSpec] {
    static R: OnceLock<Vec<EffectSpec>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(color_fx::specs());
        v.extend(distort::specs());
        v.extend(keying::specs());
        v.extend(matte::specs());
        v.extend(blur2::specs());
        v.extend(color2::specs());
        v.extend(stylize2::specs());
        v.extend(noise::specs());
        v.extend(distort2::specs());
        v.extend(distort3::specs());
        v.extend(generate3::specs());
        v.extend(keying2::specs());
        v.extend(blur3::specs());
        v.extend(distort4::specs());
        v.sort_by(|a, b| a.category.cmp(b.category).then(a.name.cmp(b.name)));
        v
    })
}

/// An effect by id.
pub fn find(id: &str) -> Option<&'static EffectSpec> {
    registry().iter().find(|s| s.id == id)
}

/// A parameter's default value on a layer of `layer_size` (point defaults are fractions of the
/// layer size).
pub fn default_value(ps: &ParamSpec, layer_size: [f64; 2]) -> Value {
    match (&ps.ui, &ps.default) {
        (ParamUi::Point, Value::Vec2(f)) => Value::Vec2([f[0] * layer_size[0], f[1] * layer_size[1]]),
        _ => ps.default.clone(),
    }
}

/// Run an effect on a buffer.
pub fn apply(spec: &EffectSpec, ctx: &EffectCtx, buf: Buf) -> Buf {
    (spec.render)(ctx, buf)
}

/// Test helper: run effect `id` on `img` (scale 1, layer size = image size) with its defaults
/// overridden by `vals`, at layer time `time`, with an optional environment.
#[cfg(test)]
pub(crate) fn run_fx(id: &str, vals: &[(&str, Value)], img: Image, time: f64, env: EffectEnv) -> Buf {
    let s = find(id).unwrap_or_else(|| panic!("no effect {id}"));
    let size = [img.width as f64, img.height as f64];
    let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), default_value(p, size))).collect() };
    for (k, v) in vals {
        assert!(params.values.contains_key(*k), "{id}: unknown param {k}");
        params.values.insert(k.to_string(), v.clone());
    }
    let ctx = EffectCtx { params: &params, time, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env };
    apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn dump_registry() {
        for s in registry() {
            println!("{} | {} | {}", s.category, s.name, s.id);
        }
    }

    #[test]
    fn registry_is_consistent() {
        let r = registry();
        assert_eq!(r.len(), 24, "the effects Kerf keeps");
        let mut ids = std::collections::HashSet::new();
        for s in r {
            assert!(ids.insert(s.id), "duplicate {}", s.id);
            assert!(CATEGORIES.contains(&s.category), "{} in unknown category {}", s.id, s.category);
            let mut pids = std::collections::HashSet::new();
            for p in &s.params {
                assert!(pids.insert(p.id), "{}: duplicate param {}", s.id, p.id);
            }
        }
    }

    /// Registry-wide parameter sanity: unique ids and names, non-empty display names, defaults
    /// of the right kind and inside their ranges, popup defaults inside their options.
    #[test]
    fn every_param_is_well_formed() {
        let mut problems = vec![];
        let mut names = std::collections::HashSet::new();
        for s in registry() {
            if s.name.trim().is_empty() {
                problems.push(format!("{}: empty display name", s.id));
            }
            if !names.insert(s.name) {
                problems.push(format!("{}: duplicate display name {}", s.id, s.name));
            }
            let mut ids = std::collections::HashSet::new();
            for p in &s.params {
                let at = format!("{} / {}", s.id, p.id);
                if !ids.insert(p.id) {
                    problems.push(format!("{at}: duplicate id"));
                }
                if p.id.is_empty() || p.id.contains(char::is_whitespace) {
                    problems.push(format!("{at}: bad id"));
                }
                if p.name.trim().is_empty() {
                    problems.push(format!("{at}: empty display name"));
                }
                match (&p.ui, &p.default) {
                    (ParamUi::Slider { min, max, slider_min, slider_max, .. }, d) => {
                        if !(min <= max && slider_min <= slider_max) {
                            problems.push(format!("{at}: inverted range"));
                        }
                        if !(slider_min >= min && slider_max <= max) {
                            problems.push(format!("{at}: slider range {slider_min}..{slider_max} outside valid range {min}..{max}"));
                        }
                        match d {
                            Value::Scalar(v) if v < min || v > max => problems.push(format!("{at}: default {v} outside {min}..{max}")),
                            Value::Scalar(_) => {}
                            d => problems.push(format!("{at}: slider default {d:?} is not a number")),
                        }
                    }
                    (ParamUi::Popup { options }, d) => {
                        if options.is_empty() || options.iter().any(|o| o.trim().is_empty()) {
                            problems.push(format!("{at}: empty popup option"));
                        }
                        let uniq: std::collections::HashSet<_> = options.iter().collect();
                        if uniq.len() != options.len() {
                            problems.push(format!("{at}: duplicate popup option"));
                        }
                        match d {
                            Value::Enum(i) if (*i as usize) < options.len() => {}
                            d => problems.push(format!("{at}: popup default {d:?} not among {} options", options.len())),
                        }
                    }
                    (ParamUi::Checkbox, d) if !matches!(d, Value::Bool(_)) => problems.push(format!("{at}: checkbox default {d:?}")),
                    (ParamUi::Color, d) if !matches!(d, Value::Color(_)) => problems.push(format!("{at}: color default {d:?}")),
                    (ParamUi::Point, d) if !matches!(d, Value::Vec2(_)) => problems.push(format!("{at}: point default {d:?}")),
                    (ParamUi::Point3, d) if !matches!(d, Value::Vec3(_)) => problems.push(format!("{at}: 3D point default {d:?}")),
                    (ParamUi::Layer, d) if !matches!(d, Value::Layer(_)) => problems.push(format!("{at}: layer default {d:?}")),
                    (ParamUi::Percent | ParamUi::Angle | ParamUi::Pixels | ParamUi::Number, d) if !matches!(d, Value::Scalar(_) | Value::Vec2(_)) => {
                        problems.push(format!("{at}: numeric default {d:?}"))
                    }
                    _ => {}
                }
            }
        }
        assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
    }

    /// Every effect runs on a small image with its default parameters without panicking and
    /// keeps alpha in range.
    #[test]
    fn all_effects_run_with_defaults() {
        let mut img = Image::new(48, 32);
        for y in 0..32 {
            for x in 0..48 {
                let a = if (8..40).contains(&x) && (6..26).contains(&y) { 1.0 } else { 0.0 };
                img.set(x, y, [x as f32 / 48.0 * a, y as f32 / 32.0 * a, 0.5 * a, a]);
            }
        }
        for s in registry() {
            let params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
            let ctx = EffectCtx { params: &params, time: 0.5, layer_size: [48.0, 32.0], seed: 1, adjustment: false, env: Default::default() };
            let out = apply(s, &ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 });
            assert!(!out.img.is_empty(), "{}", s.id);
            assert!(out.img.data.iter().all(|p| p[3].is_finite() && p[3] >= -1e-4 && p[3] <= 1.0001), "{} alpha out of range", s.id);
        }
    }
}
