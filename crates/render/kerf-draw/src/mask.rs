//! A clip's masks as coverage, carried from FilmCraft's renderer
//! (`mask.rs`).
//!
//! Each outline is flattened to a polygon in the pixels it is drawn at. For
//! a pixel center the signed distance `d` to the polygon (positive inside)
//! gives the coverage
//!
//! ```text
//! s = d + expansion
//! w = max(feather, 1)
//! u = clamp(s / w + 0.5, 0, 1)
//! c = lerp(u, smoothstep(u), min(feather, 1))
//! m = opacity · (inverted ? 1 − c : c)
//! ```
//!
//! so a hard mask is the box-filtered coverage of its edge, antialiased,
//! and a feathered one falls off smoothly over `feather` pixels centered on
//! its expanded edge. An outline that says how soft it is at each corner adds
//! that to `feather`, going evenly along each edge, and a pixel takes the edge
//! that covers it least inside the outline, or most outside it, which comes to
//! the nearest edge when they are alike. Masks combine in order by their mode, as Kerf's
//! language describes them: a first mask that subtracts takes from the
//! whole picture, and a first that intersects or darkens covers as one that
//! adds.

use kerf_model::{Catalog, Mask, MaskMode, MaskShape, Time};
use rayon::prelude::*;

/// Flattening tolerance, in pixels drawn.
const TOLERANCE: f64 = 0.05;

/// One mask ready to draw, in the pixels of the picture it cuts.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatMask {
    /// A closed polygon: the last corner joins the first.
    pub points: Vec<[f32; 2]>,
    /// How much wider than `feather` the edge's fall-off is at each corner, or empty for not at all.
    pub soft: Vec<f32>,
    pub feather: f32,
    pub expansion: f32,
    /// 0 to 1.
    pub opacity: f32,
    pub inverted: bool,
    pub mode: MaskMode,
}

/// Whether a mask cuts the picture. One that combines by `none`, or an
/// open outline, only gives effects a path.
pub fn cuts(mask: &Mask) -> bool {
    mask.mode != MaskMode::None && mask.shape.is_closed()
}

/// A mask's outline on a `width` × `height` picture drawn at `scale` of
/// its size, flattened to corners in the pixels drawn.
pub fn outline(shape: &MaskShape, width: f64, height: f64, scale: f64) -> Vec<[f32; 2]> {
    let (w, h) = (width * scale, height * scale);
    let at = |x: f64, y: f64| [(x * w) as f32, (y * h) as f32];
    match shape {
        MaskShape::Rect([l, t, r, b]) => vec![at(*l, *t), at(*r, *t), at(*r, *b), at(*l, *b)],
        MaskShape::Ellipse([x, y, rx, ry]) => {
            let (cx, cy, rx, ry) = (x * w, y * h, rx * w, ry * h);
            let radius = rx.abs().max(ry.abs()).max(TOLERANCE);
            let step = 2.0 * (1.0 - TOLERANCE / radius).clamp(-1.0, 1.0).acos();
            let count = (std::f64::consts::TAU / step.max(1e-6))
                .ceil()
                .clamp(16.0, 4096.0) as usize;
            (0..count)
                .map(|index| {
                    let angle = std::f64::consts::TAU * index as f64 / count as f64;
                    [
                        (cx + rx * angle.cos()) as f32,
                        (cy + ry * angle.sin()) as f32,
                    ]
                })
                .collect()
        }
        MaskShape::Polygon { points, .. } => points.iter().map(|[x, y]| at(*x, *y)).collect(),
        // Whoever knows the time asks for the outline at it; without one, it is where it starts.
        MaskShape::Moving(keys) => keys
            .first()
            .map_or_else(Vec::new, |key| outline(&key.shape, width, height, scale)),
    }
}

/// The masks of a clip that cut its picture, at clip time `at`, on a
/// picture of `size` drawn at `scale` of it.
pub fn prepare(masks: &[Mask], size: (f64, f64), scale: f64, at: Time) -> Vec<FlatMask> {
    masks
        .iter()
        .filter(|mask| cuts(mask))
        .map(|mask| {
            let number = |name: &str| {
                mask.params
                    .get(name)
                    .and_then(|param| param.value_at(at).as_number())
                    .or_else(|| {
                        Catalog::builtin()
                            .mask_prop(name)
                            .and_then(|def| def.default.as_number())
                    })
                    .unwrap_or(0.0)
            };
            let shape = mask.shape.at(at);
            let soft = match shape.as_ref() {
                MaskShape::Polygon { soft, .. } => {
                    soft.iter().map(|soft| (soft * scale) as f32).collect()
                }
                _ => Vec::new(),
            };
            FlatMask {
                points: outline(&shape, size.0, size.1, scale),
                soft,
                feather: (number("feather").max(0.0) * scale) as f32,
                expansion: (number("expansion") * scale) as f32,
                opacity: number("opacity").clamp(0.0, 1.0) as f32,
                inverted: mask.inverted,
                mode: mask.mode,
            }
        })
        .collect()
}

/// What a mask's coverage `covered` leaves of `kept`, the coverage of the
/// masks before it.
fn combine(mode: MaskMode, kept: f32, covered: f32) -> f32 {
    match mode {
        MaskMode::Add => kept + covered - kept * covered,
        MaskMode::Subtract => kept * (1.0 - covered),
        MaskMode::Intersect => kept * covered,
        MaskMode::Lighten => kept.max(covered),
        MaskMode::Darken => kept.min(covered),
        MaskMode::Difference => (kept - covered).abs(),
        MaskMode::None => kept,
    }
}

/// The falloff of a signed, expanded distance for a feather width.
#[inline]
fn falloff(s: f32, feather: f32) -> f32 {
    let w = feather.max(1.0);
    let u = (s / w + 0.5).clamp(0.0, 1.0);
    let smooth = u * u * (3.0 - 2.0 * u);
    u + (smooth - u) * feather.min(1.0)
}

/// How far from the edge coverage stops changing.
fn band(mask: &FlatMask) -> f32 {
    let softest = mask.soft.iter().copied().fold(0.0, f32::max);
    mask.expansion.abs() + (mask.feather + softest).max(1.0) * 0.5 + 1.0
}

/// How far from the place `(px, py)` the segment from `a` to `b` is, squared, and how far along it
/// that is, from 0 to 1.
#[inline]
fn segment_distance2(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> (f32, f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (qx, qy) = (px - a[0], py - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        ((qx * dx + qy * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ex, ey) = (qx - dx * t, qy - dy * t);
    (ex * ex + ey * ey, t)
}

/// An edge of an outline near a row: from `a` to `b`, near what lies from `left` to `right`, starting
/// at the corner it is by number.
struct NearEdge {
    a: [f32; 2],
    b: [f32; 2],
    left: f32,
    right: f32,
    corner: usize,
}

/// One mask's coverage of row `y` into `out`: the winding from the row's
/// crossings, the distance only to edges near the row.
fn mask_row(mask: &FlatMask, y: usize, out: &mut [f32]) {
    let n = mask.points.len();
    let py = y as f32 + 0.5;
    let band = band(mask);
    let kept = |covered: f32| {
        mask.opacity
            * if mask.inverted {
                1.0 - covered
            } else {
                covered
            }
    };
    let value = |distance: f32| kept(falloff(distance + mask.expansion, mask.feather));
    if n < 3 {
        out.fill(value(-band));
        return;
    }
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut near: Vec<NearEdge> = Vec::new();
    for i in 0..n {
        let a = mask.points[i];
        let b = mask.points[(i + 1) % n];
        if (a[1] <= py) != (b[1] <= py) {
            let t = (py - a[1]) / (b[1] - a[1]);
            crossings.push((a[0] + (b[0] - a[0]) * t, if b[1] > a[1] { 1 } else { -1 }));
        }
        if py >= a[1].min(b[1]) - band && py <= a[1].max(b[1]) + band {
            near.push(NearEdge {
                a,
                b,
                left: a[0].min(b[0]) - band,
                right: a[0].max(b[0]) + band,
                corner: i,
            });
        }
    }
    crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
    let (mut next, mut winding) = (0, 0);
    let band2 = band * band;
    for (x, out) in out.iter_mut().enumerate() {
        let px = x as f32 + 0.5;
        while next < crossings.len() && crossings[next].0 < px {
            winding += crossings[next].1;
            next += 1;
        }
        if mask.soft.is_empty() {
            let mut distance2 = band2;
            for edge in &near {
                if px >= edge.left && px <= edge.right {
                    distance2 = distance2.min(segment_distance2(px, py, edge.a, edge.b).0);
                }
            }
            let distance = distance2.sqrt();
            *out = value(if winding != 0 { distance } else { -distance });
            continue;
        }
        let inside = winding != 0;
        let mut covered = if inside { 1.0f32 } else { 0.0 };
        for edge in &near {
            if px >= edge.left && px <= edge.right {
                let (distance2, along) = segment_distance2(px, py, edge.a, edge.b);
                let distance = distance2.min(band2).sqrt();
                let (from, to) = (mask.soft[edge.corner], mask.soft[(edge.corner + 1) % n]);
                let wide = mask.feather + from + (to - from) * along;
                let signed = if inside { distance } else { -distance };
                let here = falloff(signed + mask.expansion, wide);
                covered = if inside {
                    covered.min(here)
                } else {
                    covered.max(here)
                };
            }
        }
        *out = kept(covered);
    }
}

/// The combined coverage of `masks` over a `w` × `h` picture, or `None`
/// with no mask that cuts.
pub fn coverage(masks: &[FlatMask], w: usize, h: usize) -> Option<Vec<f32>> {
    let cutting: Vec<&FlatMask> = masks
        .iter()
        .filter(|mask| mask.mode != MaskMode::None)
        .collect();
    let first = cutting.first()?;
    let start = if first.mode == MaskMode::Subtract {
        1.0
    } else {
        0.0
    };
    let mut kept = vec![start; w * h];
    if w == 0 {
        return Some(kept);
    }
    kept.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let mut covered = vec![0.0f32; w];
        for (index, mask) in cutting.iter().enumerate() {
            mask_row(mask, y, &mut covered);
            let mode = match mask.mode {
                MaskMode::Intersect | MaskMode::Darken if index == 0 => MaskMode::Add,
                mode => mode,
            };
            for (kept, covered) in row.iter_mut().zip(&covered) {
                *kept = combine(mode, *kept, *covered).clamp(0.0, 1.0);
            }
        }
    });
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(shape: MaskShape, size: f64, mode: MaskMode) -> FlatMask {
        FlatMask {
            points: outline(&shape, size, size, 1.0),
            soft: Vec::new(),
            feather: 0.0,
            expansion: 0.0,
            opacity: 1.0,
            inverted: false,
            mode,
        }
    }

    #[test]
    fn hard_rects_cover_whole_pixels_exactly() {
        let mask = flat(MaskShape::Rect([0.2, 0.2, 0.65, 0.6]), 10.0, MaskMode::Add);
        let covered = coverage(&[mask], 10, 10).unwrap();
        assert_eq!(covered[3 * 10 + 3], 1.0);
        assert_eq!(covered[3 * 10 + 1], 0.0);
        assert!((covered[3 * 10 + 6] - 0.5).abs() < 1e-6);
        assert_eq!(covered[8 * 10 + 3], 0.0);
    }

    #[test]
    fn ellipses_cover_their_area() {
        let mask = flat(
            MaskShape::Ellipse([0.5, 0.5, 0.3, 0.2]),
            100.0,
            MaskMode::Add,
        );
        let area: f64 = coverage(&[mask], 100, 100)
            .unwrap()
            .iter()
            .map(|c| f64::from(*c))
            .sum();
        let exact = std::f64::consts::PI * 30.0 * 20.0;
        assert!((area - exact).abs() / exact < 0.002, "{area} vs {exact}");
    }

    #[test]
    fn first_masks_start_from_what_they_combine_with() {
        let hole = MaskShape::Rect([0.25, 0.25, 0.75, 0.75]);
        let subtract = coverage(&[flat(hole.clone(), 8.0, MaskMode::Subtract)], 8, 8).unwrap();
        assert_eq!((subtract[0], subtract[4 * 8 + 4]), (1.0, 0.0));
        let intersect = coverage(&[flat(hole, 8.0, MaskMode::Intersect)], 8, 8).unwrap();
        assert_eq!((intersect[0], intersect[4 * 8 + 4]), (0.0, 1.0));
    }

    /// A square from 25 to 75 of 100, with these at its corners from the top left, going clockwise.
    fn soft_square(soft: Vec<f32>) -> FlatMask {
        let mut mask = flat(
            MaskShape::Polygon {
                points: vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]],
                closed: true,
                soft: Vec::new(),
            },
            100.0,
            MaskMode::Add,
        );
        mask.soft = soft;
        mask
    }

    #[test]
    fn softness_of_nothing_draws_as_none_does() {
        let plain = coverage(&[soft_square(Vec::new())], 100, 100).unwrap();
        let zero = coverage(&[soft_square(vec![0.0; 4])], 100, 100).unwrap();
        assert_eq!(plain, zero);
        for (feather, expansion) in [(6.0, 0.0), (10.0, -3.0), (0.5, 4.0)] {
            let with = |soft: Vec<f32>| {
                let mut mask = soft_square(soft);
                (mask.feather, mask.expansion) = (feather, expansion);
                coverage(&[mask], 100, 100).unwrap()
            };
            assert_eq!(
                with(Vec::new()),
                with(vec![0.0; 4]),
                "{feather} {expansion}"
            );
        }
    }

    #[test]
    fn an_edge_is_as_soft_as_its_corners_say_going_evenly_along_it() {
        // Hard on its left, 20 across on its right, and going from one to the other along the top and
        // the bottom.
        let covered = coverage(&[soft_square(vec![0.0, 20.0, 20.0, 0.0])], 100, 100).unwrap();
        let at = |x: usize, y: usize| covered[y * 100 + x];
        // The right edge, at x = 75 and up the middle row: smoothstep over 20 pixels.
        for x in 60..90 {
            let u = ((75.0 - (x as f32 + 0.5)) / 20.0 + 0.5).clamp(0.0, 1.0);
            let wanted = u * u * (3.0 - 2.0 * u);
            let found = at(x, 50);
            assert!((found - wanted).abs() < 0.01, "{x}: {found} not {wanted}");
        }
        // The left, sharp as ever.
        assert_eq!(at(23, 50), 0.0);
        assert_eq!(at(26, 50), 1.0);
        // Halfway along the top it is 10 across, and 2.5 pixels out from its middle is a quarter of it.
        let (u, found) = (0.25f32, at(50, 22));
        let wanted = u * u * (3.0 - 2.0 * u);
        assert!((found - wanted).abs() < 0.02, "{found} not {wanted}");
        // It stays the same for a mask turned inside out, and it scales.
        let mut inverted = soft_square(vec![0.0, 20.0, 20.0, 0.0]);
        inverted.inverted = true;
        let inverted = coverage(&[inverted], 100, 100).unwrap();
        assert!((inverted[50 * 100 + 70] - (1.0 - at(70, 50))).abs() < 1e-6);
    }

    #[test]
    fn a_soft_edge_has_no_seam_where_two_edges_meet() {
        // Whichever edge a pixel goes by, none is far from its neighbours: a fall-off at least 12 wide
        // changes by 1.5 / 12 a pixel at most.
        let covered = coverage(&[soft_square(vec![12.0, 30.0, 12.0, 30.0])], 100, 100).unwrap();
        for y in 1..99 {
            for x in 1..99 {
                let here = covered[y * 100 + x];
                let (across, down) = (covered[y * 100 + x + 1], covered[(y + 1) * 100 + x]);
                assert!(
                    (here - across).abs() < 0.2 && (here - down).abs() < 0.2,
                    "{x},{y}: {here} {across} {down}"
                );
            }
        }
    }

    #[test]
    fn a_shape_with_soft_corners_prepares_them_at_the_scale_it_draws() {
        let mask = Mask {
            name: kerf_model::Id::new("p").unwrap(),
            shape: MaskShape::Polygon {
                points: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                closed: true,
                soft: vec![4.0, 8.0, 12.0],
            },
            mode: MaskMode::Add,
            inverted: false,
            params: Default::default(),
        };
        let flat = prepare(&[mask], (100.0, 100.0), 0.5, Time::ZERO);
        assert_eq!(flat[0].soft, [2.0, 4.0, 6.0]);
    }

    #[test]
    fn open_outlines_and_none_cut_nothing() {
        let open = Mask {
            name: kerf_model::Id::new("p").unwrap(),
            shape: MaskShape::Polygon {
                points: vec![[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                closed: false,
                soft: Vec::new(),
            },
            mode: MaskMode::Add,
            inverted: false,
            params: Default::default(),
        };
        assert!(!cuts(&open));
        assert!(prepare(&[open], (10.0, 10.0), 1.0, Time::ZERO).is_empty());
    }
}
