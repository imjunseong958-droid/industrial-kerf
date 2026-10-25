//! The timeline that editor snapshots carry.
//!
//! A read-only JSON view of the first sequence of the model for clients that
//! draw a timeline (the Qt app). Times are exact seconds of the model, with
//! frame counts on the sequence rate beside them; clips appear at their own
//! spans, and transitions are listed separately with the region they cover.
//! Edits go back as typed operations (`EditOp`), never by patching this.
//!
//! `properties` carries the catalog's intrinsic clip property definitions once
//! per snapshot, keyed by track type, and `generators` the parameter
//! definitions of each generator. Each clip's `props` holds only the
//! properties and generator parameters it sets: a static value, or a keyframe
//! list when animated. Anything absent is at its default. Its `effects` list
//! the effect stack in order, each with the parameters it sets the same way;
//! their definitions are [`fx_catalog_json`]. Its `masks` list a picture
//! clip's masks in order: each one's outline as the language writes it
//! (`rect`, `ellipse`, or `points` and `closed`), its mode and inversion, and
//! the properties it sets, which `maskProperties` defines; `maskModes` lists
//! the modes.

use kerf_model::{
    Catalog, Clip, Content, Domain, FxKind, Mask, MaskMode, MaskShape, Param, ParamDef, ParamType,
    Project, Rate, Sequence, Time, Track, Value,
};
use serde_json::{json, Map, Value as Json};

pub(crate) fn timeline_json(project: &Project) -> Json {
    let mut value = json!({
        "name": project.name,
        "sources": project.media.iter().map(|media| {
            let mut source = json!({
                "id": media.id.as_str(),
                "path": media.path,
                "duration": media.duration.seconds(),
                "rate": media.rate.to_string(),
                "fps": media.rate.fps(),
                "hasAudio": media.audio.is_some(),
            });
            if let Some(video) = media.video {
                source["width"] = json!(video.width);
                source["height"] = json!(video.height);
            }
            source
        }).collect::<Vec<_>>(),
        "sequences": project.sequences.iter().map(|sequence| {
            json!({"id": sequence.id.as_str(), "name": sequence.name})
        }).collect::<Vec<_>>(),
        "properties": properties_json(),
        "maskProperties": Catalog::builtin().mask_props().iter().map(param_def_json).collect::<Vec<_>>(),
        "maskModes": MaskMode::ALL.map(MaskMode::name),
        "generators": Catalog::builtin()
            .all()
            .filter(|def| def.kind == FxKind::Generator)
            .map(|def| (def.id.to_string(), def.params.iter().map(param_def_json).collect()))
            .collect::<Map<String, Json>>(),
        "tracks": [],
        "markers": [],
    });
    let Some(sequence) = project.sequences.first() else {
        value["fps"] = json!(30.0);
        value["duration"] = json!(0.0);
        return value;
    };
    let rate = sequence.rate;
    value["sequence"] = json!(sequence.id.as_str());
    value["fps"] = json!(rate.fps());
    value["rate"] = json!(rate.to_string());
    value["width"] = json!(sequence.width);
    value["height"] = json!(sequence.height);
    value["start"] = json!(sequence.start.seconds());
    value["duration"] = json!(sequence.duration().seconds());
    value["tracks"] = sequence
        .tracks
        .iter()
        .map(|track| track_json(project, sequence, track))
        .collect();
    value["markers"] = sequence
        .markers
        .iter()
        .map(|marker| {
            let mut value = json!({
                "id": marker.id.as_str(),
                "time": marker.at.seconds(),
                "frame": frames(rate, marker.at),
                "duration": marker.duration.seconds(),
            });
            if let Some(name) = &marker.name {
                value["label"] = json!(name);
            }
            if let Some(color) = marker.color {
                value["color"] = json!(color.name());
            }
            if let Some(note) = &marker.note {
                value["note"] = json!(note);
            }
            value
        })
        .collect();
    value
}

fn properties_json() -> Json {
    [Domain::Video, Domain::Audio]
        .into_iter()
        .map(|domain| {
            let defs = Catalog::builtin().props(domain).iter();
            (domain.name().to_owned(), defs.map(param_def_json).collect())
        })
        .collect::<Map<String, Json>>()
        .into()
}

fn frames(rate: Rate, time: Time) -> i64 {
    rate.frames_round(time)
}

fn track_json(project: &Project, sequence: &Sequence, track: &Track) -> Json {
    let rate = sequence.rate;
    let mut value = json!({
        "id": track.id.as_str(),
        "type": track.kind.name(),
        "enabled": track.enabled,
        "locked": track.locked,
        "muted": track.muted,
        "clips": track.clips.iter().map(|clip| clip_json(project, rate, clip)).collect::<Vec<_>>(),
        "transitions": track.transitions.iter().map(|transition| {
            let cut = track.clip(transition.from.as_str()).map_or(Time::ZERO, Clip::end);
            let (before, after) = transition.split_at_cut(rate);
            json!({
                "id": transition.id.as_str(),
                "from": transition.from.as_str(),
                "to": transition.to.as_str(),
                "fx": transition.fx.as_str(),
                "version": transition.version,
                "align": transition.align.name(),
                "start": (cut - before).seconds(),
                "end": (cut + after).seconds(),
                "duration": transition.duration.seconds(),
            })
        }).collect::<Vec<_>>(),
    });
    if let Some(name) = &track.name {
        value["name"] = json!(name);
    }
    value
}

fn clip_json(project: &Project, rate: Rate, clip: &Clip) -> Json {
    let mut value = json!({
        "id": clip.id.as_str(),
        "timelineStart": clip.start.seconds(),
        "duration": clip.duration.seconds(),
        "startFrame": frames(rate, clip.start),
        "durationFrames": frames(rate, clip.duration),
        "enabled": clip.enabled,
    });
    let object = value.as_object_mut().expect("object");
    if let Some(name) = &clip.name {
        object.insert("name".into(), json!(name));
    }
    if let Some(link) = &clip.link {
        object.insert("link".into(), json!(link.as_str()));
    }
    if let Some((source_in, source_out)) = clip.source_range() {
        object.insert("in".into(), json!(source_in.seconds()));
        object.insert("out".into(), json!(source_out.seconds()));
        let speed = clip.speed();
        object.insert("speed".into(), json!(speed.value().abs()));
        object.insert("speedRatio".into(), json!(speed.to_string()));
        if speed.is_reverse() {
            object.insert("reverse".into(), json!(true));
        }
    }
    match &clip.content {
        Content::Media { media, .. } => {
            object.insert("kind".into(), json!("media"));
            object.insert("sourceId".into(), json!(media.as_str()));
            if let Some(media) = project.media(media.as_str()) {
                object.insert("sourceRate".into(), json!(media.rate.to_string()));
            }
        }
        Content::Sequence { sequence, .. } => {
            object.insert("kind".into(), json!("sequence"));
            object.insert("sequenceId".into(), json!(sequence.as_str()));
            object
                .entry("name")
                .or_insert_with(|| json!(sequence.as_str()));
        }
        Content::Generator {
            fx,
            version,
            params,
        } => {
            object.insert("kind".into(), json!("generator"));
            object.insert("fx".into(), json!(fx.as_str()));
            object.insert("version".into(), json!(version));
            let name = match params.get("text") {
                Some(Param::Static(Value::Text(text))) => text.clone(),
                _ => fx.to_string(),
            };
            object.entry("name").or_insert_with(|| json!(name));
        }
        Content::Caption { text } => {
            object.insert("kind".into(), json!("caption"));
            object.insert("text".into(), json!(text));
            object.entry("name").or_insert_with(|| json!(text));
        }
    }
    let params = match &clip.content {
        Content::Generator { params, .. } => Some(params),
        _ => None,
    };
    let props: Map<String, Json> = clip
        .props
        .iter()
        .chain(params.into_iter().flatten())
        .map(|(name, param)| (name.clone(), param_json(param)))
        .collect();
    if !props.is_empty() {
        object.insert("props".into(), Json::Object(props));
    }
    if !clip.effects.is_empty() {
        object.insert(
            "effects".into(),
            clip.effects
                .iter()
                .map(|effect| {
                    let params: Map<String, Json> = effect
                        .params
                        .iter()
                        .map(|(name, param)| (name.clone(), param_json(param)))
                        .collect();
                    json!({
                        "fx": effect.fx.as_str(),
                        "version": effect.version,
                        "enabled": effect.enabled,
                        "doc": Catalog::builtin().fx(effect.fx.as_str()).map(|def| def.doc.as_str()),
                        "params": params,
                    })
                })
                .collect(),
        );
    }
    if !clip.masks.is_empty() {
        object.insert("masks".into(), clip.masks.iter().map(mask_json).collect());
    }
    value
}

fn mask_json(mask: &Mask) -> Json {
    let mut value = json!({
        "name": mask.name.as_str(),
        "mode": mask.mode.name(),
        "inverted": mask.inverted,
        "params": mask
            .params
            .iter()
            .map(|(name, param)| (name.clone(), param_json(param)))
            .collect::<Map<String, Json>>(),
    });
    match &mask.shape {
        MaskShape::Rect(rect) => value["rect"] = json!(rect),
        MaskShape::Ellipse(ellipse) => value["ellipse"] = json!(ellipse),
        MaskShape::Polygon { points, closed } => {
            value["points"] = json!(points);
            value["closed"] = json!(closed);
        }
    }
    value
}

/// The picture effects and transitions an editor offers, by category, each
/// with its display name, when to use it, what renders it, and its
/// parameter definitions.
pub fn fx_catalog_json() -> Json {
    let entries: Vec<Json> = Catalog::builtin()
        .all()
        .filter(|fx| fx.domain == Domain::Video && fx.kind != FxKind::Generator)
        .map(|fx| {
            let name = fx
                .doc
                .split_once(": ")
                .map_or_else(|| fx.doc.trim_end_matches('.'), |(name, _)| name);
            json!({
                "id": fx.id.as_str(),
                "kind": fx.kind.name(),
                "category": fx.category,
                "name": name,
                "doc": fx.doc,
                "engine": fx.engine.name(),
                "params": fx.params.iter().map(param_def_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json::Array(entries)
}

fn param_def_json(def: &ParamDef) -> Json {
    let mut value = json!({
        "name": def.name,
        "type": def.ty.name(),
        "default": value_json(&def.default),
        "animatable": def.animatable,
        "doc": def.doc,
    });
    if let Some(min) = def.min {
        value["min"] = json!(min);
    }
    if let Some(max) = def.max {
        value["max"] = json!(max);
    }
    if let Some(unit) = &def.unit {
        value["unit"] = json!(unit);
    }
    match &def.ty {
        ParamType::Enum(options) => value["options"] = json!(options),
        ParamType::File(extensions) => value["extensions"] = json!(extensions),
        _ => {}
    }
    value
}

fn param_json(param: &Param) -> Json {
    match param {
        Param::Static(value) => value_json(value),
        Param::Animated(keys) => keys
            .iter()
            .map(|key| {
                json!({
                    "at": key.at.seconds(),
                    "value": value_json(&key.value),
                    "interp": key.interp.name(),
                })
            })
            .collect(),
    }
}

fn value_json(value: &Value) -> Json {
    match value {
        Value::Number(n) => json!(n),
        Value::Bool(b) => json!(b),
        Value::Text(text) | Value::Enum(text) => json!(text),
        Value::Color(color) => json!(color.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_keep_their_own_spans_beside_transitions() {
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"t\"\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=2s at=0f dur=48f\n\
             \x20     set opacity=0.5 crop_l=0.25\n\
             \x20   clip c2 media=m1 src=10s at=48f dur=24f speed=-1\n\
             \x20   transition t1 from=c1 to=c2 fx=dissolve@1 dur=12f\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=2s at=0f dur=48f\n\
             \x20     set gain=-6\n\
             \x20 track V2 video\n\
             \x20   clip g1 gen=title@1 at=0f dur=24f\n\
             \x20     set y=0.3 text=\"Day One\"\n",
            Catalog::builtin(),
        )
        .unwrap()
        .project;
        let value = timeline_json(&project);
        assert_eq!(value["fps"], 24.0);
        assert_eq!(value["sources"][0]["fps"], 24.0);
        assert_eq!(value["duration"], 3.0);
        let v1 = &value["tracks"][0];
        assert_eq!(v1["clips"][0]["duration"], 2.0);
        assert_eq!(v1["clips"][0]["durationFrames"], 48);
        assert_eq!(v1["clips"][0]["props"]["opacity"], 0.5);
        assert_eq!(v1["clips"][0]["props"]["crop_l"], 0.25);
        assert!(v1["clips"][1].get("props").is_none());
        assert_eq!(v1["clips"][1]["timelineStart"], 2.0);
        assert_eq!(v1["clips"][1]["reverse"], true);
        assert_eq!(v1["transitions"][0]["start"], 1.75);
        assert_eq!(v1["transitions"][0]["end"], 2.25);
        assert_eq!(value["tracks"][1]["clips"][0]["props"]["gain"], -6.0);
        let video = value["properties"]["video"].as_array().unwrap();
        let opacity = video.iter().find(|def| def["name"] == "opacity").unwrap();
        assert_eq!(opacity["type"], "number");
        assert_eq!(opacity["default"], 1.0);
        assert_eq!(opacity["max"], 1.0);
        assert_eq!(opacity["animatable"], true);
        let blend = video.iter().find(|def| def["name"] == "blend").unwrap();
        assert_eq!(blend["options"][0], "normal");
        let gain = &value["properties"]["audio"][0];
        let title = &value["tracks"][2]["clips"][0];
        assert_eq!(title["name"], "Day One");
        assert_eq!(title["props"]["text"], "Day One");
        assert_eq!(title["props"]["y"], 0.3);
        let title = value["generators"]["title"].as_array().unwrap();
        assert!(title
            .iter()
            .any(|def| def["name"] == "color" && def["type"] == "color"));
        assert_eq!(value["generators"]["adjustment"], json!([]));
        assert_eq!(
            (gain["name"].as_str(), gain["unit"].as_str()),
            (Some("gain"), Some("dB"))
        );
    }

    #[test]
    fn masks_carry_their_outlines_in_order() {
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"t\"\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
             \x20     mask k1 rect=0.25,0.25,0.75,0.75 feather=6\n\
             \x20     mask ring ellipse=0.5,0.5,0.25,0.3 mode=none inverted=true\n\
             \x20     mask path points=\"0,0 0.5,1 1,0\" closed=false mode=none\n",
            Catalog::builtin(),
        )
        .unwrap()
        .project;
        let value = timeline_json(&project);
        let masks = &value["tracks"][0]["clips"][0]["masks"];
        assert_eq!(
            masks[0],
            json!({"name": "k1", "rect": [0.25, 0.25, 0.75, 0.75], "mode": "add",
                   "inverted": false, "params": {"feather": 6.0}})
        );
        assert_eq!(masks[1]["ellipse"], json!([0.5, 0.5, 0.25, 0.3]));
        assert_eq!(
            (masks[1]["mode"].as_str(), masks[1]["inverted"].as_bool()),
            (Some("none"), Some(true))
        );
        assert_eq!(
            masks[2]["points"],
            json!([[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]])
        );
        assert_eq!(masks[2]["closed"], false);
        let props = value["maskProperties"].as_array().unwrap();
        assert!(props
            .iter()
            .any(|def| def["name"] == "feather" && def["unit"] == "pixels"));
        assert_eq!(value["maskModes"][0], "add");
        assert!(value["maskModes"]
            .as_array()
            .unwrap()
            .contains(&json!("none")));
    }
}
