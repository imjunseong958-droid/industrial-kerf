//! The timeline that editor snapshots carry.
//!
//! A read-only JSON view of the first sequence of the model for clients that
//! draw a timeline (the Qt app). Times are exact seconds of the model, with
//! frame counts on the sequence rate beside them; clips appear at their own
//! spans, and transitions are listed separately with the region they cover.
//! Edits go back as typed operations (`EditOp`), never by patching this.

use cut_model::{
    Catalog, Clip, Content, Domain, Param, Project, Rate, Sequence, Time, Track, Value,
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
        "clips": track.clips.iter().map(|clip| clip_json(project, rate, track, clip)).collect::<Vec<_>>(),
        "transitions": track.transitions.iter().map(|transition| {
            let cut = track.clip(transition.from.as_str()).map_or(Time::ZERO, Clip::end);
            let (before, after) = transition.split_at_cut(rate);
            json!({
                "id": transition.id.as_str(),
                "from": transition.from.as_str(),
                "to": transition.to.as_str(),
                "fx": transition.fx.as_str(),
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

fn clip_json(project: &Project, rate: Rate, track: &Track, clip: &Clip) -> Json {
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
        Content::Generator { fx, params } => {
            object.insert("kind".into(), json!("generator"));
            object.insert("fx".into(), json!(fx.as_str()));
            let text = match params.get("text") {
                Some(Param::Static(Value::Text(text))) => Some(text.clone()),
                _ => None,
            };
            if let Some(text) = &text {
                object.insert("text".into(), json!(text));
            }
            object
                .entry("name")
                .or_insert_with(|| json!(text.unwrap_or_else(|| fx.to_string())));
        }
        Content::Caption { text } => {
            object.insert("kind".into(), json!("caption"));
            object.insert("text".into(), json!(text));
            object.entry("name").or_insert_with(|| json!(text));
        }
    }
    let catalog = Catalog::builtin();
    let number = |name: &str| match clip.props.get(name) {
        Some(Param::Static(Value::Number(n))) => Some(*n),
        _ => None,
    };
    match track.kind.domain() {
        Some(Domain::Video) => {
            if let Some(opacity) = number("opacity") {
                object.insert("opacity".into(), json!(opacity));
            }
            let crop = ["crop_l", "crop_t", "crop_r", "crop_b"].map(number);
            if crop.iter().any(Option::is_some) {
                let [l, t, r, b] = crop.map(|edge| edge.unwrap_or(0.0));
                object.insert(
                    "crop".into(),
                    json!({"x": l, "y": t, "width": 1.0 - l - r, "height": 1.0 - t - b}),
                );
            }
        }
        Some(Domain::Audio) => {
            if let Some(gain) = number("gain") {
                object.insert("gain".into(), json!(gain));
                object.insert("volume".into(), json!(10f64.powf(gain / 20.0)));
            }
        }
        None => {}
    }
    let props: Map<String, Json> = clip
        .props
        .iter()
        .map(|(name, param)| (name.clone(), param_json(param)))
        .collect();
    if !props.is_empty() {
        object.insert("props".into(), Json::Object(props));
    }
    let animated: Vec<&str> = clip
        .props
        .iter()
        .filter(|(_, param)| matches!(param, Param::Animated(_)))
        .map(|(name, _)| name.as_str())
        .collect();
    if !animated.is_empty() {
        object.insert("animated".into(), json!(animated));
    }
    if !clip.effects.is_empty() {
        object.insert(
            "effects".into(),
            clip.effects
                .iter()
                .map(|effect| {
                    json!({
                        "fx": effect.fx.as_str(),
                        "enabled": effect.enabled,
                        "doc": catalog.fx(effect.fx.as_str()).map(|def| def.doc.as_str()),
                    })
                })
                .collect(),
        );
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
        let project = cut_lang::parse_document(
            "cut 1\nproject name=\"t\"\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=2s at=0f dur=48f\n\
             \x20     set opacity=0.5 crop_l=0.25\n\
             \x20   clip c2 media=m1 src=10s at=48f dur=24f speed=-1\n\
             \x20   transition t1 from=c1 to=c2 fx=dissolve dur=12f\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=2s at=0f dur=48f\n\
             \x20     set gain=-6\n",
            Catalog::builtin(),
        )
        .unwrap()
        .project;
        let value = timeline_json(&project);
        assert_eq!(value["fps"], 24.0);
        assert_eq!(value["duration"], 3.0);
        let v1 = &value["tracks"][0];
        assert_eq!(v1["clips"][0]["duration"], 2.0);
        assert_eq!(v1["clips"][0]["durationFrames"], 48);
        assert_eq!(v1["clips"][0]["opacity"], 0.5);
        assert_eq!(v1["clips"][0]["crop"]["width"], 0.75);
        assert_eq!(v1["clips"][1]["timelineStart"], 2.0);
        assert_eq!(v1["clips"][1]["reverse"], true);
        assert_eq!(v1["transitions"][0]["start"], 1.75);
        assert_eq!(v1["transitions"][0]["end"], 2.25);
        let a1 = &value["tracks"][1]["clips"][0];
        assert!((a1["volume"].as_f64().unwrap() - 0.501).abs() < 0.001);
    }
}
