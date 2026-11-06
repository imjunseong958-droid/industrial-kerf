//! How fast a clip plays its source: one ratio, or a ramp of them over the clip.
//!
//! Everything a clip needs from its speed comes down to one curve: how much source it has played
//! by a moment of its own time ([`Speed::played`]) and the way back ([`Speed::time_for`]). At one
//! ratio that is a multiplication. A ramp adds up the area under its speed, which is exact for what
//! a document can write: linear and hold in closed form, the named easings because Simpson's rule
//! is exact for cubics, and a curve near enough at 32 panels.

use crate::time::{Ratio, Time};
use crate::value::Interp;

/// A speed at a time relative to the start of its clip, and how it moves toward the next.
#[derive(Clone, Debug, PartialEq)]
pub struct RampKey {
    pub at: Time,
    pub speed: Ratio,
    pub interp: Interp,
}

/// Speeds over a clip. Their times strictly increase, they all run the same way, and the speed
/// between two stays between them, so the source only ever moves on. Before the first key and after
/// the last the nearest one holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    keys: Vec<RampKey>,
}

impl Ramp {
    pub fn keys(&self) -> &[RampKey] {
        &self.keys
    }

    /// The source played from the first key to `at`, in flicks; negative before it.
    fn area(&self, at: f64) -> f64 {
        let (first, last) = (&self.keys[0], &self.keys[self.keys.len() - 1]);
        if at <= flicks(first.at) {
            return magnitude(first) * (at - flicks(first.at));
        }
        let mut sum = 0.0;
        for pair in self.keys.windows(2) {
            let (from, to) = (flicks(pair[0].at), flicks(pair[1].at));
            if at < to {
                return sum + piece(&pair[0], &pair[1], at - from);
            }
            sum += piece(&pair[0], &pair[1], to - from);
        }
        sum + magnitude(last) * (at - flicks(last.at))
    }

    /// The slowest it goes, which bounds how long any amount of source takes.
    fn slowest(&self) -> f64 {
        self.keys
            .iter()
            .map(magnitude)
            .fold(f64::INFINITY, f64::min)
    }

    /// The speed at `at`, without its sign.
    fn at(&self, at: Time) -> f64 {
        let next = self.keys.partition_point(|key| key.at <= at);
        match (
            next.checked_sub(1).map(|i| &self.keys[i]),
            self.keys.get(next),
        ) {
            (None, Some(first)) => magnitude(first),
            (Some(last), None) => magnitude(last),
            (Some(left), Some(right)) => {
                let along = flicks(at - left.at) / flicks(right.at - left.at);
                magnitude(left) + (magnitude(right) - magnitude(left)) * left.interp.curve(along)
            }
            (None, None) => 1.0,
        }
    }
}

fn flicks(time: Time) -> f64 {
    time.flicks() as f64
}

fn magnitude(key: &RampKey) -> f64 {
    key.speed.value().abs()
}

/// The source played `by` into the stretch from `left` to `right`.
fn piece(left: &RampKey, right: &RampKey, by: f64) -> f64 {
    let (from, to) = (magnitude(left), magnitude(right));
    let length = flicks(right.at - left.at);
    let change = to - from;
    match left.interp {
        Interp::Hold => from * by,
        Interp::Linear => from * by + change * by * by / (2.0 * length),
        interp => {
            let end = by / length;
            let panels = if matches!(interp, Interp::Bezier(_)) {
                32
            } else {
                1
            };
            let width = end / f64::from(panels);
            let mut rise = 0.0;
            for panel in 0..panels {
                let start = width * f64::from(panel);
                rise += width / 6.0
                    * (interp.curve(start)
                        + 4.0 * interp.curve(start + width / 2.0)
                        + interp.curve(start + width));
            }
            from * by + change * length * rise
        }
    }
}

/// How fast a media or nested clip plays its source.
#[derive(Clone, Debug, PartialEq)]
pub enum Speed {
    Constant(Ratio),
    Ramp(Ramp),
}

static NORMAL: Speed = Speed::Constant(Ratio::ONE);

impl Speed {
    pub const ONE: Self = Self::Constant(Ratio::ONE);

    /// Normal speed, for what has no source to play.
    pub fn normal() -> &'static Self {
        &NORMAL
    }

    /// The speed these keys give: a ramp, or one ratio when they all say the same. The source must
    /// keep moving one way, so they may not change direction, and a curve between them may not
    /// overshoot.
    pub fn ramp(keys: Vec<RampKey>) -> Result<Self, String> {
        let Some(first) = keys.first() else {
            return Err("a speed ramp needs a keyframe".to_owned());
        };
        let reverse = first.speed.is_reverse();
        if keys.iter().any(|key| key.speed.is_reverse() != reverse) {
            return Err(
                "a speed ramp keeps one direction: its speeds are all forward or all reversed"
                    .to_owned(),
            );
        }
        if keys.windows(2).any(|pair| pair[0].at >= pair[1].at) {
            return Err("a speed ramp's keyframes need increasing times".to_owned());
        }
        if keys.iter().any(|key| match key.interp {
            Interp::Bezier(curve) => curve.overshoots(),
            _ => false,
        }) {
            return Err(
                "a speed ramp's curves stay within 0 to 1 up: past that the speed would run past its keyframes"
                    .to_owned(),
            );
        }
        if keys.iter().all(|key| key.speed == first.speed) {
            return Ok(Self::Constant(first.speed));
        }
        Ok(Self::Ramp(Ramp { keys }))
    }

    pub fn is_reverse(&self) -> bool {
        match self {
            Self::Constant(ratio) => ratio.is_reverse(),
            Self::Ramp(ramp) => ramp.keys[0].speed.is_reverse(),
        }
    }

    pub fn is_one(&self) -> bool {
        *self == Self::ONE
    }

    /// The one ratio it plays at, unless it ramps.
    pub fn constant(&self) -> Option<Ratio> {
        match self {
            Self::Constant(ratio) => Some(*ratio),
            Self::Ramp(_) => None,
        }
    }

    /// How much source has played by `time` of the clip, which is negative before its start. It
    /// counts up however the clip runs: a reversed clip counts back from its source out.
    pub fn played(&self, time: Time) -> Time {
        match self {
            Self::Constant(ratio) => ratio.source_span(time),
            Self::Ramp(ramp) => {
                Time::from_flicks((ramp.area(flicks(time)) - ramp.area(0.0)).round() as i64)
            }
        }
    }

    /// [`Speed::played`] in seconds, for sound that asks at every sample.
    pub fn played_seconds(&self, seconds: f64) -> f64 {
        let per = crate::time::FLICKS_PER_SECOND as f64;
        match self {
            Self::Constant(ratio) => seconds * ratio.value().abs(),
            Self::Ramp(ramp) => (ramp.area(seconds * per) - ramp.area(0.0)) / per,
        }
    }

    /// The time of the clip by which `span` of source has played: the way back from
    /// [`Speed::played`].
    pub fn time_for(&self, span: Time) -> Time {
        match self {
            Self::Constant(ratio) => ratio.timeline_span(span),
            Self::Ramp(ramp) => {
                let target = ramp.area(0.0) + flicks(span);
                let reach = flicks(span) / ramp.slowest();
                let (mut low, mut high) = if reach >= 0.0 {
                    (0.0, reach + 1.0)
                } else {
                    (reach - 1.0, 0.0)
                };
                for _ in 0..64 {
                    let middle = (low + high) / 2.0;
                    if ramp.area(middle) < target {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                Time::from_flicks(((low + high) / 2.0).round() as i64)
            }
        }
    }

    /// The source played over `[from, to)`, rounded up, for handles that must not fall short.
    pub fn played_covering(&self, from: Time, to: Time) -> Time {
        match self {
            Self::Constant(ratio) => ratio.source_span_covering(to - from),
            Self::Ramp(_) => self.played(to) - self.played(from) + Time::from_flicks(1),
        }
    }

    /// The speed at `time`, without its sign.
    pub fn at(&self, time: Time) -> f64 {
        match self {
            Self::Constant(ratio) => ratio.value().abs(),
            Self::Ramp(ramp) => ramp.at(time),
        }
    }

    /// The speed over `duration` from the clip's start taken as one number, without its sign.
    pub fn average(&self, duration: Time) -> f64 {
        match self {
            Self::Constant(ratio) => ratio.value().abs(),
            Self::Ramp(_) if duration.is_positive() => {
                flicks(self.played(duration)) / flicks(duration)
            }
            Self::Ramp(ramp) => ramp.at(Time::ZERO),
        }
    }

    /// Move its keyframes by `delta`, as when a clip's head is trimmed.
    pub fn shift(&mut self, delta: Time) {
        if let Self::Ramp(ramp) = self {
            for key in &mut ramp.keys {
                key.at += delta;
            }
        }
    }
}

impl std::fmt::Display for Speed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Constant(ratio) => ratio.fmt(f),
            Self::Ramp(_) => f.write_str("ramp"),
        }
    }
}

impl From<Ratio> for Speed {
    fn from(ratio: Ratio) -> Self {
        Self::Constant(ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Rate;

    fn frames(n: i64) -> Time {
        Rate::FPS_24.time_of(n)
    }

    fn key(at: i64, speed: &str, interp: Interp) -> RampKey {
        RampKey {
            at: frames(at),
            speed: Ratio::parse(speed).expect("a speed"),
            interp,
        }
    }

    fn slowing(interp: Interp) -> Speed {
        Speed::ramp(vec![key(0, "1", interp), key(24, "0.5", Interp::Linear)]).expect("a ramp")
    }

    #[test]
    fn a_ramp_adds_up_the_area_under_its_speed() {
        // From 1 to 1/2 over 24 frames plays 18 frames of source, then 1/2 holds.
        let speed = slowing(Interp::Linear);
        assert_eq!(speed.played(frames(24)), frames(18));
        assert_eq!(speed.played(frames(48)), frames(30));
        // A quarter of the way in, the speed is 7/8: the area is (1 + 7/8) / 2 × 6 frames.
        assert_eq!(speed.played(frames(6)).flicks(), frames(45).flicks() / 8);
        // Before the first key the first speed holds, and it counts down.
        assert_eq!(speed.played(frames(-4)), frames(-4));
    }

    #[test]
    fn hold_keeps_the_speed_to_the_next_key() {
        let speed = slowing(Interp::Hold);
        assert_eq!(speed.played(frames(24)), frames(24));
        assert_eq!(speed.played(frames(48)), frames(36));
        assert_eq!(speed.at(frames(23)), 1.0);
    }

    #[test]
    fn every_easing_plays_the_area_its_curve_says() {
        // Easing symmetrically from 1 to 1/2 spends as long fast as slow: the same 18 frames.
        assert_eq!(slowing(Interp::Ease).played(frames(24)), frames(18));
        // `ease-in` stays fast longer: 1/3 of the change on average, so 20 frames.
        assert_eq!(slowing(Interp::EaseIn).played(frames(24)), frames(20));
        assert_eq!(slowing(Interp::EaseOut).played(frames(24)), frames(16));
        // A curve drawing a straight line is the same as linear, to within a thousandth of a frame.
        let line = Interp::parse("0.25,0.25,0.75,0.75").expect("a curve");
        let drawn = slowing(line).played(frames(24));
        assert!(
            (drawn - frames(18)).abs().flicks() < frames(1).flicks() / 1000,
            "{drawn:?}"
        );
    }

    #[test]
    fn played_and_time_for_undo_each_other() {
        let ramps = [
            slowing(Interp::Linear),
            slowing(Interp::Ease),
            slowing(Interp::Hold),
            slowing(Interp::parse("0.4,0,0.2,1").expect("a curve")),
            Speed::ramp(vec![
                key(-12, "-2", Interp::Linear),
                key(36, "-1/4", Interp::Linear),
            ])
            .expect("a ramp"),
        ];
        for speed in &ramps {
            for at in [-30, -4, 0, 1, 7, 24, 25, 60] {
                let back = speed.time_for(speed.played(frames(at)));
                assert!(
                    (back - frames(at)).abs().flicks() < 2_000,
                    "{speed:?} at {at}: {back:?}"
                );
            }
        }
    }

    #[test]
    fn speed_is_continuous_and_stays_between_its_keys() {
        let speed = slowing(Interp::Linear);
        assert_eq!(speed.at(frames(12)), 0.75);
        assert_eq!(speed.at(frames(-2)), 1.0);
        assert_eq!(speed.at(frames(40)), 0.5);
        assert_eq!(speed.average(frames(24)), 0.75);
    }

    #[test]
    fn a_trim_moves_the_keys_and_the_curve_stays_where_it_was() {
        let mut trimmed = slowing(Interp::Linear);
        trimmed.shift(frames(-6));
        let original = slowing(Interp::Linear);
        assert_eq!(
            trimmed.played(frames(10)),
            original.played(frames(16)) - original.played(frames(6))
        );
    }

    #[test]
    fn keys_that_cannot_be_a_ramp_are_refused() {
        let line = Interp::Linear;
        assert!(Speed::ramp(Vec::new()).is_err());
        assert!(Speed::ramp(vec![key(0, "1", line), key(12, "-1", line)]).is_err());
        assert!(Speed::ramp(vec![key(12, "1", line), key(0, "2", line)]).is_err());
        let bounce = Interp::parse("0.2,1.8,0.4,1").expect("a curve");
        assert!(Speed::ramp(vec![key(0, "1", bounce), key(12, "2", line)]).is_err());
        // One speed throughout is no ramp.
        assert_eq!(
            Speed::ramp(vec![key(0, "2", line), key(12, "2", line)]),
            Ok(Speed::Constant(Ratio::parse("2").expect("a speed")))
        );
    }

    #[test]
    fn one_ratio_plays_as_it_always_did() {
        let half = Speed::from(Ratio::parse("1/2").expect("a speed"));
        assert_eq!(half.played(frames(24)), frames(12));
        assert_eq!(half.time_for(frames(12)), frames(24));
        assert_eq!(half.played(frames(-24)), frames(-12));
        assert!(!half.is_one());
        assert!(Speed::ONE.is_one());
        assert_eq!(half.to_string(), "0.5");
    }
}
