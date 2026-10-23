//! Render argument construction and ffmpeg spawn for `kerf-model` projects.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::encoder::{
    encode_output_args, parse_encoder_capabilities, select_video_encoder, EncoderSelection,
    SelectEncoderOptions, VideoCodec,
};
use crate::error::{EngineError, Result};
use crate::ffmpeg::{
    cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, tail, video_decode_prefix,
};
use crate::program::{compile_program, ProgramOptions};
use crate::resources::{budget, should_try_hardware_encode};
use crate::sidecar::{self, CancellationToken};

const CANCEL_FILE_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub fn detect_and_select_encoder(options: &SelectEncoderOptions) -> Result<EncoderSelection> {
    let ffmpeg = resolve_ffmpeg_bin();
    let listing = cached_encoder_listing()?;
    let capabilities = parse_encoder_capabilities(listing, ffmpeg);
    Ok(select_video_encoder(&capabilities.encoders, options))
}

fn draft_post_filters() -> Vec<String> {
    vec!["scale=-2:360".into(), "fps=15".into()]
}

fn draft_codec_args() -> Vec<String> {
    vec![
        "-crf".into(),
        "35".into(),
        "-preset".into(),
        "ultrafast".into(),
    ]
}

fn production_codec_args() -> Vec<String> {
    let budget = budget();
    vec![
        "-c:v".into(),
        "libx264".into(),
        "-crf".into(),
        budget.software_crf.to_string(),
        "-preset".into(),
        budget.software_preset.to_string(),
    ]
}

fn hardware_or_software_codec(
    prefer_hardware: bool,
    force_software: bool,
    codec: VideoCodec,
) -> Result<Vec<String>> {
    if !should_try_hardware_encode(prefer_hardware, force_software) {
        return Ok(production_codec_args());
    }
    let select = SelectEncoderOptions {
        prefer_hardware: true,
        codec,
        ..Default::default()
    };
    let selection = detect_and_select_encoder(&select)?;
    let budget = budget();
    Ok(encode_output_args(
        &selection,
        budget.software_crf,
        budget.software_preset,
    ))
}

/// How to encode a rendered program.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub output: String,
    /// 360p at 15 fps with fast, low-quality encoding.
    pub draft: bool,
    pub prefer_hardware: bool,
    pub force_software: bool,
    pub codec: VideoCodec,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            output: String::new(),
            draft: false,
            prefer_hardware: false,
            force_software: false,
            codec: VideoCodec::H264,
        }
    }
}

/// The ffmpeg arguments that render `sequence` (the first one when `None`)
/// of `project`, or `[from, to)` of it, and the output duration in seconds.
pub fn render_program_args(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
) -> Result<(Vec<String>, f64)> {
    let compiled = compile_program(
        project,
        kerf_model::Catalog::builtin(),
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            range,
            video_post_filters: if opts.draft {
                draft_post_filters()
            } else {
                Vec::new()
            },
        },
    )?;
    let mut args: Vec<String> = video_decode_prefix().to_vec();
    args.extend(compiled.argv.iter().skip(1).cloned());
    args.extend(if opts.draft {
        draft_codec_args()
    } else {
        hardware_or_software_codec(opts.prefer_hardware, opts.force_software, opts.codec)?
    });
    args.extend([
        "-r".into(),
        compiled.rate_arg(),
        "-t".into(),
        crate::program::secs(compiled.duration),
        opts.output.clone(),
    ]);
    Ok((args, compiled.duration.seconds()))
}

/// Render a project with a caller-owned cancellation token.
pub fn render_program_with_cancellation<F>(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
    cancellation: &CancellationToken,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let (args, duration) = render_program_args(project, sequence, range, opts)?;
    run_ffmpeg_with_progress(&args, duration, cancellation, on_progress)
}

pub fn render_program<F>(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let cancellation = CancellationToken::new();
    render_program_with_cancellation(project, sequence, range, opts, &cancellation, on_progress)
}

fn extract_time(line: &str) -> Option<&str> {
    let index = line.find("time=")?;
    let rest = &line[index + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Run ffmpeg to completion, reporting progress through `duration` seconds
/// of output when it is positive.
pub(crate) fn run_ffmpeg_with_progress<F>(
    args: &[String],
    duration: f64,
    cancellation: &CancellationToken,
    mut on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let mut command = Command::new(resolve_ffmpeg_bin());
    command.args(args);
    let finished = sidecar::run(command, cancellation, |line| {
        let Some(seconds) = extract_time(line).and_then(parse_time_seconds) else {
            return;
        };
        if let (Some(callback), true) = (on_progress.as_mut(), duration > 0.0) {
            callback((seconds / duration).clamp(0.0, 1.0));
        }
    })?;
    if finished.status.success() {
        return Ok(());
    }
    let detail = finished.stderr.trim();
    Err(EngineError::Ffmpeg {
        code: finished.status.code().unwrap_or(-1),
        stderr: if detail.is_empty() {
            "ffmpeg failed".into()
        } else {
            tail(detail, 2000).to_owned()
        },
    })
}

/// Watch a project-local cancellation marker for one render invocation.
pub fn spawn_cancel_file_watcher(
    path: PathBuf,
    cancellation: CancellationToken,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !cancellation.is_cancelled() {
            if path.is_file() {
                cancellation.cancel();
                break;
            }
            thread::sleep(CANCEL_FILE_POLL_INTERVAL);
        }
    })
}

/// Progress callback that writes `{"progress":n}` JSON lines to stderr.
pub fn stderr_progress_callback() -> impl FnMut(f64) {
    let mut last = -1.0_f64;
    move |ratio: f64| {
        let rounded = (ratio * 10_000.0).round() / 10_000.0;
        if (rounded - last).abs() < 0.0001 {
            return;
        }
        last = rounded;
        let _ = writeln!(std::io::stderr(), r#"{{"progress":{rounded}}}"#);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Ten seconds of one source, cut at 24 fps.
    fn project() -> kerf_model::Project {
        kerf_lang::parse_document(
            "kerf 1\nproject name=\"render\"\n\
             media s path=\"/tmp/s.mp4\" rate=24 dur=30s video=160x120\n\
             sequence s1 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip v media=s src=10s at=0f dur=10s\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("project")
        .project
    }

    #[test]
    fn production_software_uses_budgeted_x264() {
        let (args, _) = render_program_args(
            &project(),
            None,
            None,
            &RenderOptions {
                output: "out.mp4".into(),
                force_software: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v" && pair[1] == "libx264"));
        assert!(args
            .iter()
            .any(|arg| arg == "veryfast" || arg == "faster" || arg == "medium"));
        assert!(args.windows(2).any(|pair| pair[0] == "-threads"));
    }

    #[test]
    fn draft_renders_scale_down_with_fast_encoding() {
        let (args, _) = render_program_args(
            &project(),
            None,
            None,
            &RenderOptions {
                output: "out.mp4".into(),
                draft: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].as_str())
            .expect("filter_complex");
        assert!(graph.contains("scale=-2:360") && graph.contains("fps=15"));
        assert!(args.iter().any(|arg| arg == "ultrafast"));
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn range_render_args_seek_before_input_not_after_the_graph() {
        let rate = kerf_model::Rate::FPS_24;
        let (args, duration) = render_program_args(
            &project(),
            None,
            Some((rate.time_of(48), rate.time_of(120))),
            &RenderOptions {
                output: "out.mp4".into(),
                draft: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        assert_eq!(duration, 3.0);
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .expect("filter");
        assert!(args[..graph].windows(2).any(|pair| pair[0] == "-ss"));
        assert!(!args[graph..].windows(2).any(|pair| pair[0] == "-ss"));
    }

    #[test]
    fn cancellation_marker_only_cancels_its_own_render_token() {
        let directory = tempfile::tempdir().expect("temp directory");
        let first_path = directory.path().join("first.cancel");
        let second_path = directory.path().join("second.cancel");
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        let first_watcher = spawn_cancel_file_watcher(first_path.clone(), first.clone());
        let second_watcher = spawn_cancel_file_watcher(second_path, second.clone());
        fs::write(&first_path, b"cancelled\n").expect("cancel marker");
        for _ in 0..100 {
            if first.is_cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        second.cancel();
        first_watcher.join().expect("first watcher");
        second_watcher.join().expect("second watcher");
    }
}
