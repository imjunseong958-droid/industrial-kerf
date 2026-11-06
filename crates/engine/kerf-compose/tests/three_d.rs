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
