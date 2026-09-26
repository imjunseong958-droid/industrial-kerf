use cut_domain::{apply_commands, parse_command, parse_project, serialize_project};
use cut_engine::ffmpeg::resolve_ffmpeg_bin;
use cut_engine::render::{render_v2, RenderV2Options};
use serde_json::json;
use std::process::Command;

#[test]
fn focused_interval_renders_the_selected_region_and_restores_the_full_frame() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let output = dir.path().join("focused.mp4");
    let ffmpeg = resolve_ffmpeg_bin();
    let generated = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=160x90:r=10:d=3,drawbox=x=80:y=0:w=80:h=90:color=blue:t=fill",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let project = parse_project(&json!({
        "version": 2, "fps": 10, "width": 160, "height": 90,
        "sources": [{"id": "source", "path": source, "duration": 3, "width": 160,
                     "height": 90, "hasAudio": false}],
        "tracks": [{"id": "video", "type": "video", "clips": [
            {"id": "clip", "sourceId": "source", "in": 0, "out": 3, "timelineStart": 0}
        ]}]
    }))
    .unwrap();
    let batch = parse_command(&json!({"type": "batch", "commands": [
        {"type": "split_clip", "clipId": "clip", "atFrame": 10, "newClipId": "focus"},
        {"type": "update_clip", "clipId": "focus", "patch": {
            "crop": {"x": 0.5, "y": 0, "width": 0.5, "height": 1}
        }},
        {"type": "split_clip", "clipId": "focus", "atFrame": 20, "newClipId": "after"},
        {"type": "update_clip", "clipId": "after", "patch": {"crop": null}}
    ]}))
    .unwrap();
    let focused = apply_commands(&project, &[batch]).unwrap();
    let rendered_project = serialize_project(&focused).unwrap();
    render_v2::<fn(f64)>(
        &rendered_project,
        &RenderV2Options {
            output: output.to_string_lossy().into_owned(),
            force_software: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(output.metadata().unwrap().len() > 0);

    for (time, expected_blue) in [("0.5", false), ("1.5", true), ("2.5", false)] {
        let frame = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", time, "-i"])
            .arg(&output)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        assert!(
            frame.status.success(),
            "{}",
            String::from_utf8_lossy(&frame.stderr)
        );
        assert_eq!(frame.stdout.len(), 160 * 90 * 3);
        let sample = (45 * 160 + 40) * 3;
        let rgb = &frame.stdout[sample..sample + 3];
        assert_eq!(rgb[2] > rgb[0], expected_blue, "time={time}, rgb={rgb:?}");
    }
}
