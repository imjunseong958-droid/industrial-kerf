use cut_core::compile::{compile_filter_graph_v2, CompileV2Options};
use cut_core::preview::{analyze_project, PreviewMode};
use serde_json::json;
use std::fs;
use tempfile::tempdir;

fn project(clip: serde_json::Value) -> serde_json::Value {
    json!({
        "version": 2,
        "fps": 30,
        "width": 1920,
        "height": 1080,
        "sources": [{
            "id": "source-a",
            "path": "source-a.mp4",
            "duration": 10,
            "width": 1920,
            "height": 1080,
            "hasAudio": true
        }],
        "tracks": [{"id": "video-1", "type": "video", "clips": [clip]}],
        "overlays": [],
        "gaps": [],
        "transitions": [],
        "effects": [],
        "markers": []
    })
}

#[test]
fn newly_exposed_transitions_require_overlap_and_compile_with_fallback_preview() {
    let root = tempdir().expect("temporary media root");
    fs::write(root.path().join("source-a.mp4"), b"placeholder").expect("temporary source");
    let project = project(json!({
        "id": "clip-a",
        "sourceId": "source-a",
        "in": 0,
        "out": 4,
        "timelineStart": 0,
        "duration": 4,
    }));
    let mut project = project;
    project["tracks"][0]["clips"] = json!([
        project["tracks"][0]["clips"][0].clone(),
        {
            "id": "clip-b",
            "sourceId": "source-a",
            "in": 4,
            "out": 8,
            "timelineStart": 3,
            "duration": 4,
            "transitions": [{"type": "fade", "duration": 1, "alignment": "center"}]
        }
    ]);
    let graph = compile_filter_graph_v2(&project, CompileV2Options::default())
        .expect("overlapping transition compiles offline");
    assert!(graph.filter_complex.contains("xfade=transition=fade"));
    let plan = analyze_project(&project, Some(root.path()), None);
    assert_eq!(plan.mode, PreviewMode::RenderedFallback, "{plan:?}");
}
