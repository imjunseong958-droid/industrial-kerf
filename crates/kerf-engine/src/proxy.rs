//! Proxy cache keys and ffmpeg argument construction.

use sha2::{Digest, Sha256};
use std::path::Path;

use serde_json::{Map, Number, Value};

use crate::encoder::{encode_output_args, SelectEncoderOptions, VideoCodec};
use crate::error::{EngineError, Result};
use crate::render::{
    detect_and_select_encoder, run_ffmpeg_with_progress, stderr_progress_callback,
};
use crate::resources::{budget, should_try_hardware_encode};
use crate::sidecar::CancellationToken;

#[derive(Debug, Clone, Default)]
pub struct ProxySourceStat {
    pub size: u64,
    pub mtime_ms: f64,
    pub ctime_ms: Option<f64>,
    pub ino: Option<u64>,
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ProxySettings {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub max_width: Option<f64>,
    pub max_height: Option<f64>,
    pub fps: Option<f64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub crf: Option<f64>,
    pub preset: Option<String>,
    pub format: Option<String>,
    pub extension: Option<String>,
    pub pixel_format: Option<String>,
    pub duration: Option<f64>,
    pub extra: Map<String, Value>,
}

fn even_dimension(value: f64, name: &str) -> Result<i64> {
    if !value.is_finite() || value <= 0.0 {
        return Err(EngineError::msg(format!(
            "{name} must be a finite positive number"
        )));
    }
    let floored = value.floor().max(2.0) as i64;
    Ok(if floored % 2 == 0 {
        floored
    } else {
        floored - 1
    })
}

fn extension_of(settings: &ProxySettings) -> String {
    let requested = settings
        .extension
        .as_deref()
        .or(settings.format.as_deref())
        .unwrap_or("mp4");
    let normalized: String = requested
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if normalized.is_empty() {
        "mp4".into()
    } else {
        normalized
    }
}

/// JS-like canonical JSON for cache keys: sorted object keys, no whitespace.
pub fn canonical(value: &Value) -> Result<String> {
    canonical_inner(value, &mut Vec::new())
}

fn canonical_inner(value: &Value, stack: &mut Vec<*const Value>) -> Result<String> {
    match value {
        Value::Null => Ok("null".into()),
        Value::Bool(true) => Ok("true".into()),
        Value::Bool(false) => Ok("false".into()),
        Value::Number(number) => {
            if let Some(f) = number.as_f64() {
                if f.is_nan() {
                    return Ok("\"[NaN]\"".into());
                }
                if f.is_infinite() {
                    return Ok(if f.is_sign_positive() {
                        "\"[Infinity]\"".into()
                    } else {
                        "\"[-Infinity]\"".into()
                    });
                }
            }
            Ok(number.to_string())
        }
        Value::String(text) => Ok(serde_json::to_string(text)?),
        Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                parts.push(canonical_inner(item, stack)?);
            }
            Ok(format!("[{}]", parts.join(",")))
        }
        Value::Object(map) => {
            let ptr = value as *const Value;
            if stack.contains(&ptr) {
                return Err(EngineError::msg(
                    "Proxy settings must not contain circular references",
                ));
            }
            stack.push(ptr);
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut parts = Vec::with_capacity(keys.len());
            for key in keys {
                let encoded_key = serde_json::to_string(key)?;
                let encoded_value = canonical_inner(&map[key], stack)?;
                parts.push(format!("{encoded_key}:{encoded_value}"));
            }
            stack.pop();
            Ok(format!("{{{}}}", parts.join(",")))
        }
    }
}

pub fn proxy_settings_from_value(value: &Value) -> ProxySettings {
    let object = value.as_object();
    let number = |key: &str, camel: &str| {
        object.and_then(|map| {
            map.get(key)
                .or_else(|| map.get(camel))
                .and_then(Value::as_f64)
        })
    };
    let text = |key: &str, camel: &str| {
        object.and_then(|map| {
            map.get(key)
                .or_else(|| map.get(camel))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    };
    ProxySettings {
        width: number("width", "width"),
        height: number("height", "height"),
        max_width: number("max_width", "maxWidth"),
        max_height: number("max_height", "maxHeight"),
        fps: number("fps", "fps"),
        video_codec: text("video_codec", "videoCodec"),
        audio_codec: text("audio_codec", "audioCodec"),
        crf: number("crf", "crf"),
        preset: text("preset", "preset"),
        format: text("format", "format"),
        extension: text("extension", "extension"),
        pixel_format: text("pixel_format", "pixelFormat"),
        duration: number("duration", "duration"),
        extra: object.cloned().unwrap_or_default(),
    }
}

pub fn create_proxy_cache_key(
    source_path: &str,
    source_id: Option<&str>,
    source_stat: &ProxySourceStat,
    settings: &Value,
) -> Result<String> {
    let resolved = absolute_path(source_path);
    let mut identity = Map::new();
    identity.insert("path".into(), Value::String(resolved));
    if let Some(id) = source_id {
        identity.insert("id".into(), Value::String(id.to_string()));
    }
    let mut stat = Map::new();
    stat.insert("size".into(), json_u64(source_stat.size));
    stat.insert("mtimeMs".into(), json_f64(source_stat.mtime_ms));
    if let Some(ctime) = source_stat.ctime_ms {
        stat.insert("ctimeMs".into(), json_f64(ctime));
    }
    if let Some(ino) = source_stat.ino {
        stat.insert("ino".into(), json_u64(ino));
    }
    if let Some(mode) = source_stat.mode {
        stat.insert("mode".into(), json_u64(u64::from(mode)));
    }
    let payload = Value::Object({
        let mut root = Map::new();
        root.insert("version".into(), Value::Number(Number::from(1)));
        root.insert("identity".into(), Value::Object(identity));
        root.insert("stat".into(), Value::Object(stat));
        root.insert("settings".into(), settings.clone());
        root
    });
    let encoded = canonical(&payload)?;
    let digest = Sha256::digest(encoded.as_bytes());
    Ok(hex::encode(digest))
}

pub fn proxy_ffmpeg_args(
    input: &str,
    output: &str,
    settings: &ProxySettings,
) -> Result<Vec<String>> {
    let mut args = crate::ffmpeg::video_decode_prefix().to_vec();
    args.extend([
        "-hide_banner".into(),
        "-y".into(),
        "-i".into(),
        input.to_string(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0?".into(),
    ]);
    if let (Some(width), Some(height)) = (settings.width, settings.height) {
        args.push("-vf".into());
        args.push(format!(
            "scale={}:{}",
            even_dimension(width, "width")?,
            even_dimension(height, "height")?
        ));
    } else if settings.max_width.is_some() || settings.max_height.is_some() {
        let w = match settings.max_width {
            Some(value) => even_dimension(value, "maxWidth")?,
            None => -2,
        };
        let h = match settings.max_height {
            Some(value) => even_dimension(value, "maxHeight")?,
            None => -2,
        };
        args.push("-vf".into());
        args.push(format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease"
        ));
    }
    if let Some(fps) = settings.fps {
        if fps.is_finite() && fps > 0.0 {
            args.push("-r".into());
            args.push(fps.to_string());
        }
    }
    args.extend(proxy_video_codec_args(settings)?);
    args.push("-c:a".into());
    args.push(settings.audio_codec.clone().unwrap_or_else(|| "aac".into()));
    if let Some(pixel) = &settings.pixel_format {
        if !pixel.is_empty() {
            args.push("-pix_fmt".into());
            args.push(pixel.clone());
        }
    }
    let format = settings
        .format
        .clone()
        .unwrap_or_else(|| extension_of(settings));
    if format == "mp4" {
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push(output.to_string());
    Ok(args)
}

fn proxy_video_codec_args(settings: &ProxySettings) -> Result<Vec<String>> {
    let budget = budget();
    if let Some(codec) = &settings.video_codec {
        let mut args = vec!["-c:v".into(), codec.clone()];
        append_software_quality(&mut args, codec, settings, budget);
        return Ok(args);
    }
    if should_try_hardware_encode(false, false) {
        let select = SelectEncoderOptions {
            prefer_hardware: true,
            codec: VideoCodec::H264,
            ..Default::default()
        };
        if let Ok(selection) = detect_and_select_encoder(&select) {
            return Ok(encode_output_args(
                &selection,
                settings
                    .crf
                    .filter(|value| value.is_finite())
                    .map(|value| value.clamp(0.0, 51.0) as u8)
                    .unwrap_or(budget.software_crf),
                settings
                    .preset
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or(budget.software_preset),
            ));
        }
    }
    let mut args = vec!["-c:v".into(), "libx264".into()];
    append_software_quality(&mut args, "libx264", settings, budget);
    Ok(args)
}

fn append_software_quality(
    args: &mut Vec<String>,
    codec: &str,
    settings: &ProxySettings,
    budget: &crate::resources::ResourceBudget,
) {
    let software = codec == "libx264" || codec == "libx265";
    if !software {
        if let Some(crf) = settings.crf.filter(|value| value.is_finite()) {
            args.push("-crf".into());
            args.push(crf.to_string());
        }
        if let Some(preset) = settings.preset.as_deref().filter(|value| !value.is_empty()) {
            args.push("-preset".into());
            args.push(preset.to_string());
        }
        return;
    }
    let crf = settings
        .crf
        .filter(|value| value.is_finite())
        .unwrap_or(f64::from(budget.software_crf));
    args.push("-crf".into());
    args.push(crf.to_string());
    let preset = settings
        .preset
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(budget.software_preset);
    args.push("-preset".into());
    args.push(preset.to_string());
}

/// Transcode a proxy via ffmpeg with stderr progress JSON lines.
pub fn proxy_run(
    input: &str,
    output: &str,
    settings: &ProxySettings,
    duration: Option<f64>,
) -> Result<()> {
    let args = proxy_ffmpeg_args(input, output, settings)?;
    run_ffmpeg_with_progress(
        &args,
        duration.unwrap_or(0.0),
        &CancellationToken::new(),
        Some(stderr_progress_callback()),
    )
}

fn absolute_path(path: &str) -> String {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return path.to_string();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(candidate))
        .unwrap_or_else(|_| candidate.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn json_u64(value: u64) -> Value {
    Value::Number(Number::from(value))
}

fn json_f64(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Proxies are at most this wide.
pub const PROXY_WIDTH: u32 = 960;
const PROXY_FILE: &str = "proxy.mp4";

fn proxy_entry(
    src: &Path,
    cache: &crate::derivatives::DerivativeCache,
) -> Result<crate::derivatives::CacheEntry> {
    cache.source_entry(
        src,
        "proxy",
        &serde_json::json!({"version": 1, "width": PROXY_WIDTH}),
    )
}

/// The proxy of `src`, when one has been made.
pub fn cached_proxy(
    src: &Path,
    cache: &crate::derivatives::DerivativeCache,
) -> Option<std::path::PathBuf> {
    let entry = proxy_entry(src, cache).ok()?;
    entry
        .is_complete(&[PROXY_FILE])
        .then(|| entry.path(PROXY_FILE))
}

/// Make the proxy of `src`: its picture at most [`PROXY_WIDTH`] wide, frame
/// for frame, with its sound. `None` when `src` has no picture or is no
/// wider than a proxy.
pub fn make_proxy(
    src: &Path,
    cache: &crate::derivatives::DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<Option<std::path::PathBuf>> {
    let probe = crate::probe::info(&src.to_string_lossy())?;
    if probe.width.is_none_or(|width| width <= PROXY_WIDTH) {
        return Ok(None);
    }
    let entry = proxy_entry(src, cache)?;
    if entry.is_complete(&[PROXY_FILE]) {
        return Ok(Some(entry.path(PROXY_FILE)));
    }
    let partial = entry.path("proxy.partial.mp4");
    let mut command = std::process::Command::new(crate::ffmpeg::resolve_ffmpeg_bin());
    command
        .args(crate::ffmpeg::video_decode_prefix())
        .args(["-hide_banner", "-nostats", "-y", "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-map", "0:a:0?"])
        .args(["-vf", &format!("scale={PROXY_WIDTH}:-2")])
        .args([
            "-fps_mode",
            "passthrough",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
        ])
        .args(["-crf", "23", "-g", "30", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "128k", "-f", "mp4"])
        .arg(&partial);
    let made = crate::sidecar::run(command, cancellation, |_| {}).and_then(|finished| {
        if !finished.status.success() {
            return Err(EngineError::msg(format!(
                "proxy failed: {}",
                finished.stderr.trim()
            )));
        }
        Ok(std::fs::rename(&partial, entry.path(PROXY_FILE))?)
    });
    if made.is_err() {
        // Cancelled, failed, or not published: nothing reads the partial.
        let _ = std::fs::remove_file(&partial);
    }
    made?;
    entry.mark_complete(&[PROXY_FILE])?;
    Ok(Some(entry.path(PROXY_FILE)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn even_proxy_scale() {
        let settings = ProxySettings {
            width: Some(321.0),
            height: Some(241.0),
            ..ProxySettings::default()
        };
        let args = proxy_ffmpeg_args("input.mp4", "output.mp4", &settings).unwrap();
        assert!(args.iter().any(|arg| arg == "scale=320:240"));
    }

    #[test]
    fn key_changes_with_stat_and_settings() {
        let stat = ProxySourceStat {
            size: 3,
            mtime_ms: 10.0,
            ..ProxySourceStat::default()
        };
        let a =
            create_proxy_cache_key("/tmp/source.mov", None, &stat, &json!({"width": 320})).unwrap();
        let mut bigger = stat.clone();
        bigger.size = 4;
        let b = create_proxy_cache_key("/tmp/source.mov", None, &bigger, &json!({"width": 320}))
            .unwrap();
        let c =
            create_proxy_cache_key("/tmp/source.mov", None, &stat, &json!({"width": 640})).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
