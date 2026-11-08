//! Outlines combined and offset, as closed polygons, since a shape flattens to one before it draws.
//!
//! Carried from EffectCraft's `path` crate (`boolean.rs` and `offset.rs`, MIT OR Apache-2.0), which
//! works on curves: the sweep (`linesweeper`) finds where outlines cross and which side of each edge
//! is inside, so what comes out is simple closed contours with holes wound against what holds them,
//! and they fill the same whichever way they are read. Growing or shrinking is that same sweep
//! against a stroke of twice the distance (`kurbo`), with the miters a designer's offset has.

use kurbo::{BezPath, PathEl, Point, Stroke, StrokeOpts};
use linesweeper::topology::{BinaryWindingNumber, ContourIdx, Contours, Topology};

/// A closed outline: its corners, in pixels, the last joining the first.
pub type Contour = Vec<[f32; 2]>;

/// How an outline combines with what comes before it, in the words a mask uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Combine {
    #[default]
    Add,
    Subtract,
    Intersect,
    /// Where one or the other is, but not both.
    Difference,
}

/// How far a stroked corner may reach, in stroke widths, before it is cut off. Illustrator's default.
const MITER_LIMIT: f64 = 4.0;
/// Curves in what comes back, flattened this closely, in pixels.
const FLATNESS: f64 = 0.05;
/// Pieces the sweep leaves that are thinner than this are its rounding, not part of the shape.
const SLIVER: f64 = 0.01;

fn to_path(contours: &[Contour]) -> BezPath {
    let mut path = BezPath::new();
    for contour in contours.iter().filter(|contour| contour.len() >= 3) {
        path.move_to(point(contour[0]));
        for corner in &contour[1..] {
            path.line_to(point(*corner));
        }
        path.close_path();
    }
    path
}

fn point([x, y]: [f32; 2]) -> Point {
    Point::new(f64::from(x), f64::from(y))
}

/// The corners of a closed path, each subpath a contour.
fn corners(path: &BezPath) -> Vec<Contour> {
    let mut contours = Vec::new();
    let mut current: Contour = Vec::new();
    kurbo::flatten(path.iter(), FLATNESS, |element| match element {
        PathEl::MoveTo(at) => {
            if current.len() >= 3 {
                contours.push(std::mem::take(&mut current));
            }
            current = vec![[at.x as f32, at.y as f32]];
        }
        PathEl::LineTo(at) => current.push([at.x as f32, at.y as f32]),
        _ => {}
    });
    if current.len() >= 3 {
        contours.push(current);
    }
    contours
}

/// The area a contour holds, by its winding.
fn signed_area(contour: &[[f32; 2]]) -> f64 {
    let mut twice = 0.0;
    for (index, a) in contour.iter().enumerate() {
        let b = contour[(index + 1) % contour.len()];
        twice += f64::from(a[0]) * f64::from(b[1]) - f64::from(b[0]) * f64::from(a[1]);
    }
    twice / 2.0
}

fn perimeter(contour: &[[f32; 2]]) -> f64 {
    let mut length = 0.0;
    for (index, a) in contour.iter().enumerate() {
        let b = contour[(index + 1) % contour.len()];
        length += f64::from(b[0] - a[0]).hypot(f64::from(b[1] - a[1]));
    }
    length
}

/// What `contours` cover: holes wind against what holds them, so their areas take away.
pub fn area(contours: &[Contour]) -> f64 {
    contours
        .iter()
        .map(|contour| signed_area(contour))
        .sum::<f64>()
        .abs()
}

/// Contours as polygons, without what is too thin to be anything but the sweep's rounding.
fn kept(contours: impl IntoIterator<Item = BezPath>) -> Vec<Contour> {
    contours
        .into_iter()
        .flat_map(|path| corners(&path))
        .filter(|contour| {
            let around = perimeter(contour);
            around > 0.0 && 2.0 * signed_area(contour).abs() / around >= SLIVER
        })
        .collect()
}

/// The sweep's tolerance for what spans these points, as `linesweeper::binary_op` chooses it.
fn tolerance(paths: [&BezPath; 2]) -> f64 {
    use kurbo::Shape as _;
    let mut most: f64 = 0.0;
    for path in paths {
        let bounds = path.bounding_box();
        for edge in [bounds.x0, bounds.x1, bounds.y0, bounds.y1] {
            most = most.max(edge.abs());
        }
    }
    (most * f64::EPSILON * 64.0).max(1e-6)
}

fn even_odd(winding: i32) -> bool {
    winding % 2 != 0
}

/// `region` and `other` combined `how`. Each is read as a fill reads it, a point being inside when
/// an odd number of outlines surround it.
pub fn combine(region: &[Contour], other: &[Contour], how: Combine) -> Vec<Contour> {
    let (a, b) = (to_path(region), to_path(other));
    let Ok(topology) = Topology::from_paths_binary(&a, &b, tolerance([&a, &b])) else {
        return region.to_vec();
    };
    let contours = topology.contours(|winding: &BinaryWindingNumber| {
        let (in_a, in_b) = (even_odd(winding.shape_a), even_odd(winding.shape_b));
        match how {
            Combine::Add => in_a || in_b,
            Combine::Subtract => in_a && !in_b,
            Combine::Intersect => in_a && in_b,
            Combine::Difference => in_a != in_b,
        }
    });
    kept(contours.contours().map(|contour| contour.path.clone()))
}

/// `points` as a closed polygon or an open line, as a stroke of `width` would cover it.
fn stroked(path: &BezPath, width: f64) -> BezPath {
    let style = Stroke::new(width)
        .with_join(kurbo::Join::Miter)
        .with_caps(kurbo::Cap::Butt)
        .with_miter_limit(MITER_LIMIT);
    // Closed, as the sweep requires of what it is given.
    let mut closed = BezPath::new();
    for element in kurbo::stroke(path.iter(), &style, &StrokeOpts::default(), 1e-3).iter() {
        match element {
            PathEl::ClosePath => {}
            PathEl::MoveTo(at) => {
                if !closed.elements().is_empty() {
                    closed.close_path();
                }
                closed.move_to(at);
            }
            other => closed.push(other),
        }
    }
    if !closed.elements().is_empty() {
        closed.close_path();
    }
    closed
}

/// How far `at` is from the nearest edge of `contours`.
fn distance_to_edges(contours: &[Contour], at: Point) -> f64 {
    let mut nearest = f64::INFINITY;
    for contour in contours {
        for (index, a) in contour.iter().enumerate() {
            let (a, b) = (point(*a), point(contour[(index + 1) % contour.len()]));
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let along = dx * dx + dy * dy;
            let t = if along > 0.0 {
                (((at.x - a.x) * dx + (at.y - a.y) * dy) / along).clamp(0.0, 1.0)
            } else {
                0.0
            };
            nearest = nearest.min((at.x - a.x - t * dx).hypot(at.y - a.y - t * dy));
        }
    }
    nearest
}

/// A point well inside a contour: the middle of the widest span on a few lines across it.
fn deep_point(contour: &[[f32; 2]]) -> Option<Point> {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for corner in contour {
        low = low.min(f64::from(corner[1]));
        high = high.max(f64::from(corner[1]));
    }
    if high <= low {
        return None;
    }
    let mut best: Option<(f64, Point)> = None;
    for share in [0.5, 0.3, 0.7] {
        let y = low + (high - low) * share;
        let mut crossings = Vec::new();
        for (index, a) in contour.iter().enumerate() {
            let (a, b) = (point(*a), point(contour[(index + 1) % contour.len()]));
            if (a.y <= y) != (b.y <= y) {
                crossings.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
        crossings.sort_by(f64::total_cmp);
        for span in crossings.as_chunks::<2>().0 {
            let wide = span[1] - span[0];
            if best.is_none_or(|(most, _)| wide > most) {
                best = Some((wide, Point::new((span[0] + span[1]) / 2.0, y)));
            }
        }
    }
    best.map(|(_, at)| at)
}

fn marked_or_held_by_marked(contours: &Contours, mut index: usize, marked: &[bool]) -> bool {
    loop {
        if marked[index] {
            return true;
        }
        match contours[ContourIdx(index)].parent {
            Some(ContourIdx(parent)) => index = parent,
            None => return false,
        }
    }
}

/// `region` grown by `amount` pixels, or with a negative shrunk.
pub fn offset(region: &[Contour], amount: f32) -> Vec<Contour> {
    if amount == 0.0 || !amount.is_finite() {
        return region.to_vec();
    }
    let distance = f64::from(amount.abs());
    let path = to_path(region);
    let ring = stroked(&path, 2.0 * distance);
    let grow = amount > 0.0;
    let Ok(topology) = Topology::from_paths_binary(&path, &ring, tolerance([&path, &ring])) else {
        return region.to_vec();
    };
    let contours = topology.contours(|winding: &BinaryWindingNumber| {
        let (inside, near) = (even_odd(winding.shape_a), winding.shape_b != 0);
        if grow {
            inside || near
        } else {
            inside && !near
        }
    });
    // Where the distance is more than a curve's radius, the stroke's inner side folds over and its
    // loops cancel, which leaves pieces that are really within the distance of the outline: holes
    // when growing, islands when shrinking. A point deep in such a piece gives it away.
    let flat: Vec<Vec<Contour>> = contours
        .contours()
        .map(|contour| corners(&contour.path))
        .collect();
    let spurious: Vec<bool> = contours
        .contours()
        .zip(&flat)
        .map(|(contour, pieces)| {
            contour.outer != grow
                && pieces.iter().any(|piece| {
                    deep_point(piece).is_some_and(|at| {
                        distance_to_edges(region, at) < distance * (1.0 - 1e-4) - 1e-6
                    })
                })
        })
        .collect();
    kept(
        (0..spurious.len())
            .filter(|index| !marked_or_held_by_marked(&contours, *index, &spurious))
            .map(|index| contours[ContourIdx(index)].path.clone()),
    )
}

/// The open line `points` as the region within `amount` pixels of it.
pub fn around_line(points: &[[f32; 2]], amount: f32) -> Vec<Contour> {
    let Some((first, rest)) = points.split_first() else {
        return Vec::new();
    };
    let mut line = BezPath::new();
    line.move_to(point(*first));
    for corner in rest {
        line.line_to(point(*corner));
    }
    let ring = stroked(&line, 2.0 * f64::from(amount.abs()));
    let empty = BezPath::new();
    let Ok(topology) = Topology::from_paths_binary(&empty, &ring, tolerance([&empty, &ring]))
    else {
        return Vec::new();
    };
    let contours = topology.contours(|winding: &BinaryWindingNumber| winding.shape_b != 0);
    kept(contours.contours().map(|contour| contour.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn square(x: f32, y: f32, side: f32) -> Contour {
        vec![[x, y], [x + side, y], [x + side, y + side], [x, y + side]]
    }

    fn circle(x: f32, y: f32, radius: f32) -> Contour {
        (0..720)
            .map(|step| {
                let angle = std::f32::consts::TAU * step as f32 / 720.0;
                [x + radius * angle.cos(), y + radius * angle.sin()]
            })
            .collect()
    }

    fn close(found: f64, wanted: f64, share: f64) {
        assert!(
            (found - wanted).abs() <= wanted.abs() * share,
            "{found} against {wanted}"
        );
    }

    #[test]
    fn overlapping_squares_have_the_areas_they_should() {
        // Two squares of 100, overlapping in a square of 50.
        let (a, b) = (
            vec![square(0.0, 0.0, 100.0)],
            vec![square(50.0, 50.0, 100.0)],
        );
        close(area(&combine(&a, &b, Combine::Add)), 17500.0, 1e-4);
        close(area(&combine(&a, &b, Combine::Intersect)), 2500.0, 1e-4);
        close(area(&combine(&a, &b, Combine::Subtract)), 7500.0, 1e-4);
        close(area(&combine(&a, &b, Combine::Difference)), 15000.0, 1e-4);
        assert_eq!(combine(&a, &b, Combine::Add).len(), 1);
        // The difference touches at two corners: its two parts, however they are joined.
        assert!(!combine(&a, &b, Combine::Difference).is_empty());
    }

    #[test]
    fn circles_have_the_areas_their_formula_says() {
        // Two circles of radius 100 with centres 100 apart: the lens between them.
        let radius = 100.0f64;
        let lens = 2.0 * radius * radius * (PI / 3.0)
            - 0.5 * radius * (4.0 * radius * radius - radius * radius).sqrt();
        let whole = PI * radius * radius;
        let (a, b) = (
            vec![circle(0.0, 0.0, 100.0)],
            vec![circle(100.0, 0.0, 100.0)],
        );
        close(area(&combine(&a, &b, Combine::Intersect)), lens, 1e-3);
        close(
            area(&combine(&a, &b, Combine::Add)),
            2.0 * whole - lens,
            1e-3,
        );
        close(
            area(&combine(&a, &b, Combine::Subtract)),
            whole - lens,
            1e-3,
        );
        close(
            area(&combine(&a, &b, Combine::Difference)),
            2.0 * (whole - lens),
            1e-3,
        );
    }

    #[test]
    fn a_hole_is_a_contour_that_takes_away() {
        let holed = combine(
            &[square(0.0, 0.0, 100.0)],
            &[square(25.0, 25.0, 50.0)],
            Combine::Subtract,
        );
        assert_eq!(holed.len(), 2);
        close(area(&holed), 7500.0, 1e-4);
    }

    #[test]
    fn apart_they_add_and_nothing_is_left_of_what_they_share() {
        let (a, b) = (vec![square(0.0, 0.0, 10.0)], vec![square(50.0, 0.0, 10.0)]);
        assert_eq!(combine(&a, &b, Combine::Add).len(), 2);
        assert!(combine(&a, &b, Combine::Intersect).is_empty());
        close(area(&combine(&a, &b, Combine::Subtract)), 100.0, 1e-4);
        // Nothing to combine with changes nothing.
        close(area(&combine(&a, &[], Combine::Add)), 100.0, 1e-4);
        assert!(combine(&[], &a, Combine::Subtract).is_empty());
    }

    #[test]
    fn offsets_have_the_areas_their_formula_says() {
        // A square grown by 10 keeps its corners, and shrunk loses its sides.
        let square = vec![square(0.0, 0.0, 100.0)];
        close(area(&offset(&square, 10.0)), 120.0 * 120.0, 1e-4);
        close(area(&offset(&square, -10.0)), 80.0 * 80.0, 1e-4);
        // A circle's radius changes by the amount.
        let round = vec![circle(0.0, 0.0, 100.0)];
        close(area(&offset(&round, 25.0)), PI * 125.0 * 125.0, 1e-3);
        close(area(&offset(&round, -25.0)), PI * 75.0 * 75.0, 1e-3);
        // Nothing moves it not at all, and shrinking past the middle leaves nothing.
        assert_eq!(offset(&round, 0.0), round);
        assert!(offset(&round, -150.0).is_empty(), "past the middle");
    }

    #[test]
    fn an_offset_grows_a_hole_smaller() {
        let holed = combine(
            &[square(0.0, 0.0, 100.0)],
            &[square(25.0, 25.0, 50.0)],
            Combine::Subtract,
        );
        // Out by 5: outside 110 square, hole 40.
        close(
            area(&offset(&holed, 5.0)),
            110.0 * 110.0 - 40.0 * 40.0,
            1e-3,
        );
        // In by 5: outside 90 square, hole 60.
        close(area(&offset(&holed, -5.0)), 90.0 * 90.0 - 60.0 * 60.0, 1e-3);
        // Out by 30, the hole closes.
        let filled = offset(&holed, 30.0);
        close(area(&filled), 160.0 * 160.0, 1e-3);
        assert_eq!(filled.len(), 1);
    }

    #[test]
    fn a_line_grows_into_a_region() {
        let around = around_line(&[[0.0, 0.0], [100.0, 0.0]], 5.0);
        close(area(&around), 1000.0, 1e-4);
    }

    #[test]
    fn what_is_not_a_number_changes_nothing() {
        let square = vec![square(0.0, 0.0, 100.0)];
        assert_eq!(offset(&square, f32::NAN), square);
    }
}
