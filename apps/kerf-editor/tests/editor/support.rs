//! What several of the editor's test modules build their cases from: media, clips, timelines and
//! snapshots as the bridge sends them, the editor state they leave, keys, and window bounds.

use gpui::{point, px, size, Bounds, Keystroke, Pixels, Size};
use kerf_editor::model::Source;
use kerf_editor::store::EditorState;
use kerf_editor::ui::{menu_key, MenuKey};
use serde_json::{json, Value};

pub(crate) fn source(id: &str, path: &str) -> Source {
    Source {
        id: id.to_owned(),
        path: path.to_owned(),
        ..Source::default()
    }
}

pub(crate) fn snapshot_timeline() -> Value {
    json!({
        "fps": 24.0,
        "sources": [
            {"id": "a", "path": "media/Beach Day-1a2b3c4d.mov", "duration": 6.0, "width": 1280, "height": 720, "hasAudio": true},
            {"id": "t", "path": "media/tone.wav", "duration": 8.0, "hasAudio": true},
            {"id": "gone", "path": "", "duration": 2.0, "width": 640, "height": 360}
        ],
        "tracks": [
            {"id": "V1", "type": "video", "role": "video", "enabled": true, "clips": [
                {"id": "c1", "kind": "media", "sourceId": "a", "timelineStart": 0.0, "duration": 2.0, "in": 1.0, "out": 3.0, "speed": 1.0},
                {"id": "c2", "kind": "media", "sourceId": "a", "name": "Wide", "timelineStart": 2.0, "duration": 2.0, "in": 0.0, "out": 2.0},
                {"id": "c3", "kind": "media", "sourceId": "gone", "timelineStart": 4.0, "duration": 1.0},
                {"id": "c4", "kind": "title", "text": "Hello", "timelineStart": 5.0, "duration": 1.0}
            ]},
            {"id": "S1", "type": "subtitle", "role": "", "enabled": true, "clips": []},
            {"id": "A1", "type": "audio", "role": "dialogue", "enabled": true, "clips": [
                {"id": "s1", "kind": "media", "sourceId": "t", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 4.0}
            ]},
            {"id": "A2", "type": "audio", "role": "Music", "roleAssigned": true, "enabled": false, "clips": [
                {"id": "s2", "kind": "media", "sourceId": "a", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 4.0}
            ]}
        ]
    })
}

pub(crate) fn clip(id: &str, start: f64, duration: f64) -> Value {
    json!({"id": id, "name": id.to_uppercase(), "timelineStart": start, "duration": duration,
           "startFrame": (start * 25.0) as i64, "durationFrames": (duration * 25.0) as i64, "enabled": true})
}

pub(crate) fn timeline_value(clips: &[Value]) -> Value {
    json!({"name": "Cut", "fps": 25.0, "duration": 30.0,
           "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": clips}]})
}

pub(crate) fn snapshot() -> Value {
    json!({
        "revision": 7,
        "timeline": timeline_value(&[clip("a", 0.0, 4.0), clip("b", 20.0, 4.0)]),
        "proposals": [
            {"proposalId": "p-far", "status": "pending", "freshness": "current", "version": 1,
             "script": "# a comment\nuse main\ntrim a out -12f", "documentDiff": "", "author": {"kind": "kerf", "name": "Kerf"},
             "supersedes": null,
             "versions": [{"version": 1, "proposalId": "p-far", "script": "trim a out -12f", "documentDiff": ""}]},
            {"proposalId": "p-near-2", "status": "pending", "freshness": "current", "version": 2,
             "script": "move b to 21s", "documentDiff": "", "author": {"kind": "mcp", "name": "Claude Code"},
             "supersedes": "p-near-1",
             "versions": [{"version": 1, "proposalId": "p-near-1", "script": "move b to 20.5s", "documentDiff": ""},
                          {"version": 2, "proposalId": "p-near-2", "script": "move b to 21s", "documentDiff": ""}]},
            {"proposalId": "p-done", "status": "accepted", "freshness": "current", "version": 1, "appliedRevision": 6,
             "script": "trim b in +2f", "documentDiff": ""}
        ],
        "proposalTimelines": {
            "p-far": timeline_value(&[clip("a", 0.0, 3.52), clip("b", 20.0, 4.0)]),
            "p-near-1": timeline_value(&[clip("a", 0.0, 4.0), clip("b", 20.5, 4.0)]),
            "p-near-2": timeline_value(&[clip("a", 0.0, 4.0), clip("b", 21.0, 4.0)]),
        }
    })
}

pub(crate) fn apply(state: &mut EditorState, events: &[Value]) {
    for event in events {
        state.apply(event);
    }
}

pub(crate) fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

// ------------------------------------------------------------ the kit's menus

pub(crate) fn key(text: &str) -> MenuKey {
    menu_key(&Keystroke::parse(text).unwrap())
}

pub(crate) const EXISTS: &str = " already exists; choose another name or replace it";

pub(crate) fn bounds(left: f32, top: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(left), px(top)), size(px(width), px(height)))
}

pub(crate) fn window(width: f32, height: f32) -> Size<Pixels> {
    size(px(width), px(height))
}

pub(crate) fn texts(state: &EditorState) -> Vec<String> {
    state
        .notices
        .iter()
        .map(|notice| notice.text.clone())
        .collect()
}

// ------------------------------------------------------------ the preview's hold

pub(crate) fn preview(status: &str) -> Value {
    json!({"type": "preview", "status": status})
}
