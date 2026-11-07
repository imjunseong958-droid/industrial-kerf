//! How far each pixel of a solid's picture is from the outline of its alpha, for its bevel.
//!
//! A bevel takes a solid's outline in as a ray goes from a face toward its middle, so a ray has to ask,
//! at each step, how far in it is. Jump flooding answers that for a whole picture in as many passes as
//! the distance takes bits. The GPU goes by the same passes (`kerf-gpu/src/distance.wgsl`), so the two
//! agree where the exact answer would be no better: an outline's place comes to within a fraction of a
//! pixel from its alpha, which keeps the slope of a bevel smooth.

use kerf_draw::{Image, Rect};
use rayon::prelude::*;

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

/// The distance of each pixel of `picture` from the outline of its alpha within `crop`, in pixels, more
/// than 0 inside it and less outside, no further either way than [`reach`].
pub(super) fn distance_field(picture: &Image, crop: Rect, width: f64) -> Vec<f32> {
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
    let gap = |a: [f32; 2], b: [f32; 2]| {
        let (x, y) = (a[0] - b[0], a[1] - b[1]);
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
    let first = first_jump(width);
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
