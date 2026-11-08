//! A speed ramp through a whole render: the picture on its frames and the sound in time with them.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project, Time};
use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> Vec<u8> {
    let output = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Four seconds of a picture that brightens frame by frame, and a tenth of a second of tone every
/// half second.
fn source(path: &Path) {
    run(&[
        "-f",
        "lavfi",
        "-i",
        "nullsrc=s=64x36:r=24:d=4,format=gray,geq=lum='N*2.5'",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=if(lt(mod(t\\,0.5)\\,0.1)\\,0.8*sin(2*PI*997*t)\\,0):s=48000:d=4",
        "-ac",
        "2",
        "-c:v",
        "libx264",
        "-qp",
        "0",
        "-pix_fmt",
        "yuv444p",
        "-c:a",
        "pcm_s16le",
        path.to_str().expect("UTF-8 path"),
    ]);
}

fn project(path: &Path, tracks: &str) -> Project {
    parse_document(
        &format!(
            "kerf 1\nproject name=\"Ramp\"\nmedia m path=\"{}\" rate=24 dur=96f video=64x36 audio=2ch@48000\nsequence s1 rate=24 size=64x36 audio=48000\n{tracks}",
            path.display()
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

fn render(project: &Project, out: &Path) {
    render_program::<fn(f64)>(
        project,
        None,
        None,
        &RenderOptions {
            output: out.to_string_lossy().into_owned(),
            ..RenderOptions::default()
        },
        None,
    )
    .expect("renders");
}

/// The file's sound, stereo floats.
fn sound(path: &Path) -> Vec<f32> {
    run(&[
        "-i",
        path.to_str().expect("UTF-8 path"),
        "-vn",
        "-ac",
        "2",
        "-ar",
        "48000",
        "-f",
        "f32le",
        "-",
    ])
    .as_chunks::<4>().0.iter()
    .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    .collect()
}

/// The middle pixel of every frame of the file, as a brightness.
fn brightness(path: &Path) -> Vec<u8> {
    run(&[
        "-i",
        path.to_str().expect("UTF-8 path"),
        "-an",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "gray",
        "-",
    ])
    .as_chunks::<{ 64 * 36 }>().0.iter()
    .map(|frame| frame[18 * 64 + 32])
    .collect()
}

/// Where each burst of tone starts, in seconds.
fn onsets(samples: &[f32]) -> Vec<f64> {
    let block = 96;
    let loud: Vec<bool> = samples
        .chunks(block * 2)
        .map(|chunk| chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32 > 0.01)
        .collect();
    let quiet = 9_600 / block;
    loud.iter()
        .enumerate()
        .filter(|(at, on)| **on && !loud[at.saturating_sub(quiet)..*at].contains(&true))
        .map(|(at, _)| (at * block) as f64 / 48_000.0)
        .collect()
}

#[test]
fn a_rendered_ramp_keeps_its_sound_with_its_picture() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("source.mov");
    source(&path);
    // Full speed for a second, down to a quarter in a line by the third, then a quarter.
    let speed = "{0f:1 24f:1 72f:1/4}";
    let project = self::project(
        &path,
        &format!(
            "  track V1 video\n    clip v media=m src=0f at=0f dur=120f speed={speed} link=L\n  track A1 audio\n    clip a media=m src=0f at=0f dur=120f speed={speed} link=L\n"
        ),
    );
    let out = dir.path().join("out.mp4");
    render(&project, &out);
    let clip = project.clip("v").expect("the clip");
    // The sound: each burst when the ramp reaches it.
    let samples = sound(&out);
    let heard = onsets(&samples);
    assert!(heard.len() >= 5, "{heard:?}");
    for (index, onset) in heard.iter().enumerate() {
        let wanted = clip
            .played_at(Time::from_seconds(index as f64 * 0.5).expect("a time"))
            .expect("a source")
            .seconds();
        assert!(
            (onset - wanted).abs() < 0.08,
            "burst {index} heard at {onset}, the ramp says {wanted}"
        );
    }
    // The picture: each frame as bright as a plain render shows the source frame it should show.
    let plain = self::project(
        &path,
        "  track V1 video\n    clip v media=m src=0f at=0f dur=96f\n",
    );
    let plain_out = dir.path().join("plain.mp4");
    render(&plain, &plain_out);
    let truth = brightness(&plain_out);
    let frames = brightness(&out);
    assert_eq!(frames.len(), 120);
    let rate = kerf_model::Rate::FPS_24;
    for (frame, shown) in frames.iter().enumerate() {
        let source = clip
            .source_at(rate.time_of(frame as i64))
            .expect("a source");
        let wanted = truth[rate.frames_floor(source) as usize];
        assert!(
            shown.abs_diff(wanted) <= 2,
            "frame {frame} reads {shown}, its source frame reads {wanted}"
        );
    }
}

#[test]
fn a_nested_sequence_that_ramps_keeps_its_sound_in_time_too() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("source.mov");
    source(&path);
    let project = project(
        &path,
        "  track V1 video\n    clip v seq=inner src=0f at=0f dur=120f speed={0f:1 24f:1 72f:1/4}\n  track A1 audio\n    clip a seq=inner src=0f at=0f dur=120f speed={0f:1 24f:1 72f:1/4}\nsequence inner rate=24 size=64x36 audio=48000\n  track V1 video\n    clip iv media=m src=0f at=0f dur=96f\n  track A1 audio\n    clip ia media=m src=0f at=0f dur=96f\n",
    );
    let out = dir.path().join("out.mp4");
    render(&project, &out);
    let clip = project.clip("a").expect("the clip");
    let heard = onsets(&sound(&out));
    assert!(heard.len() >= 5, "{heard:?}");
    for (index, onset) in heard.iter().enumerate() {
        let wanted = clip
            .played_at(Time::from_seconds(index as f64 * 0.5).expect("a time"))
            .expect("a source")
            .seconds();
        assert!(
            (onset - wanted).abs() < 0.1,
            "burst {index} heard at {onset}, the ramp says {wanted}"
        );
    }
}

#[test]
fn a_ramp_through_a_crossfade_still_fades_across_the_cut() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("source.mov");
    source(&path);
    let project = project(
        &path,
        "  track A1 audio\n    clip a media=m src=0f at=0f dur=48f speed={0f:1 24f:1/2}\n    clip b media=m src=40f at=48f dur=48f speed={0f:1 24f:1/2}\n    transition x from=a to=b fx=crossfade@1 dur=12f\n",
    );
    let out = dir.path().join("out.wav");
    let silent = kerf_compose::ramps::with_sound(&project, |_, _, _| None);
    // Both clips became plain media, whose handles reach the fade.
    assert!(silent
        .sequences
        .iter()
        .flat_map(|sequence| &sequence.tracks)
        .flat_map(|track| &track.clips)
        .all(|clip| clip.speed().is_one()));
    let compiled = kerf_engine::program::compile_program(
        &silent,
        Catalog::builtin(),
        &kerf_engine::program::ProgramOptions {
            audio_only: true,
            ..Default::default()
        },
    )
    .expect("compiles");
    let mut args: Vec<&str> = compiled.argv[1..].iter().map(String::as_str).collect();
    args.extend(["-c:a", "pcm_f32le", out.to_str().expect("UTF-8 path")]);
    run(&args);
    let samples = sound(&out);
    assert_eq!(samples.len(), 96 * 2_000 * 2);
    assert!(samples.iter().any(|sample| sample.abs() > 0.1));
}
