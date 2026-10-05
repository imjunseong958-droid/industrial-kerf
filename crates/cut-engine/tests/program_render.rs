//! Renders `cut-model` projects through FFmpeg and inspects the output.

use cut_engine::ffmpeg::resolve_ffmpeg_bin;
use cut_engine::render::{render_program, RenderOptions};
use cut_lang::parse_document;
use cut_model::{Catalog, Project, Rate};
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
        "cut 1\nproject name=\"render\"\n\
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
        "cut 1\nproject name=\"crop\"\n\
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
