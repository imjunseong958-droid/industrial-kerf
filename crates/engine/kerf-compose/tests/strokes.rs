//! Clone, heal, and warp as a document says them and a frame draws them.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

const WIDE: u32 = 320;
const TALL: u32 = 180;

fn frame(body: &str, at: i64, accel: Accel) -> Frame {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={WIDE}x{TALL} audio=48000\n  track V1 video\n{body}"
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
    .frame(Rate::FPS_24.time_of(at), 1.0)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// A gray picture with a white square left of the middle, in a nested sequence so that an effect's
/// picture holds it, and `fx`.
fn scene(fx: &str) -> String {
    format!(
        "    clip c1 seq=picture src=0f at=0f dur=48f\n{fx}sequence picture rate=24 size={WIDE}x{TALL} audio=48000\n  track V1 video\n    clip b gen=solid@1 at=0f dur=48f\n      set color=#606060\n  track V2 video\n    clip w gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.1 x=-0.25\n"
    )
}

const WHITE: u32 = 255;

#[test]
fn a_clone_paints_the_square_elsewhere() {
    // The square is at 0.25 across. Paint at 0.75 from half a picture to the left.
    let body =
        scene("      fx image.clone@1 points=\"0.75,0.5\" radius=0.05 hardness=1 from_x=-0.5\n");
    let found = frame(&body, 3, Accel::Never);
    assert_eq!(pixel(&found, WIDE * 3 / 4, TALL / 2)[0] as u32, WHITE);
    assert_eq!(
        pixel(&found, WIDE / 4, TALL / 2)[0] as u32,
        WHITE,
        "still there"
    );
    assert_eq!(pixel(&found, WIDE * 3 / 4, TALL / 2 + 30)[0], 0x60);
}

#[test]
fn a_heal_with_no_source_named_takes_the_square_out() {
    let body = scene("      fx image.heal@1 points=\"0.25,0.5\" radius=0.1 hardness=0.8\n");
    let found = frame(&body, 3, Accel::Never);
    let at = pixel(&found, WIDE / 4, TALL / 2)[0];
    assert!(at.abs_diff(0x60) < 8, "{at}");
}

#[test]
fn strokes_paint_in_order() {
    // The second reads what the first painted.
    let body = scene(
        "      fx image.clone@1 points=\"0.5,0.5\" radius=0.05 hardness=1 from_x=-0.25\n      fx image.clone@1 points=\"0.75,0.5\" radius=0.05 hardness=1 from_x=-0.25\n",
    );
    let found = frame(&body, 3, Accel::Never);
    assert_eq!(pixel(&found, WIDE * 3 / 4, TALL / 2)[0] as u32, WHITE);
}

#[test]
fn a_warp_drags_the_square_and_animates() {
    let body =
        scene("      fx image.warp@1 moves=\"0.25,0.5>0.75,0.5\" radius=0.5 amount={0f:0 24f:1}\n");
    let start = frame(&body, 0, Accel::Never);
    assert_eq!(pixel(&start, WIDE / 4, TALL / 2)[0] as u32, WHITE);
    assert_eq!(pixel(&start, WIDE * 3 / 4, TALL / 2)[0], 0x60);
    let end = frame(&body, 24, Accel::Never);
    assert_eq!(pixel(&end, WIDE * 3 / 4, TALL / 2)[0] as u32, WHITE);
    assert_eq!(pixel(&end, WIDE / 4, TALL / 2)[0], 0x60);
}

#[test]
fn the_frame_stays_the_same_on_the_gpu_path() {
    for fx in [
        "      fx image.heal@1 points=\"0.25,0.5\" radius=0.1\n",
        "      fx image.warp@1 moves=\"0.25,0.5>0.6,0.4\" radius=0.4\n",
    ] {
        let body = scene(fx);
        let cpu = frame(&body, 3, Accel::Never);
        let gpu = frame(&body, 3, Accel::Always);
        let off = cpu
            .rgba
            .iter()
            .zip(&gpu.rgba)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(off <= 2, "{off} in {fx}");
    }
}

#[test]
fn a_stroke_that_does_not_read_is_refused_with_a_reason() {
    for (line, word) in [
        ("fx image.heal@1 points=\"nowhere\"", "points"),
        ("fx image.warp@1 moves=\"0.5,0.5\"", "moves"),
        ("fx image.clone@1 points=\"0.5,99\"", "points"),
    ] {
        let text = format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      {line}\n"
        );
        let errors = match parse_document(&text, Catalog::builtin()) {
            Ok(_) => panic!("took `{line}`"),
            Err(errors) => errors,
        };
        assert!(format!("{errors:?}").contains(word), "{errors:?}");
    }
}

#[test]
fn they_round_trip_through_the_document() {
    let text = "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      fx image.heal@1 from_frame=-3 points=\"0.4,0.5 0.5,0.5\" radius=0.02\n      fx image.warp@1 amount={0f:0 12f:1} moves=\"0.5,0.5>0.6,0.5>0.6,0.6 0.1,0.1>0.2,0.2\"\n";
    let once =
        parse_document(text, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = kerf_lang::print_document(
        &once.project,
        Catalog::builtin(),
        kerf_lang::PrintOptions::default(),
    );
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
}
