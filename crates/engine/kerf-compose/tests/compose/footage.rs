//! Media the compositor reads: a frame drawn again, at another size or by
//! another layer, is the same picture.

use kerf_compose::{ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned())
}

/// Color bars, 64x36 at 24 fps for a second.
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
            ..Default::default()
        },
    )
    .expect("composes")
}

/// The mean of each channel.
fn mean(frame: &Frame) -> [f64; 3] {
    let count = (frame.width * frame.height) as f64;
    let mut sum = [0.0; 3];
    for pixel in frame.rgba.as_chunks::<4>().0 {
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
    assert!(
        compositor.problems().is_empty(),
        "{:?}",
        compositor.problems()
    );
    let (full, half) = (mean(&full), mean(&half));
    for (a, b) in full.iter().zip(&half) {
        assert!(
            (a - b).abs() < 6.0,
            "{full:?} at full size, {half:?} at half"
        );
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
    assert!(
        compositor.problems().is_empty(),
        "{:?}",
        compositor.problems()
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn a_sound_clip_draws_the_track_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let sound = |name: &str, source: &str| {
        let path = dir.path().join(name);
        let output = Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", source, "-t", "1", "-ac", "2", "-ar", "48000"])
            .arg(&path)
            .output()
            .expect("ffmpeg runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path.display().to_string()
    };
    let tone = sound("tone.wav", "sine=frequency=220:sample_rate=48000");
    let quiet = sound("quiet.wav", "anullsrc=r=48000:cl=stereo");
    // What a `gen=sound` clip a frame wide and most of one high draws of the track on black, as
    // the rows and the columns it lights.
    let lit = |path: &str, set: &str| {
        let text = format!(
            "kerf 1\nproject name=\"t\"\n\
             media a path=\"{path}\" rate=24 dur=24f audio=2ch@48000\n\
             sequence s1 rate=24 size=320x180 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip v1 gen=sound@1 at=0f dur=24f\n\
             \x20     set track=A1 width=1.7 height=0.9 {set}\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=a src=0f at=0f dur=24f\n"
        );
        let project = parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let options = ComposeOptions {
            sequence: None,
            captions: false,
            ..Default::default()
        };
        let frame = Compositor::new(&project, Catalog::builtin(), &options)
            .unwrap()
            .frame(Rate::FPS_24.time_of(12), 1.0);
        let on = |x: u32, y: u32| frame.rgba[((y * frame.width + x) * 4) as usize] > 128;
        let rows = (0..frame.height)
            .filter(|y| (0..frame.width).any(|x| on(x, *y)))
            .count();
        let columns: Vec<u32> = (0..frame.width)
            .filter(|x| (0..frame.height).any(|y| on(*x, y)))
            .collect();
        (rows, columns)
    };
    // A wave: silence is a flat line, and the tone swings well above and below it.
    let wave = "kind=waveform window=0.02 bars=200";
    let ((flat, _), (swinging, _)) = (lit(&quiet, wave), lit(&tone, wave));
    assert!(flat > 0 && flat < 12, "{flat}");
    assert!(swinging > flat * 3, "{swinging} vs {flat}");
    // A spectrum: silence draws no bar, and the tone's lands about a quarter of the way along,
    // where 220 hertz is between 40 and 16000 by octave.
    let spectrum = "kind=spectrum bars=32";
    assert_eq!(lit(&quiet, spectrum).0, 0);
    let (tall, columns) = lit(&tone, spectrum);
    assert!(tall > 80, "{tall}");
    let (first, last) = (columns[0], columns[columns.len() - 1]);
    let along = |x: u32| (f64::from(x) - 160.0 + 153.0) / 306.0;
    assert!(along(first) > 0.2 && along(last) < 0.4, "{columns:?}");
}
