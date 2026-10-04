//! Temporary export of the kernel model to the v2 project shape.
//!
//! Until the renderer, playback, the Qt bridge, and MCP read `cut-model`
//! (ADR 0006 phase 4), `cutd` derives a v2 project from the canonical model
//! for them. The first sequence becomes the v2 timeline. Cuts with a
//! transition become v2 overlaps whose overlapping material is the handles,
//! and subtitle tracks become caption overlays. Anything v2 cannot express is
//! left out of the export (reverse playback plays forward, nested sequences
//! and most generators show nothing) but survives an import that is given
//! the original model as a hint (`import_v2_with`), so
//! `import(export(p), p) == p`.

use cut_domain as v2;
use cut_model::{
    Align, Clip, Content, Effect, Interp, Media, Param, Project, Rate, Sequence, Time, Track,
    TrackKind, Transition, Value,
};
use serde_json::{json, Map, Value as Json};
use std::collections::HashMap;

/// Convert a project to v2. Fails only if the result is not a valid v2
/// project, which would be a bug in this exporter.
pub fn export(project: &Project) -> Result<v2::Project, String> {
    v2::parse_project(&export_value(project)).map_err(|e| format!("v2 export: {e}"))
}

/// The v2 project as wire JSON.
pub fn export_json(project: &Project) -> Result<Json, String> {
    let exported = export(project)?;
    serde_json::to_value(exported).map_err(|e| e.to_string())
}

fn export_value(project: &Project) -> Json {
    let empty = v2::empty_project();
    let sequence = project.sequences.first();
    let (fps, width, height) = match sequence {
        Some(s) => (s.rate.fps(), f64::from(s.width), f64::from(s.height)),
        None => (
            empty.fps,
            empty.width.unwrap_or(1920.0),
            empty.height.unwrap_or(1080.0),
        ),
    };
    let mut tracks = Vec::new();
    let mut overlays = Vec::new();
    let mut markers = Vec::new();
    if let Some(sequence) = sequence {
        for track in &sequence.tracks {
            match track.kind {
                TrackKind::Subtitle => overlays.push(captions(track)),
                TrackKind::Video | TrackKind::Audio => tracks.extend(track_values(sequence, track)),
            }
        }
        markers = sequence.markers.iter().map(marker_value).collect();
    }
    let mut value = json!({
        "version": 2,
        "fps": fps,
        "width": width,
        "height": height,
        "sources": project.media.iter().map(media_value).collect::<Vec<_>>(),
        "tracks": tracks,
        "overlays": overlays,
        "gaps": [],
        "transitions": [],
        "effects": [],
        "markers": markers,
    });
    if !project.name.is_empty() {
        value["name"] = json!(project.name);
    }
    value
}

/// A first-sequence clip's start and end as the export shows them, with any
/// transition handles.
pub fn exported_span(project: &Project, clip: &str) -> Option<(Time, Time)> {
    let sequence = project.sequences.first()?;
    sequence.tracks.iter().find_map(|track| {
        track.clip(clip)?;
        v2_forms(sequence, track)
            .into_iter()
            .find(|form| form.id == clip)
            .map(|form| (form.start, form.end()))
    })
}

/// The kernel track an exported track id belongs to.
pub fn kernel_track(id: &str) -> &str {
    Lane::companion_of(id).unwrap_or(id)
}

fn seconds(time: Time) -> Json {
    json!(time.seconds())
}

pub(super) fn media_value(media: &Media) -> Json {
    let mut value = json!({
        "id": media.id.as_str(),
        "path": media.path,
        "duration": media.duration.seconds(),
        "hasAudio": media.audio.is_some(),
    });
    if let Some(video) = media.video {
        value["width"] = json!(video.width);
        value["height"] = json!(video.height);
    }
    value
}

pub(super) fn marker_value(marker: &cut_model::Marker) -> Json {
    let mut value = json!({"id": marker.id.as_str(), "time": marker.at.seconds()});
    if let Some(name) = &marker.name {
        value["label"] = json!(name);
    }
    if let Some(color) = marker.color {
        value["color"] = json!(color.name());
    }
    if marker.duration != Time::ZERO {
        value["duration"] = seconds(marker.duration);
    }
    value
}

fn captions(track: &Track) -> Json {
    let cues: Vec<Json> = track
        .clips
        .iter()
        .filter_map(|clip| match &clip.content {
            Content::Caption { text } => Some(json!({
                "id": clip.id.as_str(),
                "start": clip.start.seconds(),
                "end": clip.end().seconds(),
                "text": text,
            })),
            _ => None,
        })
        .collect();
    json!({"id": track.id.as_str(), "type": "captions", "cues": cues})
}

/// Which v2 track a kernel clip is exported to. v2 video and audio tracks
/// hold only media clips, so titles and adjustment layers move to companion
/// `title` and `adjustment` tracks named `<track>/titles` and
/// `<track>/adjust`. Other clips (nested sequences, other generators) have no
/// v2 form and are hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Lane {
    Main,
    Titles,
    Adjust,
    Hidden,
}

impl Lane {
    pub fn of(clip: &Clip) -> Self {
        match &clip.content {
            Content::Media { .. } => Self::Main,
            Content::Generator { fx, .. } if fx == "title" => Self::Titles,
            Content::Generator { fx, .. } if fx == "adjustment" => Self::Adjust,
            _ => Self::Hidden,
        }
    }

    /// The v2 track id and type for a lane of `track`.
    pub fn track(self, track: &Track) -> Option<(String, &'static str)> {
        match self {
            Self::Main => Some((track.id.to_string(), track.kind.name())),
            Self::Titles => Some((format!("{}/titles", track.id), "title")),
            Self::Adjust => Some((format!("{}/adjust", track.id), "adjustment")),
            Self::Hidden => None,
        }
    }

    /// The kernel track a companion v2 track belongs to.
    pub fn companion_of(id: &str) -> Option<&str> {
        id.strip_suffix("/titles")
            .or_else(|| id.strip_suffix("/adjust"))
    }
}

/// Clips of a video or audio track that v2 cannot show.
pub(super) fn hidden_clips(track: &Track) -> impl Iterator<Item = &Clip> {
    let hides = track.kind != TrackKind::Subtitle;
    track
        .clips
        .iter()
        .filter(move |clip| hides && Lane::of(clip) == Lane::Hidden)
}

/// Transitions v2 can show: both clips in the same exported lane.
pub(super) fn exported_transitions(track: &Track) -> impl Iterator<Item = &Transition> {
    track.transitions.iter().filter(|transition| {
        let lane = |id: &cut_model::Id| track.clip(id.as_str()).map(Lane::of);
        let from = lane(&transition.from);
        from.is_some_and(|lane| lane != Lane::Hidden) && from == lane(&transition.to)
    })
}

fn track_values(sequence: &Sequence, track: &Track) -> Vec<Json> {
    let forms = v2_forms(sequence, track);
    let mut out = Vec::new();
    for lane in [Lane::Main, Lane::Titles, Lane::Adjust] {
        let clips: Vec<Json> = forms
            .iter()
            .filter(|clip| Lane::of(clip) == lane)
            .map(|clip| {
                let mut value = clip_value(track.kind, clip);
                let incoming: Vec<Json> = exported_transitions(track)
                    .filter(|t| t.to == clip.id)
                    .map(transition_value)
                    .collect();
                if !incoming.is_empty() {
                    value["transitions"] = Json::Array(incoming);
                }
                value
            })
            .collect();
        if lane != Lane::Main && clips.is_empty() {
            continue;
        }
        let (id, kind) = lane.track(track).expect("exported lane");
        let mut value = json!({"id": id, "type": kind, "clips": clips});
        if let Some(name) = &track.name {
            value["name"] = json!(name);
        }
        if !track.enabled {
            value["enabled"] = json!(false);
        }
        if track.muted {
            value["muted"] = json!(true);
        }
        if track.locked {
            value["locked"] = json!(true);
        }
        out.push(value);
    }
    out
}

/// Clips as v2 stores them: each exported transition's portion before the
/// cut extends the incoming clip's head and the portion after it extends the
/// outgoing clip's tail, so the two overlap for the transition's duration.
pub(super) fn v2_forms(sequence: &Sequence, track: &Track) -> Vec<Clip> {
    let mut clips = track.clips.clone();
    for transition in exported_transitions(track) {
        let (before, after) = transition.split_at_cut(sequence.rate);
        for clip in &mut clips {
            if clip.id == transition.from {
                clip.move_tail(after);
            } else if clip.id == transition.to {
                clip.move_head(-before);
            }
        }
    }
    clips
}

pub(super) fn transition_value(transition: &Transition) -> Json {
    let mut value = json!({
        "id": transition.id.as_str(),
        "type": transition.fx.as_str(),
        "duration": transition.duration.seconds(),
        "alignment": match transition.align {
            Align::Center => "center",
            Align::Start => "in",
            Align::End => "out",
        },
    });
    let params = static_params(&transition.params);
    if !params.is_empty() {
        value["params"] = Json::Object(params);
    }
    value
}

fn static_params(params: &cut_model::Params) -> Map<String, Json> {
    params
        .iter()
        .filter_map(|(name, param)| match param {
            Param::Static(value) => Some((name.clone(), value_json(value))),
            Param::Animated(_) => None,
        })
        .collect()
}

fn value_json(value: &Value) -> Json {
    match value {
        Value::Number(n) => json!(n),
        Value::Bool(b) => json!(b),
        Value::Text(text) | Value::Enum(text) => json!(text),
        Value::Color(color) => json!(color.to_string()),
    }
}

/// v2 transform keys for each kernel transform property.
const TRANSFORM: [(&str, &str); 8] = [
    ("x", "x"),
    ("y", "y"),
    ("scale", "scale"),
    ("scale_x", "scaleX"),
    ("scale_y", "scaleY"),
    ("rotation", "rotation"),
    ("anchor_x", "anchorX"),
    ("anchor_y", "anchorY"),
];

/// Properties v2 can animate, with their keyframe names.
const KEYFRAMED: [(&str, &str); 7] = [
    ("opacity", "opacity"),
    ("scale", "scale"),
    ("scale_x", "scaleX"),
    ("scale_y", "scaleY"),
    ("x", "x"),
    ("y", "y"),
    ("rotation", "rotation"),
];

pub(super) fn clip_value(kind: TrackKind, clip: &Clip) -> Json {
    let mut value = json!({
        "id": clip.id.as_str(),
        "timelineStart": clip.start.seconds(),
        "duration": clip.duration.seconds(),
    });
    match &clip.content {
        Content::Media {
            media,
            source_in,
            speed,
        } => {
            value["sourceId"] = json!(media.as_str());
            value["in"] = seconds(*source_in);
            value["out"] = seconds(*source_in + speed.source_span(clip.duration));
            // v2 cannot play in reverse; the hint keeps the direction.
            if speed.value() != 1.0 {
                value["speed"] = json!(speed.value().abs());
            }
        }
        Content::Generator { fx, params } if fx == "title" => {
            if let Some(Param::Static(Value::Text(text))) = params.get("text") {
                value["text"] = json!(text);
            }
        }
        Content::Generator { .. } | Content::Sequence { .. } | Content::Caption { .. } => {}
    }
    let number = |name: &str| match clip.props.get(name) {
        Some(Param::Static(Value::Number(n))) => Some(*n),
        _ => None,
    };
    match kind {
        TrackKind::Video => {
            if let Some(opacity) = number("opacity") {
                value["opacity"] = json!(opacity);
            }
            let crop = ["crop_l", "crop_t", "crop_r", "crop_b"].map(number);
            if crop.iter().any(Option::is_some) {
                let [l, t, r, b] = crop.map(|edge| edge.unwrap_or(0.0));
                value["crop"] =
                    json!({"x": l, "y": t, "width": 1.0 - l - r, "height": 1.0 - t - b});
            }
            let transform: Map<String, Json> = TRANSFORM
                .iter()
                .filter_map(|(prop, key)| number(prop).map(|n| ((*key).to_owned(), json!(n))))
                .collect();
            if !transform.is_empty() {
                value["transform"] = Json::Object(transform);
            }
            if let Some(Param::Static(Value::Enum(blend))) = clip.props.get("blend") {
                value["blend"] = json!(blend);
            }
            let keyframes: Vec<Json> = KEYFRAMED
                .iter()
                .filter_map(|(prop, key)| match clip.props.get(*prop) {
                    Some(Param::Animated(keys)) => Some((key, keys)),
                    _ => None,
                })
                .flat_map(|(key, keys)| {
                    keys.iter().filter_map(move |k| {
                        let number = k.value.as_number()?;
                        let mut keyframe =
                            json!({"property": key, "time": k.at.seconds(), "value": number});
                        if k.interp != Interp::Linear {
                            keyframe["easing"] = json!(k.interp.name());
                        }
                        Some(keyframe)
                    })
                })
                .collect();
            if !keyframes.is_empty() {
                value["keyframes"] = Json::Array(keyframes);
            }
        }
        TrackKind::Audio => {
            if let Some(gain) = number("gain") {
                value["volume"] = json!(10f64.powf(gain / 20.0));
            }
        }
        TrackKind::Subtitle => {}
    }
    let effects: Vec<Json> = clip.effects.iter().map(effect_value).collect();
    if !effects.is_empty() {
        value["effects"] = Json::Array(effects);
    }
    value
}

fn effect_value(effect: &Effect) -> Json {
    let params = static_params(&effect.params);
    if effect.fx == "color.basic" {
        // v2 fills the remaining color parameters with their defaults.
        json!({"type": "color", "enabled": effect.enabled, "params": params})
    } else {
        json!({"type": effect.fx.as_str(), "enabled": effect.enabled, "params": params})
    }
}

/// What an exported project looks like, by kernel id, for comparing an edited
/// v2 project against the model it came from.
pub(super) struct Forms {
    pub rate: Rate,
    pub fps: f64,
    pub clips: HashMap<String, Json>,
    pub transitions: HashMap<String, Json>,
    pub media: HashMap<String, Json>,
    pub markers: HashMap<String, Json>,
}

impl Forms {
    pub fn of(project: &Project) -> Option<Self> {
        let exported = export(project).ok()?;
        let sequence = project.sequences.first()?;
        let mut forms = Self {
            rate: sequence.rate,
            fps: exported.fps,
            clips: HashMap::new(),
            transitions: HashMap::new(),
            media: HashMap::new(),
            markers: HashMap::new(),
        };
        for track in &exported.tracks {
            for clip in &track.clips {
                let mut value = serde_json::to_value(clip).ok()?;
                if let Some(transitions) = value
                    .as_object_mut()
                    .and_then(|object| object.remove("transitions"))
                {
                    for transition in transitions.as_array().into_iter().flatten() {
                        if let Some(id) = transition.get("id").and_then(Json::as_str) {
                            forms.transitions.insert(id.to_owned(), transition.clone());
                        }
                    }
                }
                forms.clips.insert(clip.id.as_str().to_owned(), value);
            }
        }
        for source in &exported.sources {
            forms.media.insert(
                source.id.as_str().to_owned(),
                serde_json::to_value(source).ok()?,
            );
        }
        for marker in &exported.markers {
            forms
                .markers
                .insert(marker.id.clone(), serde_json::to_value(marker).ok()?);
        }
        Some(forms)
    }
}
