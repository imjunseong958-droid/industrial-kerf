//! Canonical, typed Cut editing domain.
//!
//! JSON is an input/output concern.  The public model below contains no
//! `serde_json::Value`; unrecognised fields are retained as a private typed
//! JSON sidecar and are reattached by the wire adapter when a project is
//! serialized again.  This keeps the reducer and history independent from
//! the transport representation while preserving forward-compatible fields.

use cut_core::{FrameGrid, FrameIndex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub mod outline;

pub use outline::{
    project_outline, OutlineNode, OutlineNodeKind, OutlineTimeRange, ProjectOutline,
};

/// A JSON leaf used only for extension data and deliberately distinct from
/// `serde_json::Value`, which is kept inside the private wire adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    fn to_wire(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(*value),
            Self::Number(value) => Value::Number(value.clone()),
            Self::String(value) => Value::String(value.clone()),
            Self::Array(values) => Value::Array(values.iter().map(Self::to_wire).collect()),
            Self::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_wire()))
                    .collect(),
            ),
        }
    }
}

pub type JsonObject = BTreeMap<String, JsonValue>;

/// Domain errors are stable enough for runtime adapters to map to protocol
/// errors, while retaining a useful human-readable message for callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainError {
    pub code: &'static str,
    pub message: String,
}

impl DomainError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DomainError {}

impl From<serde_json::Error> for DomainError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("INVALID_JSON", error.to_string())
    }
}

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(DomainError::new("INVALID_ID", concat!(stringify!($name), " must be non-empty")));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str { &self.0 }
            pub fn into_string(self) -> String { self.0 }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self { value.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where S: Serializer {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where D: Deserializer<'de> {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

string_id!(ProjectId);
string_id!(SourceId);
string_id!(TrackId);
string_id!(ClipId);
string_id!(OverlayId);
string_id!(ProposalId);
string_id!(JobId);
string_id!(TransactionId);
string_id!(RequestId);
string_id!(IdempotencyKey);

/// Revision numbers are nonnegative by construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    pub const ZERO: Self = Self(0);
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for Revision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}
impl From<Revision> for u64 {
    fn from(value: Revision) -> Self {
        value.0
    }
}
impl Serialize for Revision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}
impl<'de> Deserialize<'de> for Revision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u64::deserialize(deserializer)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    Video,
    Audio,
    Title,
    Adjustment,
}

impl TrackType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Title => "title",
            Self::Adjustment => "adjustment",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: SourceId,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<JsonValue>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorEffectParams {
    pub exposure: f64,
    pub contrast: f64,
    pub brightness: f64,
    pub saturation: f64,
    pub temperature: f64,
    pub tint: f64,
    pub gamma: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub vibrance: f64,
    pub mix: f64,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub property: String,
    pub time: f64,
    pub value: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub easing: Option<String>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub duration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<TransitionAlignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonObject>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionAlignment {
    In,
    Out,
    Center,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub start: f64,
    pub duration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub id: String,
    pub time: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub params: ColorEffectParams,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenericEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonObject>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Effect {
    Color(ColorEffect),
    Generic(GenericEffect),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_v: Option<f64>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitlesOverlay {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<SubtitleStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsOverlay {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub cues: Vec<CaptionCue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<SubtitleStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenericOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Overlay {
    Subtitles(SubtitlesOverlay),
    Captions(CaptionsOverlay),
    Generic(GenericOverlay),
}

impl Overlay {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Subtitles(value) => Some(&value.id),
            Self::Captions(value) => Some(&value.id),
            Self::Generic(value) => value.id.as_deref(),
        }
    }

    pub fn kind(&self) -> Option<&str> {
        match self {
            Self::Subtitles(value) => Some(&value.kind),
            Self::Captions(value) => Some(&value.kind),
            Self::Generic(value) => value.kind.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: ClipId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "in")]
    pub source_in: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "out")]
    pub source_out: Option<f64>,
    pub timeline_start: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropRegion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframes: Option<Vec<Keyframe>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Vec<Transition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gaps: Option<Vec<Gap>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<JsonValue>,
    #[serde(flatten)]
    extensions: JsonObject,
}

impl Clip {
    pub fn source_in(&self) -> Option<f64> {
        self.source_in
    }
    pub fn source_out(&self) -> Option<f64> {
        self.source_out
    }
    pub fn timeline_duration(&self) -> Option<f64> {
        self.duration
            .or_else(|| match (self.source_in, self.source_out) {
                (Some(input), Some(output)) => Some((output - input) / self.speed.unwrap_or(1.0)),
                _ => None,
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: TrackId,
    #[serde(rename = "type")]
    pub kind: TrackType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    pub clips: Vec<Clip>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gaps: Option<Vec<Gap>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transitions: Option<Vec<Transition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markers: Option<Vec<Marker>>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<ProjectId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub fps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    pub sources: Vec<Source>,
    pub tracks: Vec<Track>,
    pub overlays: Vec<Overlay>,
    pub gaps: Vec<Gap>,
    pub transitions: Vec<Transition>,
    pub effects: Vec<Effect>,
    pub markers: Vec<Marker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    #[serde(flatten)]
    extensions: JsonObject,
}

impl Project {
    pub fn duration(&self) -> f64 {
        project_duration(self)
    }
}

/// The edge moved by a semantic clip trim command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimEdge {
    /// Move the first frame of the clip.
    Leading,
    /// Move the first frame after the clip.
    Trailing,
}

/// Canonical project commands.  The enum is the only input to the reducer;
/// command aliases are resolved by `parse_command` at the wire boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    AddTrack {
        track: Track,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    AddClip {
        track_id: TrackId,
        clip: Clip,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    RemoveClip {
        clip_id: ClipId,
        #[serde(skip_serializing_if = "Option::is_none")]
        track_id: Option<TrackId>,
    },
    /// Place an existing clip at a frame, optionally moving it to another track.
    PlaceClip {
        clip_id: ClipId,
        timeline_frame: FrameIndex,
        #[serde(skip_serializing_if = "Option::is_none")]
        track_id: Option<TrackId>,
    },
    /// Move one typed edge of an existing clip to a timeline frame.
    TrimClip {
        clip_id: ClipId,
        edge: TrimEdge,
        timeline_frame: FrameIndex,
    },
    /// Split an existing clip at an interior frame and assign the right half a new ID.
    SplitClip {
        clip_id: ClipId,
        at_frame: FrameIndex,
        new_clip_id: ClipId,
    },
    MoveClip {
        clip_id: ClipId,
        #[serde(skip_serializing_if = "Option::is_none")]
        track_id: Option<TrackId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_track_id: Option<TrackId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_index: Option<usize>,
    },
    UpdateClip {
        clip_id: ClipId,
        patch: JsonObject,
    },
    SetClip {
        clip_id: ClipId,
        clip: Clip,
    },
    AddMarker {
        marker: Marker,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    RemoveMarker {
        marker_id: String,
    },
    SetMetadata {
        patch: JsonObject,
    },
    RestoreMetadata {
        metadata: Option<JsonObject>,
    },
    AddSource {
        source: Source,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    RemoveSource {
        source_id: SourceId,
    },
    AddOverlay {
        overlay: Overlay,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    RemoveOverlay {
        overlay_id: String,
    },
    Batch {
        commands: Vec<Command>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub command: Command,
    pub before: Project,
    pub after: Project,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub project: Project,
    pub revision: Revision,
    pub past: Vec<HistoryEntry>,
    pub future: Vec<HistoryEntry>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacySource {
    pub id: SourceId,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyClip {
    pub id: ClipId,
    pub src: SourceId,
    #[serde(rename = "in")]
    pub source_in: f64,
    #[serde(rename = "out")]
    pub source_out: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropRegion>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTrack {
    pub id: TrackId,
    pub clips: Vec<LegacyClip>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyTimeline {
    pub version: u8,
    pub fps: f64,
    pub sources: Vec<LegacySource>,
    pub tracks: Vec<LegacyTrack>,
    pub overlays: Vec<Overlay>,
    #[serde(flatten)]
    extensions: JsonObject,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum VersionedProject {
    V1(LegacyTimeline),
    V2(Project),
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, DomainError> {
    value
        .as_object()
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", format!("{label} must be an object")))
}

fn object_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut Map<String, Value>, DomainError> {
    value
        .as_object_mut()
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", format!("{label} must be an object")))
}

fn required_string(
    value: &Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<String, DomainError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            DomainError::new(
                "INVALID_PROJECT",
                format!("{label}.{key} must be a non-empty string"),
            )
        })
}

fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}

fn nonnegative(value: &Value) -> Option<f64> {
    finite(value).filter(|value| *value >= 0.0)
}

fn positive(value: &Value) -> Option<f64> {
    finite(value).filter(|value| *value > 0.0)
}

fn alias(
    raw: &Map<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<Option<Value>, DomainError> {
    let mut found: Option<(&str, &Value)> = None;
    for key in keys {
        let Some(value) = raw.get(*key) else { continue };
        if let Some((first_key, first_value)) = found {
            // JSON cannot preserve JavaScript object identity.  Deep equality
            // is the useful wire equivalent for object/array aliases and is
            // stricter than silently selecting one spelling.
            if first_value != value {
                return Err(DomainError::new(
                    "ALIAS_CONFLICT",
                    format!("{label} aliases must agree ({first_key} conflicts with {key})"),
                ));
            }
        } else {
            found = Some((key, value));
        }
    }
    Ok(found.map(|(_, value)| value.clone()))
}

fn set_alias(
    raw: &mut Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
    label: &str,
) -> Result<(), DomainError> {
    if let Some(value) = alias(raw, aliases, label)? {
        raw.insert(canonical.to_owned(), value);
    }
    for key in aliases {
        if *key != canonical {
            raw.remove(*key);
        }
    }
    Ok(())
}

fn deserialize<T: for<'de> Deserialize<'de>>(
    value: Value,
    code: &'static str,
) -> Result<T, DomainError> {
    serde_json::from_value(value).map_err(|error| DomainError::new(code, error.to_string()))
}

fn normalise_source(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "source")?;
    required_string(raw, "id", "source")?;
    if raw.get("path").and_then(Value::as_str).is_none() {
        return Err(DomainError::new(code, "source.path must be a string"));
    }
    if let Some(duration) = raw.get("duration") {
        if nonnegative(duration).is_none() {
            return Err(DomainError::new(
                code,
                "source.duration must be nonnegative",
            ));
        }
    }
    if let Some(width) = raw.get("width") {
        if positive(width).is_none() {
            return Err(DomainError::new(code, "source.width must be positive"));
        }
    }
    if let Some(height) = raw.get("height") {
        if positive(height).is_none() {
            return Err(DomainError::new(code, "source.height must be positive"));
        }
    }
    Ok(value)
}

fn normalise_crop(value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object(&value, "crop")?;
    for key in ["x", "y", "width", "height"] {
        if finite(raw.get(key).unwrap_or(&Value::Null)).is_none() {
            return Err(DomainError::new(code, format!("crop.{key} must be finite")));
        }
    }
    let x = finite(raw.get("x").unwrap()).unwrap();
    let y = finite(raw.get("y").unwrap()).unwrap();
    let width = finite(raw.get("width").unwrap()).unwrap();
    let height = finite(raw.get("height").unwrap()).unwrap();
    if !(x >= 0.0
        && y >= 0.0
        && width > 0.0
        && height > 0.0
        && x <= 1.0
        && y <= 1.0
        && width <= 1.0
        && height <= 1.0
        && x + width <= 1.0
        && y + height <= 1.0)
    {
        return Err(DomainError::new(
            code,
            "crop must be normalized and stay within [0, 1] bounds",
        ));
    }
    Ok(value)
}

fn normalise_clip(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "clip")?;
    required_string(raw, "id", "clip")?;
    set_alias(
        raw,
        "sourceId",
        &["sourceId", "src", "source", "assetId"],
        "clip source",
    )?;
    set_alias(
        raw,
        "in",
        &["in", "sourceIn", "sourceStart"],
        "clip source in",
    )?;
    set_alias(
        raw,
        "out",
        &["out", "sourceOut", "sourceEnd"],
        "clip source out",
    )?;
    set_alias(
        raw,
        "timelineStart",
        &["timelineStart", "start"],
        "clip timeline start",
    )?;
    if nonnegative(raw.get("timelineStart").unwrap_or(&Value::Null)).is_none() {
        return Err(DomainError::new(
            code,
            "clip requires timelineStart or start",
        ));
    }
    let has_in = raw.contains_key("in");
    let has_out = raw.contains_key("out");
    if has_in != has_out {
        return Err(DomainError::new(
            code,
            "clip source interval requires both in and out",
        ));
    }
    if has_in {
        let input = nonnegative(raw.get("in").unwrap())
            .ok_or_else(|| DomainError::new(code, "clip.in must be nonnegative"))?;
        let output = nonnegative(raw.get("out").unwrap())
            .ok_or_else(|| DomainError::new(code, "clip.out must be nonnegative"))?;
        if output <= input {
            return Err(DomainError::new(code, "clip out must be greater than in"));
        }
    }
    for key in ["duration", "speed", "volume", "opacity"] {
        if let Some(number) = raw.get(key) {
            let valid = match key {
                "speed" => positive(number),
                "opacity" => finite(number).filter(|v| (0.0..=1.0).contains(v)),
                _ => nonnegative(number),
            };
            if valid.is_none() {
                return Err(DomainError::new(
                    code,
                    format!("clip.{key} has an invalid number"),
                ));
            }
        }
    }
    if let Some(crop) = raw.get("crop").cloned() {
        raw.insert("crop".into(), normalise_crop(crop, code)?);
    }
    if let Some(effects) = raw.get("effects").and_then(Value::as_array).cloned() {
        raw.insert(
            "effects".into(),
            Value::Array(
                effects
                    .into_iter()
                    .map(|effect| normalise_effect(effect, code))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    if let Some(gaps) = raw.get("gaps").and_then(Value::as_array).cloned() {
        raw.insert(
            "gaps".into(),
            Value::Array(
                gaps.into_iter()
                    .map(|gap| normalise_gap(gap, code))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    Ok(value)
}

fn normalise_gap(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "gap")?;
    set_alias(raw, "start", &["start", "timelineStart"], "gap start")?;
    if nonnegative(raw.get("start").unwrap_or(&Value::Null)).is_none() {
        return Err(DomainError::new(
            code,
            "gap requires start or timelineStart",
        ));
    }
    if positive(raw.get("duration").unwrap_or(&Value::Null)).is_none() {
        return Err(DomainError::new(code, "gap.duration must be positive"));
    }
    Ok(value)
}

fn normalise_marker(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "marker")?;
    required_string(raw, "id", "marker")?;
    set_alias(raw, "time", &["time", "start"], "marker time")?;
    if nonnegative(raw.get("time").unwrap_or(&Value::Null)).is_none() {
        return Err(DomainError::new(code, "marker requires time or start"));
    }
    if let Some(duration) = raw.get("duration") {
        if nonnegative(duration).is_none() {
            return Err(DomainError::new(
                code,
                "marker.duration must be nonnegative",
            ));
        }
    }
    Ok(value)
}

fn default_color_params(
    raw: &mut Map<String, Value>,
    code: &'static str,
) -> Result<(), DomainError> {
    let defaults: [(&str, f64); 11] = [
        ("exposure", 0.0),
        ("contrast", 1.0),
        ("brightness", 0.0),
        ("saturation", 1.0),
        ("temperature", 0.0),
        ("tint", 0.0),
        ("gamma", 1.0),
        ("highlights", 0.0),
        ("shadows", 0.0),
        ("vibrance", 0.0),
        ("mix", 1.0),
    ];
    for (key, value) in defaults {
        if !raw.contains_key(key) {
            raw.insert(key.into(), Value::from(value));
        }
    }
    let bounds: [(&str, f64, f64); 11] = [
        ("exposure", -3.0, 3.0),
        ("contrast", 0.0, 4.0),
        ("brightness", -1.0, 1.0),
        ("saturation", 0.0, 4.0),
        ("temperature", -1000.0, 1000.0),
        ("tint", -1.0, 1.0),
        ("gamma", 0.1, 10.0),
        ("highlights", -1.0, 1.0),
        ("shadows", -1.0, 1.0),
        ("vibrance", -1.0, 1.0),
        ("mix", 0.0, 1.0),
    ];
    for (key, min, max) in bounds {
        let value = finite(raw.get(key).unwrap_or(&Value::Null))
            .filter(|value| *value >= min && *value <= max);
        if value.is_none() {
            return Err(DomainError::new(
                code,
                format!("color params.{key} is out of range"),
            ));
        }
    }
    Ok(())
}

fn normalise_effect(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "effect")?;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::new(code, "effect.type must be a string"))?;
    if kind.eq_ignore_ascii_case("color") {
        let params = raw
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()));
        let params = params
            .as_object_mut()
            .ok_or_else(|| DomainError::new(code, "color effect.params must be an object"))?;
        default_color_params(params, code)?;
    }
    Ok(value)
}

fn normalise_overlay(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "overlay")?;
    if let Some(id) = raw.get("id") {
        if id.as_str().filter(|value| !value.is_empty()).is_none() {
            return Err(DomainError::new(
                code,
                "overlay.id must be a non-empty string",
            ));
        }
    }
    set_alias(raw, "type", &["type", "kind"], "overlay type")?;
    if let Some(kind) = raw.get("type") {
        if kind.as_str().filter(|value| !value.is_empty()).is_none() {
            return Err(DomainError::new(
                code,
                "overlay.type must be a non-empty string",
            ));
        }
    }
    if let Some(start) = raw.get("start") {
        if nonnegative(start).is_none() {
            return Err(DomainError::new(code, "overlay.start must be nonnegative"));
        }
    }
    if let Some(start) = raw.get("timelineStart") {
        if nonnegative(start).is_none() {
            return Err(DomainError::new(
                code,
                "overlay.timelineStart must be nonnegative",
            ));
        }
    }
    if let Some(duration) = raw.get("duration") {
        if positive(duration).is_none() {
            return Err(DomainError::new(code, "overlay.duration must be positive"));
        }
    }
    if raw.get("type").and_then(Value::as_str) == Some("captions") {
        let cues = raw
            .get("cues")
            .and_then(Value::as_array)
            .ok_or_else(|| DomainError::new(code, "captions overlay.cues must be an array"))?;
        if cues.is_empty() {
            return Err(DomainError::new(
                code,
                "captions overlay.cues must not be empty",
            ));
        }
        for cue in cues {
            let cue = object(cue, "caption cue")?;
            let start = nonnegative(cue.get("start").unwrap_or(&Value::Null))
                .ok_or_else(|| DomainError::new(code, "caption cue start must be nonnegative"))?;
            let end = nonnegative(cue.get("end").unwrap_or(&Value::Null))
                .ok_or_else(|| DomainError::new(code, "caption cue end must be nonnegative"))?;
            if end <= start
                || cue
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                return Err(DomainError::new(code, "caption cue is invalid"));
            }
        }
    }
    Ok(value)
}

fn normalise_track(mut value: Value, code: &'static str) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "track")?;
    required_string(raw, "id", "track")?;
    set_alias(raw, "type", &["type", "kind"], "track type")?;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| DomainError::new(code, "v2 track requires type or kind"))?;
    if !matches!(kind, "video" | "audio" | "title" | "adjustment") {
        return Err(DomainError::new(
            code,
            format!("unsupported track.type: {kind}"),
        ));
    }
    let clips = raw
        .get("clips")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new(code, "track.clips must be an array"))?;
    let clips = clips
        .iter()
        .cloned()
        .map(|clip| normalise_clip(clip, code))
        .collect::<Result<Vec<_>, _>>()?;
    raw.insert("clips".into(), Value::Array(clips));
    if let Some(gaps) = raw.get("gaps").and_then(Value::as_array) {
        raw.insert(
            "gaps".into(),
            Value::Array(
                gaps.iter()
                    .cloned()
                    .map(|gap| normalise_gap(gap, code))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    if let Some(markers) = raw.get("markers").and_then(Value::as_array) {
        raw.insert(
            "markers".into(),
            Value::Array(
                markers
                    .iter()
                    .cloned()
                    .map(|marker| normalise_marker(marker, code))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    if let Some(effects) = raw.get("effects").and_then(Value::as_array) {
        raw.insert(
            "effects".into(),
            Value::Array(
                effects
                    .iter()
                    .cloned()
                    .map(|effect| normalise_effect(effect, code))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
    }
    Ok(value)
}

fn normalise_project(mut value: Value) -> Result<Value, DomainError> {
    let raw = object_mut(&mut value, "project")?;
    if raw.get("version").and_then(Value::as_u64) != Some(2) {
        return Err(DomainError::new(
            "INVALID_PROJECT",
            "project.version must be 2",
        ));
    }
    if positive(raw.get("fps").unwrap_or(&Value::Null)).is_none() {
        return Err(DomainError::new(
            "INVALID_PROJECT",
            "project.fps must be positive",
        ));
    }
    for key in [
        "sources",
        "tracks",
        "overlays",
        "gaps",
        "transitions",
        "effects",
        "markers",
    ] {
        if !raw.contains_key(key) {
            raw.insert(key.into(), Value::Array(Vec::new()));
        }
    }
    let sources = raw
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.sources must be an array"))?;
    raw.insert(
        "sources".into(),
        Value::Array(
            sources
                .iter()
                .cloned()
                .map(|source| normalise_source(source, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let tracks = raw
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.tracks must be an array"))?;
    raw.insert(
        "tracks".into(),
        Value::Array(
            tracks
                .iter()
                .cloned()
                .map(|track| normalise_track(track, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let overlays = raw
        .get("overlays")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.overlays must be an array"))?;
    raw.insert(
        "overlays".into(),
        Value::Array(
            overlays
                .iter()
                .cloned()
                .map(|overlay| normalise_overlay(overlay, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let gaps = raw
        .get("gaps")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.gaps must be an array"))?;
    raw.insert(
        "gaps".into(),
        Value::Array(
            gaps.iter()
                .cloned()
                .map(|gap| normalise_gap(gap, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let markers = raw
        .get("markers")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.markers must be an array"))?;
    raw.insert(
        "markers".into(),
        Value::Array(
            markers
                .iter()
                .cloned()
                .map(|marker| normalise_marker(marker, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let effects = raw
        .get("effects")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::new("INVALID_PROJECT", "project.effects must be an array"))?;
    raw.insert(
        "effects".into(),
        Value::Array(
            effects
                .iter()
                .cloned()
                .map(|effect| normalise_effect(effect, "INVALID_PROJECT"))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    let source_ids: HashSet<&str> = raw["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|source| source.get("id").and_then(Value::as_str))
        .collect();
    let tracks = raw["tracks"].as_array().unwrap();
    let mut track_ids = HashSet::new();
    let mut clip_ids = HashSet::new();
    for track in tracks {
        let track_id = track.get("id").and_then(Value::as_str).unwrap();
        if !track_ids.insert(track_id) {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                format!("duplicate track id: {track_id}"),
            ));
        }
        let kind = track.get("type").and_then(Value::as_str).unwrap();
        for clip in track["clips"].as_array().unwrap() {
            let clip_id = clip.get("id").and_then(Value::as_str).unwrap();
            if !clip_ids.insert(clip_id) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate clip id: {clip_id}"),
                ));
            }
            let source_id = clip.get("sourceId").and_then(Value::as_str);
            if matches!(kind, "video" | "audio") && source_id.is_none() {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("{kind} clip requires a sourceId"),
                ));
            }
            if let Some(source_id) = source_id {
                if !source_ids.contains(source_id) {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("clip references unknown source: {source_id}"),
                    ));
                }
            }
            if source_id.is_none() && clip.get("duration").is_none() && clip.get("in").is_none() {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "source-less clip requires duration",
                ));
            }
            if let (Some(source_id), Some(output)) =
                (source_id, clip.get("out").and_then(Value::as_f64))
            {
                if let Some(source_duration) = raw["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|source| source.get("id").and_then(Value::as_str) == Some(source_id))
                    .and_then(|source| source.get("duration"))
                    .and_then(Value::as_f64)
                {
                    if output > source_duration {
                        return Err(DomainError::new(
                            "INVALID_PROJECT",
                            format!("clip out exceeds source duration {source_duration}"),
                        ));
                    }
                }
            }
        }
    }
    let mut ids = HashSet::new();
    for (key, label) in [
        ("sources", "source"),
        ("tracks", "track"),
        ("overlays", "overlay"),
        ("markers", "marker"),
    ] {
        ids.clear();
        for value in raw[key].as_array().unwrap() {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                if !ids.insert(id) {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("duplicate {label} id: {id}"),
                    ));
                }
            }
        }
    }
    Ok(value)
}

/// The only module that interprets untyped JSON.  Its helpers deliberately
/// sit at the boundary so the model and reducer below never need to inspect a
/// JSON map or discriminator string.
mod wire {
    use super::*;

    pub(super) fn parse_v1(value: Value) -> Result<LegacyTimeline, DomainError> {
        let raw = object(&value, "timeline")?;
        if raw.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(DomainError::new(
                "INVALID_TIMELINE",
                "timeline.version must be 1",
            ));
        }
        if positive(raw.get("fps").unwrap_or(&Value::Null)).is_none() {
            return Err(DomainError::new(
                "INVALID_TIMELINE",
                "timeline.fps must be positive",
            ));
        }
        let sources = raw
            .get("sources")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                DomainError::new("INVALID_TIMELINE", "timeline.sources must be an array")
            })?;
        let tracks = raw.get("tracks").and_then(Value::as_array).ok_or_else(|| {
            DomainError::new("INVALID_TIMELINE", "timeline.tracks must be an array")
        })?;
        let overlays = raw
            .get("overlays")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                DomainError::new("INVALID_TIMELINE", "timeline.overlays must be an array")
            })?;
        let sources: Vec<LegacySource> = sources
            .iter()
            .cloned()
            .map(|source| {
                normalise_source(source, "INVALID_TIMELINE")
                    .and_then(|value| deserialize(value, "INVALID_TIMELINE"))
            })
            .collect::<Result<_, _>>()?;
        let source_ids: HashSet<&str> = sources.iter().map(|source| source.id.as_str()).collect();
        let mut seen_source = HashSet::new();
        for source in &sources {
            if !seen_source.insert(source.id.as_str()) {
                return Err(DomainError::new(
                    "INVALID_TIMELINE",
                    format!("duplicate source id: {}", source.id),
                ));
            }
        }
        let mut seen_track = HashSet::new();
        let mut seen_clip = HashSet::new();
        let mut parsed_tracks: Vec<LegacyTrack> = Vec::with_capacity(tracks.len());
        for track in tracks {
            let track_raw = object(track, "track")?;
            let track_id = required_string(track_raw, "id", "track")?;
            if !seen_track.insert(track_id.clone()) {
                return Err(DomainError::new(
                    "INVALID_TIMELINE",
                    format!("duplicate track id: {track_id}"),
                ));
            }
            let clips = track_raw
                .get("clips")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    DomainError::new("INVALID_TIMELINE", "track.clips must be an array")
                })?;
            let mut parsed_clips: Vec<LegacyClip> = Vec::with_capacity(clips.len());
            for clip in clips {
                let clip_raw = object(clip, "clip")?;
                let clip_id = required_string(clip_raw, "id", "clip")?;
                if !seen_clip.insert(clip_id.clone()) {
                    return Err(DomainError::new(
                        "INVALID_TIMELINE",
                        format!("duplicate clip id: {clip_id}"),
                    ));
                }
                let source_id = required_string(clip_raw, "src", "clip")?;
                if !source_ids.contains(source_id.as_str()) {
                    return Err(DomainError::new(
                        "INVALID_TIMELINE",
                        format!("clip references unknown source: {source_id}"),
                    ));
                }
                let input =
                    nonnegative(clip_raw.get("in").unwrap_or(&Value::Null)).ok_or_else(|| {
                        DomainError::new(
                            "INVALID_TIMELINE",
                            "clip.in must be finite and nonnegative",
                        )
                    })?;
                let output =
                    nonnegative(clip_raw.get("out").unwrap_or(&Value::Null)).ok_or_else(|| {
                        DomainError::new(
                            "INVALID_TIMELINE",
                            "clip.out must be finite and nonnegative",
                        )
                    })?;
                if output <= input {
                    return Err(DomainError::new(
                        "INVALID_TIMELINE",
                        "clip out must be greater than in",
                    ));
                }
                if let Some(source_duration) = sources
                    .iter()
                    .find(|source| source.id.as_str() == source_id)
                    .and_then(|source| source.duration)
                {
                    if output > source_duration {
                        return Err(DomainError::new(
                            "INVALID_TIMELINE",
                            format!("clip out exceeds source duration {source_duration}"),
                        ));
                    }
                }
                let mut canonical = clip.clone();
                if let Some(speed) = clip_raw.get("speed") {
                    if positive(speed).is_none() {
                        return Err(DomainError::new(
                            "INVALID_TIMELINE",
                            "clip.speed must be positive",
                        ));
                    }
                }
                if let Some(volume) = clip_raw.get("volume") {
                    if nonnegative(volume).is_none() {
                        return Err(DomainError::new(
                            "INVALID_TIMELINE",
                            "clip.volume must be nonnegative",
                        ));
                    }
                }
                if let Some(crop) = clip_raw.get("crop").cloned() {
                    object_mut(&mut canonical, "clip")?
                        .insert("crop".into(), normalise_crop(crop, "INVALID_TIMELINE")?);
                }
                parsed_clips.push(deserialize(canonical, "INVALID_TIMELINE")?);
            }
            let mut canonical_track = track.clone();
            object_mut(&mut canonical_track, "track")?.insert(
                "clips".into(),
                serde_json::to_value(&parsed_clips)
                    .map_err(|error| DomainError::new("INVALID_TIMELINE", error.to_string()))?,
            );
            parsed_tracks.push(deserialize(canonical_track, "INVALID_TIMELINE")?);
        }
        let overlays = overlays
            .iter()
            .cloned()
            .map(|overlay| {
                let raw = object(&overlay, "overlay")?;
                if raw
                    .get("kind")
                    .and_then(Value::as_str)
                    .filter(|kind| !kind.is_empty())
                    .is_none()
                    && raw
                        .get("type")
                        .and_then(Value::as_str)
                        .filter(|kind| !kind.is_empty())
                        .is_none()
                {
                    return Err(DomainError::new(
                        "INVALID_TIMELINE",
                        "overlay.kind must be a non-empty string",
                    ));
                }
                normalise_overlay(overlay, "INVALID_TIMELINE")
                    .and_then(|value| deserialize(value, "INVALID_TIMELINE"))
            })
            .collect::<Result<Vec<Overlay>, _>>()?;
        let mut canonical = value;
        object_mut(&mut canonical, "timeline")?.insert(
            "sources".into(),
            serde_json::to_value(&sources)
                .map_err(|error| DomainError::new("INVALID_TIMELINE", error.to_string()))?,
        );
        object_mut(&mut canonical, "timeline")?.insert(
            "tracks".into(),
            serde_json::to_value(&parsed_tracks)
                .map_err(|error| DomainError::new("INVALID_TIMELINE", error.to_string()))?,
        );
        object_mut(&mut canonical, "timeline")?.insert(
            "overlays".into(),
            serde_json::to_value(&overlays)
                .map_err(|error| DomainError::new("INVALID_TIMELINE", error.to_string()))?,
        );
        // The legacy type intentionally uses the original v1 keys.  Parsing
        // through serde here keeps extension fields and numeric spellings.
        deserialize(canonical, "INVALID_TIMELINE")
    }

    pub(super) fn parse_v2(value: Value) -> Result<Project, DomainError> {
        let canonical = normalise_project(value)?;
        let project: Project = deserialize(canonical, "INVALID_PROJECT")?;
        validate_typed_project(&project)?;
        Ok(project)
    }

    fn validate_typed_project(project: &Project) -> Result<(), DomainError> {
        if project.version != 2 || !project.fps.is_finite() || project.fps <= 0.0 {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                "project.version/fps is invalid",
            ));
        }
        let source_ids: HashSet<&str> = project
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect();
        let mut track_ids = HashSet::new();
        let mut clip_ids = HashSet::new();
        for track in &project.tracks {
            if !track_ids.insert(track.id.as_str()) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate track id: {}", track.id),
                ));
            }
            for clip in &track.clips {
                if !clip_ids.insert(clip.id.as_str()) {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("duplicate clip id: {}", clip.id),
                    ));
                }
                if matches!(track.kind, TrackType::Video | TrackType::Audio)
                    && clip.source_id.is_none()
                {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("{} clip requires a sourceId", track.kind.as_str()),
                    ));
                }
                if let Some(source) = &clip.source_id {
                    if !source_ids.contains(source.as_str()) {
                        return Err(DomainError::new(
                            "INVALID_PROJECT",
                            format!("clip references unknown source: {source}"),
                        ));
                    }
                    if let (Some(output), Some(duration)) = (
                        clip.source_out,
                        project
                            .sources
                            .iter()
                            .find(|candidate| candidate.id == *source)
                            .and_then(|candidate| candidate.duration),
                    ) {
                        if output > duration {
                            return Err(DomainError::new(
                                "INVALID_PROJECT",
                                format!("clip out exceeds source duration {duration}"),
                            ));
                        }
                    }
                }
                if clip.source_id.is_none() && clip.duration.is_none() && clip.source_in.is_none() {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        "source-less clip requires duration",
                    ));
                }
            }
        }
        let mut ids = HashSet::new();
        for source in &project.sources {
            if !ids.insert(source.id.as_str()) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate source id: {}", source.id),
                ));
            }
        }
        ids.clear();
        for overlay in &project.overlays {
            if let Some(id) = overlay.id() {
                if !ids.insert(id) {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("duplicate overlay id: {id}"),
                    ));
                }
            }
        }
        ids.clear();
        for marker in &project.markers {
            if !ids.insert(marker.id.as_str()) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate marker id: {}", marker.id),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn normalise_command(mut value: Value) -> Result<Value, DomainError> {
        let raw = object_mut(&mut value, "command")
            .map_err(|_| DomainError::new("INVALID_COMMAND", "command must be an object"))?;
        if !raw.contains_key("type") {
            for key in ["command", "kind", "op"] {
                if let Some(candidate) = raw.get(key).and_then(Value::as_str) {
                    raw.insert("type".into(), Value::String(candidate.into()));
                    break;
                }
            }
        }
        let aliases = [
            ("addClip", "add_clip"),
            ("removeClip", "remove_clip"),
            ("deleteClip", "remove_clip"),
            ("insertClip", "add_clip"),
            ("placeClip", "place_clip"),
            ("trimClip", "trim_clip"),
            ("splitClip", "split_clip"),
            ("moveClip", "move_clip"),
            ("updateClip", "update_clip"),
            ("setClip", "set_clip"),
            ("addTrack", "add_track"),
            ("removeTrack", "remove_track"),
            ("deleteTrack", "remove_track"),
            ("insertTrack", "add_track"),
            ("addMarker", "add_marker"),
            ("removeMarker", "remove_marker"),
            ("deleteMarker", "remove_marker"),
            ("insertMarker", "add_marker"),
            ("setMetadata", "set_metadata"),
            ("addSource", "add_source"),
            ("insertSource", "add_source"),
            ("addOverlay", "add_overlay"),
            ("insertOverlay", "add_overlay"),
            ("removeOverlay", "remove_overlay"),
            ("deleteOverlay", "remove_overlay"),
        ];
        if let Some(kind) = raw.get("type").and_then(Value::as_str).map(str::to_owned) {
            if let Some((_, canonical)) = aliases.iter().find(|(alias, _)| *alias == kind) {
                raw.insert("type".into(), Value::String((*canonical).into()));
            }
        }
        if matches!(
            raw.get("type").and_then(Value::as_str),
            Some("update" | "set")
        ) {
            raw.insert("type".into(), Value::String("update_clip".into()));
            if !raw.contains_key("clipId") {
                if let Some(id) = raw.get("id").and_then(Value::as_str) {
                    raw.insert("clipId".into(), Value::String(id.into()));
                }
            }
            if !raw.contains_key("patch") {
                if let Some(changes) = raw.get("changes").cloned() {
                    raw.insert("patch".into(), changes);
                } else if let Some(value) = raw.get("value").cloned() {
                    if let Some(property) = raw.get("property").and_then(Value::as_str) {
                        let mut patch = Map::new();
                        patch.insert(property.into(), value);
                        raw.insert("patch".into(), Value::Object(patch));
                    } else {
                        raw.insert("patch".into(), value);
                    }
                }
            }
        }
        let properties = [
            ("setOpacity", "opacity"),
            ("set_opacity", "opacity"),
            ("setTransform", "transform"),
            ("set_transform", "transform"),
            ("setBlend", "blend"),
            ("set_blend", "blend"),
            ("setEffects", "effects"),
            ("set_effects", "effects"),
            ("setKeyframes", "keyframes"),
            ("set_keyframes", "keyframes"),
            ("setTransitions", "transitions"),
            ("set_transitions", "transitions"),
            ("setTimelineStart", "timelineStart"),
            ("set_timeline_start", "timelineStart"),
        ];
        if let Some(kind) = raw.get("type").and_then(Value::as_str).map(str::to_owned) {
            if let Some((_, property)) = properties.iter().find(|(alias, _)| *alias == kind) {
                raw.insert("type".into(), Value::String("update_clip".into()));
                if !raw.contains_key("clipId") {
                    if let Some(id) = raw.get("id").and_then(Value::as_str) {
                        raw.insert("clipId".into(), Value::String(id.into()));
                    }
                }
                if !raw.contains_key("patch") {
                    let value = raw
                        .get(*property)
                        .or_else(|| raw.get("value"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let mut patch = Map::new();
                    patch.insert((*property).into(), value);
                    raw.insert("patch".into(), Value::Object(patch));
                }
            }
        }
        match raw.get("type").and_then(Value::as_str) {
            Some("place_clip") => {
                set_alias(
                    raw,
                    "timelineFrame",
                    &["timelineFrame", "frame"],
                    "place clip frame",
                )?;
                set_alias(
                    raw,
                    "trackId",
                    &["trackId", "toTrackId"],
                    "place clip track",
                )?;
            }
            Some("trim_clip") => {
                set_alias(
                    raw,
                    "timelineFrame",
                    &["timelineFrame", "frame", "atFrame"],
                    "trim clip frame",
                )?;
            }
            Some("split_clip") => {
                set_alias(
                    raw,
                    "atFrame",
                    &["atFrame", "timelineFrame", "frame"],
                    "split clip frame",
                )?;
                set_alias(
                    raw,
                    "newClipId",
                    &["newClipId", "rightClipId"],
                    "split clip ID",
                )?;
            }
            _ => {}
        }
        Ok(value)
    }

    pub(super) fn parse_command(value: Value) -> Result<Command, DomainError> {
        let mut value = normalise_command(value)?;
        let kind = object(&value, "command")?
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::new("INVALID_COMMAND", "command.type is required"))?
            .to_owned();
        let raw = object_mut(&mut value, "command")?;
        match kind.as_str() {
            "add_track" => {
                let track = raw.remove("track").ok_or_else(|| {
                    DomainError::new("INVALID_COMMAND", "add_track.track is required")
                })?;
                raw.insert("track".into(), normalise_track(track, "INVALID_COMMAND")?);
            }
            "add_clip" | "set_clip" => {
                let clip = raw.remove("clip").ok_or_else(|| {
                    DomainError::new("INVALID_COMMAND", format!("{kind}.clip is required"))
                })?;
                raw.insert("clip".into(), normalise_clip(clip, "INVALID_COMMAND")?);
            }
            "add_source" => {
                let source = raw.remove("source").ok_or_else(|| {
                    DomainError::new("INVALID_COMMAND", "add_source.source is required")
                })?;
                raw.insert(
                    "source".into(),
                    normalise_source(source, "INVALID_COMMAND")?,
                );
            }
            "add_overlay" => {
                let overlay = raw.remove("overlay").ok_or_else(|| {
                    DomainError::new("INVALID_COMMAND", "add_overlay.overlay is required")
                })?;
                let overlay = normalise_overlay(overlay, "INVALID_COMMAND")?;
                if object(&overlay, "overlay")?
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .is_none()
                {
                    return Err(DomainError::new(
                        "INVALID_COMMAND",
                        "add_overlay requires overlay.id",
                    ));
                }
                raw.insert("overlay".into(), overlay);
            }
            "add_marker" => {
                let marker = raw.remove("marker").ok_or_else(|| {
                    DomainError::new("INVALID_COMMAND", "add_marker.marker is required")
                })?;
                raw.insert(
                    "marker".into(),
                    normalise_marker(marker, "INVALID_COMMAND")?,
                );
            }
            "batch" => {
                let commands = raw
                    .get("commands")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        DomainError::new("INVALID_COMMAND", "batch.commands must be an array")
                    })?
                    .clone();
                raw.insert(
                    "commands".into(),
                    Value::Array(
                        commands
                            .into_iter()
                            .map(|child| {
                                parse_command(child).and_then(|command| {
                                    serde_json::to_value(command).map_err(|error| {
                                        DomainError::new("INVALID_COMMAND", error.to_string())
                                    })
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                );
            }
            _ => {}
        }
        let command: Command = deserialize(value, "INVALID_COMMAND")?;
        validate_command(&command)?;
        Ok(command)
    }

    fn validate_command(command: &Command) -> Result<(), DomainError> {
        match command {
            Command::PlaceClip { timeline_frame, .. }
            | Command::TrimClip { timeline_frame, .. }
            | Command::SplitClip {
                at_frame: timeline_frame,
                ..
            } if *timeline_frame < 0 => Err(DomainError::new(
                "INVALID_COMMAND",
                "semantic clip command frame must be nonnegative",
            )),
            Command::SplitClip {
                clip_id,
                new_clip_id,
                ..
            } if clip_id == new_clip_id => Err(DomainError::new(
                "INVALID_COMMAND",
                "split_clip newClipId must differ from clipId",
            )),
            Command::MoveClip {
                index, to_index, ..
            } if index.is_none() && to_index.is_none() => Err(DomainError::new(
                "INVALID_COMMAND",
                "move_clip requires index or toIndex",
            )),
            Command::Batch { commands } => {
                for command in commands {
                    validate_command(command)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(super) fn clip_with_patch(current: &Clip, patch: &JsonObject) -> Result<Clip, DomainError> {
        let mut value = serde_json::to_value(current)
            .map_err(|error| DomainError::new("INVALID_COMMAND", error.to_string()))?;
        let clip_id = {
            let raw = object_mut(&mut value, "clip")?;
            for (key, value) in patch {
                if matches!(value, JsonValue::Null) {
                    raw.remove(key);
                } else {
                    raw.insert(key.clone(), value.to_wire());
                }
            }
            raw.get("id").cloned().unwrap_or(Value::Null)
        };
        let mut command = Map::new();
        command.insert("type".into(), Value::String("set_clip".into()));
        command.insert("clipId".into(), clip_id);
        command.insert("clip".into(), value);
        parse_command(Value::Object(command)).and_then(|command| match command {
            Command::SetClip { clip, .. } => Ok(clip),
            _ => unreachable!(),
        })
    }
}

pub fn parse_project(value: &Value) -> Result<Project, DomainError> {
    wire::parse_v2(value.clone())
}
pub fn parse_project_json(input: &str) -> Result<Project, DomainError> {
    parse_project(&serde_json::from_str(input)?)
}
pub fn serialize_project(project: &Project) -> Result<Value, DomainError> {
    serde_json::to_value(project)
        .map_err(|error| DomainError::new("SERIALIZE_ERROR", error.to_string()))
}
pub fn serialize_project_json(project: &Project) -> Result<String, DomainError> {
    serde_json::to_string(project)
        .map_err(|error| DomainError::new("SERIALIZE_ERROR", error.to_string()))
}

pub fn parse_v1(value: &Value) -> Result<LegacyTimeline, DomainError> {
    wire::parse_v1(value.clone())
}
pub fn parse_versioned(value: &Value) -> Result<VersionedProject, DomainError> {
    match value.get("version").and_then(Value::as_u64) {
        Some(1) => Ok(VersionedProject::V1(parse_v1(value)?)),
        Some(2) => Ok(VersionedProject::V2(parse_project(value)?)),
        _ => Err(DomainError::new(
            "INVALID_PROJECT",
            "version must be 1 or 2",
        )),
    }
}

pub fn parse_command(value: &Value) -> Result<Command, DomainError> {
    wire::parse_command(value.clone())
}
pub fn serialize_command(command: &Command) -> Result<Value, DomainError> {
    serde_json::to_value(command)
        .map_err(|error| DomainError::new("SERIALIZE_ERROR", error.to_string()))
}

fn track_type_for_id(id: &str) -> TrackType {
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

/// Convert a validated v1 sequential timeline to the explicit-placement v2
/// project used by the canonical reducer.
pub fn migrate_v1_to_v2(legacy: &LegacyTimeline) -> Result<Project, DomainError> {
    if legacy.version != 1 {
        return Err(DomainError::new(
            "INVALID_TIMELINE",
            "timeline.version must be 1",
        ));
    }
    let sources = legacy
        .sources
        .iter()
        .map(|source| Source {
            id: source.id.clone(),
            path: source.path.clone(),
            subs: source.subs.clone(),
            duration: source.duration,
            width: source.width,
            height: source.height,
            has_audio: source.has_audio,
            metadata: None,
            extensions: source.extensions.clone(),
        })
        .collect();
    let tracks = legacy
        .tracks
        .iter()
        .map(|track| {
            let mut timeline_start = 0.0;
            let clips = track
                .clips
                .iter()
                .map(|clip| {
                    let result = Clip {
                        id: clip.id.clone(),
                        source_id: Some(clip.src.clone()),
                        source_in: Some(clip.source_in),
                        source_out: Some(clip.source_out),
                        timeline_start,
                        duration: None,
                        speed: clip.speed,
                        volume: clip.volume,
                        opacity: None,
                        crop: clip.crop.clone(),
                        transform: None,
                        blend: None,
                        blend_mode: None,
                        effects: None,
                        keyframes: None,
                        transitions: None,
                        gaps: None,
                        text: None,
                        title: None,
                        extensions: JsonObject::new(),
                    };
                    timeline_start +=
                        (clip.source_out - clip.source_in) / clip.speed.unwrap_or(1.0);
                    result
                })
                .collect();
            Track {
                id: track.id.clone(),
                kind: track_type_for_id(track.id.as_str()),
                name: None,
                enabled: None,
                muted: None,
                locked: None,
                clips,
                gaps: None,
                transitions: None,
                effects: None,
                markers: None,
                extensions: JsonObject::new(),
            }
        })
        .collect();
    let project = Project {
        version: 2,
        id: None,
        name: None,
        fps: legacy.fps,
        width: None,
        height: None,
        duration: None,
        sources,
        tracks,
        overlays: legacy.overlays.clone(),
        gaps: Vec::new(),
        transitions: Vec::new(),
        effects: Vec::new(),
        markers: Vec::new(),
        metadata: None,
        extensions: JsonObject::new(),
    };
    validate_project(&project)?;
    Ok(project)
}

pub fn project_duration(project: &Project) -> f64 {
    let mut duration = project
        .duration
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0);
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(clip_duration) = clip
                .timeline_duration()
                .filter(|value| value.is_finite() && *value > 0.0)
            {
                duration = duration.max(clip.timeline_start + clip_duration);
            }
        }
    }
    for gap in &project.gaps {
        if gap.duration.is_finite() && gap.duration > 0.0 {
            duration = duration.max(gap.start + gap.duration);
        }
    }
    duration
}

fn validate_project(project: &Project) -> Result<(), DomainError> {
    if project.version != 2 {
        return Err(DomainError::new(
            "INVALID_PROJECT",
            "project.version must be 2",
        ));
    }
    if !project.fps.is_finite() || project.fps <= 0.0 {
        return Err(DomainError::new(
            "INVALID_PROJECT",
            "project.fps must be positive",
        ));
    }
    let source_ids: HashSet<&str> = project
        .sources
        .iter()
        .map(|source| source.id.as_str())
        .collect();
    let mut ids = HashSet::new();
    for source in &project.sources {
        if !ids.insert(source.id.as_str()) {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                format!("duplicate source id: {}", source.id),
            ));
        }
        if source.path.is_empty() {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                "source.path must be a string",
            ));
        }
        if source.duration.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                "source.duration must be nonnegative",
            ));
        }
    }
    ids.clear();
    let mut clip_ids = HashSet::new();
    for track in &project.tracks {
        if !ids.insert(track.id.as_str()) {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                format!("duplicate track id: {}", track.id),
            ));
        }
        for clip in &track.clips {
            if !clip_ids.insert(clip.id.as_str()) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate clip id: {}", clip.id),
                ));
            }
            if !clip.timeline_start.is_finite() || clip.timeline_start < 0.0 {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip.timelineStart must be nonnegative",
                ));
            }
            let has_in = clip.source_in.is_some();
            let has_out = clip.source_out.is_some();
            if has_in != has_out {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip source interval requires both in and out",
                ));
            }
            if let (Some(input), Some(output)) = (clip.source_in, clip.source_out) {
                if !input.is_finite() || !output.is_finite() || input < 0.0 || output <= input {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        "clip out must be greater than in",
                    ));
                }
            }
            if clip.duration.is_some_and(|v| !v.is_finite() || v <= 0.0) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip.duration must be positive",
                ));
            }
            if clip.speed.is_some_and(|v| !v.is_finite() || v <= 0.0) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip.speed must be positive",
                ));
            }
            if clip.volume.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip.volume must be nonnegative",
                ));
            }
            if clip
                .opacity
                .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "clip.opacity must be between 0 and 1",
                ));
            }
            if matches!(track.kind, TrackType::Video | TrackType::Audio) && clip.source_id.is_none()
            {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("{} clip requires a sourceId", track.kind.as_str()),
                ));
            }
            if let Some(source_id) = &clip.source_id {
                if !source_ids.contains(source_id.as_str()) {
                    return Err(DomainError::new(
                        "INVALID_PROJECT",
                        format!("clip references unknown source: {source_id}"),
                    ));
                }
                if let (Some(output), Some(source_duration)) = (
                    clip.source_out,
                    project
                        .sources
                        .iter()
                        .find(|source| source.id == *source_id)
                        .and_then(|source| source.duration),
                ) {
                    if output > source_duration {
                        return Err(DomainError::new(
                            "INVALID_PROJECT",
                            format!("clip out exceeds source duration {source_duration}"),
                        ));
                    }
                }
            }
            if clip.source_id.is_none() && clip.duration.is_none() && !has_in {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    "source-less clip requires duration",
                ));
            }
        }
    }
    ids.clear();
    for overlay in &project.overlays {
        if let Some(id) = overlay.id() {
            if !ids.insert(id) {
                return Err(DomainError::new(
                    "INVALID_PROJECT",
                    format!("duplicate overlay id: {id}"),
                ));
            }
        }
    }
    ids.clear();
    for marker in &project.markers {
        if !ids.insert(marker.id.as_str()) {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                format!("duplicate marker id: {}", marker.id),
            ));
        }
        if !marker.time.is_finite() || marker.time < 0.0 {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                "marker.time must be nonnegative",
            ));
        }
    }
    for gap in &project.gaps {
        if !gap.start.is_finite()
            || gap.start < 0.0
            || !gap.duration.is_finite()
            || gap.duration <= 0.0
        {
            return Err(DomainError::new(
                "INVALID_PROJECT",
                "gap must have nonnegative start and positive duration",
            ));
        }
    }
    Ok(())
}

pub fn validate(project: &Project) -> Result<(), DomainError> {
    validate_project(project)
}

fn track_index(project: &Project, id: &TrackId) -> Result<usize, DomainError> {
    let matches: Vec<usize> = project
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| (track.id == *id).then_some(index))
        .collect();
    match matches.as_slice() {
        [] => Err(DomainError::new(
            "NOT_FOUND",
            format!("track not found: {id}"),
        )),
        [index] => Ok(*index),
        _ => Err(DomainError::new(
            "AMBIGUOUS_ID",
            format!("ambiguous track id: {id}"),
        )),
    }
}

fn clip_location(
    project: &Project,
    id: &ClipId,
    track_id: Option<&TrackId>,
) -> Result<(usize, usize), DomainError> {
    let mut matches = Vec::new();
    for (track_index, track) in project.tracks.iter().enumerate() {
        if track_id.is_some_and(|wanted| wanted != &track.id) {
            continue;
        }
        for (clip_index, clip) in track.clips.iter().enumerate() {
            if clip.id == *id {
                matches.push((track_index, clip_index));
            }
        }
    }
    match matches.as_slice() {
        [] => Err(DomainError::new(
            "NOT_FOUND",
            format!("clip not found: {id}"),
        )),
        [match_] => Ok(*match_),
        _ => Err(DomainError::new(
            "AMBIGUOUS_ID",
            format!("ambiguous clip id: {id}"),
        )),
    }
}

pub fn apply_command(project: &Project, command: &Command) -> Result<Project, DomainError> {
    let mut next = project.clone();
    apply_command_mut(&mut next, command)?;
    validate_project(&next)?;
    Ok(next)
}

/// Compute the typed command that returns `project` to its previous value
/// after `command` is applied.  The inverse contains only domain values and
/// can therefore be journaled without retaining full project snapshots.
pub fn inverse_command(project: &Project, command: &Command) -> Result<Command, DomainError> {
    match command {
        Command::AddTrack { track, .. } => Ok(Command::RemoveTrack {
            track_id: track.id.clone(),
        }),
        Command::RemoveTrack { track_id } => {
            let index = track_index(project, track_id)?;
            Ok(Command::AddTrack {
                track: project.tracks[index].clone(),
                index: Some(index),
            })
        }
        Command::AddClip { track_id, clip, .. } => Ok(Command::RemoveClip {
            clip_id: clip.id.clone(),
            track_id: Some(track_id.clone()),
        }),
        Command::RemoveClip { clip_id, track_id } => {
            let (track, index) = clip_location(project, clip_id, track_id.as_ref())?;
            Ok(Command::AddClip {
                track_id: project.tracks[track].id.clone(),
                clip: project.tracks[track].clips[index].clone(),
                index: Some(index),
            })
        }
        Command::PlaceClip { clip_id, .. } => {
            let (source_track, source_index) = clip_location(project, clip_id, None)?;
            let before = project.tracks[source_track].clips[source_index].clone();
            let after = apply_command(project, command)?;
            let (destination_track, _) = clip_location(&after, clip_id, None)?;
            Ok(Command::Batch {
                commands: vec![
                    Command::RemoveClip {
                        clip_id: clip_id.clone(),
                        track_id: Some(after.tracks[destination_track].id.clone()),
                    },
                    Command::AddClip {
                        track_id: project.tracks[source_track].id.clone(),
                        clip: before,
                        index: Some(source_index),
                    },
                ],
            })
        }
        Command::TrimClip { clip_id, .. } => {
            let (track, index) = clip_location(project, clip_id, None)?;
            Ok(Command::SetClip {
                clip_id: clip_id.clone(),
                clip: project.tracks[track].clips[index].clone(),
            })
        }
        Command::SplitClip {
            clip_id,
            new_clip_id,
            ..
        } => {
            let (track, index) = clip_location(project, clip_id, None)?;
            let after = apply_command(project, command)?;
            let (new_track, _) = clip_location(&after, new_clip_id, None)?;
            Ok(Command::Batch {
                commands: vec![
                    Command::RemoveClip {
                        clip_id: new_clip_id.clone(),
                        track_id: Some(after.tracks[new_track].id.clone()),
                    },
                    Command::SetClip {
                        clip_id: clip_id.clone(),
                        clip: project.tracks[track].clips[index].clone(),
                    },
                ],
            })
        }
        Command::MoveClip { clip_id, .. } => {
            let (source_track, source_index) = clip_location(project, clip_id, None)?;
            let after = apply_command(project, command)?;
            let (destination_track, destination_index) = clip_location(&after, clip_id, None)?;
            Ok(Command::MoveClip {
                clip_id: clip_id.clone(),
                track_id: Some(after.tracks[destination_track].id.clone()),
                to_track_id: Some(project.tracks[source_track].id.clone()),
                index: Some(destination_index),
                to_index: Some(source_index),
            })
        }
        Command::UpdateClip { clip_id, .. } | Command::SetClip { clip_id, .. } => {
            let (track, index) = clip_location(project, clip_id, None)?;
            Ok(Command::SetClip {
                clip_id: clip_id.clone(),
                clip: project.tracks[track].clips[index].clone(),
            })
        }
        Command::AddMarker { marker, .. } => Ok(Command::RemoveMarker {
            marker_id: marker.id.clone(),
        }),
        Command::RemoveMarker { marker_id } => {
            let index = project
                .markers
                .iter()
                .position(|marker| marker.id == *marker_id)
                .ok_or_else(|| {
                    DomainError::new("NOT_FOUND", format!("marker not found: {marker_id}"))
                })?;
            Ok(Command::AddMarker {
                marker: project.markers[index].clone(),
                index: Some(index),
            })
        }
        Command::SetMetadata { .. } => Ok(Command::RestoreMetadata {
            metadata: project.metadata.clone(),
        }),
        Command::RestoreMetadata { .. } => Ok(Command::RestoreMetadata {
            metadata: project.metadata.clone(),
        }),
        Command::AddSource { source, .. } => Ok(Command::RemoveSource {
            source_id: source.id.clone(),
        }),
        Command::RemoveSource { source_id } => {
            let index = project
                .sources
                .iter()
                .position(|source| source.id == *source_id)
                .ok_or_else(|| {
                    DomainError::new("NOT_FOUND", format!("source not found: {source_id}"))
                })?;
            Ok(Command::AddSource {
                source: project.sources[index].clone(),
                index: Some(index),
            })
        }
        Command::AddOverlay { overlay, .. } => {
            let overlay_id = overlay.id().ok_or_else(|| {
                DomainError::new("INVALID_COMMAND", "add_overlay requires overlay.id")
            })?;
            Ok(Command::RemoveOverlay {
                overlay_id: overlay_id.to_owned(),
            })
        }
        Command::RemoveOverlay { overlay_id } => {
            let index = project
                .overlays
                .iter()
                .position(|overlay| overlay.id() == Some(overlay_id.as_str()))
                .ok_or_else(|| {
                    DomainError::new("NOT_FOUND", format!("overlay not found: {overlay_id}"))
                })?;
            Ok(Command::AddOverlay {
                overlay: project.overlays[index].clone(),
                index: Some(index),
            })
        }
        Command::Batch { commands } => {
            let mut evolving = project.clone();
            let mut inverses = Vec::with_capacity(commands.len());
            for command in commands {
                let inverse = inverse_command(&evolving, command)?;
                evolving = apply_command(&evolving, command)?;
                inverses.push(inverse);
            }
            inverses.reverse();
            Ok(Command::Batch { commands: inverses })
        }
    }
}

pub fn apply_commands(project: &Project, commands: &[Command]) -> Result<Project, DomainError> {
    if commands.is_empty() {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "commands must not be empty",
        ));
    }
    let mut next = project.clone();
    for command in commands {
        apply_command_mut(&mut next, command)?;
    }
    validate_project(&next)?;
    Ok(next)
}

fn apply_command_mut(project: &mut Project, command: &Command) -> Result<(), DomainError> {
    match command {
        Command::AddTrack { track, index } => {
            if project
                .tracks
                .iter()
                .any(|candidate| candidate.id == track.id)
            {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("track already exists: {}", track.id),
                ));
            }
            let destination = index.unwrap_or(project.tracks.len());
            if destination > project.tracks.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("add_track index out of range: {destination}"),
                ));
            }
            project.tracks.insert(destination, track.clone());
        }
        Command::RemoveTrack { track_id } => {
            let index = track_index(project, track_id)?;
            project.tracks.remove(index);
        }
        Command::AddClip {
            track_id,
            clip,
            index,
        } => {
            if project
                .tracks
                .iter()
                .any(|track| track.clips.iter().any(|candidate| candidate.id == clip.id))
            {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("clip already exists: {}", clip.id),
                ));
            }
            let track_index = track_index(project, track_id)?;
            let track = &mut project.tracks[track_index];
            let destination = index.unwrap_or(track.clips.len());
            if destination > track.clips.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("add_clip index out of range: {destination}"),
                ));
            }
            track.clips.insert(destination, clip.clone());
        }
        Command::RemoveClip { clip_id, track_id } => {
            let (track, clip) = clip_location(project, clip_id, track_id.as_ref())?;
            project.tracks[track].clips.remove(clip);
        }
        Command::PlaceClip {
            clip_id,
            timeline_frame,
            track_id,
        } => place_clip_mut(project, clip_id, *timeline_frame, track_id.as_ref())?,
        Command::TrimClip {
            clip_id,
            edge,
            timeline_frame,
        } => trim_clip_mut(project, clip_id, *edge, *timeline_frame)?,
        Command::SplitClip {
            clip_id,
            at_frame,
            new_clip_id,
        } => split_clip_mut(project, clip_id, *at_frame, new_clip_id)?,
        Command::MoveClip {
            clip_id,
            track_id,
            to_track_id,
            index,
            to_index,
        } => {
            let destination = to_index.or(*index).ok_or_else(|| {
                DomainError::new("INVALID_COMMAND", "move_clip requires index or toIndex")
            })?;
            let (source_track, source_index) = clip_location(project, clip_id, None)?;
            let clip = project.tracks[source_track].clips.remove(source_index);
            let target_id = to_track_id
                .as_ref()
                .or(track_id.as_ref())
                .cloned()
                .unwrap_or_else(|| project.tracks[source_track].id.clone());
            let target_track = track_index(project, &target_id)?;
            if destination > project.tracks[target_track].clips.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    "move_clip index is out of range",
                ));
            }
            project.tracks[target_track].clips.insert(destination, clip);
        }
        Command::UpdateClip { clip_id, patch } => {
            let (track, index) = clip_location(project, clip_id, None)?;
            let current = project.tracks[track].clips[index].clone();
            if let Some(value) = patch.get("id") {
                if value != &JsonValue::String(clip_id.to_string()) {
                    return Err(DomainError::new(
                        "INVALID_COMMAND",
                        "update_clip cannot change clip id",
                    ));
                }
            }
            project.tracks[track].clips[index] = wire::clip_with_patch(&current, patch)?;
        }
        Command::SetClip { clip_id, clip } => {
            if clip.id != *clip_id {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("set_clip id mismatch: {} !== {clip_id}", clip.id),
                ));
            }
            let (track, index) = clip_location(project, clip_id, None)?;
            project.tracks[track].clips[index] = clip.clone();
        }
        Command::AddMarker { marker, index } => {
            if project
                .markers
                .iter()
                .any(|candidate| candidate.id == marker.id)
            {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("marker already exists: {}", marker.id),
                ));
            }
            let destination = index.unwrap_or(project.markers.len());
            if destination > project.markers.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("add_marker index out of range: {destination}"),
                ));
            }
            project.markers.insert(destination, marker.clone());
        }
        Command::RemoveMarker { marker_id } => {
            let index = project
                .markers
                .iter()
                .position(|marker| marker.id == *marker_id)
                .ok_or_else(|| {
                    DomainError::new("NOT_FOUND", format!("marker not found: {marker_id}"))
                })?;
            project.markers.remove(index);
        }
        Command::SetMetadata { patch } => {
            let metadata = project.metadata.get_or_insert_with(JsonObject::new);
            metadata.extend(patch.clone());
        }
        Command::RestoreMetadata { metadata } => {
            project.metadata = metadata.clone();
        }
        Command::AddSource { source, index } => {
            if project
                .sources
                .iter()
                .any(|candidate| candidate.id == source.id)
            {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("source already exists: {}", source.id),
                ));
            }
            let destination = index.unwrap_or(project.sources.len());
            if destination > project.sources.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("add_source index out of range: {destination}"),
                ));
            }
            project.sources.insert(destination, source.clone());
        }
        Command::RemoveSource { source_id } => {
            let index = project
                .sources
                .iter()
                .position(|source| source.id == *source_id)
                .ok_or_else(|| {
                    DomainError::new("NOT_FOUND", format!("source not found: {source_id}"))
                })?;
            project.sources.remove(index);
        }
        Command::AddOverlay { overlay, index } => {
            let id = overlay.id().filter(|id| !id.is_empty()).ok_or_else(|| {
                DomainError::new("INVALID_COMMAND", "add_overlay requires overlay.id")
            })?;
            if project
                .overlays
                .iter()
                .any(|candidate| candidate.id() == Some(id))
            {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("overlay already exists: {id}"),
                ));
            }
            let destination = index.unwrap_or(project.overlays.len());
            if destination > project.overlays.len() {
                return Err(DomainError::new(
                    "INVALID_COMMAND",
                    format!("add_overlay index out of range: {destination}"),
                ));
            }
            project.overlays.insert(destination, overlay.clone());
        }
        Command::RemoveOverlay { overlay_id } => {
            let matches: Vec<usize> = project
                .overlays
                .iter()
                .enumerate()
                .filter_map(|(index, overlay)| {
                    (overlay.id() == Some(overlay_id.as_str())).then_some(index)
                })
                .collect();
            match matches.as_slice() {
                [] => {
                    return Err(DomainError::new(
                        "NOT_FOUND",
                        format!("overlay not found: {overlay_id}"),
                    ))
                }
                [index] => {
                    project.overlays.remove(*index);
                }
                _ => {
                    return Err(DomainError::new(
                        "AMBIGUOUS_ID",
                        format!("ambiguous overlay id: {overlay_id}"),
                    ))
                }
            }
        }
        Command::Batch { commands } => {
            for command in commands {
                apply_command_mut(project, command)?;
            }
        }
    }
    Ok(())
}

fn project_frame_grid(project: &Project) -> Result<FrameGrid, DomainError> {
    FrameGrid::new(project.fps).ok_or_else(|| {
        DomainError::new("INVALID_PROJECT", "project.fps must be finite and positive")
    })
}

fn clip_duration(project: &Project, clip: &Clip) -> Result<f64, DomainError> {
    if let Some(duration) = clip.timeline_duration() {
        if duration.is_finite() && duration > 0.0 {
            return Ok(duration);
        }
    }
    let speed = clip.speed.unwrap_or(1.0);
    if !speed.is_finite() || speed <= 0.0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            format!("clip {} speed must be positive", clip.id),
        ));
    }
    let source_duration = clip
        .source_id
        .as_ref()
        .and_then(|source_id| {
            project
                .sources
                .iter()
                .find(|source| source.id == *source_id)
        })
        .and_then(|source| source.duration);
    let duration =
        source_duration.map(|duration| (duration - clip.source_in.unwrap_or(0.0)) / speed);
    duration
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| {
            DomainError::new(
                "INVALID_COMMAND",
                format!("clip {} has no positive timeline duration", clip.id),
            )
        })
}

fn clip_location_by_id(project: &Project, clip_id: &ClipId) -> Result<(usize, usize), DomainError> {
    clip_location(project, clip_id, None)
}

fn place_clip_mut(
    project: &mut Project,
    clip_id: &ClipId,
    timeline_frame: FrameIndex,
    target_track_id: Option<&TrackId>,
) -> Result<(), DomainError> {
    if timeline_frame < 0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "place_clip frame must be nonnegative",
        ));
    }
    let grid = project_frame_grid(project)?;
    let (source_track, source_index) = clip_location_by_id(project, clip_id)?;
    let source_track_id = project.tracks[source_track].id.clone();
    let target_track_id = target_track_id.unwrap_or(&source_track_id).clone();
    track_index(project, &target_track_id)?;

    let mut clip = project.tracks[source_track].clips.remove(source_index);
    clip.timeline_start = grid.seconds_at(timeline_frame);
    let target_track = track_index(project, &target_track_id)?;
    let destination = if target_track_id == source_track_id {
        source_index.min(project.tracks[target_track].clips.len())
    } else {
        project.tracks[target_track].clips.len()
    };
    project.tracks[target_track].clips.insert(destination, clip);
    Ok(())
}

fn trim_clip_mut(
    project: &mut Project,
    clip_id: &ClipId,
    edge: TrimEdge,
    timeline_frame: FrameIndex,
) -> Result<(), DomainError> {
    if timeline_frame < 0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "trim_clip frame must be nonnegative",
        ));
    }
    let grid = project_frame_grid(project)?;
    let (track, index) = clip_location_by_id(project, clip_id)?;
    let current = project.tracks[track].clips[index].clone();
    let duration = clip_duration(project, &current)?;
    let start = current.timeline_start;
    let end = start + duration;
    let target = grid.seconds_at(timeline_frame);
    let new_duration = match edge {
        TrimEdge::Leading if target > start && target < end => end - target,
        TrimEdge::Trailing if target > start && target <= end => target - start,
        TrimEdge::Leading | TrimEdge::Trailing => {
            return Err(DomainError::new(
                "INVALID_COMMAND",
                "trim edge must stay within the clip and leave a positive duration",
            ))
        }
    };
    let speed = current.speed.unwrap_or(1.0);
    if !speed.is_finite() || speed <= 0.0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "clip speed must be positive",
        ));
    }

    let has_source = current.source_id.is_some();
    let source_in = current.source_in.unwrap_or(0.0);
    let source_out = current.source_out.unwrap_or(source_in + duration * speed);
    let mut trimmed = current;
    trimmed.timeline_start = match edge {
        TrimEdge::Leading => target,
        TrimEdge::Trailing => start,
    };
    if has_source {
        match edge {
            TrimEdge::Leading => {
                trimmed.source_in = Some(source_in + (target - start) * speed);
                trimmed.source_out = Some(source_out);
            }
            TrimEdge::Trailing => {
                trimmed.source_in = Some(source_in);
                trimmed.source_out = Some(source_in + new_duration * speed);
            }
        }
    }
    if trimmed.duration.is_some() || !has_source {
        trimmed.duration = Some(new_duration);
    }
    trim_keyframes(
        &mut trimmed,
        edge,
        if edge == TrimEdge::Leading {
            target - start
        } else {
            0.0
        },
        new_duration,
    );
    project.tracks[track].clips[index] = trimmed;
    Ok(())
}

fn split_clip_mut(
    project: &mut Project,
    clip_id: &ClipId,
    at_frame: FrameIndex,
    new_clip_id: &ClipId,
) -> Result<(), DomainError> {
    if at_frame < 0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "split_clip frame must be nonnegative",
        ));
    }
    if clip_id == new_clip_id {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "split_clip newClipId must differ from clipId",
        ));
    }
    let grid = project_frame_grid(project)?;
    let (track, index) = clip_location_by_id(project, clip_id)?;
    if project
        .tracks
        .iter()
        .any(|track| track.clips.iter().any(|clip| clip.id == *new_clip_id))
    {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            format!("clip already exists: {new_clip_id}"),
        ));
    }
    let current = project.tracks[track].clips[index].clone();
    let duration = clip_duration(project, &current)?;
    let start = current.timeline_start;
    let end = start + duration;
    let split_time = grid.seconds_at(at_frame);
    if split_time <= start || split_time >= end {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "split frame must be strictly inside the clip",
        ));
    }
    let left_duration = split_time - start;
    let right_duration = end - split_time;
    let speed = current.speed.unwrap_or(1.0);
    if !speed.is_finite() || speed <= 0.0 {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "clip speed must be positive",
        ));
    }
    let has_source = current.source_id.is_some();
    let source_in = current.source_in.unwrap_or(0.0);
    let source_out = current.source_out.unwrap_or(source_in + duration * speed);

    let mut left = current.clone();
    let mut right = current;
    right.id = new_clip_id.clone();
    right.timeline_start = split_time;
    if has_source {
        let split_source = source_in + left_duration * speed;
        left.source_in = Some(source_in);
        left.source_out = Some(split_source);
        right.source_in = Some(split_source);
        right.source_out = Some(source_out);
    }
    if left.duration.is_some() || !has_source {
        left.duration = Some(left_duration);
        right.duration = Some(right_duration);
    }
    split_keyframes(&mut left, &mut right, left_duration);

    project.tracks[track].clips[index] = left;
    project.tracks[track].clips.insert(index + 1, right);
    Ok(())
}

fn split_keyframes(left: &mut Clip, right: &mut Clip, split_time: f64) {
    let Some(keyframes) = left.keyframes.take() else {
        return;
    };
    let mut left_keyframes = Vec::new();
    let mut right_keyframes = Vec::new();
    for keyframe in keyframes {
        if keyframe.time <= split_time {
            left_keyframes.push(keyframe.clone());
        }
        if keyframe.time >= split_time {
            let mut shifted = keyframe;
            shifted.time -= split_time;
            right_keyframes.push(shifted);
        }
    }
    left.keyframes = Some(left_keyframes);
    right.keyframes = Some(right_keyframes);
}

fn trim_keyframes(clip: &mut Clip, edge: TrimEdge, removed_time: f64, duration: f64) {
    let Some(keyframes) = clip.keyframes.take() else {
        return;
    };
    let mut retained = Vec::new();
    for mut keyframe in keyframes {
        if edge == TrimEdge::Leading {
            if keyframe.time + 1e-9 < removed_time {
                continue;
            }
            keyframe.time = (keyframe.time - removed_time).max(0.0);
        }
        if keyframe.time <= duration + 1e-9 {
            retained.push(keyframe);
        }
    }
    clip.keyframes = Some(retained);
}

pub fn empty_project() -> Project {
    Project {
        version: 2,
        id: None,
        name: Some("Untitled project".into()),
        fps: 30.0,
        width: None,
        height: None,
        duration: None,
        sources: Vec::new(),
        tracks: vec![
            Track {
                id: TrackId::new("video-1").expect("static id"),
                kind: TrackType::Video,
                name: None,
                enabled: None,
                muted: None,
                locked: None,
                clips: Vec::new(),
                gaps: None,
                transitions: None,
                effects: None,
                markers: None,
                extensions: JsonObject::new(),
            },
            Track {
                id: TrackId::new("audio-1").expect("static id"),
                kind: TrackType::Audio,
                name: None,
                enabled: None,
                muted: None,
                locked: None,
                clips: Vec::new(),
                gaps: None,
                transitions: None,
                effects: None,
                markers: None,
                extensions: JsonObject::new(),
            },
        ],
        overlays: Vec::new(),
        gaps: Vec::new(),
        transitions: Vec::new(),
        effects: Vec::new(),
        markers: Vec::new(),
        metadata: None,
        extensions: JsonObject::new(),
    }
}

pub fn create_history(project: Project, revision: Revision) -> Result<History, DomainError> {
    validate_project(&project)?;
    Ok(History {
        project,
        revision,
        past: Vec::new(),
        future: Vec::new(),
    })
}

pub fn apply_history_command(
    history: &History,
    command: &Command,
    expected: Option<Revision>,
) -> Result<History, DomainError> {
    check_revision(history, expected)?;
    let after = apply_command(&history.project, command)?;
    let entry = HistoryEntry {
        command: command.clone(),
        before: history.project.clone(),
        after: after.clone(),
    };
    Ok(History {
        project: after,
        revision: Revision::new(history.revision.get() + 1),
        past: history
            .past
            .iter()
            .cloned()
            .chain(std::iter::once(entry))
            .collect(),
        future: Vec::new(),
    })
}

pub fn apply_history_commands(
    history: &History,
    commands: &[Command],
    expected: Option<Revision>,
) -> Result<History, DomainError> {
    check_revision(history, expected)?;
    if commands.is_empty() {
        return Err(DomainError::new(
            "INVALID_COMMAND",
            "commands must not be empty",
        ));
    }
    let after = apply_commands(&history.project, commands)?;
    let command = Command::Batch {
        commands: commands.to_vec(),
    };
    let entry = HistoryEntry {
        command,
        before: history.project.clone(),
        after: after.clone(),
    };
    Ok(History {
        project: after,
        revision: Revision::new(history.revision.get() + 1),
        past: history
            .past
            .iter()
            .cloned()
            .chain(std::iter::once(entry))
            .collect(),
        future: Vec::new(),
    })
}

pub fn undo(history: &History, expected: Option<Revision>) -> Result<History, DomainError> {
    check_revision(history, expected)?;
    let Some(entry) = history.past.last() else {
        return Ok(history.clone());
    };
    let mut past = history.past.clone();
    past.pop();
    let mut future = history.future.clone();
    future.push(entry.clone());
    Ok(History {
        project: entry.before.clone(),
        revision: Revision::new(history.revision.get() + 1),
        past,
        future,
    })
}

pub fn redo(history: &History, expected: Option<Revision>) -> Result<History, DomainError> {
    check_revision(history, expected)?;
    let Some(entry) = history.future.last() else {
        return Ok(history.clone());
    };
    let mut future = history.future.clone();
    future.pop();
    let mut past = history.past.clone();
    past.push(entry.clone());
    Ok(History {
        project: entry.after.clone(),
        revision: Revision::new(history.revision.get() + 1),
        past,
        future,
    })
}

fn check_revision(history: &History, expected: Option<Revision>) -> Result<(), DomainError> {
    if let Some(expected) = expected {
        if expected != history.revision {
            return Err(DomainError::new(
                "REVISION_CONFLICT",
                format!(
                    "revision conflict: expected {}, actual {}",
                    expected.get(),
                    history.revision.get()
                ),
            ));
        }
    }
    Ok(())
}

/// Compatibility names make the domain crate convenient for Rust callers
/// migrating from the original timeline wire format.
pub use apply_history_command as apply_project_command;
pub use apply_history_commands as apply_project_commands;
pub use create_history as create_project_history;
pub use parse_versioned as parse_timeline_project;
