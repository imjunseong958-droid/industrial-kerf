//! Clips in depth that move with another: a parent's place in depth is its children's. Each rig is
//! held to the same parts placed by hand, where their own numbers can say it, in the picture and in
//! the passes; and to itself, where a camera and its lights move with it.

use kerf_compose::{ComposeOptions, Compositor, Frame, Pass};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

const WIDE: f64 = 320.0;
const TALL: f64 = 180.0;

fn compositor(body: &str, pass: Pass) -> Compositor {
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
            pass,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

fn frame(body: &str, pass: Pass, at: i64) -> Frame {
    compositor(body, pass).frame(Rate::FPS_24.time_of(at), 1.0)
}

/// How far apart two frames are: the most any pixel is off by, and the share of them off by more
/// than one, which is what going to light and back moves a byte by.
fn apart(a: &Frame, b: &Frame) -> (u8, f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (mut most, mut off) = (0u8, 0usize);
    for (a, b) in a.rgba.chunks(4).zip(b.rgba.chunks(4)) {
        let worst = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
        most = most.max(worst);
        off += usize::from(worst > 1);
    }
    (most, off as f64 / (a.rgba.len() / 4) as f64)
}

/// Whether a frame shows anything but its ground.
fn draws(frame: &Frame) -> bool {
    let first = &frame.rgba[..3];
    frame.rgba.chunks(4).any(|pixel| &pixel[..3] != first)
}

const PASSES: [Pass; 4] = [Pass::Picture, Pass::Ids, Pass::Depth, Pass::Normals];

/// The two scenes are one picture, in every pass, on each of `frames`: to within the last place of
/// the numbers that say where a thing is, which moves an edge by no pixel but may tip one over.
fn same(name: &str, rigged: &str, by_hand: &str, frames: &[i64]) {
    for pass in PASSES {
        for at in frames {
            let (a, b) = (frame(rigged, pass, *at), frame(by_hand, pass, *at));
            assert!(draws(&a), "{name}: nothing in the {pass:?} pass at {at}");
            let (most, off) = apart(&a, &b);
            assert!(
                off <= 0.0005,
                "{name}: the {pass:?} pass at {at} differs, by {most} at most, over {off} of it"
            );
        }
    }
}

/// And the two are not one picture: what is asked of the rig shows.
fn differs(name: &str, a: &str, b: &str, at: i64) {
    let (most, off) = apart(&frame(a, Pass::Picture, at), &frame(b, Pass::Picture, at));
    assert!(most > 32 && off > 0.002, "{name}: {most} over {off}");
}

/// A clip a second long on a track of its own, with a parent when it names one.
fn clip(track: u32, id: &str, generator: &str, parent: &str, set: &str) -> String {
    let parent = if parent.is_empty() {
        String::new()
    } else {
        format!(" parent={parent}")
    };
    format!(
        "  track V{track} video\n    clip {id} gen={generator}@1 at=0f dur=24f{parent}\n      set {set}\n"
    )
}

/// What sees and lights a scene, and what it stands before: a camera off to one side, a light
/// that casts, and a wall behind.
fn stage() -> String {
    [
        clip(1, "wall", "solid", "", "color=#506070 three_d=true z=3 scale=4"),
        clip(20, "lamp", "light", "", "x=-0.3 y=-0.4 z=-0.5 shadow=0.7"),
        clip(21, "fill", "light", "", "kind=ambient intensity=0.3"),
        clip(22, "cam", "camera", "", "x=0.1 y=-0.1 z=-0.4 pan=-6 tilt=-5"),
    ]
    .concat()
}

/// Turned about x by `pitch` and then about y by `yaw`, in degrees, as a clip's own `rotate_x`
/// and `rotate_y` turn it.
fn turned([x, y, z]: [f64; 3], pitch: f64, yaw: f64) -> [f64; 3] {
    let (s, c) = pitch.to_radians().sin_cos();
    let (y, z) = (y * c - z * s, y * s + z * c);
    let (s, c) = yaw.to_radians().sin_cos();
    [x * c + z * s, y, -x * s + z * c]
}

/// Where a part is once a parent has taken it: the part being `at` in the world's pixels from the
/// middle of the frame, and the parent's middle `about`, turned by `pitch` and `yaw` and moved
/// `back` frame heights.
fn taken(at: [f64; 3], about: [f64; 3], (pitch, yaw): (f64, f64), back: f64) -> [f64; 3] {
    let off = turned([at[0] - about[0], at[1] - about[1], at[2] - about[2]], pitch, yaw);
    [about[0] + off[0], about[1] + off[1], about[2] + off[2] + back * TALL]
}

/// A place in the world's pixels as a clip's own `x`, `y`, and `z` say it.
fn said([x, y, z]: [f64; 3]) -> String {
    format!("x={} y={} z={}", x / WIDE, y / TALL, z / TALL)
}

#[test]
fn a_null_moved_back_takes_its_children_as_their_own_z_would() {
    // Three parts of a thing, each with a place of its own in depth, under one null that goes
    // from far to near: the nine z's of the task this was built for, said once.
    let parts = |parent: &str, back: [&str; 3]| {
        [
            stage(),
            clip(3, "body", "mesh", parent, &format!("kind=box width=0.3 height=0.4 length=0.2 color=#c04020 y=0.1 z={}", back[0])),
            clip(4, "head", "mesh", parent, &format!("kind=ball width=0.2 height=0.2 length=0.2 color=#e0c020 y=-0.22 z={}", back[1])),
            clip(5, "plate", "solid", parent, &format!("color=#20c060 three_d=true scale=0.25 x=0.25 rotate_y=30 depth=0.05 z={}", back[2])),
        ]
        .concat()
    };
    let rigged = [
        clip(2, "bot", "null", "", "z={0f:1.5 23f:0.25}"),
        parts("bot", ["0", "0.05", "-0.1"]),
    ]
    .concat();
    // The same parts with the null's keys on each, and a null that does nothing.
    let by_hand = [
        clip(2, "bot", "null", "", "x=0"),
        parts("bot", ["{0f:1.5 23f:0.25}", "{0f:1.55 23f:0.3}", "{0f:1.4 23f:0.15}"]),
    ]
    .concat();
    same("moved back", &rigged, &by_hand, &[0, 11, 23]);
    // And it is not where it would be with no null: the null's `z` is not nothing.
    let unmoved = [clip(2, "bot", "null", "", "x=0"), parts("bot", ["0", "0.05", "-0.1"])].concat();
    differs("moved back", &rigged, &unmoved, 0);
    // A null needs no `three_d` to be somewhere in depth, and one that says it is no other.
    let saying = [
        clip(2, "bot", "null", "", "three_d=true z={0f:1.5 23f:0.25}"),
        parts("bot", ["0", "0.05", "-0.1"]),
    ]
    .concat();
    same("a null in depth", &saying, &rigged, &[11]);
}

#[test]
fn a_null_turned_swings_its_children_round_its_middle() {
    // A null off the middle of the frame, turned about its upright and tipped: each part goes
    // round the null's middle, and is turned as the null is.
    let (pitch, yaw, back) = (-20.0, 40.0, 0.6);
    let about = [0.1 * WIDE, 0.05 * TALL, 0.0];
    let rig = format!("x=0.1 y=0.05 rotate_x={pitch} rotate_y={yaw} z={back}");
    // Each part by where it is from the middle of the frame with the null at rest, in pixels.
    let parts = [
        ("body", "mesh", "kind=box width=0.3 height=0.4 length=0.2 color=#c04020", [0.0, 0.1 * TALL, 0.0]),
        ("arm", "mesh", "kind=cylinder width=0.1 height=0.4 length=0.1 color=#2040c0", [0.3 * WIDE, 0.0, 0.2 * TALL]),
        ("plate", "solid", "color=#20c060 three_d=true scale=0.25 depth=0.05", [-0.25 * WIDE, -0.2 * TALL, -0.1 * TALL]),
    ];
    let mut rigged = vec![stage(), clip(2, "bot", "null", "", &rig)];
    let mut by_hand = vec![stage(), clip(2, "bot", "null", "", "x=0")];
    for (track, (id, generator, set, at)) in parts.into_iter().enumerate() {
        let track = track as u32 + 3;
        // Its own place is from the null's, which the null's `x` and `y` move it with.
        let own = [at[0] + about[0], at[1] + about[1], at[2]];
        rigged.push(clip(track, id, generator, "bot", &format!("{set} {}", said(at))));
        let there = taken(own, about, (pitch, yaw), back);
        let set = format!("{set} {} rotate_x={pitch} rotate_y={yaw}", said(there));
        by_hand.push(clip(track, id, generator, "bot", &set));
    }
    let (rigged, by_hand) = (rigged.concat(), by_hand.concat());
    same("turned", &rigged, &by_hand, &[5]);
    // A turn about the upright alone adds to a part's own, about its own middle.
    let child = "kind=box width=0.3 height=0.2 length=0.2 color=#c04020";
    let rigged = [
        stage(),
        clip(2, "bot", "null", "", "rotate_y={0f:0 20f:100}"),
        clip(3, "body", "mesh", "bot", &format!("{child} x=0.2 rotate_x=15 rotate_y=10")),
    ]
    .concat();
    let there = turned([0.2 * WIDE, 0.0, 0.0], 0.0, 50.0);
    let by_hand = [
        stage(),
        clip(2, "bot", "null", "", "x=0"),
        clip(3, "body", "mesh", "bot", &format!("{child} {} rotate_x=15 rotate_y=60", said(there))),
    ]
    .concat();
    same("turned about the upright", &rigged, &by_hand, &[10]);
    differs("turned about the upright", &rigged, &by_hand, 0);
}

#[test]
fn parents_chain_in_depth_from_the_one_a_clip_names_outward() {
    // A hand on an arm on a body: the arm turns the hand about the arm's middle, and then
    // the body turns both about its own and takes them back with it.
    let child = "kind=box width=0.15 height=0.15 length=0.15 color=#e0c020";
    let rigged = [
        stage(),
        clip(2, "body", "null", "", "z=0.8 rotate_y=25 y=0.1"),
        clip(3, "arm", "null", "body", "x=0.2 rotate_y=-40 z=0.3"),
        clip(4, "hand", "mesh", "arm", &format!("{child} x=0.15 y=-0.1")),
    ]
    .concat();
    // The body's `y` moves the arm and the hand down the frame before anything turns.
    let down = 0.1 * TALL;
    let arm = [0.2 * WIDE, down, 0.0];
    let hand = [0.35 * WIDE, down - 0.1 * TALL, 0.0];
    let by_arm = taken(hand, arm, (0.0, -40.0), 0.3);
    let by_body = taken(by_arm, [0.0, down, 0.0], (0.0, 25.0), 0.8);
    let by_hand = [
        stage(),
        clip(2, "body", "null", "", "x=0"),
        clip(3, "arm", "null", "body", "x=0"),
        clip(4, "hand", "mesh", "arm", &format!("{child} {} rotate_y=-15", said(by_body))),
    ]
    .concat();
    same("chained", &rigged, &by_hand, &[3]);
    // The other way round it is somewhere else: the one it names is first.
    let other = taken(taken(hand, [0.0, down, 0.0], (0.0, 25.0), 0.8), arm, (0.0, -40.0), 0.3);
    let wrong = by_hand.replace(&said(by_body), &said(other));
    differs("chained", &rigged, &wrong, 3);
}

#[test]
fn a_clip_in_depth_that_draws_takes_its_children_with_it() {
    // A head parented to a body that is itself somewhere in depth, and keyed: no null.
    let head = "kind=ball width=0.2 height=0.2 length=0.2 color=#e0c020";
    let body = "kind=box width=0.3 height=0.4 length=0.2 color=#c04020";
    let rigged = [
        stage(),
        clip(2, "body", "mesh", "", &format!("{body} x=0.1 y=0.1 z={{0f:2 20f:0.5}} rotate_y={{0f:0 20f:90}}")),
        clip(3, "head", "mesh", "body", &format!("{head} x=0.1 y=-0.25")),
    ]
    .concat();
    // The body's `x` and `y` move the head across the frame, as they did, and its middle,
    // which the head turns about, is where they put it. By hand the head moves with a null
    // that does nothing, and the body is where it was drawn before any of this.
    let about = [0.1 * WIDE, 0.1 * TALL, 0.0];
    let there = taken([0.2 * WIDE, -0.15 * TALL, 0.0], about, (0.0, 45.0), 1.25);
    let by_hand = [
        stage(),
        clip(2, "body", "mesh", "", &format!("{body} x=0.1 y=0.1 z=1.25 rotate_y=45")),
        clip(3, "head", "mesh", "still", &format!("{head} {} rotate_y=45", said(there))),
        clip(4, "still", "null", "", "x=0"),
    ]
    .concat();
    same("a body", &rigged, &by_hand, &[10]);
    // Let go, the head is where its own numbers have it, and the body has left it there.
    let let_go = rigged.replace(" parent=body", "");
    assert_ne!(let_go, rigged);
    differs("a body", &rigged, &let_go, 10);
}

#[test]
fn only_a_child_in_depth_follows_and_only_a_parent_in_depth_leads() {
    // A flat child keeps to the frame: its parent's place in depth is nothing to it, where
    // the parent's place on the frame still is.
    let flat = |rig: &str| {
        [
            clip(1, "bot", "null", "", rig),
            clip(2, "card", "solid", "bot", "color=#c04020 scale=0.3"),
            clip(3, "cam", "camera", "", "z=-0.2"),
        ]
        .concat()
    };
    let (still, moved) = (flat("x=0.1"), flat("x=0.1 z=2 rotate_y=50 rotate_x=20"));
    assert_eq!(
        frame(&still, Pass::Picture, 0).rgba,
        frame(&moved, Pass::Picture, 0).rgba
    );
    differs("across the frame", &still, &flat("x=-0.2"), 0);
    // A flat clip that draws has no place in depth to hand on: its `z` is nothing, to it or
    // to what moves with it.
    let led = |parent: &str| {
        [
            clip(1, "card", "solid", "", &format!("color=#c04020 scale=0.3 x=-0.2 {parent}")),
            clip(2, "part", "mesh", "card", "kind=box width=0.2 height=0.2 length=0.2 color=#e0c020 x=0.2"),
        ]
        .concat()
    };
    assert_eq!(
        frame(&led(""), Pass::Picture, 0).rgba,
        frame(&led("z=2 rotate_y=50"), Pass::Picture, 0).rgba
    );
    // In depth, the same card leads.
    differs("a card in depth", &led("three_d=true"), &led("three_d=true z=2 rotate_y=50"), 0);
}

/// A scene all of which moves with one null: things that stand, a floor, what lights them, and
/// what sees them.
fn all_on(rig: &str, turns: bool) -> String {
    // With a parent turned on the frame, a part's own turns are about the frame's own lines
    // still, so that it does not go with the parent as one thing: those parts stand square.
    let (tip, turn) = if turns { ("rotate_x=-75", "rotate_y=35") } else { ("", "") };
    [
        clip(1, "rig", "null", "", rig),
        clip(2, "floor", "solid", "rig", &format!("color=#506070 three_d=true {tip} y=0.35 z=1 scale=2")),
        clip(3, "wall", "solid", "rig", "color=#705060 three_d=true z=2 scale=3"),
        clip(4, "body", "mesh", "rig", &format!("kind=box width=0.3 height=0.4 length=0.3 color=#c04020 y=0.1 z=0.8 {turn}")),
        clip(5, "head", "mesh", "rig", "kind=ball width=0.2 height=0.2 length=0.2 color=#e0c020 y=-0.2 z=0.8"),
        clip(6, "sign", "title", "rig", &format!("text=\"HI\" size=0.2 three_d=true depth=0.05 x=-0.25 z=0.5 {turn}")),
        clip(7, "lamp", "light", "rig", "x=0.3 y=-0.5 z=-0.2 shadow=0.7 size=0.1"),
        clip(8, "sun", "light", "rig", "kind=directional x=-0.4 y=0.6 z=1 intensity=0.5 shadow=0.5"),
        clip(9, "fill", "light", "rig", "kind=ambient intensity=0.2"),
        clip(10, "cam", "camera", "rig", "x=0.15 y=-0.2 z=-0.3 pan=-8 tilt=-10 roll=4 fov=50"),
    ]
    .concat()
}

#[test]
fn a_camera_and_its_lights_move_with_a_parent_as_the_clips_do() {
    // The whole scene on one null, which is moved and turned every way that keeps it one
    // thing: what the camera sees is what it saw, since it went with all of it. So the
    // camera and each light are where the parent takes them, as the clips are.
    for (name, rig, turns) in [
        ("moved", "x=0.3 y=-0.2 z=1.5", true),
        ("turned about its upright", "rotate_y=70 z=0.4", true),
        ("tipped", "x=-0.1 rotate_x=-40 rotate_y=25 z=-0.3", true),
        ("turned on the frame", "x=0.2 rotation=30 z=0.5 rotate_y=-50", false),
    ] {
        let (still, moved) = (all_on("x=0", turns), all_on(rig, turns));
        for pass in PASSES {
            let (a, b) = (frame(&still, pass, 0), frame(&moved, pass, 0));
            assert!(draws(&a), "{name}: nothing in the {pass:?} pass");
            let (most, off) = apart(&a, &b);
            assert!(
                off <= 0.002,
                "{name}: the {pass:?} pass differs, by {most} at most, over {off} of it"
            );
        }
        // With the camera let go, the scene has gone and it has not.
        let alone = moved.replace(" parent=rig\n      set x=0.15 y=-0.2 z=-0.3 pan", "\n      set x=0.15 y=-0.2 z=-0.3 pan");
        assert_ne!(alone, moved);
        differs(name, &moved, &alone, 0);
    }
    // A light let go stays where it was while the rest turns away from it.
    let moved = all_on("rotate_y=70 z=0.4", true);
    let alone = moved.replace(" parent=rig\n      set x=0.3 y=-0.5 z=-0.2 shadow", "\n      set x=0.3 y=-0.5 z=-0.2 shadow");
    assert_ne!(alone, moved);
    let (most, off) = apart(&frame(&moved, Pass::Picture, 0), &frame(&alone, Pass::Picture, 0));
    assert!(most > 16 && off > 0.002, "a light let go: {most} over {off}");
}

#[test]
fn a_parents_place_in_depth_is_in_the_number_a_frame_is_kept_by() {
    let scene = |rig: &str| {
        [
            clip(1, "ground", "solid", "", "color=#336699"),
            clip(2, "bot", "null", "", rig),
            clip(3, "body", "mesh", "bot", "kind=box width=0.3 height=0.4 length=0.2 color=#c04020"),
        ]
        .concat()
    };
    let key = |rig: &str, at: i64| {
        compositor(&scene(rig), Pass::Picture)
            .frame_key(Rate::FPS_24.time_of(at), 1.0)
            .expect("a frame of clips the document holds has a key")
    };
    assert_ne!(key("z=0.5", 3), key("z=0.6", 3));
    assert_ne!(key("z=0.5", 3), key("z=0.5 rotate_y=10", 3));
    assert_ne!(key("z=0.5 rotate_x=10", 3), key("z=0.5 rotate_y=10", 3));
    // Keyed, each frame is another; held, they are one.
    assert_ne!(key("z={0f:0 23f:1}", 3), key("z={0f:0 23f:1}", 4));
    assert_eq!(key("z=0.5", 3), key("z=0.5", 3));
}
