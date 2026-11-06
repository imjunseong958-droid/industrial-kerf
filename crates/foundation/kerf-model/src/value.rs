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

/// A cubic curve from (0, 0) to (1, 1) through two control points, as a graph editor draws one:
/// across is time, up is the value's way from this keyframe to the next. Time only goes forward, so
/// the control points stay within 0 to 1 across. Up they may overshoot, for a bounce.
///
/// The numbers are ten-thousandths, which is all a document writes, so that a curve compares and
/// hashes exactly and prints as it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bezier {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

/// How far a curve may overshoot, up or down.
const BEZIER_REACH: f64 = 5.0;

impl Bezier {
    /// The curve through (`x1`, `y1`) and (`x2`, `y2`), if those can be one.
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Option<Self> {
        let across = 0.0..=1.0;
        let up = -BEZIER_REACH..=BEZIER_REACH;
        (across.contains(&x1) && across.contains(&x2) && up.contains(&y1) && up.contains(&y2)).then(
            || {
                let units = |value: f64| (value * 10_000.0).round() as i32;
                Self {
                    x1: units(x1),
                    y1: units(y1),
                    x2: units(x2),
                    y2: units(y2),
                }
            },
        )
    }

    /// Whether its control points go past 0 or 1 up, which is what lets it overshoot.
    pub fn overshoots(self) -> bool {
        let up = 0..=10_000;
        !up.contains(&self.y1) || !up.contains(&self.y2)
    }

    fn points(self) -> [f64; 4] {
        [self.x1, self.y1, self.x2, self.y2].map(|units| f64::from(units) / 10_000.0)
    }

    /// The cubic through 0, `a`, `b`, 1 at `s`.
    fn cubic(a: f64, b: f64, s: f64) -> f64 {
        let r = 1.0 - s;
        3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
    }

    /// Where along the curve it is `t` across. It only goes forward, so halving finds it.
    fn along(self, t: f64) -> f64 {
        let [x1, _, x2, _] = self.points();
        let (mut low, mut high) = (0.0f64, 1.0f64);
        for _ in 0..40 {
            let middle = (low + high) / 2.0;
            if Self::cubic(x1, x2, middle) < t {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) / 2.0
    }

    /// How far up the curve is when it is `t` across.
    pub fn at(self, t: f64) -> f64 {
        let [_, y1, _, y2] = self.points();
        Self::cubic(y1, y2, self.along(t))
    }

    /// The curve split where it is `t` across, as two curves that together draw it, or `None` when
    /// a part neither rises nor falls and so has no curve of its own.
    fn split(self, t: f64) -> Option<(Self, Self)> {
        let [x1, y1, x2, y2] = self.points();
        let s = self.along(t);
        // De Casteljau on (0,0), (x1,y1), (x2,y2), (1,1).
        let lerp = |a: f64, b: f64| a + (b - a) * s;
        let (ax, ay) = (lerp(0.0, x1), lerp(0.0, y1));
        let (bx, by) = (lerp(x1, x2), lerp(y1, y2));
        let (cx, cy) = (lerp(x2, 1.0), lerp(y2, 1.0));
        let (dx, dy) = (lerp(ax, bx), lerp(ay, by));
        let (ex, ey) = (lerp(bx, cx), lerp(by, cy));
        let (mx, my) = (lerp(dx, ex), lerp(dy, ey));
        const FLAT: f64 = 1e-6;
        if mx < FLAT || 1.0 - mx < FLAT || my.abs() < FLAT || (1.0 - my).abs() < FLAT {
            return None;
        }
        let head = Self::new(ax / mx, ay / my, dx / mx, dy / my)?;
        let tail = Self::new(
            (ex - mx) / (1.0 - mx),
            (ey - my) / (1.0 - my),
            (cx - mx) / (1.0 - mx),
            (cy - my) / (1.0 - my),
        )?;
        Some((head, tail))
    }
}

impl std::fmt::Display for Bezier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [x1, y1, x2, y2] = self.points();
        write!(f, "{x1},{y1},{x2},{y2}")
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
    Bezier(Bezier),
}

impl Interp {
    /// The ones with names.
    pub const ALL: [Self; 5] = [
        Self::Linear,
        Self::Hold,
        Self::EaseIn,
        Self::EaseOut,
        Self::Ease,
    ];

    /// It as a document writes it: its name, or a curve's four numbers.
    pub fn name(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Linear => "linear".into(),
            Self::Hold => "hold".into(),
            Self::EaseIn => "ease-in".into(),
            Self::EaseOut => "ease-out".into(),
            Self::Ease => "ease".into(),
            Self::Bezier(curve) => curve.to_string().into(),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        if let Some(named) = Self::ALL.into_iter().find(|interp| interp.name() == text) {
            return Some(named);
        }
        let numbers: Vec<f64> = text
            .split(',')
            .map(|number| number.parse().ok().filter(|n: &f64| n.is_finite()))
            .collect::<Option<_>>()?;
        match numbers[..] {
            [x1, y1, x2, y2] => Bezier::new(x1, y1, x2, y2).map(Self::Bezier),
            _ => None,
        }
    }

    /// It as a curve, for those that are one. The named easings are cubics in `t`, which a curve
    /// with its control points a third of the way across each draws exactly.
    fn as_curve(self) -> Option<Bezier> {
        let (y1, y2) = match self {
            Self::Linear | Self::Hold => return None,
            Self::Bezier(curve) => return Some(curve),
            Self::EaseIn => (0.0, 1.0 / 3.0),
            Self::EaseOut => (2.0 / 3.0, 1.0),
            Self::Ease => (0.0, 1.0),
        };
        Bezier::new(1.0 / 3.0, y1, 2.0 / 3.0, y2)
    }

    /// How far from this keyframe to the next it is, `t` of the way across.
    pub fn curve(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::Ease => t * t * (3.0 - 2.0 * t),
            Self::Bezier(curve) => curve.at(t),
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
        let before = keys.iter().rev().find(|key| key.at <= at);
        let after = keys.iter().find(|key| key.at > at);
        let interp = before.map_or(Interp::Linear, |key| key.interp);
        // A curve cut in two is two curves, which together draw it. So is a named easing, which is
        // a curve with a name.
        let cut = match (before, after, interp.as_curve()) {
            (Some(left), Some(right), Some(curve)) if left.at < at => {
                let span = (right.at - left.at).flicks() as f64;
                curve.split((at - left.at).flicks() as f64 / span)
            }
            _ => None,
        };
        let boundary = |at, interp| Keyframe {
            at,
            value: value.clone(),
            interp,
        };
        let mut left: Vec<_> = keys.iter().filter(|key| key.at < at).cloned().collect();
        if let (Some((head, _)), Some(last)) = (cut, left.last_mut()) {
            last.interp = Interp::Bezier(head);
        }
        left.push(boundary(at, interp));
        let interp = cut.map_or(interp, |(_, tail)| Interp::Bezier(tail));
        let mut right = vec![boundary(Time::ZERO, interp)];
        right.extend(keys.iter().filter(|key| key.at > at).map(|key| Keyframe {
            at: key.at - at,
            ..key.clone()
        }));
        (Self::Animated(left), Self::Animated(right))
    }
}

#[cfg(test)]
mod tests {
    /// The same table holds the graph editor to these curves, in `apps/kerf-qt/tests/graph_test.cpp`.
    #[test]
    fn curves_read_the_table_the_graph_editor_is_held_to() {
        for (interp, t, wanted) in [
            ("linear", 0.25, 0.25),
            ("hold", 0.9, 0.0),
            ("ease-in", 0.5, 0.25),
            ("ease-out", 0.5, 0.75),
            ("ease", 0.25, 0.15625),
            ("0.25,0.1,0.25,1", 0.5, 0.802403),
            ("0.42,0,0.58,1", 0.25, 0.129162),
            ("0.5,-1,0.5,2", 0.5, 0.5),
            ("0.5,-1,0.5,2", 0.25, -0.182322),
            ("0,0,1,1", 0.3, 0.3),
        ] {
            let curve = super::Interp::parse(interp).expect("an interp");
            assert!((curve.curve(t) - wanted).abs() < 1e-5, "{interp} at {t}");
        }
    }

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
    fn a_named_easing_keeps_its_motion_across_a_split() {
        for interp in [Interp::EaseIn, Interp::EaseOut, Interp::Ease] {
            let param = Param::Animated(vec![
                Keyframe {
                    at: frames(0),
                    value: Value::Number(0.0),
                    interp,
                },
                Keyframe {
                    at: frames(48),
                    value: Value::Number(1.0),
                    interp: Interp::Linear,
                },
            ]);
            let number = |param: &Param, at: Time| match param.value_at(at) {
                Value::Number(number) => number,
                other => panic!("{other:?}"),
            };
            let (head, tail) = param.split(frames(16));
            for at in 0..48 {
                let whole = number(&param, frames(at));
                let parted = if at < 16 {
                    number(&head, frames(at))
                } else {
                    number(&tail, frames(at - 16))
                };
                assert!(
                    (whole - parted).abs() < 2e-3,
                    "{interp:?} at {at}: {whole} against {parted}"
                );
            }
        }
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

    fn curve(text: &str) -> Interp {
        Interp::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    #[test]
    fn a_curve_reads_and_prints_as_it_is_written() {
        for text in [
            "0.25,0.1,0.25,1",
            "0,0,1,1",
            "0.5,-0.4,0.5,1.4",
            "0.1234,1,0.8765,0",
        ] {
            assert_eq!(curve(text).name(), text);
        }
        assert_eq!(curve(".5,0,.5,1").name(), "0.5,0,0.5,1");
        assert_eq!(curve("ease"), Interp::Ease);
        assert_eq!(Interp::Ease.name(), "ease");
        for refused in [
            "bounce",
            "1,2,3",
            "0.1,0.2,0.3,0.4,0.5",
            "-0.1,0,0.5,1",
            "0.5,0,1.1,1",
            "0.5,9,0.5,1",
            "0.5,NaN,0.5,1",
            "",
        ] {
            assert_eq!(Interp::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_straight_curve_is_linear_and_ends_where_it_should() {
        let straight = curve("0.25,0.25,0.75,0.75");
        let swoop = curve("0.2,0.8,0.4,1");
        for step in 0..=100 {
            let t = f64::from(step) / 100.0;
            assert!((straight.curve(t) - t).abs() < 1e-6, "{t}");
        }
        assert!(swoop.curve(0.0).abs() < 1e-9);
        assert!((swoop.curve(1.0) - 1.0).abs() < 1e-9);
        // Out fast: well past linear halfway.
        assert!(swoop.curve(0.5) > 0.8);
    }

    #[test]
    fn a_curve_matches_the_easings_it_can_draw() {
        let third = 1.0 / 3.0;
        let ease = Bezier::new(third, 0.0, 2.0 * third, 1.0).expect("a curve");
        for step in 0..=20 {
            let t = f64::from(step) / 20.0;
            assert!(
                (ease.at(t) - Interp::Ease.curve(t)).abs() < 1e-3,
                "{t}: {} against {}",
                ease.at(t),
                Interp::Ease.curve(t)
            );
        }
    }

    #[test]
    fn a_curve_can_overshoot() {
        let back = curve("0.3,1.6,0.7,1");
        let peak = (0..=100)
            .map(|step| back.curve(f64::from(step) / 100.0))
            .fold(0.0f64, f64::max);
        assert!(peak > 1.05, "{peak}");
    }

    #[test]
    fn a_curve_cut_anywhere_draws_the_same_on_both_sides() {
        for text in [
            "0.2,0.8,0.4,1",
            "0.7,0,0.3,1",
            "0.42,0,0.58,1",
            "0.3,1.3,0.7,0.9",
        ] {
            let param = Param::Animated(vec![
                Keyframe {
                    at: frames(0),
                    value: Value::Number(10.0),
                    interp: curve(text),
                },
                Keyframe {
                    at: frames(48),
                    value: Value::Number(20.0),
                    interp: Interp::Linear,
                },
            ]);
            for cut in [1, 12, 24, 37, 47] {
                let (left, right) = param.split(frames(cut));
                let number = |p: &Param, at: i64| p.value_at(frames(at)).as_number().unwrap();
                for at in 0..=cut {
                    let (a, b) = (number(&left, at), number(&param, at));
                    assert!((a - b).abs() < 2e-2, "{text} cut {cut} at {at}: {a} {b}");
                }
                for at in 0..=48 - cut {
                    let (a, b) = (number(&right, at), number(&param, at + cut));
                    assert!((a - b).abs() < 2e-2, "{text} cut {cut} at {at}: {a} {b}");
                }
            }
        }
    }

    #[test]
    fn a_cut_on_a_keyframe_or_past_the_last_changes_nothing() {
        let param = Param::Animated(vec![
            Keyframe {
                at: frames(0),
                value: Value::Number(0.0),
                interp: curve("0.2,0.8,0.4,1"),
            },
            Keyframe {
                at: frames(24),
                value: Value::Number(1.0),
                interp: curve("0.7,0,0.3,1"),
            },
            Keyframe {
                at: frames(48),
                value: Value::Number(0.0),
                interp: Interp::Linear,
            },
        ]);
        let (left, right) = param.split(frames(24));
        for at in 0..=24 {
            assert_eq!(left.value_at(frames(at)), param.value_at(frames(at)));
            assert_eq!(right.value_at(frames(at)), param.value_at(frames(at + 24)));
        }
        let (left, _) = param.split(frames(60));
        assert_eq!(left.value_at(frames(10)), param.value_at(frames(10)));
    }

    #[test]
    fn colors_parse_and_print() {
        assert_eq!(Rgba::parse("#ff8000").unwrap().to_string(), "#ff8000");
        assert_eq!(Rgba::parse("#00000080").unwrap().a, 128);
        assert!(Rgba::parse("ff8000").is_none());
        assert!(Rgba::parse("#ff80").is_none());
    }
}
