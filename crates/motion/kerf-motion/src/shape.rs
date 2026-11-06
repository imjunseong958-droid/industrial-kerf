//! Shapes: an outline, filled, stroked, and with its stroke trimmed along its length, as a line that
//! draws itself on is.

use kerf_draw::Image;
use rayon::prelude::*;

/// What a shape's outline is, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Figure {
    Rect {
        width: f32,
        height: f32,
        /// The radius of its corners.
        round: f32,
    },
    Ellipse {
        width: f32,
        height: f32,
    },
    /// Corners on an ellipse, the first straight up.
    Polygon {
        width: f32,
        height: f32,
        sides: u32,
    },
    /// Points on an ellipse, and as many between them at `inner` of the way in.
    Star {
        width: f32,
        height: f32,
        points: u32,
        inner: f32,
    },
    /// Points around the shape's center, joined, and passed through smoothly when `smooth`.
    Path {
        points: Vec<[f32; 2]>,
        closed: bool,
        smooth: bool,
    },
}

/// A shape to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub figure: Figure,
    /// Premultiplied linear light.
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    /// In pixels.
    pub stroke_width: f32,
    /// The part of the outline's length the stroke follows, 0 to 1, and how far that part is moved
    /// along it.
    pub trim: (f32, f32, f32),
}

/// Corners on a curve closely enough that it reads as one at any size drawn.
fn curve_steps(radius: f32) -> usize {
    ((radius.abs().sqrt() * 8.0).ceil() as usize).clamp(16, 720)
}

impl Figure {
    /// Its outline around (0, 0), and whether it closes.
    pub fn outline(&self) -> (Vec<[f32; 2]>, bool) {
        use std::f32::consts::{FRAC_PI_2, TAU};
        match self {
            Self::Rect {
                width,
                height,
                round,
            } => {
                let (w, h) = (width / 2.0, height / 2.0);
                let r = round.clamp(0.0, w.abs().min(h.abs()));
                if r <= 0.0 {
                    return (vec![[-w, -h], [w, -h], [w, h], [-w, h]], true);
                }
                let steps = curve_steps(r) / 4 + 2;
                let mut points = Vec::with_capacity(steps * 4);
                for (corner, start) in [
                    ([w - r, -h + r], -FRAC_PI_2),
                    ([w - r, h - r], 0.0),
                    ([-w + r, h - r], FRAC_PI_2),
                    ([-w + r, -h + r], 2.0 * FRAC_PI_2),
                ] {
                    for step in 0..=steps {
                        let angle = start + FRAC_PI_2 * step as f32 / steps as f32;
                        points.push([corner[0] + r * angle.cos(), corner[1] + r * angle.sin()]);
                    }
                }
                (points, true)
            }
            Self::Ellipse { width, height } => {
                let (w, h) = (width / 2.0, height / 2.0);
                let steps = curve_steps(w.abs().max(h.abs()));
                let points = (0..steps)
                    .map(|step| {
                        let angle = TAU * step as f32 / steps as f32 - FRAC_PI_2;
                        [w * angle.cos(), h * angle.sin()]
                    })
                    .collect();
                (points, true)
            }
            Self::Polygon {
                width,
                height,
                sides,
            } => {
                let (w, h) = (width / 2.0, height / 2.0);
                let sides = (*sides).max(3);
                let points = (0..sides)
                    .map(|side| {
                        let angle = TAU * side as f32 / sides as f32 - FRAC_PI_2;
                        [w * angle.cos(), h * angle.sin()]
                    })
                    .collect();
                (points, true)
            }
            Self::Star {
                width,
                height,
                points,
                inner,
            } => {
                let (w, h) = (width / 2.0, height / 2.0);
                let count = (*points).max(2) * 2;
                let points = (0..count)
                    .map(|index| {
                        let angle = TAU * index as f32 / count as f32 - FRAC_PI_2;
                        let reach = if index % 2 == 0 { 1.0 } else { *inner };
                        [w * reach * angle.cos(), h * reach * angle.sin()]
                    })
                    .collect();
                (points, true)
            }
            Self::Path {
                points,
                closed,
                smooth,
            } => {
                if *smooth && points.len() >= 3 {
                    (catmull_rom(points, *closed), *closed)
                } else {
                    (points.clone(), *closed)
                }
            }
        }
    }
}

/// A smooth curve through `points`.
fn catmull_rom(points: &[[f32; 2]], closed: bool) -> Vec<[f32; 2]> {
    let n = points.len();
    let at = |index: isize| -> [f32; 2] {
        if closed {
            points[index.rem_euclid(n as isize) as usize]
        } else {
            points[index.clamp(0, n as isize - 1) as usize]
        }
    };
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(spans * 16);
    for span in 0..spans {
        let i = span as isize;
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let length = (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
        let steps = curve_steps(length) / 2 + 1;
        for step in 0..steps {
            let t = step as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let axis = |k: usize| {
                0.5 * (2.0 * p1[k]
                    + (p2[k] - p0[k]) * t
                    + (2.0 * p0[k] - 5.0 * p1[k] + 4.0 * p2[k] - p3[k]) * t2
                    + (3.0 * p1[k] - p0[k] - 3.0 * p2[k] + p3[k]) * t3)
            };
            out.push([axis(0), axis(1)]);
        }
    }
    if !closed {
        out.push(at(n as isize - 1));
    }
    out
}

/// The part of an outline from `start` to `end` of its length, moved along by `offset`: runs of
/// points, since around a closed outline it can cross where the outline starts.
fn trimmed(
    points: &[[f32; 2]],
    closed: bool,
    (start, end, offset): (f32, f32, f32),
) -> Vec<Vec<[f32; 2]>> {
    let mut path: Vec<[f32; 2]> = points.to_vec();
    if closed {
        if let Some(first) = points.first() {
            path.push(*first);
        }
    }
    let (start, end) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
    if path.len() < 2 || end <= start {
        return Vec::new();
    }
    let mut lengths = Vec::with_capacity(path.len());
    lengths.push(0.0f32);
    for pair in path.windows(2) {
        let last = *lengths.last().unwrap_or(&0.0);
        lengths.push(last + (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]));
    }
    let total = *lengths.last().unwrap_or(&0.0);
    if total <= 0.0 {
        return Vec::new();
    }
    let point_at = |distance: f32| -> [f32; 2] {
        let after = lengths
            .partition_point(|length| *length <= distance)
            .clamp(1, path.len() - 1);
        let (from, to) = (lengths[after - 1], lengths[after]);
        let along = if to > from {
            (distance - from) / (to - from)
        } else {
            0.0
        };
        let (a, b) = (path[after - 1], path[after]);
        [a[0] + (b[0] - a[0]) * along, a[1] + (b[1] - a[1]) * along]
    };
    let run = |from: f32, to: f32| -> Vec<[f32; 2]> {
        let mut out = vec![point_at(from)];
        for (index, length) in lengths.iter().enumerate() {
            if *length > from && *length < to {
                out.push(path[index]);
            }
        }
        out.push(point_at(to));
        out
    };
    let (from, to) = (start + offset, end + offset);
    if closed {
        // Around and around, its part may cross the join.
        let shift = from.floor();
        let (from, to) = ((from - shift) * total, (to - shift) * total);
        if to <= total {
            vec![run(from, to)]
        } else {
            vec![run(from, total), run(0.0, to - total)]
        }
    } else {
        // An open line has nowhere to go past its ends.
        let (from, to) = (from.clamp(0.0, 1.0) * total, to.clamp(0.0, 1.0) * total);
        if to <= from {
            Vec::new()
        } else {
            vec![run(from, to)]
        }
    }
}

/// How much of each pixel of a row an outline covers: four lines through it, each exact across.
fn fill_row(points: &[[f32; 2]], y: f32, row: &mut [f32]) {
    const LINES: usize = 4;
    let mut crossings: Vec<f32> = Vec::new();
    for line in 0..LINES {
        let at = y + (line as f32 + 0.5) / LINES as f32;
        crossings.clear();
        for (index, a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            if (a[1] <= at) != (b[1] <= at) {
                crossings.push(a[0] + (at - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        crossings.sort_by(f32::total_cmp);
        for span in crossings.chunks_exact(2) {
            let (from, to) = (span[0].max(0.0), span[1].min(row.len() as f32));
            if to <= from {
                continue;
            }
            let (first, last) = (from.floor() as usize, (to.ceil() as usize).min(row.len()));
            for (x, value) in row.iter_mut().enumerate().take(last).skip(first) {
                let left = from.max(x as f32);
                let right = to.min(x as f32 + 1.0);
                *value += (right - left).max(0.0) / LINES as f32;
            }
        }
    }
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

/// How much of each pixel a stroke covers, with where that is. Each line of the outline only looks
/// at the pixels near it, since one curve is hundreds of short lines.
fn stroke_coverage(
    runs: &[Vec<[f32; 2]>],
    half: f32,
    (w, h): (usize, usize),
) -> Option<(Vec<f32>, [usize; 4])> {
    let reach = half + 1.0;
    let near = |low: f32, high: f32, most: usize| {
        (
            (low - reach).floor().clamp(0.0, most as f32) as usize,
            (high + reach).ceil().clamp(0.0, most as f32) as usize,
        )
    };
    let span = |axis: usize, points: &[[f32; 2]]| {
        points
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), point| {
                (low.min(point[axis]), high.max(point[axis]))
            })
    };
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for run in runs {
        let (low, high) = span(0, run);
        let (from, to) = near(low, high, w);
        (left, right) = (left.min(from), right.max(to));
        let (low, high) = span(1, run);
        let (from, to) = near(low, high, h);
        (top, bottom) = (top.min(from), bottom.max(to));
    }
    if right <= left || bottom <= top {
        return None;
    }
    let wide = right - left;
    let mut covered = vec![0.0f32; wide * (bottom - top)];
    for pair in runs.iter().flat_map(|run| run.windows(2)) {
        let (a, b) = (pair[0], pair[1]);
        let (x0, x1) = near(a[0].min(b[0]), a[0].max(b[0]), w);
        let (y0, y1) = near(a[1].min(b[1]), a[1].max(b[1]), h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = distance_to(a, b, x as f32 + 0.5, y as f32 + 0.5);
                let here = &mut covered[(y - top) * wide + x - left];
                *here = here.max((half - d + 0.5).clamp(0.0, 1.0));
            }
        }
    }
    Some((covered, [left, top, right, bottom]))
}

fn over(pixel: &mut [f32], paint: [f32; 4], covered: f32) {
    let keep = 1.0 - paint[3] * covered;
    for (value, paint) in pixel.iter_mut().zip(paint) {
        *value = paint * covered + *value * keep;
    }
}

impl Shape {
    /// Draw it over `image`, centered on `center`.
    pub fn draw(&self, image: &mut Image, center: [f32; 2]) {
        let (w, h) = (image.w, image.h);
        if w == 0 || h == 0 {
            return;
        }
        let (outline, closed) = self.figure.outline();
        let placed: Vec<[f32; 2]> = outline
            .iter()
            .map(|p| [p[0] + center[0], p[1] + center[1]])
            .collect();
        let bounds = |points: &[[f32; 2]], margin: f32| {
            let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
            for point in points {
                low = low.min(point[1]);
                high = high.max(point[1]);
            }
            (
                (low - margin).floor().max(0.0) as usize,
                ((high + margin).ceil().max(0.0) as usize).min(h),
            )
        };
        if self.fill[3] > 0.0 && closed && placed.len() >= 3 {
            let (top, bottom) = bounds(&placed, 1.0);
            image
                .px
                .par_chunks_mut(w * 4)
                .enumerate()
                .skip(top)
                .take(bottom.saturating_sub(top))
                .for_each(|(y, row)| {
                    let mut covered = vec![0.0f32; w];
                    fill_row(&placed, y as f32, &mut covered);
                    for (pixel, covered) in row.chunks_exact_mut(4).zip(&covered) {
                        if *covered > 0.0 {
                            over(pixel, self.fill, covered.min(1.0));
                        }
                    }
                });
        }
        if self.stroke[3] > 0.0 && self.stroke_width > 0.0 {
            let runs = trimmed(&placed, closed, self.trim);
            if let Some((covered, [left, top, right, bottom])) =
                stroke_coverage(&runs, self.stroke_width / 2.0, (w, h))
            {
                let wide = right - left;
                image
                    .px
                    .par_chunks_mut(w * 4)
                    .enumerate()
                    .skip(top)
                    .take(bottom - top)
                    .for_each(|(y, row)| {
                        let line = &covered[(y - top) * wide..(y - top + 1) * wide];
                        let pixels = row[left * 4..right * 4].chunks_exact_mut(4);
                        for (pixel, covered) in pixels.zip(line) {
                            if *covered > 0.0 {
                                over(pixel, self.stroke, *covered);
                            }
                        }
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0; 4];
    const NONE: [f32; 4] = [0.0; 4];

    fn filled(figure: Figure) -> Image {
        let mut image = Image::new(200, 200);
        Shape {
            figure,
            fill: WHITE,
            stroke: NONE,
            stroke_width: 0.0,
            trim: (0.0, 1.0, 0.0),
        }
        .draw(&mut image, [100.0, 100.0]);
        image
    }

    fn stroked(figure: Figure, width: f32, trim: (f32, f32, f32)) -> Image {
        let mut image = Image::new(200, 200);
        Shape {
            figure,
            fill: NONE,
            stroke: WHITE,
            stroke_width: width,
            trim,
        }
        .draw(&mut image, [100.0, 100.0]);
        image
    }

    fn area(image: &Image) -> f32 {
        image.px.chunks_exact(4).map(|pixel| pixel[3]).sum()
    }

    #[test]
    fn filled_shapes_cover_the_area_their_formula_says() {
        use std::f32::consts::PI;
        let close = |image: Image, wanted: f32| {
            let found = area(&image);
            assert!(
                (found - wanted).abs() < wanted * 0.01,
                "{found} against {wanted}"
            );
        };
        close(
            filled(Figure::Rect {
                width: 100.0,
                height: 60.0,
                round: 0.0,
            }),
            6000.0,
        );
        close(
            filled(Figure::Rect {
                width: 100.0,
                height: 60.0,
                round: 20.0,
            }),
            6000.0 - (4.0 - PI) * 400.0,
        );
        close(
            filled(Figure::Ellipse {
                width: 120.0,
                height: 80.0,
            }),
            PI * 60.0 * 40.0,
        );
        // A square on its corner, of diagonal 100.
        close(
            filled(Figure::Polygon {
                width: 100.0,
                height: 100.0,
                sides: 4,
            }),
            5000.0,
        );
        // A star's ten triangles.
        let (outer, inner) = (50.0f32, 25.0f32);
        let triangle = 0.5 * outer * inner * (PI / 5.0).sin();
        close(
            filled(Figure::Star {
                width: 100.0,
                height: 100.0,
                points: 5,
                inner: 0.5,
            }),
            triangle * 10.0,
        );
    }

    #[test]
    fn edges_are_antialiased() {
        let image = filled(Figure::Rect {
            width: 101.0,
            height: 101.0,
            round: 0.0,
        });
        // 100 - 50.5 = 49.5, half way across pixel 49.
        assert!((image.get(49, 100)[3] - 0.5).abs() < 0.01);
        assert_eq!(image.get(50, 100)[3], 1.0);
        assert_eq!(image.get(48, 100)[3], 0.0);
    }

    #[test]
    fn a_stroke_follows_the_outline_at_its_width() {
        // A square's perimeter by its width, give or take its corners.
        let image = stroked(
            Figure::Rect {
                width: 100.0,
                height: 100.0,
                round: 0.0,
            },
            6.0,
            (0.0, 1.0, 0.0),
        );
        let found = area(&image);
        assert!((found - 400.0 * 6.0).abs() < 40.0, "{found}");
        assert!(image.get(100, 50)[3] > 0.9, "on the top edge");
        assert_eq!(image.get(100, 100)[3], 0.0, "not in the middle");
    }

    #[test]
    fn trimming_draws_a_part_of_the_length() {
        let line = || Figure::Path {
            points: vec![[-50.0, 0.0], [50.0, 0.0]],
            closed: false,
            smooth: false,
        };
        let half = stroked(line(), 4.0, (0.0, 0.5, 0.0));
        assert!(half.get(60, 100)[3] > 0.9);
        assert_eq!(half.get(140, 100)[3], 0.0);
        let whole = area(&stroked(line(), 4.0, (0.0, 1.0, 0.0)));
        let part = area(&half);
        assert!((part / whole - 0.5).abs() < 0.03, "{part} of {whole}");
        assert_eq!(area(&stroked(line(), 4.0, (0.5, 0.5, 0.0))), 0.0);
        // Moved along, a part of a closed outline crosses where it starts; a line stops at its end.
        let circle = Figure::Ellipse {
            width: 100.0,
            height: 100.0,
        };
        let plain = area(&stroked(circle.clone(), 4.0, (0.0, 0.25, 0.0)));
        let moved = area(&stroked(circle, 4.0, (0.0, 0.25, 0.9)));
        assert!((moved - plain).abs() < plain * 0.05, "{plain} {moved}");
        let past = area(&stroked(line(), 4.0, (0.0, 0.5, 0.75)));
        assert!(past < part * 0.6, "{part} {past}");
    }

    #[test]
    fn a_smooth_path_goes_through_its_points() {
        let points = vec![[-60.0, 0.0], [0.0, -40.0], [60.0, 0.0]];
        let (smooth, _) = Figure::Path {
            points: points.clone(),
            closed: false,
            smooth: true,
        }
        .outline();
        for point in &points {
            assert!(
                smooth
                    .iter()
                    .any(|at| (at[0] - point[0]).hypot(at[1] - point[1]) < 0.01),
                "{point:?}"
            );
        }
        assert!(smooth.len() > points.len() * 4);
    }

    #[test]
    fn an_open_outline_fills_nothing() {
        let image = filled(Figure::Path {
            points: vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0]],
            closed: false,
            smooth: false,
        });
        assert_eq!(area(&image), 0.0);
    }

    #[test]
    fn colors_go_over_what_is_there() {
        let mut image = Image::filled(50, 50, [0.0, 0.0, 1.0, 1.0]);
        Shape {
            figure: Figure::Rect {
                width: 50.0,
                height: 50.0,
                round: 0.0,
            },
            fill: [0.5, 0.0, 0.0, 0.5],
            stroke: NONE,
            stroke_width: 0.0,
            trim: (0.0, 1.0, 0.0),
        }
        .draw(&mut image, [25.0, 25.0]);
        assert_eq!(image.get(25, 25), [0.5, 0.0, 0.5, 1.0]);
    }
}
