//! What a clip plays of its source at a time of its sequence: the one map every reader of a
//! clip's source goes by.
//!
//! A clip is cut to a span of its sequence, from its start for its duration, and its speed warps
//! that span onto its source from its source in. [`TimeMap::time`] is the span: the clip's own
//! time, which its keyframes, masks and effects are read at. [`TimeMap::source`] is the warp. A
//! clip of a nested sequence maps onto that sequence's own time ([`TimeMap::nested`]), where the
//! clips of that sequence map on in turn, so a time is taken down a chain of clips as a place is
//! taken down a chain of parents.
//!
//! A source position is exact to the flick. The frame of a source showing there
//! ([`TimeMap::frame`]) is worked out before that rounding, so a speed that is a ratio never moves
//! the edge of a frame.

use crate::speed::Speed;
use crate::time::{Rate, Time, FLICKS_PER_SECOND};
use std::borrow::Cow;

/// How a clip of media or of a nested sequence maps the time of its sequence onto its source.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeMap<'a> {
    start: Time,
    duration: Time,
    source_in: Time,
    speed: Cow<'a, Speed>,
}

impl<'a> TimeMap<'a> {
    /// The map of a clip that starts at `start` of its sequence, lasts `duration`, and plays its
    /// source from `source_in` at `speed`.
    pub fn new(start: Time, duration: Time, source_in: Time, speed: &'a Speed) -> Self {
        Self {
            start,
            duration,
            source_in,
            speed: Cow::Borrowed(speed),
        }
    }

    /// The same map, kept apart from the clip it was made of.
    pub fn into_owned(self) -> TimeMap<'static> {
        TimeMap {
            start: self.start,
            duration: self.duration,
            source_in: self.source_in,
            speed: Cow::Owned(self.speed.into_owned()),
        }
    }

    /// The same map with the clip starting at `start`: at zero it maps the clip's own time, and
    /// at where the clip starts in a sequence holding its own, that sequence's.
    pub fn starting(self, start: Time) -> Self {
        Self { start, ..self }
    }

    pub fn start(&self) -> Time {
        self.start
    }

    pub fn duration(&self) -> Time {
        self.duration
    }

    pub fn speed(&self) -> &Speed {
        &self.speed
    }

    pub fn is_reverse(&self) -> bool {
        self.speed.is_reverse()
    }

    /// The clip's own time at `at` of its sequence: how long after its first frame, and negative
    /// before it.
    pub fn time(&self, at: Time) -> Time {
        at - self.start
    }

    /// The source interval `[in, out)` the clip plays.
    pub fn range(&self) -> (Time, Time) {
        (self.source_in, self.out())
    }

    fn out(&self) -> Time {
        self.source_in + self.speed.played(self.duration)
    }

    /// What the clip plays at `at` of its sequence, a time on its media or nested sequence: past
    /// either edge, what it would play. A reversed clip runs back from its source out.
    pub fn source(&self, at: Time) -> Time {
        let span = self.speed.played(self.time(at));
        if self.is_reverse() {
            self.out() - span
        } else {
            self.source_in + span
        }
    }

    /// Where on its sequence the clip plays `source`: past either edge, where it would play it.
    /// The inverse of [`Self::source`].
    pub fn played_at(&self, source: Time) -> Time {
        let offset = if self.is_reverse() {
            self.out() - source
        } else {
            source - self.source_in
        };
        self.start + self.speed.time_for(offset)
    }

    /// The source interval the clip plays over `[from, to)` of its sequence, in source order.
    pub fn window(&self, from: Time, to: Time) -> (Time, Time) {
        let (first, last) = (self.source(from), self.source(to));
        if self.is_reverse() {
            (last, first)
        } else {
            (first, last)
        }
    }

    /// The frame of a source at `rate` the clip shows at `at` of its sequence, counted from the
    /// source's start: the one whose display starts at or before the source position, or, played
    /// backwards, the one showing just before it. Before the source's start it is negative, and
    /// it runs on past its end.
    ///
    /// A reversed clip counts back from its source out, which is a whole flick, by the exact
    /// amount played. Where its speed puts the out a fraction of a flick off, a position that
    /// would be exactly a frame's start can come out just past it, and the frame shown is then
    /// the one after the frame [`Self::source`], rounded, would give.
    pub fn frame(&self, at: Time, rate: Rate) -> i64 {
        let units = |time: Time| i128::from(time.flicks());
        let time = self.time(at);
        // Source flicks played, as a fraction over the speed's denominator: one ratio stays exact,
        // and a ramp is exact to the flick.
        let (played, den) = match self.speed.constant() {
            Some(ratio) => (
                units(time) * i128::from(ratio.num().unsigned_abs()),
                i128::from(ratio.den()),
            ),
            None => (units(self.speed.played(time)), 1),
        };
        let per = i128::from(FLICKS_PER_SECOND) * i128::from(rate.den()) * den;
        let frame = if self.is_reverse() {
            let position = (units(self.out()) * den - played) * i128::from(rate.num());
            -(-position).div_euclid(per) - 1
        } else {
            let position = (units(self.source_in) * den + played) * i128::from(rate.num());
            position.div_euclid(per)
        };
        frame.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// The time of the nested sequence the clip shows at `at` of its own: the start of the frame
    /// of it, at its `rate`, showing at the source position ([`Self::source`]), which is the time
    /// that sequence's own clips map on from. Before its first frame the clip shows what it
    /// starts on, and nothing before the nested sequence's own start.
    pub fn nested(&self, at: Time, rate: Rate) -> Time {
        let source = self.source(at.max(self.start)).flicks();
        let frame = rate.frame_duration().flicks().max(1);
        let index = if self.is_reverse() {
            -(-source).div_euclid(frame) - 1
        } else {
            source.div_euclid(frame)
        };
        rate.time_of(index.max(0))
    }

    /// [`Self::source`] in seconds, from seconds of the clip's own time, for what asks at every
    /// sample or between two frames: as near as an `f64` comes.
    pub fn source_seconds(&self, time: f64) -> f64 {
        let played = self.speed.played_seconds(time);
        if self.is_reverse() {
            // Back from the source out: on from the source in by what is left to play.
            let whole = self.speed.played_seconds(self.duration.seconds());
            self.source_in.seconds() + (whole - played)
        } else {
            self.source_in.seconds() + played
        }
    }
}

/// A clip's map from the time of a sequence above its own: down the clips that hold, one in the
/// next, the sequence the clip is in. Each holder takes the time to a frame of the sequence it
/// holds ([`TimeMap::nested`]), and the clip maps on from there.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeChain<'a> {
    /// The holders from the outermost in, each with the rate of the sequence it holds.
    holders: Vec<(TimeMap<'a>, Rate)>,
    clip: TimeMap<'a>,
}

impl<'a> TimeChain<'a> {
    /// The map of `clip` from the time of the sequence the first of `holders` is in. `holders`
    /// run from the outermost in, each with the rate of the sequence it holds; with none it is
    /// the clip's own map.
    pub fn new(holders: Vec<(TimeMap<'a>, Rate)>, clip: TimeMap<'a>) -> Self {
        Self { holders, clip }
    }

    /// The clip's own map, from the time of the sequence it is in.
    pub fn clip(&self) -> &TimeMap<'a> {
        &self.clip
    }

    /// The time of the clip's own sequence showing at `at` of the outermost.
    pub fn own(&self, at: Time) -> Time {
        self.holders
            .iter()
            .fold(at, |at, (holder, rate)| holder.nested(at, *rate))
    }

    /// What the clip plays at `at` of the outermost sequence ([`TimeMap::source`]).
    pub fn source(&self, at: Time) -> Time {
        self.clip.source(self.own(at))
    }

    /// The frame of a source at `rate` the clip shows at `at` of the outermost sequence
    /// ([`TimeMap::frame`]).
    pub fn frame(&self, at: Time, rate: Rate) -> i64 {
        self.clip.frame(self.own(at), rate)
    }

    /// Where on the outermost sequence the clip plays `source`, were each holder to play the
    /// sequence it holds at any moment and not frame by frame: the way back from
    /// [`Self::source`], to within a frame of each sequence on the way.
    pub fn played_at(&self, source: Time) -> Time {
        self.holders
            .iter()
            .rev()
            .fold(self.clip.played_at(source), |at, (holder, _)| {
                holder.played_at(at)
            })
    }
}
