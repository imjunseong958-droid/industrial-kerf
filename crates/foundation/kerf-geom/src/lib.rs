//! 2D geometry for Kerf's renderer and effects: vectors, rectangles, affine
//! transforms (FilmCraft's Motion math), and 3×3 projective matrices with
//! the bounds they map (EffectCraft's warps). And the affine transform of
//! space that places what is in depth, of which the 2D one is the flat case,
//! with where a clip's picture lands by its numbers ([`place`]).
//!
//! y grows downwards and angles are degrees clockwise on screen.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unimplemented,
        clippy::todo,
        clippy::unreachable
    )
)]

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

pub mod lie;
pub mod place;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }
}
pub const fn vec2(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl Div<f64> for Vec2 {
    type Output = Vec2;
    fn div(self, s: f64) -> Vec2 {
        Vec2::new(self.x / s, self.y / s)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// A rectangle by its origin and size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

/// Affine transform `[a c e; b d f; 0 0 1]` mapping (x, y) → (a x + c y + e, b x + d y + f).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    pub fn translate(x: f64, y: f64) -> Self {
        Affine {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }
    pub fn scale(sx: f64, sy: f64) -> Self {
        Affine {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }
    /// Rotation by `deg` degrees, clockwise on screen (y down), matching Premiere's Rotation.
    pub fn rotate_deg(deg: f64) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Affine {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }
    /// `self * o`: apply `o` first, then `self`.
    pub fn then_apply(&self, o: &Affine) -> Affine {
        // result = self ∘ o
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    pub fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }
    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let a = self.d * inv;
        let b = -self.b * inv;
        let c = -self.c * inv;
        let d = self.a * inv;
        Some(Affine {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
    /// Bounding box of a transformed rect.
    pub fn bounds(&self, r: &Rect) -> Rect {
        let pts = [
            Vec2::new(r.x, r.y),
            Vec2::new(r.right(), r.y),
            Vec2::new(r.x, r.bottom()),
            Vec2::new(r.right(), r.bottom()),
        ]
        .map(|p| self.apply(p));
        let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Premiere's Motion effect: the source's anchor point is placed at `position` (sequence pixels),
    /// scaled by `scale` (1.0 = 100%) around the anchor and rotated by `rotation_deg`.
    pub fn motion(position: Vec2, scale: Vec2, rotation_deg: f64, anchor: Vec2) -> Affine {
        Affine::translate(position.x, position.y)
            .then_apply(&Affine::rotate_deg(rotation_deg))
            .then_apply(&Affine::scale(scale.x, scale.y))
            .then_apply(&Affine::translate(-anchor.x, -anchor.y))
    }
}

/// Affine transform of space: where the x, y, and z axes go and where the origin goes, so that
/// `(px, py, pz)` → `x·px + y·py + z·pz + t`. x runs right, y down, and z away from the viewer.
///
/// What an [`Affine`] is on the frame this is in depth, and an [`Affine`] is the one of these
/// that leaves z alone ([`Affine3::from`], [`Affine3::flat`]). A point and a way are `[f64; 3]`,
/// as the compositor's clips in depth have them.
///
/// The order of the sums in each of these is part of what it is, and is written by it: frames
/// are drawn from them, and a sum taken in another order draws another picture in its last bits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine3 {
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
    pub t: [f64; 3],
}

impl Default for Affine3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// On the frame, at z = 0 and keeping z.
impl From<Affine> for Affine3 {
    fn from(m: Affine) -> Self {
        Affine3 {
            x: [m.a, m.b, 0.0],
            y: [m.c, m.d, 0.0],
            z: [0.0, 0.0, 1.0],
            t: [m.e, m.f, 0.0],
        }
    }
}

impl Affine3 {
    pub const IDENTITY: Affine3 = Affine3 {
        x: [1.0, 0.0, 0.0],
        y: [0.0, 1.0, 0.0],
        z: [0.0, 0.0, 1.0],
        t: [0.0; 3],
    };
    /// What it is on the frame, when it leaves z alone: none for one that tips the frame or
    /// moves it back.
    pub fn flat(&self) -> Option<Affine> {
        let keeps_z = self.x[2] == 0.0 && self.y[2] == 0.0 && self.t[2] == 0.0;
        (keeps_z && self.z == [0.0, 0.0, 1.0]).then_some(Affine {
            a: self.x[0],
            b: self.x[1],
            c: self.y[0],
            d: self.y[1],
            e: self.t[0],
            f: self.t[1],
        })
    }
    /// Where a way goes, which has no place: `(x·v.x + y·v.y) + z·v.z`.
    pub fn apply_vec(&self, v: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|k| (self.x[k] * v[0] + self.y[k] * v[1]) + self.z[k] * v[2])
    }
    /// Where a point goes: [`Self::apply_vec`] of it, and then `+ t`.
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let v = self.apply_vec(p);
        [v[0] + self.t[0], v[1] + self.t[1], v[2] + self.t[2]]
    }
    /// `self * o`: apply `o` first, then `self`. Each axis of `o` as [`Self::apply_vec`] takes
    /// it, and its origin as [`Self::apply`] does.
    pub fn then_apply(&self, o: &Affine3) -> Affine3 {
        Affine3 {
            x: self.apply_vec(o.x),
            y: self.apply_vec(o.y),
            z: self.apply_vec(o.z),
            t: self.apply(o.t),
        }
    }
    /// This, and then moved by `by`: `t + by`.
    pub fn moved(self, by: [f64; 3]) -> Affine3 {
        Affine3 {
            t: [self.t[0] + by[0], self.t[1] + by[1], self.t[2] + by[2]],
            ..self
        }
    }
    /// This, and then turned about the x axis by `deg` degrees, from y toward z: what is below
    /// the axis goes away from the viewer. `(y, z)` → `(y·cos − z·sin, y·sin + z·cos)`.
    pub fn turned_x_deg(self, deg: f64) -> Affine3 {
        let (s, c) = deg.to_radians().sin_cos();
        let turn = |[x, y, z]: [f64; 3]| [x, y * c - z * s, y * s + z * c];
        Affine3 {
            x: turn(self.x),
            y: turn(self.y),
            z: turn(self.z),
            t: turn(self.t),
        }
    }
    /// This, and then turned about the y axis by `deg` degrees, from z toward x: what is to the
    /// right of the axis comes toward the viewer. `(x, z)` → `(x·cos + z·sin, −x·sin + z·cos)`.
    pub fn turned_y_deg(self, deg: f64) -> Affine3 {
        let (s, c) = deg.to_radians().sin_cos();
        let turn = |[x, y, z]: [f64; 3]| [x * c + z * s, y, -x * s + z * c];
        Affine3 {
            x: turn(self.x),
            y: turn(self.y),
            z: turn(self.z),
            t: turn(self.t),
        }
    }
}

/// An axis-aligned extent by its corners (min inclusive, max exclusive for pixel
/// rects), as projective warps bound their output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        x0: f64::INFINITY,
        y0: f64::INFINITY,
        x1: f64::NEG_INFINITY,
        y1: f64::NEG_INFINITY,
    };
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
        Bounds { x0, y0, x1, y1 }
    }
    pub fn from_size(w: f64, h: f64) -> Bounds {
        Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: w,
            y1: h,
        }
    }
    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }
    pub fn intersect(&self, o: &Bounds) -> Bounds {
        Bounds {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }
    pub fn include(&mut self, p: Vec2) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }
    pub fn inflate(&self, d: f64) -> Bounds {
        Bounds {
            x0: self.x0 - d,
            y0: self.y0 - d,
            x1: self.x1 + d,
            y1: self.y1 + d,
        }
    }
    pub fn corners(&self) -> [Vec2; 4] {
        [
            vec2(self.x0, self.y0),
            vec2(self.x1, self.y0),
            vec2(self.x1, self.y1),
            vec2(self.x0, self.y1),
        ]
    }
    /// Smallest integer rect covering this one.
    pub fn round_out(&self) -> Bounds {
        Bounds {
            x0: self.x0.floor(),
            y0: self.y0.floor(),
            x1: self.x1.ceil(),
            y1: self.y1.ceil(),
        }
    }
}

/// Row-major 3×3 matrix acting on column vectors `(x, y, 1)`; a 2D projective transform.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Default for Mat3 {
    fn default() -> Self {
        Mat3::IDENTITY
    }
}

/// The projective one that keeps lines parallel.
impl From<Affine> for Mat3 {
    fn from(m: Affine) -> Self {
        Mat3([[m.a, m.c, m.e], [m.b, m.d, m.f], [0.0, 0.0, 1.0]])
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    pub fn translate(t: Vec2) -> Mat3 {
        Mat3([[1.0, 0.0, t.x], [0.0, 1.0, t.y], [0.0, 0.0, 1.0]])
    }
    pub fn scale(s: Vec2) -> Mat3 {
        Mat3([[s.x, 0.0, 0.0], [0.0, s.y, 0.0], [0.0, 0.0, 1.0]])
    }
    /// Clockwise on screen (y down) for positive degrees.
    pub fn rotate_deg(deg: f64) -> Mat3 {
        let (s, c) = deg.to_radians().sin_cos();
        Mat3([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }
    #[cfg(test)]
    /// The After Effects 2D layer transform: layer space → parent space.
    /// `p' = position + R(rotation) · S(scale/100) · (p - anchor)`.
    pub fn layer_2d(anchor: Vec2, position: Vec2, scale_pct: Vec2, rotation_deg: f64) -> Mat3 {
        Mat3::translate(position)
            * Mat3::rotate_deg(rotation_deg)
            * Mat3::scale(scale_pct / 100.0)
            * Mat3::translate(-anchor)
    }
    /// Where `p` goes before it is divided: across, down, and what the two are divided by.
    #[inline]
    pub fn undivided(&self, p: Vec2) -> [f64; 3] {
        let m = &self.0;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        [x, y, w]
    }
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let [x, y, w] = self.undivided(p);
        if w != 1.0 && w != 0.0 {
            vec2(x / w, y / w)
        } else {
            vec2(x, y)
        }
    }
    /// Apply to a direction (ignores translation; affine only).
    pub fn apply_vec(&self, v: Vec2) -> Vec2 {
        let m = &self.0;
        vec2(m[0][0] * v.x + m[0][1] * v.y, m[1][0] * v.x + m[1][1] * v.y)
    }
    pub fn is_affine(&self) -> bool {
        self.0[2][0] == 0.0 && self.0[2][1] == 0.0 && (self.0[2][2] - 1.0).abs() < 1e-12
    }
    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
    pub fn inverse(&self) -> Option<Mat3> {
        let d = self.determinant();
        if d.abs() < 1e-300 || !d.is_finite() {
            return None;
        }
        let m = &self.0;
        let inv = [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d,
            ],
        ];
        Some(Mat3(inv))
    }
    /// Bounding box of a rect after transformation.
    pub fn map_rect(&self, r: &Bounds) -> Bounds {
        let mut out = Bounds::EMPTY;
        for c in r.corners() {
            out.include(self.apply(c));
        }
        out
    }
    /// Average linear scale factor (for stroke widths, blur radii in transformed space).
    pub fn mean_scale(&self) -> f64 {
        self.determinant().abs().sqrt()
    }
    /// Projective transform mapping the unit square corners to `q` (in order (0,0),(1,0),(1,1),(0,1)).
    pub fn square_to_quad(q: [Vec2; 4]) -> Mat3 {
        let (x0, y0, x1, y1, x2, y2, x3, y3) = (
            q[0].x, q[0].y, q[1].x, q[1].y, q[2].x, q[2].y, q[3].x, q[3].y,
        );
        let dx1 = x1 - x2;
        let dx2 = x3 - x2;
        let dy1 = y1 - y2;
        let dy2 = y3 - y2;
        let sx = x0 - x1 + x2 - x3;
        let sy = y0 - y1 + y2 - y3;
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            return Mat3([
                [x1 - x0, x3 - x0, x0],
                [y1 - y0, y3 - y0, y0],
                [0.0, 0.0, 1.0],
            ]);
        }
        let den = dx1 * dy2 - dx2 * dy1;
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Mat3([
            [x1 - x0 + g * x1, x3 - x0 + h * x3, x0],
            [y1 - y0 + g * y1, y3 - y0 + h * y3, y0],
            [g, h, 1.0],
        ])
    }
}

impl Mul for Mat3 {
    type Output = Mat3;
    fn mul(self, o: Mat3) -> Mat3 {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] = (0..3).map(|k| self.0[i][k] * o.0[k][j]).sum();
            }
        }
        Mat3(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrip() {
        let m = Affine::motion(
            Vec2::new(960.0, 540.0),
            Vec2::new(1.5, 0.75),
            33.0,
            Vec2::new(100.0, 50.0),
        );
        let inv = m.inverse().unwrap();
        let p = Vec2::new(12.0, -7.0);
        let q = inv.apply(m.apply(p));
        assert!((p - q).length() < 1e-9);
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn rotation_is_clockwise_on_screen() {
        let p = Mat3::rotate_deg(90.0).apply(vec2(1.0, 0.0));
        assert!(close(p, vec2(0.0, 1.0)));
    }

    proptest::proptest! {
        #[test]
        fn mat3_inverse(ax in -100.0f64..100.0, ay in -100.0f64..100.0, s in 10.0f64..400.0, r in -720.0f64..720.0) {
            let m = Mat3::layer_2d(vec2(ax, ay), vec2(3.0, 4.0), vec2(s, s * 0.7), r);
            let i = m.inverse().unwrap();
            let p = vec2(17.0, 23.0);
            proptest::prop_assert!((i.apply(m.apply(p)) - p).length() < 1e-6);
        }
    }

    fn close3(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9 * (1.0 + a[k].abs()))
    }

    #[test]
    fn turns_in_depth_go_from_y_to_z_and_from_z_to_x() {
        let about_x = Affine3::IDENTITY.turned_x_deg(90.0);
        assert!(close3(about_x.apply_vec([0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        let about_y = Affine3::IDENTITY.turned_y_deg(90.0);
        assert!(close3(about_y.apply_vec([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]));
    }

    /// A placing on the frame, as a clip's geometry makes one.
    fn on_frame() -> impl proptest::strategy::Strategy<Value = Affine> {
        use proptest::strategy::Strategy;
        (
            (-2000.0f64..2000.0, -2000.0f64..2000.0),
            (-4.0f64..4.0, -4.0f64..4.0),
            -720.0f64..720.0,
            (-500.0f64..500.0, -500.0f64..500.0),
        )
            .prop_map(|(position, scale, rotation, anchor)| {
                Affine::motion(
                    vec2(position.0, position.1),
                    vec2(scale.0, scale.1),
                    rotation,
                    vec2(anchor.0, anchor.1),
                )
            })
    }

    /// A placing in depth: one on the frame, turned about x and y and moved.
    fn in_depth() -> impl proptest::strategy::Strategy<Value = Affine3> {
        use proptest::strategy::Strategy;
        (
            on_frame(),
            -360.0f64..360.0,
            -360.0f64..360.0,
            proptest::array::uniform3(-1000.0f64..1000.0),
        )
            .prop_map(|(flat, pitch, yaw, by)| {
                Affine3::from(flat)
                    .turned_x_deg(pitch)
                    .turned_y_deg(yaw)
                    .moved(by)
            })
    }

    proptest::proptest! {
        /// What is on the frame is in depth the case that keeps z: it goes there and back as
        /// it was, and there it places a point, and follows another, to the same numbers.
        #[test]
        fn a_flat_placing_is_the_one_in_depth_that_keeps_z(
            a in on_frame(),
            b in on_frame(),
            p in proptest::array::uniform3(-3000.0f64..3000.0),
        ) {
            proptest::prop_assert_eq!(Affine3::from(a).flat(), Some(a));
            let on = a.apply(vec2(p[0], p[1]));
            proptest::prop_assert_eq!(Affine3::from(a).apply(p), [on.x, on.y, p[2]]);
            let both = Affine3::from(a).then_apply(&Affine3::from(b));
            proptest::prop_assert_eq!(both.flat(), Some(a.then_apply(&b)));
        }

        /// And one that does not keep z is none on the frame.
        #[test]
        fn a_placing_off_the_frame_is_not_flat(
            a in on_frame(),
            back in 0.001f64..1000.0,
            turn in 0.1f64..179.9,
        ) {
            proptest::prop_assert_eq!(Affine3::from(a).moved([0.0, 0.0, back]).flat(), None);
            proptest::prop_assert_eq!(Affine3::from(a).turned_x_deg(turn).flat(), None);
            proptest::prop_assert_eq!(Affine3::from(a).turned_y_deg(turn).flat(), None);
        }

        #[test]
        fn one_placing_after_another_is_their_product(
            a in in_depth(),
            b in in_depth(),
            p in proptest::array::uniform3(-100.0f64..100.0),
        ) {
            let both = a.then_apply(&b);
            let (one, other) = (both.apply(p), a.apply(b.apply(p)));
            proptest::prop_assert!((0..3).all(|k| (one[k] - other[k]).abs() < 1e-6), "{one:?} {other:?}");
            let (one, other) = (both.apply_vec(p), a.apply_vec(b.apply_vec(p)));
            proptest::prop_assert!((0..3).all(|k| (one[k] - other[k]).abs() < 1e-6), "{one:?} {other:?}");
        }

        /// A turn keeps its axis and every length, a point it is made about stays where it
        /// was, and turning back undoes it.
        #[test]
        fn a_turn_about_a_point_is_rigid_and_leaves_the_point(
            about in proptest::array::uniform3(-1000.0f64..1000.0),
            p in proptest::array::uniform3(-1000.0f64..1000.0),
            pitch in -360.0f64..360.0,
            yaw in -360.0f64..360.0,
        ) {
            let away = [-about[0], -about[1], -about[2]];
            let turn = Affine3::IDENTITY.moved(away).turned_x_deg(pitch).turned_y_deg(yaw).moved(about);
            proptest::prop_assert!(close3(turn.apply(about), about));
            let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let from = [p[0] - about[0], p[1] - about[1], p[2] - about[2]];
            let to = turn.apply(p);
            let to = [to[0] - about[0], to[1] - about[1], to[2] - about[2]];
            proptest::prop_assert!((length(from) - length(to)).abs() < 1e-6);
            proptest::prop_assert_eq!(Affine3::IDENTITY.turned_x_deg(pitch).apply(p)[0], p[0]);
            proptest::prop_assert_eq!(Affine3::IDENTITY.turned_y_deg(yaw).apply(p)[1], p[1]);
            let back = turn.moved(away).turned_y_deg(-yaw).turned_x_deg(-pitch).moved(about);
            let home = back.apply(p);
            proptest::prop_assert!((0..3).all(|k| (home[k] - p[k]).abs() < 1e-6), "{home:?} {p:?}");
        }
    }
}
