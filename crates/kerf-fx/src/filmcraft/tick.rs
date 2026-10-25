//! FilmCraft's unit of time, which its effects keep: a tick is
//! 1/254 016 000 000 of a second, so common frame and sample rates divide it.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point or duration in time, in ticks.
/// Arithmetic saturates at the i64 storage limits; representable results remain exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);
    pub const MAX: Tick = Tick(i64::MAX / 4);
    pub const MIN: Tick = Tick(i64::MIN / 4);

    pub fn from_seconds_f64(s: f64) -> Tick {
        Tick((s * TICKS_PER_SECOND as f64).round() as i64)
    }
    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }
    /// Exact conversion from a count of `units` at `per_second` (e.g. samples at 48 000).
    pub fn from_units(units: i64, per_second: i64) -> Tick {
        Tick(clamp_i64(
            (units as i128 * TICKS_PER_SECOND as i128)
                .checked_div(per_second as i128)
                .unwrap_or(0),
        ))
    }
    /// Floor conversion to a count of units at `per_second`.
    pub fn to_units_floor(self, per_second: i64) -> i64 {
        clamp_i64((self.0 as i128 * per_second as i128).div_euclid(TICKS_PER_SECOND as i128))
    }
    /// Conversion from a rational timestamp `pts * num / den` seconds (container timebases).
    pub fn from_rational(pts: i64, num: i64, den: i64) -> Tick {
        if den == 0 {
            return Tick::ZERO;
        }
        let Some(t) = (pts as i128 * num as i128).checked_mul(TICKS_PER_SECOND as i128) else {
            // A product beyond i128 divided by an i64 cannot fit back into i64.
            return Tick(if (pts < 0) ^ (num < 0) ^ (den < 0) {
                i64::MIN
            } else {
                i64::MAX
            });
        };
        Tick(clamp_i64(t.div_euclid(den as i128)))
    }
    /// Inverse of [`Tick::from_rational`], floored to the timebase.
    pub fn to_rational_floor(self, num: i64, den: i64) -> i64 {
        clamp_i64(
            (self.0 as i128 * den as i128)
                .checked_div_euclid(num as i128 * TICKS_PER_SECOND as i128)
                .unwrap_or(0),
        )
    }
    /// Inverse of [`Tick::from_rational`], rounded to the nearest timebase unit (halves up).
    ///
    /// For finding a sample by its timestamp: containers store timestamps rounded to their
    /// timebase (Matroska usually to 1 ms), so a stamp can sit up to half a unit after the true
    /// time. Flooring the requested time misses every frame whose stamp was rounded up.
    pub fn to_rational_round(self, num: i64, den: i64) -> i64 {
        let n = self.0 as i128 * den as i128;
        let unit = num as i128 * TICKS_PER_SECOND as i128;
        let (n, unit) = if unit < 0 { (-n, -unit) } else { (n, unit) };
        let (Some(q), Some(r)) = (n.checked_div_euclid(unit), n.checked_rem_euclid(unit)) else {
            return 0;
        };
        // 0 <= r < unit: round up from the halfway point
        clamp_i64(if r * 2 >= unit { q + 1 } else { q })
    }
    pub fn abs(self) -> Tick {
        Tick(self.0.saturating_abs())
    }
    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }
    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }
    pub fn clamp(self, lo: Tick, hi: Tick) -> Tick {
        // max/min rather than `clamp`, which panics when the bounds cross.
        Tick(self.0.max(lo.0).min(hi.0))
    }
    /// Scale by a rational factor (`num/den`), flooring.
    pub fn mul_ratio(self, num: i64, den: i64) -> Tick {
        Tick(clamp_i64(
            (self.0 as i128 * num as i128)
                .checked_div_euclid(den as i128)
                .unwrap_or(0),
        ))
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0.saturating_add(o.0))
    }
}
impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0.saturating_sub(o.0))
    }
}
impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.saturating_neg())
    }
}
impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_add(o.0);
    }
}
impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
