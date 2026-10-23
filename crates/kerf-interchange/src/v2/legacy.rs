//! The v1 and v2 `project.cut.json` formats, read only.
//!
//! Fields keep the spellings older writers used (`src`, `sourceIn`, `kind`,
//! ...). A v1 timeline placed clips back to back; it is read as the v2
//! project that implies.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type Params = BTreeMap<String, Value>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub name: Option<String>,
    pub fps: f64,
    pub width: Option<f64>,
    pub height: Option<f64>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    #[serde(default)]
    pub transitions: Vec<Value>,
    #[serde(default)]
    pub effects: Vec<Value>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub path: String,
    pub subs: Option<String>,
    pub duration: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub has_audio: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    Video,
    Audio,
    Title,
    Adjustment,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    #[serde(rename = "type", alias = "kind")]
    pub kind: TrackType,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub muted: Option<bool>,
    pub locked: Option<bool>,
    pub clips: Vec<Clip>,
    pub transitions: Option<Vec<Transition>>,
    pub effects: Option<Vec<Value>>,
    pub markers: Option<Vec<Marker>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    #[serde(alias = "src", alias = "source", alias = "assetId")]
    pub source_id: Option<String>,
    #[serde(rename = "in", alias = "sourceIn", alias = "sourceStart")]
    pub source_in: Option<f64>,
    #[serde(rename = "out", alias = "sourceOut", alias = "sourceEnd")]
    pub source_out: Option<f64>,
    #[serde(alias = "start")]
    pub timeline_start: f64,
    pub duration: Option<f64>,
    pub speed: Option<f64>,
    pub volume: Option<f64>,
    pub opacity: Option<f64>,
    pub crop: Option<Crop>,
    pub transform: Option<Transform>,
    pub blend: Option<String>,
    pub blend_mode: Option<String>,
    pub effects: Option<Vec<Effect>>,
    pub keyframes: Option<Vec<Keyframe>>,
    pub transitions: Option<Vec<Transition>>,
    pub gaps: Option<Vec<Value>>,
    pub text: Option<String>,
    pub title: Option<Value>,
}

impl Clip {
    pub fn timeline_duration(&self) -> Option<f64> {
        self.duration
            .or_else(|| match (self.source_in, self.source_out) {
                (Some(input), Some(output)) => Some((output - input) / self.speed.unwrap_or(1.0)),
                _ => None,
            })
    }
}

#[derive(Debug, Deserialize)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub scale: Option<f64>,
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
    pub rotation: Option<f64>,
    pub rotation_x: Option<f64>,
    pub rotation_y: Option<f64>,
    pub anchor_x: Option<f64>,
    pub anchor_y: Option<f64>,
    pub position: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct Keyframe {
    pub property: String,
    pub time: f64,
    pub value: Value,
    pub easing: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Transition {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub alignment: Option<Alignment>,
    pub params: Option<Params>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    In,
    Out,
    Center,
}

/// A clip effect. `color` effects carry the basic grade.
#[derive(Debug, Deserialize)]
pub struct Effect {
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: Option<bool>,
    pub params: Option<Params>,
}

#[derive(Debug, Deserialize)]
pub struct Marker {
    pub id: String,
    #[serde(alias = "start")]
    pub time: f64,
    pub label: Option<String>,
    pub color: Option<String>,
    pub duration: Option<f64>,
}

/// A caption track (with `cues`), a subtitle file (with `path`), or
/// something else.
#[derive(Debug, Deserialize)]
pub struct Overlay {
    pub id: Option<String>,
    pub path: Option<String>,
    pub cues: Option<Vec<Cue>>,
    pub style: Option<Value>,
    pub start: Option<f64>,
    pub duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Cue {
    pub id: Option<String>,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Read a v1 or v2 project file.
pub fn parse(text: &str) -> Result<Project, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match value.get("version").and_then(Value::as_u64) {
        Some(1) => serde_json::from_value::<V1>(value)
            .map(V1::into_v2)
            .map_err(|e| e.to_string()),
        Some(2) => serde_json::from_value(value).map_err(|e| e.to_string()),
        _ => Err("version must be 1 or 2".into()),
    }
}

#[derive(Deserialize)]
struct V1 {
    fps: f64,
    sources: Vec<Source>,
    tracks: Vec<V1Track>,
    #[serde(default)]
    overlays: Vec<Overlay>,
}

#[derive(Deserialize)]
struct V1Track {
    id: String,
    clips: Vec<V1Clip>,
}

#[derive(Deserialize)]
struct V1Clip {
    id: String,
    src: String,
    #[serde(rename = "in")]
    source_in: f64,
    #[serde(rename = "out")]
    source_out: f64,
    speed: Option<f64>,
    volume: Option<f64>,
    crop: Option<Crop>,
}

impl V1 {
    fn into_v2(self) -> Project {
        let tracks = self
            .tracks
            .into_iter()
            .map(|track| {
                let mut start = 0.0;
                let clips = track
                    .clips
                    .into_iter()
                    .map(|clip| {
                        let timeline_start = start;
                        start += (clip.source_out - clip.source_in) / clip.speed.unwrap_or(1.0);
                        Clip {
                            id: clip.id,
                            source_id: Some(clip.src),
                            source_in: Some(clip.source_in),
                            source_out: Some(clip.source_out),
                            timeline_start,
                            duration: None,
                            speed: clip.speed,
                            volume: clip.volume,
                            opacity: None,
                            crop: clip.crop,
                            transform: None,
                            blend: None,
                            blend_mode: None,
                            effects: None,
                            keyframes: None,
                            transitions: None,
                            gaps: None,
                            text: None,
                            title: None,
                        }
                    })
                    .collect();
                Track {
                    kind: v1_track_type(&track.id),
                    id: track.id,
                    name: None,
                    enabled: None,
                    muted: None,
                    locked: None,
                    clips,
                    transitions: None,
                    effects: None,
                    markers: None,
                }
            })
            .collect();
        Project {
            name: None,
            fps: self.fps,
            width: None,
            height: None,
            sources: self.sources,
            tracks,
            overlays: self.overlays,
            transitions: Vec::new(),
            effects: Vec::new(),
            markers: Vec::new(),
            metadata: None,
        }
    }
}

/// v1 tracks had no type; their ids named it.
fn v1_track_type(id: &str) -> TrackType {
    let id = id.trim().to_ascii_lowercase();
    let prefixed = |prefix: &str| {
        id == prefix
            || id.strip_prefix(prefix).is_some_and(|rest| {
                rest.starts_with(['-', '_']) || rest.chars().all(|c| c.is_ascii_digit())
            })
    };
    if prefixed("audio") || prefixed("a") {
        TrackType::Audio
    } else if prefixed("title") || prefixed("titles") {
        TrackType::Title
    } else if prefixed("adj") || prefixed("adjust") || prefixed("adjustment") {
        TrackType::Adjustment
    } else {
        TrackType::Video
    }
}
