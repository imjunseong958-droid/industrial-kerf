//! Programme loudness to EBU R128 / ITU-R BS.1770: integrated loudness,
//! loudness range, true peak per channel, and the momentary and short-term
//! loudness every 100 ms.
//!
//! The program's sound is mixed by the same FFmpeg graph a render mixes
//! it with (`kerf-core`, sound only), and FFmpeg's `ebur128` filter
//! measures it in one pass. Nothing is encoded or written.
//!
//! A level with no signal behind it (silence, or a window not yet full) is
//! `f64::NEG_INFINITY`, and JSON writes it as `null`.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{EngineError, Result};
use crate::ffmpeg::{resolve_ffmpeg_bin, tail};
use crate::program::{compile_program, ProgramOptions};
use crate::sidecar::{self, CancellationToken};

/// The EBU R128 programme target, in LUFS.
pub const TARGET_LUFS: f64 = -23.0;
/// The EBU R128 maximum true peak, in dBTP.
pub const TRUE_PEAK_LIMIT_DBTP: f64 = -1.0;
/// How far apart the series' samples are, in seconds: `ebur128` measures
/// in 100 ms steps.
pub const SERIES_STEP_SECONDS: f64 = 0.1;

/// FFmpeg writes this for a window with no energy in it
/// (`-0.691 + 10·log10(1e-12)`); anything at or below it is silence.
const SILENT_LUFS: f64 = -120.0;
/// The R128 absolute gate: an integrated loudness here means no block was
/// loud enough to count.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
const KEY: &str = "lavfi.r128.";

/// A programme's loudness, measured the way R128 deliveries are checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Loudness {
    /// Integrated (programme) loudness in LUFS, gated as BS.1770-4 says.
    #[serde(deserialize_with = "level")]
    pub integrated: f64,
    /// Loudness range in LU.
    pub lra: f64,
    /// The highest true peak of the left and right channels, in dBTP. A
    /// mono programme gives its one channel twice.
    #[serde(deserialize_with = "levels")]
    pub true_peak: [f64; 2],
    /// The highest true peak of any channel, in dBTP.
    #[serde(default = "silent", deserialize_with = "level")]
    pub true_peak_max: f64,
    /// The loudest 3 s short-term window, in LUFS.
    #[serde(deserialize_with = "level")]
    pub short_term_max: f64,
    /// The loudest 400 ms momentary window, in LUFS.
    #[serde(deserialize_with = "level")]
    pub momentary_max: f64,
    /// `(t, momentary, short-term)` every [`SERIES_STEP_SECONDS`]: the
    /// loudness of the windows that end at `t` seconds into the programme.
    #[serde(deserialize_with = "series")]
    pub series: Vec<(f64, f64, f64)>,
    /// How long the measured programme is, in seconds.
    #[serde(default)]
    pub duration: f64,
}

impl Loudness {
    /// When the loudest short-term window ends, in seconds.
    pub fn short_term_max_at(&self) -> Option<f64> {
        loudest(&self.series, |sample| sample.2)
    }

    /// When the loudest momentary window ends, in seconds.
    pub fn momentary_max_at(&self) -> Option<f64> {
        loudest(&self.series, |sample| sample.1)
    }

    /// The bridge's `loudness` event fields for this measurement:
    /// `integrated, lra, truePeak, shortTermMax, momentaryMax, series,
    /// target, limit`, with silence as `null`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        value["target"] = TARGET_LUFS.into();
        value["limit"] = TRUE_PEAK_LIMIT_DBTP.into();
        value
    }
}

fn loudest(series: &[(f64, f64, f64)], level: impl Fn(&(f64, f64, f64)) -> f64) -> Option<f64> {
    series
        .iter()
        .filter(|sample| level(sample).is_finite())
        .rev()
        .max_by(|a, b| level(a).total_cmp(&level(b)))
        .map(|sample| sample.0)
}

fn silent() -> f64 {
    f64::NEG_INFINITY
}

/// A level JSON wrote as `null` reads back as silence.
fn level<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NEG_INFINITY))
}

fn levels<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<[f64; 2], D::Error> {
    let [left, right] = <[Option<f64>; 2]>::deserialize(deserializer)?;
    Ok([
        left.unwrap_or(f64::NEG_INFINITY),
        right.unwrap_or(f64::NEG_INFINITY),
    ])
}

type Sample = (f64, f64, f64);

fn series<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Sample>, D::Error> {
    Ok(
        Vec::<(f64, Option<f64>, Option<f64>)>::deserialize(deserializer)?
            .into_iter()
            .map(|(at, momentary, short)| {
                (
                    at,
                    momentary.unwrap_or(f64::NEG_INFINITY),
                    short.unwrap_or(f64::NEG_INFINITY),
                )
            })
            .collect(),
    )
}

/// Measure the loudness of `sequence` of `project` (the first one when
/// `None`). Relative media paths resolve against `dir`, the project
/// directory. The FFmpeg pass is killed when `cancellation` is requested.
pub fn measure_program(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    dir: &Path,
    cancellation: &CancellationToken,
) -> Result<Loudness> {
    measure_program_with_progress(project, sequence, dir, cancellation, |_| {})
}

/// [`measure_program`], reporting how much of the programme is measured
/// (0 to 1) as it goes.
pub fn measure_program_with_progress(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    dir: &Path,
    cancellation: &CancellationToken,
    mut on_progress: impl FnMut(f64),
) -> Result<Loudness> {
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let heard = crate::ramps::with_sound(project);
    let compiled = compile_program(
        &heard,
        kerf_model::Catalog::builtin(),
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            ..ProgramOptions::default()
        },
    )?;
    let duration = compiled.duration.seconds();
    let args = measure_args(&compiled.argv)?;
    // The same command over the same files hears the same: what an earlier
    // pass kept of it in the project is the measurement.
    let kept = crate::qc::kept::sound_file(dir, &args);
    let known = kept
        .as_deref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<Loudness>(&text).ok());
    if let Some(loudness) = known {
        on_progress(1.0);
        return Ok(loudness);
    }
    let mut command = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin());
    if dir.is_dir() {
        command.current_dir(dir);
    }
    command.args(&args);
    let mut parser = Parser::default();
    let finished = sidecar::run(command, cancellation, |line| {
        if let Some(at) = parser.line(line) {
            if duration > 0.0 {
                on_progress((at / duration).clamp(0.0, 1.0));
            }
        }
    })?;
    if !finished.status.success() {
        return Err(EngineError::Ffmpeg {
            code: finished.status.code().unwrap_or(-1),
            stderr: format!(
                "loudness measurement failed: {}",
                tail(finished.stderr.trim(), 2000)
            ),
        });
    }
    let loudness = parser.finish(duration);
    if loudness.series.is_empty() && duration > 0.0 {
        return Err(EngineError::msg(
            "loudness measurement produced no samples (does this FFmpeg have the ebur128 filter?)",
        ));
    }
    if let (Some(file), Ok(text)) = (&kept, serde_json::to_string(&loudness)) {
        crate::qc::kept::save_sound(file, &text);
    }
    on_progress(1.0);
    Ok(loudness)
}

/// The loudness of the first sound stream of the media file `src`, as it is, with nothing of a
/// programme around it.
pub fn measure_file(src: &Path, cancellation: &CancellationToken) -> Result<Loudness> {
    let mut command = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin());
    command
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(src)
        .args(["-vn", "-map", "0:a:0"])
        .args(["-af", "ebur128=peak=true:metadata=1,ametadata=mode=print"])
        .args(["-f", "null", "-"]);
    let mut parser = Parser::default();
    let mut duration = 0.0f64;
    let finished = sidecar::run(command, cancellation, |line| {
        if let Some(at) = parser.line(line) {
            duration = duration.max(at);
        }
    })?;
    if !finished.status.success() {
        return Err(EngineError::Ffmpeg {
            code: finished.status.code().unwrap_or(-1),
            stderr: format!(
                "loudness measurement failed: {}",
                tail(finished.stderr.trim(), 2000)
            ),
        });
    }
    Ok(parser.finish(duration))
}

/// The FFmpeg arguments (without the binary) that run the program's
/// sound graph `argv` through `ebur128` into nothing.
fn measure_args(argv: &[String]) -> Result<Vec<String>> {
    let mut args: Vec<String> = argv.iter().skip(1).cloned().collect();
    let graph = args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .map(|index| index + 1)
        .filter(|index| *index < args.len())
        .ok_or_else(|| EngineError::msg("the program has no sound graph"))?;
    let map = args
        .iter()
        .rposition(|arg| arg == "-map")
        .map(|index| index + 1)
        .filter(|index| *index < args.len())
        .ok_or_else(|| EngineError::msg("the program maps no sound"))?;
    let sound = args[map].clone();
    if !(sound.starts_with('[') && sound.ends_with(']')) {
        return Err(EngineError::msg(format!(
            "the program's sound is not a graph output: {sound}"
        )));
    }
    args[graph] = format!(
        "{};{sound}ebur128=peak=true:metadata=1,ametadata=mode=print[r128]",
        args[graph]
    );
    args[map] = "[r128]".into();
    let mut measured = vec!["-hide_banner".to_owned(), "-nostats".to_owned()];
    measured.extend(args);
    measured.extend(["-f", "null", "-"].map(String::from));
    Ok(measured)
}

/// Reads `ametadata` print lines of `ebur128` frames.
#[derive(Debug, Default)]
struct Parser {
    frame: Option<Frame>,
    frames: Vec<Frame>,
}

#[derive(Debug, Clone, Default)]
struct Frame {
    at: f64,
    momentary: Option<f64>,
    short_term: Option<f64>,
    integrated: Option<f64>,
    lra: Option<f64>,
    /// Linear true peaks, by channel.
    peaks: Vec<f64>,
}

impl Parser {
    /// Read one stderr line; the start time of a new frame when it begins one.
    fn line(&mut self, line: &str) -> Option<f64> {
        if let Some(index) = line.find("pts_time:") {
            let at = line[index + "pts_time:".len()..]
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())?;
            if let Some(frame) = self.frame.take() {
                self.frames.push(frame);
            }
            self.frame = Some(Frame {
                at,
                ..Frame::default()
            });
            return Some(at);
        }
        let index = line.find(KEY)?;
        let (key, value) = line[index + KEY.len()..].split_once('=')?;
        let value = value.trim().parse::<f64>().ok()?;
        let frame = self.frame.as_mut()?;
        match key {
            "M" => frame.momentary = Some(value),
            "S" => frame.short_term = Some(value),
            "I" => frame.integrated = Some(value),
            "LRA" => frame.lra = Some(value),
            _ => {
                if let Some(channel) = key
                    .strip_prefix("true_peaks_ch")
                    .and_then(|channel| channel.parse::<usize>().ok())
                {
                    if frame.peaks.len() <= channel {
                        frame.peaks.resize(channel + 1, 0.0);
                    }
                    frame.peaks[channel] = value;
                }
            }
        }
        None
    }

    /// The measurement of a programme `duration` seconds long.
    fn finish(mut self, duration: f64) -> Loudness {
        if let Some(frame) = self.frame.take() {
            self.frames.push(frame);
        }
        let lufs = |value: Option<f64>| match value {
            Some(value) if value > SILENT_LUFS => value,
            _ => f64::NEG_INFINITY,
        };
        let series: Vec<(f64, f64, f64)> = self
            .frames
            .iter()
            .map(|frame| {
                let end = frame.at + SERIES_STEP_SECONDS;
                (
                    round(if duration > 0.0 {
                        end.min(duration)
                    } else {
                        end
                    }),
                    lufs(frame.momentary),
                    lufs(frame.short_term),
                )
            })
            .collect();
        let last = self.frames.last().cloned().unwrap_or_default();
        let integrated = match last.integrated {
            Some(value) if value > ABSOLUTE_GATE_LUFS => value,
            _ => f64::NEG_INFINITY,
        };
        // The peaks are running maxima; take the highest any frame saw.
        let channels = self
            .frames
            .iter()
            .map(|frame| frame.peaks.len())
            .max()
            .unwrap_or(0);
        let peaks: Vec<f64> = (0..channels)
            .map(|channel| {
                self.frames
                    .iter()
                    .filter_map(|frame| frame.peaks.get(channel).copied())
                    .fold(0.0_f64, f64::max)
            })
            .map(dbfs)
            .collect();
        let true_peak = match peaks.as_slice() {
            [] => [f64::NEG_INFINITY; 2],
            [mono] => [*mono; 2],
            [left, right, ..] => [*left, *right],
        };
        let max = |pick: fn(&(f64, f64, f64)) -> f64| {
            series.iter().map(pick).fold(f64::NEG_INFINITY, f64::max)
        };
        Loudness {
            integrated,
            lra: last.lra.unwrap_or(0.0),
            true_peak,
            true_peak_max: peaks.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            short_term_max: max(|sample| sample.2),
            momentary_max: max(|sample| sample.1),
            series,
            duration,
        }
    }
}

/// A linear amplitude in dBFS; silence is `-inf`.
fn dbfs(linear: f64) -> f64 {
    if linear > 0.0 {
        20.0 * linear.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// Seconds to the microsecond, so 0.1 steps print as 0.1, not 0.1000000001.
fn round(seconds: f64) -> f64 {
    (seconds * 1e6).round() / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(at: f64, m: f64, s: f64, i: f64, lra: f64, peaks: [f64; 2]) -> String {
        let prefix = "[Parsed_ametadata_9 @ 0x600]";
        format!(
            "{prefix} frame:0    pts:0       pts_time:{at}\n\
             {prefix} lavfi.r128.M={m:.3}\n\
             {prefix} lavfi.r128.S={s:.3}\n\
             {prefix} lavfi.r128.I={i:.3}\n\
             {prefix} lavfi.r128.LRA={lra:.3}\n\
             {prefix} lavfi.r128.LRA.low=0.000\n\
             {prefix} lavfi.r128.true_peaks_ch0={:.3}\n\
             {prefix} lavfi.r128.true_peaks_ch1={:.3}\n\
             {prefix} lavfi.r128.true_peak=0.5\n",
            peaks[0], peaks[1]
        )
    }

    fn parse(text: &str, duration: f64) -> Loudness {
        let mut parser = Parser::default();
        for line in text.lines() {
            parser.line(line);
        }
        parser.finish(duration)
    }

    #[test]
    fn frames_become_a_series_with_silence_as_minus_infinity() {
        let text = [
            frame(0.0, -120.691, -120.691, -70.0, 0.0, [0.1, 0.05]),
            frame(0.1, -21.0, -120.691, -21.0, 0.0, [0.1, 0.05]),
            frame(0.2, -18.5, -22.0, -20.0, 1.5, [0.5, 0.25]),
            frame(0.3, -30.0, -24.0, -20.4, 2.5, [0.5, 0.25]),
        ]
        .concat();
        let loudness = parse(&text, 0.35);
        assert_eq!(loudness.series.len(), 4);
        assert_eq!(loudness.series[0].0, 0.1);
        assert_eq!(loudness.series[0].1, f64::NEG_INFINITY);
        assert_eq!(
            loudness.series[3].0, 0.35,
            "the last window ends at the end"
        );
        assert_eq!(loudness.integrated, -20.4);
        assert_eq!(loudness.lra, 2.5);
        assert_eq!(loudness.momentary_max, -18.5);
        assert_eq!(loudness.short_term_max, -22.0);
        assert_eq!(loudness.momentary_max_at(), Some(0.3));
        assert!((loudness.true_peak[0] - dbfs(0.5)).abs() < 1e-9);
        assert!((loudness.true_peak[1] - dbfs(0.25)).abs() < 1e-9);
        assert!((loudness.true_peak_max - -6.0206).abs() < 1e-3);
    }

    #[test]
    fn silence_has_no_integrated_loudness_and_json_says_null() {
        let text = [
            frame(0.0, -120.691, -120.691, -70.0, 0.0, [0.0, 0.0]),
            frame(0.1, -120.691, -120.691, -70.0, 0.0, [0.0, 0.0]),
        ]
        .concat();
        let loudness = parse(&text, 0.2);
        assert_eq!(loudness.integrated, f64::NEG_INFINITY);
        assert_eq!(loudness.true_peak, [f64::NEG_INFINITY; 2]);
        let json = loudness.to_json();
        assert!(json["integrated"].is_null());
        assert!(json["truePeak"][0].is_null());
        assert!(json["series"][0][1].is_null());
        assert_eq!(json["target"], -23.0);
        assert_eq!(json["limit"], -1.0);
        let back: Loudness = serde_json::from_value(json).expect("reads back");
        assert_eq!(back, loudness);
    }

    #[test]
    fn mono_peaks_fill_both_sides() {
        let prefix = "[Parsed_ametadata_1 @ 0x1]";
        let text = format!(
            "{prefix} frame:0 pts:0 pts_time:0\n{prefix} lavfi.r128.I=-23.0\n\
             {prefix} lavfi.r128.true_peaks_ch0=1.000\n"
        );
        let loudness = parse(&text, 0.1);
        assert_eq!(loudness.true_peak, [0.0, 0.0]);
    }

    #[test]
    fn the_sound_graph_feeds_ebur128_into_nothing() {
        let argv = [
            "ffmpeg",
            "-y",
            "-t",
            "4",
            "-i",
            "a.wav",
            "-filter_complex",
            "[0:a]anull[audio]",
            "-map",
            "[audio]",
        ]
        .map(String::from);
        let args = measure_args(&argv).expect("args");
        assert_eq!(args[..2], ["-hide_banner", "-nostats"]);
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .expect("graph");
        assert_eq!(
            args[graph + 1],
            "[0:a]anull[audio];[audio]ebur128=peak=true:metadata=1,ametadata=mode=print[r128]"
        );
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "[r128]"));
        assert_eq!(args[args.len() - 3..], ["-f", "null", "-"]);
        assert!(!args.iter().any(|arg| arg == "ffmpeg"));
    }
}
