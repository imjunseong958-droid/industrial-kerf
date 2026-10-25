//! Parameter values, as FilmCraft's effects read them. Kerf evaluates its own keyframes: a
//! parameter that changes over time asks Kerf for its value at a time.

use std::sync::Arc;

use crate::filmcraft::tick::Tick;
use kerf_geom::Vec2;

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Float(f64),
    /// A point; a NaN coordinate is "auto" (the frame or source centre).
    Vec2(Vec2),
    Color([f32; 4]),
    Bool(bool),
    Choice(u32),
    Text(String),
    /// Curve control points (x, y) in 0..1, sorted by x (Lumetri curves).
    Curve(Vec<[f32; 2]>),
}

impl ParamValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Float(v) => Some(*v),
            ParamValue::Choice(c) => Some(*c as f64),
            ParamValue::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            ParamValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<[f32; 4]> {
        match self {
            ParamValue::Color(c) => Some(*c),
            _ => None,
        }
    }
    pub fn as_curve(&self) -> Option<&[[f32; 2]]> {
        match self {
            ParamValue::Curve(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// A parameter: its value at the time drawn, and, when it changes over time, its value at any
/// time (Transform's shutter and Write-on read it around the time drawn).
#[derive(Clone)]
pub struct Param {
    pub value: ParamValue,
    over_time: Option<Arc<dyn Fn(Tick) -> ParamValue + Send + Sync>>,
}

impl Param {
    pub fn new(value: ParamValue) -> Self {
        Self {
            value,
            over_time: None,
        }
    }
    /// A parameter whose value at `t` (clip time) is `at(t)`; `value` is its value now.
    pub fn over_time(
        value: ParamValue,
        at: impl Fn(Tick) -> ParamValue + Send + Sync + 'static,
    ) -> Self {
        Self {
            value,
            over_time: Some(Arc::new(at)),
        }
    }
    pub fn is_animated(&self) -> bool {
        self.over_time.is_some()
    }
    pub fn value_at(&self, t: Tick) -> ParamValue {
        match &self.over_time {
            Some(at) => at(t),
            None => self.value.clone(),
        }
    }
    pub fn f64_at(&self, t: Tick) -> f64 {
        self.value_at(t).as_f64().unwrap_or(0.0)
    }
    pub fn vec2_at(&self, t: Tick) -> Vec2 {
        self.value_at(t).as_vec2().unwrap_or_default()
    }
}

impl std::fmt::Debug for Param {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Param")
            .field("value", &self.value)
            .field("animated", &self.is_animated())
            .finish()
    }
}

impl PartialEq for Param {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.over_time.is_none() && other.over_time.is_none()
    }
}
