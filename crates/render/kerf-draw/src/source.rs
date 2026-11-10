//! How a generator draws its picture. A crate implements [`PictureSource`]
//! for each generator it has and lists them; the compositor finds them by
//! id, as it finds effects ([`crate::effect`]), so a new generator changes
//! that crate and the catalog, not the compositor.

use crate::Image;
use kerf_model::{Catalog, Clip, Content, FxDef, Params, Rgba, Time, Value};

/// Where and when a generator draws.
#[derive(Clone, Copy, Debug)]
pub struct SourceRun {
    /// Clip time.
    pub at: Time,
    /// The picture's size, in its pixels: the sequence's canvas at the scale
    /// drawn.
    pub size: (usize, usize),
}

/// A generator drawn on a transparent picture the size of the canvas,
/// premultiplied linear light.
pub trait PictureSource: Sync {
    /// Its id in the catalog, as a document names it (`gen=shape@1`).
    fn id(&self) -> &'static str;

    /// The picture of `clip`, which holds the generator's parameters.
    fn draw(&self, clip: &Clip, run: &SourceRun) -> Image;
}

/// Parameters at a moment, each its default where it is not given.
pub struct Reading<'a> {
    pub params: Option<&'a Params>,
    pub def: Option<&'a FxDef>,
    pub time: Time,
}

impl<'a> Reading<'a> {
    /// The parameters of generator `name` at `time`, from its clip.
    pub fn generator(clip: &'a Clip, name: &str, time: Time) -> Self {
        let params = match &clip.content {
            Content::Generator { params, .. } => Some(params),
            _ => None,
        };
        Self {
            params,
            def: Catalog::builtin().fx(name),
            time,
        }
    }

    fn value(&self, name: &str) -> Option<Value> {
        match self.params?.get(name) {
            Some(param) => Some(param.value_at(self.time)),
            None => self
                .def
                .and_then(|def| def.find(name))
                .map(|def| def.default.clone()),
        }
    }

    pub fn number(&self, name: &str) -> f64 {
        self.value(name)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0)
    }

    /// A color as premultiplied linear light; transparent when it has none.
    pub fn color(&self, name: &str) -> [f32; 4] {
        match self.value(name) {
            Some(Value::Color(color)) => premultiplied(color),
            _ => [0.0; 4],
        }
    }

    pub fn word(&self, name: &str) -> String {
        match self.value(name) {
            Some(Value::Enum(word) | Value::Text(word)) => word,
            _ => String::new(),
        }
    }

    pub fn on(&self, name: &str) -> bool {
        matches!(self.value(name), Some(Value::Bool(true)))
    }
}

/// A color as premultiplied linear light.
pub fn premultiplied(color: Rgba) -> [f32; 4] {
    let decode = crate::color::decode_table();
    let alpha = f32::from(color.a) / 255.0;
    [
        decode[usize::from(color.r)] * alpha,
        decode[usize::from(color.g)] * alpha,
        decode[usize::from(color.b)] * alpha,
        alpha,
    ]
}
