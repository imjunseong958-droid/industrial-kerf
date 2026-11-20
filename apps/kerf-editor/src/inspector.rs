//! The chosen clip's inspector, beside the picture: its name and where it
//! sits, a generator's parameters, its properties, its effects and their
//! parameters, and its masks. And the toolbar's format bank.
//!
//! Nothing here keeps an editable copy of the clip. Every field draws what
//! the newest snapshot says (its timeline JSON, kept from the bridge's
//! `state` event), and a change goes back to kerfd as one typed edit: `set`,
//! `fx-add`, `fx-set`, `fx-remove` or `set-sequence`. While a slider is
//! dragged it shows where the pointer has it; the picture changes only when
//! the edit comes back as a new snapshot.

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::motion;
use crate::session::{BridgeEvent, Session};
use crate::theme::{self, height, radius, space, text};
use crate::ui;
use gpui::{
    canvas, div, prelude::*, px, relative, AnimationExt, AnyElement, ClickEvent, Context,
    CursorStyle, DispatchPhase, ElementId, Entity, Focusable, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Rgba, ScrollHandle, SharedString, Subscription, Window,
};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

/// A row's label.
const LABEL: f32 = 84.0;
/// A number's value beside its slider.
const VALUE: f32 = 58.0;
/// A slider's knob; the rail runs between half of it at each end.
const KNOB: f32 = 10.0;
/// A slider's height.
const SLIDER: f32 = 16.0;
/// A colour's swatch.
const SWATCH: f32 = 18.0;
/// Points of pointer travel a scrub on a value takes to cross its range.
const SCRUB_SPAN: f32 = 240.0;
/// Points the pointer moves before a press on a value scrubs rather than types.
const SCRUB_SLOP: f32 = 3.0;

/// Colours a picture often wants, offered under a colour's swatch.
const PALETTE: [&str; 10] = [
    "#ffffff", "#000000", "#808080", "#e03c31", "#f28c28", "#f7d23e", "#3fa34d", "#2bb3c0",
    "#2f5bd3", "#b04ac0",
];

/// The clip properties of a picture, in groups. What the catalog has that
/// no group names goes under More.
const VIDEO_GROUPS: &[Group] = &[
    (
        "Transform",
        &[
            "x", "y", "scale", "scale_x", "scale_y", "rotation", "anchor_x", "anchor_y",
        ],
        true,
    ),
    ("Compositing", &["opacity", "blend"], true),
    ("Crop", &["crop_l", "crop_r", "crop_t", "crop_b"], true),
    (
        "3D",
        &[
            "three_d",
            "z",
            "depth",
            "bevel",
            "bevel_profile",
            "rotate_x",
            "rotate_y",
        ],
        false,
    ),
    ("Motion path", &["path", "along", "orient"], false),
];

/// A title's parameters, in groups.
const TITLE_GROUPS: &[Group] = &[
    (
        "",
        &["text", "font", "size", "weight", "style", "color", "align"],
        true,
    ),
    (
        "Reveal",
        &[
            "reveal",
            "reveal_by",
            "reveal_soft",
            "from_x",
            "from_y",
            "from_scale",
            "from_rotation",
            "from_opacity",
        ],
        false,
    ),
    (
        "Font axes",
        &["wght", "wdth", "slnt", "opsz", "ital", "features"],
        false,
    ),
];

/// A group's name ("" for none), its members, and whether it is open
/// before it is opened or closed by hand.
type Group = (&'static str, &'static [&'static str], bool);

/// Words for the properties whose names say too little.
const LABELS: &[(&str, &str)] = &[
    ("x", "Position X"),
    ("y", "Position Y"),
    ("z", "Distance"),
    ("scale_x", "Scale X"),
    ("scale_y", "Scale Y"),
    ("anchor_x", "Anchor X"),
    ("anchor_y", "Anchor Y"),
    ("crop_l", "Crop Left"),
    ("crop_r", "Crop Right"),
    ("crop_t", "Crop Top"),
    ("crop_b", "Crop Bottom"),
    ("three_d", "3D"),
    ("rotate_x", "Tilt"),
    ("rotate_y", "Turn"),
    ("bevel_profile", "Bevel Shape"),
    ("along", "Along Path"),
    ("orient", "Face Path"),
    ("reveal_soft", "Softness"),
    ("wght", "Weight Axis"),
    ("wdth", "Width Axis"),
    ("slnt", "Slant Axis"),
    ("opsz", "Optical Size"),
    ("ital", "Italic Axis"),
];

/// The range a clip property's slider covers, where the catalog's is far
/// wider than a hand moves through (a typed value may still go to the
/// catalog's limits).
const SOFT: &[(&str, f64, f64)] = &[
    ("x", -1.0, 1.0),
    ("y", -1.0, 1.0),
    ("z", -10.0, 10.0),
    ("depth", 0.0, 1.0),
    ("bevel", 0.0, 0.2),
    ("scale", 0.0, 4.0),
    ("scale_x", -4.0, 4.0),
    ("scale_y", -4.0, 4.0),
    ("rotation", -180.0, 180.0),
    ("rotate_x", -180.0, 180.0),
    ("rotate_y", -180.0, 180.0),
    ("anchor_x", -1.0, 1.0),
    ("anchor_y", -1.0, 1.0),
    ("gain", -48.0, 12.0),
    ("from_x", -1.0, 1.0),
    ("from_y", -1.0, 1.0),
    ("from_scale", 0.0, 4.0),
    ("from_rotation", -180.0, 180.0),
    ("reveal_soft", 0.01, 20.0),
];

// ---------------------------------------------------------------- catalog

/// What a parameter holds, as the inspector edits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Number,
    Bool,
    Color,
    Choice,
    /// Words, a file, a track, a mask, points: held as text.
    Text,
}

/// One parameter's definition, as the catalog's JSON gives it.
#[derive(Clone, Debug, PartialEq)]
struct Param {
    name: String,
    kind: Kind,
    default: Value,
    min: Option<f64>,
    max: Option<f64>,
    unit: Option<String>,
    options: Vec<String>,
    doc: String,
}

impl Param {
    fn read(def: &Value) -> Option<Self> {
        let kind = match def["type"].as_str()? {
            "number" | "time" => Kind::Number,
            "bool" => Kind::Bool,
            "color" => Kind::Color,
            "enum" => Kind::Choice,
            _ => Kind::Text,
        };
        Some(Self {
            name: def["name"].as_str()?.to_owned(),
            kind,
            default: def["default"].clone(),
            min: def["min"].as_f64(),
            max: def["max"].as_f64(),
            unit: def["unit"].as_str().map(str::to_owned),
            options: def["options"]
                .as_array()
                .map(|options| {
                    options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            doc: def["doc"].as_str().unwrap_or_default().to_owned(),
        })
    }

    /// The limits an edit may take.
    fn hard(&self) -> (f64, f64) {
        (
            self.min.unwrap_or(f64::NEG_INFINITY),
            self.max.unwrap_or(f64::INFINITY),
        )
    }
}

fn params(defs: &Value) -> Vec<Param> {
    defs.as_array()
        .into_iter()
        .flatten()
        .filter_map(Param::read)
        .collect()
}

/// An effect, transition or generator of the catalog.
#[derive(Clone, Debug)]
struct FxEntry {
    id: String,
    kind: String,
    domain: String,
    category: String,
    name: String,
    doc: String,
    params: Vec<Param>,
}

impl FxEntry {
    fn read(entry: &Value) -> Option<Self> {
        let text = |key: &str| entry[key].as_str().unwrap_or_default().to_owned();
        Some(Self {
            id: entry["id"].as_str()?.to_owned(),
            kind: text("kind"),
            domain: entry["domain"].as_str().unwrap_or("video").to_owned(),
            category: text("category"),
            name: text("name"),
            doc: text("doc"),
            params: params(&entry["params"]),
        })
    }

    /// What its doc says past its name.
    fn blurb(&self) -> &str {
        let doc = self.doc.trim();
        doc.split_once(": ").map_or(doc, |(_, rest)| rest)
    }
}

/// The catalog this build of Kerf draws with: the clip properties by the
/// track's domain, and every effect, transition and generator.
struct Catalog {
    props: HashMap<String, Vec<Param>>,
    fx: HashMap<String, FxEntry>,
    order: Vec<String>,
}

impl Catalog {
    fn read(props: &Value, all: &Value) -> Self {
        let props = props
            .as_object()
            .map(|domains| {
                domains
                    .iter()
                    .map(|(domain, defs)| (domain.clone(), params(defs)))
                    .collect()
            })
            .unwrap_or_default();
        let entries: Vec<FxEntry> = all
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(FxEntry::read)
            .collect();
        let order = entries.iter().map(|entry| entry.id.clone()).collect();
        let fx = entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        Self { props, fx, order }
    }

    /// The effects a clip of `domain` takes whose name, id or category has
    /// `query` in it, by category and then name.
    fn effects(&self, domain: &str, query: &str) -> Vec<&FxEntry> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<&FxEntry> = self
            .order
            .iter()
            .filter_map(|id| self.fx.get(id))
            .filter(|entry| entry.kind == "effect" && entry.domain == domain)
            .filter(|entry| {
                query.is_empty()
                    || [&entry.name, &entry.id, &entry.category]
                        .iter()
                        .any(|words| words.to_lowercase().contains(&query))
            })
            .collect();
        found.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.name.cmp(&b.name))
        });
        found
    }
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::read(
            &kerf_bridge::clip_properties(),
            &kerf_bridge::fx_catalog_all(),
        )
    })
}

// ---------------------------------------------------------------- the clip

/// Whose parameter a field sets: the clip's own (its properties, and a
/// generator's parameters, which share them), or an effect's in its stack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    Clip,
    Effect(usize),
}

/// One parameter of the chosen clip.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Field {
    target: Target,
    name: String,
}

impl Field {
    fn new(target: Target, name: &str) -> Self {
        Self {
            target,
            name: name.to_owned(),
        }
    }

    /// An element id for `what` of this field.
    fn id(&self, what: &str) -> ElementId {
        match self.target {
            Target::Clip => name_id(format!("{what}-p-{}", self.name)),
            Target::Effect(index) => name_id(format!("{what}-fx{index}-{}", self.name)),
        }
    }

    /// The edit that gives it `value` (`null` restores its default).
    fn op(&self, clip: &str, value: Value) -> Value {
        let mut values = Map::new();
        values.insert(self.name.clone(), value);
        match self.target {
            Target::Clip => json!({"op": "set", "clip": clip, "props": values}),
            Target::Effect(index) => {
                json!({"op": "fx-set", "clip": clip, "index": index, "params": values})
            }
        }
    }
}

fn name_id(name: String) -> ElementId {
    ElementId::Name(name.into())
}

/// The chosen clip as the snapshot's timeline has it.
struct Inspected {
    id: String,
    name: String,
    /// "Video · a.mp4", "Title".
    kind: String,
    track: String,
    locked: bool,
    fps: f64,
    start: f64,
    duration: f64,
    /// The playhead, from the clip's first frame, within it.
    at: f64,
    /// `video` or `audio`: which properties and effects it takes.
    domain: Option<String>,
    generator: Option<String>,
    caption: Option<String>,
    props: Map<String, Value>,
    effects: Vec<Value>,
    masks: Vec<Value>,
    /// How many more clips are chosen.
    others: usize,
}

impl Inspected {
    /// Clip `id` of the snapshot's timeline `raw`, with the playhead at `position`.
    fn find(raw: &Value, id: &str, position: f64, chosen: usize) -> Option<Self> {
        let (track, clip) = raw["tracks"].as_array()?.iter().find_map(|track| {
            track["clips"]
                .as_array()?
                .iter()
                .find(|clip| clip["id"] == id)
                .map(|clip| (track, clip))
        })?;
        let track_type = track["type"].as_str().unwrap_or("video");
        let domain = matches!(track_type, "video" | "audio").then(|| track_type.to_owned());
        let start = clip["timelineStart"].as_f64().unwrap_or(0.0);
        let duration = clip["duration"].as_f64().unwrap_or(0.0).max(0.0);
        let fps = raw["fps"]
            .as_f64()
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .unwrap_or(30.0);
        let kind = match clip["kind"].as_str().unwrap_or("media") {
            "generator" => humanize(clip["fx"].as_str().unwrap_or("generator")),
            "sequence" => "Nested sequence".to_owned(),
            "caption" => "Caption".to_owned(),
            _ => {
                let what = if track_type == "audio" {
                    "Audio"
                } else {
                    "Video"
                };
                let file = clip["sourceId"].as_str().and_then(|source| {
                    raw["sources"]
                        .as_array()?
                        .iter()
                        .find(|candidate| candidate["id"] == source)
                        .and_then(|found| found["path"].as_str())
                        .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned())
                });
                file.map_or_else(|| what.to_owned(), |file| format!("{what} · {file}"))
            }
        };
        let list = |key: &str| clip[key].as_array().cloned().unwrap_or_default();
        Some(Self {
            id: id.to_owned(),
            name: clip["name"].as_str().unwrap_or(id).to_owned(),
            kind,
            track: track["name"]
                .as_str()
                .or(track["id"].as_str())
                .unwrap_or_default()
                .to_owned(),
            locked: track["locked"].as_bool().unwrap_or(false),
            fps,
            start,
            duration,
            at: clip_time(position, fps, clip["startFrame"].as_f64(), start, duration),
            domain,
            generator: (clip["kind"] == "generator")
                .then(|| clip["fx"].as_str().unwrap_or_default().to_owned()),
            caption: (clip["kind"] == "caption")
                .then(|| clip["text"].as_str().unwrap_or_default().to_owned()),
            props: clip["props"].as_object().cloned().unwrap_or_default(),
            effects: list("effects"),
            masks: list("masks"),
            others: chosen.saturating_sub(1),
        })
    }

    /// Every parameter it has a field for: its generator's, its properties,
    /// and each effect's, with what it sets.
    fn fields(&self) -> Vec<(Field, &'static Param, Option<Value>)> {
        let catalog = catalog();
        let generator = self
            .generator
            .as_deref()
            .and_then(|fx| catalog.fx.get(fx))
            .map(|entry| entry.params.as_slice())
            .unwrap_or_default();
        let props = self
            .domain
            .as_deref()
            .and_then(|domain| catalog.props.get(domain))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut fields: Vec<_> = generator
            .iter()
            .chain(props)
            .map(|param| {
                (
                    Field::new(Target::Clip, &param.name),
                    param,
                    self.props.get(&param.name).cloned(),
                )
            })
            .collect();
        for (index, effect) in self.effects.iter().enumerate() {
            let Some(entry) = effect["fx"].as_str().and_then(|fx| catalog.fx.get(fx)) else {
                continue;
            };
            for param in &entry.params {
                let set = effect["params"].get(&param.name).cloned();
                fields.push((Field::new(Target::Effect(index), &param.name), param, set));
            }
        }
        fields
    }

    /// The definition of `field`, and what the clip sets it to.
    fn lookup(&self, field: &Field) -> Option<(&'static Param, Option<Value>)> {
        self.fields()
            .into_iter()
            .find(|(candidate, ..)| candidate == field)
            .map(|(_, param, set)| (param, set))
    }
}

/// The playhead's whole frame, as the picture shows it, in seconds from the
/// clip's first frame (`start_frame`, else `start` seconds), within the clip.
fn clip_time(position: f64, fps: f64, start_frame: Option<f64>, start: f64, duration: f64) -> f64 {
    let first = start_frame.unwrap_or_else(|| (start * fps).round());
    (((position * fps).round() - first) / fps).clamp(0.0, duration)
}

/// What a parameter holds at the playhead.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    value: Value,
    /// It has keyframes: the value is theirs at the playhead.
    keyed: bool,
    /// It is set away from its default.
    changed: bool,
}

fn held(param: &Param, set: Option<&Value>, at: f64) -> Held {
    match set {
        Some(Value::Array(keys)) => Held {
            value: keyed_value(keys, at),
            keyed: true,
            changed: true,
        },
        Some(value) if !value.is_null() => Held {
            value: value.clone(),
            keyed: false,
            changed: !same_value(value, &param.default),
        },
        _ => Held {
            value: param.default.clone(),
            keyed: false,
            changed: false,
        },
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64(), a.as_str(), b.as_str()) {
        (Some(a), Some(b), ..) => (a - b).abs() < 1e-9,
        (.., Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => a == b,
    }
}

/// What keyframes `[{at (seconds from the clip's first frame), value,
/// interp}]` come to at `at`, as kerf-model's `Param::value_at` works it
/// out: each key's curve takes it to the next.
fn keyed_value(keys: &[Value], at: f64) -> Value {
    let keys: Vec<(f64, &Value, &str)> = keys
        .iter()
        .filter_map(|key| {
            Some((
                key["at"].as_f64()?,
                &key["value"],
                key["interp"].as_str().unwrap_or("linear"),
            ))
        })
        .collect();
    let next = keys.partition_point(|key| key.0 <= at);
    match (next.checked_sub(1).map(|index| keys[index]), keys.get(next)) {
        (None, Some(first)) => first.1.clone(),
        (Some(last), None) => last.1.clone(),
        (Some(left), Some(right)) => {
            let span = right.0 - left.0;
            let t = if span > 0.0 {
                ((at - left.0) / span).clamp(0.0, 1.0)
            } else {
                1.0
            };
            interpolate(left.1, right.1, curve(left.2, t))
        }
        (None, None) => Value::Null,
    }
}

/// How far from a key to the next a value is, `t` of the way across.
fn curve(interp: &str, t: f64) -> f64 {
    match interp {
        "linear" => t,
        "hold" => 0.0,
        "ease-in" => t * t,
        "ease-out" => 1.0 - (1.0 - t) * (1.0 - t),
        "ease" => t * t * (3.0 - 2.0 * t),
        bezier => {
            let numbers: Vec<f32> = bezier
                .split(',')
                .filter_map(|number| number.trim().parse().ok())
                .collect();
            match numbers[..] {
                [x1, y1, x2, y2] => f64::from(motion::cubic_bezier(x1, y1, x2, y2, t as f32)),
                _ => t,
            }
        }
    }
}

fn interpolate(a: &Value, b: &Value, t: f64) -> Value {
    if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
        return json!(a + (b - a) * t);
    }
    if let (Some(a), Some(b)) = (a.as_str().and_then(channels), b.as_str().and_then(channels)) {
        let mix = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
        let [r, g, b, alpha] = [0, 1, 2, 3].map(|index| mix(a[index], b[index]));
        return json!(hex(r, g, b, alpha));
    }
    a.clone()
}

// ---------------------------------------------------------------- numbers, colours, words

/// The range a number's slider covers: the catalog's, narrowed where it is
/// far wider than a hand moves through, and widened to hold `value`.
fn soft_range(param: &Param, clip_level: bool, value: f64) -> (f64, f64) {
    let named = clip_level
        .then(|| SOFT.iter().find(|(name, ..)| *name == param.name))
        .flatten()
        .map(|(_, lo, hi)| (*lo, *hi));
    let (mut lo, mut hi) = named.unwrap_or_else(|| guessed_range(param));
    let (min, max) = param.hard();
    lo = lo.max(min);
    hi = hi.min(max);
    if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
        (lo, hi) = guessed_range(param);
    }
    if value.is_finite() {
        lo = lo.min(value);
        hi = hi.max(value);
    }
    (lo, hi)
}

fn guessed_range(param: &Param) -> (f64, f64) {
    let default = param.default.as_f64().unwrap_or(0.0);
    let unit = param.unit.as_deref().unwrap_or_default();
    match (param.min, param.max) {
        (Some(min), Some(max)) if max > min => {
            let span = max - min;
            if unit == "degrees" && span > 720.0 {
                if min < 0.0 {
                    (-180.0, 180.0)
                } else {
                    (0.0, 360.0)
                }
            } else if (unit.starts_with("frame ") || unit.starts_with("clip ")) && span > 4.0 {
                if min < 0.0 {
                    (-1.0, 1.0)
                } else {
                    (min, min + 1.0)
                }
            } else if unit == "×" && span > 10.0 {
                (if min < 0.0 { -4.0 } else { min }, 4.0)
            } else if unit == "px" && span > 200.0 {
                (min, min + 100.0)
            } else {
                (min, max)
            }
        }
        (Some(min), None) => (min, min + (default - min).abs().max(1.0) * 4.0),
        (None, Some(max)) => (max - (max - default).abs().max(1.0) * 4.0, max),
        _ => {
            let reach = default.abs().max(1.0) * 2.0;
            (default - reach, default + reach)
        }
    }
}

/// The places a number over a range `span` wide is shown and moved in.
fn decimals(span: f64) -> usize {
    if span <= 1.5 {
        3
    } else if span <= 10.0 {
        2
    } else if span <= 200.0 {
        1
    } else {
        0
    }
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale
}

/// `value` with `decimals` places, more (up to four) when it has them.
fn format_number(value: f64, decimals: usize) -> String {
    let most = decimals.max(4);
    let places = (decimals..=most)
        .find(|places| (round_to(value, *places) - value).abs() < 1e-9)
        .unwrap_or(most);
    let mut shown = format!("{value:.places$}");
    // Past `decimals`, a zero at the end says nothing.
    if places > decimals {
        while shown.ends_with('0')
            && shown
                .split_once('.')
                .is_some_and(|(_, after)| after.len() > decimals)
        {
            shown.pop();
        }
        if shown.ends_with('.') {
            shown.pop();
        }
    }
    // No minus before a zero.
    if shown.starts_with('-') && shown[1..].chars().all(|c| c == '0' || c == '.') {
        shown.remove(0);
    }
    shown
}

/// A number as typed: a unit after it is let be, and a comma may be the point.
fn parse_number(typed: &str) -> Option<f64> {
    let typed = typed.trim().trim_start_matches('+');
    let typed = typed.trim_end_matches(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','));
    let typed = if typed.contains('.') {
        typed.to_owned()
    } else {
        typed.replace(',', ".")
    };
    typed
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// What follows a number of `unit`, where it says something short.
fn unit_suffix(unit: Option<&str>) -> &'static str {
    match unit.unwrap_or_default() {
        "degrees" => "°",
        "×" => "×",
        "dB" | "dBFS" => "dB",
        "px" => "px",
        "ms" => "ms",
        "Hz" => "Hz",
        "K" => "K",
        "%" => "%",
        _ => "",
    }
}

/// A held value in words.
fn value_words(param: &Param, value: &Value, clip_level: bool) -> String {
    match value {
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or_default();
            let (lo, hi) = soft_range(param, clip_level, number);
            format!(
                "{}{}",
                format_number(number, decimals(hi - lo)),
                unit_suffix(param.unit.as_deref())
            )
        }
        Value::Bool(on) => if *on { "On" } else { "Off" }.to_owned(),
        Value::String(words) => words.clone(),
        Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` may be left out) as its four channels.
fn channels(text: &str) -> Option<[u8; 4]> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();
    let single = |index: usize| {
        u8::from_str_radix(&digits[index..index + 1], 16)
            .ok()
            .map(|v| v * 17)
    };
    Some(match digits.len() {
        3 => [single(0)?, single(1)?, single(2)?, 255],
        4 => [single(0)?, single(1)?, single(2)?, single(3)?],
        6 => [pair(0)?, pair(2)?, pair(4)?, 255],
        8 => [pair(0)?, pair(2)?, pair(4)?, pair(6)?],
        _ => return None,
    })
}

/// A colour as kerf writes it: `#rrggbb`, with its alpha only when not opaque.
fn hex(r: u8, g: u8, b: u8, alpha: u8) -> String {
    if alpha == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{alpha:02x}")
    }
}

/// A typed colour in kerf's form, or None when it is no colour.
fn parse_hex(text: &str) -> Option<String> {
    channels(text).map(|[r, g, b, alpha]| hex(r, g, b, alpha))
}

fn swatch_color(text: &str) -> Rgba {
    let [r, g, b, alpha] = channels(text).unwrap_or([0, 0, 0, 255]);
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: f32::from(alpha) / 255.0,
    }
}

/// A parameter's or generator's name in words: `scale_x` is "Scale X".
fn humanize(name: &str) -> String {
    if let Some((_, words)) = LABELS.iter().find(|(known, _)| *known == name) {
        return (*words).to_owned();
    }
    name.split(['_', '.', '-'])
        .filter(|word| !word.is_empty())
        .enumerate()
        .map(|(index, word)| match word {
            "db" => "dB".to_owned(),
            "dbfs" => "dBFS".to_owned(),
            "hz" => "Hz".to_owned(),
            "x" | "y" | "z" | "id" | "rgb" => word.to_uppercase(),
            _ if index == 0 => {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            _ => word.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a blurb says no more than a name: "Gaussian blur." under "Gaussian blur".
fn same_words(blurb: &str, name: &str) -> bool {
    let bare = |words: &str| words.trim().trim_end_matches('.').to_lowercase();
    bare(blurb).is_empty() || bare(blurb) == bare(name)
}

/// `params` in `groups`, those no group names under More. With no groups,
/// all of them in one with no name.
fn grouped<'a>(params: &'a [Param], groups: &[Group]) -> Vec<(&'static str, bool, Vec<&'a Param>)> {
    let mut found: Vec<(&'static str, bool, Vec<&'a Param>)> = groups
        .iter()
        .map(|(name, members, open)| {
            let members = members
                .iter()
                .filter_map(|member| params.iter().find(|param| param.name == *member))
                .collect();
            (*name, *open, members)
        })
        .collect();
    let rest: Vec<&Param> = params
        .iter()
        .filter(|param| {
            !groups
                .iter()
                .any(|(_, members, _)| members.contains(&param.name.as_str()))
        })
        .collect();
    if !rest.is_empty() {
        found.push((
            if groups.is_empty() { "" } else { "More" },
            groups.is_empty(),
            rest,
        ));
    }
    found.retain(|(_, _, members)| !members.is_empty());
    found
}

// ---------------------------------------------------------------- the view

/// A number being dragged on its slider or scrubbed on its value.
#[derive(Clone, Debug)]
struct Drag {
    clip: String,
    field: Field,
    soft: (f64, f64),
    hard: (f64, f64),
    decimals: usize,
    from: f64,
    value: f64,
    start_x: f32,
    /// On the value, not the slider: a press that does not move types.
    scrub: bool,
    moved: bool,
    /// The slider's rail, left and width.
    rail: Option<(f32, f32)>,
}

impl Drag {
    fn follow(&mut self, x: f32) {
        let (lo, hi) = self.soft;
        let value = match self.rail {
            Some((left, width)) if width > 0.0 => {
                lo + f64::from(((x - left) / width).clamp(0.0, 1.0)) * (hi - lo)
            }
            _ => {
                if (x - self.start_x).abs() < SCRUB_SLOP && !self.moved {
                    return;
                }
                self.from + f64::from((x - self.start_x) / SCRUB_SPAN) * (hi - lo)
            }
        };
        self.moved = true;
        self.value = round_to(value, self.decimals).clamp(self.hard.0, self.hard.1);
    }
}

/// A number typed in its value's place.
#[derive(Clone, Debug)]
struct Typing {
    clip: String,
    field: Field,
    from: f64,
    hard: (f64, f64),
}

/// A number sent and not yet back in a snapshot: shown meanwhile, so the
/// knob does not go back to where it was before it comes forward again.
#[derive(Clone, Debug)]
struct Pending {
    field: Field,
    value: f64,
    revision: u64,
}

/// A field of words (or a colour's hex) and what was last sent from it.
struct TextField {
    input: Entity<TextInput>,
    kind: Kind,
    sent: Option<String>,
    _subscriptions: [Subscription; 2],
}

/// The list Add Effect… opens.
struct Adding {
    search: Entity<TextInput>,
    opened: u64,
    _subscription: Subscription,
}

pub struct InspectorView {
    session: Entity<Session>,
    /// The newest snapshot's timeline, as the bridge sent it.
    raw: Option<Value>,
    revision: u64,
    asked_for_state: bool,
    /// The clip the fields are for.
    clip: Option<String>,
    name: Entity<TextInput>,
    name_sent: Option<String>,
    number: Entity<TextInput>,
    typing: Option<Typing>,
    drag: Option<Drag>,
    pending: Option<Pending>,
    texts: HashMap<Field, TextField>,
    /// Where each slider's rail was drawn, left and width.
    rails: Rc<RefCell<HashMap<Field, (f32, f32)>>>,
    /// Groups opened or closed by hand.
    groups: HashMap<String, bool>,
    /// Effects folded to their heads, by their place in the stack.
    folded: HashSet<usize>,
    /// The field whose options (or palette) are open.
    menu: Option<Field>,
    adding: Option<Adding>,
    addings_opened: u64,
    /// The body's scroll, back at the top for each clip chosen.
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl InspectorView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| TextInput::new(cx, "Clip name").with_style(text::BODY_NAME));
        let number = cx.new(|cx| TextInput::new(cx, "").with_style(text::TIMECODE));
        let name_focus = name.focus_handle(cx);
        let number_focus = number.focus_handle(cx);
        let subscriptions = vec![
            cx.observe(&session, Self::session_notified),
            cx.subscribe(&session, Self::bridge_event),
            cx.subscribe_in(
                &name,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.commit_name(cx),
                    InputEvent::Cancel => {
                        this.revert_name(cx);
                        window.blur();
                    }
                    InputEvent::Changed => {}
                },
            ),
            cx.on_blur(&name_focus, window, |this, _, cx| this.commit_name(cx)),
            cx.subscribe_in(
                &number,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.commit_typed(cx),
                    InputEvent::Cancel => {
                        this.typing = None;
                        cx.notify();
                    }
                    InputEvent::Changed => {}
                },
            ),
            cx.on_blur(&number_focus, window, |this, _, cx| this.commit_typed(cx)),
        ];
        Self {
            session,
            raw: None,
            revision: 0,
            asked_for_state: false,
            clip: None,
            name,
            name_sent: None,
            number,
            typing: None,
            drag: None,
            pending: None,
            texts: HashMap::new(),
            rails: Rc::default(),
            groups: HashMap::new(),
            folded: HashSet::new(),
            menu: None,
            adding: None,
            addings_opened: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn session_notified(&mut self, session: Entity<Session>, cx: &mut Context<Self>) {
        // The snapshot came before this view listened: ask for it again, once.
        let missing = self.raw.is_none() && session.read(cx).state.has_snapshot;
        if missing && !self.asked_for_state {
            self.asked_for_state = true;
            session.update(cx, |session, _| {
                session.send(json!({"type": "refresh"}));
            });
        }
        cx.notify();
    }

    fn bridge_event(&mut self, _: Entity<Session>, event: &BridgeEvent, cx: &mut Context<Self>) {
        match event.0["type"].as_str() {
            Some("state") => {
                let snapshot = &event.0["snapshot"];
                let revision = snapshot["revision"].as_u64().unwrap_or(0);
                self.raw = Some(snapshot["timeline"].clone());
                if revision != self.revision {
                    self.forget_sent();
                }
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.revision != revision)
                {
                    self.pending = None;
                }
                self.revision = revision;
                cx.notify();
            }
            Some("error") => {
                // A refused edit leaves the clip as it was: show it so.
                self.pending = None;
                self.forget_sent();
                cx.notify();
            }
            _ => {}
        }
    }

    fn forget_sent(&mut self) {
        self.name_sent = None;
        for field in self.texts.values_mut() {
            field.sent = None;
        }
    }

    fn inspected(&self, cx: &Context<Self>) -> Option<Inspected> {
        let session = self.session.read(cx);
        let id = self.clip.as_deref()?;
        Inspected::find(
            self.raw.as_ref()?,
            id,
            session.state.position,
            session.selection.len(),
        )
    }

    fn edit(&mut self, ops: Vec<Value>, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.edit(ops);
            cx.notify();
        });
    }

    fn set_param(&mut self, clip: &str, field: &Field, value: Value, cx: &mut Context<Self>) {
        self.edit(vec![field.op(clip, value)], cx);
    }

    fn send_number(&mut self, clip: &str, field: &Field, value: f64, cx: &mut Context<Self>) {
        self.set_param(clip, field, json!(value), cx);
        self.pending = Some(Pending {
            field: field.clone(),
            value,
            revision: self.revision,
        });
    }

    /// Another clip is chosen: what was typed for the last one goes to it
    /// first, and the fields start again.
    fn choose(&mut self, clip: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.clip.is_some() {
            if self.name.focus_handle(cx).is_focused(window) {
                self.commit_name(cx);
            }
            let focused: Vec<Field> = self
                .texts
                .iter()
                .filter(|(_, field)| field.input.focus_handle(cx).is_focused(window))
                .map(|(key, _)| key.clone())
                .collect();
            for key in focused {
                self.commit_text(&key, cx);
            }
            self.commit_typed(cx);
        }
        self.clip = clip;
        self.texts.clear();
        self.rails.borrow_mut().clear();
        self.typing = None;
        self.drag = None;
        self.pending = None;
        self.menu = None;
        self.adding = None;
        self.folded.clear();
        self.scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.name_sent = None;
        let name = self.inspected(cx).map(|clip| clip.name).unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_text(name, cx));
    }

    /// Fields not being typed in show what the snapshot says.
    fn sync_fields(&mut self, clip: &Inspected, window: &mut Window, cx: &mut Context<Self>) {
        let name_focused = self.name.focus_handle(cx).is_focused(window);
        if !name_focused && self.name.read(cx).text() != clip.name {
            let name = clip.name.clone();
            self.name.update(cx, |input, cx| input.set_text(name, cx));
        }
        let mut wanted = HashSet::new();
        for (field, param, set) in clip.fields() {
            if !matches!(param.kind, Kind::Text | Kind::Color)
                || matches!(set, Some(Value::Array(_)))
            {
                continue;
            }
            let current = held(param, set.as_ref(), clip.at)
                .value
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !self.texts.contains_key(&field) {
                self.add_text_field(field.clone(), param, window, cx);
            }
            let entry = &self.texts[&field];
            let focused = entry.input.focus_handle(cx).is_focused(window);
            if !focused && entry.input.read(cx).text() != current {
                entry
                    .input
                    .update(cx, |input, cx| input.set_text(current, cx));
            }
            wanted.insert(field);
        }
        self.texts.retain(|field, _| wanted.contains(field));
    }

    fn add_text_field(
        &mut self,
        field: Field,
        param: &Param,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let style = if param.kind == Kind::Color {
            text::TIMECODE
        } else {
            text::CONTROL
        };
        let placeholder = if param.kind == Kind::Color {
            "#rrggbb".to_owned()
        } else {
            humanize(&param.name)
        };
        let input = cx.new(|cx| TextInput::new(cx, placeholder).with_style(style));
        let key = field.clone();
        let submit = cx.subscribe_in(
            &input,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submit => this.commit_text(&key, cx),
                InputEvent::Cancel => {
                    this.revert_text(&key, cx);
                    window.blur();
                }
                InputEvent::Changed => {}
            },
        );
        let focus = input.focus_handle(cx);
        let key = field.clone();
        let blur = cx.on_blur(&focus, window, move |this, _, cx| {
            this.commit_text(&key, cx)
        });
        self.texts.insert(
            field,
            TextField {
                input,
                kind: param.kind,
                sent: None,
                _subscriptions: [submit, blur],
            },
        );
    }

    fn commit_name(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let typed = self.name.read(cx).text().trim().to_owned();
        if typed == clip.name || self.name_sent.as_deref() == Some(typed.as_str()) {
            return;
        }
        self.name_sent = Some(typed.clone());
        // An empty name gives the clip back the name of what it shows.
        self.edit(
            vec![json!({"op": "set", "clip": clip.id, "name": typed})],
            cx,
        );
    }

    fn revert_name(&mut self, cx: &mut Context<Self>) {
        let name = self.inspected(cx).map(|clip| clip.name).unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_text(name, cx));
    }

    fn commit_text(&mut self, field: &Field, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let Some(entry) = self.texts.get(field) else {
            return;
        };
        let typed = entry.input.read(cx).text().to_owned();
        let current = held(param, set.as_ref(), clip.at)
            .value
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let value = if entry.kind == Kind::Color {
            match parse_hex(&typed) {
                Some(colour) => colour,
                None => {
                    self.session.update(cx, |session, _| {
                        session.state.notify(
                            &format!("“{typed}” is not a colour: write it as #rrggbb."),
                            true,
                        );
                    });
                    self.revert_text(field, cx);
                    return;
                }
            }
        } else {
            typed
        };
        if same_value(&json!(value), &json!(current))
            || entry.sent.as_deref() == Some(value.as_str())
        {
            return;
        }
        if let Some(entry) = self.texts.get_mut(field) {
            entry.sent = Some(value.clone());
        }
        self.set_param(&clip.id, field, json!(value), cx);
    }

    fn revert_text(&mut self, field: &Field, cx: &mut Context<Self>) {
        let Some(clip) = self.inspected(cx) else {
            return;
        };
        let Some((param, set)) = clip.lookup(field) else {
            return;
        };
        let current = held(param, set.as_ref(), clip.at)
            .value
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if let Some(entry) = self.texts.get(field) {
            entry
                .input
                .update(cx, |input, cx| input.set_text(current, cx));
        }
    }

    fn commit_typed(&mut self, cx: &mut Context<Self>) {
        let Some(typing) = self.typing.take() else {
            return;
        };
        let typed = self.number.read(cx).text().to_owned();
        match parse_number(&typed) {
            Some(value) => {
                let value = value.clamp(typing.hard.0, typing.hard.1);
                if (value - typing.from).abs() > 1e-12 {
                    self.send_number(&typing.clip, &typing.field, value, cx);
                }
            }
            None if typed.trim().is_empty() => {}
            None => self.session.update(cx, |session, _| {
                session
                    .state
                    .notify(&format!("“{typed}” is not a number."), true);
            }),
        }
        cx.notify();
    }

    fn start_typing(&mut self, drag: &Drag, window: &mut Window, cx: &mut Context<Self>) {
        self.typing = Some(Typing {
            clip: drag.clip.clone(),
            field: drag.field.clone(),
            from: drag.from,
            hard: drag.hard,
        });
        // The value it has stands in the empty field, so what is typed
        // replaces it, and Enter on nothing leaves it as it was.
        let shown = format_number(drag.from, drag.decimals);
        self.number.update(cx, |input, cx| {
            input.set_placeholder(shown, cx);
            input.clear(cx);
        });
        window.focus(&self.number.focus_handle(cx));
        cx.notify();
    }

    fn press(
        &mut self,
        mut drag: Drag,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();
        self.commit_typed(cx);
        self.menu = None;
        let x = f32::from(event.position.x);
        drag.start_x = x;
        if !drag.scrub {
            drag.rail = self.rails.borrow().get(&drag.field).copied();
            drag.follow(x);
        }
        self.drag = Some(drag);
        cx.notify();
    }

    fn drag_to(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            // Its button came up where it was not seen.
            self.release(window, cx);
            return;
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.follow(f32::from(event.position.x));
        }
        cx.notify();
    }

    /// The button comes up: one edit for the whole drag.
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.scrub && !drag.moved {
            self.start_typing(&drag, window, cx);
            return;
        }
        if (drag.value - drag.from).abs() > 1e-12 {
            self.send_number(&drag.clip, &drag.field, drag.value, cx);
        }
        cx.notify();
    }

    fn open_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| TextInput::new(cx, "Search effects").with_style(text::CONTROL));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => cx.notify(),
                InputEvent::Submit => {
                    // Enter adds the first that matches.
                    let first = this.inspected(cx).and_then(|clip| {
                        let query = this.adding.as_ref()?.search.read(cx).text().to_owned();
                        let domain = clip.domain.clone()?;
                        catalog()
                            .effects(&domain, &query)
                            .first()
                            .map(|entry| (clip.id, entry.id.clone()))
                    });
                    if let Some((clip, fx)) = first {
                        this.add_effect(&clip, &fx, window, cx);
                    }
                }
                InputEvent::Cancel => this.close_adding(window, cx),
            },
        );
        search.read(cx).focus(window);
        self.addings_opened += 1;
        self.adding = Some(Adding {
            search,
            opened: self.addings_opened,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_adding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adding.take().is_some() {
            window.blur();
        }
        cx.notify();
    }

    fn add_effect(&mut self, clip: &str, fx: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(vec![json!({"op": "fx-add", "clip": clip, "fx": fx})], cx);
        self.close_adding(window, cx);
    }

    // ------------------------------------------------------------ drawing

    fn head(&self, clip: &Inspected, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.name.focus_handle(cx).is_focused(window);
        let timecode = |seconds: f64| crate::model::timecode(seconds, clip.fps);
        let times = [
            ("In", timecode(clip.start)),
            ("Out", timecode(clip.start + clip.duration)),
            ("Length", timecode(clip.duration)),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(space::S8))
            .p(px(space::S12))
            .child(field_frame(&self.name, focused, height::H28))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S6))
                    .min_w_0()
                    .child(ui::line(clip.kind.clone(), text::META, theme::ink2()).min_w_0())
                    .child(ui::label("·", text::META, theme::ink3()))
                    .child(ui::line(clip.track.clone(), text::META, theme::ink3()))
                    .when(clip.locked, |row| {
                        row.child(Icon::Lock.view(12.0, theme::ink3()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::S8))
                    .children(times.into_iter().map(|(words, time)| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(ui::label(words, text::META, theme::ink3()))
                            .child(ui::line(time, text::TIMECODE, theme::ink()))
                    })),
            )
            .when(clip.others > 0, |head| {
                let more = if clip.others == 1 {
                    "1 more clip is chosen".to_owned()
                } else {
                    format!("{} more clips are chosen", clip.others)
                };
                head.child(ui::label(more, text::META, theme::ink3()))
            })
    }

    fn generator_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fx = clip.generator.as_deref()?;
        let entry = catalog().fx.get(fx);
        let params = entry
            .map(|entry| entry.params.as_slice())
            .unwrap_or_default();
        let groups = if fx == "title" { TITLE_GROUPS } else { &[] };
        let mut section = div()
            .flex()
            .flex_col()
            .child(section_head(humanize(fx), None, None));
        if params.is_empty() {
            let words = entry.map_or("Nothing to set.", |entry| entry.blurb());
            section = section.child(note(words.to_owned()));
        }
        let rows = self.group_rows(
            "generator",
            clip,
            Target::Clip,
            params,
            groups,
            &clip.props,
            window,
            cx,
        );
        Some(section.children(rows).into_any_element())
    }

    fn clip_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let domain = clip.domain.as_deref()?;
        let params = catalog().props.get(domain)?;
        let groups = if domain == "video" { VIDEO_GROUPS } else { &[] };
        let rows = self.group_rows(
            "clip",
            clip,
            Target::Clip,
            params,
            groups,
            &clip.props,
            window,
            cx,
        );
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head("Clip".to_owned(), None, None))
                .children(rows)
                .into_any_element(),
        )
    }

    /// The rows of `params` in their groups, a group's head opening or
    /// closing it. A group is open at first when it says so or when any
    /// of its parameters is set.
    fn group_rows(
        &self,
        section: &str,
        clip: &Inspected,
        target: Target,
        params: &[Param],
        groups: &[Group],
        values: &Map<String, Value>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for (name, open_at_first, members) in grouped(params, groups) {
            let key = format!("{section}/{name}");
            let any_set = members.iter().any(|param| {
                values
                    .get(&param.name)
                    .is_some_and(|value| !value.is_null())
            });
            let open = name.is_empty()
                || self
                    .groups
                    .get(&key)
                    .copied()
                    .unwrap_or(open_at_first || any_set);
            if !name.is_empty() {
                rows.push(self.group_head(name, key, open, cx));
            }
            if open {
                for param in members {
                    let field = Field::new(target.clone(), &param.name);
                    rows.push(self.param_row(
                        clip,
                        &field,
                        param,
                        values.get(&param.name),
                        window,
                        cx,
                    ));
                }
            }
        }
        rows
    }

    fn group_head(
        &self,
        name: &str,
        key: String,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(name_id(format!("group-{key}")))
            .flex()
            .items_center()
            .gap(px(space::S4))
            .h(px(height::H24))
            .px(px(space::S8))
            .cursor_pointer()
            .text_color(theme::ink3())
            .hover(|style| style.text_color(theme::ink2()))
            .child(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .view(12.0, theme::ink3()),
            )
            .child(theme::type_style(div(), text::META_STRONG).child(name.to_owned()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.groups.insert(key.clone(), !open);
                cx.notify();
            }))
            .into_any_element()
    }

    fn param_row(
        &self,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        set: Option<&Value>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let held = held(param, set, clip.at);
        let clip_level = field.target == Target::Clip;
        let label = self.row_label(clip, field, param, &held, cx);
        let row = div()
            .flex()
            .items_center()
            .gap(px(space::S8))
            .min_h(px(height::H28))
            .px(px(space::S12))
            .child(label);
        if held.keyed {
            let words = value_words(param, &held.value, clip_level);
            return row
                .child(keyed(words, param.kind == Kind::Color))
                .into_any_element();
        }
        match param.kind {
            Kind::Number => match held.value.as_f64() {
                Some(value) => {
                    self.number_row(row, clip, field, param, value, held.changed, window, cx)
                }
                None => row
                    .child(ui::line(
                        value_words(param, &held.value, clip_level),
                        text::CONTROL,
                        theme::ink2(),
                    ))
                    .into_any_element(),
            },
            Kind::Bool => {
                let on = held.value.as_bool().unwrap_or(false);
                let this = cx.entity().downgrade();
                let (clip_id, key) = (clip.id.clone(), field.clone());
                row.child(div().flex_1())
                    .child(ui::switch(
                        format!("switch-{}-{:?}-{}", clip.id, field.target, field.name),
                        on,
                        move |on, _, cx| {
                            this.update(cx, |view, cx| {
                                view.set_param(&clip_id, &key, json!(on), cx)
                            })
                            .ok();
                        },
                    ))
                    .into_any_element()
            }
            Kind::Choice => self.choice_row(
                row,
                clip,
                field,
                param,
                held.value.as_str().unwrap_or_default(),
                cx,
            ),
            Kind::Color => self.color_row(
                row,
                clip,
                field,
                held.value.as_str().unwrap_or("#000000"),
                window,
                cx,
            ),
            Kind::Text => match self.texts.get(field) {
                Some(entry) => {
                    let focused = entry.input.focus_handle(cx).is_focused(window);
                    row.child(field_frame(&entry.input, focused, height::H24).flex_1())
                        .into_any_element()
                }
                None => row
                    .child(ui::line(
                        value_words(param, &held.value, clip_level),
                        text::CONTROL,
                        theme::ink2(),
                    ))
                    .into_any_element(),
            },
        }
    }

    /// A row's label: its doc on rest, and a double press gives back its
    /// default when it is set.
    fn row_label(
        &self,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        held: &Held,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resettable = held.changed && !held.keyed;
        let mut tip = param.doc.trim().to_owned();
        if resettable {
            if !tip.is_empty() {
                tip.push('\n');
            }
            tip.push_str("Double-click to reset.");
        }
        let (clip_id, key) = (clip.id.clone(), field.clone());
        div()
            .id(field.id("label"))
            .w(px(LABEL))
            .flex_none()
            .min_w_0()
            .child(ui::line(
                humanize(&param.name),
                text::CONTROL,
                theme::ink2(),
            ))
            .when(!tip.is_empty(), |label| {
                let tip: SharedString = tip.into();
                label.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone())).into())
            })
            .when(resettable, |label| {
                label.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if event.click_count >= 2 {
                            this.set_param(&clip_id, &key, Value::Null, cx);
                        }
                    }),
                )
            })
            .into_any_element()
    }

    fn number_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        stored: f64,
        changed: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dragged = self.drag.as_ref().filter(|drag| drag.field == *field);
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.field == *field);
        let shown = dragged
            .map(|drag| drag.value)
            .or(pending.map(|pending| pending.value))
            .unwrap_or(stored);
        let (lo, hi) = match dragged {
            Some(drag) => drag.soft,
            None => soft_range(param, field.target == Target::Clip, shown),
        };
        let decimals = decimals(hi - lo);
        let start = Drag {
            clip: clip.id.clone(),
            field: field.clone(),
            soft: (lo, hi),
            hard: param.hard(),
            decimals,
            from: shown,
            value: shown,
            start_x: 0.0,
            scrub: false,
            moved: false,
            rail: None,
        };
        let row = row.child(self.slider(field, shown, (lo, hi), start.clone(), cx));
        let typing = self
            .typing
            .as_ref()
            .is_some_and(|typing| typing.field == *field);
        if typing {
            let focused = self.number.focus_handle(cx).is_focused(window);
            return row
                .child(
                    field_frame(&self.number, focused, height::H24)
                        .w(px(VALUE))
                        .flex_none(),
                )
                .into_any_element();
        }
        let ink = if dragged.is_some() || changed {
            theme::accent_ink()
        } else {
            theme::ink()
        };
        let words = format!(
            "{}{}",
            format_number(shown, decimals),
            unit_suffix(param.unit.as_deref())
        );
        let scrub = Drag {
            scrub: true,
            ..start
        };
        row.child(
            theme::type_style(div(), text::TIMECODE)
                .id(field.id("value"))
                .w(px(VALUE))
                .h(px(height::H24 - 2.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .px(px(space::S4))
                .rounded(px(radius::SMALL))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(ink)
                .cursor(CursorStyle::ResizeLeftRight)
                .hover(|style| style.bg(theme::raised()))
                .child(words)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.press(scrub.clone(), event, window, cx)
                    }),
                ),
        )
        .into_any_element()
    }

    /// A 16-point slider: a 2-point rail, its fill from the left (or from
    /// zero when the range runs either side of it) and a 10-point knob.
    fn slider(
        &self,
        field: &Field,
        value: f64,
        (lo, hi): (f64, f64),
        start: Drag,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let span = (hi - lo).max(f64::EPSILON);
        let at = ((value - lo) / span).clamp(0.0, 1.0) as f32;
        let bipolar = lo < 0.0 && hi > 0.0;
        let zero = if bipolar {
            ((0.0 - lo) / span) as f32
        } else {
            0.0
        };
        let (from, to) = (at.min(zero), at.max(zero));
        let rails = self.rails.clone();
        let key = field.clone();
        let rail_top = (SLIDER - 2.0) / 2.0;
        div()
            .id(field.id("slider"))
            .flex_1()
            .min_w_0()
            .h(px(SLIDER))
            .relative()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press(start.clone(), event, window, cx)
                }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(KNOB / 2.0))
                    .right(px(KNOB / 2.0))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                let rail =
                                    (f32::from(bounds.origin.x), f32::from(bounds.size.width));
                                rails.borrow_mut().insert(key, rail);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(rail_top))
                            .h(px(2.0))
                            .rounded(px(1.0))
                            .bg(theme::track()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(rail_top))
                            .h(px(2.0))
                            .left(relative(from))
                            .w(relative(to - from))
                            .bg(theme::track2()),
                    )
                    .when(bipolar, |rail| {
                        rail.child(
                            div()
                                .absolute()
                                .top(px((SLIDER - 6.0) / 2.0))
                                .left(relative(zero))
                                .w(px(1.0))
                                .h(px(6.0))
                                .bg(theme::track2()),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px((SLIDER - KNOB) / 2.0))
                            .left(relative(at))
                            .ml(px(-KNOB / 2.0))
                            .size(px(KNOB))
                            .rounded_full()
                            .bg(theme::ink())
                            .shadow_sm(),
                    ),
            )
            .into_any_element()
    }

    fn choice_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        param: &Param,
        chosen: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let compact = param.options.len() <= 3
            && param
                .options
                .iter()
                .all(|option| option.chars().count() <= 8);
        if compact {
            let segments = param.options.iter().enumerate().map(|(index, option)| {
                let selected = option == chosen;
                let (clip_id, key, value) = (clip.id.clone(), field.clone(), option.clone());
                theme::type_style(
                    div(),
                    if selected {
                        text::CONTROL_STRONG
                    } else {
                        text::CONTROL
                    },
                )
                .id(field.id(&format!("segment{index}")))
                .flex_1()
                .min_w_0()
                .h(px(height::H20))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .whitespace_nowrap()
                .text_color(if selected {
                    theme::ink()
                } else {
                    theme::ink2()
                })
                .when(selected, |segment| segment.bg(theme::on()))
                .when(!selected, |segment| {
                    segment.hover(|style| style.text_color(theme::ink()))
                })
                .child(humanize(option))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !selected {
                        this.set_param(&clip_id, &key, json!(value), cx);
                    }
                }))
            });
            return row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .p(px(space::S2))
                        .gap(px(space::S2))
                        .rounded(px(radius::CONTROL))
                        .bg(theme::raised())
                        .children(segments),
                )
                .into_any_element();
        }
        let open = self.menu.as_ref() == Some(field);
        let key = field.clone();
        let button = div()
            .id(field.id("choice"))
            .flex_1()
            .min_w_0()
            .h(px(height::H24))
            .px(px(space::S8))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(space::S4))
            .rounded(px(radius::SMALL))
            .bg(theme::raised())
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover2()))
            .child(ui::line(humanize(chosen), text::CONTROL, theme::ink()))
            .child(Icon::ChevronDown.view(12.0, theme::ink2()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.menu = if this.menu.as_ref() == Some(&key) {
                    None
                } else {
                    Some(key.clone())
                };
                cx.notify();
            }));
        let row = row.child(button);
        if !open {
            return row.into_any_element();
        }
        let options = param.options.iter().enumerate().map(|(index, option)| {
            let selected = option == chosen;
            let (clip_id, key, value) = (clip.id.clone(), field.clone(), option.clone());
            div()
                .id(field.id(&format!("option{index}")))
                .h(px(height::H24))
                .px(px(space::S8))
                .flex()
                .items_center()
                .gap(px(space::S6))
                .rounded(px(radius::SMALL))
                .cursor_pointer()
                .hover(|style| style.bg(theme::popup_hover()))
                .child(if selected {
                    Icon::Check.view(12.0, theme::ink()).into_any_element()
                } else {
                    crate::icons::spacer(12.0).into_any_element()
                })
                .child(ui::line(humanize(option), text::CONTROL, theme::ink()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.menu = None;
                    if !selected {
                        this.set_param(&clip_id, &key, json!(value), cx);
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(
                div()
                    .mx(px(space::S12))
                    .ml(px(space::S12 + LABEL + space::S8))
                    .mb(px(space::S4))
                    .p(px(space::S4))
                    .flex()
                    .flex_col()
                    .rounded(px(radius::CONTROL))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::line2())
                    .children(options),
            )
            .into_any_element()
    }

    fn color_row(
        &self,
        row: gpui::Div,
        clip: &Inspected,
        field: &Field,
        colour: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu.as_ref() == Some(field);
        let key = field.clone();
        let swatch = div()
            .id(field.id("swatch"))
            .size(px(SWATCH))
            .flex_none()
            .rounded(px(radius::SMALL))
            .border_1()
            .border_color(if open { theme::lineh() } else { theme::line2() })
            .bg(swatch_color(colour))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.menu = if this.menu.as_ref() == Some(&key) {
                    None
                } else {
                    Some(key.clone())
                };
                cx.notify();
            }));
        let hex_field = match self.texts.get(field) {
            Some(entry) => {
                let focused = entry.input.focus_handle(cx).is_focused(window);
                field_frame(&entry.input, focused, height::H24)
                    .flex_1()
                    .into_any_element()
            }
            None => ui::line(colour.to_owned(), text::TIMECODE, theme::ink2()).into_any_element(),
        };
        let row = row.child(swatch).child(hex_field);
        if !open {
            return row.into_any_element();
        }
        let current = parse_hex(colour).unwrap_or_default();
        let swatches = PALETTE.iter().map(|choice| {
            let selected = *choice == current;
            let (clip_id, key) = (clip.id.clone(), field.clone());
            div()
                .id(field.id(&format!("palette-{choice}")))
                .size(px(SWATCH))
                .rounded(px(radius::SMALL))
                .border_1()
                .border_color(if selected {
                    theme::ink()
                } else {
                    theme::line2()
                })
                .bg(swatch_color(choice))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.menu = None;
                    if !selected {
                        this.set_param(&clip_id, &key, json!(choice), cx);
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::S6))
                    .pl(px(space::S12 + LABEL + space::S8))
                    .pr(px(space::S12))
                    .pb(px(space::S6))
                    .children(swatches),
            )
            .into_any_element()
    }

    fn effects_section(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        clip.domain.as_deref()?;
        let add = ui::button("add-effect", "Add Effect…")
            .small()
            .ghost()
            .icon(Icon::Plus)
            .on_click(cx.listener(|this, _, window, cx| this.open_adding(window, cx)))
            .into_any_element();
        let mut section = div().flex().flex_col().child(section_head(
            "Effects".to_owned(),
            Some(clip.effects.len()),
            Some(add),
        ));
        if clip.effects.is_empty() {
            section = section.child(note("None yet.".to_owned()));
        }
        for (index, effect) in clip.effects.iter().enumerate() {
            section = section.child(self.effect_card(clip, index, effect, window, cx));
        }
        Some(section.into_any_element())
    }

    fn effect_card(
        &self,
        clip: &Inspected,
        index: usize,
        effect: &Value,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fx = effect["fx"].as_str().unwrap_or_default();
        let entry = catalog().fx.get(fx);
        let name = entry.map_or_else(|| fx.to_owned(), |entry| entry.name.clone());
        let enabled = effect["enabled"].as_bool().unwrap_or(true);
        let folded = self.folded.contains(&index);
        let this = cx.entity().downgrade();
        let clip_id = clip.id.clone();
        let toggle = move |on: bool, _: &mut Window, cx: &mut gpui::App| {
            let op = json!({"op": "fx-set", "clip": clip_id, "index": index, "enabled": on});
            this.update(cx, |view, cx| view.edit(vec![op], cx)).ok();
        };
        let clip_id = clip.id.clone();
        let head = div()
            .flex()
            .items_center()
            .gap(px(space::S6))
            .h(px(height::H32))
            .pl(px(space::S4))
            .pr(px(space::S6))
            .child(
                ui::icon_button(
                    name_id(format!("fold-{index}")),
                    if folded {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                )
                .size(height::H20)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.folded.remove(&index) {
                        this.folded.insert(index);
                    }
                    cx.notify();
                })),
            )
            .child(
                ui::line(
                    name,
                    text::CONTROL_STRONG,
                    if enabled { theme::ink() } else { theme::ink3() },
                )
                .flex_1()
                .min_w_0(),
            )
            .child(ui::switch(
                format!("effect-on-{}-{index}", clip.id),
                enabled,
                toggle,
            ))
            .child(
                ui::icon_button(name_id(format!("remove-effect-{index}")), Icon::Trash)
                    .size(height::H24)
                    .tooltip("Remove")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // The cards after it move up a place.
                        this.folded.clear();
                        this.edit(
                            vec![json!({"op": "fx-remove", "clip": clip_id, "index": index})],
                            cx,
                        );
                    })),
            );
        let mut card = div()
            .mx(px(space::S8))
            .mb(px(space::S6))
            .pb(px(if folded { 0.0 } else { space::S4 }))
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .bg(theme::card())
            .child(head);
        if folded {
            return card.into_any_element();
        }
        match entry {
            None => card = card.child(note(format!("“{fx}” is not in this version of Kerf."))),
            Some(entry) if entry.params.is_empty() => {
                card = card.child(note("Nothing to set.".to_owned()))
            }
            Some(entry) => {
                let values = effect["params"].as_object().cloned().unwrap_or_default();
                for param in &entry.params {
                    let field = Field::new(Target::Effect(index), &param.name);
                    card = card.child(self.param_row(
                        clip,
                        &field,
                        param,
                        values.get(&param.name),
                        window,
                        cx,
                    ));
                }
            }
        }
        card.into_any_element()
    }

    fn masks_section(&self, clip: &Inspected) -> Option<AnyElement> {
        if clip.masks.is_empty() {
            return None;
        }
        let rows = clip.masks.iter().map(|mask| {
            let name = mask["name"].as_str().unwrap_or_default().to_owned();
            let mode = humanize(mask["mode"].as_str().unwrap_or("add"));
            let shape = ["rect", "ellipse", "points"]
                .into_iter()
                .find(|shape| !mask[*shape].is_null())
                .map_or("Shape", |shape| match shape {
                    "rect" => "Rectangle",
                    "ellipse" => "Ellipse",
                    _ => "Path",
                });
            div()
                .flex()
                .items_center()
                .gap(px(space::S8))
                .h(px(height::H28))
                .px(px(space::S12))
                .child(
                    ui::line(name, text::TIMECODE, theme::ink())
                        .w(px(LABEL))
                        .flex_none(),
                )
                .child(
                    ui::line(format!("{shape} · {mode}"), text::CONTROL, theme::ink2())
                        .flex_1()
                        .min_w_0(),
                )
                .when(mask["inverted"].as_bool().unwrap_or(false), |row| {
                    row.child(ui::chip("Inverted", theme::ink2()))
                })
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head(
                    "Masks".to_owned(),
                    Some(clip.masks.len()),
                    None,
                ))
                .children(rows)
                .into_any_element(),
        )
    }

    fn caption_section(&self, clip: &Inspected) -> Option<AnyElement> {
        let words = clip.caption.clone()?;
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_head("Caption".to_owned(), None, None))
                .child(
                    ui::label(words, text::BODY, theme::ink())
                        .px(px(space::S12))
                        .pb(px(space::S8)),
                )
                .into_any_element(),
        )
    }

    /// The effects a clip takes, by category, to add one.
    fn adding_list(
        &self,
        clip: &Inspected,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let adding = self.adding.as_ref()?;
        let domain = clip.domain.clone()?;
        let query = adding.search.read(cx).text().to_owned();
        let focused = adding.search.focus_handle(cx).is_focused(window);
        let found = catalog().effects(&domain, &query);
        let mut list = div()
            .id("effect-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .py(px(space::S4));
        if found.is_empty() {
            list = list.child(note("No effect matches.".to_owned()).py(px(space::S12)));
        }
        let mut category = None;
        for (index, entry) in found.into_iter().enumerate() {
            if category != Some(entry.category.as_str()) {
                category = Some(entry.category.as_str());
                let words = if entry.category.is_empty() {
                    "Other"
                } else {
                    entry.category.as_str()
                };
                list = list.child(
                    ui::label(words.to_owned(), text::META_STRONG, theme::ink3())
                        .px(px(space::S12))
                        .pt(px(space::S8))
                        .pb(px(space::S2)),
                );
            }
            let (clip_id, fx) = (clip.id.clone(), entry.id.clone());
            let blurb = entry.blurb();
            list = list.child(
                div()
                    .id(("effect-entry", index))
                    .px(px(space::S12))
                    .py(px(space::S4))
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::raised()))
                    .child(ui::line(entry.name.clone(), text::CONTROL, theme::ink()))
                    .when(!same_words(blurb, &entry.name), |row| {
                        row.child(ui::line(blurb.to_owned(), text::META, theme::ink3()))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.add_effect(&clip_id, &fx, window, cx)
                    })),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .bg(theme::chrome())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S6))
                        .p(px(space::S8))
                        .child(Icon::Search.view(14.0, theme::ink3()))
                        .child(field_frame(&adding.search, focused, height::H28).flex_1())
                        .child(
                            ui::icon_button("close-effects", Icon::Close)
                                .size(height::H24)
                                .tooltip("Close")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.close_adding(window, cx)
                                    }),
                                ),
                        ),
                )
                .child(ui::hairline(true))
                .child(list)
                // It fades in where it is; Esc or its close takes it away at once.
                .with_animation(
                    ("add-effect", adding.opened),
                    motion::animation(theme::motion::FADE),
                    |list, t| list.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// While a number is dragged, it follows the pointer anywhere in the
    /// window and ends wherever its button comes up.
    fn drag_listener(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.drag.as_ref()?;
        let this = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let moved = this.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            moved
                                .update(cx, |view, cx| view.drag_to(event, window, cx))
                                .ok();
                        }
                    });
                    let released = this;
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                            released
                                .update(cx, |view, cx| view.release(window, cx))
                                .ok();
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .into_any_element(),
        )
    }
}

impl Render for InspectorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.session.read(cx).selection.first().cloned();
        if first != self.clip {
            self.choose(first, window, cx);
        }
        let root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::chrome());
        let Some(clip) = self.inspected(cx) else {
            let words = if self.raw.is_none() {
                "Waiting for the project…"
            } else {
                "Choose a clip on the timeline."
            };
            return root
                .items_center()
                .justify_center()
                .child(ui::label(words, text::CONTROL, theme::ink3()))
                .into_any_element();
        };
        self.sync_fields(&clip, window, cx);
        let sections: Vec<AnyElement> = [
            self.generator_section(&clip, window, cx),
            self.caption_section(&clip),
            self.clip_section(&clip, window, cx),
            self.effects_section(&clip, window, cx),
            self.masks_section(&clip),
        ]
        .into_iter()
        .flatten()
        .collect();
        let body = div()
            .id("inspector-body")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(space::S16))
            .child(self.head(&clip, window, cx))
            .children(sections.into_iter().map(|section| {
                div()
                    .flex()
                    .flex_col()
                    .child(ui::hairline(true))
                    .child(section)
            }));
        root.child(body)
            .children(self.drag_listener(cx))
            .children(self.adding_list(&clip, window, cx))
            .into_any_element()
    }
}

/// A section's head: its name, how many it has, and what it offers.
fn section_head(title: String, count: Option<usize>, trailing: Option<AnyElement>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S6))
        .h(px(height::H32))
        .pl(px(space::S12))
        .pr(px(space::S8))
        .mt(px(space::S4))
        .child(ui::label(title, text::CONTROL_STRONG, theme::ink()))
        .when_some(count, |head, count| {
            head.child(ui::label(count.to_string(), text::META, theme::ink3()))
        })
        .child(div().flex_1())
        .children(trailing)
}

fn note(words: String) -> gpui::Div {
    ui::label(words, text::META, theme::ink3())
        .px(px(space::S12))
        .py(px(space::S4))
}

/// A keyframed value: it is set on the timeline's keys, not here.
fn keyed(words: String, colour: bool) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::S6))
        .child(
            theme::type_style(div(), text::META)
                .flex_none()
                .h(px(height::H20 - 2.0))
                .px(px(space::S6))
                .flex()
                .items_center()
                .rounded(px(radius::SMALL))
                .bg(theme::accent_wash())
                .text_color(theme::accent_ink())
                .child("Keyframed"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S6))
                .min_w_0()
                .when(colour, |value| {
                    value.child(
                        div()
                            .size(px(SWATCH - 4.0))
                            .flex_none()
                            .rounded(px(radius::SMALL))
                            .border_1()
                            .border_color(theme::line2())
                            .bg(swatch_color(&words)),
                    )
                })
                .child(ui::line(words, text::TIMECODE, theme::ink2())),
        )
}

/// A text field in its frame, ringed in the accent while typed in. A press
/// anywhere in the frame types in it.
fn field_frame(input: &Entity<TextInput>, focused: bool, tall: f32) -> gpui::Div {
    let input = input.clone();
    div()
        .min_w_0()
        .h(px(tall))
        .flex()
        .items_center()
        .px(px(space::S6))
        .overflow_hidden()
        .rounded(px(radius::SMALL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .on_mouse_down(MouseButton::Left, {
            let input = input.clone();
            move |_, window, cx| {
                input.read(cx).focus(window);
            }
        })
        .child(input)
}

// ---------------------------------------------------------------- the format bank

/// The shapes a sequence is offered in: what it is called, its ratio, and its size.
const FORMATS: [(&str, &str, u32, u32); 4] = [
    ("Landscape", "16:9", 1920, 1080),
    ("Vertical", "9:16", 1080, 1920),
    ("Square", "1:1", 1080, 1080),
    ("Portrait", "4:5", 1080, 1350),
];

/// Which of [`FORMATS`] a sequence `width` × `height` is.
fn format_index(width: u32, height: u32) -> Option<usize> {
    FORMATS
        .iter()
        .position(|(_, _, w, h)| *w == width && *h == height)
}

/// The sequence's shape, chosen from a few (16:9, 9:16, 1:1, 4:5), in the toolbar.
/// The one the snapshot has is filled; another size fills none and says so.
pub struct FormatBank {
    session: Entity<Session>,
}

impl FormatBank {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self { session }
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((_, _, width, height)) = FORMATS.get(index).copied() else {
            return;
        };
        self.session.update(cx, |session, cx| {
            let timeline = session.timeline();
            if timeline.width == width && timeline.height == height {
                return;
            }
            let mut op = json!({"op": "set-sequence", "width": width, "height": height});
            if let Some(sequence) = timeline.sequence.clone() {
                op["sequence"] = json!(sequence);
            }
            session.edit(vec![op]);
            cx.notify();
        });
    }
}

impl Render for FormatBank {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let (width, height) = (timeline.width, timeline.height);
        if !session.has_project() || !session.state.has_snapshot || width == 0 || height == 0 {
            return div().into_any_element();
        }
        let chosen = format_index(width, height);
        let custom: Option<SharedString> = chosen
            .is_none()
            .then(|| format!("Custom · {width} × {height}").into());
        div()
            .id("format-bank")
            .flex()
            .flex_none()
            .p(px(space::S2))
            .gap(px(space::S2))
            .rounded(px(radius::CONTROL + 1.0))
            .bg(theme::raised())
            .when_some(custom, |bank, tip| {
                bank.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone())).into())
            })
            .children(
                FORMATS
                    .iter()
                    .enumerate()
                    .map(|(index, (name, ratio, w, h))| {
                        let selected = chosen == Some(index);
                        let tip: SharedString = format!("{name} {ratio} · {w}×{h}").into();
                        theme::type_style(
                            div(),
                            if selected {
                                text::CONTROL_STRONG
                            } else {
                                text::CONTROL
                            },
                        )
                        .id(("format", index))
                        .px(px(space::S8))
                        .h(px(height::H24))
                        .flex()
                        .items_center()
                        .rounded(px(radius::SMALL + 1.0))
                        .cursor_pointer()
                        .text_color(if selected {
                            theme::ink()
                        } else {
                            theme::ink2()
                        })
                        .when(selected, |segment| segment.bg(theme::on()))
                        .when(!selected, |segment| {
                            segment.hover(|style| style.text_color(theme::ink()))
                        })
                        .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone())).into())
                        .child(*ratio)
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.choose(index, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn number(name: &str, default: f64, min: f64, max: f64, unit: Option<&str>) -> Param {
        Param {
            name: name.to_owned(),
            kind: Kind::Number,
            default: json!(default),
            min: Some(min),
            max: Some(max),
            unit: unit.map(str::to_owned),
            options: Vec::new(),
            doc: String::new(),
        }
    }

    #[test]
    fn the_catalog_has_the_clip_properties_and_the_effects_a_clip_takes() {
        let catalog = catalog();
        let video: Vec<&str> = catalog.props["video"]
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        for name in ["x", "y", "scale", "rotation", "opacity", "crop_l", "blend"] {
            assert!(video.contains(&name), "{name} in {video:?}");
        }
        let audio: Vec<&str> = catalog.props["audio"]
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        assert_eq!(audio, ["gain", "pan"]);
        let title = &catalog.fx["title"];
        assert_eq!(title.kind, "generator");
        assert!(title
            .params
            .iter()
            .any(|param| param.name == "text" && param.kind == Kind::Text));
        assert!(title
            .params
            .iter()
            .any(|param| param.name == "color" && param.kind == Kind::Color));
        // A picture clip is offered picture effects only, and a search narrows them.
        let blur = catalog.effects("video", "gaussian");
        assert!(blur.iter().any(|entry| entry.id == "blur.gaussian"));
        assert!(catalog
            .effects("video", "")
            .iter()
            .all(|entry| entry.kind == "effect" && entry.domain == "video"));
        assert!(catalog
            .effects("audio", "")
            .iter()
            .any(|entry| entry.id == "audio.compressor"));
    }

    #[test]
    fn every_video_property_is_shown_in_a_group() {
        let params = &catalog().props["video"];
        let shown: usize = grouped(params, VIDEO_GROUPS)
            .iter()
            .map(|(_, _, members)| members.len())
            .sum();
        assert_eq!(shown, params.len());
        let shown: usize = grouped(params, &[])
            .iter()
            .map(|(_, _, members)| members.len())
            .sum();
        assert_eq!(shown, params.len());
    }

    #[test]
    fn a_field_sets_its_own_parameter_with_the_ops_kerfd_takes() {
        let clip = Field::new(Target::Clip, "opacity");
        assert_eq!(
            clip.op("one", json!(0.5)),
            json!({"op": "set", "clip": "one", "props": {"opacity": 0.5}})
        );
        let effect = Field::new(Target::Effect(2), "radius");
        assert_eq!(
            effect.op("one", Value::Null),
            json!({"op": "fx-set", "clip": "one", "index": 2, "params": {"radius": null}})
        );
    }

    #[test]
    fn keyframes_come_to_their_value_at_the_playhead() {
        let keys = json!([
            {"at": 0.0, "value": 0.0, "interp": "linear"},
            {"at": 1.0, "value": 1.0, "interp": "hold"},
            {"at": 2.0, "value": 3.0, "interp": "ease"},
        ]);
        let keys = keys.as_array().unwrap();
        assert_eq!(keyed_value(keys, -1.0), json!(0.0));
        assert_eq!(keyed_value(keys, 0.25), json!(0.25));
        // Hold keeps its value until the next key.
        assert_eq!(keyed_value(keys, 1.9), json!(1.0));
        assert_eq!(keyed_value(keys, 2.0), json!(3.0));
        assert_eq!(keyed_value(keys, 9.0), json!(3.0));
        let colours = json!([{"at": 0.0, "value": "#000000", "interp": "linear"}, {"at": 1.0, "value": "#ffffff"}]);
        assert_eq!(
            keyed_value(colours.as_array().unwrap(), 0.5),
            json!("#808080")
        );
        // The playhead counts on whole frames, as the picture does.
        assert_eq!(clip_time(5.0104, 24.0, Some(96.0), 4.0, 3.0), 1.0);
        assert_eq!(clip_time(9.0, 24.0, Some(96.0), 4.0, 3.0), 3.0);
        assert_eq!(clip_time(1.0, 24.0, None, 4.0, 3.0), 0.0);
        let param = number("opacity", 1.0, 0.0, 1.0, None);
        let held = held(&param, Some(&json!([{"at": 0.0, "value": 0.2}])), 0.0);
        assert!(held.keyed && held.changed);
        assert_eq!(held.value, json!(0.2));
    }

    #[test]
    fn a_slider_covers_what_a_hand_moves_through() {
        let rotation = number("rotation", 0.0, -36000.0, 36000.0, Some("degrees"));
        assert_eq!(soft_range(&rotation, true, 0.0), (-180.0, 180.0));
        // A value past it widens it.
        assert_eq!(soft_range(&rotation, true, 720.0), (-180.0, 720.0));
        let from = number("from_rotation", 0.0, -36000.0, 36000.0, Some("degrees"));
        assert_eq!(soft_range(&from, false, 0.0), (-180.0, 180.0));
        let opacity = number("opacity", 1.0, 0.0, 1.0, None);
        assert_eq!(soft_range(&opacity, true, 1.0), (0.0, 1.0));
        let scale = number("scale", 1.0, 0.0, 100.0, Some("×"));
        assert_eq!(soft_range(&scale, false, 1.0), (0.0, 4.0));
        let radius = number("radius", 4.0, 0.0, 500.0, Some("px"));
        assert_eq!(soft_range(&radius, false, 4.0), (0.0, 100.0));
    }

    #[test]
    fn names_read_as_words() {
        assert_eq!(humanize("crop_l"), "Crop Left");
        assert_eq!(humanize("threshold_db"), "Threshold dB");
        assert_eq!(humanize("title"), "Title");
        assert_eq!(humanize("reveal_by"), "Reveal by");
        assert!(same_words("Gaussian blur.", "Gaussian blur"));
        assert!(!same_words(
            "Motion blur in one direction.",
            "Directional Blur"
        ));
    }

    #[test]
    fn the_format_bank_knows_its_sizes() {
        assert_eq!(format_index(1920, 1080), Some(0));
        assert_eq!(format_index(1080, 1920), Some(1));
        assert_eq!(format_index(1080, 1080), Some(2));
        assert_eq!(format_index(1080, 1350), Some(3));
        assert_eq!(format_index(1280, 720), None);
    }

    proptest! {
        #[test]
        fn a_colour_reads_back_as_it_was_written(r: u8, g: u8, b: u8, alpha: u8) {
            let written = hex(r, g, b, alpha);
            prop_assert_eq!(channels(&written), Some([r, g, b, alpha]));
            prop_assert_eq!(parse_hex(&written), Some(written.clone()));
            prop_assert_eq!(parse_hex(&written.to_uppercase()), Some(written.clone()));
            prop_assert_eq!(parse_hex(written.trim_start_matches('#')), Some(written));
        }

        #[test]
        fn a_shown_number_reads_back_as_it_was(value in -1.0e5f64..1.0e5, places in 0usize..4) {
            let shown = format_number(value, places);
            let back = parse_number(&shown).expect("a number");
            // Never further than half its last place, which is at most the fourth.
            prop_assert!((back - value).abs() <= 0.5e-4 + 1e-9, "{} shown as {}", value, shown);
            // Showing what it reads back as changes nothing.
            prop_assert_eq!(format_number(back, places), shown);
        }

        #[test]
        fn between_two_linear_keys_a_value_stays_between_them(a in -100.0f64..100.0, b in -100.0f64..100.0, at in -1.0f64..2.0) {
            let keys = json!([{"at": 0.0, "value": a, "interp": "linear"}, {"at": 1.0, "value": b, "interp": "linear"}]);
            let value = keyed_value(keys.as_array().unwrap(), at).as_f64().unwrap();
            prop_assert!(value >= a.min(b) - 1e-9 && value <= a.max(b) + 1e-9);
        }

        #[test]
        fn a_drag_stays_within_what_the_edit_may_take(x in -1000.0f32..1000.0, from in -10.0f64..10.0) {
            for rail in [Some((10.0, 200.0)), None] {
                let mut drag = Drag {
                    clip: "c".into(),
                    field: Field::new(Target::Clip, "x"),
                    soft: (-1.0, 1.0),
                    hard: (-10.0, 10.0),
                    decimals: 2,
                    from,
                    value: from,
                    start_x: 0.0,
                    scrub: rail.is_none(),
                    moved: false,
                    rail,
                };
                drag.follow(x);
                prop_assert!(drag.value >= -10.0 && drag.value <= 10.0);
                if rail.is_some() {
                    prop_assert!(drag.value >= -1.0 && drag.value <= 1.0);
                }
            }
        }
    }
}
