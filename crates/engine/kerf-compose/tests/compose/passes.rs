//! Passes: which clip each pixel is of, and how near it is, drawn from the same clips as the
//! picture.

use kerf_compose::{ComposeOptions, Compositor, Frame, Pass};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

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

fn frame(body: &str, pass: Pass) -> Frame {
    compositor(body, pass).frame(Rate::FPS_24.time_of(3), 1.0)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// Whether two colors are the same to within what going to light and back moves a byte by.
fn same(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1)
}

/// A sky out of depth, a floor tipped back under a wall that stands on it, a small square before
/// the wall, a light with a shadow, and a camera with an aperture: a scene as a guide has one.
const SCENE: &str = concat!(
    "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
    "  track V2 video\n    clip floor gen=solid@1 at=0f dur=24f\n",
    "      set color=#404040 three_d=true rotate_x=-80 y=0.4 z=1 scale=3\n",
    "  track V3 video\n    clip wall gen=solid@1 at=0f dur=24f\n",
    "      set color=#c0c0c0 three_d=true z=2 scale=0.8 blend=screen\n",
    "  track V4 video\n    clip box gen=solid@1 at=0f dur=24f\n",
    "      set color=#ff0000 three_d=true z=0 scale=0.2\n",
    "  track V5 video\n    clip sun gen=light@1 at=0f dur=24f\n",
    "      set kind=directional x=0.3 y=0.6 z=1 shadow=0.8\n",
    "  track V6 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
    "      set focus=0 aperture=0.05\n",
    "  track V7 video\n    clip grade gen=adjustment@1 at=0f dur=24f\n",
    "      fx color.basic@1 exposure=2\n",
);

#[test]
fn an_ids_pass_gives_each_clip_one_color_where_it_shows() {
    let drawn = compositor(SCENE, Pass::Ids);
    let ids = drawn.ids();
    // Every clip that draws has a color, in the order of its tracks, and none of the others.
    assert_eq!(
        ids.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["sky", "floor", "wall", "box"]
    );
    let color = |id: &str| ids.iter().find(|(told, _)| told == id).unwrap().1;
    for (index, (_, a)) in ids.iter().enumerate() {
        assert!(
            a.iter().any(|channel| *channel > 120),
            "{a:?} is near black"
        );
        for (_, b) in &ids[index + 1..] {
            let apart: u32 = a
                .iter()
                .zip(b)
                .map(|(a, b)| u32::from(a.abs_diff(*b)))
                .sum();
            assert!(apart > 90, "{a:?} and {b:?} are too alike");
        }
    }
    let frame = drawn.frame(Rate::FPS_24.time_of(3), 1.0);
    // The box in the middle, the wall around it, the sky at the top, the floor at the bottom:
    // each flat, whatever lights, focus, blends, and the grade over them would do to a picture.
    assert!(same(pixel(&frame, 160, 90), color("box")));
    assert!(same(pixel(&frame, 160, 70), color("wall")));
    assert!(same(pixel(&frame, 118, 90), color("wall")));
    assert!(same(pixel(&frame, 10, 5), color("sky")));
    assert!(same(pixel(&frame, 160, 175), color("floor")));
    // The floor is nearer than the wall's foot there, and hides it, as in the picture.
    assert!(same(pixel(&frame, 118, 115), color("floor")));
    // The picture itself is none of those.
    let picture = compositor(SCENE, Pass::Picture).frame(Rate::FPS_24.time_of(3), 1.0);
    assert!(!same(pixel(&picture, 160, 90), color("box")));
}

#[test]
fn a_depth_pass_is_lighter_the_nearer() {
    let frame = frame(SCENE, Pass::Depth);
    let grey = |x: u32, y: u32| {
        let [r, g, b] = pixel(&frame, x, y);
        assert!(
            r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
            "{r} {g} {b} is not grey"
        );
        r
    };
    // A clip at z=0 before a camera left where it is is mid grey.
    let box_ = grey(160, 90);
    assert!(box_.abs_diff(128) <= 1, "{box_}");
    // The wall behind it is darker, and what is out of depth has no distance.
    let wall = grey(118, 90);
    assert!(wall < box_ && wall > 40, "{wall}");
    assert_eq!(grey(10, 5), 0);
    // The floor comes toward the camera down the frame, each row lighter than the one above.
    let rows: Vec<u8> = (150..180).step_by(5).map(|y| grey(160, y)).collect();
    assert!(rows.windows(2).all(|pair| pair[1] > pair[0]), "{rows:?}");
    assert!(*rows.last().unwrap() > wall, "{rows:?}");
}

#[test]
fn a_relative_depth_pass_is_stretched_to_its_frames_own_nearest_and_farthest() {
    let (depth, relative) = (frame(SCENE, Pass::Depth), frame(SCENE, Pass::Relative));
    let grey = |frame: &Frame, x: u32, y: u32| pixel(frame, x, y)[0];
    let all = |frame: &Frame| -> Vec<u8> {
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[0])
            .filter(|grey| *grey > 0)
            .collect()
    };
    // It reaches white at the nearest and stops just above black at the farthest, where the
    // pass by the scene's own numbers does neither.
    let (own, stretched) = (all(&depth), all(&relative));
    assert_eq!(stretched.len(), own.len());
    assert_eq!(*stretched.iter().max().unwrap(), 255);
    assert_eq!(*stretched.iter().min().unwrap(), 1);
    assert!(*own.iter().max().unwrap() < 255 && *own.iter().min().unwrap() > 2);
    // What is not in depth is black in both, and the order of near and far is the same.
    assert_eq!(grey(&relative, 10, 5), 0);
    let places = [(160, 90), (118, 90), (160, 150), (160, 175)];
    for pair in places.windows(2) {
        let nearer =
            |frame: &Frame| grey(frame, pair[0].0, pair[0].1) > grey(frame, pair[1].0, pair[1].1);
        assert_eq!(nearer(&depth), nearer(&relative), "{pair:?}");
    }
    // It is a pass of its own, by name and by number.
    assert_eq!(Pass::parse("relative"), Some(Pass::Relative));
    assert_eq!(Pass::Relative.name(), "relative");
    let at = Rate::FPS_24.time_of(3);
    let key = |pass| compositor(SCENE, pass).frame_key(at, 1.0);
    assert!(key(Pass::Relative).is_some() && key(Pass::Relative) != key(Pass::Depth));
}

#[test]
fn a_normals_pass_says_which_way_each_pixel_faces() {
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3);
    let frame = frame(SCENE, Pass::Normals);
    // The box and the wall face the camera: no way across or up, and all of it toward.
    assert!(
        near(pixel(&frame, 160, 90), [128, 128, 255]),
        "{:?}",
        pixel(&frame, 160, 90)
    );
    assert!(
        near(pixel(&frame, 118, 90), [128, 128, 255]),
        "{:?}",
        pixel(&frame, 118, 90)
    );
    // The floor is tipped back, so it faces up more than it faces the camera.
    let [across, up, toward] = pixel(&frame, 160, 170);
    assert!(
        across.abs_diff(128) <= 3 && up > 230 && toward > 128 && toward < 180,
        "{across} {up} {toward}"
    );
    // What is not in depth faces no way.
    assert_eq!(pixel(&frame, 10, 5), [0, 0, 0]);
    // A clip turned to its right shows red for it, and one turned the other way the less.
    let turned = |by: i32| {
        let body = format!(
            "  track V1 video\n    clip side gen=solid@1 at=0f dur=24f\n      set three_d=true rotate_y={by} scale=0.5\n"
        );
        pixel(&self::frame(&body, Pass::Normals), 160, 90)
    };
    let (one, other) = (turned(40), turned(-40));
    assert!(
        one[0].abs_diff(128) > 60 && other[0].abs_diff(128) > 60,
        "{one:?} {other:?}"
    );
    assert!((one[0] > 128) != (other[0] > 128), "{one:?} {other:?}");
    assert!(one[1].abs_diff(128) <= 3 && one[2] > 200, "{one:?}");
    assert_eq!(Pass::parse("normals"), Some(Pass::Normals));
}

#[test]
fn a_matte_is_one_clip_in_white_with_what_stands_before_it_cut_out() {
    let matte = |clip: &str| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{SCENE}"
            ),
            Catalog::builtin(),
        )
        .unwrap()
        .project;
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                matte: vec![clip.to_owned()],
                ..ComposeOptions::default()
            },
        )
    };
    let wall = matte("wall")
        .expect("a wall")
        .frame(Rate::FPS_24.time_of(3), 1.0);
    // The wall is white where it shows, and the box before it is a hole in it.
    assert_eq!(pixel(&wall, 118, 90), [255, 255, 255]);
    assert_eq!(pixel(&wall, 160, 90), [0, 0, 0]);
    assert_eq!(pixel(&wall, 10, 5), [0, 0, 0]);
    // Every pixel is one or the other.
    assert!(wall
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[0] == 0 || p[0] == 255));
    let box_ = matte("box")
        .expect("a box")
        .frame(Rate::FPS_24.time_of(3), 1.0);
    assert_eq!(pixel(&box_, 160, 90), [255, 255, 255]);
    assert_eq!(pixel(&box_, 118, 90), [0, 0, 0]);
    // A clip that draws nothing has no matte, and neither has one that is not there.
    assert!(matte("cam").is_err() && matte("nobody").is_err());
}

#[test]
fn a_matte_has_none_of_a_clip_told_long_after_its_own() {
    // A square on the left, 143 clips that follow it on its track, and on the track above a
    // square on the right, which is told 145th: where the colors once came round.
    let mut body =
        "  track V1 video\n    clip left gen=solid@1 at=0f dur=24f\n      set scale=0.4 x=-0.25\n"
            .to_owned();
    for after in 0..143 {
        body += &format!("    clip c{after} gen=solid@1 at={}f dur=1f\n", 24 + after);
    }
    body +=
        "  track V2 video\n    clip right gen=solid@1 at=0f dur=24f\n      set scale=0.4 x=0.25\n";
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let drawn = |pass: Pass, matte: Option<&str>| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                pass,
                matte: matte.map(str::to_owned).into_iter().collect(),
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let at = Rate::FPS_24.time_of(3);
    let (on_left, on_right) = ((80, 90), (240, 90));
    let ids = drawn(Pass::Ids, None);
    let colors = ids.ids();
    assert_eq!(
        (colors[0].0.as_str(), colors[144].0.as_str()),
        ("left", "right")
    );
    // Each is its own color in the pass, further from the other's than a reader takes for it.
    let frame = ids.frame(at, 1.0);
    assert!(same(pixel(&frame, on_left.0, on_left.1), colors[0].1));
    assert!(same(pixel(&frame, on_right.0, on_right.1), colors[144].1));
    let apart = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).map(|(a, b)| a.abs_diff(b)).max().unwrap();
    assert!(
        apart(colors[0].1, colors[144].1) > 6,
        "{:?} {:?}",
        colors[0].1,
        colors[144].1
    );
    // So a matte of either is white on it and black on the other.
    for (clip, its, others) in [("left", on_left, on_right), ("right", on_right, on_left)] {
        let matte = drawn(Pass::Picture, Some(clip)).frame(at, 1.0);
        assert_eq!(pixel(&matte, its.0, its.1), [255, 255, 255], "{clip}");
        assert_eq!(pixel(&matte, others.0, others.1), [0, 0, 0], "{clip}");
    }
}

#[test]
fn a_pass_holds_still_and_is_kept_apart_from_the_picture() {
    // The same scene drawn twice is the same pass to the byte, and its number is its own.
    let at = Rate::FPS_24.time_of(3);
    let ids = compositor(SCENE, Pass::Ids);
    assert_eq!(ids.frame(at, 1.0), ids.frame(at, 1.0));
    let key = |pass| compositor(SCENE, pass).frame_key(at, 1.0);
    let (picture, ids, depth) = (key(Pass::Picture), key(Pass::Ids), key(Pass::Depth));
    assert!(picture.is_some() && ids.is_some() && depth.is_some());
    assert!(picture != ids && ids != depth && picture != depth);
    // A pass is the CPU's.
    assert!(!compositor(SCENE, Pass::Ids).draws_on_gpu(at, 1.0));
}

#[test]
fn a_nested_sequence_is_one_clip_in_a_pass() {
    let body = concat!(
        "  track V1 video\n    clip inner gen=solid@1 at=0f dur=24f\n",
        "      set color=#ffffff scale=0.5\n",
        "sequence s2 rate=24 size=320x180 audio=48000\n",
        "  track V1 video\n    clip shot seq=s1 src=0f at=0f dur=24f\n",
    );
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let drawn = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some("s2".to_owned()),
            pass: Pass::Ids,
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    let ids = drawn.ids();
    assert_eq!(ids[0].0, "shot");
    let frame = drawn.frame(Rate::FPS_24.time_of(3), 1.0);
    // Where the nested sequence has a picture it is the clip's color, and nothing where it has none.
    assert!(same(pixel(&frame, 160, 90), ids[0].1));
    assert_eq!(pixel(&frame, 10, 10), [0, 0, 0]);
    assert_eq!(Pass::parse("ids"), Some(Pass::Ids));
    assert_eq!(Pass::parse(Pass::Depth.name()), Some(Pass::Depth));
    assert_eq!(Pass::parse("albedo"), None);
}

#[test]
fn an_ids_pass_drawn_hard_gives_every_pixel_to_one_clip() {
    // Two planes in depth that cross, a square at less than full opacity with a mask feathered
    // over 30 pixels, a dissolve between two clips, and a nested sequence over all of them.
    let body = concat!(
        "  track V1 video\n    clip inner gen=solid@1 at=0f dur=24f\n",
        "      set scale=0.3 rotation=30\n      mask k1 ellipse=0.5,0.5,0.4,0.4 feather=12\n",
        "sequence s2 rate=24 size=320x180 audio=48000\n",
        "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n",
        "  track V2 video\n    clip floor gen=solid@1 at=0f dur=24f\n",
        "      set three_d=true rotate_x=-80 y=0.4 z=1 scale=3\n",
        "  track V3 video\n    clip wall gen=solid@1 at=0f dur=24f\n",
        "      set three_d=true rotate_y=60 z=1 scale=0.8\n",
        "  track V4 video\n    clip soft gen=solid@1 at=0f dur=24f\n",
        "      set scale=0.5 opacity=0.8\n      mask k1 rect=0.2,0.2,0.8,0.8 feather=30\n",
        "  track V5 video\n    clip glow gen=solid@1 at=0f dur=12f\n      set scale=0.3 x=-0.3\n",
        "    clip spark gen=solid@1 at=12f dur=12f\n      set scale=0.2 x=-0.3 rotation=20\n",
        "    transition t1 from=glow to=spark fx=dissolve@1 dur=8f\n",
        "  track V6 video\n    clip shot seq=s1 src=0f at=0f dur=24f\n      set x=0.3 y=-0.2\n",
    );
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let drawn = |hard_edges: bool| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some("s2".to_owned()),
                pass: Pass::Ids,
                hard_edges,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let (soft, hard) = (drawn(false), drawn(true));
    let colors: Vec<[u8; 3]> = hard.ids().into_iter().map(|(_, color)| color).collect();
    // The nested sequence's own clip is told after the program's, and draws as the clip it is in.
    assert_eq!(colors.len(), 8, "{:?}", hard.ids());
    let (mut between, mut drawn_hard) = (0, 0);
    // Mid dissolve, and either side of it.
    for frame in [4, 9, 12, 15, 20] {
        let at = Rate::FPS_24.time_of(frame);
        let of_none = |frame: &Frame| {
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| {
                    let pixel = [pixel[0], pixel[1], pixel[2]];
                    pixel != [0; 3] && !colors.iter().any(|color| same(*color, pixel))
                })
                .count()
        };
        between += of_none(&soft.frame(at, 1.0));
        let frame = hard.frame(at, 1.0);
        drawn_hard += of_none(&frame);
        // Not a clip's color to within a step: exactly one.
        let exact = frame.rgba.as_chunks::<4>().0.iter().all(|pixel| {
            let pixel = [pixel[0], pixel[1], pixel[2]];
            pixel == [0; 3] || colors.contains(&pixel)
        });
        assert!(exact, "frame {frame:?}");
        // A pass drawn hard is a frame of its own, kept apart from one drawn soft.
        assert_ne!(soft.frame_key(at, 1.0), hard.frame_key(at, 1.0));
    }
    assert!(
        between > 1000,
        "only {between} pixels are between two clips drawn soft"
    );
    assert_eq!(drawn_hard, 0);
    // The square, at 0.8 of full opacity over the wall, is between the two drawn soft all over,
    // and drawn hard is its own color wherever it covers half a pixel or more: its feathered
    // mask cuts where it is that far through.
    let at = Rate::FPS_24.time_of(4);
    let (soft_frame, frame) = (soft.frame(at, 1.0), hard.frame(at, 1.0));
    let (wall, square) = (colors[2], colors[3]);
    assert!(!colors
        .iter()
        .any(|color| same(pixel(&soft_frame, 160, 90), *color)));
    assert_eq!(pixel(&frame, 160, 90), square);
    assert_eq!(pixel(&frame, 160, 66), square);
    assert_eq!(pixel(&frame, 160, 60), wall);
    // The picture and its other passes do not take the option.
    let picture = |hard_edges: bool| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some("s2".to_owned()),
                hard_edges,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
        .frame_key(Rate::FPS_24.time_of(4), 1.0)
    };
    assert_eq!(picture(false), picture(true));
}

/// [`SCENE`] with a model filming all of it but the box, and `s2`, the scene laid over a take.
fn filmed_scene() -> kerf_model::Project {
    let scene = SCENE
        .replace("set color=#88aaff", "set color=#88aaff filmed=true")
        .replace("set color=#404040", "set color=#404040 filmed=true")
        .replace("set color=#c0c0c0", "set color=#c0c0c0 filmed=true");
    assert_eq!(scene.matches("filmed=true").count(), 3);
    parse_document(
        &format!(
            concat!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{}",
                "sequence s2 rate=24 size=320x180 audio=48000\n",
                "  track V1 video\n    clip take gen=solid@1 at=0f dur=24f\n      set color=#00ff00\n",
                "  track V2 video\n    clip world seq=s1 src=0f at=0f dur=24f\n",
            ),
            scene
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

#[test]
fn a_guide_of_only_what_a_model_films_leaves_the_rest_out() {
    let project = filmed_scene();
    let drawn = |pass: Pass, only_filmed: bool| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                pass,
                only_filmed,
                hard_edges: true,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let at = Rate::FPS_24.time_of(3);
    let (all, only) = (drawn(Pass::Ids, false), drawn(Pass::Ids, true));
    // Each clip has the color it has with all of them drawn, the one left out too.
    assert_eq!(all.ids(), only.ids());
    let color = |id: &str| all.ids().iter().find(|(told, _)| told == id).unwrap().1;
    let (all_frame, only_frame) = (all.frame(at, 1.0), only.frame(at, 1.0));
    // The box no model films is before the wall, and with it left out the wall is there.
    assert_eq!(pixel(&all_frame, 160, 90), color("box"));
    assert_eq!(pixel(&only_frame, 160, 90), color("wall"));
    // What a model films is where it was.
    assert_eq!(pixel(&all_frame, 20, 10), color("sky"));
    assert_eq!(pixel(&only_frame, 20, 10), color("sky"));
    assert_eq!(pixel(&only_frame, 160, 170), pixel(&all_frame, 160, 170));
    // And so in the pass a model follows: the box is nearer than the wall behind it.
    let near =
        |only_filmed: bool| pixel(&drawn(Pass::Depth, only_filmed).frame(at, 1.0), 160, 90)[0];
    assert!(
        near(true) < near(false),
        "{} and {}",
        near(true),
        near(false)
    );
    // A frame of it is its own, kept apart from one of the same clips with no other in the
    // sequence, where each has another color.
    let ids_of = |body: &str, only_filmed: bool| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        let drawn = Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                pass: Pass::Ids,
                only_filmed,
                ..ComposeOptions::default()
            },
        )
        .expect("composes");
        (drawn.frame(at, 1.0), drawn.frame_key(at, 1.0))
    };
    let sky = "  track V2 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set filmed=true\n";
    let sign = "  track V1 video\n    clip sign gen=solid@1 at=0f dur=24f\n      set scale=0.5\n";
    let (with_a_sign, alone) = (ids_of(&format!("{sign}{sky}"), true), ids_of(sky, false));
    assert_ne!(with_a_sign.0, alone.0);
    assert_ne!(with_a_sign.1, alone.1);
    // A sequence no clip of which a model films has nothing to draw so.
    let none = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{SCENE}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let refused = Compositor::new(
        &none,
        Catalog::builtin(),
        &ComposeOptions {
            only_filmed: true,
            ..ComposeOptions::default()
        },
    )
    .err()
    .expect("refused")
    .to_string();
    assert!(refused.contains("`filmed=true`"), "{refused}");
}

#[test]
fn what_a_model_films_is_the_takes_in_a_pass_of_the_scene_laid_over_it() {
    let project = filmed_scene();
    let drawn = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some("s2".to_owned()),
            pass: Pass::Ids,
            hard_edges: true,
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    let ids = drawn.ids();
    let color = |id: &str| ids.iter().find(|(told, _)| told == id).unwrap().1;
    let frame = drawn.frame(Rate::FPS_24.time_of(3), 1.0);
    // The scene's clip is where the box Kerf draws is, and the take everywhere else, the sky
    // out of depth and the wall behind the box among it.
    assert_eq!(pixel(&frame, 160, 90), color("world"));
    assert_eq!(pixel(&frame, 20, 10), color("take"));
    assert_eq!(pixel(&frame, 100, 90), color("take"));
    assert_eq!(pixel(&frame, 160, 170), color("take"));
}

#[test]
fn a_guide_of_only_what_a_model_films_keeps_what_a_hidden_track_is_read_for() {
    let drawn = |body: &str| {
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
                only_filmed: true,
                ..ComposeOptions::default()
            },
        )
    };
    // A matte on a hidden track cuts a clip a model films, and no model films the matte:
    // it draws nothing itself, and is still read.
    let matted = concat!(
        "  track V1 video\n    clip g1 gen=solid@1 at=0f dur=24f\n",
        "      set color=#ff0000 filmed=true\n",
        "      fx key.trackmatte@1 matte=V2 composite=matte-luma\n",
        "  track V2 video enabled=false\n    clip m1 gen=solid@1 at=0f dur=24f\n",
        "      set color=#ffffff scale=0.5\n",
    );
    let frame = drawn(matted)
        .expect("composes")
        .frame(Rate::FPS_24.time_of(3), 1.0);
    assert_eq!(pixel(&frame, 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&frame, 10, 10), [0, 0, 0]);
    // A camera draws nothing, whatever it says: with no clip that draws a model's, such a
    // guide is of nothing.
    let camera = concat!(
        "  track V1 video\n    clip sign gen=solid@1 at=0f dur=24f\n",
        "  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n      set filmed=true\n",
    );
    assert!(drawn(camera).is_err());
}

/// How far a motion pass says the thing at a pixel goes in a frame, across and down in pixels
/// of a frame `height` tall: none where it says nothing is there.
fn motion_at(frame: &Frame, x: u32, y: u32) -> Option<(f64, f64)> {
    let [red, green, blue] = pixel(frame, x, y);
    let most = 0.125 * f64::from(frame.height);
    let side = |byte: u8| (f64::from(byte) / 255.0 - 0.5) * 2.0 * most;
    (blue > 127).then(|| (side(red), side(green)))
}

#[test]
fn a_motion_pass_says_how_far_each_clip_goes_in_a_frame() {
    // A square that slides right by 0.3 of the frame's width in 24 frames, 4 pixels a frame,
    // beside one that holds still, on nothing.
    let body = concat!(
        "  track V1 video\n    clip slides gen=solid@1 at=0f dur=24f\n",
        "      set scale=0.2 x={0f:-0.3 24f:0} y=-0.25\n",
        "  track V2 video\n    clip holds gen=solid@1 at=0f dur=24f\n",
        "      set scale=0.2 y=0.25\n",
    );
    let drawn = frame(body, Pass::Motion);
    // At frame 3 the sliding square's middle is 84 pixels left of the frame's.
    let (across, down) = motion_at(&drawn, 160 - 84, 45).expect("the sliding square");
    assert!(
        (across - 4.0).abs() < 0.2 && down.abs() < 0.2,
        "{across} {down}"
    );
    // What holds still is mid grey, as near as a byte comes to it, and blue says it is there.
    let (across, down) = motion_at(&drawn, 160, 135).expect("the still square");
    assert!(across.abs() < 0.1 && down.abs() < 0.1, "{across} {down}");
    assert_eq!(pixel(&drawn, 10, 90), [0, 0, 0]);
    // A frame is as long as the rate asked for has it: at half the rate it goes twice as far.
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let slower = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            pass: Pass::Motion,
            rate: Some(Rate::parse("12").expect("a rate")),
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    let at = Rate::FPS_24.time_of(3);
    let (across, _) = motion_at(&slower.frame(at, 1.0), 160 - 84, 45).expect("the square");
    assert!((across - 8.0).abs() < 0.2, "{across}");
    assert_ne!(
        slower.frame_key(at, 1.0),
        compositor(body, Pass::Motion).frame_key(at, 1.0)
    );
    // And it is true of the file the frames fill. One twice as wide in shape keeps half the
    // frame's height, at twice the size: there the square goes 8 pixels, by that file's height.
    let wide = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            pass: Pass::Motion,
            shape: Some((640, 180)),
            ..ComposeOptions::default()
        },
    )
    .expect("composes");
    let (across, _) = motion_at(&wide.frame(at, 1.0), 160 - 84, 45).expect("the square");
    assert!((across - 8.0).abs() < 0.4, "{across}");
    assert_ne!(
        wide.frame_key(at, 1.0),
        compositor(body, Pass::Motion).frame_key(at, 1.0)
    );
}

/// How many pixels of a motion pass's frame say something is there, and how many of those say
/// it goes anywhere.
fn there_and_going(frame: &Frame) -> (usize, usize) {
    let (mut there, mut going) = (0, 0);
    for y in 0..frame.height {
        for x in 0..frame.width {
            if let Some((across, down)) = motion_at(frame, x, y) {
                there += 1;
                going += usize::from(across.abs() > 0.1 || down.abs() > 0.1);
            }
        }
    }
    (there, going)
}

#[test]
fn what_holds_still_is_said_to_hold_still_to_its_last_frame() {
    let at = |frame: i64| Rate::FPS_24.time_of(frame);
    let still = |body: &str, frames: &[i64]| {
        let drawn = compositor(body, Pass::Motion);
        for frame in frames {
            let (there, going) = there_and_going(&drawn.frame(at(*frame), 1.0));
            assert!(
                there > 500 && going == 0,
                "frame {frame}: {going} of {there}"
            );
        }
    };
    // A wall before a camera that holds still, off to a side and turned. No camera shows a
    // frame after the last, and the one that shows is the one the wall is seen through.
    let camera = concat!(
        "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n",
        "      set color=#c0c0c0 three_d=true z=1 scale=0.6\n",
        "  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
        "      set x=0.2 z=-0.3 pan=10\n",
    );
    still(camera, &[0, 10, 22, 23]);
    // What a parent turned in depth carries, a mesh among it, and one of them ending before
    // the others: a frame after a clip's last it does not show, and is where its parent has it.
    let parent = concat!(
        "  track V1 video\n    clip rig gen=null@1 at=0f dur=24f\n      set rotate_y=40 z=0.5\n",
        "  track V2 video\n    clip card gen=solid@1 at=0f dur=24f parent=rig\n",
        "      set three_d=true scale=0.3 x=0.2\n",
        "  track V3 video\n    clip plate gen=mesh@1 at=0f dur=24f parent=rig\n",
        "      set kind=box width=0.2 height=0.2 length=0.002 x=-0.25 y=0.2\n",
        "  track V4 video\n    clip short gen=solid@1 at=0f dur=12f parent=rig\n",
        "      set three_d=true scale=0.2 x=-0.2 y=-0.25\n",
    );
    still(parent, &[5, 10, 11, 23]);
    // The frame before another camera takes over is as the camera that shows has it,
    // whatever the next one is: the same picture under the same number.
    let cameras = |next: &str| {
        format!(
            concat!(
                "  track V1 video\n    clip wall gen=solid@1 at=0f dur=24f\n",
                "      set color=#c0c0c0 three_d=true z=1 scale=0.4\n",
                "  track V2 video\n    clip one gen=camera@1 at=0f dur=12f\n      set x=-0.1\n",
                "    clip two gen=camera@1 at=12f dur=12f\n      set x={}\n",
            ),
            next
        )
    };
    still(&cameras("0.1"), &[11]);
    let (a, b) = (
        compositor(&cameras("0.1"), Pass::Motion),
        compositor(&cameras("-0.07"), Pass::Motion),
    );
    assert_eq!(a.frame(at(11), 1.0), b.frame(at(11), 1.0));
    assert_eq!(a.frame_key(at(11), 1.0), b.frame_key(at(11), 1.0));
}

#[test]
fn a_motion_pass_says_of_each_pixel_a_thing_or_nothing() {
    let nothing_between = |body: &str| {
        let drawn = frame(body, Pass::Motion);
        let (there, going) = there_and_going(&drawn);
        assert!(there > 500 && going == 0, "{going} of {there}");
        // Every pixel is nothing, black, or a thing's, blue full.
        for pixel in drawn.rgba.as_chunks::<4>().0 {
            assert!(pixel[2] == 0 || pixel[2] == 255, "{pixel:?}");
        }
        drawn
    };
    // A still clip seen through in part, over nothing: still, and there.
    let half = nothing_between(
        "  track V1 video\n    clip half gen=solid@1 at=0f dur=24f\n      set scale=0.4 opacity=0.6 x=0.2\n",
    );
    assert!(motion_at(&half, 224, 90).is_some());
    // A still clip turned, whose edges cover pixels in part, and one with a soft mask.
    nothing_between(
        "  track V1 video\n    clip turned gen=solid@1 at=0f dur=24f\n      set scale=0.4 rotation=30\n",
    );
    nothing_between(concat!(
        "  track V1 video\n    clip soft gen=solid@1 at=0f dur=24f\n      set scale=0.5\n",
        "      mask k1 rect=0.2,0.2,0.8,0.8 feather=30\n",
    ));
}

#[test]
fn what_a_motion_pass_says_of_a_pixel_is_where_its_clip_is_a_frame_on() {
    // A floor and a wall, a square and a plate that cross before them, and a camera that
    // goes in and turns: flat clips in depth, a mesh, and everything moving on the screen.
    // The mesh is a thin plate, all of it as far off as its middle, so that its middle goes
    // as far as its pixels do: the near side of a ball goes further than its outline.
    let body = concat!(
        "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
        "  track V2 video\n    clip floor gen=solid@1 at=0f dur=24f\n",
        "      set color=#404040 three_d=true rotate_x=-80 y=0.4 z=1 scale=3\n",
        "  track V3 video\n    clip wall gen=solid@1 at=0f dur=24f\n",
        "      set color=#c0c0c0 three_d=true z=2 scale=0.8\n",
        "  track V4 video\n    clip square gen=solid@1 at=0f dur=24f\n",
        "      set color=#ff0000 three_d=true scale=0.2 x={0f:-0.25 24f:0.15} y=-0.1\n",
        "  track V5 video\n    clip plate gen=mesh@1 at=0f dur=24f\n",
        "      set kind=box width=0.25 height=0.25 length=0.002 x={0f:0.3 24f:-0.1} y=0.2 z=0.5\n",
        "  track V6 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
        "      set z={0f:-0.4 24f:0.2} pan={0f:-4 24f:4}\n",
    );
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let drawn = |pass: Pass| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                pass,
                hard_edges: true,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let (ids, motion) = (drawn(Pass::Ids), drawn(Pass::Motion));
    let color = |id: &str| ids.ids().iter().find(|(told, _)| told == id).unwrap().1;
    // A pixel well inside one clip: every pixel about it is that clip's too.
    let inside = |frame: &Frame, x: i64, y: i64| -> Option<[u8; 3]> {
        let (w, h) = (i64::from(frame.width), i64::from(frame.height));
        if x < 2 || y < 2 || x >= w - 2 || y >= h - 2 {
            return None;
        }
        let here = pixel(frame, x as u32, y as u32);
        (-2..=2)
            .flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)))
            .all(|(dx, dy)| pixel(frame, (x + dx) as u32, (y + dy) as u32) == here)
            .then_some(here)
    };
    for frame in [2, 11, 20] {
        let (now, next) = (Rate::FPS_24.time_of(frame), Rate::FPS_24.time_of(frame + 1));
        let (ids_now, ids_next) = (ids.frame(now, 1.0), ids.frame(next, 1.0));
        let moves = motion.frame(now, 1.0);
        let (mut held, mut lost) = (std::collections::BTreeMap::new(), 0);
        for y in (2..178).step_by(3) {
            for x in (2..318).step_by(3) {
                let Some(clip) = inside(&ids_now, x, y) else {
                    continue;
                };
                // The sky is out of depth and holds still; all else is in depth.
                let (across, down) = motion_at(&moves, x as u32, y as u32).expect("a clip");
                let to = (
                    (x as f64 + across).round() as i64,
                    (y as f64 + down).round() as i64,
                );
                // Where it goes it is seen whole, or something has come before it there.
                match inside(&ids_next, to.0, to.1) {
                    Some(there) if there == clip => *held.entry(clip).or_insert(0) += 1,
                    Some(_) => lost += 1,
                    None => {}
                }
            }
        }
        // Each clip is found where its pixels were said to go, the mesh among them, and next
        // to none is found to be another's: only what one of them has moved in front of.
        for id in ["floor", "wall", "square", "plate"] {
            let found = held.get(&color(id)).copied().unwrap_or(0);
            assert!(found >= 15, "frame {frame}: {id} held at {found} pixels");
        }
        let all: i32 = held.values().sum();
        assert!(
            lost * 50 < all,
            "frame {frame}: {lost} of {all} went astray"
        );
    }
    // And as far. The square and the plate, which nothing comes before, are eight frames on
    // where the pass has said frame by frame: the middle of each has gone as far as its pixels
    // were said to go, to within what a whole pixel at each of its edges moves a middle by.
    let middle = |frame: &Frame, id: &str| {
        let (mut x, mut y, mut count) = (0.0, 0.0, 0.0);
        for at_y in 0..frame.height {
            for at_x in 0..frame.width {
                if pixel(frame, at_x, at_y) == color(id) {
                    (x, y, count) = (x + f64::from(at_x), y + f64::from(at_y), count + 1.0);
                }
            }
        }
        (x / count, y / count)
    };
    for id in ["square", "plate"] {
        let (mut across, mut down) = (0.0, 0.0);
        for frame in 4..12 {
            let at = Rate::FPS_24.time_of(frame);
            let (ids_now, moves) = (ids.frame(at, 1.0), motion.frame(at, 1.0));
            let (mut sum, mut count) = ((0.0, 0.0), 0.0);
            for y in 2..178 {
                for x in 2..318 {
                    if inside(&ids_now, x, y) == Some(color(id)) {
                        let said = motion_at(&moves, x as u32, y as u32).expect("a clip");
                        (sum, count) = ((sum.0 + said.0, sum.1 + said.1), count + 1.0);
                    }
                }
            }
            (across, down) = (across + sum.0 / count, down + sum.1 / count);
        }
        let at = |frame: i64| ids.frame(Rate::FPS_24.time_of(frame), 1.0);
        let (from, to) = (middle(&at(4), id), middle(&at(12), id));
        let went = (to.0 - from.0, to.1 - from.1);
        assert!(went.0.abs() > 8.0, "{id} went {went:?}");
        assert!(
            (went.0 - across).abs() < 0.8 && (went.1 - down).abs() < 0.8,
            "{id} went {went:?} and is said to go {across} {down}"
        );
    }
}
