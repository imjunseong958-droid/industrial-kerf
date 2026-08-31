//! Version 1 filtergraph compiler for sequential timelines.

use crate::overlay::{captions_filter, fmt, subtitles_filter, CaptionCue};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledV1FilterGraph {
    pub argv: Vec<String>,
    pub command: String,
    pub filter_complex: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileV1Error {
    #[error("{0}")]
    Message(String),
}

fn msg(message: impl Into<String>) -> CompileV1Error {
    CompileV1Error::Message(message.into())
}

struct LabelAllocator {
    next_id: u32,
}

impl LabelAllocator {
    fn new() -> Self {
        Self { next_id: 0 }
    }

    fn next(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_id);
        self.next_id += 1;
        label
    }
}

#[derive(Debug, Clone, Copy)]
struct Canvas {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct NormalizedCrop {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct ClipChain {
    v: String,
    a: String,
}

fn finite_positive(value: f64, name: &str) -> Result<f64, CompileV1Error> {
    if !value.is_finite() || !(value > 0.0) {
        return Err(msg(format!(
            "{name} must be a finite positive number, got {value}"
        )));
    }
    Ok(value)
}

fn clip_speed(clip: &Value) -> Result<f64, CompileV1Error> {
    match clip.get("speed") {
        None => Ok(1.0),
        Some(value) => {
            let speed = value
                .as_f64()
                .ok_or_else(|| msg("Clip speed must be a finite positive number, got invalid"))?;
            finite_positive(speed, "Clip speed")
        }
    }
}

fn clip_duration(clip: &Value) -> Result<f64, CompileV1Error> {
    let clip_id = clip.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let clip_in = clip.get("in").and_then(|v| v.as_f64()).ok_or_else(|| {
        msg(format!(
            "Clip {clip_id} duration must be a finite positive number"
        ))
    })?;
    let clip_out = clip.get("out").and_then(|v| v.as_f64()).ok_or_else(|| {
        msg(format!(
            "Clip {clip_id} duration must be a finite positive number"
        ))
    })?;
    let duration = clip_out - clip_in;
    finite_positive(duration, &format!("Clip {clip_id} duration"))
}

/// FFmpeg's atempo filter accepts [0.5, 100]. Products of those factors can represent any
/// finite positive speed, so split both very slow and very fast rates into a chain.
pub fn atempo_filters(speed: f64) -> Result<Vec<String>, CompileV1Error> {
    let speed = finite_positive(speed, "Clip speed")?;
    Ok(crate::overlay::atempo_filters(speed))
}

fn finite_metadata(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|v| v.as_f64())
        .filter(|n| n.is_finite() && *n > 0.0)
}

fn even_dimension(value: f64) -> u32 {
    let floored = (value / 2.0).floor() * 2.0;
    floored.max(2.0) as u32
}

fn choose_canvas(sources: &[&Value]) -> Canvas {
    let dimensions: Vec<(Option<f64>, Option<f64>)> = sources
        .iter()
        .map(|source| {
            (
                finite_metadata(source.get("width")),
                finite_metadata(source.get("height")),
            )
        })
        .collect();

    let mut complete: Vec<(f64, f64)> = dimensions
        .iter()
        .filter_map(|(width, height)| {
            if let (Some(width), Some(height)) = (width, height) {
                Some((*width, *height))
            } else {
                None
            }
        })
        .collect();
    complete.sort_by(|a, b| {
        let area_a = a.0 * a.1;
        let area_b = b.0 * b.1;
        area_b
            .partial_cmp(&area_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some((width, height)) = complete.first().copied() {
        return Canvas {
            width: even_dimension(width),
            height: even_dimension(height),
        };
    }

    let partial = dimensions
        .iter()
        .find(|(width, height)| width.is_some() || height.is_some());
    let Some((partial_width, partial_height)) = partial else {
        return Canvas {
            width: 1920,
            height: 1080,
        };
    };

    let ratio = 16.0 / 9.0;
    let width = partial_width.unwrap_or_else(|| partial_height.unwrap() * ratio);
    let height = partial_height.unwrap_or_else(|| partial_width.unwrap() / ratio);
    Canvas {
        width: even_dimension(width),
        height: even_dimension(height),
    }
}

fn normalize_crop(raw: &Value, clip: &Value) -> Result<Option<NormalizedCrop>, CompileV1Error> {
    if raw.is_null() {
        return Ok(None);
    }
    let clip_id = clip.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let Some(obj) = raw.as_object() else {
        return Err(msg(format!("Clip {clip_id} crop must be an object")));
    };
    let x = obj.get("x").and_then(|v| v.as_f64());
    let y = obj.get("y").and_then(|v| v.as_f64());
    let width = obj.get("width").and_then(|v| v.as_f64());
    let height = obj.get("height").and_then(|v| v.as_f64());
    if [x, y, width, height]
        .iter()
        .any(|value| value.is_none_or(|n| !n.is_finite()))
    {
        return Err(msg(format!(
            "Clip {clip_id} crop values must be finite numbers"
        )));
    }
    let x = x.unwrap();
    let y = y.unwrap();
    let width = width.unwrap();
    let height = height.unwrap();
    if !(width > 0.0) || !(height > 0.0) {
        return Err(msg(format!(
            "Clip {clip_id} crop width/height must be positive"
        )));
    }

    let width = (1.0_f64).min(width.max(1e-6));
    let height = (1.0_f64).min(height.max(1e-6));
    let x = (1.0 - width).min(x.max(0.0));
    let y = (1.0 - height).min(y.max(0.0));
    Ok(Some(NormalizedCrop {
        x,
        y,
        width,
        height,
    }))
}

fn crop_filter(crop: &NormalizedCrop) -> String {
    let w = format!("max(2\\,trunc(iw*{})/2)*2)", fmt(crop.width));
    let h = format!("max(2\\,trunc(ih*{})/2)*2)", fmt(crop.height));
    let x = format!("trunc(iw*{})/2)*2", fmt(crop.x));
    let y = format!("trunc(ih*{})/2)*2", fmt(crop.y));
    format!("crop=w='{w}':h='{h}':x='{x}':y='{y}'")
}

fn video_normalize_filters(canvas: Canvas) -> Vec<String> {
    vec![
        format!(
            "scale={}:{}:force_original_aspect_ratio=decrease",
            canvas.width, canvas.height
        ),
        format!(
            "pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black",
            canvas.width, canvas.height
        ),
        "setsar=1".to_string(),
        "format=yuv420p".to_string(),
    ]
}

fn source_has_audio(source: &Value) -> bool {
    !matches!(source.get("hasAudio"), Some(Value::Bool(false)))
}

fn build_clip_chain(
    clip: &Value,
    source: &Value,
    input_idx: usize,
    canvas: Canvas,
    labels: &mut LabelAllocator,
    parts: &mut Vec<String>,
) -> Result<ClipChain, CompileV1Error> {
    let duration = clip_duration(clip)?;
    let speed = clip_speed(clip)?;
    let v_label = labels.next("clipv");
    let a_label = labels.next("clipa");

    let mut v_filters = vec![
        format!(
            "trim=start={}:end={}",
            fmt(clip.get("in").and_then(|v| v.as_f64()).unwrap_or(0.0)),
            fmt(clip.get("out").and_then(|v| v.as_f64()).unwrap_or(0.0))
        ),
        "setpts=PTS-STARTPTS".to_string(),
    ];
    if let Some(crop_raw) = clip.get("crop") {
        if let Some(crop) = normalize_crop(crop_raw, clip)? {
            v_filters.push(crop_filter(&crop));
        }
    }
    v_filters.extend(video_normalize_filters(canvas));
    if (speed - 1.0).abs() > f64::EPSILON {
        v_filters.push(format!("setpts={}*PTS", fmt(1.0 / speed)));
    }
    parts.push(format!(
        "[{input_idx}:v]{filters}[{v_label}]",
        filters = v_filters.join(",")
    ));

    let clip_in = clip.get("in").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let clip_out = clip.get("out").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let mut a_filters = if !source_has_audio(source) {
        vec![
            "anullsrc=r=48000:cl=stereo".to_string(),
            format!("atrim=duration={}", fmt(duration)),
            "asetpts=PTS-STARTPTS".to_string(),
        ]
    } else {
        vec![
            format!("atrim=start={}:end={}", fmt(clip_in), fmt(clip_out)),
            "asetpts=PTS-STARTPTS".to_string(),
        ]
    };
    a_filters
        .push("aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo".to_string());
    if (speed - 1.0).abs() > f64::EPSILON {
        a_filters.extend(atempo_filters(speed)?);
    }
    if let Some(volume) = clip.get("volume").and_then(|v| v.as_f64()) {
        if (volume - 1.0).abs() > f64::EPSILON {
            a_filters.push(format!("volume={}", fmt(volume)));
        }
    }
    a_filters.push("apad".to_string());
    a_filters.push(format!("atrim=duration={}", fmt(duration / speed)));
    a_filters.push("asetpts=PTS-STARTPTS".to_string());
    let audio_prefix = if source_has_audio(source) {
        format!("[{input_idx}:a]")
    } else {
        String::new()
    };
    parts.push(format!(
        "{audio_prefix}{filters}[{a_label}]",
        filters = a_filters.join(",")
    ));

    Ok(ClipChain {
        v: v_label,
        a: a_label,
    })
}

fn build_concat_section(
    chains: &[ClipChain],
    labels: &mut LabelAllocator,
    parts: &mut Vec<String>,
    prefix: &str,
) -> Result<ClipChain, CompileV1Error> {
    if chains.is_empty() {
        return Err(msg("Cannot concatenate an empty stream list"));
    }
    if chains.len() == 1 {
        return Ok(chains[0].clone());
    }
    let vout = labels.next(&format!("{prefix}v"));
    let aout = labels.next(&format!("{prefix}a"));
    let inputs = chains
        .iter()
        .map(|chain| format!("[{}][{}]", chain.v, chain.a))
        .collect::<String>();
    parts.push(format!(
        "{inputs}concat=n={}:v=1:a=1[{vout}][{aout}]",
        chains.len()
    ));
    Ok(ClipChain { v: vout, a: aout })
}

fn is_subtitles_overlay(value: &Value) -> bool {
    value.get("type").and_then(|v| v.as_str()) == Some("subtitles")
        && value.get("path").and_then(|v| v.as_str()).is_some()
}

fn is_captions_overlay(value: &Value) -> bool {
    value.get("type").and_then(|v| v.as_str()) == Some("captions")
        && value.get("cues").and_then(|v| v.as_array()).is_some()
}

fn normalize_cue(cue: &Value, overlay_id: &str) -> Result<Option<CaptionCue>, CompileV1Error> {
    let Some(obj) = cue.as_object() else {
        return Err(msg(format!(
            "Caption overlay {overlay_id} has an invalid cue"
        )));
    };
    let text = obj
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| msg(format!("Caption overlay {overlay_id} has an invalid cue")))?;
    let start = obj
        .get("start")
        .and_then(|v| v.as_f64())
        .filter(|n| n.is_finite())
        .ok_or_else(|| msg(format!("Caption overlay {overlay_id} has an invalid cue")))?;
    let end = obj
        .get("end")
        .and_then(|v| v.as_f64())
        .filter(|n| n.is_finite())
        .ok_or_else(|| msg(format!("Caption overlay {overlay_id} has an invalid cue")))?;
    let start = start.max(0.0);
    let end = end.max(0.0);
    if !(end > start) {
        return Ok(None);
    }
    Ok(Some(CaptionCue {
        start,
        end,
        text: text.to_string(),
    }))
}

/// Total output duration of a v1 timeline in seconds.
pub fn timeline_duration_v1(timeline: &Value) -> Result<f64, CompileV1Error> {
    let mut total = 0.0;
    if let Some(tracks) = timeline.get("tracks").and_then(|v| v.as_array()) {
        for track in tracks {
            if let Some(clips) = track.get("clips").and_then(|v| v.as_array()) {
                for clip in clips {
                    total += clip_duration(clip)? / clip_speed(clip)?;
                }
            }
        }
    }
    Ok(total)
}

/// Compile a v1 timeline into an ffmpeg filtergraph.
pub fn compile_filter_graph_v1(
    timeline: &Value,
    video_post_filters: &[String],
) -> Result<CompiledV1FilterGraph, CompileV1Error> {
    let mut src_by_id: HashMap<String, &Value> = HashMap::new();
    if let Some(sources) = timeline.get("sources").and_then(|v| v.as_array()) {
        for source in sources {
            if let Some(id) = source.get("id").and_then(|v| v.as_str()) {
                src_by_id.insert(id.to_string(), source);
            }
        }
    }

    let mut used_source_ids: Vec<String> = Vec::new();
    let mut used_source_set = std::collections::HashSet::new();
    let mut clip_count = 0usize;
    if let Some(tracks) = timeline.get("tracks").and_then(|v| v.as_array()) {
        for track in tracks {
            if let Some(clips) = track.get("clips").and_then(|v| v.as_array()) {
                for clip in clips {
                    clip_count += 1;
                    if let Some(src) = clip.get("src").and_then(|v| v.as_str()) {
                        if !used_source_set.contains(src) {
                            used_source_set.insert(src.to_string());
                            used_source_ids.push(src.to_string());
                        }
                    }
                }
            }
        }
    }
    if clip_count == 0 {
        return Err(msg("Cannot compile an empty timeline: no clips found"));
    }

    let mut parts: Vec<String> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    let mut input_index_of_source: HashMap<String, usize> = HashMap::new();
    let mut used_sources: Vec<&Value> = Vec::new();
    for source_id in &used_source_ids {
        let source = src_by_id
            .get(source_id)
            .ok_or_else(|| msg(format!("Unknown source id: {source_id}")))?;
        input_index_of_source.insert(source_id.clone(), inputs.len() / 2);
        let path = source
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| msg(format!("Unknown source id: {source_id}")))?;
        inputs.push("-i".to_string());
        inputs.push(path.to_string());
        used_sources.push(*source);
    }

    let canvas = choose_canvas(&used_sources);
    let mut labels = LabelAllocator::new();
    let mut track_outputs: Vec<ClipChain> = Vec::new();

    if let Some(tracks) = timeline.get("tracks").and_then(|v| v.as_array()) {
        for (track_index, track) in tracks.iter().enumerate() {
            let clips = match track.get("clips").and_then(|v| v.as_array()) {
                Some(clips) if !clips.is_empty() => clips,
                _ => continue,
            };
            let mut chains = Vec::with_capacity(clips.len());
            for clip in clips {
                let src = clip
                    .get("src")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| msg("Unknown source id: "))?;
                let source = src_by_id
                    .get(src)
                    .ok_or_else(|| msg(format!("Unknown source id: {src}")))?;
                let input_idx = input_index_of_source
                    .get(src)
                    .copied()
                    .ok_or_else(|| msg(format!("Unknown source id: {src}")))?;
                chains.push(build_clip_chain(
                    clip,
                    source,
                    input_idx,
                    canvas,
                    &mut labels,
                    &mut parts,
                )?);
            }
            track_outputs.push(build_concat_section(
                &chains,
                &mut labels,
                &mut parts,
                &format!("track{track_index}"),
            )?);
        }
    }

    if track_outputs.is_empty() {
        return Err(msg("Cannot compile an empty timeline: no non-empty tracks"));
    }

    let all_tracks = build_concat_section(&track_outputs, &mut labels, &mut parts, "tracks")?;
    let mut final_v = all_tracks.v;
    let final_a = all_tracks.a;

    if !video_post_filters.is_empty() {
        let post_label = labels.next("postv");
        parts.push(format!(
            "[{final_v}]{}[{post_label}]",
            video_post_filters.join(",")
        ));
        final_v = post_label;
    }

    if let Some(overlays) = timeline.get("overlays").and_then(|v| v.as_array()) {
        for overlay in overlays {
            if is_subtitles_overlay(overlay) {
                let out_label = labels.next("subs");
                let path = overlay.get("path").and_then(|v| v.as_str()).unwrap();
                let filter = subtitles_filter(path, overlay.get("style"));
                parts.push(format!("[{final_v}]{filter}[{out_label}]"));
                final_v = out_label;
                continue;
            }
            if is_captions_overlay(overlay) {
                let overlay_id = overlay
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let style = overlay.get("style");
                if let Some(cues) = overlay.get("cues").and_then(|v| v.as_array()) {
                    for raw_cue in cues {
                        if let Some(cue) = normalize_cue(raw_cue, overlay_id)? {
                            let out_label = labels.next("caption");
                            let filter = captions_filter(&cue, style);
                            parts.push(format!("[{final_v}]{filter}[{out_label}]"));
                            final_v = out_label;
                        }
                    }
                }
            }
        }
    }

    let filter_complex = parts.join(";\n");
    let fps = timeline.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
    let fps_args = vec!["-r".to_string(), fps.to_string()];
    let filter_complex_argv = filter_complex.replace('\n', "");
    let mut argv = vec!["ffmpeg".to_string(), "-y".to_string()];
    argv.extend(inputs);
    argv.push("-filter_complex".to_string());
    argv.push(filter_complex_argv);
    argv.push("-map".to_string());
    argv.push(format!("[{final_v}]"));
    argv.push("-map".to_string());
    argv.push(format!("[{final_a}]"));
    argv.extend(fps_args);

    let command = argv.join(" ");
    Ok(CompiledV1FilterGraph {
        argv,
        command,
        filter_complex,
    })
}
