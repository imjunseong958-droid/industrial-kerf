//! Read-only structural projection of a canonical project.
//!
//! The outline is a presentation of typed domain state for agents and other
//! clients. It deliberately has no reducer or write-back path: edits must be
//! compiled into [`crate::Command`] values and sent through `cutd`.

use crate::{Effect, JsonValue, Marker, Overlay, Project, TrackType};
use serde::{Deserialize, Serialize};

/// Version of the serialized outline shape.
pub const OUTLINE_SCHEMA_VERSION: u8 = 1;

/// Entity categories exposed by the read-only project outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlineNodeKind {
    Sequence,
    Track,
    Clip,
    Caption,
    Effect,
    Marker,
}

impl OutlineNodeKind {
    /// Return the stable wire label for this node kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::Track => "track",
            Self::Clip => "clip",
            Self::Caption => "caption",
            Self::Effect => "effect",
            Self::Marker => "marker",
        }
    }
}

/// A finite project-time interval in the outline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineTimeRange {
    pub start: f64,
    pub end: f64,
}

impl OutlineTimeRange {
    fn new(start: f64, end: f64) -> Option<Self> {
        (start.is_finite() && end.is_finite() && end >= start).then_some(Self { start, end })
    }
}

/// One node in the structural project outline.
///
/// `entity_id`, when present, is copied from the corresponding domain entity.
/// Synthetic sequence and caption-cue nodes have no invented domain ID; their
/// stable `path` points back to the source project location instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineNode {
    pub kind: OutlineNodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    pub path: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<OutlineTimeRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_type: Option<TrackType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub children: Vec<Self>,
}

/// Read-only outline of the current canonical project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOutline {
    pub schema_version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub project_name: String,
    pub root: OutlineNode,
}

/// Project the typed domain into a deterministic sequence/track hierarchy.
pub fn project_outline(project: &Project) -> ProjectOutline {
    let mut sequence_children = Vec::new();

    for (track_index, track) in project.tracks.iter().enumerate() {
        let track_path = format!("/tracks/{track_index}");
        let mut children = track
            .clips
            .iter()
            .enumerate()
            .map(|(clip_index, clip)| {
                let clip_path = format!("{track_path}/clips/{clip_index}");
                let mut clip_children = clip
                    .effects
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .map(|(effect_index, effect)| {
                        effect_node(format!("{clip_path}/effects/{effect_index}"), effect)
                    })
                    .collect::<Vec<_>>();
                clip_children.shrink_to_fit();
                OutlineNode {
                    kind: OutlineNodeKind::Clip,
                    entity_id: Some(clip.id.to_string()),
                    path: clip_path,
                    label: clip_label(clip),
                    range: clip.timeline_duration().and_then(|duration| {
                        OutlineTimeRange::new(clip.timeline_start, clip.timeline_start + duration)
                    }),
                    source_id: clip.source_id.as_ref().map(ToString::to_string),
                    track_type: None,
                    enabled: None,
                    text: clip.text.clone(),
                    children: clip_children,
                }
            })
            .collect::<Vec<_>>();

        if let Some(effects) = track.effects.as_deref() {
            children.extend(effects.iter().enumerate().map(|(effect_index, effect)| {
                effect_node(format!("{track_path}/effects/{effect_index}"), effect)
            }));
        }
        if let Some(markers) = track.markers.as_deref() {
            children.extend(markers.iter().enumerate().map(|(marker_index, marker)| {
                marker_node(format!("{track_path}/markers/{marker_index}"), marker)
            }));
        }

        sequence_children.push(OutlineNode {
            kind: OutlineNodeKind::Track,
            entity_id: Some(track.id.to_string()),
            path: track_path,
            label: track
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("{} {}", track.kind.as_str(), track_index + 1)),
            range: None,
            source_id: None,
            track_type: Some(track.kind),
            enabled: Some(track.enabled != Some(false)),
            text: None,
            children,
        });
    }

    for (overlay_index, overlay) in project.overlays.iter().enumerate() {
        let path = format!("/overlays/{overlay_index}");
        match overlay {
            Overlay::Captions(captions) => {
                let children = captions
                    .cues
                    .iter()
                    .enumerate()
                    .map(|(cue_index, cue)| OutlineNode {
                        kind: OutlineNodeKind::Caption,
                        entity_id: None,
                        path: format!("{path}/cues/{cue_index}"),
                        label: caption_label(&cue.text, cue_index),
                        range: OutlineTimeRange::new(cue.start, cue.end),
                        source_id: None,
                        track_type: None,
                        enabled: None,
                        text: Some(cue.text.clone()),
                        children: Vec::new(),
                    })
                    .collect();
                sequence_children.push(OutlineNode {
                    kind: OutlineNodeKind::Caption,
                    entity_id: Some(captions.id.clone()),
                    path,
                    label: format!("{} #{}", captions.kind, captions.id),
                    range: overlay_range(captions.start, captions.duration),
                    source_id: None,
                    track_type: None,
                    enabled: None,
                    text: None,
                    children,
                });
            }
            Overlay::Subtitles(subtitles) => sequence_children.push(OutlineNode {
                kind: OutlineNodeKind::Caption,
                entity_id: Some(subtitles.id.clone()),
                path,
                label: format!("{} #{}", subtitles.kind, subtitles.id),
                range: overlay_range(subtitles.start, subtitles.duration),
                source_id: None,
                track_type: None,
                enabled: None,
                text: Some(subtitles.path.clone()),
                children: Vec::new(),
            }),
            Overlay::Generic(_) => {}
        }
    }

    sequence_children.extend(
        project
            .effects
            .iter()
            .enumerate()
            .map(|(effect_index, effect)| effect_node(format!("/effects/{effect_index}"), effect)),
    );
    sequence_children.extend(
        project
            .markers
            .iter()
            .enumerate()
            .map(|(marker_index, marker)| marker_node(format!("/markers/{marker_index}"), marker)),
    );

    let project_name = project
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Untitled project".into());
    ProjectOutline {
        schema_version: OUTLINE_SCHEMA_VERSION,
        project_id: project.id.as_ref().map(ToString::to_string),
        project_name: project_name.clone(),
        root: OutlineNode {
            kind: OutlineNodeKind::Sequence,
            entity_id: None,
            path: "/".into(),
            label: project_name,
            range: OutlineTimeRange::new(0.0, project.duration()),
            source_id: None,
            track_type: None,
            enabled: None,
            text: None,
            children: sequence_children,
        },
    }
}

fn clip_label(clip: &crate::Clip) -> String {
    clip.text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            clip.title.as_ref().and_then(|title| match title {
                JsonValue::String(value) if !value.trim().is_empty() => Some(value.clone()),
                _ => None,
            })
        })
        .or_else(|| clip.source_id.as_ref().map(|source| source.to_string()))
        .unwrap_or_else(|| format!("Clip #{}", clip.id))
}

fn effect_node(path: String, effect: &Effect) -> OutlineNode {
    let (kind, entity_id, enabled) = match effect {
        Effect::Color(effect) => (effect.kind.as_str(), effect.id.as_deref(), effect.enabled),
        Effect::Generic(effect) => (effect.kind.as_str(), effect.id.as_deref(), effect.enabled),
    };
    let label = entity_id
        .map(|id| format!("{kind} #{id}"))
        .unwrap_or_else(|| kind.to_owned());
    OutlineNode {
        kind: OutlineNodeKind::Effect,
        entity_id: entity_id.map(str::to_owned),
        path,
        label,
        range: None,
        source_id: None,
        track_type: None,
        enabled,
        text: None,
        children: Vec::new(),
    }
}

fn marker_node(path: String, marker: &Marker) -> OutlineNode {
    let range = marker
        .duration
        .and_then(|duration| OutlineTimeRange::new(marker.time, marker.time + duration));
    OutlineNode {
        kind: OutlineNodeKind::Marker,
        entity_id: Some(marker.id.clone()),
        path,
        label: marker
            .label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| format!("Marker #{}", marker.id)),
        range,
        source_id: None,
        track_type: None,
        enabled: None,
        text: None,
        children: Vec::new(),
    }
}

fn caption_label(text: &str, index: usize) -> String {
    if text.trim().is_empty() {
        format!("Caption {}", index + 1)
    } else {
        text.to_owned()
    }
}

fn overlay_range(start: Option<f64>, duration: Option<f64>) -> Option<OutlineTimeRange> {
    let start = start.unwrap_or(0.0);
    duration.and_then(|duration| OutlineTimeRange::new(start, start + duration))
}

#[cfg(test)]
mod tests {
    use super::{project_outline, OutlineNodeKind};
    use crate::{
        empty_project, CaptionCue, CaptionsOverlay, ColorEffect, ColorEffectParams, Effect, Marker,
        TrackType,
    };

    #[test]
    fn projects_domain_ids_and_nested_entities_without_write_back() {
        let mut project = empty_project();
        project.id = Some(crate::ProjectId::new("project-1").expect("project id"));
        project.name = Some("Launch".into());
        project.tracks.push(crate::Track {
            id: crate::TrackId::new("video-1").expect("track id"),
            kind: crate::TrackType::Video,
            name: None,
            enabled: None,
            muted: None,
            locked: None,
            clips: Vec::new(),
            gaps: None,
            transitions: None,
            effects: None,
            markers: None,
            extensions: Default::default(),
        });
        project.tracks[0].name = Some("Picture".into());
        project.tracks[0].clips.push(crate::Clip {
            id: crate::ClipId::new("clip-1").expect("clip id"),
            source_id: None,
            source_in: None,
            source_out: None,
            timeline_start: 1.0,
            duration: Some(2.0),
            speed: None,
            volume: None,
            opacity: None,
            crop: None,
            transform: None,
            blend: None,
            blend_mode: None,
            effects: Some(vec![Effect::Color(ColorEffect {
                id: Some("fx-1".into()),
                kind: "color".into(),
                enabled: Some(true),
                params: ColorEffectParams {
                    exposure: 0.0,
                    contrast: 0.0,
                    brightness: 0.0,
                    saturation: 0.0,
                    temperature: 0.0,
                    tint: 0.0,
                    gamma: 0.0,
                    highlights: 0.0,
                    shadows: 0.0,
                    vibrance: 0.0,
                    mix: 1.0,
                    extensions: Default::default(),
                },
                extensions: Default::default(),
            })]),
            keyframes: None,
            transitions: None,
            gaps: None,
            text: None,
            title: None,
            extensions: Default::default(),
        });
        project.tracks[0].markers = Some(vec![Marker {
            id: "marker-1".into(),
            time: 1.5,
            label: Some("Beat".into()),
            color: None,
            duration: None,
            metadata: None,
            extensions: Default::default(),
        }]);
        project
            .overlays
            .push(crate::Overlay::Captions(CaptionsOverlay {
                id: "captions-1".into(),
                kind: "captions".into(),
                cues: vec![CaptionCue {
                    start: 1.0,
                    end: 2.0,
                    text: "Hello".into(),
                    extensions: Default::default(),
                }],
                style: None,
                start: None,
                duration: None,
                extensions: Default::default(),
            }));

        let outline = project_outline(&project);
        assert_eq!(outline.project_id.as_deref(), Some("project-1"));
        assert_eq!(outline.root.kind, OutlineNodeKind::Sequence);
        assert_eq!(
            outline.root.children[0].entity_id.as_deref(),
            Some("video-1")
        );
        assert_eq!(
            outline.root.children[0].children[0].entity_id.as_deref(),
            Some("clip-1")
        );
        assert_eq!(
            outline.root.children[0].children[0].children[0]
                .entity_id
                .as_deref(),
            Some("fx-1")
        );
        assert_eq!(
            outline.root.children[0].children[1].entity_id.as_deref(),
            Some("marker-1")
        );
        let captions = &outline.root.children[1];
        assert_eq!(captions.entity_id.as_deref(), Some("captions-1"));
        assert_eq!(captions.children[0].path, "/overlays/0/cues/0");
        assert_eq!(captions.children[0].text.as_deref(), Some("Hello"));
        assert_eq!(project.tracks[0].kind, TrackType::Video);
    }

    #[test]
    fn invalid_or_point_ranges_do_not_create_fake_intervals() {
        let mut project = empty_project();
        project.markers.push(Marker {
            id: "marker-1".into(),
            time: 3.0,
            label: None,
            color: None,
            duration: None,
            metadata: None,
            extensions: Default::default(),
        });
        let outline = project_outline(&project);
        let marker = outline.root.children.last().expect("marker");
        assert!(marker.range.is_none());
    }
}
