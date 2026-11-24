//! The typed edits an editor sends: what a gesture or a field changes, in frames and ids.

use kerf_model::Interp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json};
use std::borrow::Cow;
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
    /// Property or generator parameter values ([`EditParam`]): a static
    /// value, keyframes with `at` counted from the clip's first frame, or
    /// `null` to restore the default. `name` renames the clip; an empty one
    /// gives it back the name of what it shows.
    Set {
        clip: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        props: BTreeMap<String, EditParam>,
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
        params: BTreeMap<String, EditParam>,
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
        params: BTreeMap<String, EditParam>,
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
        params: BTreeMap<String, EditParam>,
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
        params: BTreeMap<String, EditParam>,
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

/// A property's or a parameter's value as a typed op gives it, the
/// counterpart of the [`ViewParam`](crate::ViewParam) a view shows: `null`
/// to restore its default, one value, or keyframes as
/// `{"keys": [{"at": frames, "value": v, "interp": "linear"}]}`.
///
/// It reads any JSON and writes back the very text it read: kerfd journals
/// an op by its text and checks a retried request against it, so that text
/// must not change for being read into a type, nor with the build's
/// serde_json features. Keys written as [`EditKey`]s write themselves read as
/// [`EditParam::Keyed`]. Keys written as kerfd has always also taken them
/// (`at` as `24.0`, `interp` absent or `null`, other fields beside, a curve's
/// numbers written otherwise) stay [`EditParam::Value`] as written, and
/// [`EditParam::keys`] reads those too.
#[derive(Clone, Debug, PartialEq)]
pub enum EditParam {
    /// `null`: back to its default.
    Default,
    /// Keyframes, in any order: kerfd puts them in time order.
    Keyed(Vec<EditKey>),
    /// One value throughout: a number, a bool, or a text (a choice's name,
    /// a file, a color as `#rrggbb[aa]`). Or any other JSON, as it was
    /// written: keys written loosely, or what kerfd refuses.
    Value(Json),
}

impl EditParam {
    /// What `json` reads as: [`EditParam::Default`] for `null`,
    /// [`EditParam::Keyed`] for keys written exactly as they would write
    /// themselves, and `json` as it is otherwise.
    pub fn from_json(json: Json) -> Self {
        if json.is_null() {
            return Self::Default;
        }
        if let Some(keys) = read_keys(&json) {
            let keyed = Self::Keyed(keys);
            if written_alike(&keyed.to_json(), &json) {
                return keyed;
            }
        }
        Self::Value(json)
    }

    /// It as JSON: what it was read from, or what it stands for.
    pub fn to_json(&self) -> Json {
        match self {
            Self::Default => Json::Null,
            Self::Keyed(keys) => {
                let mut object = Map::new();
                object.insert(
                    "keys".into(),
                    Json::Array(keys.iter().map(EditKey::to_json).collect()),
                );
                Json::Object(object)
            }
            Self::Value(value) => value.clone(),
        }
    }

    /// Whether it restores the default: [`EditParam::Default`], or a `null`
    /// value, which writes the same.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default | Self::Value(Json::Null))
    }

    /// Whether it gives keyframes, whether or not they read: it is
    /// [`EditParam::Keyed`], or an object with `keys`.
    pub fn is_keyed(&self) -> bool {
        match self {
            Self::Default => false,
            Self::Keyed(_) => true,
            Self::Value(value) => value.get("keys").is_some(),
        }
    }

    /// Its keyframes, those written loosely too, as kerfd reads them: an
    /// object with `keys` and nothing else, a list of keys that each read
    /// ([`EditKey`]). None for one value, and for keys that do not read.
    pub fn keys(&self) -> Option<Cow<'_, [EditKey]>> {
        match self {
            Self::Default => None,
            Self::Keyed(keys) => Some(Cow::Borrowed(keys)),
            Self::Value(value) => read_keys(value).map(Cow::Owned),
        }
    }
}

impl From<Json> for EditParam {
    fn from(json: Json) -> Self {
        Self::from_json(json)
    }
}

impl From<Vec<EditKey>> for EditParam {
    fn from(keys: Vec<EditKey>) -> Self {
        Self::Keyed(keys)
    }
}

impl Serialize for EditParam {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(value) => value.serialize(serializer),
            _ => self.to_json().serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for EditParam {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Json::deserialize(deserializer).map(Self::from_json)
    }
}

/// A keyframe of an [`EditParam`]:
/// `{"at": 24, "value": 0.5, "interp": "ease"}`.
#[derive(Clone, Debug, PartialEq)]
pub struct EditKey {
    /// Whole frames of its clip's sequence from the clip's first frame (a
    /// [`ViewKey`](crate::ViewKey)'s `frame`).
    pub at: i64,
    /// A number, a bool, or a text, as [`EditParam::Value`] gives one.
    pub value: Json,
    /// How its curve takes the value to the next key's: written as its name
    /// or a curve's handles `x1,y1,x2,y2` ([`Interp::name`]).
    pub interp: Interp,
}

impl EditKey {
    /// It as a JSON object with `at`, `value` and `interp` put in that
    /// order. serde_json writes an object's fields in the order they were
    /// put in where `preserve_order` is on (the editor's build), and sorted
    /// where it is off (kerfd's), so a key comes out as the object the
    /// editor and kerfd wrote for it before it was a type.
    fn to_json(&self) -> Json {
        let mut key = Map::new();
        key.insert("at".into(), self.at.into());
        key.insert("value".into(), self.value.clone());
        key.insert("interp".into(), self.interp.name().into_owned().into());
        Json::Object(key)
    }

    /// A key as kerfd reads one: an object with `at`, a whole number of
    /// frames (`24` or `24.0`); `value`; and `interp`, a name or a curve's
    /// numbers, `linear` where it is absent or `null`. Other fields are left
    /// unread.
    fn read(json: &Json) -> Option<Self> {
        let key = json.as_object()?;
        let at = key.get("at").and_then(whole)?;
        let value = key.get("value")?.clone();
        let interp = match key.get("interp") {
            None | Some(Json::Null) => Interp::Linear,
            Some(Json::String(name)) => Interp::parse(name)?,
            Some(_) => return None,
        };
        Some(Self { at, value, interp })
    }
}

/// The keys of `{"keys": [...]}` with nothing beside them, when each reads.
fn read_keys(json: &Json) -> Option<Vec<EditKey>> {
    let object = json.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object
        .get("keys")?
        .as_array()?
        .iter()
        .map(EditKey::read)
        .collect()
}

/// A whole number within `i64`: an integer, or a number with no fraction.
fn whole(json: &Json) -> Option<i64> {
    if let Some(whole) = json.as_i64() {
        return Some(whole);
    }
    // `i64::MAX as f64` rounds up to 2^63, the first number past it.
    let number = json.as_f64()?;
    (number.fract() == 0.0 && number >= i64::MIN as f64 && number < i64::MAX as f64)
        .then_some(number as i64)
}

/// Whether `a` and `b` write the same text: the same values, and each
/// object's fields in the same order (which only a `preserve_order` build
/// keeps).
fn written_alike(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((a_name, a), (b_name, b))| a_name == b_name && written_alike(a, b))
        }
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| written_alike(a, b))
        }
        _ => a == b,
    }
}

/// A generated picture for `insert` and `overwrite`: a generator of the
/// catalog (`title`, `solid`, `adjustment`) and its parameter values,
/// static or keyframed as `set` takes them; the rest stay at their
/// defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratorSpec {
    pub fx: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, EditParam>,
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
