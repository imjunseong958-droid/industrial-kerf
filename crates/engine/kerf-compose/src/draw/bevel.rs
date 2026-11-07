//! How far each pixel of a solid's picture is from the outline of its alpha, for its bevel.
//!
//! A bevel takes a solid's outline in as a ray goes from a face toward its middle, so a ray has to ask,
//! at each step, how far in it is. Jump flooding answers that for a whole picture in as many passes as
//! the distance takes bits. The GPU goes by the same passes (`kerf-gpu/src/distance.wgsl`), so the two
//! agree where the exact answer would be no better: an outline's place comes to within a fraction of a
//! pixel from its alpha, which keeps the slope of a bevel smooth.

use kerf_draw::{Image, Rect};
use kerf_gpu::kept::Store;
use rayon::prelude::*;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Where a pixel with no point of the outline near it says it is.
const NOWHERE: [f32; 2] = [-1e6, -1e6];

/// How far jumps start from, which reaches twice as far as any bevel asks, and how far is told apart.
pub(super) fn first_jump(width: f64) -> u32 {
    (width.ceil() as u32 + 4).next_power_of_two()
}

/// The jumps of each pass, halving from the first, and the shortest once more for what the others just
/// miss.
pub(super) fn jumps(first_jump: u32) -> Vec<u32> {
    let mut jumps: Vec<u32> =
        std::iter::successors(Some(first_jump), |jump| (*jump > 1).then_some(*jump / 2)).collect();
    jumps.push(1);
    jumps
}

/// How far from the outline counts, either way: what comes of [`first_jump`] and the jumps after it.
pub(super) fn reach(first_jump: u32) -> f32 {
    (first_jump * 2) as f32
}

/// Kept distance fields, by all that goes into them: 33 MB at 4K, where making one takes 476 ms a frame
/// and hashing a picture's alpha 3, so this is four at 4K.
const KEPT_FIELD_BYTES: usize = 136 << 20;

fn kept_fields() -> &'static Mutex<Store<Arc<Vec<f32>>>> {
    static KEPT: OnceLock<Mutex<Store<Arc<Vec<f32>>>>> = OnceLock::new();
    KEPT.get_or_init(|| Mutex::new(Store::new(KEPT_FIELD_BYTES)))
}

/// What a distance field goes by: the alpha it is made from, the crop it stops at, and how far it asks.
fn field_key(picture: &Image, crop: Rect, width: f64, stretch: f64) -> String {
    let rows = picture
        .px
        .par_chunks(picture.w.max(1) * 4)
        .map(|row| {
            row.chunks_exact(4)
                .fold(0xcbf2_9ce4_8422_2325u64, |hash, pixel| {
                    (hash ^ u64::from(pixel[3].to_bits())).wrapping_mul(0x0000_0100_0000_01b3)
                })
        })
        .collect::<Vec<u64>>();
    let alpha = rows.iter().fold(0u64, |hash, row| {
        (hash.rotate_left(7) ^ row).wrapping_mul(0x9E37_79B9_7F4A_7C15)
    });
    format!(
        "{}x{}|{alpha:x}|{:x},{:x},{:x},{:x}|{:x}|{:x}",
        picture.w,
        picture.h,
        crop.x.to_bits(),
        crop.y.to_bits(),
        crop.w.to_bits(),
        crop.h.to_bits(),
        first_jump(reaching(width, stretch)),
        stretch.to_bits()
    )
}

/// [`distance_field`], kept while a picture's alpha stays the same from frame to frame.
pub(super) fn kept_distance_field(
    picture: &Image,
    crop: Rect,
    width: f64,
    stretch: f64,
) -> Arc<Vec<f32>> {
    let key = field_key(picture, crop, width, stretch);
    let lock = || kept_fields().lock().unwrap_or_else(PoisonError::into_inner);
    {
        let mut kept = lock();
        kept.tick();
        if let Some(field) = kept.get(&key) {
            return field;
        }
    }
    let field = Arc::new(distance_field(picture, crop, width, stretch));
    let bytes = field.len() * 4;
    let mut kept = lock();
    if kept.wants(&key, bytes) {
        kept.put(&key, bytes, field.clone());
    }
    field
}

/// How far across a picture a width takes jumps, where a clip scaled unevenly has its pixels wider one
/// way than the other: `stretch` is as many of the mean's pixels as a pixel across is, and one down is
/// the inverse of that.
pub(super) fn reaching(width: f64, stretch: f64) -> f64 {
    width / stretch.min(1.0 / stretch)
}

/// The distance of each pixel of `picture` from the outline of its alpha within `crop`, more than 0
/// inside it and less outside, no further either way than [`reach`]. It is measured as its pixels are in
/// the world, in the mean of their two sizes, so that a clip scaled unevenly has a bevel as wide on each
/// of its sides.
pub(super) fn distance_field(picture: &Image, crop: Rect, width: f64, stretch: f64) -> Vec<f32> {
    let (w, h) = (picture.w, picture.h);
    let alpha = |x: i64, y: i64| -> f32 {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return 0.0;
        }
        let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
        if cx < crop.x || cy < crop.y || cx > crop.right() || cy > crop.bottom() {
            return 0.0;
        }
        picture.px[(y as usize * w + x as usize) * 4 + 3]
    };
    let place = |x: usize, y: usize| [x as f32 + 0.5, y as f32 + 0.5];
    let (wide, tall) = (stretch as f32, (1.0 / stretch) as f32);
    let gap = |a: [f32; 2], b: [f32; 2]| {
        let (x, y) = ((a[0] - b[0]) * wide, (a[1] - b[1]) * tall);
        x * x + y * y
    };
    // Each pixel beside the outline knows where along the way to its neighbor the alpha crosses a half.
    let mut seeds = vec![NOWHERE; w * h];
    seeds
        .par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, seed) in row.iter_mut().enumerate() {
                let (ix, iy) = (x as i64, y as i64);
                let here = alpha(ix, iy);
                let at = place(x, y);
                for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let there = alpha(ix + dx, iy + dy);
                    if (here >= 0.5) == (there >= 0.5) {
                        continue;
                    }
                    let along = (here - 0.5) / (here - there);
                    let met = [at[0] + along * dx as f32, at[1] + along * dy as f32];
                    if gap(met, at) < gap(*seed, at) {
                        *seed = met;
                    }
                }
            }
        });
    let first = first_jump(reaching(width, stretch));
    for jump in jumps(first) {
        let step = i64::from(jump);
        let from = seeds.clone();
        seeds
            .par_chunks_mut(w.max(1))
            .enumerate()
            .for_each(|(y, row)| {
                for (x, seed) in row.iter_mut().enumerate() {
                    let at = place(x, y);
                    let mut best = *seed;
                    for dy in [-step, 0, step] {
                        for dx in [-step, 0, step] {
                            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                                continue;
                            }
                            let there = from[ny as usize * w + nx as usize];
                            if gap(there, at) < gap(best, at) {
                                best = there;
                            }
                        }
                    }
                    *seed = best;
                }
            });
    }
    let far = reach(first);
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let at = place(x, y);
                let seed = seeds[y * w + x];
                let distance = gap(seed, at).sqrt().min(far);
                *value = if alpha(x as i64, y as i64) >= 0.5 {
                    distance
                } else {
                    -distance
                };
            }
        });
    out
}

/// A distance field at a place of its picture, between its pixels, its edge carrying on past it.
pub(super) fn distance_at(field: &[f32], (w, h): (usize, usize), x: f64, y: f64) -> f64 {
    let fx = (x - 0.5).clamp(0.0, w as f64 - 1.0);
    let fy = (y - 0.5).clamp(0.0, h as f64 - 1.0);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
    let at = |x: usize, y: usize| f64::from(field[y * w + x]);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

/// How much of a pixel `distance` inside the outline is left with the outline taken in by `inset`.
pub(super) fn left_of(distance: f64, inset: f64) -> f64 {
    (distance - inset + 0.5).clamp(0.0, 1.0)
}

/// `picture` with its outline taken in by `inset` pixels.
pub(super) fn eroded(picture: &Image, field: &[f32], inset: f64) -> Image {
    let mut out = picture.clone();
    out.px
        .par_chunks_mut(4)
        .zip(field.par_iter())
        .for_each(|(pixel, distance)| {
            let left = left_of(f64::from(*distance), inset) as f32;
            for value in pixel {
                *value *= left;
            }
        });
    out
}

#[cfg(test)]
mod kept_tests {
    use super::*;

    fn disk(w: usize, h: usize, radius: f32) -> Image {
        let mut image = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0);
                let a = (radius - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                image.px[(y * w + x) * 4 + 3] = a;
            }
        }
        image
    }

    #[test]
    fn a_kept_field_is_the_field_of_that_alpha() {
        let whole = Rect::new(0.0, 0.0, 64.0, 48.0);
        let (one, other) = (disk(64, 48, 17.5), disk(64, 48, 11.5));
        let wanted = distance_field(&one, whole, 6.0, 1.0);
        // Made, made and kept, and then read back.
        for _ in 0..3 {
            assert_eq!(*kept_distance_field(&one, whole, 6.0, 1.0), wanted);
        }
        // Another alpha, crop, or reach is another field.
        assert_eq!(
            *kept_distance_field(&other, whole, 6.0, 1.0),
            distance_field(&other, whole, 6.0, 1.0)
        );
        let cropped = Rect::new(8.0, 8.0, 32.0, 32.0);
        assert_eq!(
            *kept_distance_field(&one, cropped, 6.0, 1.0),
            distance_field(&one, cropped, 6.0, 1.0)
        );
        assert_eq!(
            *kept_distance_field(&one, whole, 40.0, 1.0),
            distance_field(&one, whole, 40.0, 1.0)
        );
        // Scaled unevenly, it measures as the world does: a pixel across counts for 2 here, and one down
        // for a half, so that an edge 4 pixels away comes to 8 across it and 2 down.
        let mut rectangle = Image::new(64, 48);
        for y in 4..44 {
            for x in 8..56 {
                rectangle.px[(y * 64 + x) * 4 + 3] = 1.0;
            }
        }
        let stretched = kept_distance_field(&rectangle, whole, 6.0, 2.0);
        let at = |field: &[f32], x: usize, y: usize| field[y * 64 + x];
        let near = |found: f32, wanted: f32| (found - wanted).abs() < 1.5;
        assert!(
            near(at(&stretched, 51, 24), 8.0),
            "{}",
            at(&stretched, 51, 24)
        );
        assert!(
            near(at(&stretched, 32, 39), 2.0),
            "{}",
            at(&stretched, 32, 39)
        );
        let plain = distance_field(&rectangle, whole, 6.0, 1.0);
        assert!(near(at(&plain, 51, 24), 4.0), "{}", at(&plain, 51, 24));
        assert!(near(at(&plain, 32, 39), 4.0), "{}", at(&plain, 32, 39));
        // And its alpha going on the same is the same field, whatever its color does.
        let mut colored = one.clone();
        colored
            .px
            .chunks_exact_mut(4)
            .for_each(|pixel| pixel[0] = 0.5);
        assert_eq!(*kept_distance_field(&colored, whole, 6.0, 1.0), wanted);
    }
}
