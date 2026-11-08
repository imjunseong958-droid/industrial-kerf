//! Clips in depth on the GPU against the CPU, the reference: the same scene drawn both ways, within
//! what the architecture says it comes to. Each has to be a scene the GPU really draws, so a fallback
//! cannot pass for a match. On a machine without a GPU there is nothing to compare.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

/// The most a scene may differ from the CPU on average over the frame, in 255ths. What each scene
/// comes to is printed with `--nocapture`, and `docs/architecture.md` says what these hold.
const MEAN: f64 = 0.05;
/// How far a pixel may be off, in 255ths, and what share of a frame may be off by more. Those are a
/// solid's sides facing straight up or down, where a table of light has no one way round.
const OFF_BY: u8 = 2;
const SHARE_OFF: f64 = 0.0005;

fn compositor(body: &str, accel: Accel) -> Compositor {
    let text = if body.starts_with("sequence") {
        format!("kerf 1\nproject name=\"t\"\n{body}")
    } else {
        format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=960x540 audio=48000\n{body}")
    };
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel,
            frame_folder: None,
            keep_frames: false,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

fn difference(a: &Frame, b: &Frame) -> (f64, f64, u8) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (mut sum, mut off, mut most) = (0u64, 0u64, 0u8);
    for (a, b) in a.rgba.as_chunks::<4>().0.iter().zip(b.rgba.as_chunks::<4>().0.iter()) {
        let worst = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
        sum += u64::from(worst);
        off += u64::from(worst > OFF_BY);
        most = most.max(worst);
    }
    let pixels = u64::from(a.width) * u64::from(a.height);
    (sum as f64 / pixels as f64, off as f64 / pixels as f64, most)
}

/// The scene drawn both ways, which has to draw something and to differ by no more than is stated.
fn holds(name: &str, body: &str) {
    let at = Rate::FPS_24.time_of(12);
    if kerf_gpu::shared().is_none() {
        eprintln!("{name}: no GPU here");
        return;
    }
    let gpu = compositor(body, Accel::Always);
    assert!(gpu.draws_on_gpu(at, 1.0), "{name} went to the CPU");
    let cpu = compositor(body, Accel::Never);
    // A frame given up partway goes to the CPU, which would pass for a match.
    let drawn = gpu
        .frame_on_gpu(at, 1.0)
        .unwrap_or_else(|| panic!("{name} gave up on the GPU"));
    let reference = cpu.frame(at, 1.0);
    // And so does a kernel the device refuses.
    assert!(kerf_gpu::shared().is_some(), "{name} failed the device");
    assert!(
        reference.rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]),
        "{name} draws nothing"
    );
    let (mean, off, most) = difference(&drawn, &reference);
    println!(
        "{name:<28} mean {mean:.3}, off by >{OFF_BY} {:.3}%, most {most}",
        off * 100.0
    );
    assert!(
        mean <= MEAN && off <= SHARE_OFF,
        "{name}: mean {mean:.3}, {:.3}% off by more than {OFF_BY}, at most {most}",
        off * 100.0
    );
}

/// A white wall, a blue square before it, and a light.
fn wall(light: &str, square: &str) -> String {
    format!(
        "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip s {square}\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
    )
}

const SQUARE: &str =
    "gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3";
const TITLE: &str =
    "gen=title@1 at=0f dur=24f\n      set text=\"Soft\" size=0.4 three_d=true z=-0.2";

#[test]
fn flat_clips_hold_to_the_cpu() {
    holds(
        "apart",
        "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.6 z=0.2 rotate_y=20\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.4 z=-0.3 rotate_y=-20\n",
    );
    holds(
        "far smaller",
        "  track V1 video\n    clip a gen=title@1 at=0f dur=24f\n      set text=\"Far off\" size=0.3 three_d=true z=4\n",
    );
    holds(
        "camera moved",
        "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffaa33 three_d=true rotate_x=25 scale=0.7\n  track V2 video\n    clip c gen=camera@1 at=0f dur=24f\n      set x=0.1 y=-0.1 z=-0.2 pan=10 tilt=-5 roll=8 fov=50\n",
    );
}

#[test]
fn crossing_clips_hold_to_the_cpu() {
    holds(
        "two crossing",
        "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.6 z=0.1 rotate_y=45\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.6 rotate_y=-30\n",
    );
    holds(
        "three crossing, one blended",
        "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.6 rotate_y=45\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.6 rotate_y=-30 opacity=0.6\n  track V3 video\n    clip c gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.5 rotate_x=40 blend=screen\n",
    );
}

#[test]
fn hard_shadows_hold_to_the_cpu() {
    holds("lit", &wall("kind=directional x=0.2 z=1", SQUARE));
    holds(
        "directional shadow",
        &wall("kind=directional x=0.2 z=1 shadow=1", SQUARE),
    );
    holds(
        "point shadow",
        &wall("x=0.2 y=-0.1 z=-0.6 shadow=0.8", SQUARE),
    );
    holds(
        "shadow of a title",
        &wall("kind=directional x=0.2 z=1 shadow=1", TITLE),
    );
}

#[test]
fn soft_shadows_hold_to_the_cpu() {
    holds(
        "soft directional",
        &wall("kind=directional x=0.2 z=1 shadow=1 size=5", SQUARE),
    );
    holds(
        "soft point",
        &wall("x=0.2 y=-0.1 z=-0.6 shadow=1 size=0.1", SQUARE),
    );
    holds(
        "soft all over",
        &wall(
            "kind=directional z=1 shadow=1 size=30",
            "gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true z=-0.3",
        ),
    );
    holds(
        "soft shadow of a title",
        &wall("kind=directional x=0.2 z=1 shadow=1 size=10", TITLE),
    );
    holds(
        "soft through two",
        "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.3 z=-0.3 x=-0.1\n  track V3 video\n    clip b gen=title@1 at=0f dur=24f\n      set text=\"Two\" size=0.3 three_d=true z=-0.5 opacity=0.7\n  track V4 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=0.1 z=1 shadow=0.9 size=8\n",
    );
}

#[test]
fn solids_hold_to_the_cpu() {
    let light = "kind=directional x=-0.4 y=0.3 z=1";
    holds(
        "deep square",
        &format!(
            "  track V1 video\n    clip s gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.3 x=-0.2 depth=0.3\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
        ),
    );
    holds(
        "deep title turned",
        &format!(
            "  track V1 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 rotate_y=25\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
        ),
    );
    holds(
        "deep title with a soft shadow",
        "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.3\n  track V2 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4\n",
    );
    holds(
        "deep title, no light",
        "  track V1 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Depth\" size=0.3 three_d=true depth=0.1 rotate_y=-30 rotate_x=10\n",
    );
}

#[test]
fn bevels_hold_to_the_cpu() {
    let on = |set: &str, light: &str| {
        format!(
            "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.5\n  track V2 video\n    clip t {set}\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
        )
    };
    let title = "gen=title@1 at=0f dur=24f\n      set text=\"Cut\" size=0.3 three_d=true depth=0.1 bevel=0.03";
    let square = "gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.4 depth=0.2 bevel=0.04 rotate_y=30 rotate_x=-20";
    holds(
        "beveled title, lit",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1"),
    );
    holds(
        "beveled square, turned",
        &on(square, "kind=directional x=-0.4 y=0.3 z=1"),
    );
    holds(
        "beveled title, hard shadow",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1 shadow=1"),
    );
    holds(
        "beveled title, soft shadow",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4"),
    );
    holds(
        "beveled square, soft shadow",
        &on(square, "x=0.4 y=-0.3 z=-0.8 shadow=0.8 size=0.1"),
    );
}

#[test]
fn round_bevels_hold_to_the_cpu() {
    let on = |set: &str, light: &str| {
        format!(
            "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.5\n  track V2 video\n    clip t {set}\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
        )
    };
    let title = "gen=title@1 at=0f dur=24f\n      set text=\"Cut\" size=0.3 three_d=true depth=0.1 bevel=0.03 bevel_profile=round";
    let square = "gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.4 depth=0.2 bevel=0.04 bevel_profile=round rotate_y=30 rotate_x=-20";
    holds(
        "round title, lit",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1"),
    );
    holds(
        "round square, turned",
        &on(square, "kind=directional x=-0.4 y=0.3 z=1"),
    );
    holds(
        "round title, point light",
        &on(title, "x=0.3 y=-0.2 z=-0.7"),
    );
    holds(
        "round title, hard shadow",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1 shadow=1"),
    );
    holds(
        "round title, soft shadow",
        &on(title, "kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4"),
    );
    holds(
        "round square, soft shadow",
        &on(square, "x=0.4 y=-0.3 z=-0.8 shadow=0.8 size=0.1"),
    );
}

#[test]
fn bevels_scaled_unevenly_hold_to_the_cpu() {
    let on = |set: &str, light: &str| {
        format!(
            "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.5\n  track V2 video\n    clip t {set}\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
        )
    };
    let square = |profile: &str, rest: &str| {
        format!("gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale_x=0.7 scale_y=0.25 depth=0.2 bevel=0.04 bevel_profile={profile} {rest}")
    };
    let title = |profile: &str| {
        format!("gen=title@1 at=0f dur=24f\n      set text=\"Cut\" size=0.3 three_d=true depth=0.1 bevel=0.03 bevel_profile={profile} scale_x=1.5 scale_y=0.7")
    };
    // A size at which a solid's depth comes out as all but a whole number of pixels: a step more or
    // less through it, between 32 and 64 bits, shows along an edge that is turned.
    holds(
        "beveled, with a depth of all but a whole number",
        &on(
            "gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.41833 depth=0.2 bevel=0.04 rotation=15",
            "kind=directional x=-0.4 y=0.3 z=1",
        ),
    );
    for profile in ["flat", "round"] {
        holds(
            &format!("{profile} square, uneven"),
            &on(&square(profile, ""), "kind=directional x=-0.4 y=0.3 z=1"),
        );
        holds(
            &format!("{profile} square, uneven and turned"),
            &on(
                &square(profile, "rotate_y=30 rotate_x=-20 rotation=15"),
                "kind=directional x=-0.4 y=0.3 z=1",
            ),
        );
        holds(
            &format!("{profile} title, uneven, soft shadow"),
            &on(
                &title(profile),
                "kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4",
            ),
        );
    }
}

#[test]
fn a_light_all_around_holds_to_the_cpu() {
    let around = "  track V9 video enabled=false\n    clip e gen=shape@1 at=0f dur=24f\n      set kind=ellipse width=0.6 height=0.9 fill=#ff8000\n";
    holds(
        "environment on flat clips",
        &format!(
            "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.5 rotate_y=35\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=environment image=V9 intensity=1.5\n{around}"
        ),
    );
    holds(
        "environment on a solid",
        &format!(
            "  track V1 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Lit\" size=0.4 three_d=true depth=0.1 rotate_y=30\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=environment image=V9\n{around}"
        ),
    );
}

#[test]
fn what_goes_into_a_clip_in_depth_holds_to_the_cpu() {
    holds(
        "effect and mask on a clip in depth",
        "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.7 rotate_y=30\n      fx color.basic exposure=-0.5\n      mask m ellipse=0.5,0.5,0.3,0.4 feather=8\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set x=0.2 z=-0.5 shadow=1\n",
    );
    holds(
        "a sequence drawn in depth, itself in depth",
        "  track V1 video\n    clip n seq=inner src=0f at=0f dur=24f\n      set three_d=true rotate_y=25 scale=0.8\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=0.3 z=1 shadow=1\nsequence inner rate=24 size=960x540 audio=48000\n  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#40c040 three_d=true scale=0.5 rotate_x=30\n  track V2 video\n    clip b gen=title@1 at=0f dur=24f\n      set text=\"In\" size=0.3 three_d=true z=-0.2\n",
    );
}

#[test]
fn a_clip_in_depth_in_hdr_holds_to_the_cpu() {
    for color in ["rec2100-pq", "rec2100-hlg"] {
        holds(
            color,
            &format!(
                "sequence s1 rate=24 size=960x540 audio=48000 color={color}\n  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip s {SQUARE}\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=0.2 z=1 shadow=1 size=5 intensity=2\n"
            ),
        );
    }
}

/// What a camera's blur has to hold: a scene, with a camera over it.
fn focused(camera: &str, clips: &str) -> String {
    format!(
        "{clips}  track V9 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set {camera}\n"
    )
}

#[test]
fn a_cameras_blur_holds_to_the_cpu() {
    let square = |set: &str| {
        format!(
            "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true {set}\n"
        )
    };
    holds(
        "focus flat",
        &focused("aperture=0.03", &square("z=0.5 scale=0.6")),
    );
    holds(
        "focus wide, one level",
        &focused("aperture=0.1", &square("z=2 scale=0.6")),
    );
    holds(
        "focus turned, many levels",
        &focused("aperture=0.05", &square("rotate_y=60 scale=0.6")),
    );
    holds(
        "focus racked to a plane",
        &focused("aperture=0.1 focus=0.5", &square("z=0.5 scale=0.6")),
    );
    holds(
        "focus off the frame's edge",
        &focused("aperture=0.05", &square("z=0.5 scale=2")),
    );
    holds(
        "focus, plane past the camera",
        &focused("aperture=0.05", &square("rotate_y=85 scale=2 z=-0.2")),
    );
    holds(
        "focus, camera moved",
        &focused(
            "aperture=0.05 focus=0.3 x=0.1 z=-0.2 pan=10 tilt=-5 roll=8",
            &square("rotate_x=25 scale=0.7"),
        ),
    );
    holds(
        "focus on a title, turned",
        &focused(
            "aperture=0.08",
            "  track V1 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Far\" size=0.3 three_d=true rotate_y=50\n",
        ),
    );
}

#[test]
fn a_cameras_blur_through_more_holds_to_the_cpu() {
    holds(
        "focus, two apart",
        &focused(
            "aperture=0.05",
            "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true z=0.6\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.3 z=-0.2\n",
        ),
    );
    holds(
        "focus, two crossing",
        &focused(
            "aperture=0.05",
            "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.6 z=0.1 rotate_y=45\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.6 rotate_y=-30\n",
        ),
    );
    holds(
        "focus on a deep title",
        &focused(
            "aperture=0.05",
            "  track V1 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Deep\" size=0.3 three_d=true depth=0.1 rotate_y=30 z=0.3\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 y=0.3 z=1\n",
        ),
    );
    holds(
        "focus, beveled, soft shadow",
        &focused(
            "aperture=0.04",
            "  track V1 video\n    clip w gen=solid@1 at=0f dur=24f\n      set color=#808080 three_d=true z=0.5\n  track V2 video\n    clip t gen=title@1 at=0f dur=24f\n      set text=\"Cut\" size=0.3 three_d=true depth=0.1 bevel=0.03\n  track V3 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 y=0.3 z=1 shadow=1 size=4\n",
        ),
    );
    holds(
        "focus, half there, blended",
        &focused(
            "aperture=0.06",
            "  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#40c040\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#ff8000 three_d=true scale=0.5 z=0.8 opacity=0.5 blend=screen\n",
        ),
    );
    holds(
        "focus in hdr",
        "sequence s1 rate=24 size=960x540 audio=48000 color=rec2100-pq\n  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.5 z=0.5 rotate_y=30\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional z=1 intensity=2\n  track V9 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.05\n",
    );
    holds(
        "focus in a sequence in depth",
        "  track V1 video\n    clip n seq=inner src=0f at=0f dur=24f\n      set three_d=true rotate_y=25 scale=0.8\nsequence inner rate=24 size=960x540 audio=48000\n  track V1 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#40c040 three_d=true scale=0.5 z=0.5\n  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.05\n",
    );
}
