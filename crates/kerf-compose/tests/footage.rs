//! Media the compositor reads: a frame drawn again, at another size or by
//! another layer, is the same picture.

use kerf_compose::{ComposeOptions, Compositor, Device, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned())
}

/// Color bars, 64x36 at 24 fps for a second.
fn bars(dir: &Path) -> String {
    let path = dir.join("bars.mp4");
    let output = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "smptebars=s=64x36:r=24:d=1"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path.display().to_string()
}

fn compositor(dir: &Path, tracks: &str) -> Compositor {
    let text = format!(
        "kerf 1\nproject name=\"t\"\n\
         media m path=\"{}\" rate=24 dur=24f video=64x36\n\
         sequence s1 rate=24 size=64x36 audio=48000\n{tracks}",
        bars(dir)
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: None,
            captions: false,
            device: Device::Cpu,
        },
    )
    .expect("composes")
}

/// The mean of each channel.
fn mean(frame: &Frame) -> [f64; 3] {
    let count = (frame.width * frame.height) as f64;
    let mut sum = [0.0; 3];
    for pixel in frame.rgba.chunks_exact(4) {
        for (total, value) in sum.iter_mut().zip(pixel) {
            *total += f64::from(*value);
        }
    }
    sum.map(|total| total / count)
}

#[test]
fn a_frame_drawn_again_at_another_size_still_shows_its_media() {
    let dir = tempfile::tempdir().unwrap();
    let compositor = compositor(
        dir.path(),
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n",
    );
    let at = Rate::FPS_24.time_of(5);
    let full = compositor.frame(at, 1.0);
    let half = compositor.frame(at, 0.5);
    assert!(compositor.problems().is_empty(), "{:?}", compositor.problems());
    let (full, half) = (mean(&full), mean(&half));
    for (a, b) in full.iter().zip(&half) {
        assert!((a - b).abs() < 6.0, "{full:?} at full size, {half:?} at half");
    }
}

#[test]
fn two_layers_of_one_file_both_show_it() {
    let dir = tempfile::tempdir().unwrap();
    let compositor = compositor(
        dir.path(),
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n\
         \x20 track V2 video\n    clip c2 media=m src=0f at=0f dur=24f\n\
         \x20     set scale=0.5 x=0.25\n",
    );
    for frame in 0..3 {
        compositor.frame(Rate::FPS_24.time_of(frame), 0.5);
    }
    assert!(compositor.problems().is_empty(), "{:?}", compositor.problems());
}
