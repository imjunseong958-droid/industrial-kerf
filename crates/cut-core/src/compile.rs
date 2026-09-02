//! Version 2 filtergraph compiler.

use crate::overlay::{
    atempo_filters, captions_filter, escape_drawtext_text, fmt, subtitles_filter, CaptionCue,
};
use regex::Regex;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static T_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bt\b").expect("valid regex"));

fn rewrite_t_token<'a>(expression: &'a str, replacement: &str) -> Cow<'a, str> {
    T_TOKEN_RE.replace_all(expression, replacement)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledV2FilterGraph {
    pub argv: Vec<String>,
    pub command: String,
    pub filter_complex: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct CompileV2Options {
    pub video_post_filters: Vec<String>,
    pub include_embedded_audio: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("Unsupported v2 {feature}: {detail}")]
    UnsupportedFeature { feature: String, detail: String },
    #[error("{0}")]
    Message(String),
}

impl CompileError {
    pub fn feature(&self) -> Option<&str> {
        match self {
            CompileError::UnsupportedFeature { feature, .. } => Some(feature),
            CompileError::Message(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ColorEffectParams {
    exposure: f64,
    contrast: f64,
    brightness: f64,
    saturation: f64,
    temperature: f64,
    tint: f64,
    gamma: f64,
    highlights: f64,
    shadows: f64,
    vibrance: f64,
    mix: f64,
}

#[derive(Debug, Clone)]
struct Crop {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct NumberKeyframe {
    time: f64,
    value: f64,
}

#[derive(Debug, Clone)]
struct VisualSpec {
    #[allow(dead_code)]
    opacity: f64,
    opacity_expression: Option<String>,
    scale_expression_x: String,
    scale_expression_y: String,
    x_expression: String,
    y_expression: String,
    rotation_expression: Option<String>,
    crop: Option<Crop>,
}

#[derive(Debug, Clone)]
struct NormalizedClip {
    id: String,
    source_id: Option<String>,
    start: f64,
    duration: f64,
    speed: f64,
    source_in: f64,
    source_out: f64,
    volume: f64,
    opacity: f64,
    color_effects: Vec<ColorEffectParams>,
    visual: VisualSpec,
    raw: Value,
}

#[derive(Debug, Clone)]
struct Gap {
    start: f64,
    duration: f64,
}

#[derive(Debug, Clone)]
struct NormalizedTrack {
    id: String,
    track_type: String,
    enabled: bool,
    muted: bool,
    clips: Vec<NormalizedClip>,
    gaps: Vec<Gap>,
    raw: Value,
}

#[derive(Debug, Clone)]
struct Canvas {
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
}

struct Labels {
    id: u32,
}

impl Labels {
    fn new() -> Self {
        Self { id: 0 }
    }

    fn next(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.id);
        self.id += 1;
        label
    }
}

const COLOR_PARAM_KEYS: &[&str] = &[
    "exposure",
    "contrast",
    "brightness",
    "saturation",
    "temperature",
    "tint",
    "gamma",
    "highlights",
    "shadows",
    "vibrance",
    "mix",
];

fn unsupported(feature: &str, detail: impl Into<String>) -> CompileError {
    CompileError::UnsupportedFeature {
        feature: feature.to_string(),
        detail: detail.into(),
    }
}

fn msg(detail: impl Into<String>) -> CompileError {
    CompileError::Message(detail.into())
}

fn record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

/// Read the canonical v2 field, falling back to one input alias at the JSON boundary.
fn field<'a>(obj: &'a Map<String, Value>, canonical: &str, alias: &str) -> Option<&'a Value> {
    obj.get(canonical).or_else(|| obj.get(alias))
}

fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

fn finite_num(value: f64) -> bool {
    value.is_finite()
}

fn positive(value: &Value) -> Option<f64> {
    finite(value).filter(|v| *v > 0.0)
}

#[allow(dead_code)]
fn positive_num(value: f64) -> bool {
    finite_num(value) && value > 0.0
}

fn nonnegative(value: &Value) -> Option<f64> {
    finite(value).filter(|v| *v >= 0.0)
}

fn nonnegative_num(value: f64) -> bool {
    finite_num(value) && value >= 0.0
}

fn even_dimension(value: f64) -> u32 {
    let v = value.max(2.0);
    ((v / 2.0).floor() * 2.0) as u32
}

fn source_id_of(clip: &Map<String, Value>) -> Option<String> {
    field(clip, "sourceId", "src")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn track_type_of(track: &Map<String, Value>) -> String {
    field(track, "type", "kind")
        .and_then(|v| v.as_str())
        .unwrap_or("video")
        .to_string()
}

fn clip_start_of(clip: &Map<String, Value>, id: &str) -> Result<f64, CompileError> {
    let value = field(clip, "timelineStart", "start")
        .map(|v| {
            if v.is_null() {
                return Ok(0.0);
            }
            nonnegative(v).ok_or_else(|| {
                msg(format!(
                    "v2 clip {id} timelineStart must be finite and nonnegative"
                ))
            })
        })
        .transpose()?;
    Ok(value.unwrap_or(0.0))
}

fn clip_speed_of(clip: &Map<String, Value>, id: &str) -> Result<f64, CompileError> {
    let value = clip.get("speed").map(|v| {
        positive(v).ok_or_else(|| msg(format!("v2 clip {id} speed must be finite and positive")))
    });
    match value {
        Some(result) => result,
        None => Ok(1.0),
    }
}

fn transition_type(value: &Value) -> Option<String> {
    let obj = record(value)?;
    let t = field(obj, "type", "kind").and_then(|v| v.as_str())?;
    Some(t.to_lowercase())
}

fn parse_color_number(
    obj: &Map<String, Value>,
    key: &str,
    min: f64,
    max: f64,
    default: f64,
) -> Result<f64, CompileError> {
    match obj.get(key) {
        None => Ok(default),
        Some(value) => {
            let n = finite(value)
                .ok_or_else(|| msg(format!("params.{key} must be a finite number")))?;
            if n < min || n > max {
                return Err(msg(format!(
                    "params.{key}: Number must be greater than or equal to {min} and less than or equal to {max}"
                )));
            }
            Ok(n)
        }
    }
}

fn parse_color_effect_params(
    value: &Value,
    context: &str,
) -> Result<ColorEffectParams, CompileError> {
    let obj = if value.is_null() {
        &Map::new()
    } else {
        record(value)
            .ok_or_else(|| msg(format!("{context} color effect params must be an object")))?
    };

    let params = ColorEffectParams {
        exposure: parse_color_number(obj, "exposure", -3.0, 3.0, 0.0)?,
        contrast: parse_color_number(obj, "contrast", 0.0, 4.0, 1.0)?,
        brightness: parse_color_number(obj, "brightness", -1.0, 1.0, 0.0)?,
        saturation: parse_color_number(obj, "saturation", 0.0, 4.0, 1.0)?,
        temperature: parse_color_number(obj, "temperature", -1000.0, 1000.0, 0.0)?,
        tint: parse_color_number(obj, "tint", -1.0, 1.0, 0.0)?,
        gamma: parse_color_number(obj, "gamma", 0.1, 10.0, 1.0)?,
        highlights: parse_color_number(obj, "highlights", -1.0, 1.0, 0.0)?,
        shadows: parse_color_number(obj, "shadows", -1.0, 1.0, 0.0)?,
        vibrance: parse_color_number(obj, "vibrance", -1.0, 1.0, 0.0)?,
        mix: parse_color_number(obj, "mix", 0.0, 1.0, 1.0)?,
    };

    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|key| !COLOR_PARAM_KEYS.contains(key))
        .collect();

    if !unknown.is_empty() {
        if let Some(lut) = unknown
            .iter()
            .find(|key| key.to_lowercase().contains("lut"))
        {
            return Err(unsupported(
                "color LUT",
                format!("{context} uses {lut}; LUT files are not supported by the v2 compiler yet"),
            ));
        }
        return Err(unsupported(
            "color parameter",
            format!(
                "{context} uses {}; supported parameters are {}",
                unknown.join(", "),
                COLOR_PARAM_KEYS.join(", ")
            ),
        ));
    }

    Ok(params)
}

fn normalize_color_effects(
    value: Option<&Value>,
    context: &str,
    allow_color: bool,
) -> Result<Vec<ColorEffectParams>, CompileError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(arr) = value.as_array() else {
        return Err(msg(format!("v2 {context} effects must be an array")));
    };

    let mut colors = Vec::new();
    for effect in arr {
        let Some(obj) = record(effect) else {
            continue;
        };
        if obj.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let name = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if name.to_lowercase() != "color" {
            return Err(unsupported("effect", format!("{context} uses {name}")));
        }
        if !allow_color {
            return Err(unsupported(
                "color effect",
                format!("{context}; color effects are supported only on video/title clips"),
            ));
        }
        colors.push(parse_color_effect_params(
            obj.get("params").unwrap_or(&Value::Null),
            context,
        )?);
    }
    Ok(colors)
}

fn ensure_no_unsupported_effects(value: Option<&Value>, context: &str) -> Result<(), CompileError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(arr) = value.as_array() else {
        return Err(msg(format!("v2 {context} effects must be an array")));
    };
    for effect in arr {
        let Some(obj) = record(effect) else {
            continue;
        };
        if obj.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let name = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if name.to_lowercase() == "color" {
            return Err(unsupported(
                "color effect",
                format!("{context}; color effects are supported only on video/title clips"),
            ));
        }
        return Err(unsupported("effect", format!("{context} uses {name}")));
    }
    Ok(())
}

fn ensure_supported_blend(value: Option<&Value>, context: &str) -> Result<(), CompileError> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(s)) if s == "normal" || s == "over" => Ok(()),
        Some(other) => Err(unsupported(
            "blend mode",
            format!("{context} requests {other}; only normal/over is supported"),
        )),
    }
}

fn read_number(
    value: &Value,
    context: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<f64, CompileError> {
    let n = finite(value).ok_or_else(|| msg(format!("{context} must be a finite number")))?;
    if let Some(min) = min {
        if n < min {
            return Err(msg(format!("{context} must be >= {min}")));
        }
    }
    if let Some(max) = max {
        if n > max {
            return Err(msg(format!("{context} must be <= {max}")));
        }
    }
    Ok(n)
}

fn normalize_crop(raw: Option<&Value>, context: &str) -> Result<Option<Crop>, CompileError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(value) = record(raw) else {
        return Err(msg(format!("{context} crop must be an object")));
    };
    let x = read_number(
        value.get("x").unwrap_or(&Value::Null),
        &format!("{context} crop.x"),
        Some(0.0),
        Some(1.0),
    )?;
    let y = read_number(
        value.get("y").unwrap_or(&Value::Null),
        &format!("{context} crop.y"),
        Some(0.0),
        Some(1.0),
    )?;
    let width = read_number(
        value.get("width").unwrap_or(&Value::Null),
        &format!("{context} crop.width"),
        Some(0.0),
        Some(1.0),
    )?;
    let height = read_number(
        value.get("height").unwrap_or(&Value::Null),
        &format!("{context} crop.height"),
        Some(0.0),
        Some(1.0),
    )?;
    if width <= 0.0 || height <= 0.0 {
        return Err(msg(format!("{context} crop width/height must be positive")));
    }
    if x + width > 1.0 || y + height > 1.0 {
        return Err(msg(format!(
            "{context} crop must stay within normalized source bounds"
        )));
    }
    Ok(Some(Crop {
        x,
        y,
        width,
        height,
    }))
}

fn crop_filter(crop: &Crop) -> String {
    let width = format!("max(2\\,trunc(iw*{}/2)*2)", fmt(crop.width));
    let height = format!("max(2\\,trunc(ih*{}/2)*2)", fmt(crop.height));
    let x = format!("trunc(iw*{}/2)*2", fmt(crop.x));
    let y = format!("trunc(ih*{}/2)*2", fmt(crop.y));
    format!("crop=w='{width}':h='{height}':x='{x}':y='{y}'")
}

fn transform_object(raw: Option<&Value>) -> Map<String, Value> {
    raw.and_then(|v| v.as_object()).cloned().unwrap_or_default()
}

fn transform_number(
    transform: &Map<String, Value>,
    name: &str,
    fallback: f64,
    context: &str,
) -> Result<f64, CompileError> {
    match transform.get(name) {
        None => Ok(fallback),
        Some(value) => read_number(value, &format!("{context} transform.{name}"), None, None),
    }
}

fn canonical_keyframe_property(property: &str) -> Option<&'static str> {
    let normalized: String = property
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    match normalized.as_str() {
        "opacity" => Some("opacity"),
        "scale" | "transform.scale" | "transform.scaleuniform" => Some("scale"),
        "scalex" | "transform.scalex" => Some("scaleX"),
        "scaley" | "transform.scaley" => Some("scaleY"),
        "x" | "position.x" | "transform.x" | "transform.position.x" => Some("x"),
        "y" | "position.y" | "transform.y" | "transform.position.y" => Some("y"),
        "rotation" | "transform.rotation" => Some("rotation"),
        _ => None,
    }
}

fn keyframes_for(
    clip: &Map<String, Value>,
    clip_id: &str,
    property: &str,
    duration: f64,
) -> Result<Vec<NumberKeyframe>, CompileError> {
    let Some(keyframes) = clip.get("keyframes").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for entry in keyframes {
        let Some(obj) = record(entry) else {
            return Err(msg(format!("v2 clip {clip_id} keyframe must be an object")));
        };
        let raw_property = obj
            .get("property")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                msg(format!(
                    "v2 clip {clip_id} keyframe property must be a string"
                ))
            })?;
        let canonical = canonical_keyframe_property(raw_property)
            .ok_or_else(|| unsupported("keyframe", format!("{clip_id} uses {raw_property}")))?;
        if canonical != property {
            continue;
        }
        let time = read_number(
            obj.get("time").unwrap_or(&Value::Null),
            &format!("v2 clip {clip_id} keyframe time"),
            Some(0.0),
            None,
        )?;
        if time > duration + 1e-6 {
            return Err(msg(format!(
                "v2 clip {clip_id} keyframe time {time} exceeds clip duration {duration}"
            )));
        }
        if let Some(easing) = obj.get("easing").and_then(|v| v.as_str()) {
            if easing != "linear" {
                return Err(unsupported(
                    "keyframe easing",
                    format!("{clip_id} requests {easing}; only linear is supported"),
                ));
            }
        }
        let value = read_number(
            obj.get("value").unwrap_or(&Value::Null),
            &format!("v2 clip {clip_id} {property} keyframe value"),
            None,
            None,
        )?;
        result.push(NumberKeyframe { time, value });
    }
    result.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in 1..result.len() {
        if result[i].time == result[i - 1].time {
            return Err(msg(format!(
                "v2 clip {clip_id} has duplicate {property} keyframe times"
            )));
        }
    }
    Ok(result)
}

fn keyframe_expression(
    keyframes: &[NumberKeyframe],
    fallback: f64,
    context: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<Option<String>, CompileError> {
    if keyframes.is_empty() {
        return Ok(None);
    }
    let mut points = Vec::with_capacity(keyframes.len());
    for point in keyframes {
        points.push(NumberKeyframe {
            time: point.time,
            value: read_number(
                &Value::from(point.value),
                &format!("{context} keyframe value"),
                min,
                max,
            )?,
        });
    }
    let mut expression = fmt(points[points.len() - 1].value);
    for i in (0..points.len().saturating_sub(1)).rev() {
        let current = &points[i];
        let next = &points[i + 1];
        let slope = format!(
            "({}-{})*(t-{})/{}",
            fmt(next.value),
            fmt(current.value),
            fmt(current.time),
            fmt(next.time - current.time)
        );
        let segment = format!("({}+{})", fmt(current.value), slope);
        expression = format!(
            "if(lt(t\\,{})\\,{}\\,{})",
            fmt(next.time),
            segment,
            expression
        );
    }
    let first = &points[0];
    if first.time > 0.0 {
        expression = format!(
            "if(lt(t\\,{})\\,{}\\,{})",
            fmt(first.time),
            fmt(fallback),
            expression
        );
    }
    Ok(Some(expression))
}

fn expression_with_timeline_time(expression: &str, start: f64) -> String {
    rewrite_t_token(expression, &format!("(t-{})", fmt(start))).into_owned()
}

fn read_transform_keyframes(
    clip: &Map<String, Value>,
    clip_id: &str,
    duration: f64,
    transform: &Map<String, Value>,
    context: &str,
) -> Result<HashMap<String, String>, CompileError> {
    let default_opacity = Value::from(1.0);
    let opacity_fallback = read_number(
        clip.get("opacity").unwrap_or(&default_opacity),
        &format!("{context} opacity"),
        Some(0.0),
        Some(1.0),
    )?;
    let mut out = HashMap::new();
    if let Some(expr) = keyframe_expression(
        &keyframes_for(clip, clip_id, "opacity", duration)?,
        opacity_fallback,
        &format!("{context} opacity"),
        Some(0.0),
        Some(1.0),
    )? {
        out.insert("opacity".to_string(), expr);
    }
    for (prop, min) in [
        ("scale", Some(0.000001)),
        ("scaleX", Some(0.000001)),
        ("scaleY", Some(0.000001)),
        ("x", None),
        ("y", None),
        ("rotation", None),
    ] {
        let fallback = transform_number(
            transform,
            prop,
            if prop == "rotation" || prop == "x" || prop == "y" {
                0.0
            } else {
                1.0
            },
            context,
        )?;
        if let Some(expr) = keyframe_expression(
            &keyframes_for(clip, clip_id, prop, duration)?,
            fallback,
            &format!("{context} {prop}"),
            min,
            None,
        )? {
            out.insert(prop.to_string(), expr);
        }
    }
    Ok(out)
}

fn normalize_visual(
    clip: &Map<String, Value>,
    clip_id: &str,
    duration: f64,
) -> Result<VisualSpec, CompileError> {
    let context = format!("clip {clip_id}");
    let transform = transform_object(clip.get("transform"));
    let position = transform.get("position").and_then(|v| v.as_object());
    let anchor = transform.get("anchor").and_then(|v| v.as_object());

    for unsupported_key in ["z", "rotationX", "rotationY", "anchorX", "anchorY"] {
        if transform.contains_key(unsupported_key) {
            return Err(unsupported(
                "transform",
                format!("{context} uses {unsupported_key}"),
            ));
        }
    }
    if let Some(anchor) = anchor {
        if anchor.contains_key("x") || anchor.contains_key("y") {
            return Err(unsupported(
                "transform anchor",
                format!("{context} uses anchor"),
            ));
        }
    }
    if let Some(position) = position {
        if position.contains_key("z") {
            return Err(unsupported(
                "transform position",
                format!("{context} uses position.z"),
            ));
        }
    }

    let default_zero = Value::from(0.0);
    let base_x = transform
        .get("x")
        .or_else(|| position.and_then(|p| p.get("x")))
        .unwrap_or(&default_zero);
    let base_y = transform
        .get("y")
        .or_else(|| position.and_then(|p| p.get("y")))
        .unwrap_or(&default_zero);
    let x = read_number(base_x, &format!("{context} transform.x"), None, None)?;
    let y = read_number(base_y, &format!("{context} transform.y"), None, None)?;
    let default_opacity = Value::from(1.0);
    let opacity = read_number(
        clip.get("opacity").unwrap_or(&default_opacity),
        &format!("{context} opacity"),
        Some(0.0),
        Some(1.0),
    )?;
    let keyframes = read_transform_keyframes(clip, clip_id, duration, &transform, &context)?;
    let crop = normalize_crop(clip.get("crop"), &context)?;

    let static_scale = transform_number(&transform, "scale", 1.0, &context)?;
    let static_scale_x = transform_number(&transform, "scaleX", 1.0, &context)?;
    let static_scale_y = transform_number(&transform, "scaleY", 1.0, &context)?;
    let static_rotation = transform_number(&transform, "rotation", 0.0, &context)?;

    let scale_expression_x = if keyframes.contains_key("scaleX") || keyframes.contains_key("scale")
    {
        let static_scale_fmt = fmt(static_scale);
        let inner = keyframes
            .get("scaleX")
            .or_else(|| keyframes.get("scale"))
            .map(String::as_str)
            .unwrap_or(&static_scale_fmt);
        format!("({inner})*{}", fmt(static_scale_x))
    } else {
        fmt(static_scale * static_scale_x)
    };
    let scale_expression_y = if keyframes.contains_key("scaleY") || keyframes.contains_key("scale")
    {
        let static_scale_fmt = fmt(static_scale);
        let inner = keyframes
            .get("scaleY")
            .or_else(|| keyframes.get("scale"))
            .map(String::as_str)
            .unwrap_or(&static_scale_fmt);
        format!("({inner})*{}", fmt(static_scale_y))
    } else {
        fmt(static_scale * static_scale_y)
    };
    let x_expression = keyframes.get("x").cloned().unwrap_or_else(|| fmt(x));
    let y_expression = keyframes.get("y").cloned().unwrap_or_else(|| fmt(y));
    let rotation_expression = keyframes.get("rotation").cloned().or_else(|| {
        if static_rotation != 0.0 {
            Some(fmt(static_rotation))
        } else {
            None
        }
    });
    let opacity_expression = keyframes.get("opacity").cloned();

    Ok(VisualSpec {
        opacity,
        opacity_expression,
        scale_expression_x,
        scale_expression_y,
        x_expression,
        y_expression,
        rotation_expression,
        crop,
    })
}

fn source_duration(source: &Map<String, Value>) -> Option<f64> {
    source.get("duration").and_then(positive)
}

fn normalize_clip(
    clip_value: &Value,
    source_by_id: &HashMap<String, Map<String, Value>>,
    track_type: &str,
) -> Result<NormalizedClip, CompileError> {
    let clip = record(clip_value).ok_or_else(|| msg("v2 clip must be an object"))?;
    let id = clip
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| msg("v2 clip requires id"))?
        .to_string();
    let start = clip_start_of(clip, &id)?;
    let speed = clip_speed_of(clip, &id)?;
    let source_id = source_id_of(clip);
    let source = source_id.as_ref().and_then(|sid| source_by_id.get(sid));

    if (track_type == "video" || track_type == "audio") && source_id.is_none() {
        return Err(msg(format!(
            "v2 {track_type} clip {id} requires a sourceId"
        )));
    }
    if source_id.is_some() && source.is_none() {
        return Err(msg(format!(
            "Unknown v2 source id: {}",
            source_id.as_deref().unwrap_or("")
        )));
    }

    let color_effects = normalize_color_effects(
        clip.get("effects"),
        &format!("clip {id}"),
        track_type == "video" || track_type == "title",
    )?;
    ensure_supported_blend(clip.get("blend"), &format!("clip {id}"))?;

    let source_in = field(clip, "in", "sourceIn")
        .map(|v| {
            nonnegative(v).ok_or_else(|| {
                msg(format!(
                    "v2 clip {id} source in must be finite and nonnegative"
                ))
            })
        })
        .transpose()?
        .unwrap_or(0.0);

    let source_out_value = field(clip, "out", "sourceOut");
    if let Some(value) = source_out_value {
        if positive(value).is_none() {
            return Err(msg(format!(
                "v2 clip {id} source out must be finite and positive"
            )));
        }
        let out = value.as_f64().unwrap_or(0.0);
        if out <= source_in {
            return Err(msg(format!(
                "v2 clip {id} source out must be greater than source in"
            )));
        }
    }

    let source_out = if let Some(out) = source_out_value.and_then(positive) {
        out
    } else {
        source.and_then(source_duration).unwrap_or(0.0)
    };

    let duration = if let Some(explicit) = clip.get("duration") {
        let d = positive(explicit)
            .ok_or_else(|| msg(format!("v2 clip {id} duration must be finite and positive")))?;
        d
    } else if source_out > source_in {
        (source_out - source_in) / speed
    } else {
        return Err(msg(format!(
            "v2 clip {id} requires duration or a source in/out interval"
        )));
    };

    let actual_source_out = if source_out > source_in {
        source_out
    } else {
        source_in + duration * speed
    };

    let volume = clip
        .get("volume")
        .map(|v| {
            nonnegative(v).ok_or_else(|| {
                msg(format!(
                    "v2 clip {id} volume must be finite and nonnegative"
                ))
            })
        })
        .transpose()?
        .unwrap_or(1.0);

    let default_opacity = Value::from(1.0);
    let opacity_val = clip.get("opacity").unwrap_or(&default_opacity);
    let opacity = finite(opacity_val)
        .filter(|v| (0.0..=1.0).contains(v))
        .ok_or_else(|| msg(format!("v2 clip {id} opacity must be finite in [0, 1]")))?;

    let visual = normalize_visual(clip, &id, duration)?;

    Ok(NormalizedClip {
        id,
        source_id,
        start,
        duration,
        speed,
        source_in,
        source_out: actual_source_out,
        volume,
        opacity,
        color_effects,
        visual,
        raw: clip_value.clone(),
    })
}

fn normalize_gap(value: &Value, context: &str) -> Result<Gap, CompileError> {
    let gap = record(value).ok_or_else(|| msg(format!("{context} must be an object")))?;
    let start = field(gap, "start", "timelineStart")
        .map(|v| {
            if v.is_null() {
                return Ok(0.0);
            }
            nonnegative(v).ok_or_else(|| {
                msg(format!(
                    "{context} requires finite start >= 0 and duration > 0"
                ))
            })
        })
        .transpose()?
        .unwrap_or(0.0);
    let duration = gap.get("duration").and_then(positive).ok_or_else(|| {
        msg(format!(
            "{context} requires finite start >= 0 and duration > 0"
        ))
    })?;
    if !nonnegative_num(start) {
        return Err(msg(format!(
            "{context} requires finite start >= 0 and duration > 0"
        )));
    }
    Ok(Gap { start, duration })
}

fn normalize_track(
    track_value: &Value,
    source_by_id: &HashMap<String, Map<String, Value>>,
) -> Result<NormalizedTrack, CompileError> {
    let track = record(track_value).ok_or_else(|| msg("v2 track must be an object"))?;
    let id = track
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| msg("v2 track requires id"))?
        .to_string();
    let track_type = track_type_of(track);
    if !["video", "audio", "title", "adjustment"].contains(&track_type.as_str()) {
        return Err(unsupported("track type", format!("{id} uses {track_type}")));
    }
    ensure_no_unsupported_effects(track.get("effects"), &format!("track {id}"))?;
    ensure_supported_blend(track.get("blend"), &format!("track {id}"))?;
    if let Some(transitions) = track.get("transitions") {
        if !transitions.is_array() {
            return Err(msg(format!("v2 track {id} transitions must be an array")));
        }
    }

    let clips = track
        .get("clips")
        .and_then(|v| v.as_array())
        .ok_or_else(|| msg(format!("v2 track {id} requires clips array")))?
        .iter()
        .map(|clip| normalize_clip(clip, source_by_id, &track_type))
        .collect::<Result<Vec<_>, _>>()?;

    let gaps = if let Some(arr) = track.get("gaps").and_then(|v| v.as_array()) {
        arr.iter()
            .enumerate()
            .map(|(index, gap)| normalize_gap(gap, &format!("v2 track {id} gap {index}")))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };

    Ok(NormalizedTrack {
        id,
        track_type,
        enabled: track.get("enabled").and_then(|v| v.as_bool()) != Some(false),
        muted: track.get("muted").and_then(|v| v.as_bool()) == Some(true),
        clips,
        gaps,
        raw: track_value.clone(),
    })
}

fn choose_canvas(project: &Map<String, Value>, sources: &[Map<String, Value>]) -> (u32, u32) {
    let width = project.get("width").and_then(positive);
    let height = project.get("height").and_then(positive);
    if let (Some(w), Some(h)) = (width, height) {
        return (even_dimension(w), even_dimension(h));
    }

    let mut complete: Vec<(f64, f64)> = sources
        .iter()
        .filter_map(|source| {
            let w = source.get("width").and_then(positive)?;
            let h = source.get("height").and_then(positive)?;
            Some((w, h))
        })
        .collect();
    complete.sort_by(|a, b| {
        (b.0 * b.1)
            .partial_cmp(&(a.0 * a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some((w, h)) = complete.first().copied() {
        return (even_dimension(w), even_dimension(h));
    }
    if let Some(w) = width {
        return (even_dimension(w), even_dimension(w * 9.0 / 16.0));
    }
    if let Some(h) = height {
        return (even_dimension(h * 16.0 / 9.0), even_dimension(h));
    }
    (1920, 1080)
}

fn transition_for(
    previous: &NormalizedClip,
    next: &NormalizedClip,
    track: &NormalizedTrack,
) -> Result<Option<(String, f64)>, CompileError> {
    let mut candidates: Vec<&Value> = Vec::new();
    if let Some(raw) = record(&next.raw) {
        if let Some(arr) = raw.get("transitions").and_then(|v| v.as_array()) {
            candidates.extend(arr.iter());
        }
    }
    if let Some(raw) = record(&previous.raw) {
        if let Some(arr) = raw.get("transitions").and_then(|v| v.as_array()) {
            candidates.extend(arr.iter());
        }
    }
    if let Some(raw) = record(&next.raw) {
        if let Some(t) = raw.get("transition") {
            candidates.push(t);
        }
    }
    if let Some(raw) = record(&previous.raw) {
        if let Some(t) = raw.get("transition") {
            candidates.push(t);
        }
    }
    if let Some(raw) = record(&track.raw) {
        if let Some(arr) = raw.get("transitions").and_then(|v| v.as_array()) {
            candidates.extend(arr.iter());
        }
    }

    let candidate = candidates.into_iter().find(|v| !v.is_null());
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let obj = record(candidate).ok_or_else(|| {
        msg(format!(
            "v2 transition between {} and {} requires type and positive duration",
            previous.id, next.id
        ))
    })?;
    let transition_type = transition_type(candidate).ok_or_else(|| {
        msg(format!(
            "v2 transition between {} and {} requires type and positive duration",
            previous.id, next.id
        ))
    })?;
    let duration = obj.get("duration").and_then(positive).ok_or_else(|| {
        msg(format!(
            "v2 transition between {} and {} requires type and positive duration",
            previous.id, next.id
        ))
    })?;

    let supported: HashMap<&str, &str> = [
        ("fade", "fade"),
        ("crossfade", "fade"),
        ("dissolve", "dissolve"),
    ]
    .into_iter()
    .collect();
    let ffmpeg_type = supported
        .get(transition_type.as_str())
        .copied()
        .ok_or_else(|| {
            unsupported(
                "transition",
                format!("{transition_type}; supported transitions are fade/crossfade/dissolve"),
            )
        })?;

    let overlap = previous.start + previous.duration - next.start;
    if overlap <= 0.0 {
        return Err(unsupported(
            "transition",
            format!(
                "{} and {} must overlap for an xfade transition",
                previous.id, next.id
            ),
        ));
    }
    if (duration - overlap).abs() > 1e-6 {
        return Err(msg(format!(
            "v2 transition between {} and {} ({}s) must equal overlap ({}s)",
            previous.id,
            next.id,
            fmt(duration),
            fmt(overlap)
        )));
    }
    Ok(Some((ffmpeg_type.to_string(), duration)))
}

fn close_to_identity(value: f64, identity: f64) -> bool {
    (value - identity).abs() <= 1e-9
}

fn clamp_color_balance(value: f64) -> f64 {
    value.clamp(-1.0, 1.0)
}

fn color_effect_filters(
    params: &ColorEffectParams,
    context: &str,
) -> Result<Vec<String>, CompileError> {
    let mix = params.mix;
    if !finite_num(mix) || !(0.0..=1.0).contains(&mix) {
        return Err(msg(format!("{context} color mix must be finite in [0, 1]")));
    }
    if mix == 0.0 {
        return Ok(Vec::new());
    }

    let mut filters = Vec::new();
    let exposure = params.exposure * mix;
    if !close_to_identity(exposure, 0.0) {
        filters.push(format!("exposure=exposure={}", fmt(exposure)));
    }

    let mut eq = Vec::new();
    let contrast = 1.0 + (params.contrast - 1.0) * mix;
    let brightness = params.brightness * mix;
    let saturation = 1.0 + (params.saturation - 1.0) * mix;
    let gamma = 1.0 + (params.gamma - 1.0) * mix;
    if !close_to_identity(contrast, 1.0) {
        eq.push(format!("contrast={}", fmt(contrast)));
    }
    if !close_to_identity(brightness, 0.0) {
        eq.push(format!("brightness={}", fmt(brightness)));
    }
    if !close_to_identity(saturation, 1.0) {
        eq.push(format!("saturation={}", fmt(saturation)));
    }
    if !close_to_identity(gamma, 1.0) {
        eq.push(format!("gamma={}", fmt(gamma)));
    }
    if !eq.is_empty() {
        filters.push(format!("eq={}", eq.join(":")));
    }

    let temperature = (params.temperature / 1000.0) * mix;
    let tint = params.tint * mix;
    let shadows = params.shadows * mix;
    let highlights = params.highlights * mix;
    let rs = clamp_color_balance(temperature + tint + shadows);
    let gs = clamp_color_balance(-tint + shadows);
    let bs = clamp_color_balance(-temperature + tint + shadows);
    let rh = clamp_color_balance(highlights);
    let gh = clamp_color_balance(highlights);
    let bh = clamp_color_balance(highlights);
    if [rs, gs, bs, rh, gh, bh]
        .iter()
        .any(|value| !close_to_identity(*value, 0.0))
    {
        filters.push(format!(
            "colorbalance=rs={}:gs={}:bs={}:rh={}:gh={}:bh={}:pl=1",
            fmt(rs),
            fmt(gs),
            fmt(bs),
            fmt(rh),
            fmt(gh),
            fmt(bh)
        ));
    }

    let vibrance = params.vibrance * mix;
    if !close_to_identity(vibrance, 0.0) {
        filters.push(format!("vibrance=intensity={}", fmt(vibrance)));
    }
    Ok(filters)
}

fn video_clip_filters(
    clip: &NormalizedClip,
    input_index: usize,
    _canvas: &Canvas,
    labels: &mut Labels,
    parts: &mut Vec<String>,
) -> Result<String, CompileError> {
    let label = labels.next("v2clip");
    let mut filters = vec![
        format!(
            "trim=start={}:end={}",
            fmt(clip.source_in),
            fmt(clip.source_out)
        ),
        "setpts=PTS-STARTPTS".to_string(),
    ];
    if let Some(ref crop) = clip.visual.crop {
        filters.push(crop_filter(crop));
    }

    let scale_x = &clip.visual.scale_expression_x;
    let scale_y = &clip.visual.scale_expression_y;
    if scale_x != "1" || scale_y != "1" {
        filters.push(format!(
            "scale=w='max(2\\,trunc(iw*({scale_x})/2)*2)':h='max(2\\,trunc(ih*({scale_y})/2)*2)':eval=frame"
        ));
    }
    if let Some(ref rotation) = clip.visual.rotation_expression {
        filters.push(format!("rotate='({rotation})*PI/180':fillcolor=none"));
    }
    filters.push("format=rgba".to_string());
    for (index, color) in clip.color_effects.iter().enumerate() {
        filters.extend(color_effect_filters(
            color,
            &format!("clip {} color effect {index}", clip.id),
        )?);
    }
    if let Some(ref expression) = clip.visual.opacity_expression {
        let rewritten = rewrite_t_token(expression, "T");
        filters.push(format!(
            "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({})'",
            rewritten
        ));
    } else if clip.opacity != 1.0 {
        filters.push(format!("colorchannelmixer=aa={}", fmt(clip.opacity)));
    }
    if clip.speed != 1.0 {
        filters.push(format!("setpts={}*PTS", fmt(1.0 / clip.speed)));
    }
    filters.push(format!("trim=duration={}", fmt(clip.duration)));
    filters.push("setpts=PTS-STARTPTS".to_string());
    filters.push(format!("setpts=PTS+{}/TB", fmt(clip.start)));
    parts.push(format!(
        "[{input_index}:v]{filters}[{label}]",
        filters = filters.join(","),
    ));
    Ok(label)
}

fn title_clip(
    clip: &NormalizedClip,
    canvas: &Canvas,
    labels: &mut Labels,
    parts: &mut Vec<String>,
) -> Result<String, CompileError> {
    let raw = record(&clip.raw).ok_or_else(|| msg("title clip raw must be object"))?;
    let text_value = raw
        .get("text")
        .or_else(|| raw.get("title"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| unsupported("title clip", format!("{} has no text/title", clip.id)))?;

    if clip.visual.crop.is_some()
        || clip.visual.scale_expression_x != "1"
        || clip.visual.scale_expression_y != "1"
        || clip.visual.x_expression != "0"
        || clip.visual.y_expression != "0"
        || clip.visual.rotation_expression.is_some()
    {
        return Err(unsupported(
            "title transform",
            format!(
                "{}; title positioning/scale/rotation is not implemented",
                clip.id
            ),
        ));
    }

    let label = labels.next("v2title");
    let text = escape_drawtext_text(text_value);
    let font_size = (canvas.height as f64 / 12.0).max(18.0);
    let mut filters = vec![
        format!(
            "color=c=black@0.0:s={}x{}:r={}:d={}",
            canvas.width,
            canvas.height,
            fmt(canvas.fps),
            fmt(clip.duration)
        ),
        "format=rgba".to_string(),
        format!(
            "drawtext=text={text}:fontcolor=white:fontsize={}:x='(w-text_w)/2':y='(h-text_h)/2':expansion=none",
            fmt(font_size)
        ),
    ];
    for (index, color) in clip.color_effects.iter().enumerate() {
        filters.extend(color_effect_filters(
            color,
            &format!("clip {} color effect {index}", clip.id),
        )?);
    }
    if let Some(ref expression) = clip.visual.opacity_expression {
        let rewritten = rewrite_t_token(expression, "T");
        filters.push(format!(
            "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({})'",
            rewritten
        ));
    } else if clip.opacity != 1.0 {
        filters.push(format!("colorchannelmixer=aa={}", fmt(clip.opacity)));
    }
    filters.push(format!("setpts=PTS+{}/TB", fmt(clip.start)));
    parts.push(format!("{}[{label}]", filters.join(",")));
    Ok(label)
}

fn build_video_track_items(
    track: &NormalizedTrack,
    source_by_id: &HashMap<String, Map<String, Value>>,
    input_index: &HashMap<String, usize>,
    canvas: &Canvas,
    labels: &mut Labels,
    parts: &mut Vec<String>,
) -> Result<Vec<(String, NormalizedClip)>, CompileError> {
    if track.track_type == "adjustment" {
        return Err(unsupported("adjustment track", track.id.clone()));
    }

    let mut clips = track.clips.clone();
    clips.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut result = Vec::new();
    let mut i = 0;
    while i < clips.len() {
        let clip = clips[i].clone();
        let label = if track.track_type == "title" {
            title_clip(&clip, canvas, labels, parts)?
        } else {
            let source_id = clip.source_id.as_deref();
            let source = source_id.and_then(|id| source_by_id.get(id));
            let index = source_id.and_then(|id| input_index.get(id).copied());
            if source.is_none() || index.is_none() {
                return Err(msg(format!(
                    "v2 video clip {} has no usable source",
                    clip.id
                )));
            }
            video_clip_filters(&clip, index.unwrap_or(0), canvas, labels, parts)?
        };

        let next = clips.get(i + 1);
        let transition = if let Some(next) = next {
            transition_for(&clip, next, track)?
        } else {
            None
        };

        if let (Some((transition_type, transition_duration)), Some(next)) =
            (transition, clips.get(i + 1).cloned())
        {
            if clip.visual.x_expression != next.visual.x_expression
                || clip.visual.y_expression != next.visual.y_expression
            {
                return Err(unsupported(
                    "transition positioning",
                    format!(
                        "{}/{}; xfade requires matching normalized positions",
                        clip.id, next.id
                    ),
                ));
            }
            if clip.opacity != 1.0
                || next.opacity != 1.0
                || clip.visual.opacity_expression.is_some()
                || next.visual.opacity_expression.is_some()
            {
                return Err(unsupported(
                    "transition opacity",
                    format!(
                        "{}/{}; xfade transitions require opaque clips",
                        clip.id, next.id
                    ),
                ));
            }

            let next_label = if track.track_type == "title" {
                title_clip(&next, canvas, labels, parts)?
            } else {
                let source_id = next.source_id.as_deref();
                let source = source_id.and_then(|id| source_by_id.get(id));
                let index = source_id.and_then(|id| input_index.get(id).copied());
                if source.is_none() || index.is_none() {
                    return Err(msg(format!(
                        "v2 video clip {} has no usable source",
                        next.id
                    )));
                }
                video_clip_filters(&next, index.unwrap_or(0), canvas, labels, parts)?
            };

            let first_local = labels.next("v2xfadein");
            let second_local = labels.next("v2xfadein");
            let pair = labels.next("v2xfade");
            let mut normalize = |input: &str, output: &str| {
                parts.push(format!(
                    "[{input}]setpts=PTS-STARTPTS,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps={},format=yuv420p[{output}]",
                    canvas.width,
                    canvas.height,
                    canvas.width,
                    canvas.height,
                    fmt(canvas.fps)
                ));
            };
            normalize(&label, &first_local);
            normalize(&next_label, &second_local);
            let offset = clip.duration - transition_duration;
            parts.push(format!(
                "[{first_local}][{second_local}]xfade=transition={transition_type}:duration={}:offset={}[{pair}]",
                fmt(transition_duration),
                fmt(offset)
            ));
            let placed = labels.next("v2xfadeplaced");
            parts.push(format!(
                "[{pair}]setpts=PTS+{}/TB[{placed}]",
                fmt(clip.start)
            ));
            result.push((
                placed,
                NormalizedClip {
                    duration: clip.duration + next.duration - transition_duration,
                    ..clip
                },
            ));
            i += 2;
            continue;
        }

        result.push((label, clip));
        i += 1;
    }
    Ok(result)
}

fn audio_clip(
    clip: &NormalizedClip,
    input_index: usize,
    total_duration: f64,
    labels: &mut Labels,
    parts: &mut Vec<String>,
) -> String {
    let label = labels.next("v2audio");
    let mut filters = vec![
        format!(
            "atrim=start={}:end={}",
            fmt(clip.source_in),
            fmt(clip.source_out)
        ),
        "asetpts=PTS-STARTPTS".to_string(),
        "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo".to_string(),
    ];
    if clip.speed != 1.0 {
        filters.extend(atempo_filters(clip.speed));
    }
    if clip.volume != 1.0 {
        filters.push(format!("volume={}", fmt(clip.volume)));
    }
    filters.push("apad".to_string());
    filters.push(format!("atrim=duration={}", fmt(clip.duration)));
    filters.push("asetpts=PTS-STARTPTS".to_string());
    let delay = (clip.start * 1000.0).max(0.0).round() as i64;
    filters.push(format!("adelay={delay}:all=1"));
    filters.push("apad".to_string());
    filters.push(format!("atrim=duration={}", fmt(total_duration)));
    filters.push("asetpts=PTS-STARTPTS".to_string());
    parts.push(format!(
        "[{input_index}:a]{filters}[{label}]",
        filters = filters.join(",")
    ));
    label
}

fn silent_audio_clip(
    clip: &NormalizedClip,
    total_duration: f64,
    labels: &mut Labels,
    parts: &mut Vec<String>,
) -> String {
    let label = labels.next("v2silence");
    let delay = (clip.start * 1000.0).max(0.0).round() as i64;
    parts.push(format!(
        "aevalsrc=0:c=stereo:sample_rate=48000:d={},adelay={delay}:all=1,apad,atrim=duration={},asetpts=PTS-STARTPTS[{label}]",
        fmt(clip.duration),
        fmt(total_duration)
    ));
    label
}

fn build_audio(
    tracks: &[NormalizedTrack],
    source_by_id: &HashMap<String, Map<String, Value>>,
    input_index: &HashMap<String, usize>,
    total_duration: f64,
    labels: &mut Labels,
    parts: &mut Vec<String>,
    include_embedded_audio: bool,
) -> Result<String, CompileError> {
    let mut inputs = Vec::new();
    for track in tracks {
        if !track.enabled || track.muted {
            continue;
        }
        if track.track_type != "audio" && track.track_type != "video" {
            continue;
        }
        if track.track_type == "video" && !include_embedded_audio {
            continue;
        }
        for clip in &track.clips {
            if record(&clip.raw)
                .and_then(|r| r.get("muted"))
                .and_then(|v| v.as_bool())
                == Some(true)
            {
                continue;
            }
            let Some(source_id) = clip.source_id.as_deref() else {
                continue;
            };
            let source = source_by_id.get(source_id);
            let index = input_index.get(source_id).copied();
            if source.is_none() || index.is_none() {
                return Err(msg(format!(
                    "v2 audio clip {} has no usable source",
                    clip.id
                )));
            }
            let source = source.unwrap();
            let has_audio = source.get("hasAudio").and_then(|v| v.as_bool()) == Some(true);
            let label = if !has_audio {
                silent_audio_clip(clip, total_duration, labels, parts)
            } else {
                audio_clip(clip, index.unwrap_or(0), total_duration, labels, parts)
            };
            inputs.push(format!("[{label}]"));
        }
    }

    let silence = labels.next("v2audioBed");
    parts.push(format!(
        "aevalsrc=0:c=stereo:sample_rate=48000:d={}[{silence}]",
        fmt(total_duration)
    ));
    inputs.insert(0, format!("[{silence}]"));
    let mixed = labels.next("v2mix");
    parts.push(format!(
        "{inputs}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0,aresample=48000,atrim=duration={},asetpts=PTS-STARTPTS[{mixed}]",
        inputs.len(),
        fmt(total_duration),
        inputs = inputs.join("")
    ));
    Ok(mixed)
}

fn has_explicit_audio_track(tracks: &[NormalizedTrack]) -> bool {
    tracks.iter().any(|track| {
        track.enabled && !track.muted && track.track_type == "audio" && !track.clips.is_empty()
    })
}

fn output_duration(
    project: &Map<String, Value>,
    tracks: &[NormalizedTrack],
) -> Result<f64, CompileError> {
    let mut end = project
        .get("duration")
        .and_then(finite)
        .filter(|v| *v >= 0.0)
        .unwrap_or(0.0);
    for track in tracks {
        if !track.enabled {
            continue;
        }
        for clip in &track.clips {
            end = end.max(clip.start + clip.duration);
        }
        for gap in &track.gaps {
            end = end.max(gap.start + gap.duration);
        }
    }
    if let Some(gaps) = project.get("gaps") {
        let arr = gaps
            .as_array()
            .ok_or_else(|| msg("v2 project gaps must be an array"))?;
        for (index, raw_gap) in arr.iter().enumerate() {
            let gap = normalize_gap(raw_gap, &format!("v2 project gap {index}"))?;
            end = end.max(gap.start + gap.duration);
        }
    }
    if end <= 0.0 {
        return Err(msg(
            "Cannot compile an empty v2 timeline: no positive duration",
        ));
    }
    Ok(end)
}

fn reject_unsupported_project_features(project: &Map<String, Value>) -> Result<(), CompileError> {
    ensure_no_unsupported_effects(project.get("effects"), "project")?;
    let Some(transitions) = project.get("transitions") else {
        return Ok(());
    };
    if !transitions.is_array() {
        return Err(msg("v2 project transitions must be an array"));
    }
    if transitions.as_array().map(|a| a.len()).unwrap_or(0) > 0 {
        return Err(unsupported(
            "project transition",
            "project-level transitions are not implemented; attach transitions to adjacent clips or a track",
        ));
    }
    Ok(())
}

fn overlay_kind(value: &Value) -> Option<String> {
    let obj = record(value)?;
    let kind = field(obj, "type", "kind")?;
    kind.as_str().map(|s| s.to_lowercase())
}

fn overlay_window(obj: &Map<String, Value>) -> Result<(Option<f64>, Option<f64>), CompileError> {
    if obj.get("start").is_none() && obj.get("duration").is_none() {
        return Ok((None, None));
    }
    let start = obj
        .get("start")
        .map(|v| {
            if v.is_null() {
                return Ok(0.0);
            }
            nonnegative(v).ok_or_else(|| {
                msg("v2 overlay start must be nonnegative and duration must be positive")
            })
        })
        .transpose()?
        .unwrap_or(0.0);
    let duration = obj
        .get("duration")
        .and_then(positive)
        .ok_or_else(|| msg("v2 overlay start must be nonnegative and duration must be positive"))?;
    if !nonnegative_num(start) {
        return Err(msg(
            "v2 overlay start must be nonnegative and duration must be positive",
        ));
    }
    Ok((Some(start), Some(start + duration)))
}

fn caption_cue(value: &Value, overlay_id: &str) -> Result<CaptionCue, CompileError> {
    let obj = record(value).ok_or_else(|| {
        msg(format!(
            "v2 caption overlay {overlay_id} has an invalid cue"
        ))
    })?;
    let text = obj.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
        msg(format!(
            "v2 caption overlay {overlay_id} has an invalid cue"
        ))
    })?;
    let start = finite(obj.get("start").unwrap_or(&Value::Null)).ok_or_else(|| {
        msg(format!(
            "v2 caption overlay {overlay_id} has an invalid cue"
        ))
    })?;
    let end = finite(obj.get("end").unwrap_or(&Value::Null)).ok_or_else(|| {
        msg(format!(
            "v2 caption overlay {overlay_id} has an invalid cue"
        ))
    })?;
    if start < 0.0 || end <= start {
        return Err(msg(format!(
            "v2 caption overlay {overlay_id} cue must have 0 <= start < end"
        )));
    }
    Ok(CaptionCue {
        start,
        end,
        text: text.to_string(),
    })
}

fn overlay_filters(
    project: &Map<String, Value>,
    labels: &mut Labels,
    parts: &mut Vec<String>,
    input: &str,
) -> Result<String, CompileError> {
    let Some(overlays) = project.get("overlays").and_then(|v| v.as_array()) else {
        return Ok(input.to_string());
    };
    let mut output = input.to_string();
    for value in overlays {
        let obj = record(value).ok_or_else(|| unsupported("overlay", "missing type/kind"))?;
        let kind =
            overlay_kind(value).ok_or_else(|| unsupported("overlay", "missing type/kind"))?;
        let (window_start, window_end) = overlay_window(obj)?;

        if kind == "subtitles" {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| msg("v2 subtitles overlay requires a non-empty path"))?;
            let mut filter = subtitles_filter(path, obj.get("style"));
            if let (Some(start), Some(end)) = (window_start, window_end) {
                filter.push_str(&format!(":enable='between(t,{},{})'", fmt(start), fmt(end)));
            }
            let next = labels.next("v2subs");
            parts.push(format!("[{output}]{filter}[{next}]"));
            output = next;
            continue;
        }

        if kind == "captions" {
            let cues = obj
                .get("cues")
                .and_then(|v| v.as_array())
                .ok_or_else(|| msg("v2 captions overlay requires cues"))?;
            let style = obj.get("style");
            let overlay_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("captions");
            for raw_cue in cues {
                let cue = caption_cue(raw_cue, overlay_id)?;
                let start = cue.start.max(window_start.unwrap_or(cue.start));
                let end = cue.end.min(window_end.unwrap_or(cue.end));
                if end <= start {
                    continue;
                }
                let next = labels.next("v2caption");
                let filter = captions_filter(
                    &CaptionCue {
                        start,
                        end,
                        text: cue.text,
                    },
                    style,
                );
                parts.push(format!("[{output}]{filter}[{next}]"));
                output = next;
            }
            continue;
        }

        return Err(unsupported(
            "overlay",
            format!("{kind}; only subtitles and captions are supported"),
        ));
    }
    Ok(output)
}

fn sources_map(
    project: &Map<String, Value>,
) -> Result<HashMap<String, Map<String, Value>>, CompileError> {
    let mut source_by_id = HashMap::new();
    let sources = project
        .get("sources")
        .and_then(|v| v.as_array())
        .ok_or_else(|| msg("v2 project requires sources array"))?;
    for source_value in sources {
        let source = record(source_value).ok_or_else(|| msg("v2 source must be an object"))?;
        let id = source
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| msg("v2 source requires id"))?;
        if source_by_id.contains_key(id) {
            return Err(msg(format!("Duplicate v2 source id: {id}")));
        }
        source_by_id.insert(id.to_string(), source.clone());
    }
    Ok(source_by_id)
}

fn normalize_tracks(
    project: &Map<String, Value>,
    source_by_id: &HashMap<String, Map<String, Value>>,
) -> Result<Vec<NormalizedTrack>, CompileError> {
    project
        .get("tracks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| msg("v2 project requires tracks array"))?
        .iter()
        .map(|track| normalize_track(track, source_by_id))
        .collect()
}

/// Compile a version 2 project into a timeline-positioned compositing graph.
pub fn compile_filter_graph_v2(
    project: &Value,
    opts: CompileV2Options,
) -> Result<CompiledV2FilterGraph, CompileError> {
    let project = record(project).ok_or_else(|| msg("project must be an object"))?;
    if project.get("version").and_then(|v| v.as_i64()) != Some(2) {
        return Err(msg("compileFilterGraphV2 requires a version 2 project"));
    }
    if !project.get("fps").and_then(positive).is_some() {
        return Err(msg("v2 project fps must be finite and positive"));
    }
    let fps = project.get("fps").and_then(positive).unwrap_or(0.0);
    reject_unsupported_project_features(project)?;

    let source_by_id = sources_map(project)?;
    let tracks = normalize_tracks(project, &source_by_id)?;
    let duration = output_duration(project, &tracks)?;

    let mut used_source_ids: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for track in &tracks {
        if !track.enabled {
            continue;
        }
        for clip in &track.clips {
            if let Some(ref sid) = clip.source_id {
                if seen.insert(sid.clone()) {
                    used_source_ids.push(sid.clone());
                }
            }
        }
    }

    let used_sources: Vec<Map<String, Value>> = used_source_ids
        .iter()
        .map(|id| {
            source_by_id
                .get(id)
                .cloned()
                .ok_or_else(|| msg(format!("Unknown v2 source id: {id}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let (width, height) = choose_canvas(project, &used_sources);
    let canvas = Canvas {
        width,
        height,
        fps,
        duration,
    };

    let mut labels = Labels::new();
    let mut parts: Vec<String> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    let mut input_index = HashMap::new();
    for source in &used_sources {
        let id = source
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let path = source
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| msg(format!("v2 source {id} path must be a non-empty string")))?;
        input_index.insert(id.to_string(), inputs.len() / 2);
        inputs.push("-i".to_string());
        inputs.push(path.to_string());
    }

    let base = labels.next("v2base");
    parts.push(format!(
        "color=c=black:s={}x{}:r={}:d={},format=rgba[{base}]",
        canvas.width,
        canvas.height,
        fmt(canvas.fps),
        fmt(canvas.duration)
    ));
    let mut final_video = base;
    for track in &tracks {
        if !track.enabled {
            continue;
        }
        if track.track_type != "video" && track.track_type != "title" {
            continue;
        }
        let items = build_video_track_items(
            track,
            &source_by_id,
            &input_index,
            &canvas,
            &mut labels,
            &mut parts,
        )?;
        for (label, clip) in items {
            let out = labels.next("v2overlay");
            let x = expression_with_timeline_time(&clip.visual.x_expression, clip.start);
            let y = expression_with_timeline_time(&clip.visual.y_expression, clip.start);
            let x_expression = format!("((main_w-overlay_w)/2+main_w*({x}))");
            let y_expression = format!("((main_h-overlay_h)/2+main_h*({y}))");
            parts.push(format!(
                "[{final_video}][{label}]overlay=x='{x_expression}':y='{y_expression}':eof_action=pass:shortest=0:format=auto[{out}]"
            ));
            final_video = out;
        }
    }

    let final_video_format = labels.next("v2video");
    if !opts.video_post_filters.is_empty() {
        parts.push(format!(
            "[{final_video}]{}format=yuv420p[{final_video_format}]",
            opts.video_post_filters
                .iter()
                .map(|f| format!("{f},"))
                .collect::<String>()
        ));
    } else {
        parts.push(format!(
            "[{final_video}]format=yuv420p[{final_video_format}]"
        ));
    }

    let final_video_output =
        overlay_filters(project, &mut labels, &mut parts, &final_video_format)?;
    let explicit_audio = has_explicit_audio_track(&tracks);
    let include_embedded_audio = opts.include_embedded_audio.unwrap_or(!explicit_audio);
    let final_audio = build_audio(
        &tracks,
        &source_by_id,
        &input_index,
        duration,
        &mut labels,
        &mut parts,
        include_embedded_audio,
    )?;

    let filter_complex = parts.join(";\n");
    let argv = {
        let mut argv = vec!["ffmpeg".to_string(), "-y".to_string()];
        argv.extend(inputs);
        argv.push("-filter_complex".to_string());
        argv.push(filter_complex.replace('\n', ""));
        argv.push("-map".to_string());
        argv.push(format!("[{final_video_output}]"));
        argv.push("-map".to_string());
        argv.push(format!("[{final_audio}]"));
        argv.push("-r".to_string());
        argv.push(fps.to_string());
        argv
    };
    let command = format!("ffmpeg {}", argv[1..].join(" "));

    Ok(CompiledV2FilterGraph {
        argv,
        command,
        filter_complex,
        duration,
        width,
        height,
    })
}

/// Pure duration helper for v2 projects.
pub fn timeline_duration_v2(project: &Value) -> Result<f64, CompileError> {
    let project = record(project).ok_or_else(|| msg("project must be an object"))?;
    reject_unsupported_project_features(project)?;
    let source_by_id = sources_map(project)?;
    let tracks = normalize_tracks(project, &source_by_id)?;
    output_duration(project, &tracks)
}
