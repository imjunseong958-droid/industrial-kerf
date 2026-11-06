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

fn across_row(frame: &Frame, y: u32) -> Vec<u32> {
    flips((0..frame.width).map(|x| pixel(frame, x, y)))
}

fn down_column(frame: &Frame, x: u32) -> Vec<u32> {
    flips((0..frame.height).map(|y| pixel(frame, x, y)))
}

#[test]
fn clips_turned_about_y_are_cut_where_they_meet() {
    // Red is 18 pixels back and turned 45 degrees, right side nearer; blue is flat at z = 0. Red's
    // plane comes to z = 0 at 18 / tan 45 = 18 pixels right of its middle, and z = 0 lands on the
    // frame as it does in 2D: at x = 160 + 18. To its left blue is in front, and to its right red.
    let frame = frame(&crossing("scale=0.5 z=0.1 rotate_y=45", "scale=0.5"), 0);
    assert_eq!(pixel(&frame, 170, 90), [0, 0, 255]);
    assert_eq!(pixel(&frame, 186, 90), [255, 0, 0]);
    let found = across_row(&frame, 90);
    assert!(found.iter().any(|x| x.abs_diff(178) <= 1), "{found:?}");
    // At each height, since the line of it is upright.
    for y in [60, 75, 105, 120] {
        let found = across_row(&frame, y);
        assert!(found.iter().any(|x| x.abs_diff(178) <= 1), "{y}: {found:?}");
    }
}

#[test]
fn the_clips_trade_sides_whichever_track_is_on_top() {
    // Either way round the same line, and the same colour each side of it.
    let frame = frame(
        "  track V1 video\n    clip b gen=solid@1 at=0f dur=24f\n      set color=#0000ff three_d=true scale=0.5\n  track V2 video\n    clip r gen=solid@1 at=0f dur=24f\n      set color=#ff0000 three_d=true scale=0.5 z=0.1 rotate_y=45\n",
        0,
    );
    assert_eq!(pixel(&frame, 170, 90), [0, 0, 255]);
    assert_eq!(pixel(&frame, 186, 90), [255, 0, 0]);
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
fn the_line_falls_where_two_turned_planes_meet() {
    // Neither flat: where the rays first find the two planes at one depth, by the plane equations.
    let eye = [0.0, 0.0, -90.0 / 20f64.to_radians().tan()];
    let focal = -eye[2];
    let plane = |(cx, z): (f64, f64), yaw: f64| {
        let (s, c) = yaw.to_radians().sin_cos();
        // A point of it, and the way it faces: turned about y by `yaw`.
        ([cx, 0.0, z], [s, 0.0, c])
    };
    let red = plane((0.0, 9.0), 35.0);
    let blue = plane((16.0, -9.0), -40.0);
    let depth = |(point, normal): ([f64; 3], [f64; 3]), x: f64, y: f64| {
        let ray = [x, y, focal];
        let toward: f64 = (0..3).map(|i| ray[i] * normal[i]).sum();
        let along: f64 = (0..3).map(|i| (point[i] - eye[i]) * normal[i]).sum::<f64>() / toward;
        along * focal
    };
    // Bisect along a row for where the two depths are equal.
    let (mut left, mut right) = (-40.0, 60.0);
    let gap = |x: f64| depth(red, x, 0.5) - depth(blue, x, 0.5);
    assert!(gap(left) * gap(right) < 0.0, "{} {}", gap(left), gap(right));
    for _ in 0..50 {
        let middle: f64 = (left + right) / 2.0;
        if gap(left) * gap(middle) <= 0.0 {
            right = middle;
        } else {
            left = middle;
        }
    }
    let wanted = 160.0 + left;
    let drawn = frame(
        &crossing(
            "scale=0.6 z=0.05 rotate_y=35",
            "scale=0.6 z=-0.05 x=0.05 rotate_y=-40",
        ),
        0,
    );
    let found = across_row(&drawn, 90);
    assert!(
        found.iter().any(|x| (f64::from(*x) - wanted).abs() <= 1.5),
        "{found:?} against {wanted}"
    );
}

#[test]
fn clips_that_do_not_cross_stack_as_they_did() {
    // Parallel, on one plane, or only their planes meeting off their ends: the nearer whole, in
    // front of the other whole, or the later track where they are as near.
    let parallel = frame(&crossing("scale=0.6 z=0.2", "scale=0.3 z=-0.2"), 0);
    assert_eq!(pixel(&parallel, 160, 90), [0, 0, 255]);
    let reversed = frame(&crossing("scale=0.3 z=-0.2", "scale=0.6 z=0.2"), 0);
    assert_eq!(pixel(&reversed, 160, 90), [255, 0, 0]);
    let same = frame(&crossing("scale=0.6", "scale=0.3"), 0);
    assert_eq!(pixel(&same, 160, 90), [0, 0, 255]);
}

#[test]
fn a_cut_shares_the_pixels_it_goes_through() {
    // Along the line each pixel it goes through is part of both, which keeps it from being jagged.
    // Here the line falls 18.45 pixels along, most of the way across the pixel from 178 to 179.
    let frame = frame(&crossing("scale=0.5 z=0.1025 rotate_y=45", "scale=0.5"), 0);
    let mixed: Vec<[u8; 3]> = (170..186)
        .map(|x| pixel(&frame, x, 90))
        .filter(|[r, _, b]| *r > 20 && *b > 20)
        .collect();
    assert_eq!(mixed.len(), 1, "{mixed:?}");
    // A band a single pixel wide, not a blur, and not one colour only.
    assert!(mixed[0][0] > 60 && mixed[0][2] > 60, "{mixed:?}");
}

#[test]
fn opacity_and_what_shows_through_go_by_the_cut() {
    // Half opaque, red in front lets blue through on the right, and blue in front on the left has
    // red's half behind it.
    let frame = frame(
        &crossing("scale=0.5 z=0.1 rotate_y=45 opacity=0.5", "scale=0.5"),
        0,
    );
    let right = pixel(&frame, 195, 90);
    assert!(right[0] > 100 && right[2] > 100, "{right:?}");
    assert_eq!(pixel(&frame, 150, 90), [0, 0, 255]);
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
fn a_shadow_lands_where_the_geometry_says() {
    let frame = frame(&shadowed(CASTER, SLANTED), 0);
    let dark = |x, y| pixel(&frame, x, y)[0] < 8;
    let lit = |x, y| pixel(&frame, x, y)[0] > 120;
    assert!(dark(136, 90), "{:?}", pixel(&frame, 136, 90));
    // Its edges, a pixel or so either way.
    assert!(lit(118, 90) && dark(122, 90));
    assert!(dark(150, 90) && lit(154, 90));
    assert!(lit(136, 79) && dark(136, 83));
    assert!(dark(136, 97) && lit(136, 101));
    // And nowhere else on the wall, not even behind the square from here.
    assert!(lit(200, 90) && lit(136, 40) && lit(50, 150));
}

#[test]
fn a_shadow_is_as_dark_as_the_light_says() {
    let lit = linear(pixel(&frame(&shadowed(CASTER, SLANTED), 0), 200, 90)[0]);
    // The light comes 41.6 degrees off straight on, so the wall takes 180 / 240.8 of it.
    near(lit, 180.0 / 240.83);
    let half = frame(
        &shadowed(CASTER, "kind=directional x=0.5 z=1 shadow=0.5"),
        0,
    );
    near(linear(pixel(&half, 136, 90)[0]), lit / 2.0);
    // None at 0, which is the default, down to the byte.
    let none = frame(&shadowed(CASTER, "kind=directional x=0.5 z=1 shadow=0"), 0);
    let unsaid = frame(&shadowed(CASTER, "kind=directional x=0.5 z=1"), 0);
    assert_eq!(none.rgba, unsaid.rgba);
    assert_eq!(pixel(&none, 136, 90)[0], pixel(&none, 200, 90)[0]);
    // Ambient light comes from nowhere in particular, and lifts what is in shadow.
    let ambient = frame(
        &format!(
            "{}  track V4 video\n    clip air gen=light@1 at=0f dur=24f\n      set kind=ambient intensity=0.25\n",
            shadowed(CASTER, SLANTED)
        ),
        0,
    );
    near(linear(pixel(&ambient, 136, 90)[0]), 0.25);
}

#[test]
fn a_shadow_goes_when_the_light_moves_away() {
    // Its way goes round: by frame 23 it falls 400 to the right of the square, off the frame.
    let moving = shadowed(CASTER, "kind=directional x={0f:0.5 23f:5} z=1 shadow=1");
    let frame_of = |at| frame(&moving, at);
    assert!(pixel(&frame_of(0), 136, 90)[0] < 8);
    let last = frame_of(23);
    let across_the_wall: Vec<u8> = (0..320).map(|x| pixel(&last, x, 150)[0]).collect();
    assert!(across_the_wall.iter().all(|value| *value > 16));
    assert!(pixel(&last, 136, 90)[0] > 16, "{:?}", pixel(&last, 136, 90));
    assert!((60..320).all(|x| pixel(&last, x, 90)[0] > 16 || (62..=102).contains(&x)));
    // Behind the wall, it lights the wall through to the side the camera sees, and the square has the
    // wall in its way, so the shadow is the square's.
    let behind = frame(&shadowed(CASTER, "kind=directional x=0.5 z=-1 shadow=1"), 0);
    assert!(pixel(&behind, 136, 90)[0] > 120);
    assert!(
        pixel(&behind, 80, 90)[1] < 8,
        "{:?}",
        pixel(&behind, 80, 90)
    );
}

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
fn a_point_lights_shadow_spreads_from_it() {
    // In line with the square and twice as far from the wall as it is, the light casts a shadow twice
    // the size: from -160 to -96 in x and -18 to 18 in y, about the frame's middle.
    let frame = frame(
        &shadowed(CASTER, "kind=point x=-0.2 y=0 z=-0.5 shadow=1 intensity=2"),
        0,
    );
    let at = |x: i32, y: i32| pixel(&frame, (160 + x) as u32, (90 + y) as u32)[0];
    // Twice the square's size about the point below the light, x = -64: from -96 to -32 across and
    // -18 to 18 down.
    assert!(
        at(-34, 0) < 8 && at(-30, 0) > 40,
        "{} {}",
        at(-34, 0),
        at(-30, 0)
    );
    assert!(
        at(-34, 16) < 8 && at(-34, 20) > 40,
        "{} {}",
        at(-34, 16),
        at(-34, 20)
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
fn an_aperture_of_nothing_blurs_nothing() {
    // Down to the byte, with a focus said or not, and at no aperture with the camera moved.
    let none = frame(&focused(0.5, "fov=40"), 3);
    assert_eq!(none.rgba, frame(&focused(0.5, "aperture=0"), 3).rgba);
    assert_eq!(none.rgba, frame(&focused(0.5, "focus=3"), 3).rgba);
    assert_eq!(
        none.rgba,
        frame(&focused(0.5, "focus={0f:0 23f:3} aperture=0"), 3).rgba
    );
    assert!(edge_width(&none, 90) <= 1, "{}", edge_width(&none, 90));
}

#[test]
fn the_blur_grows_with_the_distance_from_the_focal_plane() {
    // At the camera's rest, 247.3 pixels back from z = 0, in focus. A radius is 18 pixels of aperture
    // by how far from focus over how far from the eye, and an edge takes twice that to come up.
    let camera = "aperture=0.1 focus=0";
    let eye = 90.0 / 20f64.to_radians().tan();
    let wanted = |z: f64| {
        let depth = eye + z * 180.0;
        18.0 * (z * 180.0).abs() / depth * 2.0
    };
    let width = |z: f64| f64::from(edge_width(&frame(&focused(z, camera), 3), 90));
    assert!(width(0.0) <= 1.0, "{}", width(0.0));
    let mut before = 1.0;
    for z in [0.25, 0.5, 1.0, 2.0] {
        let (found, want) = (width(z), wanted(z));
        assert!(found > before, "{z}: {found} after {before}");
        assert!(
            (found - want).abs() <= want * 0.3 + 1.5,
            "{z}: {found} against {want}"
        );
        before = found;
    }
    // Nearer than focus as well, and nearer the camera it grows faster.
    let (found, want) = (width(-0.5), wanted(-0.5));
    assert!(
        (found - want).abs() <= want * 0.3 + 1.5,
        "{found} against {want}"
    );
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
fn a_blurred_clip_filling_the_frame_keeps_its_corners() {
    let frame = frame(
        "  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      set color=#ffffff scale=2 three_d=true z=0.5\n  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set aperture=0.2\n",
        3,
    );
    for (x, y) in [(0, 0), (319, 0), (0, 179), (319, 179), (160, 90)] {
        assert_eq!(pixel(&frame, x, y), [255, 255, 255], "{x},{y}");
    }
}

#[test]
fn a_blurred_clip_keeps_what_it_had() {
    // What spreads out is what was there, as much of it.
    let total = |frame: &Frame| -> f64 {
        (0..frame.width * frame.height)
            .map(|at| linear(pixel(frame, at % frame.width, at / frame.width)[0]))
            .sum()
    };
    let sharp = total(&frame(&focused(0.5, "aperture=0"), 3));
    let soft = total(&frame(&focused(0.5, "aperture=0.1"), 3));
    assert!(
        (soft - sharp).abs() < sharp * 0.03,
        "{soft} against {sharp}"
    );
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
