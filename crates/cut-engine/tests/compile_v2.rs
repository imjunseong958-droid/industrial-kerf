use cut_engine::compile::{
    compile_filter_graph_v2, timeline_duration_v2, CompileError, CompileV2Options,
};
use regex::Regex;
use serde_json::{json, Value};

fn merge_project(base: Value, overrides: Value) -> Value {
    let mut merged = base;
    if let (Some(base_obj), Some(override_obj)) = (merged.as_object_mut(), overrides.as_object()) {
        for (key, value) in override_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    merged
}

fn project(overrides: Value) -> Value {
    let base = json!({
        "version": 2,
        "fps": 30,
        "width": 320,
        "height": 180,
        "sources": [
            { "id": "a", "path": "/tmp/a.mp4", "width": 320, "height": 180, "hasAudio": true },
            { "id": "b", "path": "/tmp/b.mp4", "width": 320, "height": 180, "hasAudio": true }
        ],
        "tracks": [
            {
                "id": "video-a",
                "type": "video",
                "clips": [{ "id": "a1", "sourceId": "a", "in": 0, "out": 2, "timelineStart": 0 }],
                "gaps": [{ "timelineStart": 2, "duration": 1 }]
            },
            {
                "id": "video-b",
                "type": "video",
                "clips": [{ "id": "b1", "sourceId": "b", "in": 0, "out": 1, "timelineStart": 1 }]
            },
            {
                "id": "audio",
                "type": "audio",
                "clips": [{ "id": "a2", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 0, "volume": 0.5 }]
            }
        ]
    });
    merge_project(base, overrides)
}

fn err_message(result: Result<cut_engine::compile::CompiledV2FilterGraph, CompileError>) -> String {
    match result {
        Ok(_) => panic!("expected error"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn composites_positioned_video_tracks_and_mixes_audio_tracks() {
    let compiled =
        compile_filter_graph_v2(&project(json!({})), CompileV2Options::default()).expect("compile");
    assert_eq!(compiled.duration, 3.0);
    assert!(compiled.filter_complex.contains("overlay="));
    assert!(compiled.filter_complex.contains("adelay=0:all=1"));
    assert!(compiled.filter_complex.contains("amix=inputs=2"));
    assert!(!compiled.filter_complex.contains("[1:a]"));
    assert!(compiled.filter_complex.contains("d=3"));
    assert!(!compiled.filter_complex.contains("concat="));
}

#[test]
fn uses_silence_stream_when_direct_compilation_lacks_audio_metadata() {
    let parsed = project(json!({
        "sources": [{ "id": "video", "path": "/tmp/video-only.mp4", "width": 320, "height": 180 }],
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{ "id": "clip", "sourceId": "video", "in": 0, "out": 1, "timelineStart": 0 }]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(!filter.contains("[0:a]"));
    assert!(filter.contains("aevalsrc=0:c=stereo"));
}

#[test]
fn emits_opacity_normalized_crop_transform_and_linear_keyframe_expressions() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 2,
                "timelineStart": 1,
                "opacity": 0.75,
                "crop": { "x": 0.1, "y": 0.2, "width": 0.5, "height": 0.6 },
                "transform": { "x": 0.2, "y": -0.1, "scale": 0.8, "rotation": 4 },
                "keyframes": [
                    { "property": "opacity", "time": 0, "value": 0 },
                    { "property": "opacity", "time": 2, "value": 1 },
                    { "property": "transform.x", "time": 0, "value": 0 },
                    { "property": "transform.x", "time": 2, "value": 0.2 }
                ]
            }]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("crop=w="));
    assert!(filter.contains("geq=r='r(X,Y)'"));
    assert!(filter.contains("if(lt(T\\,2)"));
    assert!(filter.contains("scale=w="));
    assert!(filter.contains("rotate="));
    assert!(filter.contains("main_w*(if(lt((t-1)\\,2)"));
}

#[test]
fn accounts_for_explicit_placements_and_gaps_in_duration() {
    let parsed = project(json!({
        "duration": null,
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{ "id": "c", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 4 }],
            "gaps": [{ "start": 0, "duration": 2 }]
        }]
    }));
    assert_eq!(timeline_duration_v2(&parsed).expect("duration"), 5.0);
    assert_eq!(
        compile_filter_graph_v2(&parsed, CompileV2Options::default())
            .expect("compile")
            .duration,
        5.0
    );
}

#[test]
fn includes_project_level_gaps_in_duration_and_rejects_project_effects_transitions() {
    let parsed = project(json!({
        "duration": 1,
        "gaps": [{ "timelineStart": 3, "duration": 1 }]
    }));
    let compiled = compile_filter_graph_v2(&parsed, CompileV2Options::default()).expect("compile");
    assert_eq!(compiled.duration, 4.0);
    assert!(compiled.filter_complex.contains("d=4"));

    let effects = project(json!({ "effects": [{ "type": "blur" }] }));
    let msg = err_message(compile_filter_graph_v2(
        &effects,
        CompileV2Options::default(),
    ));
    assert!(Regex::new(r"(?i)effect.*project").unwrap().is_match(&msg));

    let transitions = project(json!({ "transitions": [{ "type": "fade", "duration": 0.5 }] }));
    let msg = err_message(compile_filter_graph_v2(
        &transitions,
        CompileV2Options::default(),
    ));
    assert!(Regex::new(r"(?i)project transition")
        .unwrap()
        .is_match(&msg));
}

#[test]
fn trims_explicitly_shorter_logical_clip_duration_after_speed() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 2,
                "duration": 0.5,
                "timelineStart": 0
            }]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("trim=duration=0.5,setpts=PTS-STARTPTS"));
}

#[test]
fn rejects_unsupported_effects_instead_of_dropping_them() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 1,
                "timelineStart": 0,
                "effects": [{ "type": "blur" }]
            }]
        }]
    }));
    let err = compile_filter_graph_v2(&parsed, CompileV2Options::default()).unwrap_err();
    assert!(matches!(err, CompileError::UnsupportedFeature { .. }));
    assert!(Regex::new(r"(?i)effect.*blur")
        .unwrap()
        .is_match(&err.to_string()));
}

#[test]
fn treats_identity_color_grade_as_no_op() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 1,
                "timelineStart": 0,
                "effects": [{ "type": "color" }]
            }]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(!filter.contains("exposure="));
    assert!(!filter.contains("eq="));
    assert!(!filter.contains("colorbalance="));
    assert!(!filter.contains("vibrance="));
}

#[test]
fn compiles_color_controls_before_opacity_composition() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 1,
                "timelineStart": 0,
                "opacity": 0.75,
                "effects": [{
                    "type": "color",
                    "params": {
                        "exposure": 1,
                        "contrast": 1.2,
                        "brightness": 0.1,
                        "saturation": 1.3,
                        "temperature": 320,
                        "tint": 0.1,
                        "gamma": 1.2,
                        "highlights": 0.2,
                        "shadows": -0.1,
                        "vibrance": 0.4,
                        "mix": 1
                    }
                }]
            }]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("exposure=exposure=1"));
    assert!(filter.contains("eq=contrast=1.2:brightness=0.1:saturation=1.3:gamma=1.2"));
    assert!(filter.contains("colorbalance="));
    assert!(filter.contains("vibrance=intensity=0.4"));
    assert!(filter.find("vibrance=") < filter.find("colorchannelmixer=aa=0.75"));
}

#[test]
fn validates_color_ranges_and_rejects_unsupported_lut_paths() {
    let invalid = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 1,
                "timelineStart": 0,
                "effects": [{ "type": "color", "params": { "contrast": 9 } }]
            }]
        }]
    }));
    let msg = err_message(compile_filter_graph_v2(
        &invalid,
        CompileV2Options::default(),
    ));
    assert!(Regex::new(r"(?i)contrast").unwrap().is_match(&msg));

    let with_lut = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{
                "id": "c",
                "sourceId": "a",
                "in": 0,
                "out": 1,
                "timelineStart": 0,
                "effects": [{ "type": "color", "params": { "lut": "/tmp/look.cube" } }]
            }]
        }]
    }));
    let msg = err_message(compile_filter_graph_v2(
        &with_lut,
        CompileV2Options::default(),
    ));
    assert!(Regex::new(r"(?i)color LUT.*not supported")
        .unwrap()
        .is_match(&msg));
}

#[test]
fn supports_only_validated_overlapping_fade_crossfade_dissolve_transitions() {
    let parsed = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [
                {
                    "id": "a1",
                    "sourceId": "a",
                    "in": 0,
                    "out": 2,
                    "timelineStart": 0,
                    "transitions": [{ "type": "crossfade", "duration": 0.5 }]
                },
                { "id": "b1", "sourceId": "b", "in": 0, "out": 1.5, "timelineStart": 1.5 }
            ]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("xfade=transition=fade:duration=0.5:offset=1.5"));

    let unsupported = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [
                {
                    "id": "a1",
                    "sourceId": "a",
                    "in": 0,
                    "out": 1,
                    "timelineStart": 0,
                    "transitions": [{ "type": "wipeleft", "duration": 0.25 }]
                },
                { "id": "b1", "sourceId": "b", "in": 0, "out": 1, "timelineStart": 0.75 }
            ]
        }]
    }));
    let msg = err_message(compile_filter_graph_v2(
        &unsupported,
        CompileV2Options::default(),
    ));
    assert!(Regex::new(r"(?i)transition").unwrap().is_match(&msg));
}

#[test]
fn normalizes_differing_transition_source_dimensions_before_xfade() {
    let parsed = project(json!({
        "width": 640,
        "height": 360,
        "sources": [
            { "id": "a", "path": "/tmp/a.mp4", "width": 640, "height": 360, "hasAudio": true },
            { "id": "b", "path": "/tmp/b.mp4", "width": 1280, "height": 720, "hasAudio": true }
        ],
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [
                {
                    "id": "a1",
                    "sourceId": "a",
                    "in": 0,
                    "out": 2,
                    "timelineStart": 0,
                    "transitions": [{ "type": "fade", "duration": 0.5 }]
                },
                { "id": "b1", "sourceId": "b", "in": 0, "out": 1.5, "timelineStart": 1.5 }
            ]
        }]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert_eq!(
        filter
            .matches("scale=640:360:force_original_aspect_ratio=decrease")
            .count(),
        4
    );
    assert!(filter.contains("fps=30,format=yuv420p"));
    assert!(filter.contains("xfade=transition=fade"));
}

#[test]
fn preserves_migrated_subtitle_and_inline_caption_overlays() {
    let parsed = project(json!({
        "overlays": [
            { "id": "s", "type": "subtitles", "path": "/tmp/subs.srt" },
            {
                "id": "c",
                "type": "captions",
                "cues": [{ "start": 0.25, "end": 0.75, "text": "Hello" }]
            }
        ]
    }));
    let filter = compile_filter_graph_v2(&parsed, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("subtitles=filename='/tmp/subs.srt'"));
    assert!(filter.contains("drawtext=text=Hello"));
    assert!(filter.contains("between(t,0.25,0.75)"));
}

#[test]
fn uses_embedded_video_audio_only_when_no_explicit_audio_track_exists() {
    let video_only = project(json!({
        "tracks": [{
            "id": "video",
            "type": "video",
            "clips": [{ "id": "v", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 0 }]
        }]
    }));
    let embedded = compile_filter_graph_v2(&video_only, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(embedded.contains("[0:a]"));

    let with_explicit = project(json!({
        "tracks": [
            {
                "id": "video",
                "type": "video",
                "clips": [{ "id": "v", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 0 }]
            },
            {
                "id": "audio",
                "type": "audio",
                "clips": [{ "id": "a", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 0 }]
            }
        ]
    }));
    let explicit = compile_filter_graph_v2(&with_explicit, CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert_eq!(explicit.matches("[0:a]").count(), 1);

    let opted_in = compile_filter_graph_v2(
        &with_explicit,
        CompileV2Options {
            include_embedded_audio: Some(true),
            ..CompileV2Options::default()
        },
    )
    .expect("compile")
    .filter_complex;
    assert_eq!(opted_in.matches("[0:a]").count(), 2);
}

#[test]
fn identity_clips_composite_in_yuv_at_canvas_size() {
    let filter = compile_filter_graph_v2(&project(json!({})), CompileV2Options::default())
        .expect("compile")
        .filter_complex;
    assert!(filter.contains("format=yuv420p"));
    assert!(filter.contains("scale=320:180:force_original_aspect_ratio=decrease"));
    assert!(filter.contains("enable='between(t,"));
}

#[test]
fn range_compile_seeks_inputs_and_shortens_the_canvas() {
    let compiled = compile_filter_graph_v2(
        &project(json!({})),
        CompileV2Options {
            range: Some(cut_engine::compile::CompileRange {
                start: 1.0,
                duration: 1.0,
            }),
            ..CompileV2Options::default()
        },
    )
    .expect("compile");
    assert_eq!(compiled.duration, 1.0);
    assert!(compiled.filter_complex.contains("d=1"));
    assert!(compiled.argv.windows(2).any(|pair| pair[0] == "-ss"));
    assert!(compiled.argv.windows(2).any(|pair| pair[0] == "-t"));
    let filter_pos = compiled
        .argv
        .iter()
        .position(|arg| arg == "-filter_complex")
        .expect("filter");
    let ss_after_filter = compiled.argv[filter_pos..]
        .windows(2)
        .any(|pair| pair[0] == "-ss");
    assert!(!ss_after_filter);
}

#[test]
fn sparse_same_source_clips_get_separate_inputs() {
    let compiled = compile_filter_graph_v2(
        &project(json!({
            "tracks": [{
                "id": "video",
                "type": "video",
                "clips": [
                    { "id": "early", "sourceId": "a", "in": 0, "out": 1, "timelineStart": 0 },
                    { "id": "late", "sourceId": "a", "in": 90, "out": 91, "timelineStart": 2 }
                ]
            }]
        })),
        CompileV2Options::default(),
    )
    .expect("compile");
    let input_count = compiled.argv.iter().filter(|arg| *arg == "-i").count();
    assert_eq!(input_count, 2, "each source range should be its own -i");
    assert!(compiled.argv.windows(2).any(|pair| pair == ["-ss", "90"]));
    assert!(compiled.filter_complex.contains("concat="));
    assert!(!compiled.filter_complex.contains("overlay="));
}
