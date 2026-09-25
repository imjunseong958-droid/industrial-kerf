//! Probe parsers and ffmpeg/ffprobe wrappers.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::SystemTime;

use crate::derivatives::DerivativeCache;
use crate::error::Result;
use crate::ffmpeg::{resolve_ffmpeg_bin, resolve_ffprobe_bin, run, video_decode_prefix};
use crate::resources::extract_job_count;

static SILENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"silence_(start|end):\s*(-?\d+(?:\.\d+)?)").expect("silence regex")
});
static PTS_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"pts_time:(-?\d+(?:\.\d+)?)").expect("pts regex"));
static SCENE_SCORE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"lavfi\.scene_score=(-?\d+(?:\.\d+)?)").expect("score regex"));

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(rename = "hasAudio", skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(rename = "videoCodec", skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(rename = "audioCodec", skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilenceRange {
    pub start: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneRange {
    pub start: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedFrame {
    pub time: f64,
    pub path: String,
}

/// One exact timestamp frame stored in the derivative cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameGrab {
    pub time: f64,
    pub path: String,
    pub media_type: String,
}

/// Cached result of an exact frame request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameGrabSet {
    pub key: String,
    pub frames: Vec<FrameGrab>,
    pub cached: bool,
}

/// Cached contact sheet built from exact timestamp frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilmstripResult {
    pub key: String,
    pub path: String,
    pub frame_paths: Vec<String>,
    pub times: Vec<f64>,
    pub columns: usize,
    pub cached: bool,
}

/// One deterministic min/max/RMS bucket in a waveform envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformBucket {
    pub min: f64,
    pub max: f64,
    pub rms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaveformPayload {
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    pub duration: f64,
    pub sample_rate: u32,
    pub buckets: Vec<WaveformBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

/// Cached waveform envelope and optional waveform image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformResult {
    pub key: String,
    /// Source time covered by this waveform. Full-source requests use
    /// `0..duration`; ranged requests retain their selected interval.
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    pub sample_rate: u32,
    pub buckets: Vec<WaveformBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    pub cached: bool,
}

pub const MAX_FRAME_TIMES: usize = 128;
pub const MAX_WAVEFORM_BUCKETS: usize = 4096;
const WAVEFORM_SAMPLE_RATE: u32 = 8_000;

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    format: Option<FfprobeFormat>,
    streams: Option<Vec<FfprobeStream>>,
}

pub fn parse_rate(rate: Option<&str>) -> Option<f64> {
    let rate = rate?;
    let mut parts = rate.split('/');
    let numerator: f64 = parts.next()?.parse().ok()?;
    let denominator: f64 = parts.next()?.parse().ok()?;
    if !numerator.is_finite() || !denominator.is_finite() || denominator == 0.0 {
        return None;
    }
    Some(numerator / denominator)
}

pub fn parse_ffprobe_json(stdout: &str) -> Result<ProbeInfo> {
    let parsed: FfprobeOutput = serde_json::from_str(stdout)?;
    let streams = parsed.streams.unwrap_or_default();
    let video = streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let mut result = ProbeInfo {
        has_audio: Some(audio.is_some()),
        ..ProbeInfo::default()
    };
    if let Some(duration) = parsed.format.and_then(|format| format.duration) {
        if let Ok(value) = duration.parse::<f64>() {
            result.duration = Some(value);
        }
    }
    if let Some(video) = video {
        result.width = video.width;
        result.height = video.height;
        result.fps = parse_rate(video.avg_frame_rate.as_deref())
            .or_else(|| parse_rate(video.r_frame_rate.as_deref()));
        result.video_codec = video.codec_name.clone();
    }
    if let Some(audio) = audio {
        result.audio_codec = audio.codec_name.clone();
    }
    Ok(result)
}

/// Parse silencedetect stderr output into ranges. Missing trailing silence_end is EOF.
pub fn parse_silencedetect(stderr: &str) -> Vec<SilenceRange> {
    let mut ranges = Vec::new();
    let mut open_start: Option<f64> = None;
    for capture in SILENCE_RE.captures_iter(stderr) {
        let kind = capture.get(1).map(|value| value.as_str()).unwrap_or("");
        let time: f64 = capture
            .get(2)
            .and_then(|value| value.as_str().parse().ok())
            .unwrap_or(0.0);
        if kind == "start" {
            open_start = Some(time);
        } else if let Some(start) = open_start.take() {
            ranges.push(SilenceRange {
                start,
                end: Some(time),
            });
        }
    }
    if let Some(start) = open_start {
        ranges.push(SilenceRange { start, end: None });
    }
    ranges
}

/// Parse scene-cut timestamps from lavfi.scene_score metadata print output.
pub fn parse_scene_times(stderr: &str) -> Vec<SceneRange> {
    let mut cuts = Vec::new();
    let mut pending_time: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(capture) = PTS_TIME_RE.captures(line) {
            pending_time = capture.get(1).and_then(|value| value.as_str().parse().ok());
            continue;
        }
        if SCENE_SCORE_RE.is_match(line) {
            if let Some(time) = pending_time.take() {
                cuts.push(time);
            }
        }
    }
    cuts.iter()
        .enumerate()
        .map(|(index, &start)| SceneRange {
            start,
            end: cuts.get(index + 1).copied(),
        })
        .collect()
}

struct ProbeCacheEntry {
    len: u64,
    modified: Option<SystemTime>,
    info: ProbeInfo,
}

fn probe_cache() -> &'static Mutex<HashMap<String, ProbeCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ProbeCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn probe_identity(src: &str) -> Option<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(src).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

pub fn info(src: &str) -> Result<ProbeInfo> {
    let identity = probe_identity(src);
    if let Some((len, modified)) = identity {
        let cache = probe_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get(src) {
            if entry.len == len && entry.modified == modified {
                return Ok(entry.info.clone());
            }
        }
    }
    let output = run(
        resolve_ffprobe_bin(),
        &[
            "-v".into(),
            "quiet".into(),
            "-print_format".into(),
            "json".into(),
            "-show_format".into(),
            "-show_streams".into(),
            src.into(),
        ],
    )?;
    let info = parse_ffprobe_json(&output.stdout)?;
    if let Some((len, modified)) = identity.or_else(|| probe_identity(src)) {
        let mut cache = probe_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            src.to_string(),
            ProbeCacheEntry {
                len,
                modified,
                info: info.clone(),
            },
        );
    }
    Ok(info)
}

pub fn silence_detect(
    src: &str,
    threshold_db: f64,
    min_duration_sec: f64,
) -> Result<Vec<SilenceRange>> {
    let output = run(
        resolve_ffmpeg_bin(),
        &[
            "-hide_banner".into(),
            "-nostats".into(),
            "-i".into(),
            src.into(),
            "-af".into(),
            format!("silencedetect=noise={threshold_db}dB:d={min_duration_sec}"),
            "-f".into(),
            "null".into(),
            "-".into(),
        ],
    )?;
    Ok(parse_silencedetect(&output.stderr))
}

pub fn detect_scenes(src: &str, threshold: f64) -> Result<Vec<SceneRange>> {
    let mut args = video_decode_prefix().to_vec();
    args.extend([
        "-hide_banner".into(),
        "-nostats".into(),
        "-i".into(),
        src.into(),
        "-vf".into(),
        format!("select='gt(scene,{threshold})',metadata=print"),
        "-an".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    let output = run(resolve_ffmpeg_bin(), &args)?;
    Ok(parse_scene_times(&output.stderr))
}

fn extract_one_frame(
    ffmpeg: &str,
    src: &str,
    time: f64,
    scale: &str,
    out_path_str: String,
) -> Result<ExtractedFrame> {
    let mut args = video_decode_prefix().to_vec();
    args.extend([
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        time.to_string(),
        "-i".into(),
        src.into(),
        "-an".into(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        scale.into(),
        "-y".into(),
        out_path_str.clone(),
    ]);
    run(ffmpeg, &args)?;
    Ok(ExtractedFrame {
        time,
        path: out_path_str,
    })
}

pub fn extract_frames(
    src: &str,
    times: &[f64],
    out_dir: &str,
    max_width: u32,
) -> Result<Vec<ExtractedFrame>> {
    if max_width == 0 {
        return Err(crate::error::EngineError::msg(format!(
            "extractFrames maxWidth must be finite and positive, got {max_width}"
        )));
    }
    std::fs::create_dir_all(out_dir)?;
    let width = max_width.max(1);
    let scale = format!("scale=w='min(iw,{width})':h=-2");
    let ffmpeg = resolve_ffmpeg_bin();
    let jobs = extract_job_count().max(1);
    let mut results: Vec<Option<ExtractedFrame>> = vec![None; times.len()];
    let mut index = 0usize;
    while index < times.len() {
        let end = (index + jobs).min(times.len());
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(end - index);
            for (offset, &time) in times[index..end].iter().enumerate() {
                let slot = index + offset;
                let out_path = std::path::Path::new(out_dir).join(format!("frame_{slot}.png"));
                let out_path_str = out_path.to_string_lossy().into_owned();
                let scale = scale.as_str();
                handles.push((
                    slot,
                    scope.spawn(move || extract_one_frame(ffmpeg, src, time, scale, out_path_str)),
                ));
            }
            for (slot, handle) in handles {
                let frame = handle.join().map_err(|_| {
                    crate::error::EngineError::msg("frame extraction thread panicked")
                })??;
                results[slot] = Some(frame);
            }
            Ok::<(), crate::error::EngineError>(())
        })?;
        index = end;
    }
    results
        .into_iter()
        .map(|frame| frame.ok_or_else(|| crate::error::EngineError::msg("missing extracted frame")))
        .collect()
}

fn validate_exact_times(times: &[f64]) -> Result<()> {
    if times.is_empty() {
        return Err(crate::error::EngineError::msg(
            "at least one frame timestamp is required",
        ));
    }
    if times.len() > MAX_FRAME_TIMES {
        return Err(crate::error::EngineError::msg(format!(
            "frame timestamp count exceeds {MAX_FRAME_TIMES}"
        )));
    }
    if times.iter().any(|time| !time.is_finite() || *time < 0.0) {
        return Err(crate::error::EngineError::msg(
            "frame timestamps must be finite and nonnegative",
        ));
    }
    Ok(())
}

fn frame_file_name(index: usize) -> String {
    format!("frame-{index:03}.png")
}

fn frame_file_names(count: usize) -> Vec<String> {
    (0..count).map(frame_file_name).collect()
}

fn extract_one_exact_frame(
    ffmpeg: &str,
    src: &str,
    time: f64,
    scale: &str,
    output: String,
) -> Result<FrameGrab> {
    let preroll = 0.5_f64.min(time);
    let mut args = video_decode_prefix().to_vec();
    args.extend([
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        (time - preroll).to_string(),
        "-i".into(),
        src.into(),
        "-ss".into(),
        preroll.to_string(),
        "-an".into(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        scale.into(),
        "-y".into(),
        output.clone(),
    ]);
    run(ffmpeg, &args)?;
    Ok(FrameGrab {
        time,
        path: output,
        media_type: "image/png".into(),
    })
}

fn extract_exact_frames_to_dir(
    src: &str,
    times: &[f64],
    out_dir: &Path,
    max_width: u32,
) -> Result<Vec<FrameGrab>> {
    validate_exact_times(times)?;
    if max_width == 0 {
        return Err(crate::error::EngineError::msg(
            "maxWidth must be finite and positive",
        ));
    }
    fs::create_dir_all(out_dir)?;
    let scale = format!("scale=w='min(iw,{max_width})':h=-2");
    let ffmpeg = resolve_ffmpeg_bin();
    let jobs = extract_job_count().max(1);
    let names = frame_file_names(times.len());
    let mut results: Vec<Option<FrameGrab>> = vec![None; times.len()];
    let mut index = 0usize;
    while index < times.len() {
        let end = (index + jobs).min(times.len());
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(end - index);
            for slot in index..end {
                let output = out_dir.join(&names[slot]).to_string_lossy().into_owned();
                let scale = scale.as_str();
                handles.push((
                    slot,
                    scope.spawn(move || {
                        extract_one_exact_frame(ffmpeg, src, times[slot], scale, output)
                    }),
                ));
            }
            for (slot, handle) in handles {
                results[slot] = Some(handle.join().map_err(|_| {
                    crate::error::EngineError::msg("exact frame extraction thread panicked")
                })??);
            }
            Ok::<(), crate::error::EngineError>(())
        })?;
        index = end;
    }
    results
        .into_iter()
        .map(|frame| {
            frame.ok_or_else(|| crate::error::EngineError::msg("missing exact extracted frame"))
        })
        .collect()
}

/// Extract exact timestamp frames into a caller-selected derivative directory.
pub fn extract_exact_frames(
    src: &str,
    times: &[f64],
    out_dir: &Path,
    max_width: u32,
) -> Result<Vec<FrameGrab>> {
    extract_exact_frames_to_dir(src, times, out_dir, max_width)
}

/// Extract frames at exact requested timestamps and cache the result below the
/// project's `.cut/cache/derived` directory.
pub fn cached_exact_frames(
    src: &str,
    cache: &DerivativeCache,
    times: &[f64],
    max_width: u32,
) -> Result<FrameGrabSet> {
    validate_exact_times(times)?;
    if max_width == 0 {
        return Err(crate::error::EngineError::msg(
            "maxWidth must be finite and positive",
        ));
    }
    let options = json!({
        "exact": true,
        "times": times,
        "maxWidth": max_width,
    });
    let entry = cache.source_entry(Path::new(src), "frames", &options)?;
    let names = frame_file_names(times.len());
    let required = names.iter().map(String::as_str).collect::<Vec<_>>();
    let cached = entry.is_complete(&required);
    if !cached {
        extract_exact_frames_to_dir(src, times, &entry.directory, max_width)?;
        entry.mark_complete(&required)?;
    }
    let frames = times
        .iter()
        .zip(names)
        .map(|(time, name)| FrameGrab {
            time: *time,
            path: entry.path(&name).to_string_lossy().into_owned(),
            media_type: "image/png".into(),
        })
        .collect();
    Ok(FrameGrabSet {
        key: entry.key,
        frames,
        cached,
    })
}

/// Build a contact sheet from an ordered set of PNG frame paths.
pub fn create_contact_sheet(frame_paths: &[String], output: &Path, columns: usize) -> Result<()> {
    if frame_paths.is_empty() {
        return Err(crate::error::EngineError::msg(
            "contact sheet requires at least one frame",
        ));
    }
    if !(1..=16).contains(&columns) {
        return Err(crate::error::EngineError::msg(
            "contact sheet columns must be between 1 and 16",
        ));
    }
    let first = Path::new(&frame_paths[0]);
    let parent = first
        .parent()
        .ok_or_else(|| crate::error::EngineError::msg("frame path has no parent directory"))?;
    if frame_paths
        .iter()
        .any(|path| Path::new(path).parent() != Some(parent))
    {
        return Err(crate::error::EngineError::msg(
            "contact sheet frames must share a directory",
        ));
    }
    let rows = frame_paths.len().div_ceil(columns);
    let pattern = parent.join("frame-%03d.png");
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-framerate".into(),
        "1".into(),
        "-start_number".into(),
        "0".into(),
        "-i".into(),
        pattern.to_string_lossy().into_owned(),
        "-vf".into(),
        format!("tile={columns}x{rows}:padding=0:margin=0"),
        "-frames:v".into(),
        "1".into(),
        "-y".into(),
        output.to_string_lossy().into_owned(),
    ];
    run(resolve_ffmpeg_bin(), &args)?;
    Ok(())
}

/// Extract exact frames and assemble a cached filmstrip/contact sheet.
pub fn cached_filmstrip(
    src: &str,
    cache: &DerivativeCache,
    times: &[f64],
    max_width: u32,
    columns: usize,
) -> Result<FilmstripResult> {
    validate_exact_times(times)?;
    if max_width == 0 {
        return Err(crate::error::EngineError::msg(
            "maxWidth must be finite and positive",
        ));
    }
    let columns = columns.clamp(1, 16);
    let options = json!({
        "exact": true,
        "times": times,
        "maxWidth": max_width,
        "columns": columns,
    });
    let entry = cache.source_entry(Path::new(src), "filmstrip", &options)?;
    let names = frame_file_names(times.len());
    let mut required = names.iter().map(String::as_str).collect::<Vec<_>>();
    required.push("filmstrip.png");
    let cached = entry.is_complete(&required);
    if !cached {
        extract_exact_frames_to_dir(src, times, &entry.directory, max_width)?;
        let frame_paths = names
            .iter()
            .map(|name| entry.path(name).to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        create_contact_sheet(&frame_paths, &entry.path("filmstrip.png"), columns)?;
        entry.mark_complete(&required)?;
    }
    Ok(FilmstripResult {
        key: entry.key.clone(),
        path: entry.path("filmstrip.png").to_string_lossy().into_owned(),
        frame_paths: names
            .iter()
            .map(|name| entry.path(name).to_string_lossy().into_owned())
            .collect(),
        times: times.to_vec(),
        columns,
        cached,
    })
}

fn decode_waveform_buckets(
    src: &str,
    duration: f64,
    bucket_count: usize,
    start: f64,
) -> Result<Vec<WaveformBucket>> {
    let bucket_count = bucket_count.max(1);
    let expected = if duration.is_finite() && duration > 0.0 {
        (duration * f64::from(WAVEFORM_SAMPLE_RATE))
            .round()
            .max(1.0) as usize
    } else {
        0
    };
    let mut mins = vec![f64::INFINITY; bucket_count];
    let mut maxs = vec![f64::NEG_INFINITY; bucket_count];
    let mut sums = vec![0.0_f64; bucket_count];
    let mut counts = vec![0_usize; bucket_count];
    let sample_rate = WAVEFORM_SAMPLE_RATE.to_string();
    let start = start.to_string();
    let duration = duration.to_string();
    let mut child = Command::new(resolve_ffmpeg_bin())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            start.as_str(),
            "-t",
            duration.as_str(),
            "-i",
            src,
            "-vn",
            "-ac",
            "1",
            "-ar",
            sample_rate.as_str(),
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            reap_waveform_child(&mut child);
            return Err(crate::error::EngineError::msg(
                "ffmpeg waveform decoder did not expose stdout",
            ));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            reap_waveform_child(&mut child);
            return Err(crate::error::EngineError::msg(
                "ffmpeg waveform decoder did not expose stderr",
            ));
        }
    };
    let stderr_reader = drain_waveform_stderr(stderr);
    let mut buf = vec![0_u8; 64 * 1024];
    let mut leftover = Vec::new();
    let mut index = 0usize;
    let mut read_error = None;
    loop {
        let read = match stdout.read(&mut buf) {
            Ok(read) => read,
            Err(error) => {
                read_error = Some(error);
                break;
            }
        };
        if read == 0 {
            break;
        }
        leftover.extend_from_slice(&buf[..read]);
        let usable = leftover.len() / 4 * 4;
        for chunk in leftover[..usable].chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64;
            if sample.is_finite() {
                let bucket = if expected > 0 {
                    index.saturating_mul(bucket_count) / expected
                } else {
                    0
                }
                .min(bucket_count - 1);
                mins[bucket] = mins[bucket].min(sample);
                maxs[bucket] = maxs[bucket].max(sample);
                sums[bucket] += sample * sample;
                counts[bucket] += 1;
            }
            index += 1;
        }
        leftover.drain(..usable);
    }
    drop(stdout);
    if let Some(error) = read_error {
        reap_waveform_child_with_stderr(&mut child, stderr_reader);
        return Err(error.into());
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            reap_waveform_child_with_stderr(&mut child, stderr_reader);
            return Err(error.into());
        }
    };
    let stderr = stderr_reader
        .join()
        .map_err(|_| crate::error::EngineError::msg("ffmpeg stderr reader panicked"))??;
    if !status.success() {
        return Err(crate::error::EngineError::msg(waveform_decode_error(
            &stderr,
        )));
    }
    if index == 0 {
        return Err(crate::error::EngineError::msg(
            "audio stream did not produce waveform samples",
        ));
    }
    if expected == 0 {
        return Ok(waveform_buckets_from_counts(
            &mins,
            &maxs,
            &sums,
            &counts,
            bucket_count,
        ));
    }
    Ok((0..bucket_count)
        .map(|slot| {
            let count = counts[slot].max(1) as f64;
            WaveformBucket {
                min: if mins[slot].is_finite() {
                    mins[slot]
                } else {
                    0.0
                },
                max: if maxs[slot].is_finite() {
                    maxs[slot]
                } else {
                    0.0
                },
                rms: (sums[slot] / count).sqrt(),
            }
        })
        .collect())
}

const MAX_WAVEFORM_STDERR_BYTES: usize = 16 * 1024;

fn drain_waveform_stderr(stderr: ChildStderr) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || drain_waveform_stderr_tail(stderr))
}

fn drain_waveform_stderr_tail(mut stderr: impl Read) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::with_capacity(MAX_WAVEFORM_STDERR_BYTES);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        if read >= MAX_WAVEFORM_STDERR_BYTES {
            retained.clear();
            retained.extend_from_slice(&buffer[read - MAX_WAVEFORM_STDERR_BYTES..read]);
            continue;
        }
        let overflow = retained
            .len()
            .saturating_add(read)
            .saturating_sub(MAX_WAVEFORM_STDERR_BYTES);
        if overflow > 0 {
            retained.drain(..overflow);
        }
        retained.extend_from_slice(&buffer[..read]);
    }
}

fn reap_waveform_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn reap_waveform_child_with_stderr(
    child: &mut Child,
    stderr_reader: std::thread::JoinHandle<std::io::Result<Vec<u8>>>,
) {
    reap_waveform_child(child);
    let _ = stderr_reader.join();
}

fn waveform_decode_error(stderr: &[u8]) -> String {
    let diagnostics = String::from_utf8_lossy(stderr);
    let diagnostics = diagnostics.trim();
    if diagnostics.is_empty() {
        "ffmpeg failed while decoding waveform samples".to_string()
    } else {
        format!("ffmpeg failed while decoding waveform samples: {diagnostics}")
    }
}

fn waveform_buckets_from_counts(
    mins: &[f64],
    maxs: &[f64],
    sums: &[f64],
    counts: &[usize],
    bucket_count: usize,
) -> Vec<WaveformBucket> {
    (0..bucket_count)
        .map(|slot| {
            let count = counts[slot].max(1) as f64;
            WaveformBucket {
                min: if mins[slot].is_finite() {
                    mins[slot]
                } else {
                    0.0
                },
                max: if maxs[slot].is_finite() {
                    maxs[slot]
                } else {
                    0.0
                },
                rms: (sums[slot] / count).sqrt(),
            }
        })
        .collect()
}

fn create_waveform_image(
    src: &str,
    output: &Path,
    width: u32,
    height: u32,
    start: f64,
    duration: f64,
) -> Result<()> {
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        start.to_string(),
        "-t".into(),
        duration.to_string(),
        "-i".into(),
        src.into(),
        "-filter_complex".into(),
        format!("showwavespic=s={width}x{height}:colors=white"),
        "-frames:v".into(),
        "1".into(),
        "-y".into(),
        output.to_string_lossy().into_owned(),
    ];
    run(resolve_ffmpeg_bin(), &args)?;
    Ok(())
}

/// Generate a deterministic waveform envelope and optional image in the
/// project-local derivative cache.
pub fn cached_waveform(
    src: &str,
    cache: &DerivativeCache,
    bucket_count: usize,
    include_image: bool,
    image_width: u32,
    image_height: u32,
) -> Result<WaveformResult> {
    let metadata = info(src)?;
    let end = metadata
        .duration
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .unwrap_or(1.0);
    cached_waveform_range(
        src,
        cache,
        bucket_count,
        include_image,
        image_width,
        image_height,
        0.0,
        end,
    )
}

/// Generate a cached waveform for one bounded source interval.
///
/// The range is included in the cache identity and passed to both the audio
/// decoder and optional waveform image, so a targeted observation never
/// accidentally reuses a full-source derivative.
pub fn cached_waveform_range(
    src: &str,
    cache: &DerivativeCache,
    bucket_count: usize,
    include_image: bool,
    image_width: u32,
    image_height: u32,
    start: f64,
    end: f64,
) -> Result<WaveformResult> {
    let bucket_count = bucket_count.clamp(1, MAX_WAVEFORM_BUCKETS);
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err(crate::error::EngineError::msg(
            "waveform range must be finite with 0 <= start < end",
        ));
    }
    if image_width == 0 || image_height == 0 {
        return Err(crate::error::EngineError::msg(
            "waveform image dimensions must be finite and positive",
        ));
    }
    let options = json!({
        "bucketCount": bucket_count,
        "includeImage": include_image,
        "imageWidth": image_width,
        "imageHeight": image_height,
        "sampleRate": WAVEFORM_SAMPLE_RATE,
        "start": start,
        "end": end,
    });
    let entry = cache.source_entry(Path::new(src), "waveform", &options)?;
    let mut required = vec!["waveform.json"];
    if include_image {
        required.push("waveform.png");
    }
    let cached = entry.is_complete(&required);
    let payload = if cached {
        entry.read_json::<WaveformPayload>("waveform.json")?
    } else {
        let meta = info(src)?;
        if meta.has_audio == Some(false) {
            return Err(crate::error::EngineError::msg(
                "media does not contain an audio stream",
            ));
        }
        let source_duration = meta
            .duration
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .unwrap_or(0.0);
        if source_duration > 0.0 && end > source_duration + 1e-6 {
            return Err(crate::error::EngineError::msg(
                "waveform range exceeds source duration",
            ));
        }
        let duration = end - start;
        let buckets = decode_waveform_buckets(src, duration, bucket_count, start)?;
        let duration = if duration > 0.0 {
            duration
        } else {
            buckets.len() as f64 / WAVEFORM_SAMPLE_RATE as f64
        };
        let image_path = if include_image {
            let path = entry.path("waveform.png");
            create_waveform_image(src, &path, image_width, image_height, start, duration)?;
            Some(path.to_string_lossy().into_owned())
        } else {
            None
        };
        let payload = WaveformPayload {
            start,
            end: start + duration,
            duration,
            sample_rate: WAVEFORM_SAMPLE_RATE,
            buckets,
            image_path,
        };
        entry.write_json("waveform.json", &payload)?;
        entry.mark_complete(&required)?;
        payload
    };
    Ok(WaveformResult {
        key: entry.key,
        start: payload.start,
        end: payload.end,
        duration: payload.duration,
        sample_rate: payload.sample_rate,
        buckets: payload.buckets,
        image_path: payload.image_path,
        cached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn pairs_silence_start_end() {
        let stderr = [
            "[silencedetect @ 0x0] silence_start: 1.2",
            "[silencedetect @ 0x0] silence_end: 2.5 | silence_duration: 1.3",
            "[silencedetect @ 0x0] silence_start: 4",
            "[silencedetect @ 0x0] silence_end: 4.75 | silence_duration: 0.75",
        ]
        .join("\n");
        assert_eq!(
            parse_silencedetect(&stderr),
            vec![
                SilenceRange {
                    start: 1.2,
                    end: Some(2.5)
                },
                SilenceRange {
                    start: 4.0,
                    end: Some(4.75)
                },
            ]
        );
    }

    #[test]
    fn trailing_silence_without_end() {
        let stderr =
            "[silencedetect] silence_start: 3.1\n[silencedetect] silence_end: 3.9\n[silencedetect] silence_start: 8";
        assert_eq!(
            parse_silencedetect(stderr),
            vec![
                SilenceRange {
                    start: 3.1,
                    end: Some(3.9)
                },
                SilenceRange {
                    start: 8.0,
                    end: None
                },
            ]
        );
    }

    #[test]
    fn scene_times_pair_into_ranges() {
        let stderr = [
            "frame:0    pts:0      pts_time:0.5",
            "lavfi.scene_score=0.721300",
            "frame:1    pts:48     pts_time:2.0",
            "lavfi.scene_score=0.512000",
        ]
        .join("\n");
        assert_eq!(
            parse_scene_times(&stderr),
            vec![
                SceneRange {
                    start: 0.5,
                    end: Some(2.0)
                },
                SceneRange {
                    start: 2.0,
                    end: None
                },
            ]
        );
    }

    #[test]
    fn ranged_waveform_rejects_reversed_ranges_before_media_io() {
        let temp = tempfile::tempdir().expect("cache");
        let cache = DerivativeCache::new(temp.path());
        let error = cached_waveform_range("/missing.wav", &cache, 8, false, 800, 120, 4.0, 4.0)
            .expect_err("equal bounds");
        assert!(error.to_string().contains("0 <= start < end"));
    }

    #[test]
    fn waveform_stderr_tail_is_bounded_and_keeps_latest_diagnostics() {
        let mut input = vec![b'x'; MAX_WAVEFORM_STDERR_BYTES * 4];
        input.extend_from_slice(b"final ffmpeg error");
        let retained = drain_waveform_stderr_tail(Cursor::new(input)).expect("read stderr");

        assert!(retained.len() <= MAX_WAVEFORM_STDERR_BYTES);
        assert!(String::from_utf8_lossy(&retained).ends_with("final ffmpeg error"));
    }

    #[cfg(unix)]
    #[test]
    fn waveform_stderr_is_drained_while_stdout_is_read() {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "head -c 131072 /dev/zero >&2; printf '\\000\\000\\000\\000'; exit 1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn test child");
        let stderr = child.stderr.take().expect("stderr pipe");
        let stderr_reader = drain_waveform_stderr(stderr);
        let mut stdout = child.stdout.take().expect("stdout pipe");
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).expect("read stdout");
        let status = child.wait().expect("wait test child");
        let stderr = stderr_reader
            .join()
            .expect("stderr reader thread")
            .expect("read stderr");

        assert!(!status.success());
        assert_eq!(output.len(), 4);
        assert_eq!(stderr.len(), MAX_WAVEFORM_STDERR_BYTES);
    }
}
