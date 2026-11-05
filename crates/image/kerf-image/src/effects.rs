//! This area's effects.

use crate::fill::{dilate, membrane_fill, opaque_fill, patch_fill};
use kerf_draw::color::{to_encoded, to_linear};
use kerf_draw::effect::{EffectRun, PictureEffect};
use kerf_draw::mask;
use kerf_draw::Image;
use kerf_model::{Effect, MaskMode};
use kerf_raster::Image as Raster;
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

/// Every effect of this area.
pub const EFFECTS: &[&dyn PictureEffect] = &[&Fill];

/// `image.fill`: the area a mask draws, filled from the rest of the picture.
struct Fill;

impl PictureEffect for Fill {
    fn id(&self) -> &'static str {
        "image.fill"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let Some(hole) = hole(picture, effect, run) else {
            return;
        };
        let method = run.text(effect, "method");
        let patch = run.number(effect, "patch").round() as usize;
        let key = fill_key(picture, &hole, method.as_deref(), patch, run.seed);
        let pixels = kept_or(key, || {
            let source = encoded(picture);
            let filled = match method.as_deref() {
                Some("smooth") => membrane_fill(&source, &hole),
                _ => patch_fill(&source, &hole, None, patch, run.seed),
            };
            linear_in_hole(&opaque_fill(filled, &hole), &hole)
        });
        let holes = picture
            .px
            .chunks_exact_mut(4)
            .zip(&hole)
            .filter(|(_, hole)| **hole);
        for ((pixel, _), value) in holes.zip(pixels.iter()) {
            pixel.copy_from_slice(value);
        }
    }
}

/// What the fills made and may be asked for again, newest last. A still, or
/// a stretch of identical frames, fills once rather than once a frame, which
/// is the difference between a second and minutes: filling a large area takes
/// seconds.
static FILLS: Mutex<Vec<(u64, Arc<Vec<[f32; 4]>>)>> = Mutex::new(Vec::new());

/// The most pixels [`FILLS`] keeps: 512 MB of them.
const MOST_KEPT_PIXELS: usize = 512 << 20 >> 4;

/// The fill kept under `key`, or what `make` makes of it, which is then kept.
fn kept_or(key: u64, make: impl FnOnce() -> Vec<[f32; 4]>) -> Arc<Vec<[f32; 4]>> {
    let lock = || FILLS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, kept)) = lock().iter().find(|(kept, _)| *kept == key) {
        return kept.clone();
    }
    // Other fills go on while this one is made.
    let made = Arc::new(make());
    let mut fills = lock();
    fills.push((key, made.clone()));
    let mut kept: usize = fills.iter().map(|(_, pixels)| pixels.len()).sum();
    while kept > MOST_KEPT_PIXELS && fills.len() > 1 {
        kept -= fills.remove(0).1.len();
    }
    made
}

/// Everything a fill's result depends on, in one number.
fn fill_key(picture: &Image, hole: &[bool], method: Option<&str>, patch: usize, seed: u32) -> u64 {
    let rows: Vec<u64> = picture
        .px
        .par_chunks(picture.w.max(1) * 4)
        .map(|row| {
            row.iter().fold(0xCBF2_9CE4_8422_2325, |hash: u64, value| {
                (hash ^ u64::from(value.to_bits())).wrapping_mul(0x0000_0100_0000_01B3)
            })
        })
        .collect();
    let mut hash = DefaultHasher::new();
    (picture.w, picture.h, rows, hole, method, patch, seed).hash(&mut hash);
    hash.finish()
}

/// The pixels the effect's mask covers, grown by its `expand`: a mask whose
/// `mode=none` leaves the picture whole for the effect to read. `None` when
/// the effect names no mask of the clip, or the mask covers nothing.
fn hole(picture: &Image, effect: &Effect, run: &EffectRun) -> Option<Vec<bool>> {
    let name = run.text(effect, "path")?;
    let mut outline = run
        .masks
        .iter()
        .find(|mask| mask.name.as_str() == name)?
        .clone();
    outline.mode = MaskMode::Add;
    let per_source = run.per_source.max(1e-6);
    let size = (picture.w as f64 / per_source, picture.h as f64 / per_source);
    let flat = mask::prepare(std::slice::from_ref(&outline), size, per_source, run.at);
    let covered = mask::coverage(&flat, picture.w, picture.h)?;
    let hole: Vec<bool> = covered.iter().map(|coverage| *coverage > 0.5).collect();
    if !hole.contains(&true) {
        return None;
    }
    let grow = (run.number(effect, "expand").max(0.0) * per_source).round() as usize;
    Some(dilate(&hole, picture.w, picture.h, grow))
}

/// A picture as patches compare best: premultiplied, sRGB-encoded.
fn encoded(picture: &Image) -> Raster {
    let data = picture
        .px
        .chunks_exact(4)
        .map(|pixel| {
            let a = pixel[3];
            if a <= 1e-6 {
                return [0.0; 4];
            }
            let channel = |value: f32| to_encoded(value / a) * a;
            [channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), a]
        })
        .collect();
    Raster {
        width: picture.w as u32,
        height: picture.h as u32,
        data,
    }
}

/// The pixels of `filled` in the hole, in order, back in linear light.
fn linear_in_hole(filled: &Raster, hole: &[bool]) -> Vec<[f32; 4]> {
    filled
        .data
        .iter()
        .zip(hole)
        .filter(|(_, hole)| **hole)
        .map(|(value, _)| {
            let a = value[3];
            if a <= 1e-6 {
                return [0.0; 4];
            }
            let channel = |value: f32| to_linear(value / a) * a;
            [channel(value[0]), channel(value[1]), channel(value[2]), a]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Id, Mask, MaskShape, Param, Time, Value};
    use std::collections::BTreeMap;

    /// A flat gray picture with a bright square in the middle.
    fn with_square(w: usize, h: usize) -> Image {
        let mut picture = Image::filled(w, h, [0.2, 0.2, 0.2, 1.0]);
        for y in h / 2 - 4..h / 2 + 4 {
            for x in w / 2 - 4..w / 2 + 4 {
                picture.px[(y * w + x) * 4..][..3].copy_from_slice(&[1.0, 1.0, 1.0]);
            }
        }
        picture
    }

    fn fill(method: &str, name: &str) -> Effect {
        Effect {
            fx: Id::new("image.fill").expect("an id"),
            version: 1,
            enabled: true,
            params: BTreeMap::from([
                (
                    "path".to_owned(),
                    Param::Static(Value::Text(name.to_owned())),
                ),
                (
                    "method".to_owned(),
                    Param::Static(Value::Enum(method.to_owned())),
                ),
            ]),
        }
    }

    fn masks() -> Vec<Mask> {
        let mut mask = Mask::new(
            Id::new("k1").expect("an id"),
            MaskShape::Rect([0.3, 0.3, 0.7, 0.7]),
        );
        mask.mode = MaskMode::None;
        vec![mask]
    }

    fn run(masks: &[Mask]) -> EffectRun<'_> {
        EffectRun {
            at: Time::ZERO,
            per_source: 1.0,
            seed: 1,
            masks,
        }
    }

    #[test]
    fn a_filled_square_is_gone_and_the_rest_is_untouched() {
        for method in ["object", "smooth"] {
            let masks = masks();
            let mut picture = with_square(48, 32);
            let before = picture.clone();
            Fill.apply(&mut picture, &fill(method, "k1"), &run(&masks));
            let at = (16 * 48 + 24) * 4;
            assert!(
                (picture.px[at] - 0.2).abs() < 0.02,
                "{method}: {}",
                picture.px[at]
            );
            assert_eq!(picture.px[..4], before.px[..4], "{method}");
            assert_eq!(picture.px[at + 3], 1.0, "{method}");
        }
    }

    #[test]
    fn a_fill_is_made_once_for_the_same_picture() {
        const KEY: u64 = 0x5EED;
        let first = kept_or(KEY, || vec![[0.5; 4]]);
        let again = kept_or(KEY, || panic!("it was kept"));
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[test]
    fn a_different_picture_or_setting_is_a_different_fill() {
        let picture = with_square(16, 12);
        let hole = vec![false; 16 * 12];
        let key = fill_key(&picture, &hole, None, 7, 1);
        assert_eq!(key, fill_key(&picture.clone(), &hole, None, 7, 1));
        assert_ne!(key, fill_key(&picture, &hole, Some("smooth"), 7, 1));
        assert_ne!(key, fill_key(&picture, &hole, None, 9, 1));
        let mut changed = picture.clone();
        changed.px[100] += 0.01;
        assert_ne!(key, fill_key(&changed, &hole, None, 7, 1));
    }

    #[test]
    fn without_its_mask_the_picture_stays_as_it_was() {
        let masks = masks();
        let mut picture = with_square(48, 32);
        let before = picture.clone();
        Fill.apply(&mut picture, &fill("object", "k9"), &run(&masks));
        assert_eq!(picture, before);
    }
}
