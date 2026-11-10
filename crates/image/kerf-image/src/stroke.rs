//! Clone and heal: a stroke in the document that paints the picture with the picture, from
//! elsewhere in the frame or from another frame. Healing keeps what it paints from's detail and
//! takes its light and color from around the stroke.

use crate::fill::membrane_core;
use kerf_draw::Image;
use rayon::prelude::*;

/// A stroke, in the picture's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// One point dabs, more draw a line.
    pub points: Vec<[f32; 2]>,
    pub radius: f32,
    /// How much of the radius is solid, 0 to 1.
    pub hardness: f32,
    /// 0 to 1.
    pub opacity: f32,
}

/// How much of the pixels in a rectangle of the picture a stroke covers.
struct Covered {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    values: Vec<f32>,
}

/// How far (`x`, `y`) is from the line from `a` to `b`.
fn distance_to(a: [f32; 2], b: [f32; 2], x: f32, y: f32) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let along = dx * dx + dy * dy;
    let t = if along > 0.0 {
        (((x - a[0]) * dx + (y - a[1]) * dy) / along).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (x - a[0] - t * dx).hypot(y - a[1] - t * dy)
}

fn covered(stroke: &Stroke, (w, h): (usize, usize)) -> Option<Covered> {
    if stroke.points.is_empty() || stroke.radius <= 0.0 || w == 0 || h == 0 {
        return None;
    }
    // Room past the radius for the pixels just outside the stroke.
    let reach = stroke.radius + 2.0;
    let near = |low: f32, high: f32, most: usize| {
        (
            (low - reach).floor().clamp(0.0, most as f32) as usize,
            (high + reach).ceil().clamp(0.0, most as f32) as usize,
        )
    };
    let (mut left, mut right, mut top, mut bottom) = (w, 0, h, 0);
    for point in &stroke.points {
        let (from, to) = near(point[0], point[0], w);
        (left, right) = (left.min(from), right.max(to));
        let (from, to) = near(point[1], point[1], h);
        (top, bottom) = (top.min(from), bottom.max(to));
    }
    if right <= left || bottom <= top {
        return None;
    }
    let (width, height) = (right - left, bottom - top);
    let mut values = vec![0.0f32; width * height];
    let inner = stroke.radius * stroke.hardness.clamp(0.0, 1.0);
    let single = [stroke.points[0], stroke.points[0]];
    let lines = if stroke.points.len() > 1 {
        stroke.points.windows(2)
    } else {
        single.windows(2)
    };
    for line in lines {
        let (a, b) = (line[0], line[1]);
        let (x0, x1) = near(a[0].min(b[0]), a[0].max(b[0]), w);
        let (y0, y1) = near(a[1].min(b[1]), a[1].max(b[1]), h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = distance_to(a, b, x as f32 + 0.5, y as f32 + 0.5);
                let edge = (stroke.radius - d + 0.5).clamp(0.0, 1.0);
                let fade = if d <= inner || stroke.radius <= inner {
                    1.0
                } else {
                    let t = (d - inner) / (stroke.radius - inner);
                    1.0 - t * t * (3.0 - 2.0 * t)
                };
                let here = &mut values[(y - top) * width + x - left];
                *here = here.max(edge * fade);
            }
        }
    }
    Some(Covered {
        left,
        top,
        width,
        height,
        values,
    })
}

impl Covered {
    /// The picture's position of a value's index.
    fn position(&self, index: usize) -> (usize, usize) {
        (
            self.left + index % self.width,
            self.top + index / self.width,
        )
    }
}

/// `source` at `x`, `y` of `picture`'s pixel grid, moved by `from`, and whether that is in it.
fn read(source: &Image, x: usize, y: usize, from: [f32; 2]) -> ([f32; 4], bool) {
    let (sx, sy) = (x as f32 + 0.5 + from[0], y as f32 + 0.5 + from[1]);
    let inside = sx >= 0.0 && sy >= 0.0 && sx <= source.w as f32 && sy <= source.h as f32;
    (source.sample_bilinear_clamped(sx, sy), inside)
}

fn mix(dst: [f32; 4], src: [f32; 4], amount: f32) -> [f32; 4] {
    std::array::from_fn(|k| dst[k] + (src[k] - dst[k]) * amount)
}

/// Write `made` over `picture` by how much the stroke covers.
fn lay(picture: &mut Image, covered: &Covered, opacity: f32, made: &[([f32; 4], bool)]) {
    let w = picture.w;
    picture
        .px
        .par_chunks_mut(w * 4)
        .enumerate()
        .skip(covered.top)
        .take(covered.height)
        .for_each(|(y, row)| {
            for x in covered.left..covered.left + covered.width {
                let index = (y - covered.top) * covered.width + x - covered.left;
                let (src, inside) = made[index];
                let amount = covered.values[index] * opacity;
                if inside && amount > 0.0 {
                    let pixel = &mut row[x * 4..x * 4 + 4];
                    let dst = [pixel[0], pixel[1], pixel[2], pixel[3]];
                    pixel.copy_from_slice(&mix(dst, src, amount));
                }
            }
        });
}

/// Paint `stroke` with `source`, which is the picture itself when `None`, moved by `from`.
pub fn clone_stroke(picture: &mut Image, source: Option<&Image>, stroke: &Stroke, from: [f32; 2]) {
    let Some(covered) = covered(stroke, (picture.w, picture.h)) else {
        return;
    };
    let made: Vec<([f32; 4], bool)> = {
        let source = source.unwrap_or(picture);
        (0..covered.values.len())
            .into_par_iter()
            .map(|index| {
                let (x, y) = covered.position(index);
                read(source, x, y, from)
            })
            .collect()
    };
    lay(picture, &covered, stroke.opacity, &made);
}

/// Paint `stroke` with the detail of `source`, which is the picture itself when `None`, moved by
/// `from`, taking its light and color from what is around the stroke: what it paints differs from
/// the picture by a smooth surface through how they differ at the stroke's edge.
pub fn heal_stroke(picture: &mut Image, source: Option<&Image>, stroke: &Stroke, from: [f32; 2]) {
    let Some(covered) = covered(stroke, (picture.w, picture.h)) else {
        return;
    };
    let read_all: Vec<([f32; 4], bool)> = {
        let source = source.unwrap_or(picture);
        (0..covered.values.len())
            .into_par_iter()
            .map(|index| {
                let (x, y) = covered.position(index);
                read(source, x, y, from)
            })
            .collect()
    };
    let known: Vec<bool> = read_all
        .iter()
        .zip(&covered.values)
        .map(|((_, inside), value)| !(*inside && *value > 0.0))
        .collect();
    let mut differs: Vec<[f32; 4]> = (0..covered.values.len())
        .map(|index| {
            if !known[index] {
                return [0.0; 4];
            }
            let (x, y) = covered.position(index);
            let (dst, src) = (picture.get(x, y), read_all[index].0);
            std::array::from_fn(|k| dst[k] - src[k])
        })
        .collect();
    membrane_core(&mut differs, &known, covered.width, covered.height);
    let made: Vec<([f32; 4], bool)> = read_all
        .iter()
        .zip(&differs)
        .map(|((src, inside), differ)| {
            let mut healed: [f32; 4] = std::array::from_fn(|k| (src[k] + differ[k]).max(0.0));
            healed[3] = healed[3].min(1.0);
            (healed, *inside)
        })
        .collect();
    lay(picture, &covered, stroke.opacity, &made);
}

/// At most this many pixels are read of each part of a place being weighed.
const MOST_READ: usize = 2048;

/// Where to paint `stroke` from: the place nearby, clear of the stroke, whose pixels best match
/// what is around the stroke, and that has no more going on inside than that does. `None` when no
/// such place fits in the picture.
pub fn find_source(picture: &Image, stroke: &Stroke) -> Option<[f32; 2]> {
    let covered = covered(stroke, (picture.w, picture.h))?;
    let (rw, rh) = (covered.width, covered.height);
    let at = |x: usize, y: usize| covered.values[y * rw + x];
    let mut around = Vec::new();
    let mut inside = Vec::new();
    for y in 0..rh {
        for x in 0..rw {
            if at(x, y) > 0.0 {
                inside.push((x, y));
                continue;
            }
            let touches = (x > 0 && at(x - 1, y) > 0.0)
                || (x + 1 < rw && at(x + 1, y) > 0.0)
                || (y > 0 && at(x, y - 1) > 0.0)
                || (y + 1 < rh && at(x, y + 1) > 0.0);
            if touches {
                around.push((x, y));
            }
        }
    }
    if around.is_empty() || inside.is_empty() {
        return None;
    }
    let thin = |list: Vec<(usize, usize)>| -> Vec<(usize, usize)> {
        let step = (list.len() / MOST_READ).max(1);
        list.into_iter().step_by(step).collect()
    };
    let (around, inside) = (thin(around), thin(inside));
    let (left, top) = (covered.left as f32, covered.top as f32);
    let extent = rw.max(rh) as f32;
    let (w, h) = (picture.w as f32, picture.h as f32);
    let spread_of = |pixels: &[[f32; 4]]| -> f32 {
        let count = pixels.len().max(1) as f32;
        (0..3)
            .map(|k| {
                let mean = pixels.iter().map(|pixel| pixel[k]).sum::<f32>() / count;
                pixels
                    .iter()
                    .map(|pixel| (pixel[k] - mean).powi(2))
                    .sum::<f32>()
                    / count
            })
            .sum()
    };
    let around_here: Vec<[f32; 4]> = around
        .iter()
        .map(|(x, y)| picture.get(covered.left + x, covered.top + y))
        .collect();
    let spread_here = spread_of(&around_here);
    let mut places = Vec::new();
    for reach in [1.1f32, 1.5, 2.0, 3.0, 4.5] {
        for turn in 0..16 {
            let angle = std::f32::consts::TAU * turn as f32 / 16.0;
            let place = [
                (angle.cos() * extent * reach).round(),
                (angle.sin() * extent * reach).round(),
            ];
            let fits = left + place[0] >= 0.0
                && top + place[1] >= 0.0
                && left + rw as f32 + place[0] <= w
                && top + rh as f32 + place[1] <= h;
            if fits {
                places.push(place);
            }
        }
    }
    let weigh = |place: [f32; 2]| -> f32 {
        let (dx, dy) = (place[0] as isize, place[1] as isize);
        let there = |(x, y): &(usize, usize)| {
            picture.get(
                (covered.left + x).wrapping_add_signed(dx),
                (covered.top + y).wrapping_add_signed(dy),
            )
        };
        // What a smooth surface can make up does not count against it: their overall difference.
        let count = around.len() as f32;
        let mut mean = [0.0f32; 3];
        for (here, other) in around_here.iter().zip(around.iter().map(there)) {
            for k in 0..3 {
                mean[k] += (here[k] - other[k]) / count;
            }
        }
        let mut mismatch = 0.0;
        for (here, other) in around_here.iter().zip(around.iter().map(there)) {
            for k in 0..3 {
                mismatch += (here[k] - other[k] - mean[k]).powi(2) / count;
            }
        }
        let inside_there: Vec<[f32; 4]> = inside.iter().map(there).collect();
        mismatch + (spread_of(&inside_there) - spread_here).max(0.0)
    };
    places
        .into_iter()
        .map(|place| (place, weigh(place)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(place, _)| place)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAY: [f32; 4] = [0.3, 0.3, 0.3, 1.0];

    fn dab(x: f32, y: f32, radius: f32) -> Stroke {
        Stroke {
            points: vec![[x, y]],
            radius,
            hardness: 1.0,
            opacity: 1.0,
        }
    }

    fn square(picture: &mut Image, x0: usize, y0: usize, size: usize, color: [f32; 4]) {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                let start = (y * picture.w + x) * 4;
                picture.px[start..start + 4].copy_from_slice(&color);
            }
        }
    }

    #[test]
    fn a_stroke_paints_along_its_line_and_softly_by_its_hardness_and_opacity() {
        let mut picture = Image::filled(100, 60, GRAY);
        let other = Image::filled(100, 60, [1.0; 4]);
        let line = Stroke {
            points: vec![[20.0, 30.0], [80.0, 30.0]],
            radius: 5.0,
            hardness: 1.0,
            opacity: 0.5,
        };
        clone_stroke(&mut picture, Some(&other), &line, [0.0, 0.0]);
        assert!((picture.get(50, 30)[0] - 0.65).abs() < 1e-5);
        assert_eq!(picture.get(50, 40), GRAY);
        assert_eq!(picture.get(10, 30), GRAY);
        let mut soft = Image::filled(100, 60, GRAY);
        let dabbed = Stroke {
            hardness: 0.0,
            opacity: 1.0,
            ..dab(50.0, 30.0, 20.0)
        };
        clone_stroke(&mut soft, Some(&other), &dabbed, [0.0, 0.0]);
        let (middle, part, edge) = (
            soft.get(50, 30)[0],
            soft.get(50, 40)[0],
            soft.get(50, 48)[0],
        );
        assert!(
            middle > 0.99 && part > 0.4 && part < 0.9 && edge < 0.4,
            "{middle} {part} {edge}"
        );
    }

    /// A left-to-right ramp with a fine check over it.
    fn textured(w: usize, h: usize) -> Image {
        let mut picture = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = 0.1 + 0.6 * x as f32 / w as f32;
                let check = if (x / 3 + y / 3) % 2 == 0 {
                    0.03
                } else {
                    -0.03
                };
                let value = base + check;
                let start = (y * w + x) * 4;
                picture.px[start..start + 4].copy_from_slice(&[value, value, value, 1.0]);
            }
        }
        picture
    }

    #[test]
    fn healing_takes_detail_from_there_and_light_from_here() {
        let (w, h) = (200, 80);
        let clean = textured(w, h);
        let mut picture = clean.clone();
        // A blemish on the bright side.
        square(&mut picture, 150, 35, 8, [0.0, 0.0, 0.0, 1.0]);
        let stroke = dab(154.0, 39.0, 9.0);
        // From far to the left, which is much darker: a clone shows it, a heal does not.
        let from = [-120.0, 0.0];
        let mut cloned = picture.clone();
        clone_stroke(&mut cloned, None, &stroke, from);
        let mut healed = picture.clone();
        heal_stroke(&mut healed, None, &stroke, from);
        let off = |found: &Image| {
            let mut most = 0.0f32;
            for y in 33..46 {
                for x in 148..160 {
                    most = most.max((found.get(x, y)[0] - clean.get(x, y)[0]).abs());
                }
            }
            most
        };
        assert!(off(&cloned) > 0.3, "{}", off(&cloned));
        assert!(off(&healed) < 0.1, "{}", off(&healed));
        // Its edge does not show.
        let seam = (healed.get(145, 39)[0] - healed.get(144, 39)[0]).abs();
        assert!(seam < 0.08, "{seam}");
        // Away from it nothing changed.
        assert_eq!(healed.get(100, 10), picture.get(100, 10));
    }

    #[test]
    fn healing_holds_values_over_1_and_keeps_alpha_in_range() {
        let mut picture = Image::filled(100, 60, [3.0, 3.0, 3.0, 1.0]);
        square(&mut picture, 70, 26, 8, [0.0, 0.0, 0.0, 1.0]);
        heal_stroke(&mut picture, None, &dab(74.0, 30.0, 8.0), [-50.0, 0.0]);
        let middle = picture.get(74, 30);
        assert!((middle[0] - 3.0).abs() < 0.01, "{middle:?}");
        assert!(middle[3] <= 1.0);
    }

    #[test]
    fn a_place_to_paint_from_is_found_clear_of_other_marks() {
        let mut picture = textured(240, 120);
        let blemish = (120, 60);
        square(
            &mut picture,
            blemish.0 - 4,
            blemish.1 - 4,
            8,
            [0.0, 0.0, 0.0, 1.0],
        );
        // Marks where it would otherwise look.
        for (x, y) in [(84, 60), (156, 60), (120, 24), (120, 96)] {
            square(&mut picture, x - 4, y - 4, 8, [1.0, 1.0, 1.0, 1.0]);
        }
        let stroke = dab(blemish.0 as f32, blemish.1 as f32, 8.0);
        let from = find_source(&picture, &stroke).expect("room around it");
        let reach = from[0].hypot(from[1]);
        assert!(reach >= 20.0, "{from:?} would read itself");
        let mut healed = picture.clone();
        heal_stroke(&mut healed, None, &stroke, from);
        let middle = healed.get(blemish.0, blemish.1)[0];
        let wanted = textured(240, 120).get(blemish.0, blemish.1)[0];
        assert!(
            (middle - wanted).abs() < 0.1,
            "{middle} against {wanted} from {from:?}"
        );
        // Nothing bright came with it.
        for y in blemish.1 - 6..blemish.1 + 6 {
            for x in blemish.0 - 6..blemish.0 + 6 {
                assert!(healed.get(x, y)[0] < 0.8, "{x},{y}");
            }
        }
    }

    #[test]
    fn no_place_fits_in_a_picture_all_stroke() {
        let picture = Image::filled(20, 20, GRAY);
        assert_eq!(find_source(&picture, &dab(10.0, 10.0, 8.0)), None);
    }
}
