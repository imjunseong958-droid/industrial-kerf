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
        let nearer = |frame: &Frame| grey(frame, pair[0].0, pair[0].1) > grey(frame, pair[1].0, pair[1].1);
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
    assert!(near(pixel(&frame, 160, 90), [128, 128, 255]), "{:?}", pixel(&frame, 160, 90));
    assert!(near(pixel(&frame, 118, 90), [128, 128, 255]), "{:?}", pixel(&frame, 118, 90));
    // The floor is tipped back, so it faces up more than it faces the camera.
    let [across, up, toward] = pixel(&frame, 160, 170);
    assert!(across.abs_diff(128) <= 3 && up > 230 && toward > 128 && toward < 180, "{across} {up} {toward}");
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
    assert!(one[0].abs_diff(128) > 60 && other[0].abs_diff(128) > 60, "{one:?} {other:?}");
    assert!((one[0] > 128) != (other[0] > 128), "{one:?} {other:?}");
    assert!(one[1].abs_diff(128) <= 3 && one[2] > 200, "{one:?}");
    assert_eq!(Pass::parse("normals"), Some(Pass::Normals));
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
