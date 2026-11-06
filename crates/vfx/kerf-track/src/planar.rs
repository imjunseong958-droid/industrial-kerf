//! Following a flat surface through frames: a screen, a sign, a wall.
//!
//! A single point cannot say how a surface turned. So corners of detail inside its outline
//! (Shi-Tomasi) are each followed with Lucas-Kanade, and a homography from the first frame to this
//! one is fitted to them by RANSAC (Fischler & Bolles 1981; Hartley & Zisserman, *Multiple View
//! Geometry*, 4.7-4.8), as EffectCraft's `track/src/fit.rs` fits one, which drops what does not move
//! with the surface: a hand passing in front, a reflection. Fitting from the first frame every time
//! keeps a small error in one frame from adding up over the rest. As the points run out, new ones
//! are found inside the outline.

use crate::pyramid;
use kerf_raster::klt::{track, LkOpts};
use kerf_raster::plane::Plane;

/// Four corners in fractions of the frame: upper left, upper right, lower right, lower left.
pub type Quad = [[f64; 2]; 4];

/// A surface's way through the frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FollowedPlane {
    /// Its outline, one a frame from the first, up to where it was lost.
    pub quads: Vec<Quad>,
    /// How many points agreed on each frame, for how much to trust it.
    pub agreeing: Vec<usize>,
    /// The frame it could no longer be found in.
    pub lost_at: Option<usize>,
}

/// A projective map of the plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn apply(&self, point: [f64; 2]) -> [f64; 2] {
        let m = &self.0;
        let w = m[2][0] * point[0] + m[2][1] * point[1] + m[2][2];
        let w = if w.abs() < 1e-12 { 1e-12 } else { w };
        [
            (m[0][0] * point[0] + m[0][1] * point[1] + m[0][2]) / w,
            (m[1][0] * point[0] + m[1][1] * point[1] + m[1][2]) / w,
        ]
    }

    /// The map back, unless it flattens the plane.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
        let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
        let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
        let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
        if det.abs() < 1e-18 || !det.is_finite() {
            return None;
        }
        Some(Self(
            [
                [
                    c00,
                    m[0][2] * m[2][1] - m[0][1] * m[2][2],
                    m[0][1] * m[1][2] - m[0][2] * m[1][1],
                ],
                [
                    c01,
                    m[0][0] * m[2][2] - m[0][2] * m[2][0],
                    m[0][2] * m[1][0] - m[0][0] * m[1][2],
                ],
                [
                    c02,
                    m[0][1] * m[2][0] - m[0][0] * m[2][1],
                    m[0][0] * m[1][1] - m[0][1] * m[1][0],
                ],
            ]
            .map(|row| row.map(|value| value / det)),
        ))
    }

    fn then(&self, first: &Self) -> Self {
        let (a, b) = (&self.0, &first.0);
        let mut out = [[0.0; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        Self(out)
    }
}

fn centroid(points: &[[f64; 2]]) -> [f64; 2] {
    let n = points.len().max(1) as f64;
    let sum = points.iter().fold([0.0, 0.0], |sum, point| {
        [sum[0] + point[0], sum[1] + point[1]]
    });
    [sum[0] / n, sum[1] / n]
}

/// The scale that brings points to an average distance of √2 from `center`.
fn spread(points: &[[f64; 2]], center: [f64; 2]) -> f64 {
    let n = points.len().max(1) as f64;
    let mean = points
        .iter()
        .map(|point| (point[0] - center[0]).hypot(point[1] - center[1]))
        .sum::<f64>()
        / n;
    if mean < 1e-9 {
        1.0
    } else {
        std::f64::consts::SQRT_2 / mean
    }
}

fn solve(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for column in 0..8 {
        let pivot =
            (column..8).max_by(|x, y| a[*x][column].abs().total_cmp(&a[*y][column].abs()))?;
        if a[pivot][column].abs() < 1e-12 {
            return None;
        }
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in column + 1..8 {
            let factor = a[row][column] / a[column][column];
            if factor != 0.0 {
                for k in column..8 {
                    a[row][k] -= factor * a[column][k];
                }
                b[row] -= factor * b[column];
            }
        }
    }
    let mut x = [0.0; 8];
    for row in (0..8).rev() {
        let known: f64 = (row + 1..8).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - known) / a[row][row];
    }
    Some(x)
}

/// The homography taking `from` to `to` in the least squares, with both normalized first as
/// Hartley's method does; none when they do not pin one down.
pub fn fit(from: &[[f64; 2]], to: &[[f64; 2]]) -> Option<Homography> {
    if from.len() < 4 || from.len() != to.len() {
        return None;
    }
    let (from_center, to_center) = (centroid(from), centroid(to));
    let (from_scale, to_scale) = (spread(from, from_center), spread(to, to_center));
    let (mut a, mut b) = ([[0.0; 8]; 8], [0.0; 8]);
    for (p, q) in from.iter().zip(to) {
        let (x, y) = (
            (p[0] - from_center[0]) * from_scale,
            (p[1] - from_center[1]) * from_scale,
        );
        let (u, v) = (
            (q[0] - to_center[0]) * to_scale,
            (q[1] - to_center[1]) * to_scale,
        );
        let rows = [
            ([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u),
            ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v),
        ];
        for (row, target) in rows {
            for i in 0..8 {
                b[i] += row[i] * target;
                for j in 0..8 {
                    a[i][j] += row[i] * row[j];
                }
            }
        }
    }
    let h = solve(a, b)?;
    let normal = Homography([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]);
    let before = Homography([
        [from_scale, 0.0, -from_scale * from_center[0]],
        [0.0, from_scale, -from_scale * from_center[1]],
        [0.0, 0.0, 1.0],
    ]);
    let after = Homography([
        [1.0 / to_scale, 0.0, to_center[0]],
        [0.0, 1.0 / to_scale, to_center[1]],
        [0.0, 0.0, 1.0],
    ]);
    let found = after.then(&normal.then(&before));
    found
        .0
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(found)
}

/// The homography most of the pairs agree on within `tolerance`, fitted to those, and which they
/// are. The same input gives the same answer.
pub fn fit_robustly(
    from: &[[f64; 2]],
    to: &[[f64; 2]],
    tolerance: f64,
) -> Option<(Homography, Vec<bool>)> {
    const TRIES: usize = 300;
    let n = from.len().min(to.len());
    if n < 4 {
        return None;
    }
    let agree = |h: &Homography| -> Vec<bool> {
        from.iter()
            .zip(to)
            .map(|(p, q)| {
                let at = h.apply(*p);
                (at[0] - q[0]).hypot(at[1] - q[1]) < tolerance
            })
            .collect()
    };
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below as u64) as usize
    };
    let mut best: Option<(Homography, Vec<bool>, usize)> = None;
    for _ in 0..TRIES {
        let mut picks = [0usize; 4];
        let mut chosen = 0;
        while chosen < 4 {
            let pick = next(n);
            if !picks[..chosen].contains(&pick) {
                picks[chosen] = pick;
                chosen += 1;
            }
        }
        let (a, b) = (picks.map(|i| from[i]), picks.map(|i| to[i]));
        let Some(h) = fit(&a, &b) else {
            continue;
        };
        let agreeing = agree(&h);
        let count = agreeing.iter().filter(|agrees| **agrees).count();
        if best.as_ref().is_none_or(|(_, _, most)| count > *most) {
            best = Some((h, agreeing, count));
        }
    }
    let (mut h, mut agreeing, _) = best?;
    // Twice over: a better fit can bring in more pairs.
    for _ in 0..2 {
        let (a, b): (Vec<[f64; 2]>, Vec<[f64; 2]>) = from
            .iter()
            .zip(to)
            .zip(&agreeing)
            .filter(|(_, agrees)| **agrees)
            .map(|((p, q), _)| (*p, *q))
            .unzip();
        if let Some(better) = fit(&a, &b) {
            h = better;
            agreeing = agree(&h);
        }
    }
    Some((h, agreeing))
}

/// Whether `point` lies within a convex quad, by the side of each edge it falls on.
fn inside(quad: &[[f64; 2]; 4], point: [f64; 2]) -> bool {
    let mut sign = 0.0;
    for (a, b) in quad.iter().zip(quad.iter().cycle().skip(1)) {
        let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        if cross * sign < 0.0 {
            return false;
        }
        if cross != 0.0 {
            sign = cross;
        }
    }
    true
}

/// The area of a quad, signed by which way round it goes.
fn area(quad: &[[f64; 2]; 4]) -> f64 {
    quad.iter()
        .zip(quad.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

/// Whether the corners still make a surface: going one way round, without folding.
fn is_convex(quad: &[[f64; 2]; 4]) -> bool {
    let turn = |i: usize| {
        let (a, b, c) = (quad[i], quad[(i + 1) % 4], quad[(i + 2) % 4]);
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    };
    let turns = [turn(0), turn(1), turn(2), turn(3)];
    turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0)
}

/// Corners of detail inside `quad`, in pixels, the best in each cell of a grid, up to about `want`.
fn corners_inside(plane: &Plane, quad: &[[f64; 2]; 4], want: usize) -> Vec<[f64; 2]> {
    let (w, h) = (plane.w, plane.h);
    if w < 8 || h < 8 {
        return Vec::new();
    }
    let at = |x: usize, y: usize| f64::from(plane.data[y * w + x]);
    let (mut low_x, mut low_y, mut high_x, mut high_y) = (w, h, 0, 0);
    for corner in quad {
        low_x = low_x.min(corner[0].max(0.0) as usize);
        low_y = low_y.min(corner[1].max(0.0) as usize);
        high_x = high_x.max(corner[0].max(0.0).ceil() as usize);
        high_y = high_y.max(corner[1].max(0.0).ceil() as usize);
    }
    let (low_x, low_y) = (low_x.max(4), low_y.max(4));
    let (high_x, high_y) = (high_x.min(w - 5), high_y.min(h - 5));
    if low_x >= high_x || low_y >= high_y {
        return Vec::new();
    }
    let cell = (((high_x - low_x) * (high_y - low_y) / want.max(1)) as f64)
        .sqrt()
        .max(4.0) as usize;
    let mut found = Vec::new();
    let mut strongest = 0.0f64;
    let mut y = low_y;
    while y < high_y {
        let mut x = low_x;
        while x < high_x {
            let mut best: Option<(f64, usize, usize)> = None;
            for cy in y..(y + cell).min(high_y) {
                for cx in x..(x + cell).min(high_x) {
                    if !inside(quad, [cx as f64 + 0.5, cy as f64 + 0.5]) {
                        continue;
                    }
                    // Smallest eigenvalue of the gradients' structure in a 5x5 window.
                    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
                    for wy in cy - 2..=cy + 2 {
                        for wx in cx - 2..=cx + 2 {
                            let gx = (at(wx + 1, wy) - at(wx - 1, wy)) / 2.0;
                            let gy = (at(wx, wy + 1) - at(wx, wy - 1)) / 2.0;
                            xx += gx * gx;
                            xy += gx * gy;
                            yy += gy * gy;
                        }
                    }
                    let score = (xx + yy) / 2.0 - (((xx - yy) / 2.0).powi(2) + xy * xy).sqrt();
                    if best.is_none_or(|(most, ..)| score > most) {
                        best = Some((score, cx, cy));
                    }
                }
            }
            if let Some((score, cx, cy)) = best {
                strongest = strongest.max(score);
                found.push((score, [cx as f64 + 0.5, cy as f64 + 0.5]));
            }
            x += cell;
        }
        y += cell;
    }
    found
        .into_iter()
        .filter(|(score, _)| *score > strongest * 0.01 && *score > 1e-7)
        .map(|(_, point)| point)
        .collect()
}

/// A point being followed: where it was on the first frame, and where it is now.
#[derive(Clone, Copy)]
struct Feature {
    origin: [f64; 2],
    at: [f64; 2],
    moved: [f64; 2],
}

/// How many points to look for.
const WANTED: usize = 200;
/// Fewer than this and a surface is lost: a homography from fewer agrees with anything.
const FEWEST: usize = 10;
/// How far a point may be from where the surface puts it, in pixels.
const AGREEMENT: f64 = 1.5;

/// Follow the surface `start` outlines through `count` frames that `frame` gives as
/// `(width, height, rgba)`. It stops at the frame the surface is lost in, and at one that `frame`
/// cannot give.
pub fn follow_plane(
    count: usize,
    start: Quad,
    mut frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> FollowedPlane {
    let mut followed = FollowedPlane::default();
    let Some((width, height, rgba)) = frame(0) else {
        return followed;
    };
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return followed;
    }
    let (w, h) = (width as f64, height as f64);
    let pixels = |quad: &Quad| quad.map(|corner| [corner[0] * w, corner[1] * h]);
    let first = pixels(&start);
    let mut previous = pyramid(width, height, &rgba);
    let Some(plane) = previous.levels.first() else {
        return followed;
    };
    let mut features: Vec<Feature> = corners_inside(plane, &first, WANTED)
        .into_iter()
        .map(|at| Feature {
            origin: at,
            at,
            moved: [0.0, 0.0],
        })
        .collect();
    followed.agreeing.push(features.len());
    followed.quads.push(start);
    let wanted = features.len();
    let options = LkOpts::default();
    for index in 1..count {
        let lost = |followed: &mut FollowedPlane| followed.lost_at = Some(index);
        let Some((frame_w, frame_h, rgba)) = frame(index) else {
            lost(&mut followed);
            return followed;
        };
        if (frame_w, frame_h) != (width, height) {
            lost(&mut followed);
            return followed;
        }
        let next = pyramid(width, height, &rgba);
        let at: Vec<[f64; 2]> = features.iter().map(|feature| feature.at).collect();
        let moved: Vec<[f64; 2]> = features.iter().map(|feature| feature.moved).collect();
        let found = track(&previous, &next, &at, Some(&moved), &options);
        let mut survivors: Vec<Feature> = features
            .iter()
            .zip(found)
            .filter_map(|(feature, found)| {
                found.map(|now| Feature {
                    origin: feature.origin,
                    at: now,
                    moved: [now[0] - feature.at[0], now[1] - feature.at[1]],
                })
            })
            .collect();
        let origins: Vec<[f64; 2]> = survivors.iter().map(|feature| feature.origin).collect();
        let now: Vec<[f64; 2]> = survivors.iter().map(|feature| feature.at).collect();
        let Some((homography, agreeing)) = fit_robustly(&origins, &now, AGREEMENT) else {
            lost(&mut followed);
            return followed;
        };
        let count_agreeing = agreeing.iter().filter(|agrees| **agrees).count();
        let quad = first.map(|corner| homography.apply(corner));
        let shrink = area(&quad).abs() / area(&first).abs().max(1e-9);
        if count_agreeing < FEWEST || !is_convex(&quad) || !(0.01..100.0).contains(&shrink) {
            lost(&mut followed);
            return followed;
        }
        let mut keep = agreeing.iter();
        survivors.retain(|_| *keep.next().unwrap_or(&false));
        // New points for those gone, each said to have started where the surface puts it.
        if survivors.len() * 2 < wanted {
            if let (Some(plane), Some(back)) = (next.levels.first(), homography.inverse()) {
                for at in corners_inside(plane, &quad, WANTED) {
                    let clear = survivors
                        .iter()
                        .all(|kept| (kept.at[0] - at[0]).hypot(kept.at[1] - at[1]) > 3.0);
                    if clear {
                        survivors.push(Feature {
                            origin: back.apply(at),
                            at,
                            moved: [0.0, 0.0],
                        });
                    }
                }
            }
        }
        features = survivors;
        previous = next;
        followed.agreeing.push(count_agreeing);
        followed
            .quads
            .push(quad.map(|corner| [corner[0] / w, corner[1] / h]));
    }
    followed
}

/// The frames of a followed surface that shape its way, all four corners considered, within
/// `tolerance` of the frame's width: what to write as keyframes.
pub fn simplified_quads(quads: &[Quad], tolerance: f64) -> Vec<usize> {
    let mut kept = std::collections::BTreeSet::new();
    for corner in 0..4 {
        let points: Vec<crate::Point> = quads
            .iter()
            .map(|quad| crate::Point {
                x: quad[corner][0],
                y: quad[corner][1],
                confidence: 1.0,
            })
            .collect();
        kept.extend(crate::simplified(&points, tolerance));
    }
    kept.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (usize, usize) = (320, 180);

    /// Blobs of detail anywhere on the plane, the same every time it is asked.
    fn texture(x: f64, y: f64, seed: f64) -> f64 {
        let wave = |fx: f64, fy: f64, phase: f64| ((x * fx + y * fy) * 0.13 + phase + seed).sin();
        let value = wave(1.0, 0.31, 0.0)
            + wave(-0.42, 1.13, 1.1)
            + 0.7 * wave(2.7, 1.9, 2.3)
            + 0.7 * wave(-1.7, 3.1, 0.7)
            + 0.5 * wave(5.3, -3.7, 4.1)
            + 0.5 * wave(3.3, 6.1, 5.3);
        (value / 8.8 + 0.5).clamp(0.0, 1.0)
    }

    /// Where the surface is on frame `n`: it slides, turns, and tilts away, in pixels.
    fn truth(n: usize) -> Quad {
        let t = n as f64;
        let (cx, cy) = (150.0 + t * 1.2, 90.0 + (t * 0.1).sin() * 12.0);
        let angle = t * 0.006;
        let (cos, sin) = (angle.cos(), angle.sin());
        let tilt = 1.0 - t * 0.002;
        let corner = |x: f64, y: f64| {
            let x = x * (1.0 + y / 300.0 * (t * 0.01));
            let (u, v) = (x * cos - y * sin, x * sin + y * cos);
            [cx + u * tilt, cy + v]
        };
        [
            corner(-70.0, -50.0),
            corner(70.0, -50.0),
            corner(70.0, 50.0),
            corner(-70.0, 50.0),
        ]
    }

    fn rest() -> Quad {
        [[0.0, 0.0], [140.0, 0.0], [140.0, 100.0], [0.0, 100.0]]
    }

    /// Frame `n`: the surface on a background moving another way, and what `cover` hides of it.
    fn scene(n: usize, cover: impl Fn(usize, f64, f64) -> bool) -> (usize, usize, Vec<u8>) {
        let (w, h) = SIZE;
        let map = fit(&truth(n), &rest()).expect("a map");
        let quad = truth(n);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let value = if cover(n, px, py) {
                    texture(px + n as f64 * 9.0, py - n as f64 * 3.0, 3.0)
                } else if inside(&quad, [px, py]) {
                    let plate = map.apply([px, py]);
                    texture(plate[0], plate[1], 0.0)
                } else {
                    texture(px - n as f64 * 4.0, py + n as f64, 1.7)
                };
                let byte = (value * 255.0) as u8;
                rgba.extend_from_slice(&[byte, byte, byte, 255]);
            }
        }
        (w, h, rgba)
    }

    fn fraction(quad: Quad) -> Quad {
        quad.map(|corner| [corner[0] / SIZE.0 as f64, corner[1] / SIZE.1 as f64])
    }

    /// The worst a corner of any frame is from the truth, in pixels.
    fn worst(followed: &FollowedPlane) -> f64 {
        followed
            .quads
            .iter()
            .enumerate()
            .flat_map(|(n, quad)| {
                let truth = truth(n);
                (0..4).map(move |corner| {
                    (quad[corner][0] * SIZE.0 as f64 - truth[corner][0])
                        .hypot(quad[corner][1] * SIZE.1 as f64 - truth[corner][1])
                })
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_homography_comes_back_from_four_points() {
        let (from, to) = (rest(), truth(40));
        let h = fit(&from, &to).expect("a fit");
        for (p, q) in from.iter().zip(&to) {
            let at = h.apply(*p);
            assert!((at[0] - q[0]).hypot(at[1] - q[1]) < 1e-6, "{at:?} {q:?}");
        }
        let back = h.inverse().expect("an inverse");
        let there = back.apply(to[2]);
        assert!((there[0] - from[2][0]).hypot(there[1] - from[2][1]) < 1e-6);
    }

    #[test]
    fn what_does_not_move_with_the_surface_is_left_out() {
        let h = fit(&rest(), &truth(30)).expect("a fit");
        let from: Vec<[f64; 2]> = (0..60)
            .map(|i| [(i % 10) as f64 * 14.0, (i / 10) as f64 * 16.0])
            .collect();
        let mut to: Vec<[f64; 2]> = from.iter().map(|p| h.apply(*p)).collect();
        for moved in to.iter_mut().step_by(3) {
            moved[0] += 40.0;
        }
        let (found, agreeing) = fit_robustly(&from, &to, 1.0).expect("a fit");
        assert_eq!(agreeing.iter().filter(|a| **a).count(), 40);
        assert!(agreeing.iter().step_by(3).all(|a| !*a));
        let at = found.apply([70.0, 50.0]);
        let want = h.apply([70.0, 50.0]);
        assert!((at[0] - want[0]).hypot(at[1] - want[1]) < 1e-6);
    }

    #[test]
    fn a_surface_is_followed_as_it_slides_and_tilts() {
        let frames = 48;
        let followed = follow_plane(frames, fraction(truth(0)), |n| {
            Some(scene(n, |_, _, _| false))
        });
        assert_eq!(followed.lost_at, None, "{:?}", followed.agreeing);
        assert_eq!(followed.quads.len(), frames);
        let off = worst(&followed);
        eprintln!(
            "planar: a corner at worst {off:.3} px off, {:?} points",
            followed.agreeing.iter().min()
        );
        assert!(off < 1.0, "a corner is {off:.2} pixels off");
    }

    #[test]
    fn something_passing_in_front_does_not_carry_it_off() {
        let frames = 36;
        // A bar crossing the surface, a fifth of its width, moving its own way.
        let cover = |n: usize, x: f64, _: f64| {
            let left = 60.0 + n as f64 * 4.0;
            (left..left + 30.0).contains(&x)
        };
        let followed = follow_plane(frames, fraction(truth(0)), |n| Some(scene(n, cover)));
        assert_eq!(followed.lost_at, None, "{:?}", followed.agreeing);
        let off = worst(&followed);
        eprintln!("planar under a passing bar: a corner at worst {off:.3} px off");
        assert!(off < 2.0, "a corner is {off:.2} pixels off");
    }

    #[test]
    fn a_surface_with_nothing_on_it_is_lost_at_once() {
        let flat = |_: usize| Some((64, 64, vec![128u8; 64 * 64 * 4]));
        let start = [[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8]];
        let followed = follow_plane(4, start, flat);
        assert_eq!(followed.lost_at, Some(1));
        assert_eq!(followed.quads.len(), 1);
    }

    #[test]
    fn a_frame_cut_to_something_else_loses_it() {
        let frames = 10;
        let followed = follow_plane(frames, fraction(truth(0)), |n| {
            if n < 5 {
                Some(scene(n, |_, _, _| false))
            } else {
                let (w, h) = SIZE;
                let mut rgba = Vec::with_capacity(w * h * 4);
                for y in 0..h {
                    for x in 0..w {
                        let byte =
                            (texture(x as f64 * 2.3, y as f64 * 1.9, 9.0 + n as f64) * 255.0) as u8;
                        rgba.extend_from_slice(&[byte, byte, byte, 255]);
                    }
                }
                Some((w, h, rgba))
            }
        });
        assert_eq!(followed.lost_at, Some(5));
    }

    #[test]
    fn only_the_frames_that_shape_the_way_are_kept() {
        let moved: Vec<Quad> = (0..30)
            .map(|n| {
                let x = n as f64 * 0.01;
                [[x, 0.0], [x + 0.2, 0.0], [x + 0.2, 0.2], [x, 0.2]]
            })
            .collect();
        assert_eq!(simplified_quads(&moved, 0.001), vec![0, 29]);
    }
}
