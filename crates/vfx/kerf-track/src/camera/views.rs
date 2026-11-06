//! What two views of a scene say of it, from Hartley & Zisserman, *Multiple View Geometry* (2nd ed.)
//! and Ma et al., *An Invitation to 3-D Vision*, as EffectCraft's `track/src/camtrack/geometry.rs` has
//! it: the essential matrix by RANSAC, a flat scene's homography taken apart, triangulation, and the
//! rotation that best turns one set of rays to another.
//!
//! Points here are calibrated: pixels from the middle of the frame over the focal length. A view is
//! where it stands and how it is turned, and sees `X` at `turn (X - place)`.

use super::math::*;

/// How a view stands: `turn` takes the world to it, from `place`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub turn: Matrix,
    pub place: Vector,
}

impl View {
    pub const AT_REST: View = View {
        turn: IDENTITY,
        place: [0.0; 3],
    };

    /// From `seen = turn X + shift`.
    pub fn from_shift(turn: Matrix, shift: Vector) -> View {
        View {
            turn,
            place: times(apply_back(&turn, shift), -1.0),
        }
    }

    pub fn shift(&self) -> Vector {
        times(apply(&self.turn, self.place), -1.0)
    }

    pub fn seen(&self, point: Vector) -> Vector {
        apply(&self.turn, minus(point, self.place))
    }
}

/// Repeatable draws for RANSAC: the same input gives the same answer.
pub struct Draws(u64);

impl Draws {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n.max(1) as u64) as usize
    }

    /// A number from -0.5 to 0.5.
    #[cfg(test)]
    pub fn part(&mut self) -> f64 {
        self.below(100_000) as f64 / 100_000.0 - 0.5
    }

    fn distinct(&mut self, n: usize, want: usize, out: &mut Vec<usize>) {
        out.clear();
        let mut guard = 0;
        while out.len() < want && guard < 1000 {
            guard += 1;
            let pick = self.below(n);
            if !out.contains(&pick) {
                out.push(pick);
            }
        }
    }
}

/// Hartley's normalisation: the centroid to the origin, the mean distance to the square root of 2.
fn normalizer(points: &[[f64; 2]]) -> Matrix {
    let n = points.len().max(1) as f64;
    let c = [
        points.iter().map(|p| p[0]).sum::<f64>() / n,
        points.iter().map(|p| p[1]).sum::<f64>() / n,
    ];
    let d = points
        .iter()
        .map(|p| (p[0] - c[0]).hypot(p[1] - c[1]))
        .sum::<f64>()
        / n;
    let s = if d > 1e-12 {
        std::f64::consts::SQRT_2 / d
    } else {
        1.0
    };
    [[s, 0.0, -s * c[0]], [0.0, s, -s * c[1]], [0.0, 0.0, 1.0]]
}

/// The essential matrix of at least 8 pairs, `b E a = 0`, by the normalised 8-point algorithm, with
/// its two equal singular values and its zero enforced.
fn essential_of(a: &[[f64; 2]], b: &[[f64; 2]]) -> Option<Matrix> {
    let n = a.len().min(b.len());
    if n < 8 {
        return None;
    }
    let (ta, tb) = (normalizer(&a[..n]), normalizer(&b[..n]));
    let mut rows = Vec::with_capacity(n * 9);
    for i in 0..n {
        let p = apply(&ta, [a[i][0], a[i][1], 1.0]);
        let q = apply(&tb, [b[i][0], b[i][1], 1.0]);
        rows.extend_from_slice(&[
            q[0] * p[0],
            q[0] * p[1],
            q[0],
            q[1] * p[0],
            q[1] * p[1],
            q[1],
            p[0],
            p[1],
            1.0,
        ]);
    }
    let e = null_vector(&rows, n, 9);
    let e = [[e[0], e[1], e[2]], [e[3], e[4], e[5]], [e[6], e[7], e[8]]];
    let e = product(&transposed(&tb), &product(&e, &ta));
    let (u, s, v) = svd3(&e);
    let mean = (s[0] + s[1]) * 0.5;
    if mean <= 0.0 || !mean.is_finite() {
        return None;
    }
    let flat = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]];
    Some(product(&u, &product(&flat, &transposed(&v))))
}

/// How far a pair is from `b E a = 0`, squared (Sampson's distance).
fn off_essential(e: &Matrix, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (p, q) = ([a[0], a[1], 1.0], [b[0], b[1], 1.0]);
    let (ep, etq) = (apply(e, p), apply_back(e, q));
    let on = dot(q, ep);
    let across = ep[0] * ep[0] + ep[1] * ep[1] + etq[0] * etq[0] + etq[1] * etq[1];
    if across < 1e-300 {
        f64::INFINITY
    } else {
        on * on / across
    }
}

/// The essential matrix most of the pairs agree on within `tolerance`, fitted to those, and which they
/// are.
pub fn essential_robustly(
    a: &[[f64; 2]],
    b: &[[f64; 2]],
    tolerance: f64,
    tries: usize,
    seed: u64,
) -> Option<(Matrix, Vec<bool>)> {
    let n = a.len().min(b.len());
    if n < 8 {
        return None;
    }
    let limit = tolerance * tolerance;
    let mut draws = Draws::new(seed);
    let mut best: Option<(usize, f64, Matrix)> = None;
    let mut picks = Vec::new();
    let mut most_tries = tries;
    let mut tried = 0;
    let (mut from, mut to) = ([[0.0; 2]; 8], [[0.0; 2]; 8]);
    while tried < most_tries {
        tried += 1;
        draws.distinct(n, 8, &mut picks);
        if picks.len() < 8 {
            break;
        }
        for (k, pick) in picks.iter().enumerate() {
            from[k] = a[*pick];
            to[k] = b[*pick];
        }
        let Some(e) = essential_of(&from, &to) else {
            continue;
        };
        let (mut agreeing, mut off) = (0, 0.0);
        for i in 0..n {
            let d = off_essential(&e, a[i], b[i]);
            if d < limit {
                agreeing += 1;
                off += d;
            }
        }
        if best
            .as_ref()
            .is_none_or(|(most, least, _)| agreeing > *most || (agreeing == *most && off < *least))
        {
            best = Some((agreeing, off, e));
            // As few more as a 99% chance of a clean draw takes.
            let clean = (agreeing as f64 / n as f64).powi(8);
            if clean > 1e-9 {
                let need = (0.01f64.ln() / (1.0 - clean).max(1e-12).ln()).ceil();
                if need.is_finite() {
                    most_tries = most_tries.min((need as usize).max(30));
                }
            }
        }
    }
    let (_, _, mut e) = best?;
    let mut agree = vec![false; n];
    // Twice over: a better fit can bring in more pairs.
    for _ in 0..2 {
        for i in 0..n {
            agree[i] = off_essential(&e, a[i], b[i]) < limit;
        }
        let (p, q): (Vec<[f64; 2]>, Vec<[f64; 2]>) =
            (0..n).filter(|i| agree[*i]).map(|i| (a[i], b[i])).unzip();
        match essential_of(&p, &q) {
            Some(better) => e = better,
            None => break,
        }
    }
    for i in 0..n {
        agree[i] = off_essential(&e, a[i], b[i]) < limit;
    }
    Some((e, agree))
}

/// The four ways an essential matrix can come apart, as `(turn, shift)` with a shift a unit long.
pub fn apart(e: &Matrix) -> [(Matrix, Vector); 4] {
    let (mut u, _, mut v) = svd3(e);
    if determinant(&u) < 0.0 {
        for row in u.iter_mut() {
            row[2] = -row[2];
        }
    }
    if determinant(&v) < 0.0 {
        for row in v.iter_mut() {
            row[2] = -row[2];
        }
    }
    let w = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    let r1 = product(&u, &product(&w, &transposed(&v)));
    let r2 = product(&u, &product(&transposed(&w), &transposed(&v)));
    let t = column(&u, 2);
    [(r1, t), (r1, times(t, -1.0)), (r2, t), (r2, times(t, -1.0))]
}

/// Where the views' rays meet, by the linear method (Hartley & Zisserman 12.2).
pub fn triangulate(views: &[(View, [f64; 2])]) -> Option<Vector> {
    if views.len() < 2 {
        return None;
    }
    let mut sum = [0.0f64; 16];
    for (view, at) in views {
        let t = view.shift();
        let r = &view.turn;
        let rows = [
            [r[0][0], r[0][1], r[0][2], t[0]],
            [r[1][0], r[1][1], r[1][2], t[1]],
            [r[2][0], r[2][1], r[2][2], t[2]],
        ];
        for axis in 0..2 {
            let row: [f64; 4] = std::array::from_fn(|j| at[axis] * rows[2][j] - rows[axis][j]);
            let square = row
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .max(1e-300);
            for i in 0..4 {
                for j in 0..4 {
                    sum[i * 4 + j] += row[i] * row[j] / square;
                }
            }
        }
    }
    let (_, vectors) = symmetric_eigen(&sum, 4);
    let h = [vectors[0], vectors[4], vectors[8], vectors[12]];
    if h[3].abs() < 1e-12 {
        return None;
    }
    let point = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
    point.iter().all(|value| value.is_finite()).then_some(point)
}

/// The angle between the rays from two places to a point, in radians.
pub fn ray_angle(a: Vector, b: Vector, point: Vector) -> f64 {
    let (u, v) = (unit(minus(point, a)), unit(minus(point, b)));
    dot(u, v).clamp(-1.0, 1.0).acos()
}

/// How far a point lands from where a view saw it, squared, or none behind the view.
pub fn off_squared(view: &View, point: Vector, at: [f64; 2]) -> Option<f64> {
    let seen = view.seen(point);
    if seen[2] <= 1e-9 {
        return None;
    }
    let (u, v) = (seen[0] / seen[2] - at[0], seen[1] / seen[2] - at[1]);
    Some(u * u + v * v)
}

/// Of the ways an essential matrix comes apart, the one with the most points in front of both views:
/// the second view, the first being at rest.
pub fn in_front(ways: &[(Matrix, Vector)], a: &[[f64; 2]], b: &[[f64; 2]]) -> Option<View> {
    let mut best: Option<(View, usize)> = None;
    for (turn, shift) in ways {
        let view = View::from_shift(*turn, *shift);
        let mut count = 0;
        for i in 0..a.len() {
            let met = triangulate(&[(View::AT_REST, a[i]), (view, b[i])]);
            if met.is_some_and(|point| point[2] > 0.0 && view.seen(point)[2] > 0.0) {
                count += 1;
            }
        }
        if best.as_ref().is_none_or(|(_, most)| count > *most) {
            best = Some((view, count));
        }
    }
    best.map(|(view, _)| view)
}

/// A flat scene's calibrated homography `H ~ R + t nᵀ / d`, `b ~ H a`, taken apart into its turns and
/// shifts (Ma et al. 5.3.3), up to four, leaving out those with the plane behind the first view.
pub fn homography_apart(h: &Matrix, a: &[[f64; 2]], b: &[[f64; 2]]) -> Vec<(Matrix, Vector)> {
    let (_, s, _) = svd3(h);
    if s[1] <= 1e-12 {
        return Vec::new();
    }
    let mut h = scaled(h, 1.0 / s[1]);
    // Its sign puts what it maps in front.
    let ahead = a
        .iter()
        .zip(b)
        .filter(|(p, q)| dot([q[0], q[1], 1.0], apply(&h, [p[0], p[1], 1.0])) > 0.0)
        .count();
    if ahead * 2 < a.len() {
        h = scaled(&h, -1.0);
    }
    let square = product(&transposed(&h), &h);
    let flat: Vec<f64> = square.iter().flatten().copied().collect();
    let (values, vectors) = symmetric_eigen(&flat, 3);
    // Largest first: the middle singular value is 1.
    let v1 = [vectors[2], vectors[5], vectors[8]];
    let v2 = [vectors[1], vectors[4], vectors[7]];
    let v3 = [vectors[0], vectors[3], vectors[6]];
    let (s1, s3) = (values[2].max(1.0), values[0].clamp(0.0, 1.0));
    if s1 - s3 < 1e-10 {
        // A rotation alone says nothing of a shift.
        return Vec::new();
    }
    let k = (s1 - s3).sqrt();
    let (a1, a3) = ((1.0 - s3).sqrt(), (s1 - 1.0).sqrt());
    let u1 = times(add(times(v1, a1), times(v3, a3)), 1.0 / k);
    let u2 = times(minus(times(v1, a1), times(v3, a3)), 1.0 / k);
    let mut out = Vec::new();
    for u in [u1, u2] {
        let before = from_columns(v2, u, cross(v2, u));
        let (hv, hu) = (apply(&h, v2), apply(&h, u));
        let after = from_columns(hv, hu, cross(hv, hu));
        let turn = product(&after, &transposed(&before));
        let normal = cross(v2, u);
        let shift = apply(&plus_scaled(&h, &turn, -1.0), normal);
        for sign in [1.0, -1.0] {
            let (normal, shift) = (times(normal, sign), times(shift, sign));
            let front = a
                .iter()
                .filter(|p| dot(normal, [p[0], p[1], 1.0]) > 0.0)
                .count();
            if front * 2 >= a.len() {
                out.push((turn, shift));
            }
        }
    }
    out
}

/// The rotation that best takes unit rays `a` to `b` (Procrustes).
pub fn best_turn(a: &[Vector], b: &[Vector]) -> Option<Matrix> {
    if a.len() < 2 {
        return None;
    }
    let mut sum = [[0.0; 3]; 3];
    for (x, y) in a.iter().zip(b) {
        for (i, row) in sum.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                *value += y[i] * x[j];
            }
        }
    }
    Some(nearest_rotation(&sum))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> (Vec<Vector>, View) {
        let mut draws = Draws::new(12345);
        let mut part = |span: f64| draws.part() * span;
        let points = (0..60)
            .map(|_| [part(4.0), part(3.0), 5.0 + part(3.0)])
            .collect();
        (
            points,
            View {
                turn: rotation([0.02, -0.15, 0.03]),
                place: [0.6, 0.1, -0.2],
            },
        )
    }

    fn seen(view: &View, point: Vector) -> [f64; 2] {
        let c = view.seen(point);
        [c[0] / c[2], c[1] / c[2]]
    }

    #[test]
    fn the_essential_matrix_finds_the_second_view_past_what_does_not_agree() {
        let (points, view) = scene();
        let a: Vec<[f64; 2]> = points.iter().map(|p| seen(&View::AT_REST, *p)).collect();
        let mut b: Vec<[f64; 2]> = points.iter().map(|p| seen(&view, *p)).collect();
        b[3] = [0.3, -0.2];
        b[17] = [-0.1, 0.25];
        let (e, agree) = essential_robustly(&a, &b, 1e-3, 500, 7).unwrap();
        assert!(!agree[3] && !agree[17]);
        assert!(agree.iter().filter(|agrees| **agrees).count() >= 57);
        let found = in_front(&apart(&e), &a, &b).unwrap();
        let off = length(axis_of(&product(&found.turn, &transposed(&view.turn))));
        assert!(off < 1e-6, "{off}");
        assert!(length(minus(unit(found.place), unit(view.place))) < 1e-5);
    }

    #[test]
    fn a_flat_scenes_homography_comes_apart_into_the_views_turn() {
        let view = View {
            turn: rotation([0.05, -0.1, 0.02]),
            place: [0.4, 0.05, -0.1],
        };
        // The plane z = 6: H = R + t nᵀ / d, with n looking along z.
        let (normal, away) = ([0.0, 0.0, 1.0], 6.0);
        let t = view.shift();
        let h = plus_scaled(
            &view.turn,
            &[
                [t[0] * normal[0], t[0] * normal[1], t[0] * normal[2]],
                [t[1] * normal[0], t[1] * normal[1], t[1] * normal[2]],
                [t[2] * normal[0], t[2] * normal[1], t[2] * normal[2]],
            ],
            1.0 / away,
        );
        let points: Vec<Vector> = (0..12)
            .map(|i| [(i % 4) as f64 - 1.5, (i % 3) as f64 - 1.0, 6.0])
            .collect();
        let a: Vec<[f64; 2]> = points.iter().map(|p| seen(&View::AT_REST, *p)).collect();
        let b: Vec<[f64; 2]> = points.iter().map(|p| seen(&view, *p)).collect();
        let ways = homography_apart(&h, &a, &b);
        assert!(ways.iter().any(|(turn, shift)| {
            let off = length(axis_of(&product(turn, &transposed(&view.turn))));
            off < 1e-6 && length(minus(unit(*shift), unit(t))) < 1e-6
        }));
    }

    #[test]
    fn rays_turned_are_turned_back() {
        let turn = rotation([0.2, -0.4, 0.1]);
        let a: Vec<Vector> = [
            [0.1, 0.2, 1.0],
            [-0.3, 0.1, 1.0],
            [0.2, -0.4, 1.0],
            [0.0, 0.0, 1.0],
        ]
        .iter()
        .map(|ray| unit(*ray))
        .collect();
        let b: Vec<Vector> = a.iter().map(|ray| apply(&turn, *ray)).collect();
        let found = best_turn(&a, &b).unwrap();
        assert!(length(axis_of(&product(&found, &transposed(&turn)))) < 1e-9);
    }
}
