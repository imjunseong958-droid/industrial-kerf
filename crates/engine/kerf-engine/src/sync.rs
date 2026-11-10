//! Lining cameras up by their sound: where one recording's sound falls against another's.
//!
//! Sound is cut down to how its loudness rises, at 500 samples a second, which different microphones
//! and gains agree on far better than on the waveform, and the two are cross-correlated with an FFT.
//! The result says how much later a recording began than the first, and how much to trust it.

use crate::error::{EngineError, Result};
use crate::ffmpeg::{hidden_command, resolve_ffmpeg_bin};
use std::path::Path;

/// Samples of the rise in loudness a second.
const RATE: usize = 500;
/// The sound is read at this rate before that.
const READ_RATE: usize = 8000;

/// Where a recording falls against the first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// How many seconds into the first recording this one begins; negative when it began first.
    pub offset: f64,
    /// How far the match stands above the next best, 0 to 1.
    pub confidence: f64,
}

/// How each of `paths` falls against the first, which is the first of the answers, at 0.
pub fn align_sound(paths: &[&Path]) -> Result<Vec<Alignment>> {
    let mut rises = Vec::with_capacity(paths.len());
    for path in paths {
        rises.push(rise_of_loudness(&read_mono(path)?));
    }
    Ok(rises
        .iter()
        .map(|rise| {
            rises
                .first()
                .map_or_else(Alignment::none, |first| align(first, rise))
        })
        .collect())
}

impl Alignment {
    fn none() -> Self {
        Self {
            offset: 0.0,
            confidence: 0.0,
        }
    }
}

fn read_mono(path: &Path) -> Result<Vec<f32>> {
    let output = hidden_command(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar"])
        .arg(READ_RATE.to_string())
        .args(["-f", "f32le", "-"])
        .output()
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
        .as_chunks::<4>().0.iter()
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
fn align(first: &[f32], other: &[f32]) -> Alignment {
    if first.is_empty() || other.is_empty() {
        return Alignment::none();
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
    // Lags from `-other.len()` to `first.len()`, weighed by how much they overlap, so that a sliver
    // of overlap at the far end cannot win by chance over the real thing.
    let lag_of = |index: usize| {
        if index < n / 2 {
            index as i64
        } else {
            index as i64 - n as i64
        }
    };
    let least = (first.len().min(other.len()) / 4).max(RATE / 2) as f64;
    let score = |index: usize| {
        let lag = lag_of(index);
        let overlap = (first.len() as i64 - lag.max(0)).min(other.len() as i64 + lag.min(0));
        c_re[index] / (overlap.max(1) as f64).max(least).sqrt()
    };
    let best = (0..n)
        .max_by(|a, b| score(*a).total_cmp(&score(*b)))
        .unwrap_or(0);
    // What comes next, away from the peak's own shoulders.
    let clear = RATE / 4;
    let next = (0..n)
        .filter(|index| lag_of(*index).abs_diff(lag_of(best)) as usize > clear)
        .map(score)
        .fold(f64::NEG_INFINITY, f64::max);
    let height = score(best);
    let confidence = if height > 0.0 {
        (1.0 - next.max(0.0) / height).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // A parabola through the peak finds it between samples.
    let (before, after) = (c_re[(best + n - 1) % n], c_re[(best + 1) % n]);
    let middle = c_re[best];
    let bend = before - 2.0 * middle + after;
    let shift = if bend.abs() > 1e-9 {
        (0.5 * (before - after) / bend).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Alignment {
        offset: (lag_of(best) as f64 + shift) / RATE as f64,
        confidence,
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
                let t = begin + i as f64 / READ_RATE as f64;
                let mut value = noise() * 0.01;
                for at in times {
                    let since = t - at;
                    if (0.0..0.15).contains(&since) {
                        let tone = (since * 900.0 * std::f64::consts::TAU).sin();
                        value += (tone * (-since * 30.0).exp()) as f32 * 0.6;
                    }
                }
                value * gain
            })
            .collect()
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
    fn the_transform_comes_back_and_nothing_breaks_on_nothing() {
        let (mut re, mut im) = (vec![1.0, 2.0, -1.0, 0.5], vec![0.0; 4]);
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (found, wanted) in re.iter().zip([4.0, 8.0, -4.0, 2.0]) {
            assert!((found - wanted).abs() < 1e-9);
        }
        assert_eq!(align(&[], &[1.0]), Alignment::none());
    }
}
