//! The timeline view that editor snapshots carry: a read-only picture of the
//! model for clients that draw a timeline (the editor).
//!
//! Times are exact seconds of the model, with frame counts on the first
//! sequence's rate beside them; clips appear at their own spans, and
//! transitions are listed separately with the region they cover. Edits go
//! back as typed operations ([`EditOp`](crate::EditOp)), never by patching this.
//!
//! Every type reads leniently: a key that is absent reads as its default.
//! A parameter's definition is a [`ViewParamDef`], and the effects,
//! transitions and generators of the catalog are [`ViewFxDef`]s; a
//! timeline's definitions that do not read as one (a newer kerfd's) are
//! left out, and the rest of it still reads. A parameter's value is a
//! [`ViewParam`]: a static value, or [`ViewKey`]s when animated. Each key has
//! its clip-relative `at` in seconds, the same offset as a whole `frame` on
//! the sequence rate (what an [`EditKey`](crate::EditKey)'s `at` takes back),
//! its `value` and its `interp`. A value itself stays JSON (a number, a bool, a
//! text, or a color as `#rrggbb[aa]`): which it is, its definition's `type`
//! says. An expression shows as what it gives at the clip's first frame.
//! Anything absent is at its default.

use kerf_model::{Interp, Rgba};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// A project as a timeline shows it: its media, its sequences, the first
/// sequence's tracks and markers, and the catalog definitions its clips'
/// parameters are read by.
///
/// `sequence`, `rate`, `dropFrame`, `sampleRate`, `width`, `height` and
/// `start` describe the first sequence, and are absent when the project has
/// none; `fps` and `duration` are then 30 and 0.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimelineView {
    pub name: String,
    pub sources: Vec<ViewSource>,
    pub sequences: Vec<ViewSequence>,
    /// The catalog's intrinsic clip property definitions, by track type
    /// (`video`, `audio`).
    #[serde(deserialize_with = "defs_by_name")]
    pub properties: BTreeMap<String, Vec<ViewParamDef>>,
    /// The definitions of the properties a mask sets.
    #[serde(deserialize_with = "defs")]
    pub mask_properties: Vec<ViewParamDef>,
    /// The modes a mask can combine by.
    pub mask_modes: Vec<String>,
    /// Each generator's parameter definitions, by its fx id.
    #[serde(deserialize_with = "defs_by_name")]
    pub generators: BTreeMap<String, Vec<ViewParamDef>>,
    pub tracks: Vec<ViewTrack>,
    pub markers: Vec<ViewMarker>,
    /// The first sequence's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    pub fps: f64,
    /// The rate as documents write it: `24`, `29.97`, or `num/den`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_frame: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The timecode of its first frame, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// Where its last clip ends.
    pub duration: f64,
    /// Each media's readable name, by id: filled by a reader that names
    /// media for display (the editor), never read from or written to JSON.
    #[serde(skip)]
    pub media_names: HashMap<String, String>,
}

impl TimelineView {
    /// Frames a second: `fps`, or 30 where it is not a positive number.
    pub fn rate(&self) -> f64 {
        if self.fps.is_finite() && self.fps > 0.0 {
            self.fps
        } else {
            30.0
        }
    }

    /// Clip `id` of the first sequence, and the track it is on.
    pub fn clip(&self, id: &str) -> Option<(&ViewTrack, &ViewClip)> {
        self.tracks.iter().find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == id)
                .map(|clip| (track, clip))
        })
    }

    /// Media `id`.
    pub fn source(&self, id: &str) -> Option<&ViewSource> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// Whether any track of the first sequence has a clip.
    pub fn has_clips(&self) -> bool {
        self.tracks.iter().any(|track| !track.clips.is_empty())
    }

    /// The whole frame nearest `seconds`, at [`TimelineView::rate`].
    pub fn frame_at(&self, seconds: f64) -> i64 {
        (seconds * self.rate()).round() as i64
    }
}

/// A media of the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewSource {
    pub id: String,
    /// Its file, as the document names it.
    pub path: String,
    pub duration: f64,
    /// Its picture rate, or the timecode rate of sound-only media, as
    /// documents write it.
    pub rate: String,
    pub fps: f64,
    pub has_audio: bool,
    /// Its picture's size; absent for sound-only media.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Its sound's channels and sample rate; absent for media without sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    /// The text file another program made its file from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// How many steps the document has under it: a media those make. Absent
    /// (0) for footage.
    #[serde(skip_serializing_if = "is_zero")]
    pub steps: usize,
    /// The takes its recipe has made, in the order they were made. Only a
    /// snapshot's own timeline has them, not a proposal's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<ViewTake>,
    /// The names of the project's cast and styles its recipe names, the cast
    /// first (a name can be in both). Only a snapshot's own timeline has them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shows: Vec<String>,
}

impl ViewSource {
    /// Made from steps: steps in the document, or the recipe file it names.
    pub fn is_made(&self) -> bool {
        self.steps > 0 || self.recipe.is_some()
    }

    /// Made by steps written under its line in the document.
    pub fn steps_in_document(&self) -> bool {
        self.recipe.as_deref().is_none_or(str::is_empty) && self.steps > 0
    }

    /// Its steps are still in the recipe file it names: they are carried
    /// into the document before it is made.
    pub fn uncarried(&self) -> bool {
        self.recipe
            .as_deref()
            .is_some_and(|recipe| !recipe.is_empty())
            && self.steps == 0
    }

    /// It has a picture (a video or a still), not only sound.
    pub fn is_picture(&self) -> bool {
        self.width.is_some()
    }

    /// The take it is now, by number, when it has takes.
    pub fn chosen_take(&self) -> Option<u32> {
        self.takes
            .iter()
            .find(|take| take.chosen)
            .map(|take| take.number)
    }
}

/// A file a media's recipe has made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewTake {
    pub number: u32,
    /// Its path as the document would name it.
    pub path: String,
    /// Whether it is the one the media is.
    pub chosen: bool,
}

/// A sequence of the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewSequence {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Its rate as documents write it.
    pub rate: String,
    pub fps: f64,
    pub drop_frame: bool,
    /// Where its last clip ends.
    pub duration: f64,
    /// What it is finished in: `rec709`, or an HDR signal.
    pub color: String,
}

/// What a track holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewTrackKind {
    #[default]
    Video,
    Audio,
    Subtitle,
}

/// A track of the first sequence.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewTrack {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ViewTrackKind,
    pub enabled: bool,
    pub locked: bool,
    pub muted: bool,
    /// What it carries: the role it was given, or its kind's (`video` for
    /// picture, `dialogue` for sound, null for subtitles).
    pub role: Option<String>,
    /// Whether it was given its role.
    pub role_assigned: bool,
    /// Whether ripple edits on other tracks move it.
    pub sync_locked: bool,
    /// The bus a sound track feeds; null for the sequence's mix.
    pub bus: Option<String>,
    /// Its clips, ordered by start.
    pub clips: Vec<ViewClip>,
    pub transitions: Vec<ViewTransition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A clip, at its own span on the sequence.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewClip {
    pub id: String,
    pub timeline_start: f64,
    pub duration: f64,
    pub start_frame: i64,
    pub duration_frames: i64,
    pub enabled: bool,
    /// What it goes by: its own name, else a nested sequence's name (or id),
    /// a generator's static `text` (or its fx id), or a caption's text. Only
    /// a media clip without a name of its own has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The link group it moves with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The clip it is placed relative to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The clip it follows in time: it moves when that one moves, and ripple
    /// edits pass under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follows: Option<String>,
    /// Where in its source it starts and ends, in seconds; media and nested
    /// sequence clips only, as are `speed` and `speedRatio`.
    #[serde(rename = "in", skip_serializing_if = "Option::is_none")]
    pub source_in: Option<f64>,
    #[serde(rename = "out", skip_serializing_if = "Option::is_none")]
    pub source_out: Option<f64>,
    /// How fast it plays its source on average, without direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// Its speed as documents write it (`1`, `1/2`, `-2`), or `ramp`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_ratio: Option<String>,
    /// It plays its source backwards. Absent otherwise.
    #[serde(skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// `media`, `sequence`, `generator`, or `caption`.
    pub kind: String,
    /// The media a media clip plays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// That media's rate; absent when the project has no such media.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rate: Option<String>,
    /// The sequence a nested sequence clip plays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_id: Option<String>,
    /// A generator's fx and its version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// A caption's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A signal generator's number along the clip, for an editor to draw as
    /// a curve: 0 to 1 to four places, read every few frames (at most 512
    /// points) and on its last frame.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signal: Vec<f64>,
    /// The properties it sets, then a generator's parameters, each a
    /// parameter value. The timeline's `properties` and `generators` define
    /// them.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, ViewParam>,
    /// Its effect stack, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ViewEffect>,
    /// A picture clip's masks, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<ViewMask>,
    /// Its media's readable name, for a clip of media with a file: filled
    /// by a reader that names media for display (the editor), never read
    /// from or written to JSON.
    #[serde(skip)]
    pub media_name: Option<String>,
}

impl ViewClip {
    /// Where it ends, in seconds.
    pub fn end(&self) -> f64 {
        self.timeline_start + self.duration
    }

    /// The frame after its last.
    pub fn end_frame(&self) -> i64 {
        self.start_frame + self.duration_frames
    }

    /// The second of its source shown `offset` seconds into the clip: from
    /// its in point towards its out (which allow for its speed), or back
    /// from its out point when it plays in reverse. None for a clip with no
    /// source range (a title, a generator).
    pub fn source_at(&self, offset: f64) -> Option<f64> {
        let source_in = self.source_in?;
        let speed = self.speed.unwrap_or(1.0);
        let out = self
            .source_out
            .unwrap_or(source_in + self.duration * speed.abs());
        let rate = if self.duration > 0.0 {
            (out - source_in) / self.duration
        } else {
            speed.abs()
        };
        Some(if self.reverse || speed < 0.0 {
            out - offset * rate
        } else {
            source_in + offset * rate
        })
    }
}

/// An effect on a clip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewEffect {
    pub fx: String,
    pub version: u32,
    pub enabled: bool,
    /// The catalog's description of the fx; null where the catalog of the
    /// process that made the view does not have it.
    pub doc: Option<String>,
    /// The parameters it sets, each a parameter value, as the effect catalog
    /// (`kerf_runtime::fx_catalog_all`) defines them.
    pub params: BTreeMap<String, ViewParam>,
}

/// A parameter's value as a view gives it: in JSON the value itself, or an
/// array of its keyframes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViewParam {
    /// Keyframes, in time order.
    Keyed(Vec<ViewKey>),
    /// One value throughout: a number, a bool, a text, or a color as
    /// `#rrggbb[aa]`.
    Value(Value),
}

impl ViewParam {
    /// Its one value; none for keyframes.
    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Value(value) => Some(value),
            Self::Keyed(_) => None,
        }
    }

    /// Its keyframes; none for one value.
    pub fn keys(&self) -> &[ViewKey] {
        match self {
            Self::Keyed(keys) => keys,
            Self::Value(_) => &[],
        }
    }

    /// What it comes to `at` seconds into its clip, as kerf-model's
    /// `Param::value_at` works it out: one value throughout; before the first
    /// keyframe the first's value and from the last on the last's; between
    /// two, the first's curve takes numbers and colors to the next, and holds
    /// anything else. Null for no keyframes.
    pub fn value_at(&self, at: f64) -> Value {
        let keys = match self {
            Self::Value(value) => return value.clone(),
            Self::Keyed(keys) => keys,
        };
        let next = keys.partition_point(|key| key.at <= at);
        match (
            next.checked_sub(1).map(|index| &keys[index]),
            keys.get(next),
        ) {
            (None, Some(first)) => first.value.clone(),
            (Some(last), None) => last.value.clone(),
            (Some(left), Some(right)) => {
                let span = right.at - left.at;
                let t = if span > 0.0 {
                    (at - left.at) / span
                } else {
                    1.0
                };
                interpolate(&left.value, &right.value, left.interp.curve(t))
            }
            (None, None) => Value::Null,
        }
    }

    /// [`ViewParam::value_at`] as a number; none where that is not one.
    pub fn number_at(&self, at: f64) -> Option<f64> {
        self.value_at(at).as_f64()
    }
}

impl From<Value> for ViewParam {
    fn from(value: Value) -> Self {
        Self::Value(value)
    }
}

/// `a` `t` of the way to `b`, `t` already eased, by kerf-model's
/// `Value::interpolate`: numbers and colors move, and anything else holds at
/// `a`.
fn interpolate(a: &Value, b: &Value, t: f64) -> Value {
    let model = |value: &Value| match value {
        Value::Number(number) => number.as_f64().map(kerf_model::Value::Number),
        Value::String(text) => Rgba::parse(text).map(kerf_model::Value::Color),
        _ => None,
    };
    match (model(a), model(b)) {
        (Some(from), Some(to)) => match from.interpolate(&to, t) {
            kerf_model::Value::Number(number) => Value::from(number),
            kerf_model::Value::Color(color) => Value::String(color.to_string()),
            _ => a.clone(),
        },
        _ => a.clone(),
    }
}

/// A keyframe of a [`ViewParam`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewKey {
    /// Seconds from its clip's first frame.
    pub at: f64,
    /// The same offset in whole frames of the sequence's rate.
    pub frame: i64,
    pub value: Value,
    /// How its curve takes the value to the next key's: a name, or a
    /// curve's handles `x1,y1,x2,y2`. One this build cannot read eases
    /// linearly.
    #[serde(with = "interp_name")]
    pub interp: Interp,
}

/// An [`Interp`] as the language writes it.
mod interp_name {
    use super::*;

    pub fn serialize<S: Serializer>(interp: &Interp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&interp.name())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Interp, D::Error> {
        Ok(Value::deserialize(deserializer)?
            .as_str()
            .and_then(Interp::parse)
            .unwrap_or_default())
    }
}

/// A mask on a clip's picture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewMask {
    pub name: String,
    /// How it combines with the masks before it: one of `maskModes`.
    pub mode: String,
    pub inverted: bool,
    /// The properties it sets, each a parameter value, as `maskProperties`
    /// defines them.
    pub params: BTreeMap<String, ViewParam>,
    /// Its outline; a moving one's first keyframe's.
    #[serde(flatten)]
    pub outline: ViewOutline,
    /// A moving outline's keyframes; absent for one that holds still.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<ViewOutlineKey>>,
}

/// A mask's outline as the language writes it, in fractions of the clip's
/// picture (0, 0 its top left corner): one of `rect`, `ellipse`, or `points`
/// with `closed`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewOutline {
    /// Left, top, right, bottom.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
    /// Center x and y, then the radius across and down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ellipse: Option<[f64; 4]>,
    /// Corners in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    /// Whether the corners enclose something; an open outline is a path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    /// How many picture pixels wide the edge's fall-off is at each corner;
    /// absent for none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub soft: Vec<f64>,
}

/// An outline at a time, and how it moves toward the next.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewOutlineKey {
    /// Seconds from the clip's start.
    pub at: f64,
    pub frame: i64,
    pub interp: String,
    #[serde(flatten)]
    pub outline: ViewOutline,
}

/// A parameter's definition, as the catalog gives it: a property of a clip
/// or of a mask, or a parameter of an effect, a transition or a generator.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewParamDef {
    pub name: String,
    /// What it holds: `number`, `time` (seconds), `bool`, `color`, `enum`
    /// (one of `options`), or what is held as text: `text`, `file` (with one
    /// of `extensions`), `track`, `sound`, `mask`, `points` or `moves`.
    #[serde(rename = "type")]
    pub kind: String,
    /// What it is where nothing sets it: a static parameter value.
    pub default: Value,
    /// Whether keyframes may animate it.
    pub animatable: bool,
    pub doc: String,
    /// The least and the most it may be; absent where it has no such limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// What a number counts, such as `frame widths`; absent for a bare one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// An `enum`'s choices, in order; absent for every other type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// The extensions a `file` takes; absent for every other type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

/// An effect, a transition or a generator of the catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewFxDef {
    /// The fx id documents name it by, without its version.
    pub id: String,
    /// `effect`, `transition`, or `generator`.
    pub kind: String,
    /// Where it is listed, such as `Color Correction`.
    pub category: String,
    /// What it is called: its doc up to the first `: `, or without one, its
    /// doc less the periods it ends with.
    pub name: String,
    pub doc: String,
    /// What draws it: `graph`, `raster`, `kerf`, or `look`.
    pub engine: String,
    pub params: Vec<ViewParamDef>,
    /// What it works on: `video` or `audio`.
    pub domain: String,
}

/// A transition across the cut at the end of its `from` clip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewTransition {
    pub id: String,
    pub from: String,
    pub to: String,
    pub fx: String,
    pub version: u32,
    /// Where it sits on the cut: `center`, `start`, or `end`.
    pub align: String,
    /// The region it covers, in seconds.
    pub start: f64,
    pub end: f64,
    pub duration: f64,
}

/// A marker on the first sequence.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewMarker {
    pub id: String,
    pub time: f64,
    pub frame: i64,
    pub duration: f64,
    /// Its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Definitions, those that do not read as a [`ViewParamDef`] left out: a
/// newer kerfd's definition this build cannot read costs that definition,
/// not the timeline. Anything but a list is none.
fn defs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ViewParamDef>, D::Error> {
    Ok(readable_defs(Value::deserialize(deserializer)?))
}

/// Lists of definitions by name, each read as [`defs`] reads one. Anything
/// but an object is none.
fn defs_by_name<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Vec<ViewParamDef>>, D::Error> {
    let Value::Object(lists) = Value::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    Ok(lists
        .into_iter()
        .map(|(name, list)| (name, readable_defs(list)))
        .collect())
}

fn readable_defs(list: Value) -> Vec<ViewParamDef> {
    let Value::Array(items) = list else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect()
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

fn is_false(flag: &bool) -> bool {
    !flag
}
