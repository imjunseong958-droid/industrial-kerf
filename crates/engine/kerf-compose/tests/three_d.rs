//! Clips in depth: where they land, what is in front, and how they are lit.

use kerf_compose::{Accel, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

fn frame(body: &str, at: i64) -> Frame {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Auto,
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

/// A white square a quarter of the frame across, with what `set` says.
fn square(set: &str) -> String {
    format!("  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.25 {set}\n")
}

/// How many columns across the frame's middle row are lit.
fn width_of(frame: &Frame) -> u32 {
    (0..frame.width)
        .filter(|x| pixel(frame, *x, frame.height / 2)[0] > 128)
        .count() as u32
}

fn height_of(frame: &Frame) -> u32 {
    (0..frame.height)
        .filter(|y| pixel(frame, frame.width / 2, *y)[0] > 128)
        .count() as u32
}

#[test]
fn at_rest_a_clip_in_depth_looks_as_it_does_in_2d() {
    for set in [
        "",
        "x=0.2 y=-0.1",
        "rotation=30",
        "x=0.1 scale_x=2 rotation=-15 anchor_x=0.2",
    ] {
        let flat = frame(&square(set), 3);
        let deep = frame(&square(&format!("{set} three_d=true")), 3);
        let off: u64 = flat
            .rgba
            .iter()
            .zip(&deep.rgba)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        // Edges may differ as they are filtered; nothing else.
        let mean = off as f64 / flat.rgba.len() as f64;
        assert!(mean < 0.5, "{set}: {mean}");
    }
}

#[test]
fn a_clip_further_away_looks_smaller_by_perspective() {
    let plain = width_of(&frame(&square("three_d=true"), 3));
    // The default camera stands 180 / 2 / tan(20°) = 247 pixels away, so one frame height further on
    // is 247 / (247 + 180) of the size.
    let far = width_of(&frame(&square("three_d=true z=1"), 3));
    let wanted = f64::from(plain) * 247.3 / (247.3 + 180.0);
    assert!(
        (f64::from(far) - wanted).abs() < 2.0,
        "{plain} to {far}, not {wanted}"
    );
    let near = width_of(&frame(&square("three_d=true z=-0.5"), 3));
    let wanted = f64::from(plain) * 247.3 / (247.3 - 90.0);
    assert!(
        (f64::from(near) - wanted).abs() < 2.0,
        "{plain} to {near}, not {wanted}"
    );
}

#[test]
fn behind_the_camera_there_is_nothing() {
    let gone = frame(&square("three_d=true z=-5"), 3);
    assert_eq!(width_of(&gone), 0);
}

#[test]
fn turning_a_clip_narrows_it_and_tipping_it_shortens_it() {
    let at_rest = frame(&square("three_d=true"), 3);
    let (w, h) = (width_of(&at_rest), height_of(&at_rest));
    let turned = frame(&square("three_d=true rotate_y=60"), 3);
    assert!(
        (f64::from(width_of(&turned)) / f64::from(w) - 0.5).abs() < 0.03,
        "{w} to {}",
        width_of(&turned)
    );
    assert!(height_of(&turned).abs_diff(h) <= 1);
    let tipped = frame(&square("three_d=true rotate_x=60"), 3);
    assert!(
        (f64::from(height_of(&tipped)) / f64::from(h) - 0.5).abs() < 0.03,
        "{h} to {}",
        height_of(&tipped)
    );
    // Perspective: of a clip turned about its middle, the near side is taller than the far side.
    let edge = |x: u32| (0..180).filter(|y| pixel(&turned, x, *y)[0] > 128).count();
    let (mut left, mut right) = (320u32, 0u32);
    for x in 0..320 {
        if edge(x) > 0 {
            left = left.min(x);
            right = right.max(x);
        }
    }
    let (a, b) = (edge(left + 1), edge(right - 1));
    assert_ne!(a, b, "both sides {a} tall, so no perspective");
}

#[test]
fn what_is_nearer_covers_what_is_further_whichever_track_it_is_on() {
    let tracks = |top: &str, bottom: &str| {
        format!("  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ff0000 scale=0.5 three_d=true z={bottom}\n  track V2 video\n    clip c2 gen=solid@1 at=0f dur=24f\n      set color=#0000ff scale=0.5 three_d=true z={top}\n")
    };
    // The higher track is further away, so the lower one shows.
    assert_eq!(pixel(&frame(&tracks("1", "0"), 3), 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&frame(&tracks("0", "1"), 3), 160, 90), [0, 0, 255]);
    // A flat clip between breaks the run: what is above it stays above.
    let broken = "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ff0000 scale=0.5 three_d=true z=0\n  track V2 video\n    clip f gen=solid@1 at=0f dur=24f\n      set color=#00ff00 scale=0.25\n  track V3 video\n    clip c2 gen=solid@1 at=0f dur=24f\n      set color=#0000ff scale=0.25 three_d=true z=5\n";
    assert_eq!(pixel(&frame(broken, 3), 160, 90), [0, 0, 255]);
}

#[test]
fn moving_the_camera_shifts_near_things_more_than_far_things() {
    let scene = |camera: &str| {
        format!(
            "  track V1 video\n    clip far gen=solid@1 at=0f dur=24f\n      set color=#ff0000 scale=0.1 three_d=true z=2\n  track V2 video\n    clip near gen=solid@1 at=0f dur=24f\n      set color=#00ff00 scale=0.1 three_d=true y=0.3\n  track V3 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set {camera}\n"
        )
    };
    let middle = |frame: &Frame, y: u32, channel: usize| {
        let lit: Vec<u32> = (0..frame.width)
            .filter(|x| pixel(frame, *x, y)[channel] > 128)
            .collect();
        f64::from(lit.iter().sum::<u32>()) / lit.len().max(1) as f64
    };
    let rest = frame(&scene("x=0"), 3);
    let moved = frame(&scene("x=0.1"), 3);
    let far_shift = middle(&rest, 90, 0) - middle(&moved, 90, 0);
    let near_shift = middle(&rest, 144, 1) - middle(&moved, 144, 1);
    assert!(
        (near_shift - 32.0).abs() < 1.5,
        "the plane z=0 moves as the camera does: {near_shift}"
    );
    assert!(
        far_shift > 5.0 && far_shift < near_shift * 0.6,
        "{far_shift} {near_shift}"
    );
    // Keyframes move it, and zooming in by field of view makes things larger.
    let zoomed = frame(&scene("fov=20"), 3);
    assert!(width_of(&zoomed) > width_of(&rest));
    let animated = frame(&scene("x={0f:0 24f:0.2}"), 12);
    let at_half = middle(&rest, 144, 1) - middle(&animated, 144, 1);
    assert!((at_half - 32.0).abs() < 1.5, "{at_half}");
}

#[test]
fn lights_fall_off_by_which_way_a_clip_faces_and_only_a_light_darkens() {
    let lit = |light: &str, turn: &str| {
        frame(
            &format!(
                "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true {turn}\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set {light}\n"
            ),
            3,
        )
    };
    let middle = |frame: &Frame| pixel(frame, 160, 90)[0];
    // A directional light straight on is all of it; turned 60°, half in light, which encodes brighter.
    let on = lit("kind=directional x=0 y=0 z=1", "");
    assert_eq!(middle(&on), 255);
    let turned = lit("kind=directional x=0 y=0 z=1", "rotate_y=60");
    let half = pixel(&turned, 160, 90)[0];
    assert!((180..=195).contains(&half), "{half}");
    // Ambient is the same either way.
    let ambient = lit("kind=ambient intensity=0.5", "rotate_y=60");
    assert!(
        (180..=195).contains(&middle(&ambient)),
        "{}",
        middle(&ambient)
    );
    // A point light near one side lights that side more.
    let side = lit("kind=point x=-0.45 y=0 z=-0.1", "");
    assert!(pixel(&side, 20, 90)[0] > pixel(&side, 300, 90)[0] + 40);
    // Colored light tints.
    let red = lit("kind=ambient color=#ff0000", "");
    assert_eq!(middle(&red), 255);
    assert_eq!(pixel(&red, 160, 90)[1], 0);
    // With no light showing, clips are as they are, and a clip not in depth takes no light.
    let flat = frame(
        "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=ambient intensity=0\n",
        3,
    );
    assert_eq!(middle(&flat), 255);
}

#[test]
fn a_frame_in_depth_goes_to_the_cpu_and_its_key_knows_the_camera() {
    let project = |camera: &str| {
        parse_document(
            &format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{}  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set {camera}\n", square("three_d=true")),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
    };
    let compositor = |camera: &str| {
        Compositor::new(
            &project(camera),
            Catalog::builtin(),
            &ComposeOptions {
                accel: Accel::Always,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let at = Rate::FPS_24.time_of(3);
    assert!(!compositor("x=0").draws_on_gpu(at, 1.0));
    assert_ne!(
        compositor("x=0").frame_key(at, 1.0),
        compositor("x=0.1").frame_key(at, 1.0)
    );
}
