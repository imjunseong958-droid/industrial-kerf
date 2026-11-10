//! Video scopes as numbers: what a colorist reads off a waveform, a histogram, and a vectorscope,
//! for an agent to grade by measuring rather than by eye. Carried from FilmCraft's scopes
//! (`summary.rs`) with what a person's panel needs left out.
//!
//! A [`Reading`] is of a frame's R'G'B' code values, display-encoded: frames decode to 8-bit sRGB,
//! so nothing above white or below black is there to read. Levels are percent of full scale, as an
//! 8-bit code value / 255 × 100. Chroma is BT.709 Y'CbCr, with magnitudes in percent of 0.5 and
//! angles counter-clockwise from +Cb, as a vectorscope draws them.
//!
//! An HDR sequence is read by its own signal instead ([`read_signal`]): 16-bit PQ or HLG code values in
//! BT.2020, where levels are percent of the signal and chroma is BT.2020 Y'CbCr, with [`Hdr`] saying what
//! that comes to in light.

use kerf_color::{
    hlg_inverse_oetf, pq_eotf, pq_inverse_eotf, rgb_to_ycbcr, Matrix, REFERENCE_WHITE_NITS,
};
use serde::Serialize;

/// Samples read at most, across and down: a 1080p frame is read every fourth pixel both ways.
const MOST_ACROSS: usize = 480;
const MOST_DOWN: usize = 270;

/// The skin tone line of a vectorscope: 123° from +Cb. Skin of every complexion falls near it.
pub const SKIN_TONE_DEG: f64 = 123.0;

/// How far from the skin tone line a color still counts as on it.
const SKIN_TONE_REACH_DEG: f64 = 10.0;

/// Below this magnitude a color is neutral: it has no hue to speak of.
const NEUTRAL_MAGNITUDE: f64 = 5.0;

/// Cells across and down the vectorscope that peaks are read from.
const VECTOR_CELLS: usize = 64;

/// How far a vectorscope reaches in Cb and Cr: 100% targets of every matrix fit.
const VECTOR_EXTENT: f32 = 0.6;

/// How many bands the luma histogram has.
pub const HISTOGRAM_BANDS: usize = 16;

/// The lowest, highest, and mean of a set of levels, in percent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Level {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

/// The share of samples that clip, by channel: at or past the last code value at either end.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Clipped {
    pub black: [f64; 3],
    pub white: [f64; 3],
}

/// The share of samples whose luma falls in each third of the range tonal work thinks in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Zones {
    /// Under 25%.
    pub shadows: f64,
    pub midtones: f64,
    /// Over 75%.
    pub highlights: f64,
}

/// What color the frame holds overall.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chroma {
    /// The mean color's angle and magnitude: a cast, when it is far from zero.
    pub cast_hue_deg: f64,
    pub cast_magnitude: f64,
    /// The mean of each sample's own magnitude: how saturated the frame is.
    pub saturation: f64,
    /// The share of colored samples near the skin tone line.
    pub on_skin_line: f64,
}

/// A dense spot on the vectorscope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peak {
    pub hue_deg: f64,
    pub magnitude: f64,
    /// The share of samples there.
    pub share: f64,
}

/// A frame read as a colorist reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub luma: Level,
    pub red: Level,
    pub green: Level,
    pub blue: Level,
    pub clipped: Clipped,
    pub zones: Zones,
    /// The share of samples in each of [`HISTOGRAM_BANDS`] even bands of luma, darkest first.
    pub histogram: Vec<f64>,
    pub chroma: Chroma,
    /// The densest colors with any saturation, densest first.
    pub peaks: Vec<Peak>,
    /// What an HDR signal comes to in light; none for an SDR frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Hdr>,
}

/// The curve an HDR signal is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalCurve {
    Pq,
    Hlg,
}

/// What a frame of HDR signal holds, in light, as a master is checked.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hdr {
    /// Where SDR white falls on the signal, in percent: levels above it are over white.
    pub reference_white: f64,
    /// The brightest any channel gets, and the mean brightness, in cd/m² (the mean a MaxFALL would see).
    pub peak_nits: f64,
    pub mean_nits: f64,
    /// The share of samples brighter than SDR white.
    pub over_white: f64,
    /// The share of samples with color BT.709 cannot hold.
    pub outside_bt709: f64,
}

/// BT.2020 to BT.709 in light, row by row.
const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641, -0.072_85],
    [-0.124_55, 1.132_9, -0.008_349],
    [-0.018_151, -0.100_579, 1.118_73],
];
/// The display an HLG signal is read for: its peak in cd/m² and its system gamma.
const HLG_PEAK_NITS: f32 = 1000.0;
const HLG_GAMMA: f32 = 1.2;
/// How far below 0 a channel of BT.709 has to go for a color to be past it, in shares of the color's
/// own brightest channel: less than this is only rounding.
const PAST_BT709: f32 = 0.005;

#[derive(Default)]
struct Totals {
    min: f32,
    max: f32,
    sum: f64,
    samples: u64,
}

impl Totals {
    fn add(&mut self, value: f32) {
        if self.samples == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.sum += f64::from(value);
        self.samples += 1;
    }

    fn level(&self) -> Level {
        Level {
            min: rounded(f64::from(self.min) * 100.0),
            max: rounded(f64::from(self.max) * 100.0),
            mean: rounded(self.sum / self.samples.max(1) as f64 * 100.0),
        }
    }
}

/// To 0.01, so that JSON shows `78.43`.
fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// To 0.0001, for a share.
fn share(part: u64, whole: u64) -> f64 {
    (part as f64 / whole.max(1) as f64 * 10_000.0).round() / 10_000.0
}

/// A chroma's angle on the vectorscope, in degrees from 0 to 360.
fn angle_deg(cb: f32, cr: f32) -> f64 {
    f64::from(cr)
        .atan2(f64::from(cb))
        .to_degrees()
        .rem_euclid(360.0)
}

/// How far apart two angles are, in degrees.
fn angle_between(a: f64, b: f64) -> f64 {
    let apart = (a - b).rem_euclid(360.0);
    apart.min(360.0 - apart)
}

/// Read a frame of straight RGBA bytes, `width` × `height`. `None` for an empty frame.
pub fn read(rgba: &[u8], width: usize, height: usize) -> Option<Reading> {
    if rgba.len() < width * height * 4 {
        return None;
    }
    read_samples((width, height), Matrix::Bt709, |x, y| {
        let pixel = &rgba[(y * width + x) * 4..][..3];
        (
            [pixel[0], pixel[1], pixel[2]].map(|code| f32::from(code) / 255.0),
            [pixel[0], pixel[1], pixel[2]].map(|code| code == 0),
            [pixel[0], pixel[1], pixel[2]].map(|code| code == 255),
        )
    })
}

/// Read a frame of an HDR sequence, 16-bit little-endian RGBA of its signal in BT.2020 on `curve`.
/// `None` for an empty frame.
pub fn read_signal(
    rgba: &[u8],
    width: usize,
    height: usize,
    curve: SignalCurve,
) -> Option<Reading> {
    if rgba.len() < width * height * 8 {
        return None;
    }
    let code = |x: usize, y: usize, channel: usize| {
        let at = (y * width + x) * 8 + channel * 2;
        u16::from_le_bytes([rgba[at], rgba[at + 1]])
    };
    let mut reading = read_samples((width, height), Matrix::Bt2020Ncl, |x, y| {
        let codes = [code(x, y, 0), code(x, y, 1), code(x, y, 2)];
        (
            codes.map(|code| f32::from(code) / 65535.0),
            codes.map(|code| code == 0),
            codes.map(|code| code == 65535),
        )
    })?;
    let (step_x, step_y) = sample_steps(width, height);
    let (mut peak, mut brightness, mut over, mut past, mut samples) =
        (0f32, 0f64, 0u64, 0u64, 0u64);
    let weights = [0.2627f32, 0.6780, 0.0593];
    for y in (0..height).step_by(step_y) {
        for x in (0..width).step_by(step_x) {
            let signal = [0, 1, 2].map(|channel| f32::from(code(x, y, channel)) / 65535.0);
            let nits = in_light(signal, curve);
            let luma = nits[0] * weights[0] + nits[1] * weights[1] + nits[2] * weights[2];
            peak = peak.max(nits[0]).max(nits[1]).max(nits[2]);
            brightness += f64::from(luma);
            over += u64::from(luma > REFERENCE_WHITE_NITS as f32);
            let brightest = nits[0].max(nits[1]).max(nits[2]);
            let lowest = BT2020_TO_BT709
                .iter()
                .map(|row| row[0] * nits[0] + row[1] * nits[1] + row[2] * nits[2])
                .fold(f32::INFINITY, f32::min);
            past += u64::from(brightest > 0.0 && lowest < -PAST_BT709 * brightest);
            samples += 1;
        }
    }
    reading.hdr = Some(Hdr {
        reference_white: rounded(f64::from(match curve {
            SignalCurve::Pq => pq_inverse_eotf(REFERENCE_WHITE_NITS as f32 / 10_000.0) * 100.0,
            SignalCurve::Hlg => 75.0,
        })),
        peak_nits: rounded(f64::from(peak)),
        mean_nits: rounded(brightness / samples.max(1) as f64),
        over_white: share(over, samples),
        outside_bt709: share(past, samples),
    });
    Some(reading)
}

/// What a signal in BT.2020 comes to on its display, in cd/m².
fn in_light(signal: [f32; 3], curve: SignalCurve) -> [f32; 3] {
    match curve {
        SignalCurve::Pq => signal.map(|value| pq_eotf(value) * 10_000.0),
        SignalCurve::Hlg => {
            let scene = signal.map(hlg_inverse_oetf);
            let luma = scene[0] * 0.2627 + scene[1] * 0.6780 + scene[2] * 0.0593;
            let system = HLG_PEAK_NITS * luma.max(0.0).powf(HLG_GAMMA - 1.0);
            scene.map(|value| value * system)
        }
    }
}

fn sample_steps(width: usize, height: usize) -> (usize, usize) {
    (
        width.div_ceil(MOST_ACROSS).max(1),
        height.div_ceil(MOST_DOWN).max(1),
    )
}

/// Read a frame `size` across and down, from what `sample` says of its pixels at each place: its code
/// values from 0 to 1, and which channels are at the last code value at either end. `None` for an empty
/// frame.
fn read_samples(
    (width, height): (usize, usize),
    matrix: Matrix,
    sample: impl Fn(usize, usize) -> ([f32; 3], [bool; 3], [bool; 3]),
) -> Option<Reading> {
    if width == 0 || height == 0 {
        return None;
    }
    let (step_x, step_y) = sample_steps(width, height);
    let mut channels: [Totals; 4] = Default::default();
    let mut black = [0u64; 3];
    let mut white = [0u64; 3];
    let mut zones = [0u64; 3];
    let mut bands = [0u64; HISTOGRAM_BANDS];
    let (mut cb_sum, mut cr_sum, mut magnitude_sum) = (0f64, 0f64, 0f64);
    let (mut colored, mut on_skin) = (0u64, 0u64);
    let mut cells = vec![0u32; VECTOR_CELLS * VECTOR_CELLS];
    let mut samples = 0u64;
    for y in (0..height).step_by(step_y) {
        for x in (0..width).step_by(step_x) {
            let ([r, g, b], at_black, at_white) = sample(x, y);
            for channel in 0..3 {
                black[channel] += u64::from(at_black[channel]);
                white[channel] += u64::from(at_white[channel]);
            }
            let [luma, cb, cr] = rgb_to_ycbcr(r, g, b, matrix);
            for (totals, value) in channels.iter_mut().zip([r, g, b, luma]) {
                totals.add(value);
            }
            let zone = match luma {
                luma if luma < 0.25 => 0,
                luma if luma > 0.75 => 2,
                _ => 1,
            };
            zones[zone] += 1;
            let band = ((luma * HISTOGRAM_BANDS as f32) as usize).min(HISTOGRAM_BANDS - 1);
            bands[band] += 1;
            cb_sum += f64::from(cb);
            cr_sum += f64::from(cr);
            let magnitude = f64::from(cb.hypot(cr)) / 0.5 * 100.0;
            magnitude_sum += magnitude;
            if magnitude >= NEUTRAL_MAGNITUDE {
                colored += 1;
                if angle_between(angle_deg(cb, cr), SKIN_TONE_DEG) <= SKIN_TONE_REACH_DEG {
                    on_skin += 1;
                }
                let cell = |value: f32| {
                    (((value + VECTOR_EXTENT) / (2.0 * VECTOR_EXTENT) * VECTOR_CELLS as f32)
                        as usize)
                        .min(VECTOR_CELLS - 1)
                };
                cells[cell(cr) * VECTOR_CELLS + cell(cb)] += 1;
            }
            samples += 1;
        }
    }
    let (cb, cr) = (
        (cb_sum / samples as f64) as f32,
        (cr_sum / samples as f64) as f32,
    );
    let [red, green, blue, luma] = channels.map(|totals| totals.level());
    Some(Reading {
        luma,
        red,
        green,
        blue,
        clipped: Clipped {
            black: black.map(|count| share(count, samples)),
            white: white.map(|count| share(count, samples)),
        },
        zones: Zones {
            shadows: share(zones[0], samples),
            midtones: share(zones[1], samples),
            highlights: share(zones[2], samples),
        },
        histogram: bands.iter().map(|count| share(*count, samples)).collect(),
        chroma: Chroma {
            cast_hue_deg: rounded(angle_deg(cb, cr)),
            cast_magnitude: rounded(f64::from(cb.hypot(cr)) / 0.5 * 100.0),
            saturation: rounded(magnitude_sum / samples as f64),
            on_skin_line: share(on_skin, colored),
        },
        peaks: peaks(&cells, samples),
        hdr: None,
    })
}

/// The densest cells holding at least a hundredth of the samples, densest first, at most five.
fn peaks(cells: &[u32], samples: u64) -> Vec<Peak> {
    let mut dense: Vec<(u32, usize)> = cells
        .iter()
        .enumerate()
        .filter(|(_, count)| u64::from(**count) * 100 >= samples)
        .map(|(cell, count)| (*count, cell))
        .collect();
    dense.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    dense
        .into_iter()
        .take(5)
        .map(|(count, cell)| {
            let center = |index: usize| {
                (index as f32 + 0.5) / VECTOR_CELLS as f32 * (2.0 * VECTOR_EXTENT) - VECTOR_EXTENT
            };
            let (cb, cr) = (center(cell % VECTOR_CELLS), center(cell / VECTOR_CELLS));
            Peak {
                hue_deg: rounded(angle_deg(cb, cr)),
                magnitude: rounded(f64::from(cb.hypot(cr)) / 0.5 * 100.0),
                share: share(u64::from(count), samples),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of 16-bit signal in one color.
    fn signal(width: usize, height: usize, color: [f64; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity(width * height * 8);
        for _ in 0..width * height {
            for value in color {
                out.extend(((value * 65535.0).round() as u16).to_le_bytes());
            }
            out.extend(u16::MAX.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_pq_signal_is_read_as_the_light_it_is() {
        // 0.5807 of PQ is 203 cd/m²; 0.7518 is 1000.
        let white = read_signal(&signal(16, 16, [0.5807; 3]), 16, 16, SignalCurve::Pq).unwrap();
        close(white.luma.mean, 58.07, 0.01);
        let hdr = white.hdr.expect("an HDR reading");
        close(hdr.reference_white, 58.07, 0.02);
        close(hdr.peak_nits, 203.0, 1.0);
        close(hdr.mean_nits, 203.0, 1.0);
        close(hdr.outside_bt709, 0.0, 0.0);
        let bright = read_signal(&signal(16, 16, [0.7518; 3]), 16, 16, SignalCurve::Pq).unwrap();
        let hdr = bright.hdr.unwrap();
        close(hdr.peak_nits, 1000.0, 5.0);
        close(hdr.over_white, 1.0, 0.0);
        close(bright.luma.max, 75.18, 0.01);
        // An 8-bit frame of the same could only have said it was white.
        close(
            read(&filled(16, 16, [255; 3]), 16, 16).unwrap().luma.max,
            100.0,
            0.0,
        );
    }

    #[test]
    fn an_hlg_signal_is_read_for_its_display() {
        // 0.75 of HLG is 203 cd/m² on a 1000 cd/m² display, and all of it is that display's peak.
        let reading = read_signal(&signal(16, 16, [0.75; 3]), 16, 16, SignalCurve::Hlg).unwrap();
        let hdr = reading.hdr.unwrap();
        close(hdr.peak_nits, 203.0, 2.0);
        let peak = read_signal(&signal(16, 16, [1.0; 3]), 16, 16, SignalCurve::Hlg).unwrap();
        close(peak.hdr.unwrap().peak_nits, 1000.0, 1.0);
    }

    #[test]
    fn colors_past_bt709_are_counted_and_those_it_holds_are_not() {
        // BT.2020 green is far past it; BT.709's own green, in BT.2020, sits on its edge.
        let green =
            read_signal(&signal(16, 16, [0.0, 0.58, 0.0]), 16, 16, SignalCurve::Pq).unwrap();
        close(green.hdr.unwrap().outside_bt709, 1.0, 0.0);
        let gray = read_signal(&signal(16, 16, [0.4; 3]), 16, 16, SignalCurve::Pq).unwrap();
        close(gray.hdr.unwrap().outside_bt709, 0.0, 0.0);
    }

    #[test]
    fn an_sdr_reading_says_nothing_of_hdr() {
        let reading = read(&filled(16, 16, [128; 3]), 16, 16).unwrap();
        assert_eq!(reading.hdr, None);
        assert!(read_signal(&[0; 8], 16, 16, SignalCurve::Pq).is_none());
    }

    fn filled(width: usize, height: usize, color: [u8; 3]) -> Vec<u8> {
        [color[0], color[1], color[2], 255].repeat(width * height)
    }

    fn close(value: f64, expected: f64, tolerance: f64) {
        assert!(
            (value - expected).abs() <= tolerance,
            "{value} against {expected}"
        );
    }

    #[test]
    fn a_cast_shows_and_opposite_colors_cancel_in_it() {
        let warm = read(&filled(8, 8, [150, 120, 100]), 8, 8).expect("a reading");
        assert!(warm.chroma.cast_magnitude > 5.0);
        let mut rgba = filled(8, 8, [255, 0, 0]);
        rgba[..8 * 4 * 4].copy_from_slice(&filled(8, 4, [0, 255, 255]));
        let opposite = read(&rgba, 8, 8).expect("a reading");
        close(opposite.chroma.cast_magnitude, 0.0, 0.5);
        // Saturated all the same, which the cast alone would hide.
        assert!(opposite.chroma.saturation > 90.0);
        assert_eq!(opposite.peaks.len(), 2);
    }

    #[test]
    fn an_empty_or_short_frame_reads_as_nothing() {
        assert_eq!(read(&[], 0, 0), None);
        assert_eq!(read(&[0; 8], 4, 4), None);
    }
}
