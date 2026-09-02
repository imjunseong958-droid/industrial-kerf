use cut_core::capability::{capability, CapabilitySupport, EditCapability};
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
fn exact_offline_geometry_declared_by_the_matrix_compiles() {
    let graph = compile_filter_graph_v2(
        &project(json!({
            "id": "clip-a",
            "sourceId": "source-a",
            "in": 1,
            "out": 5,
            "timelineStart": 2,
            "speed": 2,
            "volume": 0.5,
            "crop": {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8},
            "opacity": 0.75
        })),
        CompileV2Options::default(),
    )
    .expect("declared exact offline features compile");

    for feature in [
        EditCapability::Placement,
        EditCapability::SourceRange,
        EditCapability::Speed,
        EditCapability::Volume,
        EditCapability::Crop,
        EditCapability::Opacity,
    ] {
        assert_eq!(capability(feature).offline, CapabilitySupport::Exact);
    }
    assert!(graph.filter_complex.contains("crop="));
    assert!(graph.filter_complex.contains("colorchannelmixer=aa=0.75"));
}

#[test]
fn limited_effect_support_rejects_unknown_effects() {
    let error = compile_filter_graph_v2(
        &project(json!({
            "id": "clip-a",
            "sourceId": "source-a",
            "in": 0,
            "out": 2,
            "timelineStart": 0,
            "effects": [{"type": "blur", "params": {}}]
        })),
        CompileV2Options::default(),
    )
    .expect_err("unknown effects must not silently compile");

    assert_eq!(
        capability(EditCapability::Effects).offline,
        CapabilitySupport::Limited
    );
    assert_eq!(error.feature(), Some("effect"));
}

#[test]
fn newly_exposed_visual_properties_have_preview_export_parity_contract() {
    let root = tempdir().expect("temporary media root");
    fs::write(root.path().join("source-a.mp4"), b"placeholder").expect("temporary source");
    let project = project(json!({
        "id": "clip-a",
        "sourceId": "source-a",
        "in": 0,
        "out": 4,
        "timelineStart": 0,
        "transform": {
            "x": 0.1,
            "y": -0.1,
            "scale": 1.1,
            "scaleX": 0.9,
            "scaleY": 1.2,
            "rotation": 4
        },
        "opacity": 0.75,
        "effects": [{
            "type": "color",
            "params": {
                "exposure": 0.2,
                "contrast": 1.1,
                "brightness": 0.1,
                "saturation": 1.2,
                "temperature": 0.1,
                "tint": 0.1,
                "gamma": 1.1,
                "highlights": 0.1,
                "shadows": -0.1,
                "vibrance": 0.2,
                "mix": 1
            }
        }],
        "keyframes": [
            {"property": "opacity", "time": 0, "value": 0.5, "easing": "linear"},
            {"property": "opacity", "time": 2, "value": 0.75, "easing": "linear"}
        ]
    }));
    let graph = compile_filter_graph_v2(&project, CompileV2Options::default())
        .expect("inspector-exposed visual properties compile offline");
    assert!(graph.filter_complex.contains("scale="));
    assert!(graph.filter_complex.contains("rotate="));
    assert!(graph.filter_complex.contains("geq="));
    assert!(graph.filter_complex.contains("eq="));
    let plan = analyze_project(&project, Some(root.path()), None);
    assert_eq!(plan.mode, PreviewMode::RenderedFallback, "{plan:?}");
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
