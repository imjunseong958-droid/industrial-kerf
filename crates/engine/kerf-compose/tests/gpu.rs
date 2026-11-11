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
    for (a, b) in a.rgba.as_chunks::<4>().0.iter().zip(b.rgba.as_chunks::<4>().0.iter()) {
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
        ("in depth", clip("      set three_d=true z=0.3 rotate_y=25 rotate_x=-10\n")),
        (
            "in depth, far off and cropped",
            clip("      set three_d=true z=5 crop_l=0.2 rotate_y=-15\n"),
        ),
        (
            "in depth, solid and lit",
            format!("{CLIP}      set three_d=true scale=0.5 depth=0.2 rotate_y=30\n  track V2 video\n    clip l1 gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 z=1\n"),
        ),
        (
            "in depth, with a soft shadow",
            format!("{CLIP}      set three_d=true z=0.2\n  track V2 video\n    clip c2 media=m src=4f at=0f dur=20f\n      set three_d=true scale=0.4 z=-0.3\n      mask k1 ellipse=0.5,0.5,0.4,0.4\n  track V3 video\n    clip l1 gen=light@1 at=0f dur=24f\n      set kind=directional x=0.2 z=1 shadow=1 size=6\n"),
        ),
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
        ("lumetri look", clip("      fx color.lumetri look=monochrome look_intensity=70\n")),
        ("lumetri wheels", clip("      fx color.lumetri wheel_shadows_x=-0.25 wheel_midtones_y=0.2 wheel_highlights_x=0.12 wheel_shadows_l=10\n")),
        ("lumetri both", clip("      set scale=0.9\n      fx color.lumetri exposure=-0.4 look=teal-orange wheel_midtones_x={0f:0 24f:0.4}\n")),
        ("sepia", clip("      fx look.grade.sepia@1 amount=0.6\n")),
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
        ("color key", clip("      fx key.color color=#30c040 tolerance=120 feather=20\n")),
        ("luma key", clip("      fx key.luma threshold=70 cutoff=30\n")),
        ("logo cutout", clip("      fx key.logocutout background=black threshold=10\n")),
        ("ultra key", clip("      fx key.ultra key_color=#30c040\n")),
        ("radial zoom", clip("      fx blur.radialzoom amount=30 quality=30\n")),
        ("radial spin", clip("      fx blur.radialzoom type=rotate-fading amount=-20 center_x=0.3 center_y=0.6\n")),
        ("ultra choked", clip("      fx key.ultra key_color=#30c040 setting=custom choke=30\n")),
        ("ultra softened", clip("      fx key.ultra key_color=#30c040 setting=custom soften=25 contrast=20\n")),
        ("lumetri sharpened", clip("      fx color.lumetri exposure=0.3 sharpen=60\n")),
        ("lumetri curves", clip("      fx color.lumetri curve_luma=\"0,0 0.3,0.2 0.7,0.85 1,1\" curve_red=\"0,0 0.5,0.6 1,1\"\n")),
        ("lumetri hue curves", clip("      fx color.lumetri hue_vs_sat=\"0,0.5 0.33,0.8 0.66,0.3\" hue_vs_hue=\"0,0.5 0.5,0.6\"\n")),
        ("lumetri HSL", clip("      fx color.lumetri hsl_on=true hsl_hue=120 hsl_hue_range=60 hsl_sat=150 hsl_blur=20\n")),
        ("leaks", clip("      fx light.leaks intensity=80\n")),
        ("grain", clip("      fx noise.addgrain tweaking/intensity=2\n")),
        ("look.film", clip("      fx look.film\n")),
        // As tracking a surface writes it: each corner across the picture, and moving.
        ("corner pin", clip("      fx distort.cornerpin upper_left_x=0.1 upper_left_y=0.15 upper_right_x=0.85 upper_right_y=0.05 lower_right_x=0.95 lower_right_y=0.9 lower_left_x=0.05 lower_left_y=0.8\n")),
        ("corner pin tracked", clip("      set scale=0.9\n      fx distort.cornerpin upper_left_x={0f:0.1 24f:0.3} upper_left_y={0f:0.1 24f:0.2} upper_right_x={0f:0.9 24f:0.8} upper_right_y=0.15 lower_right_x={0f:0.85 24f:0.95} lower_right_y={0f:0.9 24f:0.75} lower_left_x=0.15 lower_left_y={0f:0.85 24f:0.95}\n")),
        ("corner pin past the edges, then blurred", clip("      fx distort.cornerpin upper_left_x=-0.2 upper_left_y=-0.1 upper_right_x=1.1 upper_right_y=0.1 lower_right_x=0.9 lower_right_y=1.2 lower_left_x=0.1 lower_left_y=0.9\n      fx blur.gaussian radius=6\n")),
        ("drop shadow", clip("      set scale=0.7\n      mask k1 ellipse=0.5,0.5,0.4,0.45\n      fx perspective.dropshadow distance=14 softness=10 opacity=70\n")),
        ("drop shadow alone", clip("      set scale=0.8\n      mask k1 rect=0.2,0.2,0.8,0.8\n      fx perspective.dropshadow color=#3050c0 direction=250 distance=9 only=true\n")),
        ("rounded crop", clip("      fx utility.roundedcrop left=8 top=5 right=12 bottom=10 radius=60 border=6 border_color=#ffd040\n")),
        ("rounded crop feathered", clip("      set scale=0.9\n      fx utility.roundedcrop radius=30 feather=18\n")),
        ("spacer", clip("      fx transform.spacer left=30 radius=24 fill=true color=#203040\n")),
        ("spacer to under half", clip("      fx transform.spacer uniform=false left=40 top=110 right=20 bottom=20\n")),
        ("mosaic", clip("      fx stylize.mosaic horizontal=24 vertical=14\n")),
        ("mosaic soft", clip("      set scale=0.9\n      fx stylize.mosaic horizontal=17 vertical=9 softness=70\n")),
        ("curves", clip("      fx color.curves rgb=\"0,0 0.3,0.2 0.7,0.85 1,1\" red=\"0,0 0.5,0.6 1,1\"\n")),
        ("curves past their ends", clip("      fx color.curves rgb=\"0.15,0 0.85,1\" blue=\"0,0.1 0.5,0.45 1,0.9\" alpha=\"0,0.3 1,1\"\n")),
        // A hue turned by 25 and not 30: 30 puts the footage's pure colors half way between two bytes,
        // which the two then round apart, as `color.procamp hue=30` shows on a kernel long held.
        ("hue and saturation", clip("      fx color.huesaturation hue=25 saturation=25 redsSaturation=-60 bluesHue=40\n")),
        ("colorized", clip("      set scale=0.9\n      fx color.huesaturation colorize=true colorizeHue=200 colorizeSaturation=45\n")),
        ("selective color", clip("      fx color.selectivecolor details/reds/redsCyan=40 details/neutrals/neutralsYellow=30 details/blues/bluesBlack=-25\n")),
        ("photo filter", clip("      fx color.photofilter filter=cooling-filter-80 density=60\n")),
        (
            "unmult",
            "  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n      set color=#336699\n  track V2 video\n    clip c1 media=m src=0f at=0f dur=24f\n      fx key.unmult blackLevel=10 softness=60\n".to_owned(),
        ),
        ("cineon to linear", clip("      fx utility.cineon gamma=2 rolloff=30\n")),
        ("cineon to log", clip("      fx utility.cineon conversion=linear-to-log\n")),
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
    for look in ["look.whip", "look.flash-cut", "look.zoom-through"] {
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

/// A transition's two clips between frames drawn both ways, each moment one the GPU really draws. The
/// CPU's `apply_scaled` is the reference, and `kerf-fx/src/transitions/gpu_tests.rs` holds each kernel
/// to it per value; this holds what goes around them: the clips placed and masked and turned on their
/// way in, the blend out, and frames from footage.
#[test]
fn every_transition_draws_as_the_cpu_draws_it() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let media = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n",
        footage(dir.path())
    );
    let mut failures = Vec::new();
    let transitions = Catalog::builtin()
        .all()
        .filter(|def| kerf_fx::transitions::filmcraft_id(def.id.as_str()).is_some())
        .map(|def| def.id.as_str())
        .filter(|id| !["morph-cut"].contains(id));
    for id in transitions {
        let document = format!(
            "{media}  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx={id}@1 dur=12f\n"
        );
        let gpu = compositor(&document, Accel::Always, false);
        let cpu = compositor(&document, Accel::Never, false);
        for moment in [7, 11, 15] {
            let time = Rate::FPS_24.time_of(moment);
            if !gpu.draws_on_gpu(time, 1.0) {
                failures.push(format!("{id}: the CPU drew {moment}f"));
                continue;
            }
            let (mean, off) = difference(&gpu.frame(time, 1.0), &cpu.frame(time, 1.0), 2);
            // As `every_scene_draws_as_the_cpu_draws_it` allows, and a little more for where a hard
            // threshold of Film Roll lands on a pixel of this size.
            if mean > 0.6 || off > 0.005 {
                failures.push(format!(
                    "{id} at {moment}f: mean {mean:.3}, {:.3}% off by more than 2",
                    off * 100.0
                ));
            }
            // A device that failed has the CPU draw, which would pass for a match.
            assert!(gpu.draws_on_gpu(time, 1.0), "{id} failed the device");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What goes around a transition's kernel: each side as it would draw alone, its blend, and a clip that
/// enters from nothing.
#[test]
fn a_transition_keeps_what_its_clips_are() {
    if kerf_gpu_missing() {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder");
    let media = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=24f video=320x180\nsequence s1 rate=24 size=320x180 audio=48000\n",
        footage(dir.path())
    );
    let under = "  track V1 video\n    clip u media=m src=0f at=0f dur=24f\n";
    let cases = [
        (
            "placed, masked, and graded",
            format!("{under}  track V2 video\n    clip c1 media=m src=0f at=0f dur=12f\n      set scale=0.7 rotation=10\n      mask k1 ellipse=0.5,0.5,0.4,0.4\n      fx color.basic exposure=0.3\n    clip c2 media=m src=6f at=12f dur=12f\n      set scale=0.5 x=0.1\n    transition t1 from=c1 to=c2 fx=iris-round@1 dur=12f\n"),
        ),
        (
            "over what is below, from nothing, by its blend",
            format!("{under}  track V2 video\n    clip c0 media=m src=0f at=0f dur=12f enabled=false\n    clip c1 media=m src=6f at=12f dur=12f\n      set blend=screen scale=0.8\n    transition t1 from=c0 to=c1 fx=glow@1 dur=8f\n"),
        ),
        (
            "in depth",
            "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n      set three_d=true rotate_y=25 z=0.3\n    clip c2 media=m src=6f at=12f dur=12f\n      set three_d=true rotate_x=-15\n    transition t1 from=c1 to=c2 fx=push@1 dur=12f\n  track V2 video\n    clip l1 gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 z=1\n".to_owned(),
        ),
        (
            "in HDR, whose values go past 1",
            "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n      fx color.basic exposure=1.5\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx=light-sweep@1 dur=12f\n".to_owned(),
        ),
        (
            "nested",
            "  track V1 video\n    clip c1 seq=inner src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx=roll-3d@1 dur=12f\n".to_owned(),
        ),
    ];
    for (name, body) in cases {
        let inner = "sequence inner rate=24 size=320x180 audio=48000\n  track V1 video\n    clip n1 media=m src=0f at=0f dur=24f\n      fx look.grade.sepia@1\n";
        let document = if name == "nested" {
            format!("{media}{body}{inner}")
        } else if name.starts_with("in HDR") {
            let hdr = media.replace("audio=48000", "audio=48000 color=rec2100-pq");
            format!("{hdr}{body}")
        } else {
            format!("{media}{body}")
        };
        let gpu = compositor(&document, Accel::Always, false);
        let cpu = compositor(&document, Accel::Never, false);
        for moment in [8, 12, 15] {
            let time = Rate::FPS_24.time_of(moment);
            assert!(gpu.draws_on_gpu(time, 1.0), "{name} at {moment}f");
            let (mean, off) = difference(&gpu.frame(time, 1.0), &cpu.frame(time, 1.0), 2);
            assert!(
                mean <= 0.6 && off <= 0.005,
                "{name} at {moment}f: mean {mean:.3}, {:.3}% off by more than 2",
                off * 100.0
            );
            // A device that failed has the CPU draw, which would pass for a match.
            assert!(gpu.draws_on_gpu(time, 1.0), "{name} failed the device");
        }
    }
}

/// A compositor that draws each frame it is asked for, so that a kept frame cannot pass for a drawn one.
fn drawing_every_frame(document: &str) -> Compositor {
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Always,
            keep_frames: false,
            frame_folder: None,
            captions: true,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

/// A title's picture and distance field stay from the second frame that asks for them, which must not
/// change a byte: whether it was drawn or kept, and whatever has been drawn from it.
#[test]
fn a_kept_title_draws_what_it_drew() {
    if kerf_gpu_missing() {
        return;
    }
    let head = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=640x360 audio=48000\n";
    let light = "  track V3 video\n    clip l1 gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4\n";
    let wall = "  track V1 video\n    clip w1 gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.3\n";
    let title = |rest: &str| {
        format!("  track V2 video\n    clip t1 gen=title@1 at=0f dur=24f\n      set text=\"Kept\" size=0.3{rest}\n")
    };
    let scenes = [
        ("flat", title("")),
        ("placed", title(" scale=0.7 rotation=12")),
        (
            "masked",
            format!(
                "{}      mask k1 ellipse=0.5,0.5,0.3,0.3 feather=12\n",
                title("")
            ),
        ),
        (
            "blurred",
            format!("{}      fx blur.gaussian radius=20\n", title("")),
        ),
        (
            "graded",
            format!("{}      fx color.basic exposure=0.5\n", title("")),
        ),
        (
            "in depth",
            format!("{wall}{}{light}", title(" three_d=true")),
        ),
        (
            "solid",
            format!("{}{light}", title(" three_d=true depth=0.1")),
        ),
        (
            "beveled",
            format!("{}{light}", title(" three_d=true depth=0.1 bevel=0.02")),
        ),
        (
            "beveled, with a soft shadow",
            format!(
                "{wall}{}{light}",
                title(" three_d=true depth=0.1 bevel=0.02")
            ),
        ),
        (
            "turning, beveled",
            format!(
                "{}{light}",
                title(" three_d=true depth=0.1 bevel=0.02 rotate_y={0f:-20 24f:20}")
            ),
        ),
        ("revealed", title(" reveal={0f:0 4f:1} from_y=0.05")),
        (
            "under a caption",
            format!(
                "{}  track S1 subtitle\n    clip k1 caption=\"Line\" at=0f dur=24f\n",
                title("")
            ),
        ),
    ];
    let time = Rate::FPS_24.time_of(10);
    for (name, body) in &scenes {
        let compositor = drawing_every_frame(&format!("{head}{body}"));
        assert!(compositor.draws_on_gpu(time, 1.0), "{name}");
        let drawn = compositor.frame(time, 1.0);
        assert!(
            drawn.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[0] > 40),
            "{name} draws nothing"
        );
        // Made, then made and kept, then read back twice.
        for _ in 0..3 {
            assert_eq!(compositor.frame(time, 1.0), drawn, "{name}");
        }
        assert!(
            compositor.draws_on_gpu(time, 1.0),
            "{name} failed the device"
        );
    }
    // What was drawn from a kept picture leaves it as it was, in another compositor too.
    let plain = drawing_every_frame(&format!("{head}{}", title(" scale=0.9")));
    let before = plain.frame(time, 1.0);
    let blurred = drawing_every_frame(&format!(
        "{head}{}      mask k1 ellipse=0.5,0.5,0.2,0.2\n      fx blur.gaussian radius=30\n      fx channel.invert\n",
        title(" scale=0.9")
    ));
    for _ in 0..3 {
        assert_ne!(blurred.frame(time, 1.0), before);
    }
    for _ in 0..3 {
        assert_eq!(plain.frame(time, 1.0), before);
    }
    // Other words, and a number that moves, are not what was kept.
    let other = drawing_every_frame(&format!("{head}  track V2 video\n    clip t1 gen=title@1 at=0f dur=24f\n      set text=\"Tept\" size=0.3 scale=0.9\n"));
    for _ in 0..2 {
        assert_ne!(other.frame(time, 1.0), before);
    }
    let moving = drawing_every_frame(&format!("{head}{}", title(" reveal={0f:0 24f:1}")));
    let frames: Vec<Frame> = (6..10)
        .map(|at| moving.frame(Rate::FPS_24.time_of(at), 1.0))
        .collect();
    for pair in frames.windows(2) {
        assert_ne!(pair[0], pair[1], "a reveal under way stayed where it was");
    }
    assert!(plain.draws_on_gpu(time, 1.0), "the device failed");
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
        format!("{CLIP}      fx noise.median\n"),
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=6f at=12f dur=12f\n    transition t1 from=c1 to=c2 fx=morph-cut@1 dur=12f\n".to_owned(),
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

/// A second of test footage in `pixel` format, with what `args` says of its color.
fn made_as(dir: &Path, name: &str, codec: &[&str], filter: &str) -> Option<String> {
    let path = dir.join(name);
    let output = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=320x180:r=24:d=1"])
        .args(["-vf", filter])
        .args(codec)
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    // Not every FFmpeg has every encoder at every depth.
    output.status.success().then(|| path.display().to_string())
}

/// How far the GPU's frame of `document` is from the CPU's, as the mean and the worst of 999 in 1000
/// of its channels, and the worst of them all. A clip a touch see-through goes to neither's shortcut.
fn apart(document: &str) -> Option<(f64, u8, u8)> {
    let project = parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let make = |accel| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                accel,
                keep_frames: false,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let at = Rate::FPS_24.time_of(5);
    let gpu = make(Accel::Always);
    if !gpu.draws_on_gpu(at, 1.0) {
        return None;
    }
    let (on_gpu, on_cpu) = (gpu.frame(at, 1.0), make(Accel::Never).frame(at, 1.0));
    let mut off: Vec<u8> = on_gpu
        .rgba
        .iter()
        .zip(&on_cpu.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .collect();
    let mean = off.iter().map(|value| f64::from(*value)).sum::<f64>() / off.len() as f64;
    off.sort_unstable();
    Some((mean, off[off.len() * 999 / 1000], off[off.len() - 1]))
}

#[test]
fn frames_the_gpu_makes_from_their_planes_match_what_the_cpu_makes_of_them() {
    let dir = tempfile::tempdir().expect("a folder");
    let x264 = ["-c:v", "libx264", "-crf", "6"];
    let tagged = |matrix: &str, trc: &str, primaries: &str, pixel: &str, range: &str| {
        format!(
            "scale=out_color_matrix={matrix}:out_range={range},format={pixel},setparams=range={range}:colorspace={matrix}:color_trc={trc}:color_primaries={primaries}"
        )
    };
    let cases: Vec<(&str, Vec<&str>, String, &str)> = vec![
        ("plain", x264.to_vec(), "format=yuv420p".to_owned(), ""),
        (
            "bt709",
            x264.to_vec(),
            tagged("bt709", "bt709", "bt709", "yuv420p", "tv"),
            "",
        ),
        (
            "bt601",
            x264.to_vec(),
            tagged("smpte170m", "smpte170m", "smpte170m", "yuv420p", "tv"),
            "",
        ),
        (
            "bt2020",
            x264.to_vec(),
            tagged("bt2020nc", "bt709", "bt2020", "yuv420p", "tv"),
            "",
        ),
        (
            "full",
            x264.to_vec(),
            tagged("bt709", "bt709", "bt709", "yuv420p", "pc"),
            "",
        ),
        ("422", x264.to_vec(), "format=yuv422p".to_owned(), ""),
        ("444", x264.to_vec(), "format=yuv444p".to_owned(), ""),
        (
            "422_10",
            x264.to_vec(),
            "format=yuv422p10le".to_owned(),
            " bits=10",
        ),
        (
            "420_10",
            x264.to_vec(),
            "format=yuv420p10le".to_owned(),
            " bits=10",
        ),
        (
            "prores",
            vec!["-c:v", "prores_ks", "-profile:v", "3"],
            "format=yuv422p10le".to_owned(),
            " bits=10",
        ),
    ];
    let mut read = 0;
    for (name, codec, filter, fields) in cases {
        let extension = if name == "prores" { "mov" } else { "mp4" };
        let Some(path) = made_as(dir.path(), &format!("{name}.{extension}"), &codec, &filter)
        else {
            eprintln!("{name}: this FFmpeg cannot make it");
            continue;
        };
        let document = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=320x180{fields}\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      set opacity=0.99\n"
        );
        let Some((mean, most, worst)) = apart(&document) else {
            eprintln!("no GPU to compare with");
            return;
        };
        eprintln!("{name}: mean {mean:.3}, 99.9% within {most}, worst {worst}");
        // Deep frames come within 3; 8-bit ones have to come to the same bytes, or a frame on the GPU
        // would differ from the same frame that skips it.
        if fields.is_empty() {
            assert_eq!(worst, 0, "{name}: {mean} {most}");
        } else {
            assert!(mean < 0.5 && worst <= 3, "{name}: {mean} {most} {worst}");
        }
        read += 1;
    }
    assert!(read >= 7, "only {read} of the cases could be made");
    // A width libswscale's fast paths do not take, and an odd height: as RGBA, as it always went.
    let Some(path) = made_as(dir.path(), "odd.mp4", &x264, "scale=330:182,format=yuv420p") else {
        return;
    };
    let document = format!(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"{path}\" rate=24 dur=24f video=330x182\nsequence s1 rate=24 size=330x182 audio=48000\n  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      set opacity=0.99\n"
    );
    if let Some((mean, most, worst)) = apart(&document) {
        eprintln!("odd width: mean {mean:.3}, 99.9% within {most}, worst {worst}");
        assert_eq!(worst, 0, "{mean} {most}");
    }
}
