//! Slow motion that makes frames: a shot filmed at twice the rate is the true picture between.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn encode(source: &str, out: &Path) {
    let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(program)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(source)
        .args(["-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv444p"])
        .arg(out)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A textured picture panning `step` pixels a frame at `rate` frames a second, for a second.
fn pan(rate: u32, step: u32) -> String {
    format!(
        "mandelbrot=s=480x180:r={rate}:start_scale=0.6:end_scale=0.6:end_pts=1,crop=320:180:x='n*{step}':y=0,trim=duration=1"
    )
}

fn shown(path: &Path, rate: u32, line: &str, at: i64) -> Frame {
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate={rate} dur={rate}f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m {line}\n",
        path.display()
    );
    let project = parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Auto,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

fn difference(a: &Frame, b: &Frame) -> f64 {
    let sum: u64 = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    sum as f64 / a.rgba.len() as f64
}

#[test]
fn flow_comes_closest_to_the_frame_that_was_never_filmed() {
    let dir = tempfile::tempdir().expect("a folder");
    let (slow, fast) = (dir.path().join("slow.mp4"), dir.path().join("fast.mp4"));
    encode(&pan(24, 6), &slow);
    encode(&pan(48, 3), &fast);
    // Half speed: sequence frame 13 is half way from file frame 6 to 7, which at twice the rate
    // is a frame the slow file never had.
    let truth = shown(&fast, 48, "src=13f at=0f dur=1f", 0);
    let line = |retime: &str| format!("src=0f at=0f dur=24f speed=1/2{retime}");
    let frames = difference(&shown(&slow, 24, &line(""), 13), &truth);
    let blend = difference(&shown(&slow, 24, &line(" retime=blend"), 13), &truth);
    let flow = difference(&shown(&slow, 24, &line(" retime=flow"), 13), &truth);
    eprintln!("off the true frame: frames {frames:.2}, blend {blend:.2}, flow {flow:.2}");
    assert!(flow < blend && flow < frames, "{frames} {blend} {flow}");
    assert!(flow < frames * 0.6, "{frames} {flow}");
}

#[test]
fn a_frame_that_falls_on_the_file_is_the_file() {
    let dir = tempfile::tempdir().expect("a folder");
    let slow = dir.path().join("slow.mp4");
    encode(&pan(24, 6), &slow);
    let line = |retime: &str| format!("src=0f at=0f dur=24f speed=1/2{retime}");
    let plain = shown(&slow, 24, &line(""), 12);
    let flow = shown(&slow, 24, &line(" retime=flow"), 12);
    assert!(difference(&plain, &flow) < 0.01);
    // Real speed never lies between frames, whatever the clip asks for.
    let line = |retime: &str| format!("src=0f at=0f dur=24f{retime}");
    let plain = shown(&slow, 24, &line(""), 9);
    let flow = shown(&slow, 24, &line(" retime=flow"), 9);
    assert!(difference(&plain, &flow) < 0.01);
}

#[test]
fn a_reversed_clip_makes_frames_too() {
    let dir = tempfile::tempdir().expect("a folder");
    let slow = dir.path().join("slow.mp4");
    encode(&pan(24, 6), &slow);
    // Half speed backward, between the same two frames as forward, in the other order: both give
    // pictures, and they differ from the nearest whole frame.
    let line = |retime: &str| format!("src=0f at=0f dur=24f speed=-1/2{retime}");
    let plain = shown(&slow, 24, &line(""), 13);
    let flow = shown(&slow, 24, &line(" retime=flow"), 13);
    assert!(
        difference(&plain, &flow) > 0.5,
        "{}",
        difference(&plain, &flow)
    );
}

/// A shot at `speed`, against the one frame of the file it should be showing.
fn shows_the_frame(path: &Path, (speed, dur): (&str, i64), at: i64, file_frame: i64) {
    let ramped = shown(
        path,
        24,
        &format!("src=0f at=0f dur={dur}f speed={speed}"),
        at,
    );
    let plain = shown(path, 24, &format!("src={file_frame}f at=0f dur=1f"), 0);
    let off = difference(&ramped, &plain);
    assert!(
        off < 0.01,
        "{speed} at {at}: {off} from file frame {file_frame}"
    );
}

#[test]
fn a_ramp_shows_the_frame_its_speed_has_reached() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("pan.mp4");
    encode(&pan(24, 6), &path);
    // One speed to frame 8, then half: 8 more frames play 4.
    let speed = ("{0f:1:hold 8f:1/2}", 24);
    shows_the_frame(&path, speed, 5, 5);
    shows_the_frame(&path, speed, 8, 8);
    shows_the_frame(&path, speed, 16, 12);
    // Speeding up in a line from 1 to 2 over 8 frames plays 12; then 2 holds.
    let speed = ("{0f:1 8f:2}", 12);
    shows_the_frame(&path, speed, 8, 12);
    shows_the_frame(&path, speed, 10, 16);
    // Backward, from the frame before the source out: 12 frames at 1 and 12 at 1/2 play 18.
    let speed = ("{0f:-1:hold 12f:-1/2}", 24);
    shows_the_frame(&path, speed, 0, 17);
    shows_the_frame(&path, speed, 11, 6);
    shows_the_frame(&path, speed, 22, 0);
}

#[test]
fn a_ramp_is_the_same_frame_on_either_side_of_a_split() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("pan.mp4");
    encode(&pan(24, 6), &path);
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f speed={{0f:1 12f:1/2}}\n",
        path.display()
    );
    let project = parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let split = kerf_lang::apply_script(&project, Catalog::builtin(), "split c1 at=7f as=c2")
        .expect("splits")
        .project;
    let frame = |project: &kerf_model::Project, at: i64| {
        Compositor::new(project, Catalog::builtin(), &ComposeOptions::default())
            .expect("composes")
            .frame(Rate::FPS_24.time_of(at), 1.0)
    };
    for at in [0, 6, 7, 11, 18, 23] {
        let off = difference(&frame(&project, at), &frame(&split, at));
        assert!(off < 0.01, "frame {at}: {off}");
    }
}

#[test]
fn a_ramp_makes_frames_between_frames() {
    let dir = tempfile::tempdir().expect("a folder");
    let (slow, fast) = (dir.path().join("slow.mp4"), dir.path().join("fast.mp4"));
    encode(&pan(24, 6), &slow);
    encode(&pan(48, 3), &fast);
    // Half speed to frame 12, on to third speed: frame 5 is half way from file frame 2 to 3.
    let line = |retime: &str| format!("src=0f at=0f dur=24f speed={{0f:1/2:hold 12f:1/3}}{retime}");
    let truth = shown(&fast, 48, "src=5f at=0f dur=1f", 0);
    let frames = difference(&shown(&slow, 24, &line(""), 5), &truth);
    let flow = difference(&shown(&slow, 24, &line(" retime=flow"), 5), &truth);
    assert!(flow < frames * 0.6, "frames {frames}, flow {flow}");
}
