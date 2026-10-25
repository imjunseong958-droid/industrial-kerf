//! 2D geometry for Kerf's renderer and effects: vectors, rectangles, affine
//! transforms (FilmCraft's Motion math), and 3×3 projective matrices with
//! the bounds they map (EffectCraft's warps).
//!
//! y grows downwards and angles are degrees clockwise on screen.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

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
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
    pub fn translate(x: f64, y: f64) -> Self {
        Affine { e: x, f: y, ..Self::IDENTITY }
    }
    pub fn scale(sx: f64, sy: f64) -> Self {
        Affine { a: sx, d: sy, ..Self::IDENTITY }
    }
    /// Rotation by `deg` degrees, clockwise on screen (y down), matching Premiere's Rotation.
    pub fn rotate_deg(deg: f64) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
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
        Vec2::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
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
        Some(Affine { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) })
    }
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
    /// Bounding box of a transformed rect.
    pub fn bounds(&self, r: &Rect) -> Rect {
        let pts = [Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.x, r.bottom()), Vec2::new(r.right(), r.bottom())].map(|p| self.apply(p));
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
    pub const EMPTY: Bounds = Bounds { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY };
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
        Bounds { x0, y0, x1, y1 }
    }
    pub fn from_size(w: f64, h: f64) -> Bounds {
        Bounds { x0: 0.0, y0: 0.0, x1: w, y1: h }
    }
    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }
    pub fn intersect(&self, o: &Bounds) -> Bounds {
        Bounds { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
    pub fn include(&mut self, p: Vec2) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }
    pub fn inflate(&self, d: f64) -> Bounds {
        Bounds { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }
    pub fn corners(&self) -> [Vec2; 4] {
        [vec2(self.x0, self.y0), vec2(self.x1, self.y0), vec2(self.x1, self.y1), vec2(self.x0, self.y1)]
    }
    /// Smallest integer rect covering this one.
    pub fn round_out(&self) -> Bounds {
        Bounds { x0: self.x0.floor(), y0: self.y0.floor(), x1: self.x1.ceil(), y1: self.y1.ceil() }
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
    pub fn skew_deg(skew: f64, axis_deg: f64) -> Mat3 {
        let k = skew.to_radians().tan();
        Mat3::rotate_deg(axis_deg) * Mat3([[1.0, k, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]) * Mat3::rotate_deg(-axis_deg)
    }
    /// The After Effects 2D layer transform: layer space → parent space.
    /// `p' = position + R(rotation) · S(scale/100) · (p - anchor)`.
    pub fn layer_2d(anchor: Vec2, position: Vec2, scale_pct: Vec2, rotation_deg: f64) -> Mat3 {
        Mat3::translate(position) * Mat3::rotate_deg(rotation_deg) * Mat3::scale(scale_pct / 100.0) * Mat3::translate(-anchor)
    }
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let m = &self.0;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w != 1.0 && w != 0.0 { vec2(x / w, y / w) } else { vec2(x, y) }
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
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
    pub fn inverse(&self) -> Option<Mat3> {
        let d = self.determinant();
        if d.abs() < 1e-300 || !d.is_finite() {
            return None;
        }
        let m = &self.0;
        let inv = [
            [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d],
            [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d],
            [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d],
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
        let (x0, y0, x1, y1, x2, y2, x3, y3) = (q[0].x, q[0].y, q[1].x, q[1].y, q[2].x, q[2].y, q[3].x, q[3].y);
        let dx1 = x1 - x2;
        let dx2 = x3 - x2;
        let dy1 = y1 - y2;
        let dy2 = y3 - y2;
        let sx = x0 - x1 + x2 - x3;
        let sy = y0 - y1 + y2 - y3;
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            return Mat3([[x1 - x0, x3 - x0, x0], [y1 - y0, y3 - y0, y0], [0.0, 0.0, 1.0]]);
        }
        let den = dx1 * dy2 - dx2 * dy1;
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Mat3([[x1 - x0 + g * x1, x3 - x0 + h * x3, x0], [y1 - y0 + g * y1, y3 - y0 + h * y3, y0], [g, h, 1.0]])
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
        let m = Affine::motion(Vec2::new(960.0, 540.0), Vec2::new(1.5, 0.75), 33.0, Vec2::new(100.0, 50.0));
        let inv = m.inverse().unwrap();
        let p = Vec2::new(12.0, -7.0);
        let q = inv.apply(m.apply(p));
        assert!((p - q).length() < 1e-9);
    }

    #[test]
    fn motion_anchor_lands_on_position() {
        let m = Affine::motion(Vec2::new(960.0, 540.0), Vec2::new(2.0, 2.0), 90.0, Vec2::new(10.0, 20.0));
        let p = m.apply(Vec2::new(10.0, 20.0));
        assert!((p - Vec2::new(960.0, 540.0)).length() < 1e-9);
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn layer_2d_maps_anchor_to_position() {
        let m = Mat3::layer_2d(vec2(50.0, 25.0), vec2(960.0, 540.0), vec2(200.0, 50.0), 30.0);
        assert!(close(m.apply(vec2(50.0, 25.0)), vec2(960.0, 540.0)));
        let inv = m.inverse().unwrap();
        let p = vec2(12.0, -7.0);
        assert!(close(inv.apply(m.apply(p)), p));
    }

    #[test]
    fn rotation_is_clockwise_on_screen() {
        let p = Mat3::rotate_deg(90.0).apply(vec2(1.0, 0.0));
        assert!(close(p, vec2(0.0, 1.0)));
    }

    #[test]
    fn quad_mapping_hits_corners() {
        let q = [vec2(10.0, 10.0), vec2(110.0, 20.0), vec2(120.0, 140.0), vec2(5.0, 100.0)];
        let m = Mat3::square_to_quad(q);
        assert!(close(m.apply(vec2(0.0, 0.0)), q[0]));
        assert!(close(m.apply(vec2(1.0, 0.0)), q[1]));
        assert!(close(m.apply(vec2(1.0, 1.0)), q[2]));
        assert!(close(m.apply(vec2(0.0, 1.0)), q[3]));
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
}
