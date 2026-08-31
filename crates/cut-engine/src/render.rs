//! Render argument construction and ffmpeg spawn for v1/v2 timelines.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::compile::{compile_filter_graph_v2, timeline_duration_v2, CompileV2Options};
use crate::compile_v1::{compile_filter_graph_v1, timeline_duration_v1};
use crate::encoder::{
    encode_output_args, parse_encoder_capabilities, select_video_encoder, EncoderSelection,
    SelectEncoderOptions, VideoCodec,
};
use crate::error::{EngineError, Result};
use crate::ffmpeg::{
    cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, video_decode_prefix,
};
use crate::resources::{budget, should_try_hardware_encode};

#[derive(Debug, Clone)]
pub struct RenderV1Options {
    pub output: String,
    pub draft: bool,
}

#[derive(Debug, Clone)]
pub struct RenderV2Options {
    pub output: String,
    pub draft: bool,
    pub include_embedded_audio: Option<bool>,
    pub prefer_hardware: bool,
    pub force_software: bool,
    pub codec: VideoCodec,
}

impl Default for RenderV2Options {
    fn default() -> Self {
        Self {
            output: String::new(),
            draft: false,
            include_embedded_audio: None,
            prefer_hardware: false,
            force_software: false,
            codec: VideoCodec::H264,
        }
    }
}

pub fn detect_and_select_encoder(options: &SelectEncoderOptions) -> Result<EncoderSelection> {
    let ffmpeg = resolve_ffmpeg_bin();
    let listing = cached_encoder_listing()?;
    let capabilities = parse_encoder_capabilities(listing, &ffmpeg);
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
    let mut select = SelectEncoderOptions::default();
    select.prefer_hardware = true;
    select.codec = codec;
    let selection = detect_and_select_encoder(&select)?;
    let budget = budget();
    Ok(encode_output_args(
        &selection,
        budget.software_crf,
        budget.software_preset,
    ))
}

pub fn render_args_v1(timeline: &Value, opts: &RenderV1Options) -> Result<Vec<String>> {
    let video_post_filters = if opts.draft {
        draft_post_filters()
    } else {
        Vec::new()
    };
    let compiled =
        compile_filter_graph_v1(timeline, &video_post_filters).map_err(EngineError::CompileV1)?;
    let mut base: Vec<String> = video_decode_prefix().to_vec();
    base.extend(compiled.argv.iter().skip(1).cloned());
    if base.len() >= 2 {
        base.truncate(base.len() - 2);
    }
    let codec = if opts.draft {
        draft_codec_args()
    } else {
        hardware_or_software_codec(false, false, VideoCodec::H264)?
    };
    let fps = timeline.get("fps").and_then(Value::as_f64).unwrap_or(30.0);
    base.extend(codec);
    base.push("-r".into());
    base.push(format!("{fps}"));
    base.push(opts.output.clone());
    Ok(base)
}

pub fn render_args_v2(project: &Value, opts: &RenderV2Options) -> Result<Vec<String>> {
    let compile_opts = CompileV2Options {
        video_post_filters: if opts.draft {
            draft_post_filters()
        } else {
            Vec::new()
        },
        include_embedded_audio: opts.include_embedded_audio,
    };
    let compiled = compile_filter_graph_v2(project, compile_opts)?;
    let mut base: Vec<String> = video_decode_prefix().to_vec();
    base.extend(compiled.argv.iter().skip(1).cloned());
    if base.len() >= 2 {
        base.truncate(base.len() - 2);
    }
    let codec = if opts.draft {
        draft_codec_args()
    } else {
        hardware_or_software_codec(opts.prefer_hardware, opts.force_software, opts.codec)?
    };
    let fps = project.get("fps").and_then(Value::as_f64).unwrap_or(30.0);
    base.extend(codec);
    base.push("-r".into());
    base.push(format!("{fps}"));
    base.push(opts.output.clone());
    Ok(base)
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

    fn wait(mut self, stderr: &str) -> Result<()> {
        let status = self.child.wait()?;
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

fn interrupt_flag() -> &'static AtomicBool {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    FLAG.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        #[cfg(unix)]
        {
            let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&flag));
            let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag));
        }
        flag
    })
}

fn emit_progress_line(ratio: f64) {
    let _ = writeln!(std::io::stderr(), r#"{{"progress":{ratio}}}"#);
}

fn run_ffmpeg_with_progress<F>(
    args: &[String],
    duration: f64,
    mut on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let interrupted = interrupt_flag();
    interrupted.store(false, Ordering::SeqCst);
    let child_slot: Arc<Mutex<Option<FfmpegChild>>> = Arc::new(Mutex::new(None));

    let mut ffmpeg = FfmpegChild::spawn(args)?;
    let stderr = ffmpeg.take_stderr();

    {
        let mut guard = child_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(ffmpeg);
    }

    let mut stderr_tail = String::new();
    if let Some(stderr) = stderr {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(std::io::Result::ok) {
            if interrupted.load(Ordering::SeqCst) {
                if let Ok(mut guard) = child_slot.lock() {
                    if let Some(child) = guard.as_mut() {
                        child.kill();
                    }
                }
                break;
            }
            if stderr_tail.len() > 4_000 {
                stderr_tail = stderr_tail[stderr_tail.len() - 2_000..].to_string();
            }
            stderr_tail.push_str(&line);
            stderr_tail.push('\n');
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

    let mut ffmpeg = child_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .ok_or_else(|| EngineError::msg("ffmpeg child missing"))?;
    if interrupted.load(Ordering::SeqCst) {
        ffmpeg.kill();
        return Err(EngineError::msg("render interrupted"));
    }
    ffmpeg.wait(&stderr_tail)
}

pub fn render_v1<F>(timeline: &Value, opts: &RenderV1Options, on_progress: Option<F>) -> Result<()>
where
    F: FnMut(f64),
{
    let duration = timeline_duration_v1(timeline).map_err(EngineError::CompileV1)?;
    let args = render_args_v1(timeline, opts)?;
    run_ffmpeg_with_progress(&args, duration, on_progress)
}

pub fn render_v2<F>(project: &Value, opts: &RenderV2Options, on_progress: Option<F>) -> Result<()>
where
    F: FnMut(f64),
{
    let duration = timeline_duration_v2(project)?;
    let args = render_args_v2(project, opts)?;
    run_ffmpeg_with_progress(&args, duration, on_progress)
}

/// Run ffmpeg for proxy transcode with optional stderr progress lines.
pub fn run_proxy_ffmpeg<F>(
    args: &[String],
    duration: Option<f64>,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let total = duration.filter(|value| *value > 0.0).unwrap_or(0.0);
    run_ffmpeg_with_progress(args, total, on_progress)
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
    use serde_json::json;

    #[test]
    fn render_args_v1_draft_contains_ultrafast_and_scale() {
        let timeline = json!({
            "version": 1,
            "fps": 30,
            "sources": [
                { "id": "a", "path": "/tmp/a.mp4", "hasAudio": false }
            ],
            "tracks": [{
                "id": "t1",
                "clips": [{ "id": "c1", "src": "a", "in": 0, "out": 2 }]
            }],
            "overlays": []
        });
        let args = render_args_v1(
            &timeline,
            &RenderV1Options {
                output: "out.mp4".into(),
                draft: true,
            },
        )
        .expect("args");
        let fc = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].as_str())
            .expect("filter_complex");
        assert!(fc.contains("scale=-2:360"));
        assert!(fc.contains("fps=15"));
        assert!(args.iter().any(|arg| arg == "ultrafast"));
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn production_software_uses_budgeted_x264() {
        let project = json!({
            "version": 2,
            "fps": 24,
            "width": 160,
            "height": 120,
            "sources": [
                { "id": "s", "path": "/tmp/s.mp4", "width": 160, "height": 120, "hasAudio": false }
            ],
            "tracks": [{
                "id": "video",
                "type": "video",
                "clips": [{ "id": "v", "sourceId": "s", "in": 0, "out": 1, "timelineStart": 0 }]
            }]
        });
        let args = render_args_v2(
            &project,
            &RenderV2Options {
                output: "out.mp4".into(),
                draft: false,
                force_software: true,
                ..RenderV2Options::default()
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
}
