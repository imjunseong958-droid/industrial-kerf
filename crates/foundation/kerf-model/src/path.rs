//! A clip's motion path: a smooth curve through points a document lists, in the same units as `x`
//! and `y`, and `along` that says how far along it the clip is. Spacing is by distance on the
//! canvas, so an even `along` is an even speed however unevenly the points are spaced.

/// Samples of each span between two points, to measure distance along it.
const SAMPLES: usize = 32;

/// The curve through a clip's `path=` points.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionPath {
    points: Vec<[f64; 2]>,
    /// How many canvas heights one frame width is.
    aspect: f64,
    /// Distance along the curve at each sample, from 0, in canvas heights.
    distances: Vec<f64>,
}

/// Where a clip is on its path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub x: f64,
    pub y: f64,
    /// The direction it is heading, in degrees clockwise from the right.
    pub heading: f64,
}

fn parse_point(point: &str) -> Result<[f64; 2], String> {
    let (x, y) = point
        .split_once(',')
        .ok_or_else(|| format!("`{point}` should read x,y"))?;
    let number = |text: &str| {
        text.parse::<f64>()
            .ok()
            .filter(|number| number.is_finite() && number.abs() <= 10.0)
            .ok_or_else(|| format!("`{text}` is not a number from -10 to 10"))
    };
    Ok([number(x)?, number(y)?])
}

/// The points `text` lists, `x,y x,y`, however many.
pub fn parse_points(text: &str) -> Result<Vec<[f64; 2]>, String> {
    text.split_whitespace().map(parse_point).collect()
}

/// The moves `text` lists, `x,y>x,y x,y>x,y>x,y`: each drags from its first point through the rest.
pub fn parse_moves(text: &str) -> Result<Vec<Vec<[f64; 2]>>, String> {
    text.split_whitespace()
        .map(|chain| {
            let points = chain
                .split('>')
                .map(parse_point)
                .collect::<Result<Vec<_>, String>>()?;
            if points.len() < 2 {
                return Err(format!("`{chain}` should read x,y>x,y"));
            }
            Ok(points)
        })
        .collect()
}

/// The points `text` lists, `x,y x,y`.
pub fn parse_path_points(text: &str) -> Result<Vec<[f64; 2]>, String> {
    let points = parse_points(text)?;
    if points.len() < 2 {
        return Err("a path needs at least two points".to_owned());
    }
    Ok(points)
}

impl MotionPath {
    /// The curve through `points` on a canvas `aspect` times as wide as it is high.
    pub fn new(points: Vec<[f64; 2]>, aspect: f64) -> Self {
        let mut path = Self {
            points,
            aspect,
            distances: Vec::new(),
        };
        let total = (path.points.len() - 1) * SAMPLES;
        let mut distances = Vec::with_capacity(total + 1);
        distances.push(0.0);
        let mut last = path.curve(0.0);
        let mut walked = 0.0;
        for sample in 1..=total {
            let at = path.curve(sample as f64 / SAMPLES as f64);
            walked += ((at[0] - last[0]) * aspect).hypot(at[1] - last[1]);
            distances.push(walked);
            last = at;
        }
        path.distances = distances;
        path
    }

    /// The centripetal Catmull-Rom curve through the points, at `u` points along. Centripetal
    /// because the plain curve overshoots and doubles back where points are unevenly spaced.
    fn curve(&self, u: f64) -> [f64; 2] {
        let last = self.points.len() - 1;
        let span = (u.floor() as usize).min(last - 1);
        let t = (u - span as f64).clamp(0.0, 1.0);
        let (p1, p2) = (self.points[span], self.points[span + 1]);
        // Past the ends, a point continuing straight on.
        let p0 = if span == 0 {
            [2.0 * p1[0] - p2[0], 2.0 * p1[1] - p2[1]]
        } else {
            self.points[span - 1]
        };
        let p3 = if span + 2 > last {
            [2.0 * p2[0] - p1[0], 2.0 * p2[1] - p1[1]]
        } else {
            self.points[span + 2]
        };
        // Knots a square root of the distance apart, on the canvas.
        let gap = |a: [f64; 2], b: [f64; 2]| {
            ((a[0] - b[0]) * self.aspect)
                .hypot(a[1] - b[1])
                .sqrt()
                .max(1e-9)
        };
        let t0 = 0.0;
        let t1 = t0 + gap(p0, p1);
        let t2 = t1 + gap(p1, p2);
        let t3 = t2 + gap(p2, p3);
        let at = t1 + (t2 - t1) * t;
        // Barry and Goldman's pyramid.
        let lerp = |a: [f64; 2], b: [f64; 2], low: f64, high: f64| {
            let k = (at - low) / (high - low);
            [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]
        };
        let a1 = lerp(p0, p1, t0, t1);
        let a2 = lerp(p1, p2, t1, t2);
        let a3 = lerp(p2, p3, t2, t3);
        let b1 = lerp(a1, a2, t0, t2);
        let b2 = lerp(a2, a3, t1, t3);
        lerp(b1, b2, t1, t2)
    }

    /// `u` for a share of the whole distance.
    fn at_distance(&self, along: f64) -> f64 {
        let total = self.distances[self.distances.len() - 1];
        if total <= 0.0 {
            return 0.0;
        }
        let wanted = along.clamp(0.0, 1.0) * total;
        let next = self
            .distances
            .partition_point(|distance| *distance < wanted)
            .clamp(1, self.distances.len() - 1);
        let (low, high) = (self.distances[next - 1], self.distances[next]);
        let within = if high > low {
            (wanted - low) / (high - low)
        } else {
            0.0
        };
        ((next - 1) as f64 + within) / SAMPLES as f64
    }

    /// Where a clip `along` of the way along is, from 0 to 1 of the distance.
    pub fn at(&self, along: f64) -> PathPoint {
        let u = self.at_distance(along);
        let [x, y] = self.curve(u);
        let last = (self.points.len() - 1) as f64;
        const STEP: f64 = 1e-3;
        let (before, after) = (
            self.curve((u - STEP).max(0.0)),
            self.curve((u + STEP).min(last)),
        );
        let heading = (after[1] - before[1])
            .atan2((after[0] - before[0]) * self.aspect)
            .to_degrees();
        PathPoint { x, y, heading }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str, aspect: f64) -> MotionPath {
        MotionPath::new(parse_path_points(text).expect("points"), aspect)
    }

    fn close(a: f64, b: f64, tolerance: f64) {
        assert!((a - b).abs() <= tolerance, "{a} against {b}");
    }

    #[test]
    fn it_reads_points_and_refuses_what_is_not() {
        assert_eq!(
            parse_path_points("0,0 0.5,-0.25").expect("points"),
            [[0.0, 0.0], [0.5, -0.25]]
        );
        for refused in ["", "0,0", "0 0", "0,0 x,1", "0,0 11,0", "0,0 NaN,0"] {
            assert!(parse_path_points(refused).is_err(), "{refused}");
        }
    }

    /// `pathsAreTheCompositorsPaths` in the Qt editor's `graph_test.cpp` holds the curve the
    /// picture draws to this same table.
    #[test]
    fn paths_read_the_table_the_picture_is_held_to() {
        const WIDE: f64 = 16.0 / 9.0;
        const TALL: f64 = 9.0 / 16.0;
        const CURVED: &str = "0,0 0.2,0.3 0.5,-0.1 0.8,0.2";
        const LONG: &str = "-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2";
        const TABLE: &[(&str, f64, f64, f64, f64, f64)] = &[
            ("0,0 1,0", 1.0, 0.25, 0.25, 0.0, 0.0),
            (CURVED, WIDE, 0.0, 0.0, 0.0, 40.186380),
            (CURVED, WIDE, 0.2, 0.138852744, 0.255491011, 37.182011),
            (CURVED, WIDE, 0.5, 0.377158462, 0.044861162, -46.192624),
            (CURVED, WIDE, 0.8, 0.637278976, -0.006712763, 34.546793),
            (CURVED, WIDE, 1.0, 0.8, 0.2, 29.384498),
            ("0,0 0.01,0 0.02,0 1,0", 1.0, 0.5, 0.499950046, 0.0, 0.0),
            (
                "0,0 0.5,0 0.5,0.5",
                2.0,
                0.5,
                0.381055017,
                -0.061333311,
                0.422966,
            ),
            (
                "0,0 0.5,0 0.5,0.5",
                2.0,
                0.9,
                0.508284207,
                0.348099501,
                99.522964,
            ),
            (LONG, TALL, 0.1, -0.251897404, -0.034195522, -59.751285),
            (LONG, TALL, 0.37, 0.146873937, -0.219570365, 84.473801),
            (LONG, TALL, 0.66, 0.182237509, 0.237851333, 68.262319),
            (LONG, TALL, 0.95, 0.354996038, -0.127356395, -115.521511),
            ("0.3,0.3 0.3,0.3", 1.0, 0.5, 0.3, 0.3, 0.0),
        ];
        for (text, aspect, along, x, y, heading) in TABLE {
            let point = path(text, *aspect).at(*along);
            close(point.x, *x, 1e-6);
            close(point.y, *y, 1e-6);
            close(point.heading, *heading, 1e-3);
        }
    }

    #[test]
    fn it_passes_through_every_point_and_ends_where_it_should() {
        let curved = path("0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0);
        close(curved.at(0.0).x, 0.0, 1e-9);
        close(curved.at(1.0).x, 0.8, 1e-9);
        close(curved.at(1.0).y, 0.2, 1e-9);
        for [x, y] in [[0.2, 0.3], [0.5, -0.1]] {
            let nearest = (0..=1000)
                .map(|step| curved.at(f64::from(step) / 1000.0))
                .map(|point| (point.x - x).hypot(point.y - y))
                .fold(f64::MAX, f64::min);
            assert!(nearest < 2e-3, "{nearest}");
        }
    }

    #[test]
    fn even_along_is_even_speed_however_the_points_are_spaced() {
        // Three points close together, then a long way to the last.
        let uneven = path("0,0 0.01,0 0.02,0 1,0", 1.0);
        for step in 0..=20 {
            let along = f64::from(step) / 20.0;
            close(uneven.at(along).x, along, 0.01);
        }
    }

    #[test]
    fn speed_counts_on_the_canvas_not_in_fractions() {
        // A wide canvas: across takes longer than down over the same fraction.
        let bent = path("0,0 0.5,0 0.5,0.5", 2.0);
        // Halfway along the distance is still on the long way across, short of the corner, which
        // a smooth curve bulges around a little.
        let halfway = bent.at(0.5);
        assert!(halfway.x < 0.45 && halfway.y.abs() < 0.1, "{halfway:?}");
    }

    #[test]
    fn heading_follows_the_curve() {
        let right = path("0,0 1,0", 1.0);
        close(right.at(0.5).heading, 0.0, 1e-6);
        let down = path("0,0 0,1", 1.0);
        close(down.at(0.5).heading, 90.0, 1e-6);
        let left = path("1,0 0,0", 1.0);
        close(left.at(0.5).heading.abs(), 180.0, 1e-6);
        // At the very ends too, where the step before or after runs out.
        close(right.at(0.0).heading, 0.0, 1e-6);
        close(right.at(1.0).heading, 0.0, 1e-6);
    }

    #[test]
    fn a_path_going_nowhere_stays_put() {
        let still = path("0.3,0.3 0.3,0.3", 1.0);
        let point = still.at(0.5);
        close(point.x, 0.3, 1e-12);
        close(point.y, 0.3, 1e-12);
        assert!(point.heading.is_finite());
    }
}
