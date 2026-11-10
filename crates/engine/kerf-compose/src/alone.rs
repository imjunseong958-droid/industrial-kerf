//! A clip's effects run on one picture of it alone, outside its sequence,
//! as a preview of them does: in the order of the stack and by the code a
//! render runs, without the clip's other frames or the sequence's other
//! tracks.

use crate::domains;
use kerf_color::WorkingSpace;
use kerf_draw::effect::{EffectRun, Files};
use kerf_draw::Image;
use kerf_model::time::format_timecode;
use kerf_model::{Effect, Mask, Rate, Time};

/// Whether the compositor has code for the effect `id`; a look has none of
/// its own, being other effects by the time it is drawn.
pub fn draws_effect(id: &str) -> bool {
    domains::effect(id).is_some()
}

/// A picture of a clip, and where in the clip it is from.
pub struct Alone<'a> {
    /// The picture's size, in its pixels.
    pub size: (usize, usize),
    /// Clip time.
    pub at: Time,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f64,
    /// How many of the picture's pixels a pixel of the sequence's canvas is.
    pub per_canvas: f64,
    /// The sequence's rate.
    pub rate: Rate,
    /// How long the clip is.
    pub duration: Time,
    /// The clip's seed (`kerf_core::raster::seed`).
    pub seed: u32,
    /// The clip's masks, which effects read as paths.
    pub masks: &'a [Mask],
}

/// Run the enabled ones of `effects` in order on `rgba`, straight 8-bit
/// RGBA, shown over black. An effect with no code is passed over. A file
/// an effect names that cannot be read is an error naming it.
pub fn effects_on_picture(
    rgba: &mut [u8],
    effects: &[Effect],
    alone: &Alone,
) -> Result<(), String> {
    let enabled = || effects.iter().filter(|effect| effect.enabled);
    let mut files = Files::new();
    for effect in enabled() {
        if let Some(code) = domains::effect(effect.fx.as_str()) {
            code.prepare(effect, &mut files)?;
        }
    }
    let (w, h) = alone.size;
    let mut picture = Image::from_rgba8(rgba, w, h, w * 4, true);
    let working = WorkingSpace::Rec709;
    let timecode = format_timecode(alone.at, alone.rate, false);
    for (index, effect) in enabled().enumerate() {
        if kerf_fx::filmcraft::instance::leaves_alone(effect, alone.at, working) {
            continue;
        }
        let Some(code) = domains::effect(effect.fx.as_str()) else {
            continue;
        };
        let run = EffectRun {
            at: alone.at,
            sequence_at: alone.at,
            per_source: alone.per_source,
            per_canvas: alone.per_canvas,
            index,
            nested: false,
            seed: alone.seed.wrapping_add(index as u32),
            masks: alone.masks,
            frame_rate: alone.rate.fps(),
            duration: alone.duration,
            working,
            timecode: &timecode,
            clip_name: "",
            tracks: &[],
            sounds: &[],
            files: &files,
            host: None,
        };
        code.apply(&mut picture, effect, &run);
    }
    rgba.copy_from_slice(&picture.to_rgba8_over_black());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Catalog, Id, Param, Params, Value};

    fn effect(id: &str, params: &[(&str, f64)]) -> Effect {
        let def = Catalog::builtin().fx(id).expect("in the catalog");
        Effect {
            fx: Id::new(id).expect("an id"),
            version: def.version,
            enabled: true,
            params: params
                .iter()
                .map(|(name, value)| ((*name).to_owned(), Param::Static(Value::Number(*value))))
                .collect::<Params>(),
        }
    }

    fn alone() -> Alone<'static> {
        Alone {
            size: (16, 8),
            at: Time::ZERO,
            per_source: 1.0,
            per_canvas: 1.0,
            rate: Rate::FPS_24,
            duration: Time::from_seconds(1.0).expect("a time"),
            seed: 1,
            masks: &[],
        }
    }

    /// Mid grey with a red left half.
    fn picture() -> Vec<u8> {
        (0..16 * 8)
            .flat_map(|at| {
                if at % 16 < 8 {
                    [200, 40, 40, 255]
                } else {
                    [128, 128, 128, 255]
                }
            })
            .collect()
    }

    #[test]
    fn effects_of_every_kind_run_in_the_order_of_the_stack() {
        // One of EffectCraft's, one of FilmCraft's, and one every Kerf has had: the order they
        // are given in is the order they run in.
        let (invert, grey) = (
            effect("channel.invert", &[]),
            effect("color.basic", &[("saturation", 0.0)]),
        );
        let kept = effect("color.huesaturation", &[("hue", 120.0)]);
        let mut one = picture();
        effects_on_picture(
            &mut one,
            &[kept.clone(), invert.clone(), grey.clone()],
            &alone(),
        )
        .unwrap();
        let mut other = picture();
        effects_on_picture(&mut other, &[grey, invert, kept], &alone()).unwrap();
        assert_ne!(one, picture());
        assert_ne!(one, other);
        // Drained last, nothing has color.
        let spread = |picture: &[u8]| {
            let pixels = picture.as_chunks::<4>().0;
            pixels
                .iter()
                .map(|p| p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap())
                .max()
        };
        assert!(spread(&one) <= Some(8), "{:?}", spread(&one));
    }

    #[test]
    fn a_disabled_effect_and_one_with_no_code_are_passed_over() {
        let mut off = effect("channel.invert", &[]);
        off.enabled = false;
        let look = Effect {
            fx: Id::new("look.shake").expect("an id"),
            version: 1,
            enabled: true,
            params: Params::new(),
        };
        assert!(!draws_effect("look.shake") && draws_effect("channel.invert"));
        let mut drawn = picture();
        effects_on_picture(&mut drawn, &[off, look], &alone()).unwrap();
        assert_eq!(drawn, picture());
    }
}
