//! Sound effects and buses, heard: what each does to the sound a render mixes.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::program::{compile_program, ProgramOptions};
use kerf_lang::parse_document;
use kerf_model::Catalog;
use std::path::Path;
use std::process::Command;

fn ffmpeg(args: &[&str]) {
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
}

/// A sound file from an expression.
fn source(dir: &Path, name: &str, expression: &str, seconds: f64) -> String {
    let path = dir.join(name);
    let path = path.to_str().expect("UTF-8 path").to_owned();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("aevalsrc={expression}:s=48000:d={seconds}"),
        "-ac",
        "2",
        "-c:a",
        "pcm_f32le",
        &path,
    ]);
    path
}

/// What a sequence with these sound tracks mixes, left channel only.
fn mixed(dir: &Path, media: &[(&str, &str)], body: &str) -> Vec<f32> {
    let lines: String = media
        .iter()
        .map(|(id, path)| format!("media {id} path=\"{path}\" rate=48 dur=192f audio=2ch@48000\n"))
        .collect();
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\n{lines}sequence s1 rate=48 size=64x36 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let compiled = compile_program(
        &project,
        Catalog::builtin(),
        &ProgramOptions {
            audio_only: true,
            ..ProgramOptions::default()
        },
    )
    .expect("compiles");
    let out = dir.join("mix.f32");
    let mut args: Vec<&str> = compiled.argv[1..].iter().map(String::as_str).collect();
    let out_path = out.to_str().expect("UTF-8 path");
    args.extend(["-c:a", "pcm_f32le", "-f", "f32le", out_path]);
    ffmpeg(&args);
    std::fs::read(&out)
        .expect("the mix")
        .as_chunks::<8>().0.iter()
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|s| f64::from(s * s)).sum::<f64>() / samples.len().max(1) as f64).sqrt()
}

fn seconds(samples: &[f32], from: f64, to: f64) -> &[f32] {
    let at = |t: f64| ((t * 48_000.0) as usize).min(samples.len());
    &samples[at(from)..at(to)]
}

const TONE: &str = "0.5*sin(2*PI*440*t)";
const CLIP: &str = "    clip c1 media=m src=0f at=0f dur=192f\n";

/// Each effect against the same sound with nothing on it.
fn with_effect(dir: &Path, path: &str, fx: &str) -> (Vec<f32>, Vec<f32>) {
    let plain = mixed(dir, &[("m", path)], &format!("  track A1 audio\n{CLIP}"));
    let effected = mixed(
        dir,
        &[("m", path)],
        &format!("  track A1 audio\n{CLIP}      fx {fx}\n"),
    );
    (plain, effected)
}

#[test]
fn a_compressor_brings_a_loud_sound_down_and_makeup_brings_it_back() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = source(dir.path(), "tone.wav", TONE, 4.0);
    let (plain, squeezed) = with_effect(
        dir.path(),
        &path,
        "audio.compressor threshold_db=-30 ratio=8 attack_ms=5",
    );
    let (before, after) = (
        rms(seconds(&plain, 1.0, 3.0)),
        rms(seconds(&squeezed, 1.0, 3.0)),
    );
    assert!(after < before * 0.5, "{before} to {after}");
    let (_, raised) = with_effect(
        dir.path(),
        &path,
        "audio.compressor threshold_db=-30 ratio=8 attack_ms=5 makeup_db=12",
    );
    let louder = rms(seconds(&raised, 1.0, 3.0));
    assert!((louder / after - 3.98).abs() < 0.5, "{after} to {louder}");
}

#[test]
fn a_gate_closes_on_what_is_quiet_and_leaves_what_is_loud() {
    let dir = tempfile::tempdir().expect("a folder");
    // Loud for a second, then very quiet.
    let path = source(
        dir.path(),
        "drop.wav",
        "if(lt(t\\,2)\\,0.5\\,0.003)*sin(2*PI*440*t)",
        4.0,
    );
    let (plain, gated) = with_effect(
        dir.path(),
        &path,
        "audio.gate threshold_db=-30 ratio=20 release_ms=50",
    );
    assert!(rms(seconds(&gated, 0.5, 1.5)) > rms(seconds(&plain, 0.5, 1.5)) * 0.7);
    assert!(rms(seconds(&gated, 3.0, 3.9)) < rms(seconds(&plain, 3.0, 3.9)) * 0.5);
}

#[test]
fn a_delay_repeats_the_sound_when_it_says() {
    let dir = tempfile::tempdir().expect("a folder");
    // A short burst at the start, then nothing.
    let path = source(
        dir.path(),
        "burst.wav",
        "if(lt(t\\,0.05)\\,0.5*sin(2*PI*1000*t)\\,0)",
        4.0,
    );
    let (plain, echoed) = with_effect(
        dir.path(),
        &path,
        "audio.delay time_ms=500 feedback=0.5 mix=1",
    );
    assert!(rms(seconds(&plain, 0.5, 0.56)) < 1e-4);
    let first = rms(seconds(&echoed, 0.5, 0.56));
    let second = rms(seconds(&echoed, 1.0, 1.06));
    assert!(first > 0.05, "{first}");
    assert!((second / first - 0.5).abs() < 0.1, "{first} then {second}");
    assert!(
        rms(seconds(&echoed, 0.2, 0.45)) < 1e-3,
        "nothing before the echo"
    );
}

#[test]
fn reverb_leaves_a_tail() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = source(
        dir.path(),
        "burst.wav",
        "if(lt(t\\,0.05)\\,0.5*sin(2*PI*1000*t)\\,0)",
        4.0,
    );
    let (plain, wet) = with_effect(dir.path(), &path, "audio.reverb size=0.8 mix=0.8");
    assert!(rms(seconds(&plain, 0.1, 0.3)) < 1e-4);
    assert!(rms(seconds(&wet, 0.1, 0.3)) > 0.01);
}

#[test]
fn denoising_takes_a_steady_noise_down_and_keeps_what_stands_above_it() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = dir.path().join("noisy.wav");
    let path = path.to_str().expect("UTF-8 path");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anoisesrc=c=white:a=0.05:r=48000:d=4:s=7",
        "-ac",
        "2",
        "-c:a",
        "pcm_f32le",
        path,
    ]);
    let fx = "audio.denoise reduction_db=30 noise_db=-20";
    let (plain, clean) = with_effect(dir.path(), path, fx);
    assert!(rms(seconds(&clean, 2.0, 3.5)) < rms(seconds(&plain, 2.0, 3.5)) * 0.6);
    let loud = source(dir.path(), "loud.wav", TONE, 4.0);
    let (plain, kept) = with_effect(dir.path(), &loud, fx);
    assert!(rms(seconds(&kept, 2.0, 3.5)) > rms(seconds(&plain, 2.0, 3.5)) * 0.8);
}

#[test]
fn a_de_esser_takes_down_highs_and_leaves_lows() {
    let dir = tempfile::tempdir().expect("a folder");
    let high = source(dir.path(), "high.wav", "0.4*sin(2*PI*9000*t)", 4.0);
    let (plain, softer) = with_effect(dir.path(), &high, "audio.deesser amount=1");
    assert!(rms(seconds(&softer, 1.0, 3.0)) < rms(seconds(&plain, 1.0, 3.0)) * 0.9);
    let low = source(dir.path(), "low.wav", "0.4*sin(2*PI*200*t)", 4.0);
    let (plain, same) = with_effect(dir.path(), &low, "audio.deesser amount=1");
    let (a, b) = (
        rms(seconds(&plain, 1.0, 3.0)),
        rms(seconds(&same, 1.0, 3.0)),
    );
    assert!((a - b).abs() < a * 0.1, "{a} {b}");
}

#[test]
fn buses_add_up_and_their_gains_stack() {
    let dir = tempfile::tempdir().expect("a folder");
    let path = source(dir.path(), "tone.wav", TONE, 4.0);
    let media = [("m", path.as_str())];
    let plain = rms(seconds(
        &mixed(dir.path(), &media, &format!("  track A1 audio\n{CLIP}")),
        1.0,
        3.0,
    ));
    let through = |body: &str| rms(seconds(&mixed(dir.path(), &media, body), 1.0, 3.0));
    // Each bus takes 6 dB.
    let one = through(&format!("  bus b gain=-6\n  track A1 audio bus=b\n{CLIP}"));
    assert!((one / plain - 0.501).abs() < 0.01, "{plain} {one}");
    let two = through(&format!(
        "  bus a gain=-6 to=b\n  bus b gain=-6\n  track A1 audio bus=a\n{CLIP}"
    ));
    assert!((two / plain - 0.251).abs() < 0.01, "{plain} {two}");
    // The same in the other order in the document: a bus is mixed before what it feeds.
    let reordered = through(&format!(
        "  bus b gain=-6\n  bus a gain=-6 to=b\n  track A1 audio bus=a\n{CLIP}"
    ));
    assert!((reordered - two).abs() < 1e-3, "{two} {reordered}");
    // A gain that moves is read from the sequence's start.
    let moving = mixed(
        dir.path(),
        &media,
        &format!("  bus b gain={{0f:0 96f:-12}}\n  track A1 audio bus=b\n{CLIP}"),
    );
    let (early, late) = (rms(seconds(&moving, 0.0, 0.2)), rms(seconds(&moving, 1.9, 2.1)));
    assert!((early / plain - 0.93).abs() < 0.1, "{plain} {early}");
    assert!((late / plain - 0.25).abs() < 0.03, "{plain} {late}");
}

#[test]
fn a_bus_hears_the_sum_of_what_feeds_it() {
    let dir = tempfile::tempdir().expect("a folder");
    let (a, b) = (
        source(dir.path(), "a.wav", "0.2*sin(2*PI*300*t)", 4.0),
        source(dir.path(), "b.wav", "0.2*sin(2*PI*450*t)", 4.0),
    );
    let media = [("a", a.as_str()), ("b", b.as_str())];
    let clip = |id: &str, m: &str| format!("    clip {id} media={m} src=0f at=0f dur=192f\n");
    let fx = "audio.compressor threshold_db=-14 ratio=20 attack_ms=2 release_ms=50";
    // On each clip, neither alone reaches the threshold by much; together they go well past it.
    let each = mixed(
        dir.path(),
        &media,
        &format!(
            "  track A1 audio\n{}      fx {fx}\n  track A2 audio\n{}      fx {fx}\n",
            clip("c1", "a"),
            clip("c2", "b")
        ),
    );
    let summed = mixed(
        dir.path(),
        &media,
        &format!(
            "  bus b\n    fx {fx}\n  track A1 audio bus=b\n{}  track A2 audio bus=b\n{}",
            clip("c1", "a"),
            clip("c2", "b")
        ),
    );
    let (apart, together) = (
        rms(seconds(&each, 1.0, 3.0)),
        rms(seconds(&summed, 1.0, 3.0)),
    );
    assert!(together < apart * 0.9, "{apart} against {together}");
}
