//! The parts of FilmCraft's project model (`filmcraft-project`) its effects read: the effect
//! definitions (`effect`, carried from FilmCraft's), parameter values, and effect instances.
//!
//! Kerf builds an instance of an effect from a clip's effect at the time drawn, so a parameter
//! here is a value (or Kerf's own evaluation of it over time), not keyframes.

pub mod effect;
mod keyframe;

pub use effect::{
    effect_defs, find_effect, EffectDef, EffectInstance, EffectKind, ParamDef, ParamKind,
    BLEND_MODES,
};
pub use keyframe::{Param, ParamValue};

/// How the Warp Stabilizer's tracker follows a picture (FilmCraft's mask tracking methods).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackMethod {
    Position,
    PositionRotation,
    #[default]
    PositionScaleRotation,
}

/// Most bytes of LUT text read.
pub const MAX_LUT_TEXT_BYTES: usize = 8 << 20;

/// A LUT a Lumetri effect names as `lib:<id>`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectLut {
    pub id: String,
    pub name: String,
    /// Where it was read from (informational).
    pub source_path: Option<String>,
    /// `cube` or `3dl`.
    pub format: String,
    /// The file's text.
    pub text: std::sync::Arc<str>,
}

/// What FilmCraft's effects read from a project: the LUTs Lumetri names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub name: String,
    pub luts: Vec<ProjectLut>,
}

impl Project {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            luts: Vec::new(),
        }
    }
}

#[cfg(test)]
/// Fill a point parameter left "auto" (a NaN coordinate) in with where it starts: the effect's
/// auto position across the frame (`anchor`: across the source), or else the centre (`end`: the
/// bottom centre).
pub fn resolve_auto_points(e: &mut EffectInstance, frame: (u32, u32), source: (u32, u32)) {
    for (k, p) in e.params.iter_mut() {
        if let ParamValue::Vec2(v) = &mut p.value {
            let (w, h) = if k == "anchor" { source } else { frame };
            let (fx, fy) = effect::auto_point(&e.effect, k)
                .unwrap_or((0.5, if k == "end" { 1.0 } else { 0.5 }));
            if v.x.is_nan() {
                v.x = w as f64 * fx;
            }
            if v.y.is_nan() {
                v.y = h as f64 * fy;
            }
        }
    }
}

/// FilmCraft's graphic layers (text and shapes) are not effects in Kerf.
pub mod graphic {}
