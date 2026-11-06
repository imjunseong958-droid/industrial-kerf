//! Video scopes as numbers: what a colorist reads off a waveform, a histogram, and a vectorscope,
//! for an agent to grade by measuring rather than by eye. Carried from FilmCraft's scopes
//! (`summary.rs`) with what a person's panel needs left out.
//!
//! A [`Reading`] is of a frame's R'G'B' code values, display-encoded: frames decode to 8-bit sRGB,
//! so nothing above white or below black is there to read. Levels are percent of full scale, as an
//! 8-bit code value / 255 × 100. Chroma is BT.709 Y'CbCr, with magnitudes in percent of 0.5 and
//! angles counter-clockwise from +Cb, as a vectorscope draws them.

use kerf_color::{rgb_to_ycbcr, Matrix};
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
}

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
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return None;
    }
    let (step_x, step_y) = (
        width.div_ceil(MOST_ACROSS).max(1),
        height.div_ceil(MOST_DOWN).max(1),
    );
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
            let pixel = &rgba[(y * width + x) * 4..][..3];
            for channel in 0..3 {
                black[channel] += u64::from(pixel[channel] == 0);
                white[channel] += u64::from(pixel[channel] == 255);
            }
            let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|code| f32::from(code) / 255.0);
            let [luma, cb, cr] = rgb_to_ycbcr(r, g, b, Matrix::Bt709);
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
    fn a_gray_frame_is_its_level_with_no_color() {
        let reading = read(&filled(64, 36, [128, 128, 128]), 64, 36).expect("a reading");
        close(reading.luma.mean, 50.2, 0.05);
        assert_eq!(reading.luma.min, reading.luma.max);
        close(reading.chroma.saturation, 0.0, 0.01);
        close(reading.chroma.cast_magnitude, 0.0, 0.01);
        assert!(reading.peaks.is_empty());
        assert_eq!(reading.zones.midtones, 1.0);
        assert_eq!(reading.clipped.white, [0.0; 3]);
        assert_eq!(reading.clipped.black, [0.0; 3]);
        assert_eq!(reading.histogram[8], 1.0);
        assert_eq!(reading.histogram.iter().sum::<f64>(), 1.0);
    }

    #[test]
    fn clipping_is_counted_by_channel() {
        let reading = read(&filled(8, 8, [255, 0, 128]), 8, 8).expect("a reading");
        assert_eq!(reading.clipped.white, [1.0, 0.0, 0.0]);
        assert_eq!(reading.clipped.black, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn half_black_half_white_falls_in_the_end_zones() {
        let mut rgba = filled(8, 8, [0, 0, 0]);
        rgba[..8 * 4 * 4].copy_from_slice(&filled(8, 4, [255, 255, 255]));
        let reading = read(&rgba, 8, 8).expect("a reading");
        assert_eq!(reading.zones.shadows, 0.5);
        assert_eq!(reading.zones.highlights, 0.5);
        assert_eq!(reading.zones.midtones, 0.0);
        assert_eq!(reading.histogram[0], 0.5);
        assert_eq!(reading.histogram[15], 0.5);
        assert_eq!(reading.luma.min, 0.0);
        assert_eq!(reading.luma.max, 100.0);
    }

    #[test]
    fn primaries_land_where_a_vectorscope_draws_them() {
        // BT.709 100% targets.
        for (color, hue, magnitude) in [
            ([255, 0, 0], 102.9, 102.6),
            ([0, 255, 0], 229.7, 119.1),
            ([0, 0, 255], 354.8, 100.4),
        ] {
            let reading = read(&filled(8, 8, color), 8, 8).expect("a reading");
            close(reading.chroma.cast_hue_deg, hue, 0.5);
            close(reading.chroma.cast_magnitude, magnitude, 0.5);
            close(reading.chroma.saturation, magnitude, 0.5);
            assert_eq!(reading.peaks.len(), 1);
            close(reading.peaks[0].hue_deg, hue, 6.0);
            assert_eq!(reading.peaks[0].share, 1.0);
        }
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
    fn skin_sits_on_the_skin_line_and_sky_does_not() {
        let skin = read(&filled(8, 8, [224, 172, 138]), 8, 8).expect("a reading");
        assert_eq!(skin.chroma.on_skin_line, 1.0);
        let sky = read(&filled(8, 8, [90, 150, 220]), 8, 8).expect("a reading");
        assert_eq!(sky.chroma.on_skin_line, 0.0);
    }

    #[test]
    fn a_large_frame_is_read_in_steps_and_reads_the_same() {
        let small = read(&filled(40, 20, [200, 100, 50]), 40, 20).expect("a reading");
        let large = read(&filled(3840, 2160, [200, 100, 50]), 3840, 2160).expect("a reading");
        assert_eq!(small, large);
    }

    #[test]
    fn an_empty_or_short_frame_reads_as_nothing() {
        assert_eq!(read(&[], 0, 0), None);
        assert_eq!(read(&[0; 8], 4, 4), None);
    }
}
