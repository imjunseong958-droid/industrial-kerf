//! Schemas and examples for the canonical project command contract.
#[cfg(test)]
use cut_domain::Command;
use serde_json::{json, Value};

const NAMES: &[&str] = &[
    "add_track",
    "remove_track",
    "add_clip",
    "remove_clip",
    "place_clip",
    "trim_clip",
    "split_clip",
    "move_clip",
    "update_clip",
    "set_clip",
    "add_marker",
    "remove_marker",
    "set_metadata",
    "restore_metadata",
    "set_canvas",
    "restore_canvas",
    "add_source",
    "remove_source",
    "add_overlay",
    "remove_overlay",
    "batch",
];

pub fn names() -> &'static [&'static str] {
    NAMES
}

fn id() -> Value {
    json!({"type":"string","minLength":1})
}
fn obj(required: &[&str], properties: Value) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn command_schema(kind: &str) -> Option<Value> {
    let schema = match kind {
        "add_track" => obj(
            &["track"],
            json!({"track": {"type":"object","required":["id","type","clips"],"properties":{"id":id(),"type":{"enum":["video","audio","title","adjustment"]},"name":{"type":"string"},"enabled":{"type":"boolean"},"muted":{"type":"boolean"},"locked":{"type":"boolean"},"clips":{"type":"array"}}}}),
        ),
        "remove_track" => obj(&["trackId"], json!({"trackId":id()})),
        "add_clip" => obj(
            &["trackId", "clip"],
            json!({"trackId":id(),"clip": clip_schema()}),
        ),
        "remove_clip" => obj(&["clipId"], json!({"clipId":id(),"trackId":id()})),
        "place_clip" => obj(
            &["clipId", "timelineFrame"],
            json!({"clipId":id(),"timelineFrame":{"type":"integer","minimum":0},"trackId":id()}),
        ),
        "trim_clip" => obj(
            &["clipId", "edge", "timelineFrame"],
            json!({"clipId":id(),"edge":{"enum":["leading","trailing"]},"timelineFrame":{"type":"integer","minimum":0}}),
        ),
        "split_clip" => obj(
            &["clipId", "atFrame", "newClipId"],
            json!({"clipId":id(),"atFrame":{"type":"integer","minimum":0},"newClipId":id()}),
        ),
        "move_clip" => obj(
            &["clipId"],
            json!({"clipId":id(),"trackId":id(),"toTrackId":id(),"index":{"type":"integer","minimum":0},"toIndex":{"type":"integer","minimum":0}}),
        ),
        "update_clip" => obj(
            &["clipId", "patch"],
            json!({"clipId":id(),"patch":{"type":"object"}}),
        ),
        "set_clip" => obj(
            &["clipId", "clip"],
            json!({"clipId":id(),"clip":clip_schema()}),
        ),
        "add_marker" => obj(
            &["marker"],
            json!({"marker":{"type":"object","required":["id","time"],"properties":{"id":id(),"time":{"type":"number"},"label":{"type":"string"},"color":{"type":"string"},"duration":{"type":"number"}}},"index":{"type":"integer","minimum":0}}),
        ),
        "remove_marker" => obj(&["markerId"], json!({"markerId":id()})),
        "set_metadata" => obj(&["patch"], json!({"patch":{"type":"object"}})),
        "restore_metadata" => obj(&[], json!({"metadata":{"type":["object","null"]}})),
        "set_canvas" => obj(
            &["width", "height"],
            json!({"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0}}),
        ),
        "restore_canvas" => obj(
            &["width", "height"],
            json!({"width":{"type":["number","null"]},"height":{"type":["number","null"]}}),
        ),
        "add_source" => obj(
            &["source"],
            json!({"source":{"type":"object","required":["id","path"],"properties":{"id":id(),"path":{"type":"string","minLength":1},"subs":{"type":"string"},"duration":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"},"hasAudio":{"type":"boolean"}}},"index":{"type":"integer","minimum":0}}),
        ),
        "remove_source" => obj(&["sourceId"], json!({"sourceId":id()})),
        "add_overlay" => obj(
            &["overlay"],
            json!({"overlay":{"type":"object"},"index":{"type":"integer","minimum":0}}),
        ),
        "remove_overlay" => obj(&["overlayId"], json!({"overlayId":id()})),
        "batch" => obj(
            &["commands"],
            json!({"commands":{"type":"array","minItems":1,"items":{"type":"object"}}}),
        ),
        _ => return None,
    };
    Some(schema)
}

fn clip_schema() -> Value {
    json!({"type":"object","required":["id","timelineStart"],"properties":{
        "id":id(),"sourceId":id(),"in":{"type":"number"},"out":{"type":"number"},"timelineStart":{"type":"number"},"duration":{"type":"number"},"speed":{"type":"number"},"volume":{"type":"number"},"opacity":{"type":"number"},"text":{"type":"string"},
        "crop":{"type":"object","required":["x","y","width","height"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"}},"description":"Normalized source rectangle; must stay within the frame."},
        "transform":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"z":{"type":"number"},"scale":{"type":"number"},"scaleX":{"type":"number"},"scaleY":{"type":"number"},"rotation":{"type":"number"},"rotationX":{"type":"number"},"rotationY":{"type":"number"},"anchorX":{"type":"number"},"anchorY":{"type":"number"},"position":{},"anchor":{}}},
        "blend":{"type":"string"},"blendMode":{"type":"string"},"title":{},
        "effects":{"type":"array","items":{"type":"object","required":["type"],"properties":{"type":{"type":"string"},"enabled":{"type":"boolean"},"params":{"type":"object"}}},"description":"Use timeline.check to verify the requested effect is supported by preview and export."},
        "keyframes":{"type":"array","items":{"type":"object","required":["property","time","value"],"properties":{"id":{"type":"string"},"property":{"type":"string"},"time":{"type":"number"},"value":{},"easing":{"type":"string"}}}},
        "transitions":{"type":"array","items":{"type":"object","required":["type","duration"],"properties":{"type":{"type":"string"},"duration":{"type":"number"},"alignment":{"enum":["in","out","center"]},"params":{"type":"object"}}}},
        "gaps":{"type":"array","items":{"type":"object","required":["start","duration"],"properties":{"start":{"type":"number"},"duration":{"type":"number"},"label":{"type":"string"}}}}
    }})
}

pub fn schema(kind: &str) -> Option<Value> {
    let mut schema = command_schema(kind)?;
    schema["properties"]["type"] = json!({"const":kind});
    schema["required"].as_array_mut()?.push(json!("type"));
    if matches!(kind, "add_track" | "add_clip") {
        schema["properties"]["index"] = json!({"type":"integer","minimum":0});
    }
    if kind == "add_track" {
        schema["properties"]["track"]["properties"]["clips"]["items"] = clip_schema();
    }
    if kind == "update_clip" {
        let mut patch = clip_schema();
        patch["required"] = json!([]);
        patch["description"] = json!("Patch existing clip fields; omitted fields remain unchanged. Recheck timeline.check for live/offline support.");
        schema["properties"]["patch"] = patch;
    }
    if kind == "add_overlay" {
        schema["properties"]["overlay"] = json!({"type":"object","properties":{
            "id":id(),"type":{"type":"string","description":"captions for inline cues; subtitles for a local subtitle path. Other overlay kinds must pass timeline.check."},
            "path":{"type":"string"},"start":{"type":"number","minimum":0},"duration":{"type":"number","exclusiveMinimum":0},
            "cues":{"type":"array","items":{"type":"object","required":["start","end","text"],"properties":{"start":{"type":"number"},"end":{"type":"number"},"text":{"type":"string"}}}},
            "style":{"type":"object","properties":{"fontSize":{"type":"number"},"fontColor":{"type":"string"},"font":{"type":"string"}}}
        }});
    }
    if kind == "batch" {
        schema["properties"]["commands"]["maxItems"] = json!(256);
    }
    Some(schema)
}

pub fn example(kind: &str) -> Option<Value> {
    let value = match kind {
        "add_track" => {
            json!({"type":"add_track","track":{"id":"track-1","type":"video","clips":[]}})
        }
        "remove_track" => json!({"type":"remove_track","trackId":"track-1"}),
        "add_clip" => {
            json!({"type":kind,"trackId":"track-1","clip":{"id":"clip-1","sourceId":"source-1","in":0.0,"out":1.0,"timelineStart":0.0}})
        }
        "set_clip" => {
            json!({"type":kind,"clipId":"clip-1","clip":{"id":"clip-1","sourceId":"source-1","in":0.0,"out":1.0,"timelineStart":0.0}})
        }
        "remove_clip" => json!({"type":kind,"clipId":"clip-1"}),
        "place_clip" => json!({"type":kind,"clipId":"clip-1","timelineFrame":0}),
        "trim_clip" => json!({"type":kind,"clipId":"clip-1","edge":"trailing","timelineFrame":10}),
        "split_clip" => json!({"type":kind,"clipId":"clip-1","atFrame":10,"newClipId":"clip-2"}),
        "move_clip" => json!({"type":kind,"clipId":"clip-1","toIndex":0}),
        "update_clip" => json!({"type":kind,"clipId":"clip-1","patch":{"volume":0.5}}),
        "add_marker" => json!({"type":kind,"marker":{"id":"marker-1","time":0.0}}),
        "remove_marker" => json!({"type":kind,"markerId":"marker-1"}),
        "set_metadata" => json!({"type":kind,"patch":{}}),
        "restore_metadata" => json!({"type":kind,"metadata":null}),
        "set_canvas" => json!({"type":kind,"width":1920.0,"height":1080.0}),
        "restore_canvas" => json!({"type":kind,"width":null,"height":null}),
        "add_source" => json!({"type":kind,"source":{"id":"source-1","path":"media.mp4"}}),
        "remove_source" => json!({"type":kind,"sourceId":"source-1"}),
        "add_overlay" => {
            json!({"type":kind,"overlay":{"id":"captions-1","type":"captions","cues":[{"start":0.0,"end":1.0,"text":"Hello"}]}})
        }
        "remove_overlay" => json!({"type":kind,"overlayId":"overlay-1"}),
        "batch" => json!({"type":kind,"commands":[{"type":"remove_clip","clipId":"clip-1"}]}),
        _ => return None,
    };
    Some(value)
}

#[cfg(test)]
fn parse_example(kind: &str) -> Option<Command> {
    example(kind).and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Exhaustive matching makes a new domain variant require a catalog review.
    fn kind(command: &Command) -> &'static str {
        match command {
            Command::AddTrack { .. } => "add_track",
            Command::RemoveTrack { .. } => "remove_track",
            Command::AddClip { .. } => "add_clip",
            Command::RemoveClip { .. } => "remove_clip",
            Command::PlaceClip { .. } => "place_clip",
            Command::TrimClip { .. } => "trim_clip",
            Command::SplitClip { .. } => "split_clip",
            Command::MoveClip { .. } => "move_clip",
            Command::UpdateClip { .. } => "update_clip",
            Command::SetClip { .. } => "set_clip",
            Command::AddMarker { .. } => "add_marker",
            Command::RemoveMarker { .. } => "remove_marker",
            Command::SetMetadata { .. } => "set_metadata",
            Command::RestoreMetadata { .. } => "restore_metadata",
            Command::SetCanvas { .. } => "set_canvas",
            Command::RestoreCanvas { .. } => "restore_canvas",
            Command::AddSource { .. } => "add_source",
            Command::RemoveSource { .. } => "remove_source",
            Command::AddOverlay { .. } => "add_overlay",
            Command::RemoveOverlay { .. } => "remove_overlay",
            Command::Batch { .. } => "batch",
        }
    }
    #[test]
    fn every_command_has_parseable_example() {
        for name in names() {
            let schema = schema(name).unwrap();
            let example = example(name).unwrap();
            assert_eq!(kind(&parse_example(name).unwrap()), *name);
            assert_eq!(schema["properties"]["type"]["const"], *name);
            for required in schema["required"].as_array().unwrap() {
                assert!(
                    example.get(required.as_str().unwrap()).is_some(),
                    "{name}: missing {required}"
                );
            }
            for key in example.as_object().unwrap().keys() {
                assert!(
                    schema["properties"].get(key).is_some(),
                    "{name}: unadvertised {key}"
                );
            }
        }
    }
}
