//! Forward-mode automatic differentiation over [`Num`].
//!
//! Any kernel written generically over [`Num`] can be evaluated with [`Dual`] to obtain both its
//! value and exact analytical derivative in a single pass, enabling gradient-based optimization
//! (inverse color grading, parameter fitting, and director-guided control).

use crate::Num;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A dual number `value + deriv * ε` with `ε² = 0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dual {
    pub value: f32,
    pub deriv: f32,
}

impl Dual {
    /// A constant with zero derivative.
    #[inline(always)]
    pub const fn constant(value: f32) -> Self {
        Self { value, deriv: 0.0 }
    }

    /// An independent variable with unit derivative (`d/dx x = 1`).
    #[inline(always)]
    pub const fn variable(value: f32) -> Self {
        Self { value, deriv: 1.0 }
    }

    #[inline(always)]
    pub const fn new(value: f32, deriv: f32) -> Self {
        Self { value, deriv }
    }
}

impl Add for Dual {
    type Output = Self;
    #[inline(always)]
    fn add(self, rhs: Self) -> Self {
        Self {
            value: self.value + rhs.value,
            deriv: self.deriv + rhs.deriv,
        }
    }
}

impl Sub for Dual {
    type Output = Self;
    #[inline(always)]
    fn sub(self, rhs: Self) -> Self {
        Self {
            value: self.value - rhs.value,
            deriv: self.deriv - rhs.deriv,
        }
    }
}

impl Mul for Dual {
    type Output = Self;
    #[inline(always)]
    fn mul(self, rhs: Self) -> Self {
        Self {
            value: self.value * rhs.value,
            deriv: self.deriv * rhs.value + self.value * rhs.deriv,
        }
    }
}

impl Div for Dual {
    type Output = Self;
    #[inline(always)]
    fn div(self, rhs: Self) -> Self {
        let inv = 1.0 / rhs.value;
        Self {
            value: self.value * inv,
            deriv: (self.deriv * rhs.value - self.value * rhs.deriv) * (inv * inv),
        }
    }
}

impl Neg for Dual {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self {
        Self {
            value: -self.value,
            deriv: -self.deriv,
        }
    }
}

impl Num for Dual {
    #[inline(always)]
    fn lit(value: f32) -> Self {
        Self::constant(value)
    }

    #[inline(always)]
    fn min(self, other: Self) -> Self {
        if self.value <= other.value {
            self
        } else {
            other
        }
    }

    #[inline(always)]
    fn max(self, other: Self) -> Self {
        if self.value >= other.value {
            self
        } else {
            other
        }
    }

    #[inline(always)]
    fn sqrt(self) -> Self {
        let v = self.value.sqrt();
        let d = if v > 0.0 && self.deriv != 0.0 {
            self.deriv / (2.0 * v)
        } else {
            0.0
        };
        Self { value: v, deriv: d }
    }

    #[inline(always)]
    fn abs(self) -> Self {
        if self.value >= 0.0 {
            self
        } else {
            -self
        }
    }

    #[inline(always)]
    fn hypot(self, other: Self) -> Self {
        let h = self.value.hypot(other.value);
        let d = if h > 0.0 {
            (self.value * self.deriv + other.value * other.deriv) / h
        } else {
            0.0
        };
        Self { value: h, deriv: d }
    }

    #[inline(always)]
    fn pow(self, power: f32) -> Self {
        let value = self.value.powf(power);
        let deriv = if self.deriv == 0.0 || power == 0.0 {
            0.0
        } else if (power - 1.0).abs() == 0.0 {
            self.deriv
        } else {
            power * self.value.powf(power - 1.0) * self.deriv
        };
        Self { value, deriv }
    }

    #[inline(always)]
    fn powf(self, power: Self) -> Self {
        let value = self.value.powf(power.value);
        let d_base = if self.deriv == 0.0 || power.value == 0.0 {
            0.0
        } else if (power.value - 1.0).abs() == 0.0 {
            self.deriv
        } else {
            power.value * self.value.powf(power.value - 1.0) * self.deriv
        };
        let d_exp = if power.deriv == 0.0 || self.value <= 0.0 {
            0.0
        } else {
            value * self.value.ln() * power.deriv
        };
        Self {
            value,
            deriv: d_base + d_exp,
        }
    }

    #[inline(always)]
    fn exp(self) -> Self {
        let v = self.value.exp();
        Self {
            value: v,
            deriv: v * self.deriv,
        }
    }

    #[inline(always)]
    fn log10(self) -> Self {
        let value = self.value.log10();
        let deriv = if self.deriv == 0.0 {
            0.0
        } else {
            self.deriv / (self.value * std::f32::consts::LN_10)
        };
        Self { value, deriv }
    }

    #[inline(always)]
    fn rem_euclid(self, by: Self) -> Self {
        let q = (self.value / by.value).floor();
        Self {
            value: self.value.rem_euclid(by.value),
            deriv: self.deriv - q * by.deriv,
        }
    }

    #[inline(always)]
    fn if_at_most<const N: usize>(
        self,
        limit: Self,
        yes: impl FnOnce() -> [Self; N],
        no: impl FnOnce() -> [Self; N],
    ) -> [Self; N] {
        if self.value <= limit.value {
            yes()
        } else {
            no()
        }
    }
}

/// Evaluate `f(x)` and its exact derivative `f'(x)` at `at`.
pub fn derivative(f: impl Fn(Dual) -> Dual, at: f32) -> (f32, f32) {
    let out = f(Dual::variable(at));
    (out.value, out.deriv)
}

/// Evaluate `f(x)` and its exact gradient `∇f(x)` at `at` in `K` forward passes.
pub fn gradient<const K: usize>(f: impl Fn([Dual; K]) -> Dual, at: [f32; K]) -> (f32, [f32; K]) {
    let mut value = 0.0;
    let mut grad = [0.0; K];
    for axis in 0..K {
        let args = std::array::from_fn(|i| {
            if i == axis {
                Dual::variable(at[i])
            } else {
                Dual::constant(at[i])
            }
        });
        let out = f(args);
        if axis == 0 {
            value = out.value;
        }
        grad[axis] = out.deriv;
    }
    (value, grad)
}

/// Evaluate `f(x)` and its exact `O × I` Jacobian matrix `J[out][in]` at `at`.
pub fn jacobian<const I: usize, const O: usize>(
    f: impl Fn([Dual; I]) -> [Dual; O],
    at: [f32; I],
) -> ([f32; O], [[f32; I]; O]) {
    let mut values = [0.0; O];
    let mut jac = [[0.0; I]; O];
    for col in 0..I {
        let args = std::array::from_fn(|i| {
            if i == col {
                Dual::variable(at[i])
            } else {
                Dual::constant(at[i])
            }
        });
        let out = f(args);
        for row in 0..O {
            if col == 0 {
                values[row] = out[row].value;
            }
            jac[row][col] = out[row].deriv;
        }
    }
    (values, jac)
}
