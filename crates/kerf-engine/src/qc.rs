//! A picture QC pass over a program: broadcast-legal gamut, luma levels,
//! photosensitive flashing, and black.
//!
//! The compositor draws every frame of the program at its own rate, 360
//! pixels tall (as a draft render is), and FFmpeg converts each one to
//! 10-bit 4:2:2 Rec.709 limited range the way a master is encoded, then
//! measures it with `signalstats` and `blackdetect` in one pass. Nothing is
//! encoded or written.
//!
//! Levels are 10-bit code values: legal Y′ is 64 to 940.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use kerf_compose::{ComposeOptions, Compositor};
use kerf_model::{Rate, Time};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, Result};
use crate::ffmpeg::{resolve_ffmpeg_bin, tail};
use crate::program::rate_arg;
use crate::sidecar::CancellationToken;

/// Frames are measured this tall (or at their own size when smaller).
pub const QC_HEIGHT: u32 = 360;
/// Legal Y′ in 10-bit code values.
pub const LUMA_LEGAL_MIN: f64 = 64.0;
pub const LUMA_LEGAL_MAX: f64 = 940.0;
/// A frame is out of gamut when more than this share of its pixels lie
/// outside the broadcast range (`signalstats` BRNG).
pub const OUT_OF_RANGE_LIMIT: f64 = 0.01;
/// A flash is a change of mean luminance of at least this share of the
/// legal range, in one direction then the other.
pub const FLASH_STEP: f64 = 0.10;
/// More transitions than this within one second is flashing.
pub const FLASH_TRANSITIONS_LIMIT: usize = 3;
pub const FLASH_WINDOW_SECONDS: f64 = 1.0;
/// Black shorter than this is a cut, not a segment.
pub const BLACK_MIN_SECONDS: f64 = 0.5;
/// A pixel darker than this share of the legal range counts as black.
const BLACK_PIXEL: f64 = 0.10;
/// Frames drawn at once.
const DRAWERS: usize = 3;
/// How much of FFmpeg's other output is kept for an error message.
const TAIL_BYTES: usize = 4096;

pub const CODE_OUT_OF_RANGE: &str = "picture.out_of_range";
pub const CODE_LUMA_LEVELS: &str = "picture.luma_levels";
pub const CODE_FLASHING: &str = "picture.flashing";
pub const CODE_BLACK: &str = "picture.black";

/// `[start, end)` seconds into the program.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QcRange {
    pub start: f64,
    pub end: f64,
}

/// One check's verdict and the value it was judged on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureCheck {
    pub code: String,
    pub name: String,
    pub passed: bool,
    /// The worst value found: the out-of-range share in percent, the
    /// furthest Y′ from legal, or the number of flashing or black stretches.
    pub value: f64,
    pub limit: f64,
    pub unit: String,
    /// When the worst value was found, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    /// Where the check fails, merged across adjacent frames.
    #[serde(default)]
    pub ranges: Vec<QcRange>,
}

/// What [`picture_qc`] finds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureQc {
    pub duration: f64,
    pub frames: usize,
    /// Gamut Rec.709: pixels outside the broadcast range.
    pub out_of_range: PictureCheck,
    /// Y′ outside 64–940.
    pub luma_levels: PictureCheck,
    /// More than three luminance transitions of 10% within a second.
    pub flashing: PictureCheck,
    /// Stretches of black of at least [`BLACK_MIN_SECONDS`].
    pub black: PictureCheck,
    /// The darkest and brightest Y′ of any frame, in 10-bit code values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma_max: Option<f64>,
    /// The most luminance transitions found in any one second.
    #[serde(default)]
    pub flash_transitions_max: usize,
}

impl PictureQc {
    /// The four checks, in the order a QC report lists them.
    pub fn checks(&self) -> [&PictureCheck; 4] {
        [
            &self.out_of_range,
            &self.luma_levels,
            &self.flashing,
            &self.black,
        ]
    }

    pub fn passed(&self) -> bool {
        self.checks().iter().all(|check| check.passed)
    }
}

/// One measured frame, in 10-bit code values except `brng`, a share.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    pub at: f64,
    pub ymin: f64,
    pub ymax: f64,
    pub yavg: f64,
    pub brng: f64,
}

/// Run the picture QC pass over `sequence` of `project` (the first one
/// when `None`). FFmpeg runs in `dir`, the project directory. The pass is
/// stopped when `cancellation` is requested.
pub fn picture_qc(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    dir: &Path,
    cancellation: &CancellationToken,
) -> Result<PictureQc> {
    picture_qc_with_progress(project, sequence, dir, cancellation, |_| {})
}

/// [`picture_qc`], reporting the share of frames measured (0 to 1).
pub fn picture_qc_with_progress(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    dir: &Path,
    cancellation: &CancellationToken,
    mut on_progress: impl FnMut(f64),
) -> Result<PictureQc> {
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let target = crate::program::program_sequence(project, sequence)?;
    let rate = target.rate;
    let duration = target.duration();
    if !duration.is_positive() {
        return Err(EngineError::msg(format!(
            "sequence `{}` is empty",
            target.id
        )));
    }
    let compositor = Compositor::new(
        project,
        kerf_model::Catalog::builtin(),
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
        },
    )?;
    let canvas = compositor.size();
    let scale = (f64::from(QC_HEIGHT) / f64::from(canvas.1.max(1))).min(1.0);
    let size = (
        ((f64::from(canvas.0) * scale).round() as u32).max(2),
        ((f64::from(canvas.1) * scale).round() as u32).max(2),
    );
    let times = frame_times(rate, duration);
    let args = qc_args(size, rate);
    let mut command = Command::new(resolve_ffmpeg_bin());
    if dir.is_dir() {
        command.current_dir(dir);
    }
    let mut child = command
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| EngineError::msg(format!("ffmpeg could not start: {error}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    // Parse as FFmpeg writes, so the log of a long program is never held.
    let reader = thread::spawn(move || {
        let mut parser = StatsParser::default();
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::io::Result::ok)
        {
            parser.line(&line);
        }
        parser
    });
    let total = times.len();
    let mut written = 0;
    let drawn = crate::render::draw_in_order(
        total,
        DRAWERS,
        |index| compositor.frame(times[index], scale),
        |frame| {
            if (frame.width, frame.height) != size {
                return Err(EngineError::msg(format!(
                    "a frame came out {}x{}, not {}x{}",
                    frame.width, frame.height, size.0, size.1
                )));
            }
            stdin.write_all(&frame.rgba)?;
            written += 1;
            on_progress(written as f64 / total as f64);
            Ok(())
        },
        cancellation,
    );
    drop(stdin);
    if drawn.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let parser = reader.join().unwrap_or_default();
    match drawn {
        Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
        Err(EngineError::Io(_)) | Ok(()) if !status.success() => {
            return Err(EngineError::Ffmpeg {
                code: status.code().unwrap_or(-1),
                stderr: format!("picture QC failed: {}", tail(parser.tail.trim(), 2000)),
            })
        }
        Err(error) => return Err(error),
        Ok(()) => {}
    }
    if parser.frames.is_empty() {
        return Err(EngineError::msg(
            "picture QC measured no frames (does this FFmpeg have signalstats?)",
        ));
    }
    Ok(analyze(
        &parser.frames,
        &parser.black,
        rate.frame_duration().seconds(),
        duration.seconds(),
    ))
}

/// The time of each frame of a program `duration` long at `rate`.
fn frame_times(rate: Rate, duration: Time) -> Vec<Time> {
    let frame = rate.frame_duration().flicks();
    let count = -(-duration.flicks()).div_euclid(frame);
    (0..count.max(1)).map(|index| rate.time_of(index)).collect()
}

/// FFmpeg's arguments: RGBA frames of `size` at `rate` on stdin, measured
/// as 10-bit Rec.709 limited range, into nothing.
fn qc_args(size: (u32, u32), rate: Rate) -> Vec<String> {
    let stat = |key: &str| format!("metadata=mode=print:key=lavfi.signalstats.{key}");
    let graph = [
        "scale=w=trunc(iw/2)*2:h=trunc(ih/2)*2:out_color_matrix=bt709:out_range=tv".to_owned(),
        "format=yuv422p10le".to_owned(),
        "signalstats=stat=brng".to_owned(),
        stat("YMIN"),
        stat("YMAX"),
        stat("YAVG"),
        stat("BRNG"),
        format!("blackdetect=d={BLACK_MIN_SECONDS}:pix_th={BLACK_PIXEL}"),
    ]
    .join(",");
    [
        "-hide_banner",
        "-nostats",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &format!("{}x{}", size.0, size.1),
        "-framerate",
        &rate_arg(rate),
        "-i",
        "pipe:0",
        "-vf",
        &graph,
        "-f",
        "null",
        "-",
    ]
    .map(str::to_owned)
    .into()
}

/// Reads `metadata` print lines and `blackdetect` lines.
#[derive(Debug, Default)]
struct StatsParser {
    frames: Vec<FrameStats>,
    black: Vec<QcRange>,
    at: Option<f64>,
    /// The last lines that were neither, for an error message.
    tail: String,
}

impl StatsParser {
    fn line(&mut self, line: &str) {
        if let Some(index) = line.find("pts_time:") {
            self.at = line[index + "pts_time:".len()..]
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok());
            return;
        }
        if let Some(index) = line.find("lavfi.signalstats.") {
            let Some((key, value)) = line[index + "lavfi.signalstats.".len()..].split_once('=')
            else {
                return;
            };
            let (Some(at), Ok(value)) = (self.at, value.trim().parse::<f64>()) else {
                return;
            };
            // Each frame passes every printer in turn; a new time starts a
            // new frame.
            if self.frames.last().is_none_or(|frame| frame.at != at) {
                self.frames.push(FrameStats {
                    at,
                    ..FrameStats::default()
                });
            }
            let frame = self.frames.last_mut().expect("a frame was pushed");
            match key {
                "YMIN" => frame.ymin = value,
                "YMAX" => frame.ymax = value,
                "YAVG" => frame.yavg = value,
                "BRNG" => frame.brng = value,
                _ => {}
            }
            return;
        }
        if line.contains("black_start:") {
            let field = |name: &str| {
                line.split_whitespace()
                    .find_map(|token| token.strip_prefix(name))
                    .and_then(|value| value.parse::<f64>().ok())
            };
            if let (Some(start), Some(end)) = (field("black_start:"), field("black_end:")) {
                self.black.push(QcRange { start, end });
            }
            return;
        }
        self.tail.push_str(line);
        self.tail.push('\n');
        if self.tail.len() > TAIL_BYTES * 2 {
            self.tail = tail(&self.tail, TAIL_BYTES).to_owned();
        }
    }
}

/// The verdicts for measured frames `frame` seconds apart, with the black
/// stretches `blackdetect` found, for a program `duration` seconds long.
pub fn analyze(frames: &[FrameStats], black: &[QcRange], frame: f64, duration: f64) -> PictureQc {
    let ranges_where = |failing: &dyn Fn(&FrameStats) -> bool| -> Vec<QcRange> {
        let mut ranges: Vec<QcRange> = Vec::new();
        for stats in frames.iter().filter(|stats| failing(stats)) {
            let end = (stats.at + frame).min(duration).max(stats.at);
            match ranges.last_mut() {
                // Adjacent frames, or one good frame between, are one stretch.
                Some(last) if stats.at - last.end <= frame * 1.5 => last.end = end,
                _ => ranges.push(QcRange {
                    start: stats.at,
                    end,
                }),
            }
        }
        ranges
    };
    let worst = |value: &dyn Fn(&FrameStats) -> f64| {
        frames
            .iter()
            // The first of equals: `max_by` keeps the last.
            .rev()
            .max_by(|a, b| value(a).total_cmp(&value(b)))
            .copied()
    };

    let brng = worst(&|stats| stats.brng);
    let out_of_range = ranges_where(&|stats| stats.brng > OUT_OF_RANGE_LIMIT);
    let out_of_range = PictureCheck {
        code: CODE_OUT_OF_RANGE.into(),
        name: "Gamut Rec.709".into(),
        passed: out_of_range.is_empty(),
        value: brng.map_or(0.0, |stats| round(stats.brng * 100.0)),
        limit: OUT_OF_RANGE_LIMIT * 100.0,
        unit: "%".into(),
        at: brng.filter(|stats| stats.brng > 0.0).map(|stats| stats.at),
        ranges: out_of_range,
    };

    let luma_min = frames.iter().map(|stats| stats.ymin).reduce(f64::min);
    let luma_max = frames.iter().map(|stats| stats.ymax).reduce(f64::max);
    let excess = |stats: &FrameStats| {
        (LUMA_LEGAL_MIN - stats.ymin)
            .max(stats.ymax - LUMA_LEGAL_MAX)
            .max(0.0)
    };
    let furthest = worst(&excess);
    let luma = ranges_where(&|stats| stats.ymin < LUMA_LEGAL_MIN || stats.ymax > LUMA_LEGAL_MAX);
    let (value, limit) = match furthest {
        Some(stats)
            if stats.ymin < LUMA_LEGAL_MIN && LUMA_LEGAL_MIN - stats.ymin >= excess(&stats) =>
        {
            (stats.ymin, LUMA_LEGAL_MIN)
        }
        Some(stats) if stats.ymax > LUMA_LEGAL_MAX => (stats.ymax, LUMA_LEGAL_MAX),
        _ => (luma_max.unwrap_or(LUMA_LEGAL_MIN), LUMA_LEGAL_MAX),
    };
    let luma_levels = PictureCheck {
        code: CODE_LUMA_LEVELS.into(),
        name: "Luma levels".into(),
        passed: luma.is_empty(),
        value,
        limit,
        unit: "Y′ 10-bit".into(),
        at: furthest
            .filter(|stats| excess(stats) > 0.0)
            .map(|stats| stats.at),
        ranges: luma,
    };

    let step = FLASH_STEP * (LUMA_LEGAL_MAX - LUMA_LEGAL_MIN);
    let changes = transitions(
        &frames
            .iter()
            .map(|stats| (stats.at, stats.yavg))
            .collect::<Vec<_>>(),
        step,
    );
    let (flash_ranges, flash_max, flash_at) = flashing_windows(&changes, frame);
    let flashing = PictureCheck {
        code: CODE_FLASHING.into(),
        name: "Flashing".into(),
        passed: flash_ranges.is_empty(),
        value: flash_ranges.len() as f64,
        limit: 0.0,
        unit: "stretches".into(),
        at: flash_at,
        ranges: flash_ranges,
    };

    let black_ranges: Vec<QcRange> = black
        .iter()
        .map(|range| QcRange {
            start: round(range.start),
            end: round(range.end.min(duration)),
        })
        .collect();
    let longest = black_ranges
        .iter()
        .rev()
        .max_by(|a, b| (a.end - a.start).total_cmp(&(b.end - b.start)));
    let black = PictureCheck {
        code: CODE_BLACK.into(),
        name: "Black".into(),
        passed: black_ranges.is_empty(),
        value: black_ranges.len() as f64,
        limit: 0.0,
        unit: "stretches".into(),
        at: longest.map(|range| range.start),
        ranges: black_ranges.clone(),
    };

    PictureQc {
        duration,
        frames: frames.len(),
        out_of_range,
        luma_levels,
        flashing,
        black,
        luma_min,
        luma_max,
        flash_transitions_max: flash_max,
    }
}

/// When mean luminance turns: each time it moves at least `step` away
/// from its last extreme in the other direction.
fn transitions(samples: &[(f64, f64)], step: f64) -> Vec<f64> {
    let mut found = Vec::new();
    let Some(&(_, first)) = samples.first() else {
        return found;
    };
    let (mut low, mut high) = (first, first);
    // 1 rising, -1 falling, 0 before the first transition.
    let mut direction = 0_i8;
    let mut extreme = first;
    for &(at, value) in samples {
        match direction {
            0 => {
                low = low.min(value);
                high = high.max(value);
                if value - low >= step {
                    found.push(at);
                    (direction, extreme) = (1, value);
                } else if high - value >= step {
                    found.push(at);
                    (direction, extreme) = (-1, value);
                }
            }
            1 if value > extreme => extreme = value,
            1 if extreme - value >= step => {
                found.push(at);
                (direction, extreme) = (-1, value);
            }
            -1 if value < extreme => extreme = value,
            -1 if value - extreme >= step => {
                found.push(at);
                (direction, extreme) = (1, value);
            }
            _ => {}
        }
    }
    found
}

/// The stretches where more than [`FLASH_TRANSITIONS_LIMIT`] transitions
/// fall within [`FLASH_WINDOW_SECONDS`], the most transitions in any
/// window, and where the worst window starts.
fn flashing_windows(changes: &[f64], frame: f64) -> (Vec<QcRange>, usize, Option<f64>) {
    let mut ranges: Vec<QcRange> = Vec::new();
    let mut most = 0;
    let mut worst = None;
    let mut end = 0;
    for (start, &from) in changes.iter().enumerate() {
        end = end.max(start);
        while end < changes.len() && changes[end] - from < FLASH_WINDOW_SECONDS - 1e-9 {
            end += 1;
        }
        let inside = end - start;
        if inside > most {
            most = inside;
            worst = Some(round(from));
        }
        if inside > FLASH_TRANSITIONS_LIMIT {
            let stretch = QcRange {
                start: round(from),
                end: round(changes[end - 1] + frame),
            };
            match ranges.last_mut() {
                Some(last) if stretch.start <= last.end => last.end = last.end.max(stretch.end),
                _ => ranges.push(stretch),
            }
        }
    }
    let worst = worst.filter(|_| most > FLASH_TRANSITIONS_LIMIT);
    (ranges, most, worst)
}

/// Seconds to the microsecond.
fn round(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 24.0;

    fn frames(count: usize, stats: impl Fn(usize) -> (f64, f64, f64, f64)) -> Vec<FrameStats> {
        (0..count)
            .map(|index| {
                let (ymin, ymax, yavg, brng) = stats(index);
                FrameStats {
                    at: index as f64 * FRAME,
                    ymin,
                    ymax,
                    yavg,
                    brng,
                }
            })
            .collect()
    }

    #[test]
    fn a_legal_steady_picture_passes_every_check() {
        let qc = analyze(&frames(48, |_| (64.0, 940.0, 500.0, 0.0)), &[], FRAME, 2.0);
        assert!(qc.passed(), "{qc:#?}");
        assert_eq!(qc.frames, 48);
        assert_eq!((qc.luma_min, qc.luma_max), (Some(64.0), Some(940.0)));
        assert_eq!(qc.out_of_range.at, None);
        assert_eq!(qc.flash_transitions_max, 0);
        let codes: Vec<&str> = qc
            .checks()
            .iter()
            .map(|check| check.code.as_str())
            .collect();
        assert_eq!(
            codes,
            [
                CODE_OUT_OF_RANGE,
                CODE_LUMA_LEVELS,
                CODE_FLASHING,
                CODE_BLACK
            ]
        );
    }

    #[test]
    fn superwhites_fail_luma_and_gamut_with_their_times() {
        // Frames 24 to 35 go to Y′ 1000 with a fifth of the picture illegal.
        let qc = analyze(
            &frames(72, |index| {
                if (24..36).contains(&index) {
                    (64.0, 1000.0, 600.0, 0.2)
                } else {
                    (64.0, 900.0, 500.0, 0.0)
                }
            }),
            &[],
            FRAME,
            3.0,
        );
        assert!(!qc.luma_levels.passed);
        assert_eq!(qc.luma_levels.value, 1000.0);
        assert_eq!(qc.luma_levels.limit, 940.0);
        assert_eq!(qc.luma_levels.at, Some(1.0));
        assert_eq!(qc.luma_levels.ranges.len(), 1);
        assert!((qc.luma_levels.ranges[0].start - 1.0).abs() < 1e-9);
        assert!((qc.luma_levels.ranges[0].end - 1.5).abs() < 1e-9);
        assert!(!qc.out_of_range.passed);
        assert_eq!(qc.out_of_range.value, 20.0);
        assert_eq!(qc.out_of_range.limit, 1.0);
        assert!(qc.flashing.passed, "one rise and one fall is not flashing");
    }

    #[test]
    fn crushed_blacks_fail_low() {
        let qc = analyze(&frames(24, |_| (20.0, 900.0, 400.0, 0.0)), &[], FRAME, 1.0);
        assert!(!qc.luma_levels.passed);
        assert_eq!((qc.luma_levels.value, qc.luma_levels.limit), (20.0, 64.0));
    }

    #[test]
    fn strobing_is_flashing_and_slow_changes_are_not() {
        // Two frames bright, two dark: twelve transitions a second.
        let strobe = analyze(
            &frames(48, |index| {
                let bright = index % 4 < 2;
                (64.0, 940.0, if bright { 900.0 } else { 100.0 }, 0.0)
            }),
            &[],
            FRAME,
            2.0,
        );
        assert!(!strobe.flashing.passed);
        assert_eq!(strobe.flashing.ranges.len(), 1);
        assert!(strobe.flash_transitions_max >= 11, "{strobe:#?}");
        assert_eq!(strobe.flashing.at, Some(round(2.0 * FRAME)));

        // A change every half second is two transitions a second.
        let slow = analyze(
            &frames(96, |index| {
                (
                    64.0,
                    940.0,
                    if (index / 12) % 2 == 0 { 900.0 } else { 100.0 },
                    0.0,
                )
            }),
            &[],
            FRAME,
            4.0,
        );
        assert!(slow.flashing.passed, "{:?}", slow.flashing);
        assert_eq!(slow.flash_transitions_max, 2);

        // Small flicker below 10% is never a transition.
        let flicker = analyze(
            &frames(48, |index| {
                (64.0, 940.0, 500.0 + (index % 2) as f64 * 60.0, 0.0)
            }),
            &[],
            FRAME,
            2.0,
        );
        assert!(flicker.flashing.passed);
    }

    #[test]
    fn black_stretches_are_reported() {
        let qc = analyze(
            &frames(96, |_| (64.0, 940.0, 500.0, 0.0)),
            &[
                QcRange {
                    start: 1.0,
                    end: 2.0,
                },
                QcRange {
                    start: 3.5,
                    end: 4.2,
                },
            ],
            FRAME,
            4.0,
        );
        assert!(!qc.black.passed);
        assert_eq!(qc.black.value, 2.0);
        assert_eq!(qc.black.at, Some(1.0));
        assert_eq!(qc.black.ranges[1].end, 4.0, "clamped to the program");
    }

    #[test]
    fn ffmpeg_lines_are_parsed_into_frames_and_black() {
        let mut parser = StatsParser::default();
        for line in [
            "[Parsed_metadata_4 @ 0x1] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_4 @ 0x1] lavfi.signalstats.YMIN=64",
            "[Parsed_metadata_5 @ 0x2] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_5 @ 0x2] lavfi.signalstats.YMAX=940",
            "[Parsed_metadata_6 @ 0x3] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_6 @ 0x3] lavfi.signalstats.YAVG=480.5",
            "[Parsed_metadata_7 @ 0x4] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_7 @ 0x4] lavfi.signalstats.BRNG=0.25",
            "[Parsed_metadata_4 @ 0x1] frame:1    pts:1       pts_time:0.0416667",
            "[Parsed_metadata_4 @ 0x1] lavfi.signalstats.YMIN=70",
            "[Parsed_blackdetect_8 @ 0x5] black_start:2 black_end:3 black_duration:1",
            "[out#0/null @ 0x6] video:0KiB audio:0KiB",
        ] {
            parser.line(line);
        }
        assert_eq!(parser.frames.len(), 2);
        assert_eq!(
            parser.frames[0],
            FrameStats {
                at: 0.0,
                ymin: 64.0,
                ymax: 940.0,
                yavg: 480.5,
                brng: 0.25
            }
        );
        assert_eq!(parser.frames[1].ymin, 70.0);
        assert_eq!(
            parser.black,
            [QcRange {
                start: 2.0,
                end: 3.0
            }]
        );
        assert!(parser.tail.contains("video:0KiB"));
    }

    #[test]
    fn the_pass_reads_rgba_and_measures_ten_bit_limited_range() {
        let args = qc_args((640, 360), Rate::FPS_24);
        let at = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|index| args[index + 1].as_str())
        };
        assert_eq!(at("-pix_fmt"), Some("rgba"));
        assert_eq!(at("-s"), Some("640x360"));
        assert_eq!(at("-framerate"), Some("24/1"));
        assert_eq!(at("-i"), Some("pipe:0"));
        let graph = at("-vf").expect("graph");
        assert!(graph.contains("out_range=tv") && graph.contains("yuv422p10le"));
        assert!(graph.contains("signalstats=stat=brng"));
        assert!(graph.contains("blackdetect=d=0.5"));
        assert_eq!(args[args.len() - 3..], ["-f", "null", "-"]);
        assert_eq!(
            frame_times(Rate::FPS_24, Rate::FPS_24.time_of(48)).len(),
            48
        );
    }
}
