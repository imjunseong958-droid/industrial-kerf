//! Render argument construction and ffmpeg spawn for `cut-model` projects.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::encoder::{
    encode_output_args, parse_encoder_capabilities, select_video_encoder, EncoderSelection,
    SelectEncoderOptions, VideoCodec,
};
use crate::error::{EngineError, Result};
use crate::ffmpeg::{
    cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, video_decode_prefix,
};
use crate::program::{compile_program, ProgramOptions};
use crate::resources::{budget, should_try_hardware_encode};
use crate::transcription::CancellationToken;

const RENDER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const STDERR_QUEUE_CAPACITY: usize = 128;

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
    project: &cut_model::Project,
    sequence: Option<&str>,
    range: Option<(cut_model::Time, cut_model::Time)>,
    opts: &RenderOptions,
) -> Result<(Vec<String>, f64)> {
    let compiled = compile_program(
        project,
        cut_model::Catalog::builtin(),
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
    project: &cut_model::Project,
    sequence: Option<&str>,
    range: Option<(cut_model::Time, cut_model::Time)>,
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
    project: &cut_model::Project,
    sequence: Option<&str>,
    range: Option<(cut_model::Time, cut_model::Time)>,
    opts: &RenderOptions,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let cancellation = CancellationToken::new();
    render_program_with_cancellation(project, sequence, range, opts, &cancellation, on_progress)
}

struct FfmpegChild {
    child: Child,
}

impl FfmpegChild {
    fn spawn(args: &[String]) -> Result<Self> {
        let ffmpeg = resolve_ffmpeg_bin();
        let child = Command::new(ffmpeg)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self { child })
    }

    fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
    }

    fn reap_after_kill(&mut self) {
        let _ = self.child.wait();
    }
}

impl Drop for FfmpegChild {
    fn drop(&mut self) {
        self.kill();
    }
}

fn extract_time(line: &str) -> Option<&str> {
    let index = line.find("time=")?;
    let rest = &line[index + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    Some(&rest[..end])
}

fn tail_stderr(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else {
        value[value.len() - max..].to_string()
    }
}

fn emit_progress_line(ratio: f64) {
    let _ = writeln!(std::io::stderr(), r#"{{"progress":{ratio}}}"#);
}

fn drain_stderr<F>(
    receiver: &Receiver<String>,
    stderr_tail: &mut String,
    duration: f64,
    on_progress: &mut Option<F>,
) where
    F: FnMut(f64),
{
    for line in receiver.try_iter() {
        stderr_tail.push_str(&line);
        stderr_tail.push('\n');
        if stderr_tail.len() > 4_000 {
            let mut start = stderr_tail.len() - 2_000;
            while start < stderr_tail.len() && !stderr_tail.is_char_boundary(start) {
                start += 1;
            }
            stderr_tail.drain(..start);
        }
        if let Some(time_str) = extract_time(&line) {
            if let Some(seconds) = parse_time_seconds(time_str) {
                if duration > 0.0 {
                    let ratio = (seconds / duration).clamp(0.0, 1.0);
                    if let Some(callback) = on_progress.as_mut() {
                        callback(ratio);
                    }
                }
            }
        }
    }
}

fn finish_stderr_reader<F>(
    reader: thread::JoinHandle<()>,
    receiver: &Receiver<String>,
    stderr_tail: &mut String,
    duration: f64,
    on_progress: &mut Option<F>,
) where
    F: FnMut(f64),
{
    while !reader.is_finished() {
        drain_stderr(receiver, stderr_tail, duration, on_progress);
        thread::sleep(RENDER_POLL_INTERVAL);
    }
    let _ = reader.join();
    drain_stderr(receiver, stderr_tail, duration, on_progress);
}

fn result_from_status(status: std::process::ExitStatus, stderr: &str) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        let detail = stderr.trim();
        Err(EngineError::Ffmpeg {
            code: status.code().unwrap_or(-1),
            stderr: if detail.is_empty() {
                "ffmpeg failed".into()
            } else {
                tail_stderr(detail, 2000)
            },
        })
    }
}

fn run_ffmpeg_with_progress<F>(
    args: &[String],
    duration: f64,
    cancellation: &CancellationToken,
    mut on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let mut ffmpeg = FfmpegChild::spawn(args)?;
    let stderr = ffmpeg.take_stderr().ok_or_else(|| {
        ffmpeg.kill();
        ffmpeg.reap_after_kill();
        EngineError::msg("ffmpeg stderr pipe is unavailable")
    })?;
    let (sender, receiver) = mpsc::sync_channel::<String>(STDERR_QUEUE_CAPACITY);
    let reader = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(std::result::Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut stderr_tail = String::new();
    loop {
        drain_stderr(&receiver, &mut stderr_tail, duration, &mut on_progress);
        if cancellation.is_cancelled() {
            ffmpeg.kill();
            ffmpeg.reap_after_kill();
            finish_stderr_reader(
                reader,
                &receiver,
                &mut stderr_tail,
                duration,
                &mut on_progress,
            );
            return Err(EngineError::Cancelled);
        }
        let status = match ffmpeg.child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                ffmpeg.kill();
                ffmpeg.reap_after_kill();
                finish_stderr_reader(
                    reader,
                    &receiver,
                    &mut stderr_tail,
                    duration,
                    &mut on_progress,
                );
                return Err(error.into());
            }
        };
        if let Some(status) = status {
            finish_stderr_reader(
                reader,
                &receiver,
                &mut stderr_tail,
                duration,
                &mut on_progress,
            );
            return result_from_status(status, &stderr_tail);
        }
        thread::sleep(RENDER_POLL_INTERVAL);
    }
}

/// Run ffmpeg for proxy transcode with optional stderr progress lines.
pub fn run_proxy_ffmpeg_with_cancellation<F>(
    args: &[String],
    duration: Option<f64>,
    cancellation: &CancellationToken,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let total = duration.filter(|value| *value > 0.0).unwrap_or(0.0);
    run_ffmpeg_with_progress(args, total, cancellation, on_progress)
}

pub fn run_proxy_ffmpeg<F>(
    args: &[String],
    duration: Option<f64>,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let cancellation = CancellationToken::new();
    run_proxy_ffmpeg_with_cancellation(args, duration, &cancellation, on_progress)
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
            thread::sleep(RENDER_POLL_INTERVAL);
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
        emit_progress_line(rounded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Ten seconds of one source, cut at 24 fps.
    fn project() -> cut_model::Project {
        cut_lang::parse_document(
            "cut 1\nproject name=\"render\"\n\
             media s path=\"/tmp/s.mp4\" rate=24 dur=30s video=160x120\n\
             sequence s1 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip v media=s src=10s at=0f dur=10s\n",
            cut_model::Catalog::builtin(),
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
        let rate = cut_model::Rate::FPS_24;
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
