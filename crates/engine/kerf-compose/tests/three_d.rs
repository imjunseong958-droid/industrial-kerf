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
fn a_frame_keys_by_the_camera() {
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
    assert_ne!(
        compositor("x=0").frame_key(at, 1.0),
        compositor("x=0.1").frame_key(at, 1.0)
    );
}

/// Two clips in depth over nothing: a red one, then a blue one.
fn crossing(red: &str, blue: &str) -> String {
    format!(
        "  track V1 video\n    clip r gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true {red}\n  track V2 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true {blue}\n"
    )
}

/// Where along a line of pixels the colour goes from mostly one to mostly the other.
fn flips(colors: impl Iterator<Item = [u8; 3]>) -> Vec<u32> {
    let mut last = None;
    let mut found = Vec::new();
    for (place, [r, _, b]) in colors.enumerate() {
        if r < 128 && b < 128 {
            continue;
        }
        let red = r > b;
        if last.is_some_and(|last| last != red) {
            found.push(place as u32);
        }
        last = Some(red);
    }
    found
}

fn down_column(frame: &Frame, x: u32) -> Vec<u32> {
    flips((0..frame.height).map(|y| pixel(frame, x, y)))
}

#[test]
fn clips_turned_about_x_are_cut_along_a_row() {
    // Red is 18 pixels near and turned 45 degrees, its foot going back: it comes to z = 0 at
    // 18 / tan 45 = 18 pixels below its middle, so at y = 90 + 18, with red above that and blue below.
    let frame = frame(&crossing("scale=0.5 z=-0.1 rotate_x=45", "scale=0.5"), 0);
    assert_eq!(pixel(&frame, 160, 100), [255, 0, 0]);
    assert_eq!(pixel(&frame, 160, 116), [0, 0, 255]);
    let found = down_column(&frame, 160);
    assert!(found.iter().any(|y| y.abs_diff(108) <= 1), "{found:?}");
}

#[test]
fn three_clips_cross_in_the_order_of_each_pixel() {
    let frame = frame(
        &format!(
            "{}  track V3 video\n    clip g gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.5 z=0.3\n",
            crossing("scale=0.5 z=0.1 rotate_y=45", "scale=0.5"),
        ),
        0,
    );
    // Green is behind both of them wherever they are, whatever its track.
    assert_eq!(pixel(&frame, 170, 90), [0, 0, 255]);
    assert_eq!(pixel(&frame, 186, 90), [255, 0, 0]);
}

/// A wall filling the frame at z = 0, a small square in front of it, and a light. As `CASTER` has it,
/// the square is 32 by 18 pixels, from x = -80 to -48 about the middle of the frame and y = -9 to 9,
/// and 45 pixels nearer.
fn shadowed(square: &str, light: &str) -> String {
    format!(
        "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip caster gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.1 {square}\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set {light}\n"
    )
}

const CASTER: &str = "x=-0.2 z=-0.25";

/// What a channel of a frame is in linear light, which is what lights add up in.
fn linear(value: u8) -> f64 {
    let encoded = f64::from(value) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

fn near(found: f64, wanted: f64) {
    assert!((found - wanted).abs() < 0.02, "{found} against {wanted}");
}

/// The light goes 160 across for every 180 back, so what is 45 pixels in front of the wall falls 40
/// to the right of where it is: from x = -40 to -8, or columns 120 to 152, and rows 81 to 99.
const SLANTED: &str = "kind=directional x=0.5 z=1 shadow=1";

#[test]
fn a_shadow_goes_when_what_casts_it_moves_away() {
    // Moved 0.1 of the frame's width to the right, so is its shadow, by 32 pixels, to columns 152
    // to 184. Where it was is lit now, past the square itself, which is at 101 to 140 from here.
    let moved = frame(&shadowed("x=-0.1 z=-0.25", SLANTED), 0);
    assert!(pixel(&moved, 146, 90)[0] > 120);
    assert!(pixel(&moved, 168, 90)[0] < 8);
    // Keyed right off the frame, and its shadow with it.
    let gone = shadowed("x={0f:-0.2 23f:3} z=-0.25", SLANTED);
    assert!(pixel(&frame(&gone, 0), 136, 90)[0] < 8);
    let last = frame(&gone, 23);
    assert!((0..320).all(|x| pixel(&last, x, 90)[0] > 120));
    // Past the light, it is in nobody's way. The light is 18 pixels from the wall, above x = -64: with
    // the square 45 from the wall it is lit, and with the square 9 from the wall, halfway to the
    // light, it is twice the size on the wall, from -96 to -32, and covers column 120 (x = -40).
    let light = "kind=point x=-0.2 y=0 z=-0.1 shadow=1";
    let past = frame(&shadowed(CASTER, light), 0);
    let lit = linear(pixel(&past, 120, 90)[0]);
    // 18 away over a distance of 30, so 0.6 of it.
    near(lit, 0.6);
    let between = frame(&shadowed("x=-0.2 z=-0.05", light), 0);
    assert!(
        pixel(&between, 120, 90)[0] < 8,
        "{:?}",
        pixel(&between, 120, 90)
    );
}

#[test]
fn only_what_is_opaque_casts() {
    // Half opaque, half a shadow.
    let lit = linear(pixel(&frame(&shadowed(CASTER, SLANTED), 0), 200, 90)[0]);
    let half = frame(&shadowed("x=-0.2 z=-0.25 opacity=0.5", SLANTED), 0);
    near(linear(pixel(&half, 136, 90)[0]), lit / 2.0);
    // A picture of an ellipse casts an ellipse. It is 160 by 90 pixels about the frame's middle, and
    // the light goes 640 across for every 180 back, so its shadow's middle is at 160 to the right.
    let masked = frame(
        "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip caster gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.5 z=-0.25\n      mask k1 ellipse=0.5,0.5,0.5,0.5\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional z=1 x=2 shadow=1\n",
        0,
    );
    let at = |x: i32, y: i32| pixel(&masked, (160 + x) as u32, (90 + y) as u32);
    // Inside the ellipse, 60 to the left of its middle.
    assert!(at(100, 0)[0] < 8, "{:?}", at(100, 0));
    // In the corner of its box, outside the ellipse.
    assert!(at(85, 40)[0] > 40, "{:?}", at(85, 40));
}

/// A wall at z = 0, and on it, nearer by `z` of the frame's height, a clip 160 by 90 pixels whose right
/// edge is at x = -60 about the frame's middle, or column 100, with a light.
fn edge(z: f64, light: &str) -> String {
    format!(
        "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip caster gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.5 x=-0.4375 z=-{z}\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set {light}\n"
    )
}

/// How much of the light gets to each column of the frame's middle row, by what that row is with its
/// shadow and with none.
fn light_getting_there(z: f64, light: &str) -> Vec<f64> {
    let with = frame(&edge(z, &format!("{light} shadow=1")), 0);
    let without = frame(&edge(z, light), 0);
    (0..with.width)
        .map(|x| linear(pixel(&with, x, 90)[0]) / linear(pixel(&without, x, 90)[0]).max(1e-6))
        .collect()
}

/// From the last column where a caster's edge takes all of the light to the first where it takes none,
/// looking out from `around`, so as not to take in the wall behind the clip as it is seen.
fn penumbra(getting: &[f64], around: usize) -> (usize, usize) {
    let dark = (0..=around).rev().find(|x| getting[*x] < 0.01);
    let lit = (around..getting.len()).find(|x| getting[*x] > 0.99);
    (dark.expect("goes dark"), lit.expect("goes light"))
}

#[test]
fn a_point_lights_penumbra_is_as_wide_as_its_disc_says() {
    // The light is 135 pixels up, right over the edge, and 18 across: what is d from the wall has a
    // penumbra 18 d / (135 - d) wide about right below it.
    let light = "x=-0.1875 z=-0.75 size=0.1";
    let width = |z: f64| {
        let getting = light_getting_there(z, light);
        let (dark, lit) = penumbra(&getting, 100);
        let middle = (getting[99] + getting[100]) / 2.0;
        assert!((middle - 0.5).abs() < 0.05, "{z}: {middle}");
        (lit - dark) as f64
    };
    for (z, wanted) in [(0.25, 9.0), (0.5, 36.0)] {
        let found = width(z);
        assert!((found - wanted).abs() < 3.0, "{z}: {found}, not {wanted}");
    }
    // The further from the wall it is, the wider, and the nearer the less.
    let (near, middle, far) = (width(0.1), width(0.3), width(0.5));
    assert!(near < middle && middle < far, "{near} {middle} {far}");
}

#[test]
fn a_clip_with_holes_casts_a_soft_shadow_with_them() {
    // Lit from far to one side, so that its shadow falls clear of it: a clip whose left half a mask
    // takes away casts as a solid clip as wide as what is left, read through its alpha.
    let scene = |caster: &str| {
        format!(
            "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip caster gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true z=-0.25 {caster}\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=0.7 z=1 size=20 shadow=1\n"
        )
    };
    let holed = frame(
        &scene("scale=0.5 x=-0.4375\n      mask m rect=0.5,0,1,1"),
        0,
    );
    let solid = frame(&scene("scale_x=0.25 scale_y=0.5 x=-0.3125"), 0);
    for x in 90..320 {
        let (a, b) = (
            linear(pixel(&holed, x, 90)[0]),
            linear(pixel(&solid, x, 90)[0]),
        );
        assert!((a - b).abs() < 0.03, "{x}: {a} {b}");
    }
    // That is a shadow, and only that.
    assert!(linear(pixel(&holed, 120, 90)[0]) < 0.01);
    assert!(linear(pixel(&holed, 200, 90)[0]) > 0.5);
}

#[test]
fn a_light_with_no_size_draws_as_it_did() {
    for (with, without) in [
        (
            "kind=directional x=0.5 z=1 shadow=1 size=0",
            "kind=directional x=0.5 z=1 shadow=1",
        ),
        (
            "x=-0.2 y=0 z=-0.5 shadow=1 intensity=2 size=0",
            "x=-0.2 y=0 z=-0.5 shadow=1 intensity=2",
        ),
        // With nothing to block, it does not matter how wide it is.
        (
            "kind=directional x=0.5 z=1 size=20",
            "kind=directional x=0.5 z=1",
        ),
        ("x=-0.2 z=-0.5 size=0.2", "x=-0.2 z=-0.5"),
        (
            "kind=ambient intensity=0.5 size=20",
            "kind=ambient intensity=0.5",
        ),
    ] {
        assert_eq!(
            frame(&shadowed(CASTER, with), 3).rgba,
            frame(&shadowed(CASTER, without), 3).rgba,
            "{with}"
        );
    }
}

#[test]
fn a_lights_size_animates() {
    let scene = edge(0.25, "kind=directional z=1 size={0f:0 23f:20} shadow=1");
    let hard = frame(&edge(0.25, "kind=directional z=1 shadow=1"), 0);
    assert_eq!(frame(&scene, 0).rgba, hard.rgba);
    let wider = frame(&scene, 23);
    assert_eq!(
        wider.rgba,
        frame(&edge(0.25, "kind=directional z=1 size=20 shadow=1"), 0).rgba
    );
    assert_ne!(wider.rgba, hard.rgba);
}

#[test]
fn what_two_clips_take_of_a_light_between_them_is_all_of_it() {
    // Side by side at z = -45 with the line between them at x = -60: from x = -20 on the wall, the
    // rays go up past x = -77 to -48 there, so each clip takes about half of them and together nearly
    // all, where a clip at a time would leave a quarter. Neither hides that part of the wall.
    let scene = |left: bool, right: bool, shadow: f64| {
        let mut body = String::from(
            "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n",
        );
        if left {
            body += "  track V2 video\n    clip a gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.5 x=-0.4375 z=-0.25\n";
        }
        if right {
            body += "  track V3 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale_x=0.0625 scale_y=0.5 x=-0.15625 z=-0.25\n";
        }
        body += &format!(
            "  track V4 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=0.5 z=1 size=20 shadow={shadow}\n"
        );
        linear(pixel(&frame(&body, 0), 140, 90)[0])
    };
    let whole = scene(true, true, 0.0);
    assert!(whole > 0.5, "{whole}");
    let (left, right) = (
        scene(true, false, 1.0) / whole,
        scene(false, true, 1.0) / whole,
    );
    assert!(
        (left - 0.5).abs() < 0.05 && (right - 0.5).abs() < 0.05,
        "{left} {right}"
    );
    let both = scene(true, true, 1.0) / whole;
    assert!(both < 0.08, "{both}");
}

/// A square a quarter of the frame across, in depth, with a light.
fn deep(set: &str, light: &str) -> String {
    let light = if light.is_empty() {
        String::new()
    } else {
        format!("  track V2 video\n    clip sun gen=light@1 at=0f dur=24f\n      set {light}\n")
    };
    format!(
        "  track V1 video\n    clip solid gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.25 three_d=true {set}\n{light}"
    )
}

#[test]
fn a_clip_with_no_depth_draws_as_it_did() {
    for set in ["", "x=0.2 rotate_y=30"] {
        for light in ["", "kind=directional x=0.5 z=1"] {
            assert_eq!(
                frame(&deep(set, light), 3).rgba,
                frame(&deep(&format!("{set} depth=0"), light), 3).rgba,
                "{set} {light}"
            );
        }
    }
    // Not in depth, depth is nothing.
    let flat = "  track V1 video\n    clip c gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.25 depth=0.5\n";
    let plain = "  track V1 video\n    clip c gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.25\n";
    assert_eq!(frame(flat, 3).rgba, frame(plain, 3).rgba);
}

#[test]
fn a_solids_sides_take_the_light_by_the_way_they_face() {
    // Off to the left, its right side shows, which faces right.
    let set = "x=-0.25 depth=0.5";
    let front = 100;
    let side = 125;
    // A light straight on: all on its front and none on a side square to it.
    let on = frame(&deep(set, "kind=directional z=1"), 3);
    assert_eq!(pixel(&on, front, 90)[0], 255);
    assert!(pixel(&on, side, 90)[0] < 8, "{:?}", pixel(&on, side, 90));
    // Shining to the right, from the left, it lights neither what faces right nor what faces the viewer.
    let from_left = frame(&deep(set, "kind=directional x=1 z=0.001"), 3);
    assert!(
        pixel(&from_left, side, 90)[0] < 8,
        "{:?}",
        pixel(&from_left, side, 90)
    );
    assert!(
        pixel(&from_left, front, 90)[0] < 16,
        "{:?}",
        pixel(&from_left, front, 90)
    );
    // Shining to the left, from the right, it lights that side and hardly the front.
    let from_right = frame(&deep(set, "kind=directional x=-1 z=0.001"), 3);
    assert!(
        pixel(&from_right, side, 90)[0] > 200,
        "{:?}",
        pixel(&from_right, side, 90)
    );
    assert!(
        pixel(&from_right, front, 90)[0] < 16,
        "{:?}",
        pixel(&from_right, front, 90)
    );
    // A light behind a solid does not come through it, as one behind a flat clip does.
    let behind = frame(&deep(set, "kind=directional z=-1"), 3);
    assert!(
        pixel(&behind, front, 90)[0] < 8,
        "{:?}",
        pixel(&behind, front, 90)
    );
    let flat_behind = frame(&deep("x=-0.25", "kind=directional z=-1"), 3);
    assert_eq!(pixel(&flat_behind, front, 90)[0], 255);
    // Ambient light is the same on all of it.
    let ambient = frame(&deep(set, "kind=ambient intensity=0.5"), 3);
    assert_eq!(pixel(&ambient, front, 90), pixel(&ambient, side, 90));
}

#[test]
fn a_title_with_depth_is_lit_on_its_sides_and_keeps_its_holes() {
    let title = |depth: &str| {
        format!(
            "  track V1 video\n    clip words gen=title@1 at=0f dur=24f\n      set text=\"O\" size=0.8 color=#ffffff three_d=true {depth}\n  track V2 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.4 y=0.4 z=1\n"
        )
    };
    let flat = frame(&title(""), 3);
    let deep = frame(&title("depth=0.3"), 3);
    let lit = |frame: &Frame| {
        (0..frame.width * frame.height)
            .filter(|at| pixel(frame, at % frame.width, at / frame.width)[0] > 8)
            .count()
    };
    // More of it, where its sides show, and shades of them: the flat one has one level of light.
    assert!(
        lit(&deep) > lit(&flat) + 100,
        "{} {}",
        lit(&deep),
        lit(&flat)
    );
    let levels = |frame: &Frame| {
        let mut seen = std::collections::BTreeSet::new();
        for at in 0..frame.width * frame.height {
            seen.insert(pixel(frame, at % frame.width, at / frame.width)[0]);
        }
        seen.len()
    };
    assert!(levels(&deep) > levels(&flat) + 10);
    // Its middle is a hole, and the frame shows through it.
    assert_eq!(pixel(&deep, 160, 90)[0], 0, "{:?}", pixel(&deep, 160, 90));
    // Its alpha is whole where it shows and nothing where it does not.
    let at = |x: u32, y: u32| ((y * deep.width + x) * 4 + 3) as usize;
    assert_eq!(deep.rgba[at(160, 90)], 255);
}

#[test]
fn a_clips_depth_animates_and_is_in_the_frames_key() {
    let keyed = deep("x=-0.25 depth={0f:0 23f:0.5}", "");
    assert_eq!(frame(&keyed, 0).rgba, frame(&deep("x=-0.25", ""), 0).rgba);
    let end = frame(&keyed, 23);
    let edge = (0..320)
        .rev()
        .find(|x| pixel(&end, *x, 90)[0] > 128)
        .unwrap();
    assert!(edge.abs_diff(130) <= 1, "{edge}");
    let compositor = |set: &str| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{}",
                deep(set, "")
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default()).expect("composes")
    };
    let at = Rate::FPS_24.time_of(3);
    assert_ne!(
        compositor("depth=0.1").frame_key(at, 1.0),
        compositor("depth=0.2").frame_key(at, 1.0)
    );
}

/// A clip turned with `set`, lit by an environment on a hidden track, whose clip is all white on its top
/// half and black below.
fn in_a_sky(set: &str, light: &str) -> String {
    format!(
        "  track V1 video\n    clip c gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.5 {set}\n  track sky video enabled=false\n    clip s gen=solid@1 at=0f dur=24f\n      set color=#ffffff\n      mask top rect=0,0,1,0.5\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=environment image=sky {light}\n"
    )
}

#[test]
fn an_environment_light_comes_from_where_its_picture_says() {
    // Lit in linear light, and the middle of the frame shows the clip.
    let at_middle = |document: &str| linear(pixel(&frame(document, 3), 160, 90)[0]);
    // Facing the camera, it sees as much sky as ground.
    near(at_middle(&in_a_sky("", "")), 0.5);
    // Tipped back, it faces up more, and tipped forward, down more, each by its turn.
    let up = at_middle(&in_a_sky("rotate_x=60", ""));
    let down = at_middle(&in_a_sky("rotate_x=-60", ""));
    assert!(up > 0.75 || down > 0.75, "{up} {down}");
    assert!(up < 0.25 || down < 0.25, "{up} {down}");
    assert!((up + down - 1.0).abs() < 0.05, "{up} {down}");
    // Its color and intensity go on top, and it adds to other lights.
    near(at_middle(&in_a_sky("", "intensity=0.5")), 0.25);
    let tinted = frame(&in_a_sky("", "color=#ff0000"), 3);
    near(linear(pixel(&tinted, 160, 90)[0]), 0.5);
    assert_eq!(pixel(&tinted, 160, 90)[1], 0);
    // With nothing on the track at that moment, it gives nothing.
    let gone = in_a_sky("", "").replace(
        "clip s gen=solid@1 at=0f dur=24f",
        "clip s gen=solid@1 at=20f dur=4f",
    );
    assert_eq!(pixel(&frame(&gone, 3), 160, 90)[0], 0);
    assert!(pixel(&frame(&gone, 22), 160, 90)[0] > 100);
    // And with no image it is ambient light.
    let plain = in_a_sky("", "").replace("image=sky ", "");
    assert_eq!(pixel(&frame(&plain, 3), 160, 90)[0], 255);
}

#[test]
fn a_solid_in_an_environment_is_lit_on_each_face_by_what_it_faces() {
    // Well below the middle of the frame, the camera looks down onto its top, which faces the sky. Its
    // near top edge is 63 pixels below the middle, and its far one, 90 pixels behind, 46.
    let body = |y: &str| in_a_sky(&format!("depth=0.5 y={y}"), "");
    let at = |frame: &Frame, y: u32| linear(pixel(frame, 160, y)[0]);
    let low = frame(&body("0.6"), 3);
    assert!(at(&low, 145) > 0.9, "{}", at(&low, 145));
    near(at(&low, 170), 0.5);
    // Well above it, the camera looks up at its underside, which faces the dark.
    let high = frame(&body("-0.6"), 3);
    assert!(at(&high, 35) < 0.1, "{}", at(&high, 35));
    near(at(&high, 10), 0.5);
}

#[test]
fn a_frame_lit_by_another_track_is_not_kept_by_a_key_that_cannot_see_it() {
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{}",
            in_a_sky("", "")
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let compositor = Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default())
        .expect("composes");
    assert_eq!(compositor.frame_key(Rate::FPS_24.time_of(3), 1.0), None);
}

#[test]
fn a_shadow_falls_on_a_clip_that_is_cut() {
    // Crossing and in shadow at once: the wall is cut by a turned clip, and both still take shadows.
    let drawn = frame(
        &format!(
            "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true\n  track V2 video\n    clip turned gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.3 x=0.3 z=0.05 rotate_y=45\n  track V3 video\n    clip caster gen=solid@1 at=0f dur=24f\n      set color=#00ff00 three_d=true scale=0.1 x=-0.2 z=-0.25\n  track V4 video\n    clip sun gen=light@1 at=0f dur=24f\n      set {SLANTED}\n"
        ),
        0,
    );
    assert!(
        pixel(&drawn, 136, 90)[0] < 8,
        "{:?}",
        pixel(&drawn, 136, 90)
    );
    assert!(pixel(&drawn, 200, 150)[0] > 120);
}

/// A white square a quarter of the frame across at `z`, and a camera that focuses as `camera` says.
fn focused(z: f64, camera: &str) -> String {
    format!(
        "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.25 three_d=true z={z}\n  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set {camera}\n"
    )
}

/// How many pixels the left edge of what is in the middle of a row takes to come up: from where it
/// has 2% of what the middle has to where it has 98%, in linear light.
fn edge_width(frame: &Frame, y: u32) -> u32 {
    let row: Vec<f64> = (0..frame.width)
        .map(|x| linear(pixel(frame, x, y)[0]))
        .collect();
    let middle = row[frame.width as usize / 2];
    let first = row.iter().position(|value| *value > middle * 0.02).unwrap();
    let full = row.iter().position(|value| *value > middle * 0.98).unwrap();
    (full - first) as u32
}

#[test]
fn what_is_in_focus_goes_by_what_the_camera_says() {
    // Sharp at the z it names, soft at the z it does not, either way round.
    let at_half = "aperture=0.1 focus=0.5";
    assert!(edge_width(&frame(&focused(0.5, at_half), 3), 90) <= 1);
    assert!(edge_width(&frame(&focused(0.0, at_half), 3), 90) > 5);
    // It animates: a rack from one to the other.
    let rack = "aperture=0.1 focus={0f:0 23f:0.5}";
    assert!(edge_width(&frame(&focused(0.0, rack), 0), 90) <= 1);
    assert!(edge_width(&frame(&focused(0.0, rack), 23), 90) > 5);
    // So does how wide it goes.
    let opening = "aperture={0f:0 23f:0.2} focus=0.5";
    let (shut, open) = (
        edge_width(&frame(&focused(0.0, opening), 0), 90),
        edge_width(&frame(&focused(0.0, opening), 23), 90),
    );
    assert!(shut <= 1 && open > 8, "{shut} {open}");
    // Its focus goes with the camera's own place, as a clip's z does not: moving the camera along
    // leaves the same z in focus.
    let moved = "aperture=0.1 focus=0.5 z=-0.3";
    assert!(edge_width(&frame(&focused(0.5, moved), 3), 90) <= 1);
}

#[test]
fn a_turned_clip_goes_softer_along_its_length() {
    // Its right side goes back, its left comes forward, about a middle in focus.
    let frame = frame(
        "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=0.5 three_d=true rotate_y=-60\n  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.1\n",
        3,
    );
    // Top and bottom edges: how many rows each column takes to come up.
    let rise = |x: u32| {
        let column: Vec<f64> = (0..frame.height)
            .map(|y| linear(pixel(&frame, x, y)[0]))
            .collect();
        let most = column[frame.height as usize / 2];
        let first = column
            .iter()
            .position(|value| *value > most * 0.02)
            .unwrap();
        let full = column
            .iter()
            .position(|value| *value > most * 0.98)
            .unwrap();
        full - first
    };
    assert!(rise(160) <= 2, "{}", rise(160));
    assert!(rise(190) > rise(160), "{} {}", rise(190), rise(160));
    assert!(rise(130) > rise(160), "{} {}", rise(130), rise(160));
}

#[test]
fn a_blur_is_each_clips_own_over_what_is_behind_it() {
    // A soft clip in front lets what is behind it show through its edge; a sharp clip in front keeps
    // its edge against a soft one behind.
    let body = |near: f64, far: f64, focus: f64| {
        format!(
            "  track V1 video\n    clip far gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true z={far}\n  track V2 video\n    clip near gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.25 z={near}\n  track V3 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.1 focus={focus}\n"
        )
    };
    let sharp_near = frame(&body(0.0, 1.0, 0.0), 3);
    let row: Vec<[u8; 3]> = (0..320).map(|x| pixel(&sharp_near, x, 90)).collect();
    let mixed = row.iter().filter(|[r, _, b]| *r > 30 && *b > 30).count();
    assert!(mixed <= 2, "{mixed}");
    let soft_near = frame(&body(0.0, 1.0, 1.0), 3);
    let row: Vec<[u8; 3]> = (0..320).map(|x| pixel(&soft_near, x, 90)).collect();
    let mixed = row.iter().filter(|[r, _, b]| *r > 30 && *b > 30).count();
    assert!(mixed > 6, "{mixed}");
}

#[test]
fn clips_that_cross_are_cut_and_blurred() {
    let drawn = frame(
        &format!(
            "{}  track V3 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.05\n",
            crossing("scale=0.5 z=0.1 rotate_y=45", "scale=0.5")
        ),
        0,
    );
    // Well away from the line each side still shows; flat blue at z = 0 stays sharp.
    assert_eq!(pixel(&drawn, 150, 90), [0, 0, 255]);
    let right = pixel(&drawn, 195, 90);
    assert!(right[0] > 200, "{right:?}");
}

/// A square solid, looked at straight on, with a light.
fn beveled(set: &str, light: &str) -> String {
    deep(&format!("depth=0.4 {set}"), light)
}

#[test]
fn a_bevel_of_nothing_is_no_bevel_and_it_needs_a_depth() {
    let light = "kind=directional x=-0.4 y=0.3 z=1";
    assert_eq!(
        frame(&beveled("", light), 3).rgba,
        frame(&beveled("bevel=0", light), 3).rgba
    );
    // Without a depth there is nothing to cut.
    assert_eq!(
        frame(&deep("", light), 3).rgba,
        frame(&deep("bevel=0.1", light), 3).rgba
    );
    // Two bevels meet at half its depth, and no further.
    assert_eq!(
        frame(&beveled("bevel=0.2", light), 3).rgba,
        frame(&beveled("bevel=5", light), 3).rgba
    );
}

#[test]
fn a_title_keeps_its_holes_and_gains_shades_with_a_bevel() {
    let title = |set: &str| {
        format!(
            "  track V1 video\n    clip words gen=title@1 at=0f dur=24f\n      set text=\"O\" size=0.8 color=#ffffff three_d=true depth=0.2 {set}\n  track V2 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=-0.5 y=0.4 z=1\n"
        )
    };
    let (plain, cut) = (frame(&title(""), 3), frame(&title("bevel=0.03"), 3));
    assert_ne!(plain.rgba, cut.rgba);
    assert_eq!(pixel(&cut, 160, 90)[0], 0);
    let levels = |frame: &Frame| {
        let mut seen = std::collections::BTreeSet::new();
        for at in 0..frame.width * frame.height {
            seen.insert(pixel(frame, at % frame.width, at / frame.width)[0]);
        }
        seen.len()
    };
    assert!(
        levels(&cut) > levels(&plain),
        "{} {}",
        levels(&cut),
        levels(&plain)
    );
}

#[test]
fn a_bevels_shadow_is_cut_at_its_faces_only() {
    let scene = |set: &str, light: &str| {
        format!(
            "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true z=0.6\n  track V2 video\n    clip s gen=solid@1 at=0f dur=24f\n      set color=#000000 three_d=true scale=0.3 {set}\n  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n      set {light}\n"
        )
    };
    let dark = |frame: &Frame| {
        (0..frame.width * frame.height)
            .filter(|at| pixel(frame, at % frame.width, at / frame.width)[0] < 128)
            .count() as f64
    };
    // Sideways, it runs across the planes through the solid, and those at the faces are smaller: less of
    // the shadow falls, and yet not so much less as a solid with no depth at all.
    let sideways = "kind=directional x=-1 z=1 shadow=1";
    let (plain, cut) = (
        frame(&scene("depth=0.4", sideways), 3),
        frame(&scene("depth=0.4 bevel=0.2", sideways), 3),
    );
    let flat = frame(&scene("depth=0", sideways), 3);
    assert_ne!(plain.rgba, cut.rgba);
    assert!(dark(&cut) < dark(&plain), "{} {}", dark(&cut), dark(&plain));
    assert!(dark(&cut) > dark(&flat), "{} {}", dark(&cut), dark(&flat));
    // Soft, the same frame comes every time.
    let soft = "kind=directional x=-1 z=1 shadow=1 size=6";
    assert_eq!(
        frame(&scene("depth=0.4 bevel=0.2", soft), 3).rgba,
        frame(&scene("depth=0.4 bevel=0.2", soft), 7).rgba
    );
}

#[test]
fn a_round_bevel_turns_across_its_width() {
    let front = "kind=directional z=1";
    let round = frame(&beveled("bevel=0.1 bevel_profile=round", front), 3);
    let flat = frame(&beveled("bevel=0.1", front), 3);
    let row = |square: &Frame| {
        (0..320)
            .map(|x| linear(pixel(square, x, 90)[0]))
            .collect::<Vec<_>>()
    };
    let (curved, cut) = (row(&round), row(&flat));
    let lit: Vec<usize> = (0..320).filter(|x| curved[*x] > 0.02).collect();
    let (left, right) = (lit[0], lit[lit.len() - 1]);
    // Its face takes all of a light from straight in front, and it takes less toward the outline.
    near(curved[(left + right) / 2], 1.0);
    let band: Vec<f64> = (left..(left + right) / 2)
        .map(|x| curved[x])
        .take_while(|value| *value < 0.995)
        .collect();
    assert!(band.len() >= 10, "{band:?}");
    // The cosine of the way it faces, turning without a step, where a flat cut has one value throughout.
    for pair in band.windows(2) {
        assert!(pair[1] + 0.02 >= pair[0], "{band:?}");
        assert!(pair[1] - pair[0] < 0.35, "{band:?}");
    }
    assert!(band[1] < 0.5, "{band:?}");
    assert!(*band.last().expect("a band") > 0.9, "{band:?}");
    let spread = |values: &[f64]| {
        values.iter().copied().fold(f64::MIN, f64::max)
            - values.iter().copied().fold(f64::MAX, f64::min)
    };
    let cut_band: Vec<f64> = (left + 2..left + 10).map(|x| cut[x]).collect();
    assert!(spread(&cut_band) < 0.02, "{cut_band:?}");
    assert!(spread(&band) > 0.5, "{band:?}");
    // At a quarter of a circle, what lies a third of the way in faces as the circle says.
    let wanted = |share: f64| (1.0 - (1.0 - share) * (1.0 - share)).sqrt();
    let third = band[band.len() / 3];
    assert!(
        (third - wanted(1.0 / 3.0)).abs() < 0.12,
        "{third} of {}",
        wanted(1.0 / 3.0)
    );
    // A light from the left falls on its left side and turns away toward its face.
    let side = frame(
        &beveled("bevel=0.1 bevel_profile=round", "kind=directional x=1 z=0"),
        3,
    );
    let side = row(&side);
    assert!(side[left + 2] > 0.5, "{}", side[left + 2]);
    assert!(side[left + 20] < side[left + 2], "{}", side[left + 20]);
    assert!(side[right - 2] < 0.02, "{}", side[right - 2]);
}

#[test]
fn a_flat_profile_is_what_a_bevel_always_was_and_a_round_one_is_another() {
    let light = "kind=directional x=-0.4 y=0.3 z=1 shadow=1";
    let flat = frame(&beveled("bevel=0.1 bevel_profile=flat", light), 3);
    assert_eq!(flat.rgba, frame(&beveled("bevel=0.1", light), 3).rgba);
    assert_ne!(
        flat.rgba,
        frame(&beveled("bevel=0.1 bevel_profile=round", light), 3).rgba
    );
    // With no bevel, or no depth, it has nothing to shape.
    assert_eq!(
        frame(&beveled("bevel_profile=round", light), 3).rgba,
        frame(&beveled("", light), 3).rgba
    );
}

/// How many pixels a bevel takes from the outline to the face, across the middle and down it.
fn bevel_bands(set: &str) -> (usize, usize) {
    let document = format!(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=640x360 audio=48000\n  track V1 video\n    clip t gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true depth=0.3 bevel=0.05 {set}\n  track V2 video\n    clip l gen=light@1 at=0f dur=24f\n      set kind=directional z=1\n  track V3 video\n    clip k gen=camera@1 at=0f dur=24f\n      set fov=4 z=-12.9\n"
    );
    let project = parse_document(&document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let drawn = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            keep_frames: false,
            frame_folder: None,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .frame(Rate::FPS_24.time_of(3), 1.0);
    let (w, h) = (drawn.width, drawn.height);
    let band = |line: Vec<u8>| {
        let start = line.iter().position(|value| *value > 8).expect("it shows");
        line[start..]
            .iter()
            .position(|value| *value >= 250)
            .expect("it has a face")
    };
    (
        band((0..w).map(|x| pixel(&drawn, x, h / 2)[0]).collect()),
        band((0..h).map(|y| pixel(&drawn, w / 2, y)[0]).collect()),
    )
}

/// A bevel goes by the world, so a clip scaled unevenly has it as wide across as it has it down, 18
/// pixels here, through a long lens from far back, which takes nothing off for perspective. It used to go by the mean of
/// its two scales, which at 2 to 0.5 came to 30 pixels and 9.
#[test]
fn a_clip_scaled_unevenly_has_a_bevel_as_wide_on_each_side() {
    let (across, down) = bevel_bands("scale=0.4");
    for profile in ["flat", "round"] {
        for (wide, tall) in [(0.64, 0.32), (0.8, 0.2), (0.2, 0.6), (0.3, 0.9)] {
            let (x, y) = bevel_bands(&format!(
                "scale_x={wide} scale_y={tall} bevel_profile={profile}"
            ));
            assert!(
                x.abs_diff(y) <= 3,
                "{profile} at {wide} by {tall}: {x} across and {y} down"
            );
            assert!(
                (12..=20).contains(&x) && (12..=20).contains(&y),
                "{profile} at {wide} by {tall}: {x} across and {y} down"
            );
        }
    }
    assert!(across.abs_diff(down) <= 3, "{across} and {down}");
}
