//! Fill missing source metadata before compile/render (matches TS timelineForRender).

use serde_json::{Map, Value};

use crate::error::Result;
use crate::probe::info;

fn source_is_used_v1(timeline: &Value, source_id: &str) -> bool {
    timeline
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get("clips").and_then(Value::as_array))
        .flatten()
        .any(|clip| clip.get("src").and_then(Value::as_str) == Some(source_id))
}

fn source_needs_probe_v1(source: &Value) -> bool {
    source.get("hasAudio").is_none()
        || source.get("width").is_none()
        || source.get("height").is_none()
}

fn source_id_of_clip_v2(clip: &Value) -> Option<&str> {
    clip.get("sourceId")
        .or_else(|| clip.get("src"))
        .and_then(Value::as_str)
}

fn source_is_used_v2(project: &Value, source_id: &str) -> bool {
    project
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get("clips").and_then(Value::as_array))
        .flatten()
        .any(|clip| source_id_of_clip_v2(clip) == Some(source_id))
}

fn source_duration_is_needed_v2(project: &Value, source_id: &str) -> bool {
    project
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get("clips").and_then(Value::as_array))
        .flatten()
        .any(|clip| {
            if source_id_of_clip_v2(clip) != Some(source_id) {
                return false;
            }
            let has_interval = clip.get("in").is_some()
                || clip.get("out").is_some()
                || clip.get("sourceIn").is_some()
                || clip.get("sourceOut").is_some();
            !has_interval && clip.get("duration").is_none()
        })
}

fn source_needs_probe_v2(project: &Value, source: &Value) -> bool {
    let id = source.get("id").and_then(Value::as_str).unwrap_or("");
    source.get("hasAudio").is_none()
        || source.get("width").is_none()
        || source.get("height").is_none()
        || (source.get("duration").is_none() && source_duration_is_needed_v2(project, id))
}

fn fill_source_from_probe(source: &mut Map<String, Value>, probed: &crate::probe::ProbeInfo) {
    if source.get("duration").is_none() {
        if let Some(duration) = probed.duration {
            source.insert("duration".into(), Value::from(duration));
        }
    }
    if source.get("width").is_none() {
        if let Some(width) = probed.width {
            source.insert("width".into(), Value::from(width));
        }
    }
    if source.get("height").is_none() {
        if let Some(height) = probed.height {
            source.insert("height".into(), Value::from(height));
        }
    }
    if source.get("hasAudio").is_none() {
        if let Some(has_audio) = probed.has_audio {
            source.insert("hasAudio".into(), Value::from(has_audio));
        }
    }
}

/// Clone a v1 timeline and probe any used sources missing width/height/hasAudio.
pub fn enrich_timeline_v1(timeline: &Value) -> Result<Value> {
    let sources = match timeline.get("sources").and_then(Value::as_array) {
        Some(sources) => sources,
        None => return Ok(timeline.clone()),
    };
    let pending: Vec<&Value> = sources
        .iter()
        .filter(|source| {
            let Some(id) = source.get("id").and_then(Value::as_str) else {
                return false;
            };
            source_is_used_v1(timeline, id) && source_needs_probe_v1(source)
        })
        .collect();
    if pending.is_empty() {
        return Ok(timeline.clone());
    }
    let mut enriched = timeline.clone();
    let Some(sources_mut) = enriched.get_mut("sources").and_then(Value::as_array_mut) else {
        return Ok(enriched);
    };
    for source in sources_mut.iter_mut() {
        let Some(id) = source.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !source_is_used_v1(timeline, id) || !source_needs_probe_v1(source) {
            continue;
        }
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            continue;
        };
        let probed = info(path)?;
        if let Some(map) = source.as_object_mut() {
            fill_source_from_probe(map, &probed);
        }
    }
    Ok(enriched)
}

/// Clone a v2 project and probe any used sources missing required metadata.
pub fn enrich_project_v2(project: &Value) -> Result<Value> {
    let sources = match project.get("sources").and_then(Value::as_array) {
        Some(sources) => sources,
        None => return Ok(project.clone()),
    };
    let pending: Vec<&Value> = sources
        .iter()
        .filter(|source| {
            let Some(id) = source.get("id").and_then(Value::as_str) else {
                return false;
            };
            source_is_used_v2(project, id) && source_needs_probe_v2(project, source)
        })
        .collect();
    if pending.is_empty() {
        return Ok(project.clone());
    }
    let mut enriched = project.clone();
    let Some(sources_mut) = enriched.get_mut("sources").and_then(Value::as_array_mut) else {
        return Ok(enriched);
    };
    for source in sources_mut.iter_mut() {
        let Some(id) = source.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !source_is_used_v2(project, id) || !source_needs_probe_v2(project, source) {
            continue;
        }
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            continue;
        };
        let probed = info(path)?;
        if let Some(map) = source.as_object_mut() {
            fill_source_from_probe(map, &probed);
        }
    }
    Ok(enriched)
}
