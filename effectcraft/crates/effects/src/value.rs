//! Parameter values and how an effect's parameters are presented, as the
//! kept effects read them (from EffectCraft's `effectcraft-keyframe` and
//! `effectcraft-project`).

/// The value of a parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Scalar(f64),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    /// Straight RGBA 0..1.
    Color([f64; 4]),
    Bool(bool),
    /// Index into a popup's options, from 0.
    Enum(u32),
    /// Layer reference (layer id): the sound an audio visualizer draws.
    Layer(Option<u64>),
    Str(String),
}

impl Value {
    /// Numeric components (empty for non-numeric values).
    pub fn components(&self) -> Vec<f64> {
        match self {
            Value::Scalar(v) => vec![*v],
            Value::Vec2(v) => v.to_vec(),
            Value::Vec3(v) => v.to_vec(),
            Value::Color(v) => v.to_vec(),
            Value::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
            Value::Enum(i) => vec![*i as f64],
            _ => vec![],
        }
    }
    pub fn as_f64(&self) -> f64 {
        self.components().first().copied().unwrap_or(0.0)
    }
    pub fn as_vec2(&self) -> [f64; 2] {
        let c = self.components();
        [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)]
    }
    pub fn as_color(&self) -> [f32; 4] {
        let c = self.components();
        let g = |i: usize, d: f64| c.get(i).copied().unwrap_or(d) as f32;
        [g(0, 0.0), g(1, 0.0), g(2, 0.0), g(3, 1.0)]
    }
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            v => v.as_f64() != 0.0,
        }
    }
    pub fn as_enum(&self) -> u32 {
        match self {
            Value::Enum(i) => *i,
            v => v.as_f64().max(0.0) as u32,
        }
    }
    pub fn as_layer(&self) -> Option<u64> {
        if let Value::Layer(l) = self { *l } else { None }
    }
}

/// How a parameter is presented and edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ParamUi {
    /// Plain number with an optional valid range and a (narrower) slider range.
    #[default]
    Number,
    Slider {
        min: f64,
        max: f64,
        slider_min: f64,
        slider_max: f64,
        decimals: u8,
    },
    Percent,
    /// Angle in degrees.
    Angle,
    Pixels,
    Point,
    Point3,
    Color,
    Checkbox,
    Popup {
        options: Vec<String>,
    },
    Layer,
    /// A mask of the effect's own layer by 1-based index, 0 = none (a scalar value).
    Mask,
    Text,
    /// A file path (a string value), offering files with these extensions.
    File {
        filters: Vec<String>,
    },
    /// Not shown (internal data).
    Hidden,
}
