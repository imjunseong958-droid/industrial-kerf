//! Removing something from footage that moves: the fill should be the scene behind it.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg(source: &str, out: &Path) {
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

/// A scene that pans, six pixels a frame.
const SCENE: &str = "mandelbrot=s=480x180:r=24:start_scale=0.6:end_scale=0.6:end_pts=1,crop=320:180:x='n*6':y=0,trim=duration=1";

fn frame(clean: &str, body: &str, at: i64) -> Frame {
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{clean}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n{body}"
    );
    let project = parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

/// The mean difference between two frames inside a rectangle of pixels.
fn difference(a: &Frame, b: &Frame, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> f64 {
    let (mut sum, mut count) = (0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let at = ((y * a.width + x) * 4) as usize;
            for k in 0..3 {
                sum += u64::from(a.rgba[at + k].abs_diff(b.rgba[at + k]));
                count += 1;
            }
        }
    }
    sum as f64 / count as f64
}

#[test]
fn video_fill_recovers_the_scene_behind_a_logo_on_a_moving_shot() {
    let dir = tempfile::tempdir().expect("a folder");
    let clean = dir.path().join("clean.mp4");
    let boxed = dir.path().join("boxed.mp4");
    ffmpeg(SCENE, &clean);
    // A logo that stays put while the shot pans behind it.
    ffmpeg(
        &format!("{SCENE},drawbox=x=140:y=70:w=30:h=30:color=black@1:t=fill"),
        &boxed,
    );
    let (clean, boxed) = (clean.display().to_string(), boxed.display().to_string());
    let at = 12;
    let keyed = |method: &str| {
        format!(
            "      mask k1 rect={:.4},{:.4},{:.4},{:.4} mode=none\n      fx image.fill@1 path=k1 method={method} reach=6\n",
            138.0 / 320.0,
            68.0 / 180.0,
            172.0 / 320.0,
            102.0 / 180.0
        )
    };
    let reference = frame(&clean, "", at);
    let area = (140, 70, 170, 100);
    let by_frame = frame(&boxed, &keyed("object"), at);
    let by_video = frame(&boxed, &keyed("video"), at);
    let (frame_off, video_off) = (
        difference(&by_frame, &reference, area),
        difference(&by_video, &reference, area),
    );
    assert!(
        video_off < frame_off * 0.6,
        "video {video_off:.2} off the clean plate, one frame alone {frame_off:.2}"
    );
    assert!(video_off < 12.0, "video {video_off:.2} off the clean plate");
}

#[test]
fn video_fill_removes_what_moves_when_its_outline_moves_with_it() {
    let dir = tempfile::tempdir().expect("a folder");
    let clean = dir.path().join("clean.mp4");
    let boxed = dir.path().join("boxed.mp4");
    ffmpeg(SCENE, &clean);
    // Something crosses the frame, 8 pixels a frame, while the shot pans behind it.
    ffmpeg(
        &format!("{SCENE},format=yuv444p,geq=lum='if(between(X,20+8*N,50+8*N)*between(Y,70,100),0,lum(X,Y))':cb='if(between(X,20+8*N,50+8*N)*between(Y,70,100),128,cb(X,Y))':cr='if(between(X,20+8*N,50+8*N)*between(Y,70,100),128,cr(X,Y))'"),
        &boxed,
    );
    let (clean, boxed) = (clean.display().to_string(), boxed.display().to_string());
    let (w, h) = (320.0, 180.0);
    let rect = |frame: f64| {
        format!(
            "{:.4},{:.4},{:.4},{:.4}",
            (18.0 + 8.0 * frame) / w,
            68.0 / h,
            (52.0 + 8.0 * frame) / w,
            102.0 / h
        )
    };
    let keyed = |method: &str| {
        format!(
            "      mask k1 rect={{0f:{} 23f:{}}} mode=none\n      fx image.fill@1 path=k1 method={method} reach=6\n",
            rect(0.0),
            rect(23.0)
        )
    };
    let still = format!(
        "      mask k1 rect={} mode=none\n      fx image.fill@1 path=k1 method=video reach=6\n",
        rect(12.0)
    );
    for at in [6, 12, 17] {
        let reference = frame(&clean, "", at);
        let x = 20 + 8 * at as u32;
        let area = (x, 70, x + 30, 100);
        let untouched = difference(&frame(&boxed, "", at), &reference, area);
        let by_frame = difference(&frame(&boxed, &keyed("object"), at), &reference, area);
        let by_video = difference(&frame(&boxed, &keyed("video"), at), &reference, area);
        eprintln!(
            "frame {at}: left {untouched:.2}, one frame alone {by_frame:.2}, video {by_video:.2}"
        );
        assert!(untouched > 30.0, "the thing was there: {untouched:.2}");
        assert!(
            by_video < by_frame * 0.6,
            "video {by_video:.2}, one frame {by_frame:.2}"
        );
        assert!(by_video < 12.0, "video {by_video:.2} off the clean plate");
        // And a still outline only catches it where it happens to be.
        if at != 12 {
            let missed = difference(&frame(&boxed, &still, at), &reference, area);
            assert!(missed > by_video * 2.0, "{missed:.2} against {by_video:.2}");
        }
    }
}

#[test]
fn a_clip_without_frames_to_read_still_fills() {
    // A generator's fill has no footage, and an adjustment layer no clip of its own: it falls back.
    let body = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#404040\n  track V2 video\n    clip g2 gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n      mask k9 rect=0.45,0.45,0.55,0.55\n  track V3 video\n    clip a1 gen=adjustment@1 at=0f dur=24f\n      mask k1 rect=0.3,0.3,0.7,0.7 mode=none\n      fx image.fill@1 path=k1 method=video\n";
    let project = parse_document(body, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let drawn = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(3), 1.0);
    let at = ((45 * 160 + 80) * 4) as usize;
    assert!(drawn.rgba[at] < 110, "{}", drawn.rgba[at]);
}
