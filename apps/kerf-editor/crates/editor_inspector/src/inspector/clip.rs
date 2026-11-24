//! The chosen clip as the snapshot's timeline has it, the fields of its
//! parameters and the edits that set them, and what a parameter holds at
//! the playhead, its keyframes worked out as kerf-model works them out.

use super::catalog::{catalog, Param};
use super::values::humanize;
use editor_state::model::{EditParam, Timeline, TrackKind, ViewParam};
use gpui::ElementId;
use kerf_bridge::event::{ViewEffect, ViewMask};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Whose parameter a field sets: the clip's own (its properties, and a
/// generator's parameters, which share them), or an effect's in its stack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    Clip,
    Effect(usize),
}

/// One parameter of the chosen clip.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Field {
    pub(super) target: Target,
    pub(super) name: String,
}

impl Field {
    pub(super) fn new(target: Target, name: &str) -> Self {
        Self {
            target,
            name: name.to_owned(),
        }
    }

    /// An element id for `what` of this field.
    pub(super) fn id(&self, what: &str) -> ElementId {
        match self.target {
            Target::Clip => name_id(format!("{what}-p-{}", self.name)),
            Target::Effect(index) => name_id(format!("{what}-fx{index}-{}", self.name)),
        }
    }

    /// The edit that gives it `value`.
    pub(super) fn op(&self, clip: &str, value: EditParam) -> Value {
        let values = BTreeMap::from([(self.name.clone(), value)]);
        match self.target {
            Target::Clip => json!({"op": "set", "clip": clip, "props": values}),
            Target::Effect(index) => {
                json!({"op": "fx-set", "clip": clip, "index": index, "params": values})
            }
        }
    }
}

pub(super) fn name_id(name: String) -> ElementId {
    ElementId::Name(name.into())
}

/// The chosen clip as the snapshot's timeline has it.
pub(super) struct Inspected {
    pub(super) id: String,
    pub(super) name: String,
    /// "Video · a.mp4", "Title".
    pub(super) kind: String,
    pub(super) track: String,
    pub(super) locked: bool,
    pub(super) fps: f64,
    pub(super) start: f64,
    pub(super) duration: f64,
    /// The playhead, from the clip's first frame, within it.
    pub(super) at: f64,
    /// `video` or `audio`: which properties and effects it takes.
    pub(super) domain: Option<String>,
    pub(super) generator: Option<String>,
    pub(super) caption: Option<String>,
    pub(super) props: BTreeMap<String, ViewParam>,
    pub(super) effects: Vec<ViewEffect>,
    pub(super) masks: Vec<ViewMask>,
    /// How many more clips are chosen.
    pub(super) others: usize,
}

impl Inspected {
    /// Clip `id` of the snapshot's `timeline`, with the playhead at `position`.
    pub(super) fn find(
        timeline: &Timeline,
        id: &str,
        position: f64,
        chosen: usize,
    ) -> Option<Self> {
        let (track, clip) = timeline.tracks.iter().find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == id)
                .map(|clip| (track, clip))
        })?;
        let domain = match track.kind {
            TrackKind::Video => Some("video".to_owned()),
            TrackKind::Audio => Some("audio".to_owned()),
            TrackKind::Subtitle => None,
        };
        let start = clip.timeline_start;
        let duration = clip.duration.max(0.0);
        let fps = if timeline.fps.is_finite() && timeline.fps > 0.0 {
            timeline.fps
        } else {
            30.0
        };
        let kind = match clip.kind.as_str() {
            "generator" => humanize(clip.fx.as_deref().unwrap_or("generator")),
            "sequence" => "Nested sequence".to_owned(),
            "caption" => "Caption".to_owned(),
            _ => {
                let what = if track.kind == TrackKind::Audio {
                    "Audio"
                } else {
                    "Video"
                };
                let file = clip
                    .source_id
                    .as_deref()
                    .and_then(|source| timeline.source(source))
                    .map(|found| {
                        let path = found.path.as_str();
                        path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
                    });
                file.map_or_else(|| what.to_owned(), |file| format!("{what} · {file}"))
            }
        };
        Some(Self {
            id: id.to_owned(),
            name: clip.name.as_deref().unwrap_or(id).to_owned(),
            kind,
            track: track.name.as_deref().unwrap_or(&track.id).to_owned(),
            locked: track.locked,
            fps,
            start,
            duration,
            at: clip_time(position, fps, clip.start_frame, duration),
            domain,
            generator: (clip.kind == "generator").then(|| clip.fx.clone().unwrap_or_default()),
            caption: (clip.kind == "caption").then(|| clip.text.clone().unwrap_or_default()),
            props: clip.props.clone(),
            effects: clip.effects.clone(),
            masks: clip.masks.clone(),
            others: chosen.saturating_sub(1),
        })
    }

    /// Every parameter it has a field for: its generator's, its properties,
    /// and each effect's, with what it sets.
    pub(super) fn fields(&self) -> Vec<(Field, &'static Param, Option<ViewParam>)> {
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
            let Some(entry) = catalog.fx.get(effect.fx.as_str()) else {
                continue;
            };
            for param in &entry.params {
                let set = effect.params.get(&param.name).cloned();
                fields.push((Field::new(Target::Effect(index), &param.name), param, set));
            }
        }
        fields
    }

    /// The definition of `field`, and what the clip sets it to.
    pub(super) fn lookup(&self, field: &Field) -> Option<(&'static Param, Option<ViewParam>)> {
        self.fields()
            .into_iter()
            .find(|(candidate, ..)| candidate == field)
            .map(|(_, param, set)| (param, set))
    }
}

/// The playhead's whole frame, as the picture shows it, in seconds from the
/// clip's first frame (`start_frame`), within the clip.
fn clip_time(position: f64, fps: f64, start_frame: i64, duration: f64) -> f64 {
    (((position * fps).round() - start_frame as f64) / fps).clamp(0.0, duration)
}

/// What a parameter holds at the playhead.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Held {
    pub(super) value: Value,
    /// It has keyframes: the value is theirs at the playhead.
    pub(super) keyed: bool,
    /// It is set away from its default.
    pub(super) changed: bool,
}

pub(super) fn held(param: &Param, set: Option<&ViewParam>, at: f64) -> Held {
    match set {
        Some(keyed @ ViewParam::Keyed(_)) => Held {
            value: keyed.value_at(at),
            keyed: true,
            changed: true,
        },
        Some(ViewParam::Value(value)) if !value.is_null() => Held {
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

pub(super) fn same_value(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64(), a.as_str(), b.as_str()) {
        (Some(a), Some(b), ..) => (a - b).abs() < 1e-9,
        (.., Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => a == b,
    }
}
