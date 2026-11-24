//! The typed edits an editor sends: what a gesture or a field changes, in frames and ids.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrimSide {
    In,
    Out,
}

/// A typed edit for the editor. Amounts and positions count frames of the
/// sequence: `frames` is a signed amount, `at` a frame offset from the
/// sequence's first frame. `sourceIn` counts frames of the media. Generated
/// ids follow the language's rules; `ids`/`tail` name new clips explicitly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum EditOp {
    Trim {
        clip: String,
        edge: TrimSide,
        frames: i64,
        #[serde(default)]
        ripple: bool,
    },
    Roll {
        clip: String,
        frames: i64,
    },
    Slip {
        clip: String,
        frames: i64,
    },
    Slide {
        clip: String,
        frames: i64,
    },
    Move {
        clip: String,
        #[serde(default)]
        frames: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
    },
    Split {
        clip: String,
        at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail: Option<String>,
    },
    Blade {
        at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracks: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    Delete {
        clips: Vec<String>,
    },
    RippleDelete {
        clips: Vec<String>,
    },
    /// Cut the time the clips cover out of every unlocked track and close
    /// it: what ripple-delete refuses when it would cut a clip on another
    /// track.
    Extract {
        clips: Vec<String>,
    },
    /// Property or generator parameter values: a static value, keyframes
    /// as `{"keys": [{"at": frames, "value": v, "interp": "linear"}]}` with
    /// `at` counted from the clip's first frame, or `null` to restore the
    /// default. `name` renames the clip; an empty one gives it back the
    /// name of what it shows.
    Set {
        clip: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        props: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Speed {
        clip: String,
        speed: String,
    },
    /// Add an effect to the end of the clip's stack, or at `index`, with
    /// parameter values, static or keyframed; the rest stay at their
    /// defaults.
    FxAdd {
        clip: String,
        fx: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Turn the effect at `index` on or off, or set its parameters, static
    /// or keyframed; `null` restores a parameter's default.
    FxSet {
        clip: String,
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
    },
    FxRemove {
        clip: String,
        index: usize,
    },
    /// Add a mask to a picture clip, after its others or at `index`. Its
    /// name is the first of `k1`, `k2`, … the clip does not use unless
    /// `name` gives one; `params` sets feather, opacity, and expansion.
    MaskAdd {
        clip: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(flatten)]
        outline: MaskOutline,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inverted: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Replace a mask's outline, or change its mode, inversion, or
    /// properties; `null` restores a property's default.
    MaskSet {
        clip: String,
        name: String,
        #[serde(flatten)]
        outline: MaskOutline,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inverted: Option<bool>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Json>,
    },
    MaskRemove {
        clip: String,
        name: String,
    },
    /// Place media or a generator at `at` on each of `tracks`, pushing
    /// what follows later by `frames`.
    Insert {
        /// The media to place, from `sourceIn` (frames of the media); or
        /// else `generator`, a generated picture such as a title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
        #[serde(default)]
        source_in: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorSpec>,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        /// `insert` or `overwrite` in place of the op's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    /// Place media or a generator at `at` on each of `tracks` over what
    /// is there.
    Overwrite {
        /// The media to place, from `sourceIn` (frames of the media); or
        /// else `generator`, a generated picture such as a title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
        #[serde(default)]
        source_in: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorSpec>,
        frames: i64,
        at: i64,
        tracks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        /// `insert` or `overwrite` in place of the op's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    AddMarker {
        at: i64,
        #[serde(default)]
        frames: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// Move, retime, rename, recolour, or annotate the marker `marker`:
    /// `at` is a frame offset from the sequence's first frame and `frames`
    /// its length, as `add-marker` counts them. An empty `name`, `note`, or
    /// `color` clears it; what is absent stays as it is.
    SetMarker {
        marker: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frames: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    RemoveMarker {
        id: String,
    },
    Link {
        clips: Vec<String>,
    },
    Unlink {
        clips: Vec<String>,
    },
    /// `set-track`: a track's switches, its role, and its sync lock. `role`
    /// is one of `video`, `titles`, `dialogue`, `music`, `effects` (any
    /// case) or other one-line text, and an empty one gives back the
    /// kind's own; `syncLocked` false keeps ripple edits on other tracks
    /// from moving the track.
    SetTrack {
        track: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sync_locked: Option<bool>,
    },
    /// An empty track after the others of its kind (`video`, `audio`, or
    /// `subtitle`), with a generated id unless `id` names it.
    AddTrack {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
    /// A sequence's name, frame size, frame rate, and start timecode;
    /// what is absent stays as it is. `width` and `height` go together.
    /// `rate` (`{"num": 30, "den": 1}`, or text such as `"29.97"`) changes
    /// only while the sequence has no clips or markers, and the start
    /// keeps its timecode at the new rate. `start` is timecode text, with
    /// `;` before the frames for drop-frame.
    SetSequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<RateSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
    },
}

/// A generated picture for `insert` and `overwrite`: a generator of the
/// catalog (`title`, `solid`, `adjustment`) and its parameter values,
/// static or keyframed as `set` takes them; the rest stay at their
/// defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratorSpec {
    pub fx: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Json>,
}

/// A frame rate as the editor gives one: `{"num": 30000, "den": 1001}`,
/// text the language reads (`"29.97"`, `"24000/1001"`), or a number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RateSpec {
    Fraction { num: u32, den: u32 },
    Text(String),
    Fps(f64),
}

/// A mask's outline, as the language writes it: at most one of `rect`
/// (left, top, right, bottom), `ellipse` (center x and y, then the radius
/// across and down), or `points` (corners in order, closed unless `closed`
/// is false, and `soft`, how soft the edge is at each, in pixels), in fractions of the clip's picture
/// with 0, 0 at its top left.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MaskOutline {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ellipse: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<Vec<f64>>,
}
