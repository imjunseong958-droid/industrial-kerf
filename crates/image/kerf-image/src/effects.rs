//! This area's effects.

use crate::fill::{dilate, fill_frames, membrane_fill, opaque_fill, patch_fill};
use crate::stroke::{clone_stroke, find_source, heal_stroke, Stroke};
use crate::warp::warp;
use kerf_draw::color::{to_encoded, to_linear};
use kerf_draw::effect::{EffectRun, PictureEffect};
use kerf_draw::mask;
use kerf_draw::Image;
use kerf_model::{parse_moves, parse_points, Effect, MaskMode, Time};
use kerf_raster::Image as Raster;
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

/// Every effect of this area.
pub const EFFECTS: &[&dyn PictureEffect] = &[&Fill, &Clone, &Heal, &Warp];

/// `image.clone`: a stroke painted with the picture from elsewhere.
struct Clone;

impl PictureEffect for Clone {
    fn id(&self) -> &'static str {
        "image.clone"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        paint(picture, effect, run, clone_stroke);
    }
}

/// `image.heal`: a stroke with the detail of elsewhere, and the light and color of where it goes.
struct Heal;

impl PictureEffect for Heal {
    fn id(&self) -> &'static str {
        "image.heal"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        paint(picture, effect, run, heal_stroke);
    }
}

/// `image.warp`: a mesh over the picture, dragged by moves.
struct Warp;

impl PictureEffect for Warp {
    fn id(&self) -> &'static str {
        "image.warp"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let Some(moves) = run.text(effect, "moves") else {
            return;
        };
        let Ok(chains) = parse_moves(&moves) else {
            return;
        };
        let place = placing(picture, effect, run);
        let chains: Vec<Vec<[f32; 2]>> = chains
            .iter()
            .map(|chain| chain.iter().map(|point| place(*point)).collect())
            .collect();
        let radius = (run.number(effect, "radius") * picture.h as f64) as f32;
        warp(
            picture,
            &chains,
            radius,
            run.number(effect, "amount") as f32,
        );
    }
}

/// What turns a point of the document, in fractions of the picture, into its pixels, moved with the
/// picture by the effect's `move_x` and `move_y`.
fn placing(picture: &Image, effect: &Effect, run: &EffectRun) -> impl Fn([f64; 2]) -> [f32; 2] {
    let (w, h) = (picture.w as f64, picture.h as f64);
    let moved = [run.number(effect, "move_x"), run.number(effect, "move_y")];
    move |point| {
        [
            ((point[0] + moved[0]) * w) as f32,
            ((point[1] + moved[1]) * h) as f32,
        ]
    }
}

/// Paint the stroke an effect says, from where it says, with `paint`.
fn paint(
    picture: &mut Image,
    effect: &Effect,
    run: &EffectRun,
    paint: fn(&mut Image, Option<&Image>, &Stroke, [f32; 2]),
) {
    let Some(points) = run.text(effect, "points") else {
        return;
    };
    let Ok(points) = parse_points(&points) else {
        return;
    };
    let place = placing(picture, effect, run);
    let stroke = Stroke {
        points: points.into_iter().map(place).collect(),
        radius: (run.number(effect, "radius") * picture.h as f64) as f32,
        hardness: run.number(effect, "hardness") as f32,
        opacity: run.number(effect, "opacity") as f32,
    };
    let (w, h) = (picture.w as f64, picture.h as f64);
    let mut from = [
        (run.number(effect, "from_x") * w) as f32,
        (run.number(effect, "from_y") * h) as f32,
    ];
    let frames = run.number(effect, "from_frame").round();
    if from == [0.0, 0.0] && frames == 0.0 {
        // Told nowhere to paint from, it finds a place.
        match find_source(picture, &stroke) {
            Some(found) => from = found,
            None => return,
        }
    }
    let other = (frames != 0.0)
        .then(|| other_frame(picture, run, frames))
        .flatten();
    paint(picture, other.as_ref(), &stroke, from);
}

/// The clip's picture `frames` away, held to its ends; `None` when it has no other frames to read.
fn other_frame(picture: &Image, run: &EffectRun, frames: f64) -> Option<Image> {
    let clip = run.frames()?;
    let frame = 1.0 / run.frame_rate.max(1.0);
    let last = (run.duration.seconds() - frame).max(0.0);
    let time = Time::from_seconds((run.at.seconds() + frames * frame).clamp(0.0, last))?;
    clip.at(time)
        .filter(|other| (other.w, other.h) == (picture.w, picture.h))
}

/// `image.fill`: the area a mask draws, filled from the rest of the picture.
struct Fill;

impl PictureEffect for Fill {
    fn id(&self) -> &'static str {
        "image.fill"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let Some(hole) = hole(picture, effect, run, run.at) else {
            return;
        };
        let method = run.text(effect, "method");
        let patch = run.number(effect, "patch").round() as usize;
        let key = fill_key(picture, &hole, &run_method(effect, run), patch, run.seed);
        let pixels = kept_or(key, || {
            if method.as_deref() == Some("video") {
                if let Some(pixels) = fill_run(picture, effect, run, patch, &hole) {
                    return pixels;
                }
            }
            let source = encoded(picture);
            let filled = match method.as_deref() {
                Some("smooth") => membrane_fill(&source, &hole),
                _ => patch_fill(&source, &hole, None, patch, run.seed),
            };
            linear_in_hole(&opaque_fill(filled, &hole), &hole)
        });
        let holes = picture
            .px
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&hole)
            .filter(|(_, hole)| **hole);
        for ((pixel, _), value) in holes.zip(pixels.iter()) {
            pixel.copy_from_slice(value);
        }
    }
}

/// What a fill is told to do, as its key knows it.
fn run_method(effect: &Effect, run: &EffectRun) -> String {
    let method = run.text(effect, "method").unwrap_or_default();
    match method.as_str() {
        "video" => format!("video {}", run.number(effect, "reach").round()),
        _ => method,
    }
}

/// Fill the run of frames around this one together and keep them all, so that the frames of a run
/// come at the cost of one and agree with each other. Runs fall on fixed frames of the clip, so that
/// any frame asked for first gives the same fills. This frame's fill, or `None` without other frames
/// to read.
fn fill_run(
    picture: &Image,
    effect: &Effect,
    run: &EffectRun,
    patch: usize,
    hole_here: &[bool],
) -> Option<Vec<[f32; 4]>> {
    let clip = run.frames()?;
    let frame_seconds = 1.0 / run.frame_rate.max(1.0);
    let length = run.number(effect, "reach").round().max(1.0) as i64 * 2 + 1;
    let here = (run.at.seconds() / frame_seconds + 1e-6).floor() as i64;
    let first = here.div_euclid(length) * length;
    let time_of = |frame: i64| Time::from_seconds(frame as f64 * frame_seconds);
    let mut pictures = Vec::with_capacity(length as usize);
    let mut sources = Vec::with_capacity(length as usize);
    let mut holes = Vec::with_capacity(length as usize);
    let mut place = None;
    for frame in first..first + length {
        let Some(time) = time_of(frame) else {
            continue;
        };
        if time >= run.duration {
            continue;
        }
        if frame == here {
            place = Some(pictures.len());
            sources.push(encoded(picture));
            holes.push(hole_here.to_vec());
            pictures.push(picture.clone());
            continue;
        }
        let Some(other) = clip.at(time) else {
            continue;
        };
        if (other.w, other.h) != (picture.w, picture.h) {
            continue;
        }
        let Some(hole) = hole(&other, effect, run, time) else {
            // Nothing to fill there: it shows all the scene, for others to read from.
            sources.push(encoded(&other));
            holes.push(vec![false; other.w * other.h]);
            pictures.push(other);
            continue;
        };
        sources.push(encoded(&other));
        holes.push(hole);
        pictures.push(other);
    }
    let place = place?;
    let filled = fill_frames(&sources, &holes, patch, run.seed);
    let mut here_pixels = None;
    for (index, (fill, hole)) in filled.iter().zip(&holes).enumerate() {
        let pixels = linear_in_hole(fill, hole);
        if index == place {
            here_pixels = Some(pixels);
        } else if hole.contains(&true) {
            let key = fill_key(
                &pictures[index],
                hole,
                &run_method(effect, run),
                patch,
                run.seed,
            );
            keep(key, pixels);
        }
    }
    here_pixels
}

/// What the fills made and may be asked for again, newest last. A still, or
/// a stretch of identical frames, fills once rather than once a frame: filling
/// a large area takes over a second.
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
    keep_shared(key, made.clone());
    made
}

/// Keep a fill made along with another.
fn keep(key: u64, pixels: Vec<[f32; 4]>) {
    keep_shared(key, Arc::new(pixels));
}

fn keep_shared(key: u64, pixels: Arc<Vec<[f32; 4]>>) {
    let mut fills = FILLS.lock().unwrap_or_else(PoisonError::into_inner);
    if fills.iter().any(|(kept, _)| *kept == key) {
        return;
    }
    fills.push((key, pixels));
    let mut kept: usize = fills.iter().map(|(_, pixels)| pixels.len()).sum();
    while kept > MOST_KEPT_PIXELS && fills.len() > 1 {
        kept -= fills.remove(0).1.len();
    }
}

/// Everything a fill's result depends on, in one number.
fn fill_key(picture: &Image, hole: &[bool], method: &str, patch: usize, seed: u32) -> u64 {
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
fn hole(picture: &Image, effect: &Effect, run: &EffectRun, at: Time) -> Option<Vec<bool>> {
    let name = run.text(effect, "path")?;
    let mut outline = run
        .masks
        .iter()
        .find(|mask| mask.name.as_str() == name)?
        .clone();
    outline.mode = MaskMode::Add;
    let per_source = run.per_source.max(1e-6);
    let size = (picture.w as f64 / per_source, picture.h as f64 / per_source);
    let flat = mask::prepare(std::slice::from_ref(&outline), size, per_source, at);
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
        .as_chunks::<4>()
        .0
        .iter()
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
