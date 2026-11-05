//! Parameter values and keyframed animation.

use crate::time::Time;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parse `#rrggbb` or `#rrggbbaa`.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
        Some(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: if hex.len() == 8 { channel(6)? } else { 255 },
        })
    }

    fn lerp(self, other: Self, t: f64) -> Self {
        let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Text(String),
    Color(Rgba),
    Enum(String),
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    fn interpolate(&self, other: &Self, t: f64) -> Self {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => Self::Number(a + (b - a) * t),
            (Self::Color(a), Self::Color(b)) => Self::Color(a.lerp(*b, t)),
            _ => self.clone(),
        }
    }
}

/// How a keyframe's value moves toward the next keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interp {
    #[default]
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    Ease,
}

impl Interp {
    pub const ALL: [Self; 5] = [
        Self::Linear,
        Self::Hold,
        Self::EaseIn,
        Self::EaseOut,
        Self::Ease,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Hold => "hold",
            Self::EaseIn => "ease-in",
            Self::EaseOut => "ease-out",
            Self::Ease => "ease",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|interp| interp.name() == text)
    }

    fn curve(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::Ease => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// A keyframe at a time relative to the start of its clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub at: Time,
    pub value: Value,
    pub interp: Interp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Static(Value),
    /// Keyframes in strictly increasing time order.
    Animated(Vec<Keyframe>),
}

impl Param {
    /// The value at `at`, clip-relative. Before the first keyframe and after
    /// the last, the nearest keyframe's value holds.
    pub fn value_at(&self, at: Time) -> Value {
        let keys = match self {
            Self::Static(value) => return value.clone(),
            Self::Animated(keys) => keys,
        };
        let next = keys.partition_point(|key| key.at <= at);
        match (next.checked_sub(1).map(|i| &keys[i]), keys.get(next)) {
            (None, Some(first)) => first.value.clone(),
            (Some(last), None) => last.value.clone(),
            (Some(left), Some(right)) => {
                let span = (right.at - left.at).flicks() as f64;
                let t = (at - left.at).flicks() as f64 / span;
                left.value.interpolate(&right.value, left.interp.curve(t))
            }
            (None, None) => unreachable!("animated parameters have keyframes"),
        }
    }

    /// Move every keyframe by `delta`, as when a clip's head is trimmed.
    pub fn shifted(&self, delta: Time) -> Self {
        match self {
            Self::Static(_) => self.clone(),
            Self::Animated(keys) => Self::Animated(
                keys.iter()
                    .map(|key| Keyframe {
                        at: key.at + delta,
                        ..key.clone()
                    })
                    .collect(),
            ),
        }
    }

    /// Split at clip-relative `at`. Each half gets a keyframe at the cut that
    /// carries the evaluated value, so both halves animate exactly as before.
    pub fn split(&self, at: Time) -> (Self, Self) {
        let keys = match self {
            Self::Static(_) => return (self.clone(), self.clone()),
            Self::Animated(keys) => keys,
        };
        let value = self.value_at(at);
        let interp = keys
            .iter()
            .rev()
            .find(|key| key.at <= at)
            .map_or(Interp::Linear, |key| key.interp);
        let boundary = |at| Keyframe {
            at,
            value: value.clone(),
            interp,
        };
        let mut left: Vec<_> = keys.iter().filter(|key| key.at < at).cloned().collect();
        left.push(boundary(at));
        let mut right = vec![boundary(Time::ZERO)];
        right.extend(keys.iter().filter(|key| key.at > at).map(|key| Keyframe {
            at: key.at - at,
            ..key.clone()
        }));
        (Self::Animated(left), Self::Animated(right))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Rate;

    fn frames(n: i64) -> Time {
        Rate::FPS_24.time_of(n)
    }

    fn ramp() -> Param {
        Param::Animated(vec![
            Keyframe {
                at: frames(0),
                value: Value::Number(0.0),
                interp: Interp::Linear,
            },
            Keyframe {
                at: frames(24),
                value: Value::Number(1.0),
                interp: Interp::Hold,
            },
            Keyframe {
                at: frames(48),
                value: Value::Number(0.5),
                interp: Interp::Linear,
            },
        ])
    }

    #[test]
    fn evaluates_linear_hold_and_extrapolation() {
        let param = ramp();
        assert_eq!(param.value_at(frames(12)), Value::Number(0.5));
        assert_eq!(param.value_at(frames(36)), Value::Number(1.0));
        assert_eq!(param.value_at(frames(-5)), Value::Number(0.0));
        assert_eq!(param.value_at(frames(100)), Value::Number(0.5));
    }

    #[test]
    fn split_preserves_the_curve_on_both_sides() {
        let param = ramp();
        let (left, right) = param.split(frames(12));
        let near =
            |a: Value, b: Value| (a.as_number().unwrap() - b.as_number().unwrap()).abs() < 1e-12;
        for n in 0..=12 {
            assert!(near(left.value_at(frames(n)), param.value_at(frames(n))));
        }
        for n in 0..=60 {
            assert!(near(
                right.value_at(frames(n)),
                param.value_at(frames(n + 12))
            ));
        }
    }

    #[test]
    fn colors_parse_and_print() {
        assert_eq!(Rgba::parse("#ff8000").unwrap().to_string(), "#ff8000");
        assert_eq!(Rgba::parse("#00000080").unwrap().a, 128);
        assert!(Rgba::parse("ff8000").is_none());
        assert!(Rgba::parse("#ff80").is_none());
    }
}
