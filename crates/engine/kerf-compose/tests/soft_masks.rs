//! Masks whose edges are softer in some places than others, through a document and a frame.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

const SQUARE: &str = "0.25,0.25 0.75,0.25 0.75,0.75 0.25,0.75";

fn compositor(mask: &str, accel: Accel) -> Compositor {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n      mask k1 {mask}\n"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

fn frame(mask: &str, at: i64) -> Frame {
    compositor(mask, Accel::Never).frame(Rate::FPS_24.time_of(at), 1.0)
}

/// How much of a pixel is the white clip, over black: in light.
fn covered(frame: &Frame, x: u32, y: u32) -> f64 {
    let at = ((y * frame.width + x) * 4) as usize;
    let encoded = f64::from(frame.rgba[at]) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// How wide the fall-off across row `y` is around column `x`, from where it starts to where it ends.
fn width_across(frame: &Frame, y: u32, around: u32) -> u32 {
    let start = (0..=around)
        .rev()
        .find(|x| covered(frame, *x, y) > 0.995)
        .unwrap_or(0);
    let end = (around..frame.width)
        .find(|x| covered(frame, *x, y) < 0.005)
        .unwrap_or(frame.width - 1);
    end - start
}

#[test]
fn a_mask_that_says_none_draws_as_it_did() {
    let plain = frame(&format!("points=\"{SQUARE}\""), 3);
    for said in [
        "points=\"0.25,0.25,0 0.75,0.25,0 0.75,0.75,0 0.25,0.75,0\"",
        // What says nothing says 0.
        "points=\"0.25,0.25,0 0.75,0.25 0.75,0.75 0.25,0.75\"",
    ] {
        assert_eq!(plain.rgba, frame(said, 3).rgba, "{said}");
    }
    let plain = frame(
        &format!("points=\"{SQUARE}\" feather=6 expansion=-3 inverted=true"),
        3,
    );
    let zero = frame(
        "points=\"0.25,0.25,0 0.75,0.25,0 0.75,0.75,0 0.25,0.75,0\" feather=6 expansion=-3 inverted=true",
        3,
    );
    assert_eq!(plain.rgba, zero.rgba);
}

#[test]
fn softness_moves_with_the_outline() {
    let mask = "points={0f:0.25,0.25,0;0.75,0.25,0;0.75,0.75,0;0.25,0.75,0 24f:0.25,0.25,32;0.75,0.25,32;0.75,0.75,32;0.25,0.75,32}";
    let widths: Vec<u32> = [0, 12, 23]
        .map(|at| width_across(&frame(mask, at), 90, 240))
        .into();
    assert!(widths[0] <= 2, "{widths:?}");
    assert!((14..=18).contains(&widths[1]), "{widths:?}");
    assert!((29..=33).contains(&widths[2]), "{widths:?}");
    // Between keys it is drawn as if said then.
    let between = frame(mask, 12);
    let said = frame(
        "points=\"0.25,0.25,16 0.75,0.25,16 0.75,0.75,16 0.25,0.75,16\"",
        3,
    );
    assert_eq!(between.rgba, said.rgba);
}

#[test]
fn softness_is_in_the_frames_key() {
    let at = Rate::FPS_24.time_of(3);
    let key = |mask: &str| compositor(mask, Accel::Never).frame_key(at, 1.0);
    assert_ne!(
        key("points=\"0.25,0.25,1 0.75,0.25 0.75,0.75 0.25,0.75\""),
        key("points=\"0.25,0.25,2 0.75,0.25 0.75,0.75 0.25,0.75\"")
    );
}

#[test]
fn soft_edges_draw_alike_on_the_cpu_and_the_gpu() {
    let mask = "points=\"0.25,0.25,0 0.75,0.25,24 0.75,0.75,8 0.25,0.75,2\" feather=2";
    let gpu = compositor(mask, Accel::Always);
    let at = Rate::FPS_24.time_of(3);
    // Linux CI has no GPU.
    if !gpu.draws_on_gpu(at, 1.0) {
        eprintln!("no GPU to compare with");
        return;
    }
    let (on_gpu, on_cpu) = (gpu.frame(at, 1.0), frame(mask, 3));
    let worst = on_gpu
        .rgba
        .iter()
        .zip(&on_cpu.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 3, "{worst}");
}
