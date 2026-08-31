//! Probe parsers and ffmpeg/ffprobe wrappers.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::SystemTime;

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
            for slot in index..end {
                let time = times[slot];
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
