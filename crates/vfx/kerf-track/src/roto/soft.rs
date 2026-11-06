//! How soft an object's edge is, measured across its outline.
//!
//! A cut says which pixels are the object, and a soft edge is more than that: hair, focus, and motion
//! blur spread it over pixels that are partly each. At each corner of the outline this reads the
//! picture across the edge, in light rather than in code values since that is where a blur happens,
//! takes what lies well inside and well outside as the two colours being mixed, and finds how wide a
//! fall-off, centred where, a mask would need to draw what it sees. What is written is one width at a
//! corner, not a matte: strands of hair come out as a band, soft on average, not as strands.

use super::{matting, Picture};
use rayon::prelude::*;

/// What an edge is at a corner, in the pixels of the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Across {
    /// How wide the fall-off is, as a mask's `feather` would say it.
    pub wide: f64,
    /// How far out from the corner its middle is.
    pub out: f64,
    /// How much to go by this, from 0 where the two sides look alike to 1 where they are plain.
    pub weight: f64,
}

/// How wide a hard edge reads, by way of its pixels and of the cut wandering along it.
const HARD: f64 = 3.0;
/// How many steps at most there are from either side of an edge to the corner.
const STEPS: f64 = 96.0;
/// How little a measure may be gone by, on average around it, for anything to come of it.
const DOUBTFUL: f64 = 0.25;
/// Where the lines across an edge are, along it: few pixels apart, or an edge that turns between them
/// would read as wider than it is.
const LINES: [f64; 5] = [-2.0, -1.0, 0.0, 1.0, 2.0];

fn linear(code: f32) -> f32 {
    if code <= 0.04045 {
        code / 12.92
    } else {
        ((code + 0.055) / 1.055).powf(2.4)
    }
}

fn mean(colors: &[[f32; 3]]) -> Option<[f32; 3]> {
    if colors.is_empty() {
        return None;
    }
    let mut sum = [0.0; 3];
    for color in colors {
        for (sum, value) in sum.iter_mut().zip(color) {
            *sum += value;
        }
    }
    Some(sum.map(|value| value / colors.len() as f32))
}

fn gap2(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(&b).map(|(a, b)| (a - b) * (a - b)).sum()
}

/// What a mask draws at `from` pixels in from an edge `wide` across: `kerf_draw::mask`'s fall-off.
fn fall_off(from: f64, wide: f64) -> f64 {
    let u = (from / wide.max(1.0) + 0.5).clamp(0.0, 1.0);
    let smooth = u * u * (3.0 - 2.0 * u);
    u + (smooth - u) * wide.min(1.0)
}

/// How far off `alpha`, read every `step` pixels from `-inner`, is from a fall-off `wide` centred at `out`.
fn cost(alpha: &[f64], (step, inner): (f64, f64), wide: f64, out: f64) -> f64 {
    alpha
        .iter()
        .enumerate()
        .map(|(index, alpha)| {
            let off = alpha - fall_off(out - (index as f64 * step - inner), wide);
            off * off
        })
        .sum()
}

/// The fall-off that best draws `alpha`, read every `step` pixels from `-inner`: its width and where it
/// is centred.
fn fit(alpha: &[f64], (step, inner): (f64, f64)) -> (f64, f64) {
    let room = alpha.len() as f64 * step;
    let mut best = (f64::INFINITY, 0.0, 0.0);
    let mut wide = 0.0;
    while wide <= room {
        // Near the corner, as a cut does not go far wrong, and further for what is wide.
        let reach = (wide + 6.0).min(room / 2.0);
        let by = (wide / 8.0).max(0.25);
        let steps = (reach / by) as i32;
        for at in -steps..=steps {
            let out = f64::from(at) * by;
            let off = cost(alpha, (step, inner), wide, out);
            if off < best.0 {
                best = (off, wide, out);
            }
        }
        wide = if wide < 1.0 {
            1.0
        } else {
            (wide * 1.1).max(wide + 0.5)
        };
    }
    // Then closer around what was found.
    let (near_wide, near_out) = (best.1, best.2);
    let by = (near_wide / 32.0).max(0.0625);
    for across in -4..=4 {
        let wide = (near_wide * (1.0 + f64::from(across) * 0.03)).max(0.0);
        for along in -4..=4 {
            let out = near_out + f64::from(along) * by;
            let off = cost(alpha, (step, inner), wide, out);
            if off < best.0 {
                best = (off, wide, out);
            }
        }
    }
    (best.1, best.2)
}

/// Which way is out from an outline at each of its corners, a unit long.
pub(super) fn normals(corners: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = corners.len();
    // By the shoelace, with y going down, this is above 0 going clockwise as seen.
    let turning: f64 = (0..n)
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    (0..n)
        .map(|i| {
            let (a, b) = (corners[(i + n - 1) % n], corners[(i + 1) % n]);
            let (x, y) = (b[0] - a[0], b[1] - a[1]);
            let length = x.hypot(y).max(1e-9);
            let side = if turning >= 0.0 { 1.0 } else { -1.0 };
            [side * y / length, -side * x / length]
        })
        .collect()
}

/// What the edge is at each of `corners`, in the pixels of `picture`, around the cut `matte`.
pub(super) fn measured(picture: &Picture, matte: &[u8], corners: &[[f64; 2]]) -> Vec<Across> {
    let (w, h) = (picture.width, picture.height);
    let signed = matting::signed_distance(matte, w, h);
    let channel: [Vec<f32>; 3] =
        std::array::from_fn(|at| picture.rgb.iter().map(|color| linear(color[at])).collect());
    let reach = (w.max(h) as f64 / 8.0).clamp(16.0, 120.0);
    let step = (reach / STEPS).max(0.5);
    let normals = normals(corners);
    let n = corners.len();
    (0..n)
        .into_par_iter()
        .map(|i| {
            let (at, out) = (corners[i], normals[i]);
            let along = [-out[1], out[0]];
            // Across the point of a turn, what reads as a fall-off is the turn.
            let (a, b) = (corners[(i + n - 1) % n], corners[(i + 1) % n]);
            let leg = |from: [f64; 2], to: [f64; 2]| {
                let (x, y) = (to[0] - from[0], to[1] - from[1]);
                let length = x.hypot(y).max(1e-9);
                [x / length, y / length]
            };
            let (into, onto) = (leg(a, at), leg(at, b));
            let straight = (into[0] * onto[0] + into[1] * onto[1]).max(0.0).powi(4);
            let depth =
                |x: f64| matting::sample(&signed, w, h, at[0] + out[0] * x, at[1] + out[1] * x);
            // As far in as the object keeps getting deeper, and as far out as it keeps falling away,
            // or what is read comes round to some other edge.
            let far = |sign: f64| {
                let mut x = 1.0;
                while x < reach && (depth(sign * x) - depth(sign * (x - 1.0))) * -sign as f32 > 0.0
                {
                    x += 1.0;
                }
                x
            };
            let (inner, outer) = (far(-1.0), far(1.0));
            let steps = ((inner + outer) / step) as usize + 1;
            let colors: Vec<[f32; 3]> = (0..steps)
                .map(|at_step| {
                    let x = at_step as f64 * step - inner;
                    let mut sum = [0.0f32; 3];
                    for line in LINES {
                        let (px, py) = (
                            at[0] + out[0] * x + along[0] * line,
                            at[1] + out[1] * x + along[1] * line,
                        );
                        for (sum, channel) in sum.iter_mut().zip(&channel) {
                            *sum += matting::sample(channel, w, h, px, py);
                        }
                    }
                    sum.map(|value| value / LINES.len() as f32)
                })
                .collect();
            // The outer stretch of either end is what is being mixed.
            let split = |from: f64| ((from + inner) / step) as usize;
            let (inside, outside) = (
                &colors[..=split(-0.6 * inner).min(steps - 1)],
                &colors[split(0.6 * outer).min(steps - 1)..],
            );
            let (Some(fore), Some(back)) = (mean(inside), mean(outside)) else {
                return Across::default();
            };
            let contrast = gap2(fore, back);
            let spread = |colors: &[[f32; 3]], middle: [f32; 3]| {
                colors.iter().map(|color| gap2(*color, middle)).sum::<f32>() / colors.len() as f32
            };
            let noise = spread(inside, fore) + spread(outside, back);
            let weight = straight * f64::from(contrast / (contrast + 8.0 * noise + 1e-4));
            let line: [f32; 3] = std::array::from_fn(|at| fore[at] - back[at]);
            let alpha: Vec<f64> = colors
                .iter()
                .map(|color| {
                    let off: f32 = (0..3).map(|at| (color[at] - back[at]) * line[at]).sum();
                    f64::from(off / contrast.max(1e-6))
                })
                .collect();
            let (wide, off) = fit(&alpha, (step, inner));
            Across {
                wide: wide.min(2.0 * inner.min(outer)),
                out: off,
                weight,
            }
        })
        .collect()
}

/// `frames` of measures smoothed over a few corners and a few frames, by how far each is to be gone by,
/// so that a mask does not flicker with a measure's noise, and hard where it reads as hard.
pub(super) fn settled(frames: &[Vec<Across>]) -> Vec<Vec<Across>> {
    let count = frames.first().map_or(0, Vec::len);
    frames
        .iter()
        .enumerate()
        .map(|(frame, corners)| {
            (0..corners.len())
                .map(|corner| {
                    let (mut wide, mut out, mut weight, mut seen) = (0.0, 0.0, 0.0, 0.0);
                    let before = frame.saturating_sub(2);
                    for near in &frames[before..(frame + 3).min(frames.len())] {
                        for off in 0..3 {
                            let at = near[(corner + count + off - 1) % count];
                            wide += at.wide * at.weight;
                            out += at.out * at.weight;
                            weight += at.weight;
                            seen += 1.0;
                        }
                    }
                    // Where the two sides hardly differ, nothing was seen and it stays as it was cut.
                    let plain = ((weight / seen - DOUBTFUL) / DOUBTFUL).clamp(0.0, 1.0);
                    if plain <= 0.0 {
                        return Across::default();
                    }
                    let (wide, out) = (wide / weight, out / weight);
                    // Taking from how wide it reads what any edge reads, as blurs add.
                    let wide = (wide * wide - HARD * HARD).max(0.0).sqrt();
                    let out = out * (wide / HARD).min(1.0);
                    Across {
                        wide: wide * plain,
                        out: out * plain,
                        weight,
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fall_off_is_found_as_wide_as_it_is_and_where_it_is() {
        for (wide, middle) in [(0.0, 0.0), (6.0, 0.0), (12.0, 3.0), (20.0, -4.0)] {
            let (inner, step) = (30.0, 0.5);
            let alpha: Vec<f64> = (0..120)
                .map(|at| fall_off(middle - (f64::from(at) * step - inner), wide))
                .collect();
            let (found, at) = fit(&alpha, (step, inner));
            assert!((found - wide).abs() <= 1.0, "{wide}: {found}");
            assert!((at - middle).abs() <= 0.5, "{middle}: {at}");
        }
    }

    #[test]
    fn out_is_out_whichever_way_an_outline_goes() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let clockwise = normals(&square);
        // At the top right corner, up and to the right.
        assert!(
            clockwise[1][0] > 0.0 && clockwise[1][1] < 0.0,
            "{clockwise:?}"
        );
        let mut reversed = square;
        reversed.reverse();
        let other = normals(&reversed);
        assert!(other[2][0] > 0.0 && other[2][1] < 0.0, "{other:?}");
    }

    #[test]
    fn a_measure_that_says_little_goes_for_little_and_hard_stays_hard() {
        let at = |wide: f64, weight: f64| Across {
            wide,
            out: 0.0,
            weight,
        };
        let frames = vec![vec![at(10.0, 1.0), at(40.0, 0.01), at(10.0, 1.0), at(10.0, 1.0)]; 3];
        let settled = settled(&frames);
        assert!((settled[1][1].wide - 9.5).abs() < 1.0, "{settled:?}");
        let hard = vec![vec![at(2.5, 1.0); 4]; 3];
        assert_eq!(super::settled(&hard)[1][1].wide, 0.0);
        let nothing = vec![vec![at(9.0, 0.0); 4]; 3];
        assert_eq!(super::settled(&nothing)[1][1].wide, 0.0);
    }
}
