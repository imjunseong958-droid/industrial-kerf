//! How an area's effect runs on a clip's picture. An area's crate (`image`,
//! `vfx`, `motion`, `color`) implements [`PictureEffect`] for each effect it
//! has and lists them; the compositor finds them by id, so a new effect
//! changes that area's crate and the catalog, not the compositor.

use crate::Image;
use kerf_model::{Catalog, Effect, Mask, Time, Value};

/// What an effect may read beyond its picture.
pub struct EffectRun<'a> {
    /// Clip time.
    pub at: Time,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f64,
    /// Makes random effects differ between clips and stay the same between
    /// renders.
    pub seed: u32,
    /// The clip's masks, which effects read as paths.
    pub masks: &'a [Mask],
}

impl EffectRun<'_> {
    /// A parameter's value at the run's time, or its default.
    fn value(&self, effect: &Effect, name: &str) -> Option<Value> {
        match effect.params.get(name) {
            Some(param) => Some(param.value_at(self.at)),
            None => Catalog::builtin()
                .fx(effect.fx.as_str())
                .and_then(|def| def.find(name))
                .map(|def| def.default.clone()),
        }
    }

    /// A number parameter, or 0 when it has none.
    pub fn number(&self, effect: &Effect, name: &str) -> f64 {
        self.value(effect, name)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0)
    }

    /// A text, choice, or mask parameter, or `None` when it is empty.
    pub fn text(&self, effect: &Effect, name: &str) -> Option<String> {
        match self.value(effect, name)? {
            Value::Text(text) | Value::Enum(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }
}

/// An effect an area's crate draws on a clip's picture, premultiplied linear
/// light, and masked by the clip's cutting masks.
pub trait PictureEffect: Sync {
    /// Its id in the catalog, as a document names it.
    fn id(&self) -> &'static str;

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun);
}
