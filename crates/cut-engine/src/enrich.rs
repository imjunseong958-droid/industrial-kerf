//! Fill missing source metadata before compile/render (matches TS timelineForRender).

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

use crate::error::Result;
use crate::probe::info;

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

// Index references once: scanning every clip for each source made preparation
// quadratic even when all source metadata was already present.
fn used_sources(project: &Value, v2: bool) -> HashMap<&str, bool> {
    let mut used = HashMap::new();
    for clip in project
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| track.get("clips").and_then(Value::as_array))
        .flatten()
    {
        let id = if v2 {
            source_id_of_clip_v2(clip)
        } else {
            clip.get("src").and_then(Value::as_str)
        };
        if let Some(id) = id {
            let needs_duration = v2
                && ["in", "out", "sourceIn", "sourceOut", "duration"]
                    .iter()
                    .all(|field| clip.get(*field).is_none());
            *used.entry(id).or_insert(false) |= needs_duration;
        }
    }
    used
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
    let used = used_sources(timeline, false);
    let pending: HashSet<&str> = sources
        .iter()
        .filter_map(|source| {
            let id = source.get("id")?.as_str()?;
            (used.contains_key(id) && source_needs_probe_v1(source)).then_some(id)
        })
        .collect();
    if pending.is_empty() {
        return Ok(timeline.clone());
    }
    let mut enriched = timeline.clone();
    let Some(sources_mut) = enriched.get_mut("sources").and_then(Value::as_array_mut) else {
        return Ok(enriched);
    };
    let mut probes = HashMap::new();
    for source in sources_mut.iter_mut() {
        let Some(id) = source.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !pending.contains(id) {
            continue;
        }
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !probes.contains_key(path) {
            probes.insert(path.to_owned(), info(path)?);
        }
        let probed = &probes[path];
        if let Some(map) = source.as_object_mut() {
            fill_source_from_probe(map, probed);
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
    let used = used_sources(project, true);
    let pending: HashSet<&str> = sources
        .iter()
        .filter_map(|source| {
            let id = source.get("id")?.as_str()?;
            let needs_duration = *used.get(id)?;
            (source_needs_probe_v1(source) || (needs_duration && source.get("duration").is_none()))
                .then_some(id)
        })
        .collect();
    if pending.is_empty() {
        return Ok(project.clone());
    }
    let mut enriched = project.clone();
    let Some(sources_mut) = enriched.get_mut("sources").and_then(Value::as_array_mut) else {
        return Ok(enriched);
    };
    let mut probes = HashMap::new();
    for source in sources_mut.iter_mut() {
        let Some(id) = source.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !pending.contains(id) {
            continue;
        }
        let Some(path) = source.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !probes.contains_key(path) {
            probes.insert(path.to_owned(), info(path)?);
        }
        let probed = &probes[path];
        if let Some(map) = source.as_object_mut() {
            fill_source_from_probe(map, probed);
        }
    }
    Ok(enriched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duration_requirement_accumulates_across_references_and_aliases() {
        let project = json!({"tracks":[{"clips":[
            {"sourceId":"a","in":0,"out":1}, {"src":"b","duration":2},
            {"sourceId":"a"}, {"sourceId":"b","sourceIn":1,"sourceOut":2}
        ]}]});
        let used = used_sources(&project, true);
        assert_eq!(used.get("a"), Some(&true));
        assert_eq!(used.get("b"), Some(&false));
        assert_eq!(used.len(), 2);
    }

    #[test]
    fn complete_and_unused_sources_do_not_probe_missing_paths() {
        let project = json!({"version":2,"sources":[
            {"id":"used","path":"/nonexistent/used.mov","width":1920,"height":1080,"hasAudio":false},
            {"id":"unused","path":"/nonexistent/unused.mov"}
        ],"tracks":[{"clips":[{"sourceId":"used","in":0,"out":2}]}]});
        assert_eq!(enrich_project_v2(&project).unwrap(), project);
    }
}
