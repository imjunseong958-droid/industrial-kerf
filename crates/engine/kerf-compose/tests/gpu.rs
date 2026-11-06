//! The GPU against the CPU, the reference: the same document, on footage it decodes, drawn both
//! ways. Each scene also has to be one the GPU really draws, so a fallback cannot pass for a match.
//! On a machine without a GPU there is nothing to compare.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned())
}

/// Moving test footage, 320x180 at 24 fps for a second.
fn footage(dir: &Path) -> String {
    let path = dir.join("footage.mp4");
    let output = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=320x180:r=24:d=1"])
        .args(["-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p"])
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

fn compositor(document: &str, accel: Accel, captions: bool) -> Compositor {
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: None,
            captions,
            accel,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

/// The mean difference of a channel value and how many pixels are off by more than `allowed`.
fn difference(a: &Frame, b: &Frame, allowed: u8) -> (f64, f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (mut sum, mut off) = (0u64, 0u64);
    for (a, b) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        let worst = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
        sum += u64::from(worst);
        off += u64::from(worst > allowed);
    }
    let pixels = u64::from(a.width) * u64::from(a.height);
    (sum as f64 / pixels as f64, off as f64 / pixels as f64)
}

/// A clip line and what goes under it.
const CLIP: &str = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n";

/// Under the first clip, a second to draw over.
const OVER: &str = "  track V2 video\n    clip c2 media=m src=4f at=0f dur=20f\n";

/// (name, what follows the sequence line).
fn scenes() -> Vec<(&'static str, String)> {
    let clip = |body: &str| format!("{CLIP}{body}");
    let over = |body: &str| format!("{CLIP}{OVER}{body}");
    let mut scenes = vec![
        ("scaled", clip("      set scale=0.8\n")),
        ("moved", clip("      set x=0.1 y=-0.05 scale=0.7\n")),
        ("turned", clip("      set rotation=17 scale=0.8\n")),
        ("flipped", clip("      set scale_x=-1 scale=0.9\n")),
        ("cropped", clip("      set crop_l=0.1 crop_t=0.05 crop_r=0.2 scale=0.9\n")),
        ("faint", clip("      set opacity=0.6 scale=0.9\n")),
        (
            "animated",
            clip("      set scale={0f:0.5 24f:1:ease} opacity={0f:0 12f:1}\n"),
        ),
        (
            "masked",
            clip("      mask k1 ellipse=0.5,0.5,0.3,0.4 feather=12\n"),
        ),
        (
            "two masks",
            clip("      mask k1 rect=0.1,0.1,0.9,0.9\n      mask k2 ellipse=0.5,0.5,0.2,0.2 mode=subtract\n"),
        ),
        ("lumetri", clip("      set scale=0.95\n      fx color.lumetri exposure=0.5 contrast=20 saturation=130\n")),
        ("graded", clip("      fx color.levels gamma=120\n      fx color.tint amount=40\n")),
        ("blur", clip("      fx blur.gaussian radius=24\n")),
        ("wide blur", clip("      fx blur.gaussian radius=160\n")),
        ("directional", clip("      fx blur.directional direction=30 length=12\n")),
        ("sharpen", clip("      fx blur.unsharp amount=120 radius=3\n")),
        ("corrected", clip("      fx color.basic exposure=0.4 contrast=1.2 saturation=1.3 temperature=200\n")),
        ("effects masked", clip("      mask k1 rect=0.2,0.2,0.8,0.8 feather=8\n      fx channel.invert\n")),
        (
            "solid under",
            "  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#336699\n  track V2 video\n    clip c1 media=m src=0f at=0f dur=24f\n      set scale=0.6 opacity=0.8\n".to_owned(),
        ),
        (
            "title over",
            format!("{CLIP}  track V2 video\n    clip t1 gen=title@1 at=0f dur=24f\n      set text=\"KERF\" size=0.25\n"),
        ),
    ];
    let adjustment = |body: &str| {
        format!("{CLIP}      set scale=0.9\n  track V2 video\n    clip a1 gen=adjustment@1 at=0f dur=24f\n{body}")
    };
    scenes.extend([
        ("adjusted", adjustment("      fx color.tint amount=60\n")),
        ("adjusted faint", adjustment("      set opacity=0.5\n      fx color.lumetri exposure=0.4\n")),
        ("adjusted masked", adjustment("      mask k1 ellipse=0.5,0.5,0.3,0.3 feather=20\n      fx blur.gaussian radius=16\n")),
        ("adjusted moved", adjustment("      set scale=0.5 x=0.1 rotation=10\n      fx channel.invert\n")),
        ("adjusted blended", adjustment("      set blend=screen opacity=0.7\n      fx color.basic exposure=0.5\n")),
        ("glow", clip("      fx light.wonderglow threshold=30 radius=30\n")),
        ("glint", clip("      fx light.glint threshold=60\n")),
        ("split", clip("      fx light.rgbsplit amount=8\n")),
        ("wave", clip("      fx distort.wavewarp height=8 width=60\n")),
        ("noise", clip("      fx noise.noise amount=30 grain_size=2\n")),
    ]);
    for look in [
        "look.dreamy",
        "look.glitch",
        "look.vhs",
        "look.noir",
        "look.sparkle",
        "look.shake",
        "look.flash",
    ] {
        scenes.push((look, clip(&format!("      fx {look}\n"))));
    }
    for look in ["look.whip", "look.flash-cut"] {
        scenes.push((look, format!("  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx={look} dur=12f\n")));
    }
    for mode in [
        "add",
        "multiply",
        "screen",
        "overlay",
        "darken",
        "lighten",
        "difference",
    ] {
        let name: &'static str = Box::leak(format!("blend {mode}").into_boxed_str());
        scenes.push((name, over(&format!("      set blend={mode} opacity=0.8\n"))));
    }
    scenes
}

#[test]
fn every_scene_draws_as_the_cpu_draws_it() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let media = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=640x360 audio=48000\n",
        footage(dir.path())
    );
    let mut failures = Vec::new();
    for (name, body) in scenes() {
        let document = format!("{media}{body}");
        let gpu = compositor(&document, Accel::Always, false);
        let cpu = compositor(&document, Accel::Never, false);
        for at in [0, 11] {
            let time = Rate::FPS_24.time_of(at);
            assert!(gpu.draws_on_gpu(time, 1.0), "{name} goes to the CPU");
            let on_gpu = gpu.frame(time, 1.0);
            let on_cpu = cpu.frame(time, 1.0);
            // A byte either way is rounding. A dissolve's edge and a title's own glyphs excepted,
            // nothing should go further.
            let (mean, off) = difference(&on_gpu, &on_cpu, 2);
            if mean > 0.6 || off > 0.002 {
                failures.push(format!(
                    "{name} at {at}: mean {mean:.3}, {:.3}% off",
                    off * 100.0
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn what_the_kernels_cannot_draw_goes_to_the_cpu() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let media = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n",
        footage(dir.path())
    );
    let time = Rate::FPS_24.time_of(6);
    let refused = [
        format!("{CLIP}      fx light.leaks\n"),
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx=push@1 dur=12f\n".to_owned(),
    ];
    for body in refused {
        let document = format!("{media}{body}");
        let gpu = compositor(&document, Accel::Always, false);
        let cpu = compositor(&document, Accel::Never, false);
        assert!(!gpu.draws_on_gpu(time, 1.0), "{body}");
        assert_eq!(gpu.frame(time, 1.0), cpu.frame(time, 1.0), "{body}");
    }
}

#[test]
fn small_frames_stay_on_the_cpu_unless_asked() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n{CLIP}      set scale=0.9\n",
        footage(dir.path())
    );
    let time = Rate::FPS_24.time_of(0);
    let auto = compositor(&document, Accel::Auto, false);
    let cpu = compositor(&document, Accel::Never, false);
    // Exactly the reference: it is the reference that drew it.
    assert_eq!(auto.frame(time, 1.0), cpu.frame(time, 1.0));
}

#[test]
fn burned_in_captions_draw_as_the_cpu_draws_them() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=640x360 audio=48000\n{CLIP}      set scale=0.9\n  track S1 subtitle\n    clip cap1 caption=\"Where are we?\" at=0f dur=24f\n",
        footage(dir.path())
    );
    let time = Rate::FPS_24.time_of(3);
    let gpu = compositor(&document, Accel::Always, true);
    let cpu = compositor(&document, Accel::Never, true);
    assert!(gpu.draws_on_gpu(time, 1.0));
    let (mean, off) = difference(&gpu.frame(time, 1.0), &cpu.frame(time, 1.0), 2);
    assert!(
        mean < 0.6 && off < 0.002,
        "mean {mean:.3}, {:.3}% off",
        off * 100.0
    );
}

/// Whether there is no GPU to compare with.
fn kerf_gpu_missing() -> bool {
    let missing = compositor(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#336699\n",
        Accel::Always,
        false,
    )
    .draws_on_gpu(Rate::FPS_24.time_of(0), 1.0)
    .eq(&false);
    if missing {
        eprintln!("no GPU: nothing to compare");
    }
    missing
}
