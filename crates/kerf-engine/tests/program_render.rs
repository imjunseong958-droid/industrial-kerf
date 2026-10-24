//! Renders `kerf-model` projects through FFmpeg and inspects the output.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project, Rate};
use std::path::Path;
use std::process::Command;

fn run(command: &mut Command) -> Vec<u8> {
    let output = command.output().expect("runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Four seconds of a solid color with a tone.
fn source(dir: &Path, name: &str, color: &str, tone: u32) -> String {
    let path = dir.join(name);
    run(Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("color=c={color}:s=160x90:r=24:d=4"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=f={tone}:sample_rate=48000:d=4"))
        .args([
            "-ac",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(&path));
    path.to_string_lossy().into_owned()
}

fn project(dir: &Path, body: &str) -> Project {
    let red = source(dir, "red.mp4", "red", 440);
    let blue = source(dir, "blue.mp4", "blue", 660);
    let text = format!(
        "kerf 1\nproject name=\"render\"\n\
         media red path=\"{red}\" rate=24 dur=96f video=160x90 audio=2ch@48000\n\
         media blue path=\"{blue}\" rate=24 dur=96f video=160x90 audio=2ch@48000\n\
         {body}"
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn render(project: &Project, sequence: Option<&str>, range: Option<(i64, i64)>, output: &Path) {
    let range = range.map(|(from, to)| (Rate::FPS_24.time_of(from), Rate::FPS_24.time_of(to)));
    render_program::<fn(f64)>(
        project,
        sequence,
        range,
        &RenderOptions {
            output: output.to_string_lossy().into_owned(),
            force_software: true,
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
}

/// Video frames and audio duration of a file.
fn probe(path: &Path) -> (u64, f64) {
    let frames = run(Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(path));
    let audio = run(Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0"])
        .args(["-show_entries", "stream=duration", "-of", "csv=p=0"])
        .arg(path));
    (
        String::from_utf8_lossy(&frames)
            .trim()
            .parse()
            .expect("frame count"),
        String::from_utf8_lossy(&audio)
            .trim()
            .parse()
            .expect("audio duration"),
    )
}

/// How loud the sound of `[from, to)` seconds is: its RMS.
fn loudness(path: &Path, from: f64, to: f64) -> f64 {
    let raw = run(Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-ss", &from.to_string(), "-t", &(to - from).to_string()])
        .args(["-map", "0:a", "-c:a", "pcm_f32le", "-f", "f32le", "-"]));
    let samples: Vec<f64> = raw
        .chunks_exact(4)
        .map(|bytes| f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))
        .collect();
    (samples.iter().map(|sample| sample * sample).sum::<f64>() / samples.len().max(1) as f64).sqrt()
}

/// The RGB color at the center of frame `index`.
fn pixel(path: &Path, index: u32) -> [u8; 3] {
    let raw = run(Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .arg("-vf")
        .arg(format!("select=eq(n\\,{index}),scale=160:90"))
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]));
    let at = (45 * 160 + 80) * 3;
    [raw[at], raw[at + 1], raw[at + 2]]
}

fn is(color: [u8; 3], expected: [u8; 3]) -> bool {
    color
        .iter()
        .zip(expected)
        .all(|(value, want)| value.abs_diff(want) < 40)
}

#[test]
fn cuts_transitions_and_ranges_render_frame_exact() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(
        dir.path(),
        "sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=red src=12f at=0f dur=48f\n\
         \x20   clip c2 media=blue src=12f at=48f dur=48f\n\
         \x20   transition t1 from=c1 to=c2 fx=dissolve dur=12f\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=red src=12f at=0f dur=48f\n\
         \x20   clip a2 media=blue src=12f at=48f dur=48f\n\
         \x20   transition t2 from=a1 to=a2 fx=crossfade dur=12f\n",
    );
    let output = dir.path().join("full.mp4");
    render(&project, None, None, &output);
    let (frames, audio) = probe(&output);
    assert_eq!(frames, 96);
    assert!((audio - 4.0).abs() < 0.05, "audio lasts {audio}");
    assert!(
        is(pixel(&output, 12), [255, 0, 0]),
        "{:?}",
        pixel(&output, 12)
    );
    assert!(
        is(pixel(&output, 80), [0, 0, 255]),
        "{:?}",
        pixel(&output, 80)
    );
    let middle = pixel(&output, 48);
    assert!(
        middle[0] > 60 && middle[2] > 60,
        "the dissolve mixes both clips at the cut: {middle:?}"
    );
    // One tone plays before the crossfade and the other after it, not both
    // at once from the start.
    let (before, after) = (loudness(&output, 0.5, 1.5), loudness(&output, 2.5, 3.5));
    assert!(before > 0.05 && before < 0.1, "before the cut: {before}");
    assert!(after > 0.05 && after < 0.1, "after the cut: {after}");

    let ranged = dir.path().join("range.mp4");
    render(&project, None, Some((42, 66)), &ranged);
    let (frames, audio) = probe(&ranged);
    assert_eq!(frames, 24);
    assert!((audio - 1.0).abs() < 0.05, "audio lasts {audio}");
    let middle = pixel(&ranged, 6);
    assert!(middle[0] > 60 && middle[2] > 60, "{middle:?}");
}

/// Titles and captions need FFmpeg built with libfreetype.
fn has_drawtext() -> bool {
    Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-filters"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(" drawtext "))
}

#[test]
fn generators_nested_sequences_and_reverse_render() {
    let dir = tempfile::tempdir().unwrap();
    let text = if has_drawtext() {
        "\x20 track V2 video\n\
         \x20   clip t1 gen=title at=0f dur=24f\n\
         \x20     set text=\"Cut\" size=0.5\n\
         \x20 track S1 subtitle\n\
         \x20   clip cap caption=\"Hello\" at=48f dur=24f\n"
    } else {
        ""
    };
    let project = project(
        dir.path(),
        &format!(
            "sequence inner rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip i1 media=blue src=0f at=0f dur=48f\n\
         sequence main rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip bg gen=solid at=0f dur=24f\n\
         \x20     set color=#00ff00\n\
         \x20   clip n1 seq=inner src=0f at=24f dur=24f\n\
         \x20   clip r1 media=red src=0f at=48f dur=24f speed=-1\n\
         {text}"
        ),
    );
    let output = dir.path().join("generated.mp4");
    render(&project, Some("main"), None, &output);
    let (frames, _) = probe(&output);
    assert_eq!(frames, 72);
    // A title's background is transparent, so the solid shows at its corner.
    let corner = {
        let raw = run(Command::new(resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&output)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]));
        [raw[0], raw[1], raw[2]]
    };
    assert!(is(corner, [0, 255, 0]), "{corner:?}");
    assert!(
        is(pixel(&output, 30), [0, 0, 255]),
        "{:?}",
        pixel(&output, 30)
    );
    assert!(
        is(pixel(&output, 60), [255, 0, 0]),
        "{:?}",
        pixel(&output, 60)
    );
}

#[test]
fn a_cropped_cut_shows_its_region_between_full_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("halves.mp4");
    run(Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=red:s=160x90:r=24:d=3,drawbox=x=80:y=0:w=80:h=90:color=blue:t=fill")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-y"])
        .arg(&path));
    let text = format!(
        "kerf 1\nproject name=\"crop\"\n\
         media m path=\"{}\" rate=24 dur=72f video=160x90\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m src=0f at=0f dur=24f\n\
         \x20   clip c2 media=m src=24f at=24f dur=24f\n\
         \x20     set crop_l=0.5\n\
         \x20   clip c3 media=m src=48f at=48f dur=24f\n",
        path.display()
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let output = dir.path().join("cropped.mp4");
    render(&project, None, None, &output);
    // Left of center: red in the full frame, blue in the centered right half.
    let left_of_center = |index: u32| {
        let raw = run(Command::new(resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&output)
            .arg("-vf")
            .arg(format!("select=eq(n\\,{index})"))
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]));
        let at = (45 * 160 + 60) * 3;
        [raw[at], raw[at + 1], raw[at + 2]]
    };
    for (frame, expected) in [(12, [255, 0, 0]), (36, [0, 0, 255]), (60, [255, 0, 0])] {
        let color = left_of_center(frame);
        assert!(is(color, expected), "frame {frame}: {color:?}");
    }
}

#[test]
fn raster_effects_follow_their_keyframes_and_the_graph_bakes_them_once() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(
        dir.path(),
        "sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=red src=0f at=0f dur=48f\n\
         \x20     fx channel.invert blend={0f:0 47f:100}\n",
    );
    let output = dir.path().join("inverted.mp4");
    let composited = RenderOptions {
        output: output.to_string_lossy().into_owned(),
        force_software: true,
        project_dir: Some(dir.path().to_path_buf()),
        ..RenderOptions::default()
    };
    let options = RenderOptions {
        graph: true,
        ..composited.clone()
    };
    for options in [&composited, &options] {
        render_program::<fn(f64)>(&project, None, None, options, None).expect("renders");
        // Red inverted is cyan; blending back to the original ends on red.
        assert!(
            is(pixel(&output, 0), [0, 255, 255]),
            "{:?}",
            pixel(&output, 0)
        );
        assert!(
            is(pixel(&output, 47), [255, 0, 0]),
            "{:?}",
            pixel(&output, 47)
        );
    }

    // The graph bakes the effect once, and reads the bake again.
    let bakes = || {
        std::fs::read_dir(
            dir.path()
                .join(kerf_engine::derivatives::DERIVED_CACHE_RELATIVE_PATH),
        )
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("raster-"))
        .map(|entry| entry.path().join("frames.mkv"))
        .collect::<Vec<_>>()
    };
    let baked = bakes();
    assert_eq!(baked.len(), 1);
    let made = std::fs::metadata(&baked[0]).unwrap().modified().unwrap();
    render_program::<fn(f64)>(&project, None, None, &options, None).expect("renders again");
    assert_eq!(bakes(), baked);
    assert_eq!(
        std::fs::metadata(&baked[0]).unwrap().modified().unwrap(),
        made
    );
}

#[test]
fn transparency_from_raster_effects_shows_the_track_below() {
    let dir = tempfile::tempdir().unwrap();
    let black = source(dir.path(), "black.mp4", "black", 330);
    let project = project(
        dir.path(),
        &format!(
            "media black path=\"{black}\" rate=24 dur=96f video=160x90 audio=2ch@48000\n\
             sequence s1 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=red src=0f at=0f dur=24f\n\
             \x20 track V2 video\n\
             \x20   clip c2 media=black src=0f at=0f dur=24f\n\
             \x20     fx key.unmult\n"
        ),
    );
    let output = dir.path().join("keyed.mp4");
    let options = RenderOptions {
        output: output.to_string_lossy().into_owned(),
        force_software: true,
        project_dir: Some(dir.path().to_path_buf()),
        ..RenderOptions::default()
    };
    render_program::<fn(f64)>(&project, None, None, &options, None).expect("renders");
    // Unmult keys black out, so the red clip beneath shows through.
    assert!(
        is(pixel(&output, 12), [255, 0, 0]),
        "{:?}",
        pixel(&output, 12)
    );
}

/// The RGB color at pixel (`x`, 45) of frame `index`.
fn pixel_at(path: &Path, index: u32, x: usize) -> [u8; 3] {
    let raw = run(Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .arg("-vf")
        .arg(format!("select=eq(n\\,{index}),scale=160:90"))
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]));
    let at = (45 * 160 + x) * 3;
    [raw[at], raw[at + 1], raw[at + 2]]
}

#[test]
fn raster_transitions_wipe_from_one_clip_to_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(
        dir.path(),
        "sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=red src=0f at=0f dur=24f\n\
         \x20   clip c2 media=blue src=24f at=24f dur=24f\n\
         \x20   transition t1 from=c1 to=c2 fx=transition.linearwipe dur=12f\n",
    );
    let output = dir.path().join("wipe.mp4");
    let options = RenderOptions {
        output: output.to_string_lossy().into_owned(),
        force_software: true,
        project_dir: Some(dir.path().to_path_buf()),
        ..RenderOptions::default()
    };
    render_program::<fn(f64)>(&project, None, None, &options, None).expect("renders");
    let red = [255, 0, 0];
    let blue = [0, 0, 255];
    assert!(
        is(pixel_at(&output, 16, 80), red),
        "{:?}",
        pixel_at(&output, 16, 80)
    );
    assert!(
        is(pixel_at(&output, 31, 80), blue),
        "{:?}",
        pixel_at(&output, 31, 80)
    );
    // Halfway, the wipe has crossed half the picture.
    let (left, right) = (pixel_at(&output, 24, 10), pixel_at(&output, 24, 150));
    assert!(
        (is(left, red) && is(right, blue)) || (is(left, blue) && is(right, red)),
        "{left:?} {right:?}"
    );
}

#[test]
fn titles_solids_and_nested_sequences_take_raster_effects() {
    let dir = tempfile::tempdir().unwrap();
    let title = if has_drawtext() {
        "\x20 track V2 video\n\
         \x20   clip t1 gen=title at=24f dur=24f\n\
         \x20     set text=\"Hi\" size=0.5\n\
         \x20     fx channel.invert\n"
    } else {
        ""
    };
    let project = project(
        dir.path(),
        &format!(
            "sequence s1 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip n1 seq=s2 src=0f at=0f dur=24f\n\
             \x20     fx channel.invert\n\
             \x20   clip c1 media=red src=0f at=24f dur=24f\n\
             \x20   clip g1 gen=solid at=48f dur=24f\n\
             \x20     set color=#ff0000\n\
             \x20     fx channel.invert\n\
             {title}\
             sequence s2 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c2 media=red src=0f at=0f dur=24f\n"
        ),
    );
    let output = dir.path().join("generated.mp4");
    let options = RenderOptions {
        output: output.to_string_lossy().into_owned(),
        force_software: true,
        project_dir: Some(dir.path().to_path_buf()),
        ..RenderOptions::default()
    };
    render_program::<fn(f64)>(&project, None, None, &options, None).expect("renders");
    let cyan = [0, 255, 255];
    // The nested sequence's red, inverted.
    assert!(
        is(pixel_at(&output, 12, 80), cyan),
        "{:?}",
        pixel_at(&output, 12, 80)
    );
    // The inverted title keeps its transparency: red shows around it.
    assert!(
        is(pixel_at(&output, 36, 3), [255, 0, 0]),
        "{:?}",
        pixel_at(&output, 36, 3)
    );
    // The red solid, inverted.
    assert!(
        is(pixel_at(&output, 60, 80), cyan),
        "{:?}",
        pixel_at(&output, 60, 80)
    );
}
