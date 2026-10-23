//! Exact media time.
//!
//! Every time in the kernel is an integer count of flicks, 1/705,600,000 of a
//! second. A flick divides the frame duration of every common film and video
//! rate, including the NTSC 1000/1001 family, and the sample period of 44.1,
//! 48, 96, and 192 kHz audio. Edits never accumulate floating-point drift and a
//! frame boundary is an exact integer.

use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// Largest magnitude accepted from text or conversions: 10,000 hours. Sums of
/// a handful of such values cannot overflow `i64`.
pub const MAX_TIME: Time = Time(10_000 * 3600 * FLICKS_PER_SECOND);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimeError {
    InvalidRate(String),
    InexactRate { num: u32, den: u32 },
    InvalidTimecode { text: String, reason: &'static str },
    DropFrameUnsupported(Rate),
    InvalidRatio(String),
    OutOfRange,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate(text) => write!(f, "invalid frame rate `{text}`"),
            Self::InexactRate { num, den } => write!(
                f,
                "frame rate {num}/{den} does not divide the flick timebase exactly"
            ),
            Self::InvalidTimecode { text, reason } => {
                write!(f, "invalid timecode `{text}`: {reason}")
            }
            Self::DropFrameUnsupported(rate) => {
                write!(f, "drop-frame timecode requires 29.97 or 59.94, not {rate}")
            }
            Self::InvalidRatio(text) => write!(f, "invalid speed `{text}`"),
            Self::OutOfRange => write!(f, "time is outside the supported range of 10,000 hours"),
        }
    }
}

impl std::error::Error for TimeError {}

/// A point or span on a timeline, in flicks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(i64);

impl Time {
    pub const ZERO: Self = Self(0);

    pub const fn from_flicks(flicks: i64) -> Self {
        Self(flicks)
    }

    pub const fn flicks(self) -> i64 {
        self.0
    }

    /// Nearest flick to `seconds`, or `None` when non-finite or out of range.
    pub fn from_seconds(seconds: f64) -> Option<Self> {
        let flicks = (seconds * FLICKS_PER_SECOND as f64).round();
        (flicks.is_finite() && flicks.abs() <= MAX_TIME.0 as f64).then_some(Self(flicks as i64))
    }

    pub fn seconds(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }

    pub const fn in_range(self) -> bool {
        self.0.abs() <= MAX_TIME.0
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .filter(|t| t.in_range())
    }
}

impl Add for Time {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Time {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for Time {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl AddAssign for Time {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl SubAssign for Time {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl fmt::Display for Time {
    /// Seconds for diagnostics. Text formats print timecode or frames instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}s", self.seconds())
    }
}

/// A frame rate as a reduced rational whose frame duration is a whole number
/// of flicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
    num: u32,
    den: u32,
}

impl Rate {
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    pub const FPS_23_976: Self = Self {
        num: 24000,
        den: 1001,
    };
    pub const FPS_25: Self = Self { num: 25, den: 1 };
    pub const FPS_29_97: Self = Self {
        num: 30000,
        den: 1001,
    };
    pub const FPS_30: Self = Self { num: 30, den: 1 };

    pub fn new(num: u32, den: u32) -> Result<Self, TimeError> {
        if num == 0 || den == 0 {
            return Err(TimeError::InvalidRate(format!("{num}/{den}")));
        }
        let divisor = gcd(u64::from(num), u64::from(den)) as u32;
        let (num, den) = (num / divisor, den / divisor);
        if !(FLICKS_PER_SECOND as u64 * u64::from(den)).is_multiple_of(u64::from(num)) {
            return Err(TimeError::InexactRate { num, den });
        }
        Ok(Self { num, den })
    }

    /// Parse `24`, `23.976`, `29.97`, `12.5`, or `24000/1001`.
    ///
    /// Decimal forms within 0.005 of an NTSC rate (n×1000/1001) mean that
    /// rate, so `23.976` and `23.98` are both 24000/1001.
    pub fn parse(text: &str) -> Result<Self, TimeError> {
        let invalid = || TimeError::InvalidRate(text.to_owned());
        if let Some((num, den)) = text.split_once('/') {
            let num = num.parse().map_err(|_| invalid())?;
            let den = den.parse().map_err(|_| invalid())?;
            return Self::new(num, den);
        }
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        if whole.is_empty()
            || fraction.len() > 6
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let scale = 10u64.pow(fraction.len() as u32);
        let digits: u64 = format!("{whole}{fraction}")
            .parse()
            .map_err(|_| invalid())?;
        if digits.is_multiple_of(scale) {
            let fps = u32::try_from(digits / scale).map_err(|_| invalid())?;
            return Self::new(fps, 1);
        }
        let value = digits as f64 / scale as f64;
        if let Some(rate) = Self::near_ntsc(value, 0.005) {
            return Ok(rate);
        }
        let num = u32::try_from(digits).map_err(|_| invalid())?;
        Self::new(num, scale as u32)
    }

    /// The rate nearest to a probed floating-point frame rate.
    pub fn from_fps(fps: f64) -> Result<Self, TimeError> {
        let invalid = || TimeError::InvalidRate(fps.to_string());
        if !fps.is_finite() || fps <= 0.0 || fps > 1_000_000.0 {
            return Err(invalid());
        }
        if (fps - fps.round()).abs() < 1e-6 {
            return Self::new(fps.round() as u32, 1);
        }
        if let Some(rate) = Self::near_ntsc(fps, 0.0006) {
            return Ok(rate);
        }
        Self::new((fps * 1000.0).round() as u32, 1000)
    }

    fn near_ntsc(value: f64, tolerance: f64) -> Option<Self> {
        let nominal = (value * 1.001).round();
        if nominal < 1.0 || (value - nominal * 1000.0 / 1001.0).abs() >= tolerance {
            return None;
        }
        Self::new(nominal as u32 * 1000, 1001).ok()
    }

    pub const fn num(self) -> u32 {
        self.num
    }

    pub const fn den(self) -> u32 {
        self.den
    }

    pub fn fps(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    pub const fn frame_duration(self) -> Time {
        Time(FLICKS_PER_SECOND * self.den as i64 / self.num as i64)
    }

    pub const fn time_of(self, frames: i64) -> Time {
        Time(frames * self.frame_duration().0)
    }

    pub const fn is_aligned(self, time: Time) -> bool {
        time.0 % self.frame_duration().0 == 0
    }

    pub const fn frames_exact(self, time: Time) -> Option<i64> {
        if self.is_aligned(time) {
            Some(time.0 / self.frame_duration().0)
        } else {
            None
        }
    }

    pub const fn frames_floor(self, time: Time) -> i64 {
        time.0.div_euclid(self.frame_duration().0)
    }

    /// Nearest frame; an exact half rounds toward the later frame.
    pub const fn frames_round(self, time: Time) -> i64 {
        let duration = self.frame_duration().0;
        (time.0 + duration / 2).div_euclid(duration)
    }

    pub const fn round(self, time: Time) -> Time {
        self.time_of(self.frames_round(time))
    }

    /// Frames counted per timecode second: 24 for 23.976, 30 for 29.97.
    pub const fn timecode_base(self) -> u32 {
        self.num.div_ceil(self.den)
    }

    pub const fn supports_drop_frame(self) -> bool {
        self.den == 1001 && (self.num == 30000 || self.num == 60000)
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else if (self.den == 1001 && self.num.is_multiple_of(1000)) || 1000 % self.den == 0 {
            let text = format!("{:.3}", self.fps());
            write!(f, "{}", text.trim_end_matches('0').trim_end_matches('.'))
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Format `time` as timecode on `rate`, flooring to the frame.
///
/// Drop-frame uses `;` before the frame field and is valid for 29.97 and
/// 59.94 only; other rates fall back to non-drop.
pub fn format_timecode(time: Time, rate: Rate, drop_frame: bool) -> String {
    let frames = rate.frames_floor(time);
    let base = i64::from(rate.timecode_base());
    let drop = drop_frame && rate.supports_drop_frame();
    let sign = if frames < 0 { "-" } else { "" };
    let mut count = frames.abs();
    if drop {
        let dropped = base / 15;
        let per_ten_minutes = base * 600 - dropped * 9;
        let per_minute = base * 60 - dropped;
        let tens = count / per_ten_minutes;
        let rest = count % per_ten_minutes;
        count += dropped * 9 * tens;
        if rest > dropped {
            count += dropped * ((rest - dropped) / per_minute);
        }
    }
    let ff = count % base;
    let total_seconds = count / base;
    let separator = if drop { ';' } else { ':' };
    format!(
        "{sign}{:02}:{:02}:{:02}{separator}{:02}",
        total_seconds / 3600,
        total_seconds / 60 % 60,
        total_seconds % 60,
        ff
    )
}

/// Parse `HH:MM:SS:FF` or drop-frame `HH:MM:SS;FF` into a frame count on
/// `rate`. Returns the frames and whether the text was drop-frame.
pub fn parse_timecode_frames(text: &str, rate: Rate) -> Result<(i64, bool), TimeError> {
    let invalid = |reason| TimeError::InvalidTimecode {
        text: text.to_owned(),
        reason,
    };
    let (negative, body) = match text.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, text),
    };
    let drop = body.contains(';');
    let fields = body
        .split([':', ';'])
        .map(|field| {
            (!field.is_empty() && field.len() <= 5 && field.bytes().all(|b| b.is_ascii_digit()))
                .then(|| field.parse::<i64>().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid("expected HH:MM:SS:FF"))?;
    let [hh, mm, ss, ff] = fields[..] else {
        return Err(invalid("expected HH:MM:SS:FF"));
    };
    if drop && (body.matches(';').count() != 1 || body.rfind(':') > body.find(';')) {
        return Err(invalid("`;` separates only the frame field"));
    }
    let base = i64::from(rate.timecode_base());
    if mm >= 60 || ss >= 60 {
        return Err(invalid("minutes and seconds must be below 60"));
    }
    if ff >= base {
        return Err(invalid("frame field exceeds the rate"));
    }
    if drop && !rate.supports_drop_frame() {
        return Err(TimeError::DropFrameUnsupported(rate));
    }
    let total_minutes = hh * 60 + mm;
    let mut frames = (total_minutes * 60 + ss) * base + ff;
    if drop {
        let dropped = base / 15;
        if ss == 0 && ff < dropped && mm % 10 != 0 {
            return Err(invalid("drop-frame skips this frame number"));
        }
        frames -= dropped * (total_minutes - total_minutes / 10);
    }
    let frames = if negative { -frames } else { frames };
    if !rate.time_of(frames).in_range() {
        return Err(TimeError::OutOfRange);
    }
    Ok((frames, drop))
}

/// Parse timecode into a time on `rate`.
pub fn parse_timecode(text: &str, rate: Rate) -> Result<(Time, bool), TimeError> {
    let (frames, drop) = parse_timecode_frames(text, rate)?;
    Ok((rate.time_of(frames), drop))
}

/// A playback speed as a reduced rational. Negative plays in reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratio {
    num: i32,
    den: u32,
}

impl Ratio {
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// Speeds from 1% to 10,000% in either direction.
    pub fn new(num: i32, den: u32) -> Result<Self, TimeError> {
        let invalid = || TimeError::InvalidRatio(format!("{num}/{den}"));
        if num == 0 || den == 0 {
            return Err(invalid());
        }
        let divisor = gcd(u64::from(num.unsigned_abs()), u64::from(den));
        let num = num / divisor as i32;
        let den = den / divisor as u32;
        let magnitude = u64::from(num.unsigned_abs());
        if magnitude * 100 < u64::from(den) || magnitude > u64::from(den) * 100 {
            return Err(invalid());
        }
        Ok(Self { num, den })
    }

    /// Parse `2`, `0.5`, `-1`, `1.25`, or `3/2`.
    pub fn parse(text: &str) -> Result<Self, TimeError> {
        let invalid = || TimeError::InvalidRatio(text.to_owned());
        if let Some((num, den)) = text.split_once('/') {
            return Self::new(
                num.parse().map_err(|_| invalid())?,
                den.parse().map_err(|_| invalid())?,
            );
        }
        let (negative, body) = match text.strip_prefix('-') {
            Some(body) => (true, body),
            None => (false, text),
        };
        let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
        if whole.is_empty()
            || fraction.len() > 4
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let den = 10u32.pow(fraction.len() as u32);
        let digits: i32 = format!("{whole}{fraction}")
            .parse()
            .map_err(|_| invalid())?;
        Self::new(if negative { -digits } else { digits }, den)
    }

    pub const fn num(self) -> i32 {
        self.num
    }

    pub const fn den(self) -> u32 {
        self.den
    }

    pub const fn is_reverse(self) -> bool {
        self.num < 0
    }

    pub fn value(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Source time covered by `duration` of timeline at this speed.
    pub fn source_span(self, duration: Time) -> Time {
        scale(
            duration,
            i128::from(self.num.unsigned_abs()),
            i128::from(self.den),
        )
    }

    /// Timeline duration that plays `span` of source at this speed.
    pub fn timeline_span(self, span: Time) -> Time {
        scale(
            span,
            i128::from(self.den),
            i128::from(self.num.unsigned_abs()),
        )
    }

    /// Source time covered by `duration` of timeline, rounded up so that its
    /// `timeline_span` is never shorter than `duration`. Use it for handles:
    /// `source_span` rounds to the nearest flick and can fall one short.
    pub fn source_span_covering(self, duration: Time) -> Time {
        let product = i128::from(duration.0) * i128::from(self.num.unsigned_abs());
        Time(-(-product).div_euclid(i128::from(self.den)) as i64)
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else if 10_000 % self.den == 0 {
            let text = format!("{:.4}", self.value());
            write!(f, "{}", text.trim_end_matches('0').trim_end_matches('.'))
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// `time × num / den`, rounded to the nearest flick (halves away from zero).
fn scale(time: Time, num: i128, den: i128) -> Time {
    let product = i128::from(time.0) * num;
    let half = den / 2;
    let rounded = if product >= 0 {
        (product + half) / den
    } else {
        (product - half) / den
    };
    Time(rounded as i64)
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_standard_rate_has_an_exact_frame() {
        for text in [
            "23.976", "24", "25", "29.97", "30", "47.952", "48", "50", "59.94", "60", "119.88",
            "120", "12.5", "15",
        ] {
            let rate = Rate::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(rate.to_string(), text);
            let one_hour = rate.time_of(i64::from(rate.timecode_base()) * 3600);
            assert!(one_hour.is_positive());
        }
        for samples in [44_100, 48_000, 96_000, 192_000] {
            assert_eq!(FLICKS_PER_SECOND % samples, 0);
        }
    }

    #[test]
    fn ntsc_spellings_normalize() {
        assert_eq!(Rate::parse("23.98").unwrap(), Rate::FPS_23_976);
        assert_eq!(Rate::parse("24000/1001").unwrap(), Rate::FPS_23_976);
        assert_eq!(Rate::from_fps(29.970_029_97).unwrap(), Rate::FPS_29_97);
        assert_eq!(Rate::from_fps(25.0).unwrap(), Rate::FPS_25);
        assert_eq!(Rate::parse("12.5").unwrap(), Rate::new(25, 2).unwrap());
    }

    #[test]
    fn inexact_and_malformed_rates_are_rejected() {
        assert!(matches!(
            Rate::new(37, 1),
            Err(TimeError::InexactRate { .. })
        ));
        assert!(Rate::parse("0").is_err());
        assert!(Rate::parse("abc").is_err());
        assert!(Rate::parse("24/0").is_err());
    }

    #[test]
    fn twenty_three_976_is_exact_over_long_spans() {
        let rate = Rate::FPS_23_976;
        let frames = 24 * 3600 * 3;
        let time = rate.time_of(frames);
        assert_eq!(rate.frames_exact(time), Some(frames));
        assert_eq!(time.flicks(), frames * 29_429_400);
    }

    #[test]
    fn drop_frame_skips_the_documented_numbers() {
        let rate = Rate::FPS_29_97;
        let (before, drop) = parse_timecode_frames("00:00:59;29", rate).unwrap();
        assert!(drop);
        let next = rate.time_of(before + 1);
        assert_eq!(format_timecode(next, rate, true), "00:01:00;02");
        assert!(parse_timecode_frames("00:01:00;00", rate).is_err());
        assert_eq!(
            parse_timecode_frames("00:10:00;00", rate).unwrap().0,
            17_982
        );
        assert_eq!(
            format_timecode(rate.time_of(17_982), rate, true),
            "00:10:00;00"
        );
    }

    #[test]
    fn drop_frame_round_trips_every_frame_of_an_hour() {
        for rate in [Rate::FPS_29_97, Rate::new(60000, 1001).unwrap()] {
            for frame in (0..rate.timecode_base() as i64 * 3600).step_by(7) {
                let text = format_timecode(rate.time_of(frame), rate, true);
                assert_eq!(
                    parse_timecode_frames(&text, rate).unwrap().0,
                    frame,
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn non_drop_timecode_round_trips() {
        let rate = Rate::FPS_23_976;
        let (time, drop) = parse_timecode("01:00:00:00", rate).unwrap();
        assert!(!drop);
        assert_eq!(rate.frames_exact(time), Some(86_400));
        assert_eq!(format_timecode(time, rate, false), "01:00:00:00");
        assert_eq!(
            format_timecode(-rate.time_of(25), rate, false),
            "-00:00:01:01"
        );
        assert!(parse_timecode("00:00:00:24", rate).is_err());
        assert!(matches!(
            parse_timecode("00:00:00;10", Rate::FPS_25),
            Err(TimeError::DropFrameUnsupported(_))
        ));
    }

    #[test]
    fn rounding_snaps_to_the_nearest_frame() {
        let rate = Rate::FPS_24;
        let third = rate.frame_duration().flicks() / 3;
        assert_eq!(rate.frames_round(Time::from_flicks(third)), 0);
        assert_eq!(rate.frames_round(Time::from_flicks(2 * third)), 1);
        assert_eq!(rate.frames_floor(Time::from_flicks(-1)), -1);
    }

    #[test]
    fn speeds_scale_spans_exactly() {
        let double = Ratio::parse("2").unwrap();
        let second = Time::from_flicks(FLICKS_PER_SECOND);
        assert_eq!(double.source_span(second).flicks(), 2 * FLICKS_PER_SECOND);
        assert_eq!(double.timeline_span(second).flicks(), FLICKS_PER_SECOND / 2);
        let reverse = Ratio::parse("-1.25").unwrap();
        assert!(reverse.is_reverse());
        assert_eq!(reverse.to_string(), "-1.25");
        assert_eq!(Ratio::parse("3/2").unwrap().to_string(), "1.5");
        assert!(Ratio::parse("0").is_err());
        assert!(Ratio::parse("1000").is_err());
    }

    #[test]
    fn covering_spans_play_back_at_least_as_long() {
        let six_frames = Rate::FPS_29_97.time_of(6);
        let speed = Ratio::parse("0.37").unwrap();
        // Rounding to the nearest flick loses one here.
        assert!(speed.timeline_span(speed.source_span(six_frames)) < six_frames);
        for speed in ["0.37", "-0.37", "0.3333", "7/13", "2.33", "100", "0.01"] {
            let speed = Ratio::parse(speed).unwrap();
            for frames in 0..48 {
                let duration = Rate::FPS_29_97.time_of(frames);
                let span = speed.source_span_covering(duration);
                assert!(speed.timeline_span(span) >= duration, "{speed} {frames}f");
                assert!(span.flicks() - speed.source_span(duration).flicks() <= 1);
            }
        }
    }
}
