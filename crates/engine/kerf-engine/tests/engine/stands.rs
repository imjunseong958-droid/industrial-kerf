//! What each clip in depth stands on, measured with nothing drawn: for each thing said, a
//! scene that has to be said and one that must not be.

use kerf_engine::stands::{standing, How, Run, Standing, Stood, TOUCH};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project, Rate};
use std::path::{Path, PathBuf};

fn project_of(size: &str, body: &str) -> Project {
    parse_document(
        &format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={size} audio=48000\n{body}"),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

fn project(body: &str) -> Project {
    project_of("320x180", body)
}

fn read(body: &str) -> Standing {
    standing(&project(body), None, &[]).expect("reads")
}

fn clip<'a>(standing: &'a Standing, id: &str) -> &'a Stood {
    standing
        .clips
        .iter()
        .find(|stood| stood.clip == id)
        .unwrap_or_else(|| panic!("no {id} in {standing:?}"))
}

/// The one way a clip stands, where it stands one way on every frame read.
fn only<'a>(standing: &'a Standing, id: &str) -> &'a Run {
    let stood = clip(standing, id);
    assert_eq!(stood.runs.len(), 1, "{stood:?}");
    &stood.runs[0]
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// A clip a second long on a track of its own.
fn on_track(track: u32, id: &str, generator: &str, set: &str) -> String {
    format!(
        "  track V{track} video\n    clip {id} gen={generator}@1 at=0f dur=24f\n      set {set}\n"
    )
}

/// A level floor 0.4 under the middle of the frame, a camera, and a light.
fn level() -> String {
    [
        on_track(
            1,
            "floor",
            "solid",
            "color=#404040 three_d=true rotate_x=-90 y=0.4 z=1 scale=4",
        ),
        on_track(8, "cam", "camera", "z=-0.5"),
        on_track(9, "sun", "light", "kind=ambient"),
    ]
    .concat()
}

/// The floor the `model-shot` skill lays: tipped back 6 degrees, lower the nearer.
fn tipped() -> String {
    on_track(
        1,
        "road",
        "solid",
        "color=#404448 three_d=true rotate_x=-84 y=0.75 z=4 scale=16",
    )
}

/// A box 0.2 each way with its middle at `set`.
fn crate_at(set: &str) -> String {
    on_track(
        2,
        "crate",
        "mesh",
        &format!("kind=box width=0.2 height=0.2 length=0.2 z=1 {set}"),
    )
}

#[test]
fn a_box_on_a_floor_rests_and_one_lifted_floats_and_one_let_down_is_sunk() {
    // Its foot is a tenth under its middle, and the floor four tenths under the frame's.
    let stands = read(&[level(), crate_at("y=0.3")].concat());
    let rests = only(&stands, "crate");
    assert_eq!(
        (rests.how, rests.on.as_deref()),
        (How::Rests, Some("floor"))
    );
    assert_eq!(
        (rests.frames, rests.slope < 0.01, rests.open < 1e-9),
        ((0, 23), true, true)
    );
    assert_eq!(
        (
            &clip(&stands, "crate").script,
            stands.frames.len(),
            stands.every
        ),
        (&None, 24, 1)
    );
    // A hundredth up, it floats by that, and the line that sets it down says where.
    let stands = read(&[level(), crate_at("y=0.29")].concat());
    let floats = only(&stands, "crate");
    assert!(
        matches!(floats.how, How::Floats(by) if near(by, 0.01)),
        "{floats:?}"
    );
    assert_eq!(floats.on.as_deref(), Some("floor"));
    assert_eq!(
        clip(&stands, "crate").script.as_deref(),
        Some("set crate y=0.3")
    );
    // A hundredth down, all of its foot is in the floor by that.
    let stands = read(&[level(), crate_at("y=0.31")].concat());
    let sunk = only(&stands, "crate");
    assert!(
        matches!(sunk.how, How::Sunk { deep, share } if near(deep, 0.01) && share == 1.0),
        "{sunk:?}"
    );
    assert_eq!(
        clip(&stands, "crate").script.as_deref(),
        Some("set crate y=0.3")
    );
    // Within a touch either way it rests, and nothing is to be done.
    for by in [-0.8 * TOUCH, 0.8 * TOUCH] {
        let stands = read(&[level(), crate_at(&format!("y={}", 0.3 + by))].concat());
        assert_eq!(only(&stands, "crate").how, How::Rests, "{by}");
        assert_eq!(clip(&stands, "crate").script, None);
    }
}

#[test]
fn a_clip_set_down_by_its_line_rests() {
    // Of each kind of clip, floating and sunk, on a floor that is level and on one that
    // leans: the document with the line run on it is read again, and the clip is down.
    let things = [
        ("mesh", "kind=box width=0.2 height=0.3 length=0.4 z=2.2"),
        ("mesh", "kind=ball width=0.3 height=0.3 length=0.3 z=2.2"),
        (
            "mesh",
            "kind=cylinder width=0.2 height=0.3 length=0.2 z=2.2 x=0.1",
        ),
        ("solid", "color=#ff0000 three_d=true scale=0.3 z=2.2"),
        (
            "solid",
            "color=#ff0000 three_d=true scale=0.3 depth=0.4 z=2.2 rotate_y=30",
        ),
        (
            "shape",
            "kind=rect width=0.9 height=0.22 fill=#c02020 three_d=true depth=0.4 z=2",
        ),
    ];
    let floors = [
        on_track(
            1,
            "road",
            "solid",
            "color=#404448 three_d=true rotate_x=-90 y=0.9 z=4 scale=16",
        ),
        tipped(),
    ];
    for floor in &floors {
        for (generator, set) in things {
            for y in [0.317, 0.5, 0.8631, 1.04] {
                let body = [
                    floor.clone(),
                    on_track(2, "thing", generator, &format!("{set} y={y}")),
                ]
                .concat();
                let before = project(&body);
                let stands = standing(&before, None, &[]).expect("reads");
                let Some(line) = clip(&stands, "thing").script.clone() else {
                    // Only one that is down, or too far up to have been meant to be, has no line.
                    let how = only(&stands, "thing").how;
                    assert!(
                        matches!(how, How::Rests | How::Air),
                        "{generator} {set} y={y}: {how:?}"
                    );
                    continue;
                };
                let after = apply_script(&before, Catalog::builtin(), &line)
                    .unwrap_or_else(|error| panic!("{line}: {error:?}"))
                    .project;
                let stands = standing(&after, None, &[]).expect("reads");
                let run = only(&stands, "thing");
                assert_eq!(
                    run.how,
                    How::Rests,
                    "{generator} {set} y={y}, then `{line}`: {run:?}"
                );
                assert_eq!(
                    (run.on.as_deref(), &clip(&stands, "thing").script),
                    (Some("road"), &None)
                );
            }
        }
    }
}

#[test]
fn on_the_skills_floor_a_foot_is_down_where_it_says_and_what_is_long_rests_on_its_far_side() {
    // The floor is 0.1 lower for each frame height nearer, and 1.0 where `z` is 1.6: a card
    // a fifth of the frame tall, its middle at 0.9 there, has its foot on it.
    let card = on_track(
        2,
        "card",
        "solid",
        "color=#ff0000 three_d=true scale=0.2 z=1.6 y=0.9",
    );
    let stands = read(&[tipped(), card].concat());
    let run = only(&stands, "card");
    assert_eq!((run.how, run.on.as_deref()), (How::Rests, Some("road")));
    assert!((run.slope - 6.0).abs() < 1e-6, "{run:?}");
    assert_eq!(stands.ground.as_deref(), Some("road"));
    // A box 0.4 long is level, and the floor is not: it is down on its far side, where the
    // floor is highest, and 0.4 of the floor's lean is open under its near one.
    let long = on_track(
        2,
        "car",
        "mesh",
        "kind=box width=0.3 height=0.2 length=0.4 z=1.8 y=0.86",
    );
    let stands = read(&[tipped(), long].concat());
    let run = only(&stands, "car");
    assert_eq!((run.how, run.on.as_deref()), (How::Rests, Some("road")));
    assert!((run.slope - 6.0).abs() < 1e-6, "{run:?}");
    assert!(
        (run.open - 0.4 * 6f64.to_radians().tan()).abs() < 1e-3,
        "{run:?}"
    );
    assert_eq!(clip(&stands, "car").script, None);
    // Put down by its middle, as the rule had it, its far half is in the floor.
    let long = on_track(
        2,
        "car",
        "mesh",
        "kind=box width=0.3 height=0.2 length=0.4 z=1.8 y=0.881",
    );
    let stands = read(&[tipped(), long].concat());
    let run = only(&stands, "car");
    assert!(
        matches!(run.how, How::Sunk { deep, share } if (deep - 0.021).abs() < 1e-3 && share == 0.5),
        "{run:?}"
    );
    assert_eq!(
        clip(&stands, "car").script.as_deref(),
        Some("set car y=0.86")
    );
}

#[test]
fn a_box_through_a_wall_is_said_and_one_before_it_is_not() {
    // A wall half the frame tall facing the viewer, on the floor, and a box on the floor.
    let scene = |z: f64| {
        [
            on_track(
                1,
                "floor",
                "solid",
                "color=#404040 three_d=true rotate_x=-90 y=0.25 z=1 scale=4",
            ),
            on_track(
                2,
                "wall",
                "solid",
                "color=#808080 three_d=true scale=0.5 z=1",
            ),
            on_track(
                3,
                "crate",
                "mesh",
                &format!("kind=box width=0.2 height=0.2 length=0.2 y=0.15 z={z}"),
            ),
        ]
        .concat()
    };
    let stands = read(&scene(1.0));
    let found = &clip(&stands, "crate").through;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].clip.as_str(), found[0].frames), ("wall", (0, 23)));
    // Half of it is on either side, its ends a tenth past the wall.
    assert!(
        near(found[0].deep, 0.1) && near(found[0].share, 0.5),
        "{found:?}"
    );
    // Each stands on the floor all the same, and the wall goes through nothing.
    assert_eq!(only(&stands, "crate").how, How::Rests);
    assert_eq!(only(&stands, "wall").how, How::Rests);
    assert!(clip(&stands, "wall").through.is_empty() && clip(&stands, "floor").through.is_empty());
    // A little way in, less of it is past, by less.
    let stands = read(&scene(0.93));
    let found = &clip(&stands, "crate").through;
    assert!(
        near(found[0].deep, 0.03) && near(found[0].share, 0.5),
        "{found:?}"
    );
    // Before the wall, touching it, and well before it, it goes through nothing.
    for z in [0.9, 0.7] {
        let stands = read(&scene(z));
        assert!(clip(&stands, "crate").through.is_empty(), "{z}");
    }
    // A sign across the wall goes through it, flat as both are; the wall, whose edges
    // pass the sign by, is not said to go through the sign.
    let sign = on_track(
        4,
        "sign",
        "solid",
        "color=#ff0000 three_d=true scale=0.3 rotate_y=90 z=1 y=0.1",
    );
    let stands = read(&[scene(0.5), sign].concat());
    assert_eq!(clip(&stands, "sign").through[0].clip, "wall");
    assert!(clip(&stands, "wall").through.is_empty());
    // A solid through it is said too, and one that stands on another goes through nothing.
    let post = on_track(
        4,
        "post",
        "solid",
        "color=#ff0000 three_d=true scale=0.1 depth=0.5 z=0.8 y=0.2",
    );
    let stands = read(&[scene(0.5), post].concat());
    assert_eq!(clip(&stands, "post").through[0].clip, "wall");
    assert!((clip(&stands, "post").through[0].deep - 0.2).abs() < 1e-6);
    let top = on_track(
        4,
        "top",
        "mesh",
        "kind=box width=0.1 height=0.1 length=0.1 y=0 z=0.5",
    );
    let stands = read(&[scene(0.5), top].concat());
    assert_eq!(only(&stands, "top").on.as_deref(), Some("crate"));
    assert!(clip(&stands, "top").through.is_empty() && clip(&stands, "crate").through.is_empty());
    // Two meshes in each other are not measured, and that is said.
    let other = on_track(
        4,
        "other",
        "mesh",
        "kind=box width=0.2 height=0.2 length=0.2 y=0.15 z=0.55 x=0.03",
    );
    let stands = read(&[scene(0.5), other].concat());
    assert!(clip(&stands, "other").through.is_empty());
    assert!(
        stands
            .not_measured
            .iter()
            .any(|said| said.contains("`crate` and `other`")),
        "{:?}",
        stands.not_measured
    );
}

#[test]
fn a_ball_in_the_air_is_in_the_air_and_the_floor_is_not() {
    let ball = |y: f64| {
        on_track(
            2,
            "ball",
            "mesh",
            &format!("kind=ball width=0.1 height=0.1 length=0.1 z=1 y={y}"),
        )
    };
    let stands = read(&[level(), ball(-0.05)].concat());
    let run = only(&stands, "ball");
    assert_eq!((run.how, run.on.as_deref()), (How::Air, Some("floor")));
    assert!(near(run.up.unwrap(), 0.4), "{run:?}");
    assert_eq!(clip(&stands, "ball").script, None);
    // Nothing is under the floor, and it is the ground, not a thing in the air.
    assert_eq!(only(&stands, "floor").how, How::Ground);
    assert_eq!(stands.ground.as_deref(), Some("floor"));
    // Within its own height of the floor it floats, and on the floor it rests.
    assert!(
        matches!(only(&read(&[level(), ball(0.3)].concat()), "ball").how, How::Floats(by) if near(by, 0.05))
    );
    assert_eq!(
        only(&read(&[level(), ball(0.35)].concat()), "ball").how,
        How::Rests
    );
    // Off the floor's edge there is nothing under it: in the air, over nothing.
    let stands = read(
        &[
            level(),
            on_track(
                2,
                "ball",
                "mesh",
                "kind=ball width=0.1 height=0.1 length=0.1 z=1 x=5",
            ),
        ]
        .concat(),
    );
    let run = only(&stands, "ball");
    assert_eq!((run.how, run.on.as_deref(), run.up), (How::Air, None, None));
}

/// A mesh a second long on track `track`, its middle at `z=1`.
fn mesh(track: u32, id: &str, set: &str) -> String {
    on_track(track, id, "mesh", &format!("{set} z=1"))
}

#[test]
fn what_stands_on_a_clip_and_reaches_into_it_is_sunk_and_the_clip_under_it_is_not() {
    // A snowman: a ball on the floor, a ball set two hundredths into its top, and a third
    // as far into that one's. The underside of each is inside the one under it, and is no
    // floor through it: the lower one stands as it stood, and the upper one is in it.
    let snowman = [
        level(),
        mesh(
            2,
            "base",
            "kind=ball width=0.3 height=0.3 length=0.3 y=0.25",
        ),
        mesh(
            3,
            "belly",
            "kind=ball width=0.2 height=0.2 length=0.2 y=0.02",
        ),
        mesh(
            4,
            "head",
            "kind=ball width=0.1 height=0.1 length=0.1 y=-0.11",
        ),
    ]
    .concat();
    let before = project(&snowman);
    let stands = standing(&before, None, &[]).expect("reads");
    let base = only(&stands, "base");
    assert_eq!(
        (base.how, base.on.as_deref()),
        (How::Rests, Some("floor")),
        "{base:?}"
    );
    assert_eq!(clip(&stands, "base").script, None);
    for (upper, lower) in [("belly", "base"), ("head", "belly")] {
        let run = only(&stands, upper);
        assert!(
            matches!(run.how, How::Sunk { deep, share } if near(deep, 0.02) && share == 1.0),
            "{upper}: {run:?}"
        );
        assert_eq!(run.on.as_deref(), Some(lower));
    }
    // Each set down by its line, from the lowest up, the whole of it rests.
    let mut after = before;
    for upper in ["belly", "head"] {
        let stands = standing(&after, None, &[]).expect("reads");
        let line = clip(&stands, upper).script.clone().expect("a line");
        after = apply_script(&after, Catalog::builtin(), &line)
            .expect("runs")
            .project;
    }
    let stands = standing(&after, None, &[]).expect("reads");
    for (id, on) in [("base", "floor"), ("belly", "base"), ("head", "belly")] {
        let run = only(&stands, id);
        assert_eq!(
            (run.how, run.on.as_deref()),
            (How::Rests, Some(on)),
            "{id}: {run:?}"
        );
    }

    // Two crates as wide as each other, the upper a hundredth into the lower: a line down
    // the lower one's corner runs down the upper one's edge, and says the same.
    let stack = [
        level(),
        mesh(2, "lower", "kind=box width=0.2 height=0.2 length=0.2 y=0.3"),
        mesh(
            3,
            "upper",
            "kind=box width=0.2 height=0.2 length=0.2 y=0.11",
        ),
    ]
    .concat();
    let stands = read(&stack);
    assert_eq!(
        only(&stands, "lower").how,
        How::Rests,
        "{:?}",
        clip(&stands, "lower")
    );
    let upper = only(&stands, "upper");
    assert!(
        matches!(upper.how, How::Sunk { deep, .. } if near(deep, 0.01)),
        "{upper:?}"
    );
    assert_eq!(upper.on.as_deref(), Some("lower"));

    // A roof wider than its walls and a clip with a `depth` are closed the same way, and
    // what hangs under a body, a leg a little way up into it, is not sunk in the body.
    let barn = [
        level(),
        mesh(2, "walls", "kind=box width=0.2 height=0.2 length=0.2 y=0.3"),
        on_track(
            3,
            "roof",
            "solid",
            "color=#802020 three_d=true scale=0.4 depth=0.3 z=0.85 y=0.01",
        ),
        mesh(
            4,
            "leg",
            "kind=box width=0.05 height=0.1 length=0.05 y=0.35 x=0.3",
        ),
        mesh(
            5,
            "body",
            "kind=box width=0.2 height=0.1 length=0.2 y=0.26 x=0.3",
        ),
    ]
    .concat();
    let stands = read(&barn);
    for id in ["walls", "leg"] {
        let run = only(&stands, id);
        assert_eq!(
            (run.how, run.on.as_deref()),
            (How::Rests, Some("floor")),
            "{id}: {run:?}"
        );
    }
    // The roof's foot is at 0.21, a hundredth under the walls' top.
    let roof = only(&stands, "roof");
    assert!(
        matches!(roof.how, How::Sunk { deep, .. } if near(deep, 0.01)),
        "{roof:?}"
    );
    assert_eq!(roof.on.as_deref(), Some("walls"));
    let body = only(&stands, "body");
    assert!(
        matches!(body.how, How::Sunk { deep, .. } if near(deep, 0.01)),
        "{body:?}"
    );
    assert_eq!(body.on.as_deref(), Some("leg"));

    // A clip that is in another from above is still sunk in it: a crate let half its
    // height into a block, the block's top being a floor through it.
    let block = [
        level(),
        mesh(2, "block", "kind=box width=0.4 height=0.2 length=0.4 y=0.3"),
        mesh(3, "crate", "kind=box width=0.1 height=0.1 length=0.1 y=0.2"),
    ]
    .concat();
    let stands = read(&block);
    let sunk = only(&stands, "crate");
    assert!(
        matches!(sunk.how, How::Sunk { deep, share } if near(deep, 0.05) && share == 1.0),
        "{sunk:?}"
    );
    assert_eq!(
        (sunk.on.as_deref(), only(&stands, "block").how),
        (Some("block"), How::Rests)
    );
}

#[test]
fn a_clip_wider_than_what_it_stands_on_stands_by_the_feet_that_are_over_it() {
    // A brim on a head it is wider than: of its feet only its middle is over the head, the
    // rest being round its rim with the floor far under them. Level with the head's top it
    // rests on it, and pressed a hundredth into it, it is sunk in it by that one foot and
    // is not in the air.
    let hat = |y: f64| {
        [
            level(),
            mesh(2, "head", "kind=box width=0.1 height=0.3 length=0.1 y=0.25"),
            mesh(
                3,
                "brim",
                &format!("kind=cylinder width=0.3 height=0.02 length=0.3 y={y}"),
            ),
        ]
        .concat()
    };
    let rests = read(&hat(0.09));
    let run = only(&rests, "brim");
    assert_eq!(
        (run.how, run.on.as_deref()),
        (How::Rests, Some("head")),
        "{run:?}"
    );
    let before = project(&hat(0.1));
    let stands = standing(&before, None, &[]).expect("reads");
    let run = only(&stands, "brim");
    assert!(
        matches!(run.how, How::Sunk { deep, share } if near(deep, 0.01) && share < 0.1),
        "{run:?}"
    );
    assert_eq!(run.on.as_deref(), Some("head"));
    let line = clip(&stands, "brim").script.clone().expect("a line");
    assert_eq!(line, "set brim y=0.09");
    let after = apply_script(&before, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    assert_eq!(
        only(&standing(&after, None, &[]).expect("reads"), "brim").how,
        How::Rests
    );
    // A corner that dips where the rest of it is on the floor is still passed over: a ball
    // a touch and a half into the floor has its lowest corner in it and its lowest ring on it.
    let stands = read(
        &[
            level(),
            mesh(
                2,
                "ball",
                "kind=ball width=0.3 height=0.3 length=0.3 y=0.2545",
            ),
        ]
        .concat(),
    );
    let ball = only(&stands, "ball");
    assert!(!matches!(ball.how, How::Air), "{ball:?}");
}

#[test]
fn a_shapes_floor_is_as_large_as_its_numbers_and_not_its_frame() {
    // A floor six frame heights across and eight long, its middle three back, the upper half of its
    // picture its near half: where the frame cut it, it went from 2.5 to 3.5 back, and a phone at
    // the front of it was in the air.
    let floor = |set: &str| {
        let set = format!("width=6 height=8 fill=#4080ff three_d=true rotate_x=90 {set} z=3");
        [
            on_track(1, "floor", "shape", &set),
            on_track(
                2,
                "phone",
                "shape",
                "width=0.2 height=0.4 three_d=true y=0.15 z=0",
            ),
            on_track(8, "cam", "camera", "z=0"),
        ]
        .concat()
    };
    let stands = read(&floor("y=0.35"));
    let run = only(&stands, "phone");
    assert_eq!(
        (run.how, run.on.as_deref()),
        (How::Rests, Some("floor")),
        "{run:?}"
    );
    // Its crop cuts it where it says on the frame, and what is left goes where the crop puts it,
    // a quarter of a frame up or down, which its `y` takes back. With the lower half of its frame
    // cut, what is left of it reaches from its middle as far forward as it goes, under the phone.
    let stands = read(&floor("crop_b=0.5 y=0.1"));
    let run = only(&stands, "phone");
    assert_eq!(
        (run.how, run.on.as_deref()),
        (How::Rests, Some("floor")),
        "{run:?}"
    );
    // With the upper half cut, it reaches from its middle back, and the phone is before it.
    let stands = read(&floor("crop_t=0.5 y=0.6"));
    let run = only(&stands, "phone");
    assert!(matches!(run.how, How::Air), "{run:?}");
}

#[test]
fn what_moves_along_a_floor_rests_all_the_way_and_where_it_lifts_is_the_run_said() {
    let along = crate_at("y=0.3 x={0f:-0.3 23f:0.3}");
    let stands = read(&[level(), along].concat());
    let run = only(&stands, "crate");
    assert_eq!((run.how, run.frames), (How::Rests, (0, 23)));
    // It lifts off between frames 8 and 16, by a twentieth at the most on frame 12.
    let lifts = crate_at("x={0f:-0.3 23f:0.3} y={0f:0.3 8f:0.3 12f:0.25 16f:0.3 23f:0.3}");
    let before = project(&[level(), lifts].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    let runs = &clip(&stands, "crate").runs;
    let said: Vec<(u8, (i64, i64))> = runs
        .iter()
        .map(|run| (matches!(run.how, How::Floats(_)) as u8, run.frames))
        .collect();
    assert_eq!(said, [(0, (0, 8)), (1, (9, 15)), (0, (16, 23))], "{runs:?}");
    assert!(
        matches!(runs[1].how, How::Floats(by) if near(by, 0.05)) && runs[1].worst == 12,
        "{runs:?}"
    );
    // One `y` keeps it down on every frame, and its way across is left as it was.
    let line = clip(&stands, "crate").script.clone().expect("a line");
    assert_eq!(line, "set crate y=0.3");
    let after = apply_script(&before, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    assert_eq!(
        only(&standing(&after, None, &[]).expect("reads"), "crate").how,
        How::Rests
    );

    // Going away over a floor that rises, a clip that holds its `y` goes into it, and what
    // sets it down is a key at each end, a floor being flat.
    let away = on_track(
        2,
        "crate",
        "mesh",
        "kind=box width=0.2 height=0.2 length=0.2 y=0.86 z={0f:1.9 23f:2.9}",
    );
    let before = project(&[tipped(), away].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    let runs = &clip(&stands, "crate").runs;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(
        matches!(runs[1].how, How::Sunk { share, .. } if share == 1.0) && runs[1].worst == 23,
        "{runs:?}"
    );
    let line = clip(&stands, "crate").script.clone().expect("a line");
    assert_eq!(line, "set crate y={0f:0.86 23f:0.755}");
    let after = apply_script(&before, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    let stands = standing(&after, None, &[]).expect("reads");
    assert_eq!(
        (only(&stands, "crate").how, &clip(&stands, "crate").script),
        (How::Rests, &None)
    );

    // A clip that starts later has its keys counted from its own first frame.
    let later = "  track V2 video\n    clip crate gen=mesh@1 at=6f dur=18f\n      set kind=box width=0.2 height=0.2 length=0.2 y=0.86 z={0f:1.9 17f:2.9}\n";
    let late = project(&[tipped(), later.to_owned()].concat());
    let stands = standing(&late, None, &[]).expect("reads");
    assert_eq!(clip(&stands, "crate").runs[0].frames.0, 6);
    let line = clip(&stands, "crate").script.clone().expect("a line");
    assert_eq!(line, "set crate y={0f:0.86 17f:0.755}");
    let set = apply_script(&late, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    assert_eq!(
        only(&standing(&set, None, &[]).expect("reads"), "crate").how,
        How::Rests
    );

    // Read on a few frames alone, what moves has no one `y`, and no keys are made up
    // between them: it says to read every frame.
    let rate = Rate::FPS_24;
    let stands = standing(&before, None, &[rate.time_of(0), rate.time_of(23)]).expect("reads");
    let stood = clip(&stands, "crate");
    assert_eq!(
        (stands.frames.as_slice(), &stood.script),
        ([0i64, 23].as_slice(), &None)
    );
    assert!(
        stood
            .cannot
            .as_deref()
            .is_some_and(|why| why.contains("no `at`")),
        "{stood:?}"
    );
    // What holds still is set down from a few frames as from all of them.
    let still = project(&[level(), crate_at("y=0.25")].concat());
    let stands = standing(&still, None, &[rate.time_of(3), rate.time_of(20)]).expect("reads");
    assert_eq!(
        clip(&stands, "crate").script.as_deref(),
        Some("set crate y=0.3")
    );
    assert_eq!(only(&stands, "crate").frames, (3, 20));
}

/// A level road 0.4 under the middle of the frame, 81 frames long.
fn road() -> String {
    "  track V1 video\n    clip road gen=solid@1 at=0f dur=81f\n      set color=#404448 three_d=true rotate_x=-90 y=0.4 z=4 scale=16\n".to_owned()
}

/// A part of a thing on that road: a mesh `x` across from the thing's middle and `off` it in
/// `z`, in frame heights. The thing is a little right of the frame's middle, and where it
/// `moves` it goes away down the road, each part by the same keys but for its own `off`.
fn part(moves: bool, track: u32, id: &str, set: &str, (x, off): (f64, f64)) -> String {
    // A line's `x` is in frame widths, of a frame 16 by 9.
    let x = (x + 0.07) * 9.0 / 16.0;
    let z = if moves {
        format!("{{0f:{} 80f:{}}}", 1.3 + off, 6.1 + off)
    } else {
        format!("{}", 1.3 + off)
    };
    format!("  track V{track} video\n    clip {id} gen=mesh@1 at=0f dur=81f\n      set {set} x={x} z={z}\n")
}

#[test]
fn what_is_as_near_to_two_clips_is_on_the_lower_tracks_on_every_frame() {
    // A plank across two blocks, each turned a little, the three going away down a road
    // together. Its ends are as far over the one block as over the other but for the last
    // places of the two numbers, and which is the less is another from frame to frame: it
    // rests on the block of the lower track on all of them, whichever side that one is on,
    // as it does parked.
    let block = |y: f64| format!("kind=box width=0.1 height=0.1 length=0.1 rotate_y=30 y={y}");
    let plank = |moves: bool, one: (f64, f64), other: (f64, f64)| {
        let scene = [
            road(),
            part(moves, 2, "one", &block(one.1), (one.0, 0.0)),
            part(moves, 3, "other", &block(other.1), (other.0, 0.0)),
            part(
                moves,
                4,
                "plank",
                "kind=box width=0.5 height=0.02 length=0.06 y=0.29",
                (0.0, 0.0),
            ),
        ]
        .concat();
        standing(&project(&scene), None, &[]).expect("reads")
    };
    let rests_on = |stands: &Standing| {
        let run = only(stands, "plank");
        assert_eq!((run.how, run.frames), (How::Rests, (0, 80)), "{run:?}");
        run.on.clone().expect("a block")
    };
    for moves in [true, false] {
        for (one, other) in [(-0.25, 0.25), (0.25, -0.25)] {
            assert_eq!(
                rests_on(&plank(moves, (one, 0.35), (other, 0.35))),
                "one",
                "{moves} {one}"
            );
        }
        // A block written a thousandth lower is not as near, nor is one a touch lower, which
        // a touch either way of would be on some frames and not on others: the plank is on
        // the block that was left, whichever its track.
        for lower in [0.001, TOUCH] {
            assert_eq!(
                rests_on(&plank(moves, (-0.25, 0.35 + lower), (0.25, 0.35))),
                "other",
                "{moves} {lower}"
            );
            assert_eq!(
                rests_on(&plank(moves, (-0.25, 0.35), (0.25, 0.35 + lower))),
                "one",
                "{moves} {lower}"
            );
        }
    }

    // A cart: an axle through a wheel at each end, a bed on the two axles, a board along
    // each side of the bed, and one across its back that goes down into those two.
    let cart = |moves: bool| {
        let wheel = "kind=cylinder rotation=90 width=0.2 height=0.06 length=0.2 y=0.3";
        let axle = "kind=box width=0.5 height=0.03 length=0.03 y=0.3";
        let side = "kind=box width=0.03 height=0.14 length=0.6 y=0.175";
        let scene = [
            road(),
            part(moves, 2, "wheel_lf", wheel, (-0.25, 0.3)),
            part(moves, 3, "wheel_rf", wheel, (0.25, 0.3)),
            part(moves, 4, "wheel_lb", wheel, (-0.25, -0.3)),
            part(moves, 5, "wheel_rb", wheel, (0.25, -0.3)),
            part(moves, 6, "axle_f", axle, (0.0, 0.3)),
            part(moves, 7, "axle_b", axle, (0.0, -0.3)),
            part(
                moves,
                8,
                "bed",
                "kind=box width=0.4 height=0.04 length=0.6 y=0.265",
                (0.0, 0.0),
            ),
            part(moves, 9, "side_l", side, (-0.185, 0.0)),
            part(moves, 10, "side_r", side, (0.185, 0.0)),
            part(
                moves,
                11,
                "back",
                "kind=box width=0.37 height=0.2 length=0.03 y=0.145",
                (0.0, -0.285),
            ),
        ]
        .concat();
        standing(&project(&scene), None, &[]).expect("reads")
    };
    for moves in [true, false] {
        let stands = cart(moves);
        assert_eq!(stands.frames.len(), 81);
        // Each axle's ends are inside its wheels, whose undersides are no floor under it:
        // what it is over is the road, on every frame, and neither wheel.
        for axle in ["axle_f", "axle_b"] {
            let run = only(&stands, axle);
            assert_eq!(
                (run.how, run.on.as_deref(), run.frames),
                (How::Air, Some("road"), (0, 80)),
                "{moves}"
            );
        }
        // The bed is on both axles, and the back board as deep in each side board.
        let run = only(&stands, "bed");
        assert_eq!(
            (run.how, run.on.as_deref()),
            (How::Rests, Some("axle_f")),
            "{moves}"
        );
        let run = only(&stands, "back");
        assert!(
            matches!(run.how, How::Sunk { deep, share } if near(deep, 0.14) && share == 1.0),
            "{moves} {run:?}"
        );
        assert_eq!(run.on.as_deref(), Some("side_l"), "{moves}");
        // Every part is said once, and so is what is not measured of the board it is not in.
        for stood in &stands.clips {
            assert_eq!(
                (stood.runs.len(), stood.through.len()),
                (1, 0),
                "{moves} {stood:?}"
            );
        }
        assert_eq!(
            stands.not_measured,
            ["whether `side_r` and `back` go into each other: a mesh into another mesh"],
            "{moves}"
        );
    }
}

/// A glTF file of a sheet 2 across and 1 up standing on its ground, which its one animation,
/// `lift`, takes as far up as it is tall in a second.
fn lifted_sheet(folder: &Path) -> PathBuf {
    let floats = |values: &[f32]| {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>()
    };
    let parts: [(Vec<u8>, &str); 4] = [
        (
            floats(&[-1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0]),
            "\"componentType\":5126,\"count\":4,\"type\":\"VEC3\"",
        ),
        (
            [0u16, 1, 2, 0, 2, 3]
                .iter()
                .flat_map(|index| index.to_le_bytes())
                .collect(),
            "\"componentType\":5123,\"count\":6,\"type\":\"SCALAR\"",
        ),
        (
            floats(&[0.0, 1.0]),
            "\"componentType\":5126,\"count\":2,\"type\":\"SCALAR\"",
        ),
        (
            floats(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            "\"componentType\":5126,\"count\":2,\"type\":\"VEC3\"",
        ),
    ];
    let (mut bytes, mut views, mut accessors) = (Vec::new(), Vec::new(), Vec::new());
    for (index, (part, kind)) in parts.iter().enumerate() {
        views.push(format!(
            "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{}}}",
            bytes.len(),
            part.len()
        ));
        accessors.push(format!("{{\"bufferView\":{index},{kind}}}"));
        bytes.extend(part);
    }
    let mut json = format!(
        concat!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],",
            "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}}],",
            "\"animations\":[{{\"name\":\"lift\",\"channels\":[{{\"sampler\":0,\"target\":{{\"node\":0,\"path\":\"translation\"}}}}],",
            "\"samplers\":[{{\"input\":2,\"output\":3}}]}}],",
            "\"buffers\":[{{\"byteLength\":{}}}],\"bufferViews\":[{}],\"accessors\":[{}]}}",
        ),
        bytes.len(),
        views.join(","),
        accessors.join(",")
    )
    .into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut file = Vec::new();
    file.extend(b"glTF");
    file.extend(2u32.to_le_bytes());
    file.extend((12 + 8 + json.len() as u32 + 8 + bytes.len() as u32).to_le_bytes());
    file.extend((json.len() as u32).to_le_bytes());
    file.extend(b"JSON");
    file.extend(&json);
    file.extend((bytes.len() as u32).to_le_bytes());
    file.extend(b"BIN\0");
    file.extend(&bytes);
    let path = folder.join("sheet.glb");
    std::fs::write(&path, file).expect("a file");
    path
}

#[test]
fn a_files_mesh_stands_by_its_feet_as_its_animation_has_them() {
    let folder = tempfile::tempdir().expect("a folder");
    lifted_sheet(folder.path());
    // Half the frame tall about the frame's middle, its foot is a quarter under it, where
    // the floor is.
    let scene = |set: &str| {
        let mut project = project(
            &[
                on_track(
                    1,
                    "floor",
                    "solid",
                    "color=#404040 three_d=true rotate_x=-90 y=0.25 z=1 scale=4",
                ),
                on_track(
                    2,
                    "walker",
                    "mesh",
                    &format!("file=\"sheet.glb\" height=0.5 z=1 {set}"),
                ),
            ]
            .concat(),
        );
        project.resolve_paths(Catalog::builtin(), folder.path());
        project
    };
    // At rest it stands on every frame.
    let stands = standing(&scene(""), None, &[]).expect("reads");
    let run = only(&stands, "walker");
    assert_eq!(
        (run.how, run.on.as_deref(), run.frames),
        (How::Rests, Some("floor"), (0, 23))
    );
    // Playing, it is down on its first frame and has gone up since: half a second in, by
    // half its height.
    let rate = Rate::FPS_24;
    let playing = scene("animation=\"lift\"");
    let stands = standing(&playing, None, &[rate.time_of(0), rate.time_of(12)]).expect("reads");
    let runs = &clip(&stands, "walker").runs;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!((runs[0].how, runs[0].frames), (How::Rests, (0, 0)));
    assert!(
        matches!(runs[1].how, How::Floats(by) if near(by, 0.25)) && runs[1].frames == (12, 12),
        "{runs:?}"
    );
    // And as its `seconds` has it: held half a second in, it floats on every frame.
    let stands = standing(&scene("animation=\"lift\" seconds=0.5"), None, &[]).expect("reads");
    assert!(matches!(only(&stands, "walker").how, How::Floats(by) if near(by, 0.25)));
    assert_eq!(
        clip(&stands, "walker").script.as_deref(),
        Some("set walker y=0.25")
    );
}

#[test]
fn a_clip_with_a_parent_is_set_down_by_its_own_y_where_that_can_be() {
    let floor = on_track(
        1,
        "floor",
        "solid",
        "color=#404040 three_d=true rotate_x=-90 y=0.5 z=1 scale=4",
    );
    let child = |rig: &str, y: f64| {
        format!(
            concat!(
                "  track V2 video\n    clip rig gen=null@1 at=0f dur=24f\n      set {rig}\n",
                "  track V3 video\n    clip crate gen=mesh@1 at=0f dur=24f parent=rig\n",
                "      set kind=box width=0.2 height=0.2 length=0.2 z=1 y={y}\n",
            ),
            rig = rig,
            y = y
        )
    };
    // A parent twice the size has it twice as large and twice as far from the middle: its
    // foot is at twice its `y` and two tenths, and a tenth of its own `y` is two of the world's.
    let before = project(&[floor.clone(), child("scale=2", 0.1)].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    assert!(matches!(only(&stands, "crate").how, How::Floats(by) if near(by, 0.1)));
    let line = clip(&stands, "crate").script.clone().expect("a line");
    assert_eq!(line, "set crate y=0.15");
    let after = apply_script(&before, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    assert_eq!(
        only(&standing(&after, None, &[]).expect("reads"), "crate").how,
        How::Rests
    );
    // Moved by its parent, it is where the parent has it, and its own `y` is still its own.
    let before = project(&[floor.clone(), child("y=0.1", 0.25)].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    assert!(matches!(only(&stands, "crate").how, How::Floats(by) if near(by, 0.05)));
    assert_eq!(
        clip(&stands, "crate").script.as_deref(),
        Some("set crate y=0.3")
    );
    // Turned by its parent, its own `y` moves it across as well, and that is said.
    let stands = read(&[floor, child("rotation=30", 0.3)].concat());
    let stood = clip(&stands, "crate");
    assert!(
        matches!(only(&stands, "crate").how, How::Floats(_)),
        "{stood:?}"
    );
    assert!(
        stood.script.is_none()
            && stood
                .cannot
                .as_deref()
                .is_some_and(|why| why.contains("parent")),
        "{stood:?}"
    );
    // The null it moves with is no thing, and stands nowhere.
    assert!(stands.clips.iter().all(|stood| stood.clip != "rig"));
}

#[test]
fn a_clip_a_parent_takes_into_depth_stands_where_it_is_drawn() {
    // A step no wider than the box, two frame heights back, and nothing else under anything.
    let step = on_track(
        1,
        "step",
        "solid",
        "color=#404040 three_d=true rotate_x=-90 y=0.5 z=2 scale=0.5",
    );
    let child = |rig: &str, set: &str| {
        format!(
            concat!(
                "  track V2 video\n    clip rig gen=null@1 at=0f dur=24f\n      set {rig}\n",
                "  track V3 video\n    clip crate gen=mesh@1 at=0f dur=24f parent=rig\n",
                "      set kind=box width=0.2 height=0.2 length=0.2 {set}\n",
            ),
            rig = rig,
            set = set
        )
    };
    // Where its own numbers have it, the box is over nothing.
    let stands = read(&[step.clone(), child("x=0", "y=0.3")].concat());
    assert!(
        !matches!(only(&stands, "crate").how, How::Floats(_) | How::Rests),
        "{stands:?}"
    );
    // Its parent moved back, it is over the step, by the tenth its foot is short of it, and
    // its own `y` still sets it down.
    let before = project(&[step.clone(), child("z=2", "y=0.3")].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    let floats = only(&stands, "crate");
    assert!(
        matches!(floats.how, How::Floats(by) if near(by, 0.1)),
        "{floats:?}"
    );
    assert_eq!(floats.on.as_deref(), Some("step"));
    let line = clip(&stands, "crate").script.clone().expect("a line");
    assert_eq!(line, "set crate y=0.4");
    let after = apply_script(&before, Catalog::builtin(), &line)
        .expect("runs")
        .project;
    assert_eq!(
        only(&standing(&after, None, &[]).expect("reads"), "crate").how,
        How::Rests
    );
    // Swung round its parent's upright from a frame height to the right of it, two of them
    // back: down is still down, so it is set down the same way. The frame is 320 by 180.
    let before = project(&[step.clone(), child("rotate_y=-90", "x=1.125 y=0.35")].concat());
    let stands = standing(&before, None, &[]).expect("reads");
    let floats = only(&stands, "crate");
    assert!(
        matches!(floats.how, How::Floats(by) if near(by, 0.05)),
        "{floats:?}"
    );
    assert_eq!(floats.on.as_deref(), Some("step"));
    assert_eq!(
        clip(&stands, "crate").script.as_deref(),
        Some("set crate y=0.4")
    );
    // With its parent a quarter of the way round it is beside the step again.
    let stands = read(&[step.clone(), child("rotate_y=-45", "x=1.125 y=0.35")].concat());
    assert!(
        !matches!(only(&stands, "crate").how, How::Floats(_) | How::Rests),
        "{stands:?}"
    );
    // Tipped by its parent, its own `y` moves it back as well as down, and that is said.
    let stands = read(&[step, child("z=2 rotate_x=10", "y=0.3")].concat());
    let stood = clip(&stands, "crate");
    assert!(
        matches!(only(&stands, "crate").how, How::Floats(_)),
        "{stood:?}"
    );
    assert!(
        stood.script.is_none()
            && stood
                .cannot
                .as_deref()
                .is_some_and(|why| why.contains("parent")),
        "{stood:?}"
    );
}

#[test]
fn only_what_is_a_thing_in_depth_stands_and_the_rest_is_said_not_to_be_measured() {
    let scene = [
        on_track(10, "sky", "solid", "color=#88aaff"),
        level(),
        crate_at("y=0.3"),
        on_track(3, "words", "title", "text=\"HI\" three_d=true z=1"),
        on_track(
            4,
            "wheel",
            "shape",
            "kind=ellipse width=0.2 height=0.2 fill=#202020 three_d=true depth=0.1 z=1 x=0.2 y=0.3",
        ),
        on_track(5, "tune", "adjustment", "three_d=true"),
        on_track(6, "rig", "null", "three_d=true"),
        on_track(
            7,
            "ghost",
            "mesh",
            "kind=box width=0.2 height=0.2 length=0.2 z=1 x=-0.3 opacity=0",
        ),
    ]
    .concat();
    let stands = read(&scene);
    // From the lowest track up: a camera, a light, a null, an adjustment layer, a clip out
    // of depth, and one that is wholly clear are none of them.
    let said: Vec<&str> = stands
        .clips
        .iter()
        .map(|stood| stood.clip.as_str())
        .collect();
    assert_eq!(said, ["floor", "crate", "wheel"]);
    assert_eq!(stands.ground.as_deref(), Some("floor"));
    // A round shape is measured as the box around it, and stands on its lowest line.
    assert_eq!(
        (
            only(&stands, "wheel").how,
            only(&stands, "wheel").on.as_deref()
        ),
        (How::Rests, Some("floor"))
    );
    let not = stands.not_measured.join("\n");
    assert!(
        not.contains("the outline of `wheel` within the box around it"),
        "{not}"
    );
    assert!(not.contains("where `words` stands"), "{not}");
    assert!(not.contains("`sky`, out of depth"), "{not}");
    assert!(
        !not.contains("crate") && !not.contains("cam") && !not.contains("ghost"),
        "{not}"
    );
}

#[test]
fn a_sequence_of_any_size_says_the_same() {
    // Nothing here is on a whole pixel of any of them, and a clip that holds still is
    // drawn on whole pixels: what is read is where the document puts it.
    let scene = [
        tipped(),
        on_track(2, "body", "shape", "kind=rect width=0.9 height=0.22 fill=#c02020 three_d=true depth=0.4 x=-0.1337 y=0.8431 z=2"),
        on_track(3, "crate", "mesh", "kind=box width=0.213 height=0.217 length=0.4 x=0.3 y=0.7719 z=2.3 rotate_y=20"),
        on_track(4, "card", "solid", "color=#ff0000 three_d=true scale=0.2 z=1.6 y=0.8873 x=-0.31"),
        on_track(5, "ball", "mesh", "kind=ball width=0.1 height=0.1 length=0.1 z=3 y={0f:0.2 23f:0.9}"),
    ]
    .concat();
    let read_at = |size: &str| standing(&project_of(size, &scene), None, &[]).expect("reads");
    // To a millionth of the frame: a shape's outline is kept in 32 bits, of its pixels.
    let rounded = |value: f64| (value * 1e6).round() / 1e6;
    let said = |standing: &Standing| -> Vec<String> {
        standing
            .clips
            .iter()
            .map(|stood| {
                let runs: Vec<String> = stood
                    .runs
                    .iter()
                    .map(|run| {
                        let how = match run.how {
                            How::Floats(by) => format!("floats {}", rounded(by)),
                            How::Sunk { deep, share } => {
                                format!("sunk {} {}", rounded(deep), rounded(share))
                            }
                            other => format!("{other:?}"),
                        };
                        format!(
                            "{how} on {:?} {} {} {:?} {}",
                            run.on,
                            rounded(run.slope),
                            rounded(run.open),
                            run.frames,
                            run.worst
                        )
                    })
                    .collect();
                format!(
                    "{}: {} | {:?} {:?}",
                    stood.clip,
                    runs.join("; "),
                    stood.script,
                    stood.cannot
                )
            })
            .collect()
    };
    let small = read_at("320x180");
    // Something of each kind is said, or this would hold of nothing.
    let text = said(&small).join("\n");
    assert!(
        text.contains("sunk") && text.contains("floats") && text.contains("Air"),
        "{text}"
    );
    for size in ["1920x1080", "832x468", "1664x936"] {
        assert_eq!(said(&read_at(size)), said(&small), "{size}");
    }
}

fn fixture(name: &str) -> Project {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/shots")
        .join(name);
    let text = std::fs::read_to_string(folder.join("project.kerf")).expect("the fixture");
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{name}: {errors:?}"))
        .project
}

#[test]
fn what_the_fixtures_scenes_stand_on_is_what_was_found_of_them() {
    // The car on a road, as it is kept: its takes are of this guide, so what is found is
    // written down here and the scene is left as it is. The body's foot is level at 0.95
    // and the road leans, 0.960 under its front and 0.918 under its back: its far half is in
    // the road by 0.032, which the road hides in a depth pass, and what is drawn of a car
    // is as tall as what shows of it.
    let car = fixture("car");
    for (sequence, body, cabin, road) in [
        ("s2", "body", "cabin", "road"),
        ("s3", "parked_body", "parked_cabin", "parked_road"),
    ] {
        let stands = standing(&car, Some(sequence), &[]).expect("reads");
        assert_eq!(
            (stands.frames.len(), stands.ground.as_deref()),
            (96, Some(road))
        );
        let run = only(&stands, body);
        assert!(
            matches!(run.how, How::Sunk { deep, share } if (deep - 0.032).abs() < 5e-4 && share == 0.5),
            "{run:?}"
        );
        // It is as deep on every frame, so its worst is its first.
        assert_eq!(
            (run.on.as_deref(), run.frames, run.worst),
            (Some(road), (0, 95), 0)
        );
        assert!(
            (run.slope - 6.0).abs() < 1e-6 && (run.open - 0.042).abs() < 5e-4,
            "{run:?}"
        );
        assert_eq!(
            clip(&stands, body).script.as_deref(),
            Some(format!("set {body} y=0.808").as_str())
        );
        // The cabin is on the body, and would have to follow it up.
        let run = only(&stands, cabin);
        assert_eq!((run.how, run.on.as_deref()), (How::Rests, Some(body)));
        assert!(
            stands.clips.iter().all(|stood| stood.through.is_empty()),
            "{stands:?}"
        );
        // Both are shapes with a depth, plain rectangles, so their boxes are what they are.
        let sky = if sequence == "s2" {
            "sky"
        } else {
            "parked_sky"
        };
        assert_eq!(
            stands.not_measured,
            [format!("`{sky}`, out of depth: it is nowhere in the scene")]
        );
    }
    // A walk in layers has nothing in depth: nothing stands, and that is what is said.
    let walk = fixture("walk");
    for (sequence, flat) in [
        ("s1", "`back`, `words`, `front`"),
        ("s2", "`plate`, `mover`"),
    ] {
        let stands = standing(&walk, Some(sequence), &[]).expect("reads");
        assert!(
            stands.clips.is_empty() && stands.ground.is_none(),
            "{stands:?}"
        );
        assert_eq!(
            stands.not_measured,
            [format!("{flat}, out of depth: it is nowhere in the scene")]
        );
    }
}

#[test]
fn what_the_scenes_of_solids_stand_on_is_what_was_found_of_them() {
    // Things stacked from solids, as an agent blocked them for a model to film
    // (`fixtures/shots/solids`): each part a little way into the one under it, or hanging
    // under it. The lower part of each was said to be sunk in the upper, a snowman's base
    // 0.494 in its belly, with the line that would have set it on top of its belly.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/shots/solids");
    let mut solids = fixture("solids");
    // Three of its things are meshes of files, which are read from where the paths say.
    solids.resolve_paths(Catalog::builtin(), &folder);
    let first = [Rate::new(16, 1).expect("a rate").time_of(0)];
    let reads = |sequence: &str| standing(&solids, Some(sequence), &first).expect("reads");
    let rests = |stands: &Standing, id: &str, on: &str| {
        let run = only(stands, id);
        assert_eq!(
            (run.how, run.on.as_deref()),
            (How::Rests, Some(on)),
            "{id}: {run:?}"
        );
    };
    let sunk = |stands: &Standing, id: &str, on: &str, by: f64| {
        let run = only(stands, id);
        assert!(
            matches!(run.how, How::Sunk { deep, .. } if (deep - by).abs() < 5e-4),
            "{id}: {run:?}"
        );
        assert_eq!(run.on.as_deref(), Some(on), "{id}");
    };

    // A snowman: its base is on the snow, and each part above is in the one under it by
    // what it was written to reach in. The brim is wider than the head, on which it stands
    // by its middle alone.
    let snowman = reads("snowman");
    assert_eq!(snowman.ground.as_deref(), Some("snow_ground"));
    rests(&snowman, "snow_base", "snow_ground");
    sunk(&snowman, "snow_belly", "snow_base", 0.07);
    sunk(&snowman, "snow_head", "snow_belly", 0.04);
    sunk(&snowman, "snow_brim", "snow_head", 0.025);
    sunk(&snowman, "snow_hat", "snow_brim", 0.005);
    assert_eq!(clip(&snowman, "snow_base").script, None);
    assert_eq!(
        clip(&snowman, "snow_belly").script.as_deref(),
        Some("set snow_belly y=-0.27")
    );

    // A barn under a roof that is a box turned on its edge, and a cow whose legs go a
    // little way up into its body: the barn and the legs are on the field.
    let saucer = reads("saucer");
    rests(&saucer, "barn", "field");
    assert_eq!(only(&saucer, "barn_roof").on.as_deref(), Some("barn"));
    for leg in ["cow_leg0", "cow_leg1", "cow_leg2", "cow_leg3"] {
        rests(&saucer, leg, "field");
    }

    // A lighthouse: the lantern is on the gallery under it, and the cap is the one in the
    // lantern. A robot's legs are on the floor of the alley and not in its body.
    let lighthouse = reads("lighthouse");
    rests(&lighthouse, "lantern", "gallery");
    sunk(&lighthouse, "cap", "lantern", 0.12);
    let robot = reads("robot");
    rests(&robot, "bot_leg_l", "alley_floor");
    rests(&robot, "bot_leg_r", "alley_floor");
    rests(&robot, "bot_antenna", "bot_head");

    // The files' meshes stand as any do: the rocket on its pad, the donut on its plate.
    rests(&reads("rocket"), "ship", "pad");
    rests(&reads("donut"), "ring_cake", "plate");
}
