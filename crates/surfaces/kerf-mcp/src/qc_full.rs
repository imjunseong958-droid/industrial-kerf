//! The full QC check: everything [`check`] finds, and what needs the
//! program measured. The sound is mixed and measured to EBU R128
//! (integrated loudness, true peak, loudness range) by
//! `kerf_engine::loudness`, and every frame is drawn and measured for
//! Rec.709 gamut, luma levels, photosensitive flashing, and black by
//! `kerf_engine::qc`, and titles and captions are read against what is
//! behind them from frames drawn for it. That takes about as long as a draft
//! render, so MCP tools keep using the fast [`check`] and the editor's QC
//! runs this one.
//!
//! Each measured finding sits at its worst sample: the loudest short-term
//! window of a programme that is too loud, the stretch over the true-peak
//! limit around its highest peak, the stretch of picture around its worst
//! frame. A measurement that cannot run fails its rows as `unavailable`
//! rather than passing them.

use crate::qc::{
    check, CheckReport, Finding, RowDef, Severity, CAPTION_DURATION, CAPTION_GAPS, CAPTION_OVERLAP,
    CUTS_IN_WORDS, DOCUMENT, FRAME_EDGES, MEDIA, READING_SPEED_ROW, RENDER, TITLE_SAFE,
};
use kerf_engine::loudness::{self, Loudness};
use kerf_engine::program::{compile_program, ProgramOptions};
use kerf_engine::qc::{self as picture, PictureCheck, PictureQc, QcRange};
use kerf_engine::{CancellationToken, EngineError};
use kerf_lang::Clock;
use kerf_model::{Catalog, Project, Sequence, Time};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// What the measured checks allow. The default is EBU R128: −23 LUFS
/// ±1 LU integrated, true peak at most −1 dBTP, loudness range at most
/// 20 LU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QcLimits {
    /// Integrated loudness target, LUFS.
    pub target_lufs: f64,
    /// How far from the target integrated loudness may be, LU.
    pub tolerance_lu: f64,
    /// The highest true peak allowed, dBTP.
    pub true_peak_dbtp: f64,
    /// The widest loudness range allowed, LU.
    pub range_lu: f64,
}

impl Default for QcLimits {
    fn default() -> Self {
        Self {
            target_lufs: loudness::TARGET_LUFS,
            tolerance_lu: 1.0,
            true_peak_dbtp: loudness::TRUE_PEAK_LIMIT_DBTP,
            range_lu: 20.0,
        }
    }
}

const INTEGRATED: &str = "loudness.integrated";
const TRUE_PEAK: &str = "loudness.true_peak";
const RANGE: &str = "loudness.range";
/// The sound could not be measured: fails every loudness row.
const LOUDNESS_UNAVAILABLE: &str = "loudness.unavailable";
/// The picture could not be measured: fails every picture row.
const PICTURE_UNAVAILABLE: &str = "picture.unavailable";
const FLASH_BLACK: &str = "picture.flash_black";
/// Short clips and slivers of another shot, from the fast check.
const FLASH_FRAME_CODES: [&str; 2] = ["clip.flash", "cut.near_scene_cut"];

/// The rows [`check_full`] reports, in the order the editor's QC table
/// lists them: the measured checks and the text checks first, then the
/// rest of [`crate::qc::ROWS`]. "Flash and black" joins the measured
/// flashing and black with the fast check's flash frames and gaps.
pub const FULL_ROWS: &[RowDef] = &[
    RowDef {
        code: INTEGRATED,
        name: "Integrated loudness",
        codes: &[INTEGRATED, LOUDNESS_UNAVAILABLE],
        noun: "",
        limit: None,
    },
    RowDef {
        code: TRUE_PEAK,
        name: "True peak",
        codes: &[TRUE_PEAK, LOUDNESS_UNAVAILABLE],
        noun: "",
        limit: None,
    },
    RowDef {
        code: RANGE,
        name: "Loudness range",
        codes: &[RANGE, LOUDNESS_UNAVAILABLE],
        noun: "",
        limit: None,
    },
    RowDef {
        code: picture::CODE_OUT_OF_RANGE,
        name: "Gamut Rec.709",
        codes: &[picture::CODE_OUT_OF_RANGE, PICTURE_UNAVAILABLE],
        noun: "",
        limit: None,
    },
    RowDef {
        code: picture::CODE_LUMA_LEVELS,
        name: "Luma levels",
        codes: &[picture::CODE_LUMA_LEVELS, PICTURE_UNAVAILABLE],
        noun: "",
        limit: None,
    },
    TITLE_SAFE,
    CAPTION_OVERLAP,
    RowDef {
        code: FLASH_BLACK,
        name: "Flash and black",
        codes: &[
            picture::CODE_FLASHING,
            picture::CODE_BLACK,
            FLASH_FRAME_CODES[0],
            FLASH_FRAME_CODES[1],
            PICTURE_UNAVAILABLE,
        ],
        noun: "",
        limit: None,
    },
    RowDef {
        code: "text.contrast",
        name: "Text contrast",
        codes: &["text.contrast"],
        noun: "faint",
        limit: Some("3:1"),
    },
    RowDef {
        code: "text.clipped",
        name: "Clipped text",
        codes: &["text.clipped"],
        noun: "cut off",
        limit: None,
    },
    MEDIA,
    RENDER,
    DOCUMENT,
    FRAME_EDGES,
    CUTS_IN_WORDS,
    READING_SPEED_ROW,
    CAPTION_DURATION,
    CAPTION_GAPS,
];

/// Share of the progress the sound takes, and the picture after it; reading text takes the rest.
const SOUND_SHARE: f64 = 0.3;
const PICTURE_SHARE: f64 = 0.6;

/// Everything [`check`] finds, and the program's loudness and picture
/// measured on the first sequence, to EBU R128. `project` has its media
/// paths resolved; `dir` is the project. Only cancelling fails it
/// ([`EngineError::Cancelled`]); a measurement that cannot run fails its
/// rows instead.
pub fn check_full(
    project: &Project,
    dir: &Path,
    cancel: &CancellationToken,
) -> Result<CheckReport, EngineError> {
    check_full_with(project, None, dir, &QcLimits::default(), cancel, |_| {})
}

/// [`check_full`] measuring `sequence` (the first when `None`) against
/// `limits`, reporting how much is done (0 to 1) as it goes.
pub fn check_full_with(
    project: &Project,
    sequence: Option<&str>,
    dir: &Path,
    limits: &QcLimits,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(f64),
) -> Result<CheckReport, EngineError> {
    let mut findings = check(project, dir).findings;
    let target = match sequence {
        Some(id) => project.sequence(id),
        None => project.sequences.first(),
    };
    let Some(target) = target else {
        let what = sequence.map_or("the project has no sequence".to_owned(), |id| {
            format!("no sequence `{id}` in the project")
        });
        findings.push(unavailable(LOUDNESS_UNAVAILABLE, None, &what));
        findings.push(unavailable(PICTURE_UNAVAILABLE, None, &what));
        let mut report = CheckReport::new(findings, FULL_ROWS);
        label_rows(&mut report, None, None, limits);
        return Ok(report);
    };
    let id = Some(target.id.as_str());

    let sound = loudness::measure_program_with_progress(project, id, dir, cancel, |done| {
        on_progress(done * SOUND_SHARE)
    });
    let sound = match sound {
        Ok(sound) => {
            findings.extend(loudness_findings(
                project, target, dir, &sound, limits, cancel,
            )?);
            Some(sound)
        }
        Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
        Err(error) => {
            findings.push(unavailable(
                LOUDNESS_UNAVAILABLE,
                Some(target),
                &format!("the sound could not be measured: {error}"),
            ));
            None
        }
    };
    on_progress(SOUND_SHARE);

    let measured = picture::picture_qc_with_progress(project, id, dir, cancel, |done| {
        on_progress(SOUND_SHARE + done * PICTURE_SHARE)
    });
    let measured = match measured {
        Ok(measured) => {
            let gaps: Vec<(f64, f64)> = findings
                .iter()
                .filter(|finding| {
                    finding.code == picture::CODE_BLACK && finding.sequence.as_deref() == id
                })
                .filter_map(|finding| {
                    Some((finding.at?, finding.at? + finding.duration.unwrap_or(0.0)))
                })
                .collect();
            findings.extend(picture_findings(target, &measured, &gaps));
            Some(measured)
        }
        Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
        Err(error) => {
            findings.push(unavailable(
                PICTURE_UNAVAILABLE,
                Some(target),
                &format!("the picture could not be measured: {error}"),
            ));
            None
        }
    };
    on_progress(SOUND_SHARE + PICTURE_SHARE);

    crate::text_lint::contrast_of(
        project,
        target,
        whole(target),
        |done| {
            on_progress(SOUND_SHARE + PICTURE_SHARE + done * (1.0 - SOUND_SHARE - PICTURE_SHARE));
            !cancel.is_cancelled()
        },
        &mut findings,
    );
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    on_progress(1.0);

    let mut report = CheckReport::new(findings, FULL_ROWS);
    label_rows(&mut report, sound.as_ref(), measured.as_ref(), limits);
    // These read the 8-bit frames a monitor gets, as Rec.709 video, and say nothing of an HDR master.
    if target.color.is_hdr() {
        for row in &mut report.summary.rows {
            if row.code == picture::CODE_OUT_OF_RANGE {
                row.name = "Gamut, SDR copy".into();
            } else if row.code == picture::CODE_LUMA_LEVELS {
                row.name = "Luma, SDR copy".into();
            }
        }
    }
    Ok(report)
}

fn unavailable(code: &str, sequence: Option<&Sequence>, message: &str) -> Finding {
    let mut finding = Finding::new(
        Severity::Warning,
        code,
        sequence.map(|sequence| sequence.id.as_str()),
        message,
    );
    finding.sequence = sequence.map(|sequence| sequence.id.to_string());
    finding
}

/// `[from, to)` seconds on `sequence`, as times on it.
fn span(sequence: &Sequence, from: f64, to: f64) -> (Time, Time) {
    let time = |seconds: f64| Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO);
    let end = sequence.duration();
    let from = time(from).min(end);
    (from, time(to).min(end).max(from))
}

fn whole(sequence: &Sequence) -> (Time, Time) {
    (Time::ZERO, sequence.duration())
}

/// The R128 findings of `sound`, measured on `sequence`.
fn loudness_findings(
    project: &Project,
    sequence: &Sequence,
    dir: &Path,
    sound: &Loudness,
    limits: &QcLimits,
    cancel: &CancellationToken,
) -> Result<Vec<Finding>, EngineError> {
    let entity = sequence.id.as_str();
    let mut findings = Vec::new();
    let integrated = sound.integrated;
    if !integrated.is_finite() {
        let (from, to) = whole(sequence);
        findings.push(
            Finding::warning(
                INTEGRATED,
                entity,
                format!(
                    "`{entity}` is silent: there is no programme loudness to measure \
                     against {} LUFS",
                    number(limits.target_lufs, 0)
                ),
            )
            .during(sequence, from, to),
        );
    } else if (integrated - limits.target_lufs).abs() > limits.tolerance_lu {
        let loud = integrated > limits.target_lufs;
        let limit = if loud {
            limits.target_lufs + limits.tolerance_lu
        } else {
            limits.target_lufs - limits.tolerance_lu
        };
        // Too loud: where it is loudest. Too quiet: all of it.
        let (from, to) = match sound.short_term_max_at().filter(|_| loud) {
            Some(end) => span(sequence, end - 3.0, end),
            None => whole(sequence),
        };
        findings.push(
            Finding::new(
                Severity::Error,
                INTEGRATED,
                Some(entity),
                format!(
                    "integrated loudness of `{entity}` is {} LUFS, {} the {} LUFS target \
                     ±{} LU",
                    number(integrated, 1),
                    if loud { "above" } else { "below" },
                    number(limits.target_lufs, 0),
                    number(limits.tolerance_lu, 0)
                ),
            )
            .during(sequence, from, to)
            .measured(round(integrated, 1), limit, "LUFS"),
        );
    }
    let peak = sound.true_peak_max;
    if peak.is_finite() && peak > limits.true_peak_dbtp {
        // Where it peaks only places the finding: a pass that cannot run
        // leaves it on the whole programme.
        let located = match locate_peak(project, sequence, dir, limits.true_peak_dbtp, cancel) {
            Ok(located) => located,
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(_) => None,
        };
        let (from, to) = match located {
            Some(stretch) => span(sequence, stretch.start, stretch.end),
            None => whole(sequence),
        };
        let mut message = format!(
            "true peak of `{entity}` is {} dBTP, over the {} dBTP limit",
            number(peak, 1),
            number(limits.true_peak_dbtp, 1)
        );
        if let Some(stretch) = located.filter(|stretch| stretch.over > 1) {
            message.push_str(&format!(
                "; {} stretches go over, this one the highest",
                stretch.over
            ));
        }
        findings.push(
            Finding::new(Severity::Error, TRUE_PEAK, Some(entity), message)
                .during(sequence, from, to)
                .measured(round(peak, 1), limits.true_peak_dbtp, "dBTP"),
        );
    }
    if integrated.is_finite() && sound.lra > limits.range_lu {
        let (from, to) = whole(sequence);
        findings.push(
            Finding::warning(
                RANGE,
                entity,
                format!(
                    "loudness range of `{entity}` is {} LU, wider than {} LU",
                    number(sound.lra, 1),
                    number(limits.range_lu, 0)
                ),
            )
            .during(sequence, from, to)
            .measured(round(sound.lra, 1), limits.range_lu, "LU"),
        );
    }
    Ok(findings)
}

/// The findings of a picture QC pass over `sequence`. Black that the fast
/// check already found as a gap (`gaps`, seconds) is not found twice.
fn picture_findings(
    sequence: &Sequence,
    measured: &PictureQc,
    gaps: &[(f64, f64)],
) -> Vec<Finding> {
    let entity = sequence.id.as_str();
    let clock = Clock::of_sequence(sequence);
    let at = |seconds: f64| {
        let time = Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO);
        clock.print_position(clock.rate.round(time))
    };
    let mut findings = Vec::new();
    // Gamut and levels: one finding, at the stretch with the worst frame.
    for check in [&measured.out_of_range, &measured.luma_levels] {
        if check.passed {
            continue;
        }
        let stretch = worst_range(check);
        let (from, to) = match stretch {
            Some(range) => span(sequence, range.start, range.end),
            None => whole(sequence),
        };
        let worst = check
            .at
            .map(|seconds| format!(" at {}", at(seconds)))
            .unwrap_or_default();
        let stretches = match check.ranges.len() {
            0 | 1 => String::new(),
            count => format!(" ({count} stretches)"),
        };
        let message = if check.code == picture::CODE_OUT_OF_RANGE {
            format!(
                "{}% of the picture of `{entity}` is outside the Rec.709 legal range{worst}, \
                 over {}%{stretches}",
                number(check.value, 1),
                number(check.limit, 0)
            )
        } else {
            format!(
                "luma of `{entity}` reaches Y′ {}{worst}, outside the legal {}–{}{stretches}",
                number(check.value, 0),
                number(picture::LUMA_LEGAL_MIN, 0),
                number(picture::LUMA_LEGAL_MAX, 0)
            )
        };
        findings.push(
            Finding::new(Severity::Error, check.code.as_str(), Some(entity), message)
                .during(sequence, from, to)
                .measured(check.value, check.limit, &check.unit),
        );
    }
    // Flashing and black: one finding a stretch.
    for range in &measured.flashing.ranges {
        let (from, to) = span(sequence, range.start, range.end);
        findings.push(
            Finding::new(
                Severity::Error,
                picture::CODE_FLASHING,
                Some(entity),
                format!(
                    "the picture of `{entity}` flashes from {} to {}: more than {} \
                     changes of brightness within a second",
                    at(range.start),
                    at(range.end),
                    picture::FLASH_TRANSITIONS_LIMIT
                ),
            )
            .during(sequence, from, to),
        );
    }
    for range in &measured.black.ranges {
        let known = gaps
            .iter()
            .any(|(start, end)| range.start < *end && *start < range.end);
        if known {
            continue;
        }
        let (from, to) = span(sequence, range.start, range.end);
        findings.push(
            Finding::warning(
                picture::CODE_BLACK,
                entity,
                format!(
                    "the picture of `{entity}` is black from {} to {}",
                    at(range.start),
                    at(range.end)
                ),
            )
            .during(sequence, from, to),
        );
    }
    findings
}

/// The failing stretch that holds the check's worst frame, or its first.
fn worst_range(check: &PictureCheck) -> Option<QcRange> {
    check
        .at
        .and_then(|at| {
            check
                .ranges
                .iter()
                .find(|range| range.start <= at && at <= range.end)
        })
        .or(check.ranges.first())
        .copied()
}

/// Give the measured rows the values the editor's table shows: `−23.1
/// LUFS · ±1`, `−0.4 dBTP · ≤ −1.0`, `6.2 LU · ≤ 20`, `legal`,
/// `Y′ 64–940`, and `0 · 0` (flashing · black); `unavailable` where the
/// measurement could not run.
fn label_rows(
    report: &mut CheckReport,
    sound: Option<&Loudness>,
    measured: Option<&PictureQc>,
    limits: &QcLimits,
) {
    let count = |codes: &[&str]| {
        report
            .findings
            .iter()
            .filter(|finding| finding.severity.fails() && codes.contains(&finding.code.as_str()))
            .count()
    };
    let flashes = count(&[
        picture::CODE_FLASHING,
        FLASH_FRAME_CODES[0],
        FLASH_FRAME_CODES[1],
    ]);
    let blacks = count(&[picture::CODE_BLACK]);
    let level = |value: f64, unit: &str| {
        if value.is_finite() {
            format!("{} {unit}", number(value, 1))
        } else {
            "no signal".to_owned()
        }
    };
    let target = if limits.target_lufs == loudness::TARGET_LUFS {
        format!("±{}", number(limits.tolerance_lu, 0))
    } else {
        format!(
            "{} ±{}",
            number(limits.target_lufs, 0),
            number(limits.tolerance_lu, 0)
        )
    };
    for row in &mut report.summary.rows {
        let (value, limit) = match row.code.as_str() {
            INTEGRATED => (
                sound.map(|sound| level(sound.integrated, "LUFS")),
                Some(target.clone()),
            ),
            TRUE_PEAK => (
                sound.map(|sound| level(sound.true_peak_max, "dBTP")),
                Some(format!("≤ {}", number(limits.true_peak_dbtp, 1))),
            ),
            RANGE => (
                sound.map(|sound| {
                    if sound.integrated.is_finite() {
                        format!("{} LU", number(sound.lra, 1))
                    } else {
                        "no signal".to_owned()
                    }
                }),
                Some(format!("≤ {}", number(limits.range_lu, 0))),
            ),
            picture::CODE_OUT_OF_RANGE => (
                measured.map(|measured| {
                    let check = &measured.out_of_range;
                    if check.passed {
                        "legal".to_owned()
                    } else {
                        format!("{}% outside", number(check.value, 1))
                    }
                }),
                None,
            ),
            picture::CODE_LUMA_LEVELS => (
                measured.map(|measured| match (measured.luma_min, measured.luma_max) {
                    (Some(low), Some(high)) => {
                        format!("Y′ {}–{}", number(low, 0), number(high, 0))
                    }
                    _ => "no frames".to_owned(),
                }),
                None,
            ),
            FLASH_BLACK => (measured.map(|_| format!("{flashes} · {blacks}")), None),
            _ => continue,
        };
        row.value = value.unwrap_or_else(|| "unavailable".to_owned());
        row.limit = limit;
    }
}

/// `value` to `places` decimals, negative with a minus sign (−), as the
/// editor prints levels.
fn number(value: f64, places: usize) -> String {
    let text = format!("{:.*}", places, round(value, places as i32));
    match text.strip_prefix('-') {
        Some(rest) if rest.chars().any(|c| c.is_ascii_digit() && c != '0') => format!("−{rest}"),
        Some(rest) => rest.to_owned(),
        None => text,
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    (value * scale).round() / scale
}

/// A stretch of the programme whose true peak goes over the limit.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PeakStretch {
    start: f64,
    end: f64,
    /// How many stretches go over the limit, this one included.
    over: usize,
}

/// The audio is cut into blocks this long to find where it peaks.
const PEAK_BLOCK_SECONDS: f64 = 0.1;
/// BS.1770 measures true peak on the signal oversampled four times.
const PEAK_OVERSAMPLE: u32 = 4;

/// Where `sequence`'s sound goes over `limit` dBTP: the stretch around its
/// highest peak. `ebur128` gives only the programme's highest true peak,
/// so this mixes the sound again, oversampled four times as BS.1770 does,
/// and reads the peak of every 100 ms block. `None` when the program maps
/// no sound graph or the pass finds no block over the limit.
fn locate_peak(
    project: &Project,
    sequence: &Sequence,
    dir: &Path,
    limit: f64,
    cancel: &CancellationToken,
) -> Result<Option<PeakStretch>, EngineError> {
    let heard = kerf_engine::ramps::with_sound(project);
    let compiled = compile_program(
        &heard,
        Catalog::builtin(),
        &ProgramOptions {
            sequence: Some(sequence.id.to_string()),
            ..ProgramOptions::default()
        },
    )
    .map_err(|error| EngineError::msg(error.to_string()))?;
    // The program mixes its sound at one rate whatever the media's.
    let rate = kerf_engine::program::SAMPLE_RATE * PEAK_OVERSAMPLE;
    let block = (f64::from(rate) * PEAK_BLOCK_SECONDS).round() as u32;
    let Some(args) = peak_args(&compiled.argv, rate, block) else {
        return Ok(None);
    };
    let blocks = run_ffmpeg_lines(&args, dir, cancel, parse_peaks)?;
    Ok(stretch_over(&blocks, limit))
}

/// The FFmpeg arguments that run the program's sound graph `argv` through
/// an oversampled peak meter into nothing.
fn peak_args(argv: &[String], rate: u32, block: u32) -> Option<Vec<String>> {
    let mut args: Vec<String> = argv.iter().skip(1).cloned().collect();
    let graph = args.iter().position(|arg| arg == "-filter_complex")? + 1;
    let map = args.iter().rposition(|arg| arg == "-map")? + 1;
    let sound = args.get(map)?.clone();
    if graph >= args.len() || !(sound.starts_with('[') && sound.ends_with(']')) {
        return None;
    }
    args[graph] = format!(
        "{};{sound}aresample={rate},asetnsamples=n={block}:p=0,\
         astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,\
         ametadata=mode=print:key=lavfi.astats.Overall.Peak_level[peaks]",
        args[graph]
    );
    args[map] = "[peaks]".into();
    let mut measured = vec!["-hide_banner".to_owned(), "-nostats".to_owned()];
    measured.extend(args);
    measured.extend(["-f", "null", "-"].map(String::from));
    Some(measured)
}

/// `(start seconds, peak dBFS)` of each block FFmpeg printed.
fn parse_peaks(lines: &[String]) -> Vec<(f64, f64)> {
    let mut blocks = Vec::new();
    let mut at = None;
    for line in lines {
        if let Some(index) = line.find("pts_time:") {
            at = line[index + "pts_time:".len()..]
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok());
        } else if let Some(index) = line.find("lavfi.astats.Overall.Peak_level=") {
            let value = line[index + "lavfi.astats.Overall.Peak_level=".len()..].trim();
            // `-inf` is a silent block.
            let level = if value.contains("inf") {
                Some(f64::NEG_INFINITY)
            } else {
                value.parse::<f64>().ok()
            };
            if let (Some(start), Some(level)) = (at.take(), level) {
                blocks.push((start, level));
            }
        }
    }
    blocks
}

/// The stretch of blocks over `limit` that holds the highest one, and how
/// many such stretches there are. Blocks are [`PEAK_BLOCK_SECONDS`] long;
/// touching ones join.
fn stretch_over(blocks: &[(f64, f64)], limit: f64) -> Option<PeakStretch> {
    let mut stretches: Vec<(f64, f64, f64)> = Vec::new();
    for &(start, level) in blocks.iter().filter(|(_, level)| *level > limit) {
        let end = start + PEAK_BLOCK_SECONDS;
        match stretches.last_mut() {
            Some(last) if start - last.1 <= PEAK_BLOCK_SECONDS * 0.5 => {
                last.1 = end;
                last.2 = last.2.max(level);
            }
            _ => stretches.push((start, end, level)),
        }
    }
    let over = stretches.len();
    stretches
        .iter()
        .rev()
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|&(start, end, _)| PeakStretch {
            start: round(start, 3),
            end: round(end, 3),
            over,
        })
}

/// Run FFmpeg with `args` in `dir`, killing it when `cancel` is asked
/// for, and read what it wrote to stderr with `parse`.
fn run_ffmpeg_lines<T>(
    args: &[String],
    dir: &Path,
    cancel: &CancellationToken,
    parse: impl FnOnce(&[String]) -> T,
) -> Result<T, EngineError> {
    let mut command = Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin());
    if dir.is_dir() {
        command.current_dir(dir);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| EngineError::msg(format!("ffmpeg could not start: {error}")))?;
    let stderr = child.stderr.take().expect("stderr is piped");
    let reader = thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(std::io::Result::ok)
            .collect::<Vec<_>>()
    });
    let status = loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(EngineError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(EngineError::msg(format!("ffmpeg: {error}"))),
        }
    };
    let lines = reader.join().unwrap_or_default();
    if !status.success() {
        let tail: Vec<&str> = lines.iter().rev().take(8).map(String::as_str).collect();
        return Err(EngineError::msg(format!(
            "the peak pass failed: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        )));
    }
    Ok(parse(&lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_print_as_the_editor_shows_them() {
        assert_eq!(number(-23.06, 1), "−23.1");
        assert_eq!(number(-0.04, 1), "0.0");
        assert_eq!(number(6.2, 1), "6.2");
        assert_eq!(number(-1.0, 1), "−1.0");
        assert_eq!(number(940.4, 0), "940");
    }

    #[test]
    fn peaks_are_read_per_block_and_joined_into_stretches() {
        let lines: Vec<String> = [
            "[Parsed_ametadata_5 @ 0x1] frame:0    pts:0       pts_time:0",
            "[Parsed_ametadata_5 @ 0x1] lavfi.astats.Overall.Peak_level=-20.000000",
            "[Parsed_ametadata_5 @ 0x1] frame:1    pts:19200   pts_time:0.1",
            "[Parsed_ametadata_5 @ 0x1] lavfi.astats.Overall.Peak_level=-0.500000",
            "[Parsed_ametadata_5 @ 0x1] frame:2    pts:38400   pts_time:0.2",
            "[Parsed_ametadata_5 @ 0x1] lavfi.astats.Overall.Peak_level=-0.100000",
            "[Parsed_ametadata_5 @ 0x1] frame:3    pts:57600   pts_time:0.3",
            "[Parsed_ametadata_5 @ 0x1] lavfi.astats.Overall.Peak_level=-inf",
            "[Parsed_ametadata_5 @ 0x1] frame:4    pts:76800   pts_time:0.4",
            "[Parsed_ametadata_5 @ 0x1] lavfi.astats.Overall.Peak_level=-0.800000",
        ]
        .map(String::from)
        .to_vec();
        let blocks = parse_peaks(&lines);
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[3], (0.3, f64::NEG_INFINITY));
        let stretch = stretch_over(&blocks, -1.0).unwrap();
        assert_eq!(
            stretch,
            PeakStretch {
                start: 0.1,
                end: 0.3,
                over: 2
            }
        );
        assert_eq!(stretch_over(&blocks, 0.0), None);
    }

    #[test]
    fn the_peak_pass_reads_the_program_sound_graph() {
        let argv: Vec<String> = [
            "ffmpeg",
            "-y",
            "-i",
            "a.wav",
            "-filter_complex",
            "[0:a]anull[audio]",
            "-map",
            "[audio]",
        ]
        .map(String::from)
        .to_vec();
        let args = peak_args(&argv, 192_000, 19_200).unwrap();
        assert_eq!(args[0], "-hide_banner");
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.starts_with("[0:a]anull[audio];[audio]aresample=192000,"));
        assert!(graph.ends_with("[peaks]"), "{graph}");
        assert_eq!(
            args[args.iter().rposition(|a| a == "-map").unwrap() + 1],
            "[peaks]"
        );
        assert_eq!(&args[args.len() - 3..], ["-f", "null", "-"]);
        assert_eq!(peak_args(&argv[..5], 192_000, 19_200), None);
    }
}
