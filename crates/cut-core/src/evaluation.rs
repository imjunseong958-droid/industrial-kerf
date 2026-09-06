//! Small, dependency-free evaluation plan shared by preview and export.
//!
//! The authored project remains owned by `cutd`. This plan is an immutable
//! interpretation of timing and layer order, so native playback and the
//! offline compiler can agree on the basic frame/source mapping without
//! introducing another state owner.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One source-backed or title clip in evaluation coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationClip {
    pub id: String,
    pub track_id: String,
    pub track_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub timeline_start: f64,
    pub duration: f64,
    pub source_start: f64,
    pub source_end: f64,
    pub speed: f64,
    pub enabled: bool,
    pub muted: bool,
}

impl EvaluationClip {
    /// End of this clip in project time.
    pub fn timeline_end(&self) -> f64 {
        self.timeline_start + self.duration
    }

    /// Whether the clip contributes at a project timestamp.
    pub fn contains(&self, time: f64) -> bool {
        time.is_finite() && time >= self.timeline_start && time < self.timeline_end()
    }

    /// Map project time to source time using the clip speed.
    pub fn source_time_at(&self, time: f64) -> f64 {
        (self.source_start + (time - self.timeline_start).max(0.0) * self.speed)
            .min(self.source_end)
    }
}

/// Immutable, serializable interpretation of the timing/layer portion of a
/// v2 project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationPlan {
    pub fps: f64,
    pub duration: f64,
    pub clips: Vec<EvaluationClip>,
}

impl EvaluationPlan {
    /// Build an evaluation plan from a canonical v2 JSON projection.
    pub fn from_project_json(project: &Value) -> Result<Self, String> {
        let object = project
            .as_object()
            .ok_or_else(|| "project must be an object".to_owned())?;
        if object.get("version").and_then(Value::as_u64) != Some(2) {
            return Err("evaluation requires a version 2 project".into());
        }
        let fps = object
            .get("fps")
            .and_then(Value::as_f64)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .ok_or_else(|| "project fps must be finite and positive".to_owned())?;
        let sources = object
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let source_durations = sources
            .iter()
            .filter_map(|source| {
                Some((
                    source.get("id")?.as_str()?.to_owned(),
                    source.get("duration")?.as_f64()?.max(0.0),
                ))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut clips = Vec::new();
        let mut duration = object
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .unwrap_or(0.0);
        for track in object
            .get("tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let track_id = track
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "track id is required".to_owned())?;
            let track_type = track
                .get("type")
                .or_else(|| track.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("video");
            let enabled = track
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let muted = track.get("muted").and_then(Value::as_bool).unwrap_or(false);
            for clip in track
                .get("clips")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let id = clip
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "clip id is required".to_owned())?;
                let timeline_start = clip
                    .get("timelineStart")
                    .or_else(|| clip.get("start"))
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or_else(|| format!("clip {id} timelineStart is invalid"))?;
                let speed = clip
                    .get("speed")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .unwrap_or(1.0);
                let source_id = clip
                    .get("sourceId")
                    .or_else(|| clip.get("src"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let source_start = clip
                    .get("in")
                    .or_else(|| clip.get("sourceIn"))
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .unwrap_or(0.0);
                let source_end = clip
                    .get("out")
                    .or_else(|| clip.get("sourceOut"))
                    .and_then(Value::as_f64)
                    .or_else(|| {
                        source_id
                            .as_deref()
                            .and_then(|id| source_durations.get(id).copied())
                    });
                let duration_from_source = source_end.map(|end| (end - source_start) / speed);
                let clip_duration = clip
                    .get("duration")
                    .and_then(Value::as_f64)
                    .or(duration_from_source)
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| format!("clip {id} has no positive duration"))?;
                let source_end = source_end.unwrap_or(source_start + clip_duration * speed);
                if !source_end.is_finite() || source_end <= source_start {
                    return Err(format!("clip {id} source range is invalid"));
                }
                duration = duration.max(timeline_start + clip_duration);
                clips.push(EvaluationClip {
                    id: id.to_owned(),
                    track_id: track_id.to_owned(),
                    track_type: track_type.to_owned(),
                    source_id,
                    timeline_start,
                    duration: clip_duration,
                    source_start,
                    source_end,
                    speed,
                    enabled,
                    muted,
                });
            }
        }
        Ok(Self {
            fps,
            duration,
            clips,
        })
    }

    /// Return visible clips at a project timestamp in layer order.
    pub fn clips_at(&self, time: f64) -> Vec<&EvaluationClip> {
        self.clips
            .iter()
            .filter(|clip| clip.enabled && clip.contains(time))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_maps_timeline_to_source_with_speed() {
        let plan = EvaluationPlan::from_project_json(&json!({
            "version": 2,
            "fps": 30,
            "sources": [{"id":"source-1","duration":10}],
            "tracks": [{"id":"video-1","type":"video","clips":[{
                "id":"clip-1","sourceId":"source-1","in":2,"out":6,
                "timelineStart":4,"speed":2
            }]}]
        }))
        .expect("evaluation plan");
        let clip = plan.clips_at(5.0)[0];
        assert_eq!(clip.source_time_at(5.0), 4.0);
        assert_eq!(plan.duration, 6.0);
    }
}
