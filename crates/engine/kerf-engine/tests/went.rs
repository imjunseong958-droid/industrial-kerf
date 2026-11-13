//! Where a take took a clip's subject, beside where its guide has it.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_engine::went::{Going, Went};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project, Rate};

/// A sky out of depth; in depth a road tipped back, and a car that drives across it.
fn scene(car: &str) -> Project {
    let text = format!(
        concat!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n",
            "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
            "  track V2 video\n    clip road gen=solid@1 at=0f dur=24f\n",
            "      set color=#404040 three_d=true rotate_x=-84 y=0.75 z=4 scale=16\n",
            "  track V3 video\n    clip car gen=shape@1 at=0f dur=24f\n",
            "      set kind=rect width=0.5 height=0.3 fill=#c02020 three_d=true depth=0.4 y=0.8 {car}\n",
            "  track V4 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
        ),
        car = car,
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// Where `clip` went in a take whose picture is what `take` draws.
fn through(guide: &Project, take: &Project, clip: &str) -> Went {
    let mut going = Going::new(guide, None, clip).expect("a guide");
    let drawn = Compositor::new(take, Catalog::builtin(), &ComposeOptions::default())
        .expect("a take");
    let (width, height) = going.size();
    for frame in 0..24 {
        let at = Rate::FPS_24.time_of(frame);
        let picture = drawn.frame(at, f64::from(height) / 180.0);
        assert_eq!((picture.width, picture.height), (width, height));
        going.frame(at, &picture.rgba);
    }
    going.done().expect("frames")
}

#[test]
fn a_take_that_took_the_subject_elsewhere_says_how_far_and_which_way() {
    let guide = scene("z=2 x={0f:-0.3 24f:0.3}");
    // The take that followed, and one that set the car down where the guide has it and then
    // left it there.
    let right = through(&guide, &guide, "car");
    let still = through(&guide, &scene("z=2 x=-0.3"), "car");
    assert_eq!(right.clip, "car");
    assert_eq!(right.frames, 24);
    assert_eq!(right.lost, None);
    // A take that has the subject where the guide has it is off by a pixel or two of the
    // small frame it is read on, and drew it about the same size: a cut takes in the edge it
    // is let look at, so it comes out a little wider than the box the ids pass gives.
    assert!(right.off < 0.02, "{right:?}");
    assert!((right.wide - 1.0).abs() < 0.2, "{right:?}");
    // The take that left the car behind is off by half of what the guide moved it, on
    // average, and worst at the end, where the guide has it a frame's width away.
    assert_eq!(still.frames, 24);
    assert_eq!(still.lost, None);
    assert!(still.off > 0.1 && still.off < 0.4, "{still:?}");
    assert!(still.worst.0 > 19 && still.worst.1 > 0.2, "{still:?}");
    // Which way it went: the guide takes the car to the right, so the take has it to the
    // left of where the guide does, and no higher or lower.
    assert!(still.across < -0.1, "{still:?}");
    assert!(still.down.abs() < 0.05, "{still:?}");
    // The guide says the car is there on every frame, and how much of the frame it is.
    assert!(right.shows > 0.005 && right.shows < 0.2, "{right:?}");
}

#[test]
fn a_take_with_nothing_where_the_guide_starts_the_subject_is_lost_and_says_so() {
    // The car drives the other way, so where the guide starts it there is only road: the
    // tracker has nothing to hold, and the frames it did read say how few they were.
    let guide = scene("z=2 x={0f:-0.3 24f:0.3}");
    let gone = through(&guide, &scene("z=2 x={0f:0.3 24f:-0.3}"), "car");
    assert!(gone.lost.is_some_and(|frame| frame < 4), "{gone:?}");
    assert!(gone.frames < 4, "{gone:?}");
}

#[test]
fn a_take_that_drew_the_subject_nearer_drew_it_wider() {
    let guide = scene("z=2 x=0");
    let near = through(&guide, &scene("z=1.4 x=0"), "car");
    assert!(near.wide > 1.1, "{near:?}");
}

#[test]
fn a_clip_that_draws_nothing_in_the_guide_is_said_plainly() {
    let guide = scene("z=2 x=0");
    let why = Going::new(&guide, None, "cam").err().expect("a camera draws nothing");
    assert!(why.to_string().contains("draws nothing"), "{why}");
    let missing = Going::new(&guide, None, "nothing").err().expect("no such clip");
    assert!(missing.to_string().contains("draws nothing"), "{missing}");
}
