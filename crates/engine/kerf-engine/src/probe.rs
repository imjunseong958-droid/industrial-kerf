//! Probe parsers and ffmpeg/ffprobe wrappers.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, Stdio};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::SystemTime;

use crate::derivatives::DerivativeCache;
use crate::error::{EngineError, Result};
use crate::ffmpeg::{
    resolve_ffmpeg_bin, resolve_ffprobe_bin, run, run_allow_failure, video_decode_prefix,
};
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
    /// Whether the picture's pixel format carries transparency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<bool>,
    /// Bits per picture component.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits: Option<u32>,
    /// How its code values become light, as a color space's id, when it is not BT.709.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
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
/// Exact frames and contact sheets are JPEG: a fraction of the size of PNG
/// for footage, which keeps several of them within what a model accepts.
pub const FRAME_MEDIA_TYPE: &str = "image/jpeg";
/// FFmpeg's JPEG quality scale runs from 2 (best) to 31.
const FRAME_QUALITY: &str = "3";
pub const MAX_WAVEFORM_BUCKETS: usize = 4096;
const WAVEFORM_SAMPLE_RATE: u32 = 8_000;

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    bits_per_raw_sample: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
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

/// The color space a file's tags say it is in, when that is not BT.709. Log footage rarely tags
/// itself, so that is for a person or an agent to write in the document.
fn input_color(transfer: Option<&str>, primaries: Option<&str>) -> Option<kerf_model::ColorSpace> {
    use kerf_model::ColorSpace;
    match (transfer, primaries) {
        (Some("smpte2084"), _) => Some(ColorSpace::Rec2100Pq),
        (Some("arib-std-b67"), _) => Some(ColorSpace::Rec2100Hlg),
        (_, Some("bt2020")) => Some(ColorSpace::Rec2020),
        (_, Some("smpte432")) => Some(ColorSpace::P3D65),
        _ => None,
    }
}

impl ProbeInfo {
    /// What a document holds about a picture of this size with these properties.
    pub fn video_info(&self, width: u32, height: u32) -> kerf_model::VideoInfo {
        kerf_model::VideoInfo {
            width,
            height,
            alpha: self.alpha == Some(true),
            bits: self
                .bits
                .map(|bits| match bits {
                    0..=8 => 8,
                    9 | 10 => 10,
                    11 | 12 => 12,
                    _ => 16,
                })
                .unwrap_or(8),
            color: self
                .color
                .as_deref()
                .and_then(kerf_model::ColorSpace::from_id)
                .unwrap_or_default(),
        }
    }
}

/// Whether an FFmpeg pixel format carries transparency: `yuva420p`,
/// `rgba`, `bgra`, `argb`, `gbrap`, `ya8`, and their deeper variants.
fn has_alpha(pix_fmt: &str) -> bool {
    ["yuva", "gbrap", "ya", "rgba", "bgra", "argb", "abgr"]
        .iter()
        .any(|prefix| pix_fmt.starts_with(prefix))
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
        result.alpha = video.pix_fmt.as_deref().map(has_alpha);
        result.bits = video
            .bits_per_raw_sample
            .as_deref()
            .and_then(|bits| bits.parse().ok())
            .filter(|bits| *bits > 0)
            .or_else(|| video.pix_fmt.as_deref().and_then(pix_fmt_bit_depth));
        result.color = input_color(
            video.color_transfer.as_deref(),
            video.color_primaries.as_deref(),
        )
        .map(|space| space.id().to_owned());
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

/// Facts about a media file a bin or an inspector shows: its codecs, its
/// picture's pixel format, bit depth and colour tags, and the camera, lens,
/// reel and timecode tags the file carries. Each field is present only when
/// the file says it: an untagged colour space is absent, not guessed.
///
/// These are facts about the file, not the edit, so they belong in a cache
/// beside the project, never in the document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// The picture's codec, or the sound's for a file with no picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// The codec profile, such as `HQ` for ProRes 422 HQ or `High` for H.264.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pix_fmt: Option<String>,
    /// Bits per picture component: the stream's `bits_per_raw_sample`, or
    /// what its pixel format holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u32>,
    /// The YUV matrix, such as `bt709`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    /// The transfer characteristic, such as `bt709` or `arib-std-b67`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primaries: Option<String>,
    /// `tv` (limited) or `pc` (full).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_range: Option<String>,
    /// The camera's make and model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reel: Option<String>,
    /// The start timecode, such as `01:00:00:00`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    /// The sound's sample format, such as `s24le` or `fltp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_sample_fmt: Option<String>,
    /// Bits per sound sample, when the file states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_bit_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_sample_rate: Option<u32>,
}

/// Tags that name the camera's make, then its model, most specific first.
const MAKE_TAGS: [&str; 3] = [
    "com.apple.quicktime.make",
    "com.android.manufacturer",
    "make",
];
const MODEL_TAGS: [&str; 3] = ["com.apple.quicktime.model", "com.android.model", "model"];
const LENS_TAGS: [&str; 4] = [
    "com.apple.quicktime.camera.lens_model",
    "com.apple.quicktime.lens_model",
    "lens_model",
    "lens",
];
const REEL_TAGS: [&str; 4] = ["reel_name", "com.apple.proapps.reel", "reel", "tape_name"];

/// A tag's value, looked up without regard to case, when it is not blank.
fn tag(tags: &serde_json::Map<String, serde_json::Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        tags.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| match value {
                serde_json::Value::String(text) => Some(text.trim().to_owned()),
                serde_json::Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
    })
}

/// A colour tag FFmpeg reports, or nothing when it says the file has none.
fn colour_tag(value: Option<&serde_json::Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty() && !matches!(text, "unknown" | "unspecified" | "reserved"))
        .then(|| text.to_owned())
}

/// A count FFmpeg gives as a number or as a decimal string; 0 is "unknown".
fn count(value: Option<&serde_json::Value>) -> Option<u32> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| *number > 0)
}

/// Bits per component a pixel format holds: `yuv422p10le` 10, `p010le`
/// 10, `nv12` 8, `rgb48le` 16. Nothing for a format it does not know.
pub fn pix_fmt_bit_depth(pix_fmt: &str) -> Option<u32> {
    let name = pix_fmt
        .strip_suffix("le")
        .or_else(|| pix_fmt.strip_suffix("be"))
        .unwrap_or(pix_fmt);
    let digits = |text: &str| -> Option<u32> { text.parse().ok() };
    // Semi-planar 4:2:0, 4:2:2 and 4:4:4: p010, p210, p416, and y210.
    if let Some(rest) = name
        .strip_prefix('p')
        .or_else(|| name.strip_prefix('y'))
        .filter(|rest| rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit()))
    {
        return digits(&rest[1..]);
    }
    if let Some(index) = name.rfind('p') {
        let tail = &name[index + 1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            return digits(tail);
        }
    }
    if let Some(tail) = name
        .strip_prefix("gray")
        .or_else(|| name.strip_prefix("ya"))
    {
        return if tail.is_empty() {
            Some(8)
        } else {
            digits(tail)
        };
    }
    match name {
        "rgb48" | "bgr48" | "rgba64" | "bgra64" | "ayuv64" => Some(16),
        "x2rgb10" | "x2bgr10" | "v30x" | "xv30" => Some(10),
        "xv36" => Some(12),
        _ if name.starts_with("yuv")
            || name.starts_with("yuvj")
            || name.starts_with("nv")
            || name.starts_with("gbrp")
            || matches!(
                name,
                "rgb24"
                    | "bgr24"
                    | "rgba"
                    | "bgra"
                    | "argb"
                    | "abgr"
                    | "rgb0"
                    | "bgr0"
                    | "0rgb"
                    | "0bgr"
                    | "uyvy422"
                    | "yuyv422"
                    | "yvyu422"
                    | "vuya"
                    | "vuyx"
            ) =>
        {
            Some(8)
        }
        _ => None,
    }
}

/// [`MediaInfo`] from `ffprobe -print_format json -show_format
/// -show_streams` output.
pub fn parse_media_info_json(stdout: &str) -> Result<MediaInfo> {
    let parsed: serde_json::Value = serde_json::from_str(stdout)?;
    let empty = serde_json::Map::new();
    let streams = parsed
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let of_type = |kind: &str| {
        streams.iter().find(|stream| {
            stream.get("codec_type").and_then(serde_json::Value::as_str) == Some(kind)
                // Cover art is a picture stream too; it is not the media's picture.
                && stream
                    .get("disposition")
                    .and_then(|disposition| disposition.get("attached_pic"))
                    .and_then(serde_json::Value::as_u64)
                    != Some(1)
        })
    };
    let tags_of = |value: Option<&serde_json::Value>| {
        value
            .and_then(|value| value.get("tags"))
            .and_then(serde_json::Value::as_object)
            .unwrap_or(&empty)
            .clone()
    };
    let text = |value: Option<&serde_json::Value>, key: &str| {
        value
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty() && *text != "unknown")
            .map(str::to_owned)
    };
    let video = of_type("video");
    let audio = of_type("audio");
    let format_tags = tags_of(parsed.get("format"));
    // Camera and reel tags live on the container, on the picture, or on a
    // timecode track, depending on who wrote the file.
    let mut every = vec![format_tags, tags_of(video)];
    every.extend(streams.iter().map(|stream| tags_of(Some(stream))));
    let any = |names: &[&str]| every.iter().find_map(|tags| tag(tags, names));
    let make = any(&MAKE_TAGS);
    let model = any(&MODEL_TAGS);
    let camera = match (make, model) {
        // "Apple" + "iPhone 16 Pro", but "Canon" + "Canon EOS R5" once.
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(model)
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (make, model) => model.or(make),
    };
    let pix_fmt = text(video, "pix_fmt");
    let bit_depth = count(video.and_then(|video| video.get("bits_per_raw_sample")))
        .or_else(|| pix_fmt.as_deref().and_then(pix_fmt_bit_depth));
    Ok(MediaInfo {
        codec: text(video, "codec_name").or_else(|| text(audio, "codec_name")),
        profile: text(video, "profile").or_else(|| text(audio, "profile")),
        pix_fmt,
        bit_depth,
        color_space: colour_tag(video.and_then(|video| video.get("color_space"))),
        transfer: colour_tag(video.and_then(|video| video.get("color_transfer"))),
        primaries: colour_tag(video.and_then(|video| video.get("color_primaries"))),
        color_range: colour_tag(video.and_then(|video| video.get("color_range"))),
        camera,
        lens: any(&LENS_TAGS),
        reel: any(&REEL_TAGS),
        timecode: any(&["timecode"]),
        audio_codec: text(audio, "codec_name"),
        audio_sample_fmt: text(audio, "sample_fmt"),
        audio_bit_depth: count(audio.and_then(|audio| audio.get("bits_per_raw_sample")))
            .or_else(|| count(audio.and_then(|audio| audio.get("bits_per_sample")))),
        audio_channels: count(audio.and_then(|audio| audio.get("channels"))),
        audio_sample_rate: count(audio.and_then(|audio| audio.get("sample_rate"))),
    })
}

struct MediaInfoCacheEntry {
    len: u64,
    modified: Option<SystemTime>,
    info: MediaInfo,
}

fn media_info_cache() -> &'static Mutex<HashMap<String, MediaInfoCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, MediaInfoCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The [`MediaInfo`] of `src`, probed once per version of the file.
pub fn media_info(src: &str) -> Result<MediaInfo> {
    let identity = probe_identity(src);
    if let Some((len, modified)) = identity {
        let cache = media_info_cache()
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
    let info = parse_media_info_json(&output.stdout)?;
    if let Some((len, modified)) = identity.or_else(|| probe_identity(src)) {
        media_info_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                src.to_string(),
                MediaInfoCacheEntry {
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
        return Err(EngineError::msg(format!(
            "extractFrames maxWidth must be finite and positive, got {max_width}"
        )));
    }
    std::fs::create_dir_all(out_dir)?;
    let scale = format!("scale=w='min(iw,{max_width})':h=-2");
    let ffmpeg = resolve_ffmpeg_bin();
    extract_in_parallel(times.len(), |slot| {
        let out_path = Path::new(out_dir).join(format!("frame_{slot}.png"));
        extract_one_frame(
            ffmpeg,
            src,
            times[slot],
            &scale,
            out_path.to_string_lossy().into_owned(),
        )
    })
}

/// Run `extract(slot)` for every slot in `0..count`, at most
/// `extract_job_count()` at a time, and return the results in slot order.
fn extract_in_parallel<T, F>(count: usize, extract: F) -> Result<Vec<T>>
where
    T: Send,
    F: Fn(usize) -> Result<T> + Sync,
{
    let jobs = extract_job_count().max(1);
    let mut results = Vec::with_capacity(count);
    let extract = &extract;
    for start in (0..count).step_by(jobs) {
        std::thread::scope(|scope| {
            let handles = (start..count.min(start + jobs))
                .map(|slot| scope.spawn(move || extract(slot)))
                .collect::<Vec<_>>();
            for handle in handles {
                let result = handle
                    .join()
                    .map_err(|_| EngineError::msg("frame extraction thread panicked"))?;
                results.push(result?);
            }
            Ok::<(), EngineError>(())
        })?;
    }
    Ok(results)
}

fn validate_frame_request(times: &[f64], max_width: u32) -> Result<()> {
    if max_width == 0 {
        return Err(EngineError::msg("maxWidth must be finite and positive"));
    }
    if times.is_empty() {
        return Err(EngineError::msg("at least one frame timestamp is required"));
    }
    if times.len() > MAX_FRAME_TIMES {
        return Err(EngineError::msg(format!(
            "frame timestamp count exceeds {MAX_FRAME_TIMES}"
        )));
    }
    if times.iter().any(|time| !time.is_finite() || *time < 0.0) {
        return Err(EngineError::msg(
            "frame timestamps must be finite and nonnegative",
        ));
    }
    Ok(())
}

/// The name of the `index`th exact frame in a derivative directory.
pub fn frame_file_name(index: usize) -> String {
    format!("frame-{index:03}.jpg")
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
        "-q:v".into(),
        FRAME_QUALITY.into(),
        "-y".into(),
        output.clone(),
    ]);
    run(ffmpeg, &args)?;
    Ok(FrameGrab {
        time,
        path: output,
        media_type: FRAME_MEDIA_TYPE.into(),
    })
}

/// Extract exact timestamp frames into a caller-selected derivative directory.
pub fn extract_exact_frames(
    src: &str,
    times: &[f64],
    out_dir: &Path,
    max_width: u32,
) -> Result<Vec<FrameGrab>> {
    validate_frame_request(times, max_width)?;
    fs::create_dir_all(out_dir)?;
    let scale = format!("scale=w='min(iw,{max_width})':h=-2");
    let ffmpeg = resolve_ffmpeg_bin();
    let names = frame_file_names(times.len());
    extract_in_parallel(times.len(), |slot| {
        let output = out_dir.join(&names[slot]).to_string_lossy().into_owned();
        extract_one_exact_frame(ffmpeg, src, times[slot], &scale, output)
    })
}

/// Extract frames at exact requested timestamps and cache the result below the
/// project's `.kerf/cache/derived` directory.
pub fn cached_exact_frames(
    src: &str,
    cache: &DerivativeCache,
    times: &[f64],
    max_width: u32,
) -> Result<FrameGrabSet> {
    validate_frame_request(times, max_width)?;
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
        extract_exact_frames(src, times, &entry.directory, max_width)?;
        entry.mark_complete(&required)?;
    }
    let frames = times
        .iter()
        .zip(names)
        .map(|(time, name)| FrameGrab {
            time: *time,
            path: entry.path(&name).to_string_lossy().into_owned(),
            media_type: FRAME_MEDIA_TYPE.into(),
        })
        .collect();
    Ok(FrameGrabSet {
        key: entry.key,
        frames,
        cached,
    })
}

/// Build a JPEG contact sheet from exact frames named by `frame_file_name`.
pub fn create_contact_sheet(frame_paths: &[String], output: &Path, columns: usize) -> Result<()> {
    create_labelled_contact_sheet(frame_paths, output, columns, &[], Path::new(""))
}

/// Whether this ffmpeg has `drawtext`, which wants a build with FreeType.
fn draws_text() -> bool {
    static DRAWS: OnceLock<bool> = OnceLock::new();
    *DRAWS.get_or_init(|| {
        run_allow_failure(
            resolve_ffmpeg_bin(),
            &["-hide_banner".into(), "-filters".into()],
        )
        .is_ok_and(|output| output.stdout.contains(" drawtext "))
    })
}

/// Text as a quoted option of an ffmpeg filter takes it.
fn filter_text(text: &str) -> String {
    text.replace('\\', "/")
        .replace('\'', "\u{2019}")
        .replace(':', "\\:")
        .replace('%', "\\%")
}

/// [`create_contact_sheet`] with a label in the corner of each frame, one for each in
/// order: its time, so that whoever reads the sheet sees which moment a tile is without
/// counting tiles. Drawn in the font file `font`; left off where this ffmpeg has no
/// `drawtext`, or the labels are not one a frame.
pub fn create_labelled_contact_sheet(
    frame_paths: &[String],
    output: &Path,
    columns: usize,
    labels: &[String],
    font: &Path,
) -> Result<()> {
    if frame_paths.is_empty() {
        return Err(EngineError::msg(
            "contact sheet requires at least one frame",
        ));
    }
    if !(1..=16).contains(&columns) {
        return Err(EngineError::msg(
            "contact sheet columns must be between 1 and 16",
        ));
    }
    let first = Path::new(&frame_paths[0]);
    let parent = first
        .parent()
        .ok_or_else(|| EngineError::msg("frame path has no parent directory"))?;
    if frame_paths
        .iter()
        .any(|path| Path::new(path).parent() != Some(parent))
    {
        return Err(EngineError::msg(
            "contact sheet frames must share a directory",
        ));
    }
    let rows = frame_paths.len().div_ceil(columns);
    let pattern = parent.join("frame-%03d.jpg");
    // Each label on its own frame, before they are laid out: white on a dark box, an
    // eleventh of the frame high.
    let mut labelled = String::new();
    if labels.len() == frame_paths.len() && font.is_file() && draws_text() {
        let font = filter_text(&font.to_string_lossy());
        for (index, label) in labels.iter().enumerate() {
            labelled.push_str(&format!(
                "drawtext=fontfile='{font}':text='{}':enable='eq(n\\,{index})':x=w/40:y=w/40:\
                 fontsize=h/11:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=6,",
                filter_text(label)
            ));
        }
    }
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
        format!("{labelled}tile={columns}x{rows}:padding=0:margin=0"),
        "-frames:v".into(),
        "1".into(),
        "-q:v".into(),
        FRAME_QUALITY.into(),
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
    validate_frame_request(times, max_width)?;
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
    required.push("filmstrip.jpg");
    let cached = entry.is_complete(&required);
    if !cached {
        extract_exact_frames(src, times, &entry.directory, max_width)?;
        let frame_paths = names
            .iter()
            .map(|name| entry.path(name).to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        create_contact_sheet(&frame_paths, &entry.path("filmstrip.jpg"), columns)?;
        entry.mark_complete(&required)?;
    }
    Ok(FilmstripResult {
        key: entry.key.clone(),
        path: entry.path("filmstrip.jpg").to_string_lossy().into_owned(),
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
    let mut child = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin())
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
            return Err(EngineError::msg(
                "ffmpeg waveform decoder did not expose stdout",
            ));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            reap_waveform_child(&mut child);
            return Err(EngineError::msg(
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
        for chunk in leftover[..usable].as_chunks::<4>().0 {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64;
            if sample.is_finite() {
                let bucket = index
                    .saturating_mul(bucket_count)
                    .checked_div(expected)
                    .unwrap_or(0)
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
        .map_err(|_| EngineError::msg("ffmpeg stderr reader panicked"))??;
    if !status.success() {
        return Err(EngineError::msg(waveform_decode_error(&stderr)));
    }
    if index == 0 {
        return Err(EngineError::msg(
            "audio stream did not produce waveform samples",
        ));
    }
    Ok(waveform_buckets_from_counts(&mins, &maxs, &sums, &counts))
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
) -> Vec<WaveformBucket> {
    let finite_or_zero = |value: f64| if value.is_finite() { value } else { 0.0 };
    (0..counts.len())
        .map(|slot| WaveformBucket {
            min: finite_or_zero(mins[slot]),
            max: finite_or_zero(maxs[slot]),
            rms: (sums[slot] / counts[slot].max(1) as f64).sqrt(),
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
#[allow(clippy::too_many_arguments)]
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
        return Err(EngineError::msg(
            "waveform range must be finite with 0 <= start < end",
        ));
    }
    if image_width == 0 || image_height == 0 {
        return Err(EngineError::msg(
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
            return Err(EngineError::msg("media does not contain an audio stream"));
        }
        let source_duration = meta
            .duration
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .unwrap_or(0.0);
        if source_duration > 0.0 && end > source_duration + 1e-6 {
            return Err(EngineError::msg("waveform range exceeds source duration"));
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
    fn transparency_comes_from_the_pixel_format() {
        for format in [
            "yuva420p",
            "rgba",
            "bgra",
            "argb",
            "gbrap10le",
            "ya8",
            "rgba64le",
        ] {
            assert!(has_alpha(format), "{format}");
        }
        for format in ["yuv420p", "yuvj422p", "rgb24", "gbrp", "nv12", "gray"] {
            assert!(!has_alpha(format), "{format}");
        }
    }

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
        let mut child = std::process::Command::new("sh")
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

    #[test]
    fn bit_depth_comes_from_the_pixel_format() {
        for (format, bits) in [
            ("yuv420p", Some(8)),
            ("yuvj422p", Some(8)),
            ("nv12", Some(8)),
            ("uyvy422", Some(8)),
            ("rgba", Some(8)),
            ("gbrp", Some(8)),
            ("yuv422p10le", Some(10)),
            ("yuva444p12be", Some(12)),
            ("gbrp16le", Some(16)),
            ("p010le", Some(10)),
            ("p210le", Some(10)),
            ("p416le", Some(16)),
            ("y210le", Some(10)),
            ("gray", Some(8)),
            ("gray10le", Some(10)),
            ("rgb48le", Some(16)),
            ("x2rgb10le", Some(10)),
            ("videotoolbox_vld", None),
        ] {
            assert_eq!(pix_fmt_bit_depth(format), bits, "{format}");
        }
    }

    #[test]
    fn media_info_reads_codecs_colour_and_camera_tags() {
        let probe = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "hevc", "profile": "Main 10",
                 "pix_fmt": "yuv420p10le", "color_space": "bt2020nc",
                 "color_transfer": "arib-std-b67", "color_primaries": "bt2020",
                 "color_range": "tv", "tags": {"handler_name": "Core Media Video"}},
                {"codec_type": "audio", "codec_name": "aac", "sample_fmt": "fltp",
                 "sample_rate": "48000", "channels": 2, "bits_per_sample": 0},
                {"codec_type": "data", "codec_tag_string": "tmcd",
                 "tags": {"timecode": "01:00:00:00", "reel_name": "A001"}}
            ],
            "format": {"tags": {
                "com.apple.quicktime.make": "Apple",
                "com.apple.quicktime.model": "iPhone 16 Pro",
                "com.apple.quicktime.camera.lens_model": "iPhone 16 Pro back camera 6.765mm f/1.78"
            }}
        }"#;
        let info = parse_media_info_json(probe).expect("parses");
        assert_eq!(info.codec.as_deref(), Some("hevc"));
        assert_eq!(info.profile.as_deref(), Some("Main 10"));
        assert_eq!(info.pix_fmt.as_deref(), Some("yuv420p10le"));
        assert_eq!(info.bit_depth, Some(10));
        assert_eq!(info.color_space.as_deref(), Some("bt2020nc"));
        assert_eq!(info.transfer.as_deref(), Some("arib-std-b67"));
        assert_eq!(info.primaries.as_deref(), Some("bt2020"));
        assert_eq!(info.color_range.as_deref(), Some("tv"));
        assert_eq!(info.camera.as_deref(), Some("Apple iPhone 16 Pro"));
        assert_eq!(
            info.lens.as_deref(),
            Some("iPhone 16 Pro back camera 6.765mm f/1.78")
        );
        assert_eq!(info.reel.as_deref(), Some("A001"));
        assert_eq!(info.timecode.as_deref(), Some("01:00:00:00"));
        assert_eq!(info.audio_codec.as_deref(), Some("aac"));
        assert_eq!(info.audio_channels, Some(2));
        assert_eq!(info.audio_sample_rate, Some(48_000));
        assert_eq!(info.audio_bit_depth, None, "0 bits means unstated");
        let json = serde_json::to_value(&info).expect("serializes");
        assert_eq!(json["pixFmt"], "yuv420p10le");
        assert_eq!(json["bitDepth"], 10);
        assert_eq!(json["colorSpace"], "bt2020nc");
    }

    #[test]
    fn media_info_leaves_out_what_the_file_does_not_say() {
        let probe = r#"{
            "streams": [
                {"codec_type": "video", "codec_name": "mjpeg", "pix_fmt": "yuvj420p",
                 "disposition": {"attached_pic": 1}},
                {"codec_type": "audio", "codec_name": "pcm_s24le", "sample_fmt": "s32",
                 "bits_per_raw_sample": "24", "sample_rate": "48000", "channels": 2}
            ],
            "format": {"tags": {"make": "Canon", "model": "Canon EOS R5"}}
        }"#;
        let info = parse_media_info_json(probe).expect("parses");
        // Cover art is not a picture; the sound's codec stands in.
        assert_eq!(info.codec.as_deref(), Some("pcm_s24le"));
        assert_eq!(info.pix_fmt, None);
        assert_eq!(info.bit_depth, None);
        assert_eq!(info.color_space, None);
        assert_eq!(info.audio_bit_depth, Some(24));
        assert_eq!(info.camera.as_deref(), Some("Canon EOS R5"));
        assert_eq!(info.lens, None);
        let json = serde_json::to_value(&info).expect("serializes");
        assert!(json.get("colorSpace").is_none() && json.get("lens").is_none());

        let untagged = r#"{"streams": [{"codec_type": "video", "codec_name": "h264",
            "pix_fmt": "yuv420p", "bits_per_raw_sample": "8", "color_space": "unknown",
            "color_primaries": "unknown"}]}"#;
        let info = parse_media_info_json(untagged).expect("parses");
        assert_eq!(info.bit_depth, Some(8));
        assert_eq!((info.color_space, info.primaries), (None, None));
    }
}

#[cfg(test)]
mod input_color_tests {
    use super::*;
    use kerf_model::ColorSpace;

    fn probe(stream: &str) -> ProbeInfo {
        parse_ffprobe_json(&format!(
            r#"{{"streams":[{{"codec_type":"video","width":64,"height":36,{stream}}}]}}"#
        ))
        .expect("parses")
    }

    #[test]
    fn hdr_and_wide_gamut_are_found_from_their_tags() {
        for (stream, space, bits) in [
            (
                r#""pix_fmt":"yuv420p10le","color_transfer":"smpte2084","color_primaries":"bt2020""#,
                ColorSpace::Rec2100Pq,
                10,
            ),
            (
                r#""pix_fmt":"yuv420p10le","color_transfer":"arib-std-b67","color_primaries":"bt2020""#,
                ColorSpace::Rec2100Hlg,
                10,
            ),
            (
                r#""pix_fmt":"yuv422p10le","color_primaries":"bt2020""#,
                ColorSpace::Rec2020,
                10,
            ),
            (
                r#""pix_fmt":"yuv420p","color_primaries":"smpte432""#,
                ColorSpace::P3D65,
                8,
            ),
            (r#""pix_fmt":"yuv420p12le""#, ColorSpace::Rec709, 12),
            (
                r#""pix_fmt":"yuv420p","color_transfer":"bt709","color_primaries":"bt709""#,
                ColorSpace::Rec709,
                8,
            ),
        ] {
            let info = probe(stream).video_info(64, 36);
            assert_eq!((info.color, info.bits), (space, bits), "{stream}");
        }
    }

    #[test]
    fn the_raw_sample_size_is_taken_over_the_pixel_format() {
        let info =
            probe(r#""pix_fmt":"yuv420p16le","bits_per_raw_sample":"10""#).video_info(64, 36);
        assert_eq!(info.bits, 10);
    }
}
