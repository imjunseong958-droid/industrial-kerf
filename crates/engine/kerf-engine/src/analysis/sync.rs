//! Lining cameras up by their sound: where one recording's sound falls against another's.
//!
//! Sound is cut down to how its loudness rises, at 500 samples a second, which different microphones
//! and gains agree on far better than on the waveform, and the two are cross-correlated with an FFT.
//! The result says how much later a recording began than the first, and how much to trust it.
//!
//! One offset is right for the whole of a recording only where neither clock runs fast and neither
//! recording lost time. So the recording is read again a stretch at a time, each stretch placed on
//! its own, and what the stretches come to is its [`Course`]: one offset, an offset that changes
//! evenly (two clocks drifting apart), or one that steps where a recording stopped and went on. A
//! stretch that is quiet, or that the first has nothing like, or whose sound keeps a beat and so
//! matches itself a beat on, has no offset and is given none.
//!
//! How well it tells these apart has been measured on recordings made for its tests, claps over
//! noise, and on none recorded in a room.

use crate::error::{EngineError, Result};
use crate::process::ffmpeg::{hidden_command, resolve_ffmpeg_bin};
use std::path::Path;

/// Samples of the rise in loudness a second.
const RATE: usize = 500;
/// The sound is read at this rate before that.
const READ_RATE: usize = 8000;
/// How long a stretch placed on its own is, in seconds, where the recording is long enough for
/// four of them.
const STRETCH: f64 = 60.0;
/// The shortest stretch placed on its own: under it too few sounds are left to tell a match by.
const SHORTEST: f64 = 10.0;
/// A stretch is placed where its best match stands twice as high as the next: under that it is
/// quiet, or the first has nothing like it, and nothing is said of it.
const TRUSTED: f64 = 0.5;
/// Offsets this near each other, in seconds, are one offset. Sound takes about that long to cross
/// three metres, so two people speaking at different distances from two microphones move it by as
/// much, and a step or a drift smaller than this is not told from that.
pub const SLACK: f64 = 0.010;
/// The most two clocks drift apart, in seconds a second: a recording conformed between 30 and
/// 29.97 frames a second is off by half of this. A change faster than it between two stretches is
/// a step.
const MOST_DRIFT: f64 = 0.002;
/// How many stretches that match nothing near where they are expected are looked for in the whole
/// of the first, which finds a step of any size and costs a transform as long as the recording.
const SEARCHED_WIDE: usize = 6;
/// Where a step is, is found by short pieces this long, in samples of the rise, half over half.
const PIECE: usize = 2 * RATE;
/// How much better a piece has to fit one side of a step than the other to be counted on it, as a
/// correlation: unrelated pieces of this length come to 0.03 either way.
const FITS_BETTER: f64 = 0.2;

/// Where a recording falls against the first.
#[derive(Clone, Debug, PartialEq)]
pub struct Alignment {
    /// How many seconds into the first recording this one begins, by the whole of it; negative
    /// when it began first.
    pub offset: f64,
    /// How far the match stands above the next best, 0 to 1.
    pub confidence: f64,
    /// How that offset goes over the recording.
    pub course: Course,
}

/// How a recording's offset against the first goes over its length, a stretch at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Course {
    /// The stretches of the recording placed on their own, in order: those that could fall within
    /// the first. A stretch with no offset was quiet, or the first has nothing like it.
    pub stretches: Vec<Stretch>,
    /// How much the offset grows, in seconds a second, between its steps; 0 where nothing says.
    pub drift: f64,
    /// Where the offset steps, in order.
    pub jumps: Vec<Jump>,
}

/// A stretch of a recording placed on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stretch {
    /// The seconds of the recording it runs from and to.
    pub from: f64,
    pub to: f64,
    /// How many seconds into the first recording the recording would begin, by this stretch alone.
    pub offset: Option<f64>,
}

/// A step in a recording's offset: one of the two stopped and went on, or lost some sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jump {
    /// The seconds of the recording it is between, as near as the sound around it says.
    pub after: f64,
    pub before: f64,
    /// The offset before it and the offset after it.
    pub from: f64,
    pub to: f64,
}

/// What a recording's stretches come to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holds {
    /// Fewer than two stretches could be placed, so nothing says whether one offset holds.
    Unknown,
    /// One offset fits every stretch placed, to within [`SLACK`].
    Steady,
    /// The offset changes evenly, by more than [`SLACK`] over what was placed.
    Drift,
    /// The offset steps.
    Jump,
}

impl Stretch {
    /// The second of the recording its offset is taken to be the offset at.
    pub fn middle(&self) -> f64 {
        (self.from + self.to) / 2.0
    }
}

impl Course {
    /// The stretches that were placed.
    pub fn placed(&self) -> impl Iterator<Item = &Stretch> {
        self.stretches
            .iter()
            .filter(|stretch| stretch.offset.is_some())
    }

    pub fn holds(&self) -> Holds {
        if self.ends().is_none() {
            Holds::Unknown
        } else if !self.jumps.is_empty() {
            Holds::Jump
        } else if self.drifts() {
            Holds::Drift
        } else {
            Holds::Steady
        }
    }

    /// Whether the offset drifts by more than [`SLACK`] from the first stretch placed to the
    /// last, its steps aside.
    pub fn drifts(&self) -> bool {
        self.ends().is_some_and(|(first, last)| {
            (self.drift * (last.middle() - first.middle())).abs() > SLACK
        })
    }

    /// The first and the last stretch placed, of two or more.
    pub fn ends(&self) -> Option<(&Stretch, &Stretch)> {
        let mut placed = self.placed();
        let first = placed.next()?;
        Some((first, placed.last()?))
    }

    /// The one offset nearest to every stretch placed, and how far the furthest of them is from
    /// it, in seconds: none where the offset steps, or where nothing says.
    pub fn fit(&self) -> Option<(f64, f64)> {
        if matches!(self.holds(), Holds::Unknown | Holds::Jump) {
            return None;
        }
        let offsets = || self.placed().filter_map(|stretch| stretch.offset);
        let least = offsets().fold(f64::INFINITY, f64::min);
        let most = offsets().fold(f64::NEG_INFINITY, f64::max);
        Some(((least + most) / 2.0, (most - least) / 2.0))
    }
}

/// How each of `paths` falls against the first, which is the first of the answers, at 0.
pub fn align_sound(paths: &[&Path]) -> Result<Vec<Alignment>> {
    let mut rises = Vec::with_capacity(paths.len());
    for path in paths {
        rises.push(rise_of_loudness(&read_mono(path)?));
    }
    let Some((first, others)) = rises.split_first() else {
        return Ok(Vec::new());
    };
    let mut found = vec![Alignment {
        offset: 0.0,
        confidence: 1.0,
        course: Course::default(),
    }];
    found.extend(others.iter().map(|other| line_up(first, other)));
    Ok(found)
}

/// Where `other` falls against `first`, whole and a stretch at a time.
fn line_up(first: &[f32], other: &[f32]) -> Alignment {
    let whole = align(first, other);
    Alignment {
        offset: whole.offset,
        confidence: whole.confidence,
        course: course(first, other, whole),
    }
}

/// Where one run of sound falls in another.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Match {
    /// How many seconds into the first the other begins.
    offset: f64,
    confidence: f64,
}

impl Match {
    fn none() -> Self {
        Self {
            offset: 0.0,
            confidence: 0.0,
        }
    }
}

fn read_mono(path: &Path) -> Result<Vec<f32>> {
    // A sidecar: it reads the whole of the file's sound.
    let output = crate::process::sidecar::output(
        hidden_command(resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(path)
            .args(["-vn", "-ac", "1", "-ar"])
            .arg(READ_RATE.to_string())
            .args(["-f", "f32le", "-"]),
    )
    .map_err(|error| EngineError::msg(format!("ffmpeg could not start: {error}")))?;
    if !output.status.success() {
        return Err(EngineError::msg(format!(
            "{} has no sound to read: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect())
}

/// How fast loudness rises, ups only, around a mean of zero and at one spread.
fn rise_of_loudness(samples: &[f32]) -> Vec<f32> {
    let per = READ_RATE / RATE;
    let loudness: Vec<f32> = samples
        .chunks(per)
        .map(|chunk| chunk.iter().map(|s| s.abs()).sum::<f32>() / chunk.len() as f32)
        .map(|level| (level + 1e-4).ln())
        .collect();
    let mut rise: Vec<f32> = loudness
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).max(0.0))
        .collect();
    let mean = rise.iter().sum::<f32>() / rise.len().max(1) as f32;
    let spread = (rise.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / rise.len().max(1) as f32)
        .sqrt()
        .max(1e-6);
    for value in &mut rise {
        *value = (*value - mean) / spread;
    }
    rise
}

/// An in-place FFT, `invert` for the way back, without its scale. `re` and `im` are a power of two
/// long.
fn fft(re: &mut [f64], im: &mut [f64], invert: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = std::f64::consts::TAU / length as f64 * if invert { 1.0 } else { -1.0 };
        let (step_re, step_im) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(length) {
            let (mut w_re, mut w_im) = (1.0f64, 0.0f64);
            for k in 0..length / 2 {
                let (a, b) = (start + k, start + k + length / 2);
                let (t_re, t_im) = (re[b] * w_re - im[b] * w_im, re[b] * w_im + im[b] * w_re);
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                (w_re, w_im) = (
                    w_re * step_re - w_im * step_im,
                    w_re * step_im + w_im * step_re,
                );
            }
        }
        length <<= 1;
    }
}

/// How far into `first` the start of `other` falls.
fn align(first: &[f32], other: &[f32]) -> Match {
    Correlation::of(first, other).map_or_else(Match::none, |found| found.peak(1))
}

/// How alike two runs of the rise are at each lag of the one against the other.
struct Correlation {
    /// The sum of their products at each lag, those below zero at the far end.
    values: Vec<f64>,
    first: usize,
    other: usize,
}

impl Correlation {
    fn of(first: &[f32], other: &[f32]) -> Option<Self> {
        if first.is_empty() || other.is_empty() {
            return None;
        }
        let n = (first.len() + other.len()).next_power_of_two();
        let load = |values: &[f32]| {
            let mut re = vec![0.0f64; n];
            for (out, value) in re.iter_mut().zip(values) {
                *out = f64::from(*value);
            }
            re
        };
        let (mut a_re, mut a_im) = (load(first), vec![0.0; n]);
        let (mut b_re, mut b_im) = (load(other), vec![0.0; n]);
        fft(&mut a_re, &mut a_im, false);
        fft(&mut b_re, &mut b_im, false);
        let mut c_re = vec![0.0; n];
        let mut c_im = vec![0.0; n];
        for i in 0..n {
            // One by the conjugate of the other.
            c_re[i] = a_re[i] * b_re[i] + a_im[i] * b_im[i];
            c_im[i] = a_im[i] * b_re[i] - a_re[i] * b_im[i];
        }
        fft(&mut c_re, &mut c_im, true);
        Some(Self {
            values: c_re,
            first: first.len(),
            other: other.len(),
        })
    }

    /// The lag a place in `values` is: those from 0 come first, for as long as the first is, which
    /// is past the middle where the other is the shorter, and those below 0 are at the far end.
    fn lag_of(&self, index: usize) -> i64 {
        if index < self.first {
            index as i64
        } else {
            index as i64 - self.values.len() as i64
        }
    }

    /// The lag they are most alike at.
    fn peak(&self, smear: usize) -> Match {
        self.peak_among(smear).0
    }

    /// The lag they are most alike at, and its place in `values`. `smear` is how many lags next
    /// to each other count as one, 1 for each alone: where one clock runs faster than the other
    /// the lag moves over the run, and what would be one peak is spread over that many, each too
    /// low to stand out.
    fn peak_among(&self, smear: usize) -> (Match, usize) {
        let n = self.values.len();
        let spread;
        let values = if smear > 1 {
            spread = spread_over(&self.values, smear);
            &spread
        } else {
            &self.values
        };
        // Lags from `-other.len()` to `first.len()`, weighed by how much they overlap, so that a
        // sliver of overlap at the far end cannot win by chance over the real thing.
        let least = (self.first.min(self.other) / 4).max(RATE / 2) as f64;
        let score = |index: usize| {
            let lag = self.lag_of(index);
            let overlap = (self.first as i64 - lag.max(0)).min(self.other as i64 + lag.min(0));
            values[index] / (overlap.max(1) as f64).max(least).sqrt()
        };
        let best = (0..n)
            .max_by(|a, b| score(*a).total_cmp(&score(*b)))
            .unwrap_or(0);
        // What comes next, away from the peak's own shoulders.
        let clear = RATE / 4 + smear;
        let next = (0..n)
            .filter(|index| self.lag_of(*index).abs_diff(self.lag_of(best)) as usize > clear)
            .map(score)
            .fold(f64::NEG_INFINITY, f64::max);
        let height = score(best);
        let confidence = if height > 0.0 {
            (1.0 - next.max(0.0) / height).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // A parabola through the peak finds it between samples.
        let (before, after) = (values[(best + n - 1) % n], values[(best + 1) % n]);
        let middle = values[best];
        let bend = before - 2.0 * middle + after;
        let shift = if bend.abs() > 1e-9 {
            (0.5 * (before - after) / bend).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let found = Match {
            offset: (self.lag_of(best) as f64 + shift) / RATE as f64,
            confidence,
        };
        (found, best)
    }

    /// The lag they are most alike at where that stands out, in seconds: as it is, or with a few
    /// lags counted as one and then more, the fewest that does, so that a stretch whose clock
    /// drifts is found and one that holds its offset is found as sharply as it was. A peak that a
    /// drift has spread is highest wherever most sounds happen to fall, anywhere along it, so it
    /// is put at its middle, which is the lag at the middle of the run; a peak that is not spread
    /// has its middle within a sample of its top, and keeps its top.
    fn trusted(&self) -> Option<f64> {
        SMEARS.into_iter().find_map(|smear| {
            let (found, index) = self.peak_among(smear);
            if found.confidence < TRUSTED {
                return None;
            }
            let middle = self.middle_around(index);
            Some(if (middle - found.offset).abs() > 1.0 / RATE as f64 {
                middle
            } else {
                found.offset
            })
        })
    }

    /// The middle of the peak around a place in `values`, in seconds: halfway between where it
    /// comes down to half its height either side, with as many lags counted as one as are ever
    /// tried, so that the holes in a peak spread thin are not taken for its ends.
    fn middle_around(&self, index: usize) -> f64 {
        let widest = SMEARS[SMEARS.len() - 1] as i64;
        let values = spread_over(&self.values, widest as usize);
        let n = values.len() as i64;
        let at = |lag: i64| values[lag.rem_euclid(n) as usize];
        let near = self.lag_of(index);
        let top = (near - widest..=near + widest)
            .max_by(|a, b| at(*a).total_cmp(&at(*b)))
            .unwrap_or(near);
        let half = at(top) / 2.0;
        // No further than a drift can spread it.
        let furthest = 4 * widest;
        let (mut low, mut high) = (top, top);
        while top - low < furthest && at(low - 1) >= half {
            low -= 1;
        }
        while high - top < furthest && at(high + 1) >= half {
            high += 1;
        }
        (low + high) as f64 / 2.0 / RATE as f64
    }
}

/// How many lags next to each other are counted as one to find a peak that a drift has spread.
const SMEARS: [usize; 4] = [1, 5, 15, 45];

/// Each value with the `width` around it, an odd number, as their mean, the ends meeting.
fn spread_over(values: &[f64], width: usize) -> Vec<f64> {
    let (n, half) = (values.len(), width / 2);
    let mut sum: f64 = (0..width).map(|k| values[(n + k - half) % n]).sum();
    (0..n)
        .map(|index| {
            let mean = sum / width as f64;
            sum += values[(index + half + 1) % n] - values[(index + n - half) % n];
            mean
        })
        .collect()
}

/// The stretches a recording of `len` samples is placed by: a minute each, a quarter of a
/// recording too short for four of those, and no shorter than the shortest. The last takes what
/// is left over.
fn spans(len: usize) -> Vec<(usize, usize)> {
    let longest = (STRETCH * RATE as f64) as usize;
    let shortest = (SHORTEST * RATE as f64) as usize;
    let length = longest.min(len / 4).max(shortest);
    let count = (len / length).max(1);
    (0..count)
        .map(|index| {
            let to = if index + 1 == count {
                len
            } else {
                (index + 1) * length
            };
            (index * length, to)
        })
        .collect()
}

/// A run of the rise about its own mean, so that a stretch louder than the rest of its recording
/// matches nothing by that alone.
fn levelled(values: &[f32]) -> Vec<f32> {
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter().map(|value| value - mean).collect()
}

/// How the offset of `other` against `first` goes over its length, `whole` being where all of it
/// falls.
fn course(first: &[f32], other: &[f32], whole: Match) -> Course {
    if first.is_empty() || other.is_empty() {
        return Course::default();
    }
    let mut stretches = place_stretches(first, other, whole);
    let (drift, steps) = settle(&mut stretches);
    let jumps = steps
        .into_iter()
        .map(|(before, after)| {
            find_step(first, other, &stretches[before], &stretches[after], drift)
        })
        .collect();
    Course {
        stretches,
        drift,
        jumps,
    }
}

/// The part of `first` a stretch of `other`, its samples `from` to `to`, would fall in were the
/// recording to begin `lag` samples into the first, with a minute either side, and where that part
/// starts: none where the first was recording for less than a quarter of the stretch.
fn near(first: &[f32], from: usize, to: usize, lag: i64) -> Option<(usize, Vec<f32>)> {
    let within = |sample: i64| sample.clamp(0, first.len() as i64) as usize;
    let room = (STRETCH * RATE as f64) as i64;
    let (starts, ends) = (from as i64 + lag, to as i64 + lag);
    if within(ends) - within(starts) < (to - from) / 4 {
        return None;
    }
    let low = within(starts - room);
    Some((low, levelled(&first[low..within(ends + room)])))
}

/// Each stretch of `other` placed in `first` on its own. It is looked for a minute either side of
/// where the stretch before it fell, which follows a drift and a step of up to that, and failing
/// that, for the first few, in the whole of the first.
fn place_stretches(first: &[f32], other: &[f32], whole: Match) -> Vec<Stretch> {
    let seconds = |samples: usize| samples as f64 / RATE as f64;
    let samples = |seconds: f64| (seconds * RATE as f64).round() as i64;
    // How far into the first the recording is expected to begin, in samples.
    let mut expected = (whole.confidence >= TRUSTED).then(|| samples(whole.offset));
    let mut wide = SEARCHED_WIDE;
    let mut stretches = Vec::new();
    for (from, to) in spans(other.len()) {
        let stretch = levelled(&other[from..to]);
        let (mut offset, mut outside) = (None, false);
        if let Some(lag) = expected {
            match near(first, from, to, lag) {
                Some((low, part)) => {
                    offset = Correlation::of(&part, &stretch)
                        .and_then(|found| found.trusted())
                        .map(|found| seconds(low) + found - seconds(from));
                }
                None => outside = true,
            }
        }
        if offset.is_none() && wide > 0 {
            wide -= 1;
            offset = Correlation::of(first, &stretch)
                .and_then(|found| found.trusted())
                .map(|found| found - seconds(from));
        }
        match offset {
            Some(offset) => expected = Some(samples(offset)),
            // Before the first began or after it ended, as far as anything says.
            None if outside => continue,
            None => {}
        }
        stretches.push(Stretch {
            from: seconds(from),
            to: seconds(to),
            offset,
        });
    }
    stretches
}

/// What the stretches placed come to: how fast their offset drifts, and between which of them it
/// steps, each a stretch and the next one placed. A stretch neither neighbour agrees with, while
/// they agree with each other, was misplaced and not a step out and a step back: it loses its
/// offset.
fn settle(stretches: &mut [Stretch]) -> (f64, Vec<(usize, usize)>) {
    // Each stretch placed: which it is, its middle, and its offset.
    type Placed = (usize, f64, f64);
    let placed = |stretches: &[Stretch]| -> Vec<Placed> {
        stretches
            .iter()
            .enumerate()
            .filter_map(|(index, stretch)| Some((index, stretch.middle(), stretch.offset?)))
            .collect()
    };
    // How far apart two offsets are with a drift taken out.
    let apart = |a: &Placed, b: &Placed, drift: f64| ((b.2 - a.2) - drift * (b.1 - a.1)).abs();
    let points = placed(stretches);
    // The drift that most neighbours agree on, and none where as many agree on that. A step
    // between two stretches is no drift, and one neighbour in several does not carry the rest.
    let agreeing = |drift: f64| {
        points
            .windows(2)
            .filter(|pair| apart(&pair[0], &pair[1], drift) <= SLACK)
            .count()
    };
    let (mut drift, mut most) = (0.0, agreeing(0.0));
    for pair in points.windows(2) {
        let rate = (pair[1].2 - pair[0].2) / (pair[1].1 - pair[0].1);
        if rate.abs() <= MOST_DRIFT && agreeing(rate) > most {
            (drift, most) = (rate, agreeing(rate));
        }
    }
    let misplaced: Vec<usize> = points
        .windows(3)
        .filter(|three| {
            apart(&three[0], &three[2], drift) <= SLACK
                && apart(&three[0], &three[1], drift) > SLACK
                && apart(&three[1], &three[2], drift) > SLACK
        })
        .map(|three| three[1].0)
        .collect();
    for index in misplaced {
        stretches[index].offset = None;
    }
    let points = placed(stretches);
    // Where neighbours are further apart than a drift accounts for, after which point placed.
    let steps_at = |drift: f64| -> Vec<usize> {
        points
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| apart(&pair[0], &pair[1], drift) > SLACK)
            .map(|(index, _)| index)
            .collect()
    };
    // One slope through every run between steps, each run at its own height, by least squares:
    // none where no run has two stretches.
    let slope = |steps: &[usize]| -> Option<f64> {
        let (mut over, mut under, mut start) = (0.0, 0.0, 0);
        for end in steps.iter().map(|step| step + 1).chain([points.len()]) {
            let run = &points[start..end];
            let count = run.len().max(1) as f64;
            let at = run.iter().map(|point| point.1).sum::<f64>() / count;
            let offset = run.iter().map(|point| point.2).sum::<f64>() / count;
            for point in run {
                over += (point.1 - at) * (point.2 - offset);
                under += (point.1 - at).powi(2);
            }
            start = end;
        }
        (under > 0.0).then(|| over / under)
    };
    // The slope shows steps a guess at it hid, and they change the slope: once round is enough.
    let mut steps = steps_at(drift);
    for _ in 0..2 {
        drift = slope(&steps).unwrap_or(drift);
        steps = steps_at(drift);
    }
    let between = steps
        .iter()
        .map(|step| (points[*step].0, points[step + 1].0))
        .collect();
    (drift, between)
}

/// Where between two stretches the offset steps from the one's to the other's: every short piece
/// of the recording from the first to the end of the second is held to both offsets, and the step
/// is put where the fewest pieces are on the wrong side of it, between the last piece before it
/// that fits the old offset and the first after it that fits the new. Pieces with nothing to hear
/// fit neither, so a step in a pause is as wide as the pause.
fn find_step(first: &[f32], other: &[f32], before: &Stretch, after: &Stretch, drift: f64) -> Jump {
    let (from, to) = (before.offset.unwrap_or(0.0), after.offset.unwrap_or(0.0));
    let sample = |seconds: f64| (seconds * RATE as f64).round() as usize;
    let seconds = |sample: usize| sample as f64 / RATE as f64;
    let (low, high) = (sample(before.from), sample(after.to).min(other.len()));
    // Which offset the piece at `at` fits: -1 the one before, 1 the one after, 0 neither.
    let side = |at: usize| -> i8 {
        let fit = |stretch: &Stretch, offset: f64| {
            // The offset as the drift has it by here, and a sample or two either way.
            let here = offset + drift * (seconds(at) - stretch.middle());
            let lag = (here * RATE as f64).round() as i64;
            (-2..=2)
                .map(|nudge| likeness(first, other, at, lag + nudge))
                .fold(0.0, f64::max)
        };
        let (old, new) = (fit(before, from), fit(after, to));
        if new - old > FITS_BETTER {
            1
        } else if old - new > FITS_BETTER {
            -1
        } else {
            0
        }
    };
    let pieces: Vec<(usize, i8)> = (low..high.saturating_sub(PIECE))
        .step_by(PIECE / 2)
        .map(|at| (at, side(at)))
        .collect();
    // With the step before every piece, the wrong ones are those that fit the old offset.
    let mut wrong = pieces.iter().filter(|(_, side)| *side < 0).count();
    let (mut fewest, mut split) = (wrong, 0);
    for (index, (_, side)) in pieces.iter().enumerate() {
        match side {
            1 => wrong += 1,
            -1 => wrong -= 1,
            _ => {}
        }
        if wrong < fewest {
            (fewest, split) = (wrong, index + 1);
        }
    }
    let last_old = pieces[..split].iter().rev().find(|(_, side)| *side < 0);
    let first_new = pieces[split..].iter().find(|(_, side)| *side > 0);
    Jump {
        after: seconds(last_old.map_or(low, |(at, _)| *at)),
        before: seconds(first_new.map_or(high, |(at, _)| at + PIECE)),
        from,
        to,
    }
}

/// How alike the piece of `other` from `at` is to the piece of `first` it falls on where the
/// recording begins `lag` samples into the first: their correlation, and 0 where the first has no
/// such piece or either is flat.
fn likeness(first: &[f32], other: &[f32], at: usize, lag: i64) -> f64 {
    let (Some(here), Ok(start)) = (other.get(at..at + PIECE), usize::try_from(at as i64 + lag))
    else {
        return 0.0;
    };
    let Some(there) = first.get(start..start + PIECE) else {
        return 0.0;
    };
    let mean = |values: &[f32]| values.iter().map(|v| f64::from(*v)).sum::<f64>() / PIECE as f64;
    let (mean_here, mean_there) = (mean(here), mean(there));
    let (mut both, mut power_here, mut power_there) = (0.0, 0.0, 0.0);
    for (here, there) in here.iter().zip(there) {
        let (here, there) = (f64::from(*here) - mean_here, f64::from(*there) - mean_there);
        both += here * there;
        power_here += here * here;
        power_there += there * there;
    }
    if power_here > 0.0 && power_there > 0.0 {
        both / (power_here * power_there).sqrt()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claps at irregular times, each a short decaying burst, through a gain and over a noise floor
    /// of its own.
    fn claps(seconds: f64, begin: f64, gain: f32, seed: u32) -> Vec<f32> {
        let times = [
            0.7, 1.9, 2.3, 4.1, 4.4, 6.2, 7.75, 9.1, 11.3, 12.05, 14.6, 17.2, 19.9,
        ];
        heard(&times, seconds, |t| Some(begin + t), gain, seed)
    }

    /// `seconds` of a recording of claps at `times`, sorted and further apart than a clap lasts.
    /// `moment` says which moment of them the recording has at each second of itself, as its own
    /// clock and its own stops have it, and none where it hears only its noise.
    fn heard(
        times: &[f64],
        seconds: f64,
        moment: impl Fn(f64) -> Option<f64>,
        gain: f32,
        seed: u32,
    ) -> Vec<f32> {
        let frames = (seconds * READ_RATE as f64) as usize;
        let mut state = seed | 1;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 1000) as f32 / 1000.0 - 0.5
        };
        (0..frames)
            .map(|i| {
                let mut value = noise() * 0.01;
                if let Some(t) = moment(i as f64 / READ_RATE as f64) {
                    let last = times.partition_point(|at| *at <= t);
                    let since = last
                        .checked_sub(1)
                        .map_or(f64::INFINITY, |at| t - times[at]);
                    if since < 0.15 {
                        let tone = (since * 900.0 * std::f64::consts::TAU).sin();
                        value += (tone * (-since * 30.0).exp()) as f32 * 0.6;
                    }
                }
                value * gain
            })
            .collect()
    }

    /// Ten minutes of claps, 0.35 to 2.55 s apart, from a minute before the moment called 0.
    fn event() -> Vec<f64> {
        let mut state = 0x2545_f491u32;
        let mut at = -60.0;
        let mut times = Vec::new();
        while at < 540.0 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            at += 0.35 + f64::from(state % 1000) / 1000.0 * 2.2;
            times.push(at);
        }
        times
    }

    /// The first recording of the event, five minutes from its moment 0, and another of
    /// `seconds` that has the event's moment `moment(t)` at its own second `t`, lined up.
    fn lined_up(seconds: f64, moment: impl Fn(f64) -> Option<f64>) -> Alignment {
        let times = event();
        let first = rise_of_loudness(&heard(&times, 300.0, Some, 1.0, 11));
        let other = rise_of_loudness(&heard(&times, seconds, moment, 0.4, 23));
        line_up(&first, &other)
    }

    #[test]
    fn clocks_that_drift_apart_are_told_by_how_fast() {
        // Its clock runs slow by one part in 5000, 12 ms a minute: after five minutes it is
        // 60 ms further into the first than it began.
        let found = lined_up(290.0, |t| Some(t * 1.0002 + 3.0));
        let course = &found.course;
        assert_eq!(course.holds(), Holds::Drift, "{course:?}");
        assert!(course.jumps.is_empty(), "{course:?}");
        assert!((course.drift - 0.0002).abs() < 0.00002, "{course:?}");
        // No one offset is right. The stretches are read at their middles, 30 s and 235 s in:
        // the nearest to both is halfway, and each is 21 ms from it.
        let (first, last) = course.ends().unwrap();
        assert!((first.offset.unwrap() - 3.006).abs() < 0.002, "{course:?}");
        assert!((last.offset.unwrap() - 3.047).abs() < 0.002, "{course:?}");
        let (offset, spread) = course.fit().unwrap();
        assert!((offset - 3.0265).abs() < 0.002, "{course:?}");
        assert!((spread - 0.0205).abs() < 0.002, "{course:?}");
        // One conformed from 30 frames a second to 29.97 is slow by one part in 1000, which
        // moves its offset 60 ms inside each minute and leaves the whole of it matching nothing.
        let found = lined_up(290.0, |t| Some(t * 1.001 + 3.0));
        assert!(found.confidence < TRUSTED, "{found:?}");
        let course = &found.course;
        assert_eq!(course.holds(), Holds::Drift, "{course:?}");
        assert_eq!(course.placed().count(), 4, "{course:?}");
        assert!((course.drift - 0.001).abs() < 0.00002, "{course:?}");
        // A drift of a hundredth of that, 0.6 ms a minute, is 2.4 ms over four minutes: it is
        // measured, and one offset holds.
        let found = lined_up(290.0, |t| Some(t * 1.00001 + 3.0));
        let course = &found.course;
        assert_eq!(course.holds(), Holds::Steady, "{course:?}");
        assert!((course.drift - 0.00001).abs() < 0.00001, "{course:?}");
    }

    #[test]
    fn a_recording_that_stopped_and_went_on_is_told_by_where_and_how_far() {
        // It began 5 s into the first, and at 137 s of itself lost half a second of the event.
        let found = lined_up(290.0, |t| Some(if t < 137.0 { t + 5.0 } else { t + 5.5 }));
        let course = &found.course;
        assert_eq!(course.holds(), Holds::Jump, "{course:?}");
        assert_eq!(course.jumps.len(), 1, "{course:?}");
        let jump = course.jumps[0];
        assert!(
            (jump.from - 5.0).abs() < 0.002 && (jump.to - 5.5).abs() < 0.002,
            "{jump:?}"
        );
        assert!(jump.after <= 137.0 && jump.before >= 137.0, "{jump:?}");
        assert!(jump.before - jump.after < 8.0, "{jump:?}");
        // Either side of it the recording holds its offset, and the two are not one.
        assert!(course.drift.abs() < 0.00002, "{course:?}");
        assert_eq!(course.fit(), None);
        // A minute and a half lost is further than a stretch is looked for beside the last, and
        // is found in the whole of the first.
        let found = lined_up(290.0, |t| Some(if t < 137.0 { t + 5.0 } else { t + 95.0 }));
        let course = &found.course;
        assert_eq!(course.jumps.len(), 1, "{course:?}");
        let jump = course.jumps[0];
        assert!((jump.to - jump.from - 90.0).abs() < 0.002, "{jump:?}");
        assert!(jump.after <= 137.0 && jump.before >= 137.0, "{jump:?}");
        // A drift and a step are told apart in one recording.
        let found = lined_up(290.0, |t| {
            Some(t * 1.0001 + if t < 137.0 { 5.0 } else { 5.5 })
        });
        let course = &found.course;
        assert_eq!(course.jumps.len(), 1, "{course:?}");
        assert!((course.drift - 0.0001).abs() < 0.00002, "{course:?}");
        // A step too small to hear as one, a twentieth of a second back, is a step all the same.
        let found = lined_up(290.0, |t| Some(if t < 200.0 { t + 5.0 } else { t + 4.95 }));
        let course = &found.course;
        assert_eq!(course.holds(), Holds::Jump, "{course:?}");
        let jump = course.jumps[0];
        assert!((jump.to - jump.from + 0.05).abs() < 0.002, "{jump:?}");
        assert!(jump.after <= 200.0 && jump.before >= 200.0, "{jump:?}");
    }

    #[test]
    fn sound_that_has_nothing_in_common_is_not_trusted() {
        let first = rise_of_loudness(&claps(20.0, 0.0, 1.0, 11));
        let noise: Vec<f32> = (0..160_000u32)
            .map(|i| ((i.wrapping_mul(2_654_435_761) >> 16) as f32 / 65_536.0 - 0.5) * 0.2)
            .collect();
        let found = align(&first, &rise_of_loudness(&noise));
        assert!(found.confidence < 0.3, "{found:?}");
    }

    #[test]
    fn a_short_recording_late_in_a_long_one_begins_after_its_start() {
        // 25 s from 270 s of the first's five minutes: past the middle of what the transform
        // holds, where a lag was once read as one before the first began.
        let found = lined_up(25.0, |t| Some(t + 270.0));
        assert!((found.offset - 270.0).abs() < 0.002, "{found:?}");
        assert!(found.confidence > 0.5, "{found:?}");
    }
}
