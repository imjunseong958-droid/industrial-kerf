//! The full QC check against generated sound and picture: R128 loudness,
//! true peak and loudness range, and the picture's gamut, levels,
//! flashing and black, each placed at its worst sample.

use kerf_engine::{CancellationToken, EngineError};
use kerf_mcp::{check_full, check_full_with, CheckReport, Finding, QcLimits, Row, Severity};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok()
}

fn ffmpeg(directory: &Path, args: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .current_dir(directory)
        .status()
        .expect("ffmpeg");
    assert!(status.success(), "ffmpeg {args:?}");
}

/// A 997 Hz tone of `seconds` in both channels of 48 kHz stereo PCM, at
/// `amplitude` of full scale; within `burst` (from, to, amplitude) it
/// rises to that amplitude.
fn tone(
    directory: &Path,
    name: &str,
    seconds: f64,
    amplitude: f64,
    burst: Option<(f64, f64, f64)>,
) {
    let level = match burst {
        Some((from, to, loud)) => format!("if(between(t\\,{from}\\,{to})\\,{loud}\\,{amplitude})"),
        None => format!("{amplitude}"),
    };
    let wave = format!("{level}*sin(2*PI*997*t)");
    let source = format!("aevalsrc={wave}|{wave}:s=48000:d={seconds}");
    ffmpeg(
        directory,
        &["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le", name],
    );
}

/// `seconds` of 320x180 picture at 30 fps: mid grey, or with `flashing`,
/// black and white every three frames.
fn picture(directory: &Path, name: &str, seconds: f64, flashing: bool) {
    let source = if flashing {
        format!(
            "nullsrc=s=320x180:r=30:d={seconds},format=gray,\
             geq=lum='if(mod(floor(N/3)\\,2)\\,200\\,30)',format=yuv420p"
        )
    } else {
        format!("color=c=0x808080:s=320x180:r=30:d={seconds},format=yuv420p")
    };
    ffmpeg(
        directory,
        &[
            "-f",
            "lavfi",
            "-i",
            &source,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            name,
        ],
    );
}

/// A four-second spot: `picture.mp4` over `sound.wav`.
fn spot(directory: &Path) -> Project {
    let document = "kerf 1
project name=\"Spot\"
media m1 path=\"picture.mp4\" rate=30 dur=00:00:04:00 video=320x180
media m2 path=\"sound.wav\" rate=30 dur=00:00:04:00 audio=2ch@48000
sequence s1 rate=30 size=320x180
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00
  track A1 audio
    clip c2 media=m2 at=00:00:00:00 dur=00:00:04:00
";
    let mut project = kerf_lang::parse_document(document, Catalog::builtin())
        .unwrap_or_else(|error| panic!("{error:#?}"))
        .project;
    kerf_runtime::resolve_media_paths(&mut project, directory);
    project
}

fn run(directory: &Path) -> CheckReport {
    check_full(&spot(directory), directory, &CancellationToken::new()).expect("check_full")
}

fn with<'a>(report: &'a CheckReport, code: &str) -> Vec<&'a Finding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.code == code)
        .collect()
}

fn row<'a>(report: &'a CheckReport, code: &str) -> &'a Row {
    report
        .summary
        .rows
        .iter()
        .find(|row| row.code == code)
        .unwrap_or_else(|| panic!("no row {code}: {:#?}", report.summary))
}

/// The amplitude at which the stereo tone reads `lufs` LUFS: a 0 dBFS
/// 997 Hz sine reads −3.01 LUFS in one channel (BS.1770), so 0 LUFS in
/// two.
fn amplitude_for(lufs: f64) -> f64 {
    10_f64.powf(lufs / 20.0)
}

#[test]
fn a_clean_spot_passes_the_measured_rows_with_their_readings() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-23.0),
        None,
    );
    let report = run(directory.path());
    eprintln!("{:#?}", report.summary);
    let codes: Vec<&str> = report
        .summary
        .rows
        .iter()
        .map(|row| row.code.as_str())
        .collect();
    assert_eq!(
        &codes[..8],
        [
            "loudness.integrated",
            "loudness.true_peak",
            "loudness.range",
            "picture.out_of_range",
            "picture.luma_levels",
            "text.safe_area",
            "text.overlap",
            "picture.flash_black",
        ]
    );
    assert_eq!(report.summary.total, 18);
    for code in &codes[..8] {
        assert!(row(&report, code).passed, "{:#?}", row(&report, code));
    }
    let integrated = row(&report, "loudness.integrated");
    assert!(integrated.value.ends_with(" LUFS"), "{integrated:?}");
    assert!(integrated.value.starts_with("−2"), "{integrated:?}");
    assert_eq!(integrated.limit.as_deref(), Some("±1"));
    assert!(row(&report, "loudness.true_peak").value.ends_with(" dBTP"));
    assert_eq!(
        row(&report, "loudness.true_peak").limit.as_deref(),
        Some("≤ −1.0")
    );
    assert_eq!(
        row(&report, "loudness.range").limit.as_deref(),
        Some("≤ 20")
    );
    assert_eq!(row(&report, "picture.out_of_range").value, "legal");
    assert!(row(&report, "picture.luma_levels").value.starts_with("Y′ "));
    assert_eq!(row(&report, "picture.flash_black").value, "0 · 0");
    assert!(report.valid, "{report:#?}");
}

#[test]
fn an_hdr_sequence_says_its_picture_rows_read_the_sdr_copy() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-23.0),
        None,
    );
    let mut project = spot(directory.path());
    project.sequences[0].color = kerf_model::ColorSpace::Rec2100Pq;
    let report = check_full(&project, directory.path(), &CancellationToken::new()).unwrap();
    assert_eq!(
        row(&report, kerf_engine::qc::CODE_OUT_OF_RANGE).name,
        "Gamut, SDR copy"
    );
    assert_eq!(
        row(&report, kerf_engine::qc::CODE_LUMA_LEVELS).name,
        "Luma, SDR copy"
    );
}

#[test]
fn a_spot_mixed_at_minus_14_lufs_fails_integrated_loudness() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-14.0),
        None,
    );
    let report = run(directory.path());
    let found = with(&report, "loudness.integrated");
    assert_eq!(found.len(), 1, "{report:#?}");
    let finding = found[0];
    assert_eq!(finding.severity, Severity::Error);
    let value = finding.value.unwrap();
    assert!((-15.0..=-13.0).contains(&value), "{finding:?}");
    assert_eq!(finding.limit, Some(-22.0));
    assert_eq!(finding.unit.as_deref(), Some("LUFS"));
    assert!(
        finding.message.contains("above the −23 LUFS target ±1 LU"),
        "{finding:?}"
    );
    // At the loudest three-second window.
    let at = finding.at.unwrap();
    let duration = finding.duration.unwrap();
    assert!((duration - 3.0).abs() < 0.11, "{finding:?}");
    assert!(at + duration <= 4.0 + 1e-6, "{finding:?}");
    assert_eq!(finding.sequence.as_deref(), Some("s1"));
    let integrated = row(&report, "loudness.integrated");
    assert!(!integrated.passed);
    assert_eq!(integrated.blocking, 1);
    assert!(
        !report.valid,
        "an integrated loudness off target blocks delivery"
    );
    // True peak and range pass.
    assert!(row(&report, "loudness.true_peak").passed, "{report:#?}");
    assert!(row(&report, "loudness.range").passed);
}

#[test]
fn a_peak_at_full_scale_fails_true_peak_where_it_is() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-23.0),
        Some((2.0, 2.002, 0.999)),
    );
    let report = run(directory.path());
    let found = with(&report, "loudness.true_peak");
    assert_eq!(found.len(), 1, "{report:#?}");
    let finding = found[0];
    assert_eq!(finding.severity, Severity::Error);
    assert!(finding.value.unwrap() > -1.0, "{finding:?}");
    assert_eq!(finding.limit, Some(-1.0));
    assert_eq!(finding.unit.as_deref(), Some("dBTP"));
    // The stretch over the limit holds the burst at 2 s.
    let (at, duration) = (finding.at.unwrap(), finding.duration.unwrap());
    assert!(at <= 2.0 && 2.0 < at + duration, "{finding:?}");
    assert!(duration <= 0.31, "{finding:?}");
    assert_eq!(
        finding.timecode.as_deref(),
        Some("00:00:02:00"),
        "{finding:?}"
    );
    let peak = row(&report, "loudness.true_peak");
    assert!(!peak.passed);
    assert!(!peak.value.starts_with('−'), "{peak:?}");
    assert_eq!(peak.at, Some(at));
    assert!(row(&report, "loudness.integrated").passed, "{report:#?}");
}

#[test]
fn a_strobing_picture_fails_flash_and_black() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, true);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-23.0),
        None,
    );
    let report = run(directory.path());
    let found = with(&report, "picture.flashing");
    assert!(!found.is_empty(), "{report:#?}");
    assert!(found
        .iter()
        .all(|finding| finding.severity == Severity::Error));
    assert!(found[0].at.is_some() && found[0].duration.unwrap() > 0.0);
    let flash = row(&report, "picture.flash_black");
    assert!(!flash.passed);
    assert_eq!(flash.value, format!("{} · 0", found.len()));
    assert!(!report.valid);
}

#[test]
fn silence_is_no_signal_not_a_pass() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(directory.path(), "sound.wav", 4.0, 0.0, None);
    let report = run(directory.path());
    let integrated = row(&report, "loudness.integrated");
    assert!(!integrated.passed, "{integrated:?}");
    assert_eq!(integrated.value, "no signal");
    assert_eq!(integrated.blocking, 0, "a warning: nothing to measure");
    assert_eq!(row(&report, "loudness.range").value, "no signal");
    assert!(row(&report, "loudness.true_peak").passed);
}

#[test]
fn a_project_target_and_an_unmeasurable_program_are_reported() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let directory = TempDir::new().unwrap();
    picture(directory.path(), "picture.mp4", 4.0, false);
    tone(
        directory.path(),
        "sound.wav",
        4.0,
        amplitude_for(-16.0),
        None,
    );
    let project = spot(directory.path());
    let web = QcLimits {
        target_lufs: -16.0,
        ..QcLimits::default()
    };
    let mut progress = Vec::new();
    let report = check_full_with(
        &project,
        Some("s1"),
        directory.path(),
        &web,
        &CancellationToken::new(),
        |done| progress.push(done),
    )
    .unwrap();
    let integrated = row(&report, "loudness.integrated");
    assert!(integrated.passed, "{report:#?}");
    assert_eq!(integrated.limit.as_deref(), Some("−16 ±1"));
    assert_eq!(progress.last(), Some(&1.0));
    assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));

    // A sequence that is not there fails the measured rows as unavailable.
    let missing = check_full_with(
        &project,
        Some("s9"),
        directory.path(),
        &QcLimits::default(),
        &CancellationToken::new(),
        |_| {},
    )
    .unwrap();
    for code in [
        "loudness.integrated",
        "loudness.true_peak",
        "loudness.range",
        "picture.out_of_range",
        "picture.luma_levels",
        "picture.flash_black",
    ] {
        let row = row(&missing, code);
        assert!(!row.passed, "{row:?}");
        assert_eq!(row.value, "unavailable");
    }

    // Cancelling stops it.
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        check_full(&project, directory.path(), &cancel),
        Err(EngineError::Cancelled)
    ));
}

/// A spot of plain mid gray with a title over it, in `color`, made in `directory`.
fn spot_with_title(directory: &Path, color: &str) -> Project {
    spot_with_title_set(directory, color, "")
}

fn spot_with_title_set(directory: &Path, color: &str, set: &str) -> Project {
    tone(directory, "sound.wav", 4.0, 0.05, None);
    picture(directory, "picture.mp4", 4.0, false);
    let document = format!(
        "kerf 1
project name=\"Spot\"
media m1 path=\"picture.mp4\" rate=30 dur=00:00:04:00 video=320x180
media m2 path=\"sound.wav\" rate=30 dur=00:00:04:00 audio=2ch@48000
sequence s1 rate=30 size=320x180
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00
  track A1 audio
    clip c2 media=m2 at=00:00:00:00 dur=00:00:04:00
  track T1 video
    clip words gen=title@1 at=00:00:01:00 dur=00:00:01:00
      set text=\"Plain words\" color={color} size=0.12 {set}
"
    );
    let mut project = kerf_lang::parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|error| panic!("{error:#?}"))
        .project;
    kerf_runtime::resolve_media_paths(&mut project, directory);
    project
}

fn contrast_row(report: &CheckReport) -> Row {
    report
        .summary
        .rows
        .iter()
        .find(|row| row.code == "text.contrast")
        .expect("the row")
        .clone()
}

#[test]
fn a_full_check_says_which_text_is_too_faint() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().expect("a folder");
    let dir = directory.path();
    // White on mid gray reads, at 3.9:1, and the row passes; the first eight rows are as the mockup has
    // them.
    let fine = spot_with_title(dir, "#ffffff");
    let report = check_full(&fine, dir, &CancellationToken::new()).expect("check_full");
    let row = contrast_row(&report);
    assert!(row.passed, "{row:?}");
    assert_eq!(row.name, "Text contrast");
    assert_eq!(row.limit.as_deref(), Some("3:1"));
    assert_eq!(report.summary.rows[7].code, "picture.flash_black");
    assert_eq!(report.summary.rows[8].code, "text.contrast");
    assert_eq!(report.summary.rows[9].code, "text.clipped");
    assert_eq!(report.summary.rows[9].name, "Clipped text");
    assert!(report.summary.rows[9].passed);
    // A gray a little lighter than that, at 1.5:1, does not, and it goes to the end, only forward.
    let faint = spot_with_title(dir, "#a0a0a0");
    let mut fractions = Vec::new();
    let report = check_full_with(
        &faint,
        None,
        dir,
        &QcLimits::default(),
        &CancellationToken::new(),
        |done| fractions.push(done),
    )
    .expect("check_full_with");
    let found = with(&report, "text.contrast");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].entity.as_deref(), Some("words"));
    let row = contrast_row(&report);
    assert!(!row.passed, "{row:?}");
    assert!(
        fractions.windows(2).all(|pair| pair[1] >= pair[0] - 1e-9),
        "{fractions:?}"
    );
    assert_eq!(fractions.last(), Some(&1.0));
    // And nothing of it runs in the fast check.
    let fast = kerf_mcp::check(&faint, dir);
    assert!(fast
        .findings
        .iter()
        .all(|finding| finding.code != "text.contrast"));
    assert!(fast
        .summary
        .rows
        .iter()
        .all(|row| row.code != "text.contrast"));
}

#[test]
fn cancelling_stops_reading_text() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().expect("a folder");
    let dir = directory.path();
    let project = spot_with_title(dir, "#ffffff");
    let cancel = CancellationToken::new();
    let result = check_full_with(&project, None, dir, &QcLimits::default(), &cancel, |done| {
        if done > 0.9 {
            cancel.cancel();
        }
    });
    assert!(
        matches!(result, Err(EngineError::Cancelled)),
        "{:?}",
        result.err()
    );
}

#[test]
fn a_full_check_says_which_text_the_frame_cuts_off() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().expect("a folder");
    let dir = directory.path();
    let words = |set: &str| spot_with_title_set(dir, "#ffffff", set);
    let clipped = |project: &Project| {
        let report = check_full(project, dir, &CancellationToken::new()).expect("check_full");
        (
            with(&report, "text.clipped").len(),
            report
                .summary
                .rows
                .iter()
                .find(|row| row.code == "text.clipped")
                .expect("the row")
                .passed,
        )
    };
    // Well inside the frame, nothing; off its right edge, one, and the row fails.
    assert_eq!(clipped(&words("x=0")), (0, true));
    assert_eq!(clipped(&words("x=0.45")), (1, false));
    // Moved wholly off it, there are no glyphs left to cut.
    assert_eq!(clipped(&words("x=3")), (0, true));
    // Cut off only between the moments it is read at, on the way across and back.
    assert_eq!(
        clipped(&words("x={0f:0 11f:0 12f:0.47 13f:0 96f:0}")),
        (1, false)
    );
    assert_eq!(
        clipped(&words("x={0f:0 11f:0 12f:0.05 13f:0 96f:0}")),
        (0, true)
    );
    // A crop that takes off glyphs, and one that takes off only the empty frame around them.
    assert_eq!(clipped(&words("crop_l=0.5")), (1, false));
    assert_eq!(clipped(&words("crop_l=0.02")), (0, true));
    // A crop that moves is a reveal, and is taken as meant.
    assert_eq!(clipped(&words("crop_l={0f:0.5 24f:0}")), (0, true));
}

/// What the check costs in frames of what these add: `cargo test --release -p kerf-mcp --test qc_full
/// measure_clipped_text -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_clipped_text() {
    if !ffmpeg_available() {
        return;
    }
    let directory = TempDir::new().expect("a folder");
    let dir = directory.path();
    for set in [
        "x=0",
        "x={0f:0 11f:0 12f:0.47 13f:0 96f:0}",
        "crop_l=0.5",
        "x=0.45",
    ] {
        let project = spot_with_title_set(dir, "#ffffff", set);
        let mut times: Vec<f64> = (0..5)
            .map(|_| {
                let began = std::time::Instant::now();
                check_full(&project, dir, &CancellationToken::new()).expect("check_full");
                began.elapsed().as_secs_f64()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        eprintln!("{set:<40} {:.3} s", times[2]);
    }
}
