//! Small dense linear algebra for solving a camera, from the textbook methods: rotations by
//! Rodrigues' formula and its inverse, the one-sided Jacobi SVD (Golub & Van Loan, *Matrix
//! Computations*, 8.6.4), cyclic Jacobi for symmetric matrices (8.5), and Cholesky for the normal
//! equations of [`super::adjust`]. EffectCraft's `track/src/camtrack/linalg.rs`, reduced to what a
//! solve takes.

pub type Vector = [f64; 3];
pub type Matrix = [[f64; 3]; 3];

pub const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn minus(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn times(a: Vector, by: f64) -> Vector {
    [a[0] * by, a[1] * by, a[2] * by]
}

pub fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn length(a: Vector) -> f64 {
    dot(a, a).sqrt()
}

pub fn unit(a: Vector) -> Vector {
    let long = length(a);
    if long > 0.0 {
        times(a, 1.0 / long)
    } else {
        a
    }
}

/// `m v`.
pub fn apply(m: &Matrix, v: Vector) -> Vector {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// `mᵀ v`.
pub fn apply_back(m: &Matrix, v: Vector) -> Vector {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

/// `a b`.
pub fn product(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

pub fn transposed(a: &Matrix) -> Matrix {
    [
        [a[0][0], a[1][0], a[2][0]],
        [a[0][1], a[1][1], a[2][1]],
        [a[0][2], a[1][2], a[2][2]],
    ]
}

/// `a + b by`.
pub fn plus_scaled(a: &Matrix, b: &Matrix, by: f64) -> Matrix {
    let mut out = *a;
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] += by * b[i][j];
        }
    }
    out
}

pub fn scaled(a: &Matrix, by: f64) -> Matrix {
    plus_scaled(&[[0.0; 3]; 3], a, by)
}

pub fn determinant(m: &Matrix) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

pub fn skew(v: Vector) -> Matrix {
    [[0.0, -v[2], v[1]], [v[2], 0.0, -v[0]], [-v[1], v[0], 0.0]]
}

pub fn column(m: &Matrix, j: usize) -> Vector {
    [m[0][j], m[1][j], m[2][j]]
}

pub fn from_columns(a: Vector, b: Vector, c: Vector) -> Matrix {
    [[a[0], b[0], c[0]], [a[1], b[1], c[1]], [a[2], b[2], c[2]]]
}

pub fn inverse(m: &Matrix) -> Option<Matrix> {
    let det = determinant(m);
    if det.abs() < 1e-300 {
        return None;
    }
    let minor =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [minor(1, 1, 2, 2), -minor(0, 1, 2, 2), minor(0, 1, 1, 2)],
        [-minor(1, 0, 2, 2), minor(0, 0, 2, 2), -minor(0, 0, 1, 2)],
        [minor(1, 0, 2, 1), -minor(0, 0, 2, 1), minor(0, 0, 1, 1)],
    ];
    Some(scaled(&adjugate, 1.0 / det))
}

/// The rotation of an axis scaled by its angle (Rodrigues' formula).
pub fn rotation(axis: Vector) -> Matrix {
    let angle = length(axis);
    let k = skew(axis);
    if angle < 1e-12 {
        return plus_scaled(&IDENTITY, &k, 1.0);
    }
    let a = angle.sin() / angle;
    let b = (1.0 - angle.cos()) / (angle * angle);
    plus_scaled(&plus_scaled(&IDENTITY, &k, a), &product(&k, &k), b)
}

/// The axis scaled by its angle of a rotation, the inverse of [`rotation`].
pub fn axis_of(r: &Matrix) -> Vector {
    let cosine = ((r[0][0] + r[1][1] + r[2][2] - 1.0) * 0.5).clamp(-1.0, 1.0);
    let angle = cosine.acos();
    let v = [r[2][1] - r[1][2], r[0][2] - r[2][0], r[1][0] - r[0][1]];
    if angle < 1e-9 {
        return times(v, 0.5);
    }
    if std::f64::consts::PI - angle < 1e-6 {
        // Near half a turn the axis is the largest column of R + I.
        let b = plus_scaled(r, &IDENTITY, 1.0);
        let j = (0..3)
            .max_by(|a, c| b[*a][*a].total_cmp(&b[*c][*c]))
            .unwrap_or(0);
        return times(unit(column(&b, j)), angle);
    }
    times(v, angle / (2.0 * angle.sin()))
}

/// The rotation nearest `m`.
pub fn nearest_rotation(m: &Matrix) -> Matrix {
    let (mut u, _, v) = svd3(m);
    if determinant(&product(&u, &transposed(&v))) < 0.0 {
        for row in u.iter_mut() {
            row[2] = -row[2];
        }
    }
    product(&u, &transposed(&v))
}

/// A rotation part way from `a` to `b`.
pub fn between(a: &Matrix, b: &Matrix, t: f64) -> Matrix {
    let turn = axis_of(&product(b, &transposed(a)));
    product(&rotation(times(turn, t)), a)
}

/// The SVD `A = U diag(s) Vᵀ` of an `m` by `n` matrix in rows (one-sided Jacobi): `U` has
/// `max(m, n)` rows and `n` columns, `s` comes largest first, and `V` is `n` by `n`.
pub fn svd(a: &[f64], m: usize, n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let rows = m.max(n);
    let mut u = vec![0.0; rows * n];
    u[..m * n].copy_from_slice(&a[..m * n]);
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..60 {
        let mut off = 0.0f64;
        for p in 0..n {
            for q in p + 1..n {
                let (mut alpha, mut beta, mut gamma) = (0.0, 0.0, 0.0);
                for r in 0..rows {
                    let (x, y) = (u[r * n + p], u[r * n + q]);
                    alpha += x * x;
                    beta += y * y;
                    gamma += x * y;
                }
                if gamma == 0.0 {
                    continue;
                }
                let tilt = f64::abs(gamma) / f64::max(f64::sqrt(alpha * beta), 1e-300);
                off = off.max(tilt);
                if tilt < 1e-15 {
                    continue;
                }
                let zeta: f64 = (beta - alpha) / (2.0 * gamma);
                let t = if zeta == 0.0 {
                    1.0
                } else {
                    zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt())
                };
                let c = 1.0 / (1.0 + t * t).sqrt();
                let s = c * t;
                for r in 0..rows {
                    let (x, y) = (u[r * n + p], u[r * n + q]);
                    u[r * n + p] = c * x - s * y;
                    u[r * n + q] = s * x + c * y;
                }
                for r in 0..n {
                    let (x, y) = (v[r * n + p], v[r * n + q]);
                    v[r * n + p] = c * x - s * y;
                    v[r * n + q] = s * x + c * y;
                }
            }
        }
        if off < 1e-15 {
            break;
        }
    }
    let values: Vec<f64> = (0..n)
        .map(|j| {
            (0..rows)
                .map(|r| u[r * n + j] * u[r * n + j])
                .sum::<f64>()
                .sqrt()
        })
        .collect();
    for (j, value) in values.iter().enumerate() {
        if *value > 1e-300 {
            for r in 0..rows {
                u[r * n + j] /= value;
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
    let (mut sorted_u, mut sorted_v) = (vec![0.0; rows * n], vec![0.0; n * n]);
    for (k, j) in order.iter().enumerate() {
        for r in 0..rows {
            sorted_u[r * n + k] = u[r * n + j];
        }
        for r in 0..n {
            sorted_v[r * n + k] = v[r * n + j];
        }
    }
    let sorted = order.iter().map(|j| values[*j]).collect();
    (sorted_u, sorted, sorted_v)
}

/// The SVD of a 3 by 3 matrix: `U`, its singular values largest first, and `V`.
pub fn svd3(m: &Matrix) -> (Matrix, Vector, Matrix) {
    let flat: Vec<f64> = m.iter().flatten().copied().collect();
    let (u, s, v) = svd(&flat, 3, 3);
    let grid = |x: &[f64]| [[x[0], x[1], x[2]], [x[3], x[4], x[5]], [x[6], x[7], x[8]]];
    let mut left = grid(&u);
    // The column of a zero singular value is undefined: complete the basis.
    if s[2] < 1e-300 {
        let last = cross(column(&left, 0), column(&left, 1));
        for (r, row) in left.iter_mut().enumerate() {
            row[2] = last[r];
        }
    }
    (left, [s[0], s[1], s[2]], grid(&v))
}

/// The unit vector `x` that makes `A x`, of an `m` by `n` matrix in rows, smallest.
pub fn null_vector(a: &[f64], m: usize, n: usize) -> Vec<f64> {
    let (_, _, v) = svd(a, m, n);
    (0..n).map(|r| v[r * n + n - 1]).collect()
}

/// The eigenvalues of a symmetric `n` by `n` matrix, smallest first, and their eigenvectors as the
/// columns of a matrix in rows (cyclic Jacobi).
pub fn symmetric_eigen(a: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = a.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..100 {
        let mut off = 0.0;
        for p in 0..n {
            for q in p + 1..n {
                off += a[p * n + q] * a[p * n + q];
            }
        }
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta: f64 = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = if theta == 0.0 {
                    1.0
                } else {
                    theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
                };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|x, y| a[*x * n + *x].total_cmp(&a[*y * n + *y]));
    let values = order.iter().map(|j| a[*j * n + *j]).collect();
    let mut vectors = vec![0.0; n * n];
    for (k, j) in order.iter().enumerate() {
        for r in 0..n {
            vectors[r * n + k] = v[r * n + j];
        }
    }
    (values, vectors)
}

/// Dot product in four lanes, which the compiler can vectorise.
fn lanes_dot(a: &[f64], b: &[f64]) -> f64 {
    let mut lanes = [0.0f64; 4];
    let (a4, b4) = (a.chunks_exact(4), b.chunks_exact(4));
    let (rest_a, rest_b) = (a4.remainder(), b4.remainder());
    for (x, y) in a4.zip(b4) {
        for k in 0..4 {
            lanes[k] += x[k] * y[k];
        }
    }
    let mut sum = lanes[0] + lanes[1] + lanes[2] + lanes[3];
    for (x, y) in rest_a.iter().zip(rest_b) {
        sum += x * y;
    }
    sum
}

/// Solve `A x = b` in place for a symmetric positive definite `n` by `n` matrix in rows (Cholesky).
/// False when it is not positive definite.
pub fn solve_positive(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d <= 0.0 || !d.is_finite() {
            return false;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        let (head, tail) = a.split_at_mut((j + 1) * n);
        let row_j = &head[j * n..j * n + j];
        for row in tail.chunks_mut(n) {
            row[j] = (row[j] - lanes_dot(&row[..j], row_j)) / d;
        }
    }
    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= a[i * n + k] * b[k];
        }
        b[i] = sum / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut sum = b[i];
        for k in i + 1..n {
            sum -= a[k * n + i] * b[k];
        }
        b[i] = sum / a[i * n + i];
    }
    b.iter().all(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svd_gives_back_what_it_took() {
        let a = [3.0, 1.0, 2.0, -1.0, 4.0, 0.5, 2.0, 2.0, 7.0, 0.1, -3.0, 1.0];
        let (u, s, v) = svd(&a, 4, 3);
        for r in 0..4 {
            for c in 0..3 {
                let x: f64 = (0..3).map(|k| u[r * 3 + k] * s[k] * v[c * 3 + k]).sum();
                assert!((x - a[r * 3 + c]).abs() < 1e-10);
            }
        }
        assert!(s[0] >= s[1] && s[1] >= s[2]);
    }

    #[test]
    fn a_rotation_gives_back_its_axis() {
        let axis = [0.3, -1.2, 0.7];
        let r = rotation(axis);
        assert!(length(minus(axis, axis_of(&r))) < 1e-12);
        assert!((determinant(&r) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn eigenvectors_and_cholesky_solve_what_they_were_given() {
        let a = [4.0, 1.0, 0.5, 1.0, 3.0, 0.2, 0.5, 0.2, 2.0];
        let (values, vectors) = symmetric_eigen(&a, 3);
        for k in 0..3 {
            let v = [vectors[k], vectors[3 + k], vectors[6 + k]];
            let image = [
                dot([a[0], a[1], a[2]], v),
                dot([a[3], a[4], a[5]], v),
                dot([a[6], a[7], a[8]], v),
            ];
            assert!(length(minus(image, times(v, values[k]))) < 1e-10);
        }
        let mut m = a;
        let mut b = [1.0, 2.0, 3.0];
        assert!(solve_positive(&mut m, 3, &mut b));
        let back = [
            dot([a[0], a[1], a[2]], b),
            dot([a[3], a[4], a[5]], b),
            dot([a[6], a[7], a[8]], b),
        ];
        assert!(length(minus(back, [1.0, 2.0, 3.0])) < 1e-12);
    }
}
