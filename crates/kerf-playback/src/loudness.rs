//! The program's level as it plays: sample peak and RMS per channel, and
//! loudness as ITU-R BS.1770-4 measures it.
//!
//! Loudness runs each channel through the K-weighting filter (a high shelf
//! for the head, then the RLB high-pass), sums the channels' mean squares,
//! and reads `-0.691 + 10 log10` of the sum in LUFS. EBU Tech 3341 names the
//! two live readings: momentary over the last 400 ms and short-term over the
//! last 3 s, both updated every 100 ms. Neither is gated.

use std::collections::VecDeque;
use std::f64::consts::PI;

/// The lowest level reported, in dBFS or LUFS: digital silence and anything
/// quieter read as this floor rather than minus infinity.
pub const LEVEL_FLOOR_DB: f32 = -120.0;

/// How often the loudness readings update, in hops per second.
const HOPS_PER_SECOND: u32 = 10;
/// Hops in the momentary window (400 ms).
const MOMENTARY_HOPS: usize = 4;
/// Hops in the short-term window (3 s).
const SHORT_TERM_HOPS: usize = 30;

/// One second-order section, in transposed direct form II.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Biquad {
    /// b0, b1, b2.
    b: [f64; 3],
    /// a1, a2, with a0 normalised to 1.
    a: [f64; 2],
    state: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Self {
            b,
            a,
            state: [0.0; 2],
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.state[0];
        self.state[0] = self.b[1] * x - self.a[0] * y + self.state[1];
        self.state[1] = self.b[2] * x - self.a[1] * y;
        y
    }

    /// The magnitude of the section's response at `frequency` Hz.
    #[cfg(test)]
    fn magnitude(&self, frequency: f64, rate: f64) -> f64 {
        let w = 2.0 * PI * frequency / rate;
        // H(z) at z = e^{jw}, with z^-1 = cos w - j sin w.
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            self.b[0] + self.b[1] * c1 + self.b[2] * c2,
            self.b[1] * s1 + self.b[2] * s2,
        );
        let den = (
            1.0 + self.a[0] * c1 + self.a[1] * c2,
            self.a[0] * s1 + self.a[1] * s2,
        );
        (num.0.hypot(num.1)) / (den.0.hypot(den.1))
    }

    #[cfg(test)]
    pub(crate) fn coefficients(&self) -> ([f64; 3], [f64; 2]) {
        (self.b, self.a)
    }
}

/// The BS.1770-4 K-weighting filter for one channel.
///
/// BS.1770 tabulates the coefficients at 48 kHz only. These come from the
/// analogue prototypes those coefficients were designed from (as libebur128
/// derives them), so other rates get the same curve; at 48 kHz they match
/// the tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KWeighting {
    pub(crate) shelf: Biquad,
    pub(crate) highpass: Biquad,
}

impl KWeighting {
    pub(crate) fn new(rate: u32) -> Self {
        let rate = f64::from(rate.max(1));
        // Stage 1: the high shelf that models the head.
        let f0 = 1_681.974_450_955_533;
        let gain = 3.999_843_853_973_347;
        let q = 0.707_175_236_955_419_6;
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad::new(
            [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        );
        // Stage 2: the revised low-frequency B-curve high-pass.
        let f0 = 38.135_470_876_024_44;
        let q = 0.500_327_037_323_877_3;
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let highpass = Biquad::new(
            [1.0, -2.0, 1.0],
            [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        );
        Self { shelf, highpass }
    }

    fn process(&mut self, x: f64) -> f64 {
        self.highpass.process(self.shelf.process(x))
    }

    /// The filter's gain at `frequency` Hz, in dB.
    #[cfg(test)]
    pub(crate) fn gain_db(&self, frequency: f64, rate: u32) -> f64 {
        let rate = f64::from(rate);
        20.0 * (self.shelf.magnitude(frequency, rate) * self.highpass.magnitude(frequency, rate))
            .log10()
    }
}

/// Momentary and short-term loudness of a stream of interleaved samples.
///
/// The channels are summed with a weight of 1, as BS.1770 weighs left,
/// right and centre; the program mix is stereo.
#[derive(Clone, Debug)]
pub(crate) struct LoudnessMeter {
    channels: usize,
    filters: Vec<KWeighting>,
    /// Frames in one 100 ms hop.
    hop: usize,
    /// Frames in the hop being filled, and each channel's sum of squares.
    filled: usize,
    squares: Vec<f64>,
    /// For each finished hop, newest last: the sum over channels of its
    /// K-weighted mean square.
    hops: VecDeque<f64>,
    momentary: Option<f32>,
    short_term: Option<f32>,
}

impl LoudnessMeter {
    pub(crate) fn new(rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            filters: vec![KWeighting::new(rate); channels],
            hop: (rate.max(HOPS_PER_SECOND) / HOPS_PER_SECOND) as usize,
            filled: 0,
            squares: vec![0.0; channels],
            hops: VecDeque::with_capacity(SHORT_TERM_HOPS + 1),
            momentary: None,
            short_term: None,
        }
    }

    /// Measure the next samples, interleaved.
    pub(crate) fn push(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            for ((filter, square), sample) in self
                .filters
                .iter_mut()
                .zip(self.squares.iter_mut())
                .zip(frame)
            {
                let weighted = filter.process(f64::from(*sample));
                *square += weighted * weighted;
            }
            self.filled += 1;
            if self.filled == self.hop {
                self.finish_hop();
            }
        }
    }

    fn finish_hop(&mut self) {
        let hop = self.hop as f64;
        let power = self.squares.iter().map(|square| square / hop).sum();
        self.squares.iter_mut().for_each(|square| *square = 0.0);
        self.filled = 0;
        self.hops.push_back(power);
        if self.hops.len() > SHORT_TERM_HOPS {
            self.hops.pop_front();
        }
        self.momentary = self.window(MOMENTARY_HOPS);
        self.short_term = self.window(SHORT_TERM_HOPS);
    }

    /// The loudness of the last `hops` hops; `None` until that much has
    /// been measured.
    fn window(&self, hops: usize) -> Option<f32> {
        if self.hops.len() < hops {
            return None;
        }
        let power = self.hops.iter().rev().take(hops).sum::<f64>() / hops as f64;
        Some(lufs(power))
    }

    /// Loudness over the last 400 ms, in LUFS.
    pub(crate) fn momentary(&self) -> Option<f32> {
        self.momentary
    }

    /// Loudness over the last 3 s, in LUFS.
    pub(crate) fn short_term(&self) -> Option<f32> {
        self.short_term
    }
}

/// The loudness of a K-weighted channel sum of mean squares.
fn lufs(power: f64) -> f32 {
    if power > 0.0 {
        ((-0.691 + 10.0 * power.log10()) as f32).max(LEVEL_FLOOR_DB)
    } else {
        LEVEL_FLOOR_DB
    }
}

/// A linear amplitude (1.0 is a full-scale sample) in dBFS, never under
/// the floor.
fn dbfs(amplitude: f64) -> f32 {
    if amplitude > 0.0 {
        ((20.0 * amplitude.log10()) as f32).max(LEVEL_FLOOR_DB)
    } else {
        LEVEL_FLOOR_DB
    }
}

/// The sample peak and the RMS of each of the first two channels of
/// `samples`, interleaved, in dBFS. A mono stream reads the same on both.
/// RMS is unweighted: a full-scale sine reads −3.01 dBFS.
pub(crate) fn peak_and_rms(samples: &[f32], channels: usize) -> ([f32; 2], [f32; 2]) {
    let channels = channels.max(1);
    let mut peak = [0.0_f64; 2];
    let mut squares = [0.0_f64; 2];
    let mut frames = 0_usize;
    for frame in samples.chunks_exact(channels) {
        for side in 0..2 {
            let sample = f64::from(frame[side.min(channels - 1)]);
            peak[side] = peak[side].max(sample.abs());
            squares[side] += sample * sample;
        }
        frames += 1;
    }
    let frames = frames.max(1) as f64;
    (
        [dbfs(peak[0]), dbfs(peak[1])],
        [
            dbfs((squares[0] / frames).sqrt()),
            dbfs((squares[1] / frames).sqrt()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{peak_and_rms, KWeighting, LoudnessMeter, LEVEL_FLOOR_DB};
    use std::f64::consts::PI;

    /// `seconds` of a sine at `frequency` Hz and peak `amplitude`, on both
    /// channels.
    fn stereo_sine(frequency: f64, amplitude: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let frames = (f64::from(rate) * seconds) as usize;
        (0..frames)
            .flat_map(|n| {
                let value =
                    (amplitude * (2.0 * PI * frequency * n as f64 / f64::from(rate)).sin()) as f32;
                [value, value]
            })
            .collect()
    }

    fn close(actual: f64, expected: f64, tolerance: f64) -> bool {
        (actual - expected).abs() <= tolerance
    }

    #[test]
    fn k_weighting_matches_the_bs1770_tables_at_48_khz() {
        let filter = KWeighting::new(48_000);
        let (b, a) = filter.shelf.coefficients();
        let expected_b = [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
        ];
        let expected_a = [-1.690_659_293_182_41, 0.732_480_774_215_85];
        for (actual, expected) in b.iter().zip(expected_b).chain(a.iter().zip(expected_a)) {
            assert!(
                close(*actual, expected, 1e-8),
                "{actual} against {expected}"
            );
        }
        let (b, a) = filter.highpass.coefficients();
        assert_eq!(b, [1.0, -2.0, 1.0]);
        let expected_a = [-1.990_047_454_833_98, 0.990_072_250_366_21];
        for (actual, expected) in a.iter().zip(expected_a) {
            assert!(
                close(*actual, expected, 1e-8),
                "{actual} against {expected}"
            );
        }
    }

    #[test]
    fn k_weighting_has_the_bs1770_response() {
        for rate in [44_100, 48_000, 96_000] {
            let filter = KWeighting::new(rate);
            // The −0.691 in the loudness formula takes out the filter's gain
            // at 997 Hz, so a 997 Hz tone reads its own level.
            let at_1k = filter.gain_db(997.0, rate);
            assert!(
                close(at_1k - 0.691, 0.0, 0.1),
                "{rate} Hz: {at_1k} dB at 1 kHz"
            );
            // The head's high shelf: about +4 dB well above 2 kHz. At 2 kHz
            // the curve is still on its way up: the BS.1770 48 kHz filters
            // give +3.07 dB there, not the shelf's +4.
            let shelf = filter.gain_db(10_000.0, rate);
            assert!(close(shelf, 4.0, 0.3), "{rate} Hz: {shelf} dB at 10 kHz");
            let at_2k = filter.gain_db(2_000.0, rate);
            assert!(close(at_2k, 3.07, 0.1), "{rate} Hz: {at_2k} dB at 2 kHz");
            assert!(
                at_1k < at_2k && at_2k < shelf,
                "{rate} Hz: {at_2k} dB at 2 kHz"
            );
            // The RLB high-pass cuts the lows.
            let low = filter.gain_db(20.0, rate);
            assert!(low < -10.0, "{rate} Hz: {low} dB at 20 Hz");
        }
    }

    #[test]
    fn a_stereo_sine_at_minus_20_dbfs_reads_minus_20_lufs() {
        for rate in [44_100, 48_000] {
            let mut meter = LoudnessMeter::new(rate, 2);
            let amplitude = 10f64.powf(-20.0 / 20.0);
            meter.push(&stereo_sine(1_000.0, amplitude, rate, 0.39));
            assert_eq!(meter.momentary(), None, "not 400 ms yet");
            meter.push(&stereo_sine(1_000.0, amplitude, rate, 3.2));
            let momentary = meter.momentary().expect("momentary after 400 ms");
            let short_term = meter.short_term().expect("short-term after 3 s");
            assert!(
                close(momentary.into(), -20.0, 0.3),
                "{rate} Hz: {momentary}"
            );
            assert!(
                close(short_term.into(), -20.0, 0.3),
                "{rate} Hz: {short_term}"
            );
        }
    }

    #[test]
    fn short_term_waits_for_its_three_seconds_and_follows_the_last_ones() {
        let rate = 48_000;
        let mut meter = LoudnessMeter::new(rate, 2);
        meter.push(&stereo_sine(1_000.0, 10f64.powf(-20.0 / 20.0), rate, 2.9));
        assert!(meter.momentary().is_some());
        assert_eq!(meter.short_term(), None);
        // Three seconds of a tone 10 dB quieter replace the window.
        meter.push(&stereo_sine(1_000.0, 10f64.powf(-30.0 / 20.0), rate, 3.0));
        let short_term = meter.short_term().expect("short-term");
        assert!(close(short_term.into(), -30.0, 0.3), "{short_term}");
    }

    #[test]
    fn silence_reads_the_floor() {
        let mut meter = LoudnessMeter::new(48_000, 2);
        meter.push(&vec![0.0; 48_000 * 2]);
        assert_eq!(meter.momentary(), Some(LEVEL_FLOOR_DB));
        assert_eq!(
            peak_and_rms(&[0.0; 8], 2),
            ([LEVEL_FLOOR_DB; 2], [LEVEL_FLOOR_DB; 2])
        );
    }

    #[test]
    fn peak_and_rms_of_a_known_buffer() {
        // Left: a square wave at half scale. Right: a full-scale sine.
        let frames = 4_800;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|n| {
                let left = if n % 2 == 0 { 0.5 } else { -0.5 };
                let right = (2.0 * PI * 1_000.0 * n as f64 / 48_000.0).sin() as f32;
                [left, right]
            })
            .collect();
        let (peak, rms) = peak_and_rms(&samples, 2);
        let half = (20.0 * 0.5f64.log10()) as f32;
        assert!((peak[0] - half).abs() < 1e-4, "{peak:?}");
        assert!((rms[0] - half).abs() < 1e-4, "{rms:?}");
        assert!(peak[1].abs() < 0.01, "{peak:?}");
        assert!((rms[1] + 3.0103).abs() < 0.01, "{rms:?}");
        // Mono reads the same on both sides.
        let (peak, rms) = peak_and_rms(&[0.25, -0.25], 1);
        assert_eq!(peak[0], peak[1]);
        assert_eq!(rms[0], rms[1]);
    }
}
