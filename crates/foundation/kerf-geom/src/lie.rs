//! Unified 3D rigid-body kinematics on the Lie group $SE(3)$ and its Lie algebra $\mathfrak{se}(3)$.
//!
//! Kerf's world coordinates are right-handed: `x` runs right, `y` runs down, and `z` runs away
//! from the viewer (`x × y = z`). This module provides:
//! - [`Quat`]: unit quaternions on $SU(2) \to SO(3)$ with $\exp/\log$ and shortest-arc `slerp`.
//! - [`Twist`]: 6-DOF spatial/body velocity $\xi = (\omega, v) \in \mathfrak{se}(3)$.
//! - [`Se3`]: rigid poses with closed-form Rodrigues $\exp/\log$, geodesic interpolation, Adjoint
//!   action $\mathrm{Ad}_T$, lossless bridges to [`Affine3`], [`Camera`], and [`Lens`], and
//!   analytical screen-space optical flow ([`screen_velocity`]).
//! - [`Ray3`] and [`Plane3`]: exact ray-plane and plane-plane geometric primitives.

use crate::place::{Camera, Lens};
use crate::Affine3;
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

type Vector = [f64; 3];

#[inline]
fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: Vector, s: f64) -> Vector {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn norm(a: Vector) -> f64 {
    dot(a, a).sqrt()
}

/// Unit quaternion `[w, x, y, z]` representing a 3D rotation in $SO(3)$.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(w: f64, x: f64, y: f64, z: f64) -> Self {
        Self { w, x, y, z }
    }

    /// Normalize to unit length, returning [`Self::IDENTITY`] for degenerate zero quaternions.
    pub fn normalized(self) -> Self {
        let len = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len <= 1e-15 || !len.is_finite() {
            return Self::IDENTITY;
        }
        let inv = 1.0 / len;
        Self {
            w: self.w * inv,
            x: self.x * inv,
            y: self.y * inv,
            z: self.z * inv,
        }
    }

    /// Conjugate (equals the inverse for unit quaternions).
    #[inline]
    pub const fn conjugate(self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Inverse rotation.
    #[inline]
    pub fn inverse(self) -> Self {
        self.normalized().conjugate()
    }

    /// Rotation by `angle_rad` radians around `axis` (right-handed).
    pub fn from_axis_angle(axis: Vector, angle_rad: f64) -> Self {
        let len = norm(axis);
        if len <= 1e-15 {
            return Self::IDENTITY;
        }
        let (s, c) = (0.5 * angle_rad).sin_cos();
        let k = s / len;
        Self {
            w: c,
            x: axis[0] * k,
            y: axis[1] * k,
            z: axis[2] * k,
        }
        .normalized()
    }

    /// Exponential map $\exp: \mathfrak{so}(3) \to SO(3)$ mapping a rotation vector `omega`
    /// (axis times angle in radians) to a unit quaternion.
    pub fn exp(omega: Vector) -> Self {
        let theta2 = dot(omega, omega);
        let (c, sinc_half) = if theta2 < 1e-10 {
            let t2 = theta2;
            (
                1.0 - t2 / 8.0 + t2 * t2 / 384.0,
                0.5 - t2 / 48.0 + t2 * t2 / 3840.0,
            )
        } else {
            let theta = theta2.sqrt();
            let half = 0.5 * theta;
            (half.cos(), half.sin() / theta)
        };
        Self {
            w: c,
            x: omega[0] * sinc_half,
            y: omega[1] * sinc_half,
            z: omega[2] * sinc_half,
        }
        .normalized()
    }

    /// Logarithm map $\log: SO(3) \to \mathfrak{so}(3)$ returning the minimal rotation vector
    /// with $\|\omega\| \le \pi$.
    pub fn log(self) -> Vector {
        let q = self.normalized();
        // Canonical hemisphere w >= 0 gives the shortest geodesic (angle in [0, pi]).
        let (w, v) = if q.w < 0.0 {
            (-q.w, [-q.x, -q.y, -q.z])
        } else {
            (q.w, [q.x, q.y, q.z])
        };
        let v_norm = norm(v);
        if v_norm < 1e-10 {
            // 2 * atan2(v_norm, w) / v_norm = 2 / w * (1 - v_norm^2 / (3 w^2) + ...)
            let inv_w = 1.0 / w.max(1e-15);
            let ratio = v_norm * inv_w;
            let factor = 2.0 * inv_w * (1.0 - ratio * ratio / 3.0);
            scale(v, factor)
        } else {
            let theta = 2.0 * v_norm.atan2(w);
            scale(v, theta / v_norm)
        }
    }

    /// Rotate a 3D vector by this unit quaternion (`q * [0, v] * q^{-1}`).
    #[inline]
    pub fn rotate_vec(self, v: Vector) -> Vector {
        let u = [self.x, self.y, self.z];
        let t = scale(cross(u, v), 2.0);
        add(add(v, scale(t, self.w)), cross(u, t))
    }

    /// Shortest-arc geodesic spherical linear interpolation on $SO(3)$.
    pub fn slerp(self, other: Self, t: f64) -> Self {
        let a = self.normalized();
        let b = other.normalized();
        let d = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
        let b = if d < 0.0 {
            Self::new(-b.w, -b.x, -b.y, -b.z)
        } else {
            b
        };
        let d = d.abs().min(1.0);
        if d > 1.0 - 1e-8 {
            return Self::new(
                a.w + (b.w - a.w) * t,
                a.x + (b.x - a.x) * t,
                a.y + (b.y - a.y) * t,
                a.z + (b.z - a.z) * t,
            )
            .normalized();
        }
        let theta = d.acos();
        let sin_theta = theta.sin();
        let wa = ((1.0 - t) * theta).sin() / sin_theta;
        let wb = (t * theta).sin() / sin_theta;
        Self::new(
            wa * a.w + wb * b.w,
            wa * a.x + wb * b.x,
            wa * a.y + wb * b.y,
            wa * a.z + wb * b.z,
        )
        .normalized()
    }

    /// Basis columns `[x, y, z]` of the 3×3 rotation matrix.
    pub fn to_basis(self) -> [Vector; 3] {
        [
            self.rotate_vec([1.0, 0.0, 0.0]),
            self.rotate_vec([0.0, 1.0, 0.0]),
            self.rotate_vec([0.0, 0.0, 1.0]),
        ]
    }

    /// Construct a unit quaternion from right-handed orthonormal basis columns `(x, y, z)` using
    /// Shepperd's numerically stable extraction.
    pub fn from_basis(x: Vector, y: Vector, z: Vector) -> Self {
        // Matrix entries R[row][col] where columns are x, y, z.
        let (m00, m10, m20) = (x[0], x[1], x[2]);
        let (m01, m11, m21) = (y[0], y[1], y[2]);
        let (m02, m12, m22) = (z[0], z[1], z[2]);
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Self::new(0.25 * s, (m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s)
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            Self::new((m21 - m12) / s, 0.25 * s, (m01 + m10) / s, (m02 + m20) / s)
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            Self::new((m02 - m20) / s, (m01 + m10) / s, 0.25 * s, (m12 + m21) / s)
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            Self::new((m10 - m01) / s, (m02 + m20) / s, (m12 + m21) / s, 0.25 * s)
        };
        q.normalized()
    }

    /// Kerf's camera Euler convention: `Ry(pan) * Rx(tilt) * Rz(roll)` in degrees.
    pub fn from_euler_yxz_deg(tilt: f64, pan: f64, roll: f64) -> Self {
        let qy = Self::from_axis_angle([0.0, 1.0, 0.0], pan.to_radians());
        let qx = Self::from_axis_angle([1.0, 0.0, 0.0], tilt.to_radians());
        let qz = Self::from_axis_angle([0.0, 0.0, 1.0], roll.to_radians());
        qy * qx * qz
    }

    /// Extract `(tilt, pan, roll)` in degrees matching [`Self::from_euler_yxz_deg`].
    pub fn to_euler_yxz_deg(self) -> (f64, f64, f64) {
        let [right, down, forward] = self.to_basis();
        // forward = [sin(pan)*cos(tilt), -sin(tilt), cos(pan)*cos(tilt)]
        // Check: Rx(tilt) * [0,0,1] = [0, -sin(tilt), cos(tilt)], then Ry(pan) maps that to
        // [sin(pan)*cos(tilt), -sin(tilt), cos(pan)*cos(tilt)].
        let sin_tilt = (-forward[1]).clamp(-1.0, 1.0);
        let tilt = sin_tilt.asin();
        let cos_tilt = tilt.cos();
        let (pan, roll) = if cos_tilt.abs() > 1e-7 {
            let pan = forward[0].atan2(forward[2]);
            let roll = right[1].atan2(down[1]);
            (pan, roll)
        } else {
            // Gimbal lock: set roll = 0 and solve pan from right vector.
            let pan = (-right[2]).atan2(right[0]);
            (pan, 0.0)
        };
        (tilt.to_degrees(), pan.to_degrees(), roll.to_degrees())
    }
}

impl Mul for Quat {
    type Output = Self;
    #[inline]
    fn mul(self, r: Self) -> Self {
        Self {
            w: self.w * r.w - self.x * r.x - self.y * r.y - self.z * r.z,
            x: self.w * r.x + self.x * r.w + self.y * r.z - self.z * r.y,
            y: self.w * r.y - self.x * r.z + self.y * r.w + self.z * r.x,
            z: self.w * r.z + self.x * r.y - self.y * r.x + self.z * r.w,
        }
    }
}

/// 6-DOF Lie algebra element $\xi = (\omega, v) \in \mathfrak{se}(3)$: angular velocity `omega`
/// (rad/s) and linear velocity `v` (world units/s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Twist {
    pub omega: Vector,
    pub v: Vector,
}

impl Twist {
    pub const ZERO: Self = Self {
        omega: [0.0; 3],
        v: [0.0; 3],
    };

    pub const fn new(omega: Vector, v: Vector) -> Self {
        Self { omega, v }
    }

    pub const fn linear(v: Vector) -> Self {
        Self { omega: [0.0; 3], v }
    }

    pub const fn angular(omega: Vector) -> Self {
        Self { omega, v: [0.0; 3] }
    }

    #[inline]
    pub fn scale(self, s: f64) -> Self {
        Self {
            omega: scale(self.omega, s),
            v: scale(self.v, s),
        }
    }

    /// Spatial velocity $\dot{p} = \omega \times p + v$ of a point `p` rigidly moving under this
    /// spatial twist.
    #[inline]
    pub fn point_velocity(self, p: Vector) -> Vector {
        add(cross(self.omega, p), self.v)
    }

    #[inline]
    pub fn norm_sq(self) -> f64 {
        dot(self.omega, self.omega) + dot(self.v, self.v)
    }
}

impl Add for Twist {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            omega: add(self.omega, rhs.omega),
            v: add(self.v, rhs.v),
        }
    }
}

impl Sub for Twist {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            omega: sub(self.omega, rhs.omega),
            v: sub(self.v, rhs.v),
        }
    }
}

impl Neg for Twist {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            omega: scale(self.omega, -1.0),
            v: scale(self.v, -1.0),
        }
    }
}

/// Apply the $SO(3)$ left Jacobian $V(\omega)$ to `v`.
fn left_jacobian_apply(omega: Vector, v: Vector) -> Vector {
    let theta2 = dot(omega, omega);
    let (a, b) = if theta2 < 1e-8 {
        (
            0.5 - theta2 / 24.0 + theta2 * theta2 / 720.0,
            1.0 / 6.0 - theta2 / 120.0 + theta2 * theta2 / 5040.0,
        )
    } else {
        let theta = theta2.sqrt();
        (
            (1.0 - theta.cos()) / theta2,
            (theta - theta.sin()) / (theta2 * theta),
        )
    };
    let w_x_v = cross(omega, v);
    let w_x_w_x_v = cross(omega, w_x_v);
    add(add(v, scale(w_x_v, a)), scale(w_x_w_x_v, b))
}

/// Apply the inverse $SO(3)$ left Jacobian $V(\omega)^{-1}$ to `t`.
fn left_jacobian_inv_apply(omega: Vector, t: Vector) -> Vector {
    let theta2 = dot(omega, omega);
    let c = if theta2 < 1e-8 {
        1.0 / 12.0 + theta2 / 720.0 + theta2 * theta2 / 30_240.0
    } else {
        let theta = theta2.sqrt();
        let half = 0.5 * theta;
        (1.0 - half * (half.cos() / half.sin())) / theta2
    };
    let w_x_t = cross(omega, t);
    let w_x_w_x_t = cross(omega, w_x_t);
    add(sub(t, scale(w_x_t, 0.5)), scale(w_x_w_x_t, c))
}

/// A 3D rigid-body pose $T = (R, t) \in SE(3)$ acting on points as $p \mapsto R p + t$.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Se3 {
    pub rot: Quat,
    pub trans: Vector,
}

impl Se3 {
    pub const IDENTITY: Self = Self {
        rot: Quat::IDENTITY,
        trans: [0.0; 3],
    };

    pub const fn new(rot: Quat, trans: Vector) -> Self {
        Self { rot, trans }
    }

    pub const fn from_translation(trans: Vector) -> Self {
        Self {
            rot: Quat::IDENTITY,
            trans,
        }
    }

    pub const fn from_rotation(rot: Quat) -> Self {
        Self {
            rot,
            trans: [0.0; 3],
        }
    }

    /// Group product `self * rhs`: apply `rhs` first, then `self`.
    #[inline]
    pub fn compose(&self, rhs: &Self) -> Self {
        Self {
            rot: (self.rot * rhs.rot).normalized(),
            trans: add(self.rot.rotate_vec(rhs.trans), self.trans),
        }
    }

    /// Group inverse $T^{-1} = (R^T, -R^T t)$.
    #[inline]
    pub fn inverse(&self) -> Self {
        let inv_rot = self.rot.inverse();
        Self {
            rot: inv_rot,
            trans: scale(inv_rot.rotate_vec(self.trans), -1.0),
        }
    }

    /// Transform a 3D point: $R p + t$.
    #[inline]
    pub fn apply_point(&self, p: Vector) -> Vector {
        add(self.rot.rotate_vec(p), self.trans)
    }

    /// Transform a 3D direction vector: $R v$.
    #[inline]
    pub fn apply_vec(&self, v: Vector) -> Vector {
        self.rot.rotate_vec(v)
    }

    /// Exponential map $\exp: \mathfrak{se}(3) \to SE(3)$ (exact screw motion).
    pub fn exp(twist: Twist) -> Self {
        Self {
            rot: Quat::exp(twist.omega),
            trans: left_jacobian_apply(twist.omega, twist.v),
        }
    }

    /// Logarithm map $\log: SE(3) \to \mathfrak{se}(3)$.
    pub fn log(&self) -> Twist {
        let omega = self.rot.log();
        let v = left_jacobian_inv_apply(omega, self.trans);
        Twist { omega, v }
    }

    /// Left-invariant geodesic interpolation on $SE(3)$ with constant body-frame twist.
    pub fn interpolate(&self, other: &Self, t: f64) -> Self {
        let delta = self.inverse().compose(other).log();
        self.compose(&Self::exp(delta.scale(t)))
    }

    /// Adjoint action $\mathrm{Ad}_T(\xi)$ mapping a twist in the child frame to the parent frame:
    /// $\omega' = R \omega$, $v' = t \times (R \omega) + R v$.
    pub fn adjoint(&self, twist: Twist) -> Twist {
        let omega = self.rot.rotate_vec(twist.omega);
        let v = add(cross(self.trans, omega), self.rot.rotate_vec(twist.v));
        Twist { omega, v }
    }

    /// 6×6 Adjoint matrix $\mathrm{Ad}_T$ acting on $[\omega_x, \omega_y, \omega_z, v_x, v_y, v_z]^T$.
    pub fn adjoint_matrix(&self) -> [[f64; 6]; 6] {
        let cols = self.rot.to_basis();
        let mut m = [[0.0; 6]; 6];
        for (col, basis) in cols.iter().enumerate() {
            let t_x_r = cross(self.trans, *basis);
            for row in 0..3 {
                m[row][col] = basis[row];
                m[row + 3][col] = t_x_r[row];
                m[row + 3][col + 3] = basis[row];
            }
        }
        m
    }

    /// Lossless conversion to Kerf's [`Affine3`].
    pub fn to_affine3(&self) -> Affine3 {
        let [x, y, z] = self.rot.to_basis();
        Affine3 {
            x,
            y,
            z,
            t: self.trans,
        }
    }

    /// Extract a rigid [`Se3`] from an [`Affine3`] when its linear part is a proper right-handed
    /// orthonormal basis (`R^T R = I`, `det(R) = +1`) within `1e-5`.
    pub fn try_from_affine3(a: &Affine3) -> Option<Self> {
        let eps = 1e-5;
        let ok_unit = |v: Vector| (dot(v, v) - 1.0).abs() <= eps;
        let ok_orth = |u: Vector, v: Vector| dot(u, v).abs() <= eps;
        if !(ok_unit(a.x) && ok_unit(a.y) && ok_unit(a.z))
            || !(ok_orth(a.x, a.y) && ok_orth(a.y, a.z) && ok_orth(a.z, a.x))
        {
            return None;
        }
        let det = dot(cross(a.x, a.y), a.z);
        if (det - 1.0).abs() > eps {
            return None;
        }
        Some(Self {
            rot: Quat::from_basis(a.x, a.y, a.z),
            trans: a.t,
        })
    }

    /// Pose of a [`Camera`] in world coordinates (maps camera-local `[right, down, forward]` to
    /// world points).
    pub fn from_camera(camera: &Camera) -> Self {
        Self {
            rot: Quat::from_basis(camera.right, camera.down, camera.forward),
            trans: camera.eye,
        }
    }

    /// Build a [`Camera`] from this world-from-camera pose and optical intrinsics.
    pub fn to_camera(&self, focal: f64, center: (f64, f64), height: f64) -> Camera {
        let [right, down, forward] = self.rot.to_basis();
        Camera {
            eye: self.trans,
            right,
            down,
            forward,
            focal,
            center,
            height,
        }
    }

    /// World pose of a [`Lens`] on a frame of `(width, height)` pixels, matching
    /// [`Camera::of`] with no parent rig.
    pub fn from_lens(lens: &Lens, (width, height): (f64, f64)) -> Self {
        let rest_dist = height / 2.0 / (Camera::RESTING_FOV.to_radians() / 2.0).tan();
        let eye = [
            lens.at[0] * width,
            lens.at[1] * height,
            lens.at[2] * height - rest_dist,
        ];
        Self {
            rot: Quat::from_euler_yxz_deg(lens.tilt, lens.pan, lens.roll),
            trans: eye,
        }
    }

    /// Convert this camera world pose back into a [`Lens`] on a frame of `(width, height)` pixels.
    pub fn to_lens(&self, (width, height): (f64, f64), fov: f64) -> Lens {
        let rest_dist = height / 2.0 / (Camera::RESTING_FOV.to_radians() / 2.0).tan();
        let (tilt, pan, roll) = self.rot.to_euler_yxz_deg();
        let w = width.max(1e-9);
        let h = height.max(1e-9);
        Lens {
            at: [
                self.trans[0] / w,
                self.trans[1] / h,
                (self.trans[2] + rest_dist) / h,
            ],
            tilt,
            pan,
            roll,
            fov,
        }
    }
}

impl Mul for Se3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self.compose(&rhs)
    }
}

/// Analytical instantaneous screen-space velocity `(du/dt, dv/dt)` in pixels/second of a 3D world
/// point `world_point` moving at `world_vel` when viewed by `camera` whose world pose evolves
/// under the spatial twist `cam_twist` (`T_cam(dt) = exp(cam_twist * dt) * T_cam`).
///
/// Returns `None` if `world_point` is at or behind the camera plane (`ahead <= 1e-9`).
pub fn screen_velocity(
    camera: &Camera,
    cam_twist: Twist,
    world_point: Vector,
    world_vel: Vector,
) -> Option<(f64, f64)> {
    let rel = sub(world_point, camera.eye);
    let qx = dot(rel, camera.right);
    let qy = dot(rel, camera.down);
    let qz = dot(rel, camera.forward);
    if qz <= 1e-9 {
        return None;
    }
    let v_rel = sub(world_vel, cam_twist.point_velocity(world_point));
    let dqx = dot(v_rel, camera.right);
    let dqy = dot(v_rel, camera.down);
    let dqz = dot(v_rel, camera.forward);
    let inv_qz2 = 1.0 / (qz * qz);
    Some((
        camera.focal * (dqx * qz - qx * dqz) * inv_qz2,
        camera.focal * (dqy * qz - qy * dqz) * inv_qz2,
    ))
}

/// A 3D ray `origin + t * dir`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray3 {
    pub origin: Vector,
    pub dir: Vector,
}

impl Ray3 {
    pub const fn new(origin: Vector, dir: Vector) -> Self {
        Self { origin, dir }
    }

    #[inline]
    pub fn at(&self, t: f64) -> Vector {
        add(self.origin, scale(self.dir, t))
    }

    /// Unit ray from `camera.eye` through output pixel `(x, y)`.
    pub fn from_camera_pixel(camera: &Camera, (x, y): (f64, f64)) -> Self {
        let raw = add(
            add(
                scale(camera.right, x - camera.center.0),
                scale(camera.down, y - camera.center.1),
            ),
            scale(camera.forward, camera.focal),
        );
        let len = norm(raw).max(1e-15);
        Self {
            origin: camera.eye,
            dir: scale(raw, 1.0 / len),
        }
    }

    /// Intersect this ray with `plane`, returning `(t, hit_point)` when non-parallel (`|dir·n| > 1e-12`).
    pub fn intersect_plane(&self, plane: &Plane3) -> Option<(f64, Vector)> {
        let denom = dot(self.dir, plane.normal);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = dot(sub(plane.origin, self.origin), plane.normal) / denom;
        Some((t, self.at(t)))
    }
}

/// An oriented 3D plane defined by a point `origin` and a unit `normal`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane3 {
    pub origin: Vector,
    pub normal: Vector,
}

impl Plane3 {
    pub fn new(origin: Vector, normal: Vector) -> Option<Self> {
        let len = norm(normal);
        (len > 1e-12).then(|| Self {
            origin,
            normal: scale(normal, 1.0 / len),
        })
    }

    #[inline]
    pub fn signed_distance(&self, point: Vector) -> f64 {
        dot(sub(point, self.origin), self.normal)
    }

    #[inline]
    pub fn project_point(&self, point: Vector) -> Vector {
        sub(point, scale(self.normal, self.signed_distance(point)))
    }

    /// Line of intersection of two non-parallel planes, returned as a unit-direction [`Ray3`].
    pub fn intersect_plane(&self, other: &Self) -> Option<Ray3> {
        let dir = cross(self.normal, other.normal);
        let denom = dot(dir, dir);
        if denom < 1e-12 {
            return None;
        }
        let d1 = dot(self.normal, self.origin);
        let d2 = dot(other.normal, other.origin);
        let origin = scale(
            cross(sub(scale(other.normal, d1), scale(self.normal, d2)), dir),
            1.0 / denom,
        );
        Some(Ray3 {
            origin,
            dir: scale(dir, 1.0 / denom.sqrt()),
        })
    }
}
