use cut_engine::compile_v1::{
    atempo_filters, compile_filter_graph_v1, timeline_duration_v1, CompileV1Error,
};
use cut_engine::overlay::{captions_filter, CaptionCue};
use regex::Regex;
use serde_json::json;

fn two_clip_timeline() -> serde_json::Value {
    json!({
        "version": 1,
        "fps": 30,
        "sources": [
            { "id": "a", "path": "/tmp/a.mp4" },
            { "id": "b", "path": "/tmp/b.mp4" }
        ],
        "tracks": [{
            "id": "t1",
            "clips": [
                { "src": "a", "in": 0, "out": 2, "id": "c1" },
                { "src": "b", "in": 5, "out": 8, "speed": 2, "volume": 0.5, "id": "c2" }
            ]
        }],
        "overlays": []
    })
}

fn err_message(result: Result<cut_engine::compile_v1::CompiledV1FilterGraph, CompileV1Error>) -> String {
    match result {
        Ok(_) => panic!("expected error"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn contains_trim_filters_for_both_clips() {
    let timeline = two_clip_timeline();
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert!(compiled.filter_complex.contains("trim=start=0:end=2"));
    assert!(compiled.filter_complex.contains("atrim=start=5:end=8"));
}

#[test]
fn contains_concat_filter_across_track_clips() {
    let timeline = two_clip_timeline();
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert!(compiled.filter_complex.contains("concat=n=2:v=1:a=1"));
}

#[test]
fn applies_setpts_atempo_and_volume_for_speed_and_volume() {
    let timeline = two_clip_timeline();
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert!(compiled.filter_complex.contains("setpts=0.5*PTS"));
    assert!(compiled.filter_complex.contains("atempo=2"));
    assert!(compiled.filter_complex.contains("volume=0.5"));
}

#[test]
fn produces_argv_starting_with_ffmpeg_and_array_safe_args() {
    let timeline = two_clip_timeline();
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert_eq!(compiled.argv.first().map(String::as_str), Some("ffmpeg"));
    let fc_idx = compiled
        .argv
        .iter()
        .position(|arg| arg == "-filter_complex")
        .expect("filter_complex flag");
    assert!(compiled.argv.get(fc_idx + 1).is_some());
    assert!(!compiled.command.contains("&&"));
}

#[test]
fn throws_on_unknown_source_id() {
    let mut timeline = two_clip_timeline();
    timeline["tracks"][0]["clips"][0]["src"] = json!("nope");
    let err = err_message(compile_filter_graph_v1(&timeline, &[]));
    assert!(err.contains("Unknown source"));
}

#[test]
fn computes_duration_accounting_for_speed() {
    let timeline = two_clip_timeline();
    let duration = timeline_duration_v1(&timeline).expect("duration");
    assert!((duration - 3.5).abs() < f64::EPSILON);
}

#[test]
fn allocates_independent_labels_when_same_source_is_reused() {
    let timeline = json!({
        "version": 1,
        "fps": 30,
        "sources": [{ "id": "same", "path": "/tmp/repeated.mp4", "hasAudio": false }],
        "tracks": [{
            "id": "t1",
            "clips": [
                { "id": "one", "src": "same", "in": 0, "out": 1 },
                { "id": "two", "src": "same", "in": 1, "out": 2 }
            ]
        }],
        "overlays": []
    });
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    let clipv0 = Regex::new(r"\[0:v\][^;]*\[clipv_0\]").expect("regex");
    let clipv2 = Regex::new(r"\[0:v\][^;]*\[clipv_2\]").expect("regex");
    assert!(clipv0.is_match(&compiled.filter_complex));
    assert!(clipv2.is_match(&compiled.filter_complex));
    assert!(compiled.filter_complex.contains("anullsrc=r=48000:cl=stereo"));
    assert!(compiled.filter_complex.contains("concat=n=2:v=1:a=1"));
}

#[test]
fn skips_empty_tracks_and_rejects_timeline_with_no_clips() {
    let timeline = json!({
        "version": 1,
        "fps": 30,
        "sources": [{ "id": "a", "path": "/tmp/a.mp4" }],
        "tracks": [
            { "id": "empty", "clips": [] },
            { "id": "used", "clips": [{ "id": "c", "src": "a", "in": 0, "out": 1 }] }
        ],
        "overlays": []
    });
    compile_filter_graph_v1(&timeline, &[]).expect("compile with empty track");
    let empty_only = json!({
        "version": 1,
        "fps": 30,
        "sources": [{ "id": "a", "path": "/tmp/a.mp4" }],
        "tracks": [{ "id": "empty", "clips": [] }],
        "overlays": []
    });
    let err = err_message(compile_filter_graph_v1(&empty_only, &[]));
    assert!(err.to_lowercase().contains("empty timeline"));
}

#[test]
fn uses_known_dimensions_for_canvas_and_emits_safe_crop() {
    let timeline = json!({
        "version": 1,
        "fps": 24,
        "sources": [{ "id": "a", "path": "/tmp/a.mp4", "width": 320, "height": 240 }],
        "tracks": [{
            "id": "t",
            "clips": [{
                "id": "c",
                "src": "a",
                "in": 0,
                "out": 1,
                "crop": { "x": 0.25, "y": 0.1, "width": 0.5, "height": 0.75 }
            }]
        }],
        "overlays": []
    });
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert!(compiled.filter_complex.contains("crop=w="));
    assert!(compiled.filter_complex.contains("scale=320:240:"));
    assert!(!compiled.filter_complex.contains("scale=1920:1080"));
}

#[test]
fn uses_largest_complete_source_aspect_for_portrait_landscape_mixes() {
    let timeline = json!({
        "version": 1,
        "fps": 30,
        "sources": [
            { "id": "landscape", "path": "/tmp/landscape.mp4", "width": 1280, "height": 720 },
            { "id": "portrait", "path": "/tmp/portrait.mp4", "width": 1080, "height": 1920 }
        ],
        "tracks": [{
            "id": "t",
            "clips": [
                { "id": "l", "src": "landscape", "in": 0, "out": 1 },
                { "id": "p", "src": "portrait", "in": 0, "out": 1 }
            ]
        }],
        "overlays": []
    });
    let compiled = compile_filter_graph_v1(&timeline, &[]).expect("compile");
    assert!(compiled.filter_complex.contains("scale=1080:1920:"));
    assert!(!compiled.filter_complex.contains("scale=1280:1920:"));
    assert!(!compiled.filter_complex.contains("scale=1920:1920:"));
}

#[test]
fn keeps_atempo_factors_within_ffmpeg_bounds_for_arbitrary_speeds() {
    for speed in [0.1, 0.25, 0.5, 2.0, 100.0, 10_000.0] {
        let factors = atempo_filters(speed)
            .expect("atempo")
            .into_iter()
            .map(|filter| {
                filter
                    .strip_prefix("atempo=")
                    .expect("atempo prefix")
                    .parse::<f64>()
                    .expect("factor")
            })
            .collect::<Vec<_>>();
        assert!(factors.iter().all(|factor| (0.5..=100.0).contains(factor)));
        let product = factors.iter().product::<f64>();
        assert!((product - speed).abs() < 1e-8 || (product - speed).abs() / speed < 1e-8);
    }
}

#[test]
fn escapes_timed_caption_text_and_maps_style_position() {
    let cue = CaptionCue {
        start: 1.0,
        end: 2.5,
        text: "A:b,c; it's 100%".to_string(),
    };
    let filter = captions_filter(
        &cue,
        Some(&json!({
            "fontSize": 22,
            "fontColor": "#ffffff",
            "position": "top",
            "marginV": 18
        })),
    );
    assert!(filter.contains("drawtext=text="));
    assert!(filter.contains("between(t,1,2.5)"));
    assert!(filter.contains("fontsize=22"));
    assert!(filter.contains("fontcolor=0xffffff"));
    assert!(filter.contains("y='18'"));
    assert!(filter.contains("A\\\\:b"));
    assert!(filter.contains("100\\\\%"));
}

#[test]
fn draft_post_filters_scale_and_fps() {
    let timeline = two_clip_timeline();
    let compiled = compile_filter_graph_v1(
        &timeline,
        &["scale=-2:360".to_string(), "fps=15".to_string()],
    )
    .expect("compile");
    assert!(compiled.filter_complex.contains("scale=-2:360"));
    assert!(compiled.filter_complex.contains("fps=15"));
}
