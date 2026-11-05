//! The EffectCraft effects Kerf keeps beside FilmCraft's, which FilmCraft
//! has nothing like, run by Kerf's own renderer on its pictures: restoration
//! (grain, dust, rolling shutter, lens, upscaling), keying and matte
//! clean-up, curves and finer color tools, a few blurs and distortions, and
//! the audio visualizers.
//!
//! Kerf's pictures are premultiplied linear light; EffectCraft's effects
//! run on premultiplied sRGB-encoded floats, so a picture is encoded for
//! them and decoded after.

use crate::{spec, value_at};
use kerf_draw::color::{to_encoded, to_linear};
use kerf_model::{Effect, Mask, MaskShape, Param, Time, Value};
use kerf_raster_fx::{
    Buf, EffectCtx, EffectEnv, EffectHost, Image, LayerPixels, ParamUi, Params, Value as FxValue,
};

/// The kept effects, by Kerf id.
pub const KEPT: &[&str] = &[
    "blur.bilateral",
    "blur.radialzoom",
    "color.autocolor",
    "color.autocontrast",
    "color.autolevels",
    "color.curves",
    "color.huesaturation",
    "color.photofilter",
    "color.selectivecolor",
    "color.shadowhighlight",
    "distort.bulge",
    "distort.opticscompensation",
    "distort.rollingshutterrepair",
    "distort.upscale",
    "generate.audiospectrum",
    "generate.audiowaveform",
    "key.advancedspill",
    "key.keycleaner",
    "key.unmult",
    "matte.simplechoker",
    "noise.addgrain",
    "noise.dustscratches",
    "noise.removegrain",
    "stylize.motiontile",
];

/// Whether Kerf's renderer runs the raster effect `id`.
pub fn kept(id: &str) -> bool {
    KEPT.contains(&id)
}

/// What a kept effect reads beyond its picture.
pub trait Host: Sync {
    /// The clip's picture at clip time `at` as the effect at `before` among
    /// its effects gets it: masked, with the effects before it run.
    fn picture_at(&self, at: Time, before: usize) -> Option<kerf_draw::Image>;
    /// `frames` stereo frames (interleaved) of sound track `track` from
    /// sequence time `start` seconds, at `rate` Hz.
    fn sound(&self, track: &str, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>>;
}

/// How a kept effect runs on a clip's picture.
pub struct Run<'a> {
    /// Clip time.
    pub at: Time,
    /// Sequence time, in seconds.
    pub sequence_time: f64,
    pub frame_rate: f64,
    /// The effect's place among the clip's effects.
    pub index: usize,
    /// Makes random effects differ between clips and stay the same between
    /// renders.
    pub seed: u32,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f64,
    /// The clip's masks, which effects read as paths.
    pub masks: &'a [Mask],
    /// The sequence's sound tracks, by id.
    pub sounds: &'a [String],
    pub host: Option<&'a dyn Host>,
}

/// Run the kept effect `effect` on `picture`, premultiplied linear light.
pub fn apply(picture: &mut kerf_draw::Image, effect: &Effect, run: &Run) {
    let Some(spec) = spec(effect.fx.as_str()).filter(|_| kept(effect.fx.as_str())) else {
        return;
    };
    let per_source = run.per_source.max(1e-6);
    // The layer is the picture in source pixels.
    let layer_size = [picture.w as f64 / per_source, picture.h as f64 / per_source];
    let masks: Vec<kerf_raster_fx::MaskShape> = run
        .masks
        .iter()
        .map(|mask| kerf_raster_fx::MaskShape {
            name: mask.name.to_string(),
            points: kerf_draw::mask::outline(&mask.shape, layer_size[0], layer_size[1], 1.0)
                .into_iter()
                .map(|[x, y]| [f64::from(x), f64::from(y)])
                .collect(),
            closed: !matches!(mask.shape, MaskShape::Polygon { closed: false, .. }),
            inverted: mask.inverted,
        })
        .collect();
    let mut values = Params::default();
    for param in &spec.params {
        let value = match param.ui {
            // A mask by name, from 1; 0 for none.
            ParamUi::Mask => FxValue::Scalar(
                text(effect.params.get(param.id))
                    .and_then(|name| run.masks.iter().position(|mask| mask.name.as_str() == name))
                    .map_or(0.0, |index| (index + 1) as f64),
            ),
            // A sound track by id, from 1; another picture track reads as
            // none, as no kept effect reads one.
            ParamUi::Layer => FxValue::Layer(
                text(effect.params.get(param.id))
                    .and_then(|track| run.sounds.iter().position(|id| *id == track))
                    .map(|index| index as u64 + 1),
            ),
            _ => value_at(param, &effect.params, run.at, layer_size),
        };
        values.values.insert(param.id.to_owned(), value);
    }
    let host = run.host.map(|host| Adapter {
        host,
        sounds: run.sounds,
        per_source,
    });
    let ctx = EffectCtx {
        params: &values,
        time: run.at.seconds(),
        layer_size,
        seed: run.seed,
        adjustment: false,
        env: EffectEnv {
            masks: &masks,
            host: host.as_ref().map(|host| host as &dyn EffectHost),
            comp_time: run.sequence_time,
            frame_rate: run.frame_rate,
            effect_index: run.index,
            ..EffectEnv::default()
        },
    };
    let buf = Buf {
        img: encoded(picture),
        offset: [0.0; 2],
        scale: per_source,
    };
    let buf = kerf_raster_fx::apply(spec, &ctx, buf);
    decode_into(&buf, picture, per_source);
}

fn text(param: Option<&Param>) -> Option<String> {
    match param? {
        Param::Static(Value::Text(text) | Value::Enum(text)) if !text.is_empty() => {
            Some(text.clone())
        }
        _ => None,
    }
}

/// Kerf's picture as EffectCraft's: premultiplied, sRGB-encoded.
fn encoded(picture: &kerf_draw::Image) -> Image {
    let mut image = Image::new(picture.w as u32, picture.h as u32);
    for (pixel, linear) in image.data.iter_mut().zip(picture.px.chunks_exact(4)) {
        let alpha = linear[3];
        *pixel = if alpha <= 1e-6 {
            [0.0; 4]
        } else {
            let channel = |c: f32| to_encoded((c / alpha).max(0.0)) * alpha;
            [
                channel(linear[0]),
                channel(linear[1]),
                channel(linear[2]),
                alpha,
            ]
        };
    }
    image
}

/// EffectCraft's buffer back over the picture it came from. An effect may
/// grow the buffer or move its origin; the picture keeps its rectangle.
fn decode_into(buf: &Buf, picture: &mut kerf_draw::Image, per_source: f64) {
    let ratio = buf.scale / per_source;
    let (bw, bh) = (i64::from(buf.img.width), i64::from(buf.img.height));
    let width = picture.w;
    for (index, out) in picture.px.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((index % width) as f64, (index / width) as f64);
        let column = (x * ratio + buf.offset[0]).round() as i64;
        let row = (y * ratio + buf.offset[1]).round() as i64;
        let pixel = if (0..bw).contains(&column) && (0..bh).contains(&row) {
            buf.img.data[(row * bw + column) as usize]
        } else {
            [0.0; 4]
        };
        let alpha = pixel[3].clamp(0.0, 1.0);
        let channel = |c: f32| {
            if alpha <= 1e-6 {
                0.0
            } else {
                to_linear((c / alpha).max(0.0)) * alpha
            }
        };
        out.copy_from_slice(&[
            channel(pixel[0]),
            channel(pixel[1]),
            channel(pixel[2]),
            alpha,
        ]);
    }
}

/// Kerf's host as EffectCraft's.
struct Adapter<'a> {
    host: &'a dyn Host,
    sounds: &'a [String],
    per_source: f64,
}

impl EffectHost for Adapter<'_> {
    fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
        None
    }

    fn audio(&self, id: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
        let track = self.sounds.get(usize::try_from(id).ok()?.checked_sub(1)?)?;
        self.host.sound(track, start, frames, rate)
    }

    fn self_at(&self, layer_time: f64, effects: usize) -> Option<Buf> {
        let picture = self
            .host
            .picture_at(Time::from_seconds(layer_time)?, effects)?;
        Some(Buf {
            img: encoded(&picture),
            offset: [0.0; 2],
            scale: self.per_source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Catalog, Engine, Id};

    fn effect(id: &str, params: &[(&str, Param)]) -> Effect {
        Effect {
            fx: Id::new(id).unwrap(),
            version: 1,
            enabled: true,
            params: params
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        }
    }

    fn run<'a>() -> Run<'a> {
        Run {
            at: Time::ZERO,
            sequence_time: 0.0,
            frame_rate: 24.0,
            index: 0,
            seed: 0,
            per_source: 1.0,
            masks: &[],
            sounds: &[],
            host: None,
        }
    }

    #[test]
    fn every_kept_effect_is_a_raster_effect_that_runs() {
        for id in KEPT {
            let fx = Catalog::builtin().fx(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(fx.engine, Engine::Raster, "{id}");
            let mut picture = kerf_draw::Image::filled(24, 16, [0.2, 0.3, 0.1, 1.0]);
            apply(&mut picture, &effect(id, &[]), &run());
            assert!(picture.px.iter().all(|v| v.is_finite()), "{id}");
        }
    }

    #[test]
    fn a_kept_effect_runs_in_encoded_light() {
        // Hue/Saturation at no change leaves the picture as it was.
        let mut picture = kerf_draw::Image::filled(8, 8, [0.2, 0.05, 0.6, 1.0]);
        let before = picture.px.clone();
        apply(&mut picture, &effect("color.huesaturation", &[]), &run());
        assert!(picture
            .px
            .iter()
            .zip(&before)
            .all(|(a, b)| (a - b).abs() < 1e-3));
        // Desaturated fully, it is grey.
        let grey = effect(
            "color.huesaturation",
            &[("saturation", Param::Static(Value::Number(-100.0)))],
        );
        apply(&mut picture, &grey, &run());
        let p = &picture.px[..4];
        assert!(
            (p[0] - p[1]).abs() < 1e-3 && (p[1] - p[2]).abs() < 1e-3,
            "{p:?}"
        );
    }
}
