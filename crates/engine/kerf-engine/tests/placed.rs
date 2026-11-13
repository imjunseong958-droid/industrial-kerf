//! Where a take has a clip's subject, beside where its guide has it.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_engine::placed::{Placed, Placing};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project, Rate};

/// A sky out of depth; in depth a road tipped back, and a car that drives across it, small
/// at the start and nearer, so larger, at the end.
fn scene(car: &str) -> Project {
    let text = format!(
        concat!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n",
            "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
            "  track V2 video\n    clip road gen=solid@1 at=0f dur=24f\n",
            "      set color=#404040 three_d=true rotate_x=-84 y=0.75 z=4 scale=16\n",
            "  track V3 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
            "  track V4 video\n    clip car gen=shape@1 at=0f dur=24f\n",
            "      set kind=rect width=0.5 height=0.3 fill=#c02020 three_d=true depth=0.4 y=0.8 {car}\n",
        ),
        car = car,
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// The guide's car looked for in a take that is the picture `take` draws.
fn placed(guide: &Project, take: &Project) -> Option<Placed> {
    let mut placing = Placing::new(guide, None, "car").expect("a guide");
    let drawn =
        Compositor::new(take, Catalog::builtin(), &ComposeOptions::default()).expect("a take");
    let (width, height) = placing.size();
    for frame in 0..24 {
        let at = Rate::FPS_24.time_of(frame);
        let picture = drawn.frame(at, f64::from(height) / 180.0);
        assert_eq!((picture.width, picture.height), (width, height));
        placing.frame(at, &picture.rgba);
    }
    placing.done()
}

const DRIVES: &str = "x={0f:-0.25 24f:0.25} z={0f:3.2 24f:2}";

#[test]
fn a_take_with_its_subject_where_the_guide_has_it_is_not_off() {
    let guide = scene(DRIVES);
    let same = placed(&guide, &guide).expect("the car");
    // It is found where the guide shows most of it, at the end, and followed back to the
    // start, where it is smaller.
    assert_eq!(same.clip, "car");
    assert!(same.shown >= 22 && same.frames >= 18, "{same:?}");
    assert_eq!((same.from, same.read.1), (23, 23), "{same:?}");
    assert!(same.read.0 <= 6, "{same:?}");
    // The guide's own picture is followed as the take is, so the same picture is not off
    // at all, whatever a tracker's box adds to an ids pass's.
    assert_eq!((same.off, same.size), (0.0, 1.0), "{same:?}");
    assert!(same.shows > 0.005, "{same:?}");
}

#[test]
fn a_take_with_its_subject_to_one_side_says_how_far_and_which_way() {
    let guide = scene(DRIVES);
    // The take has the car a tenth of the frame's width to the right throughout, which at
    // this distance is some of a frame height on the picture, and nearer, so larger.
    let aside = placed(&guide, &scene("x={0f:-0.15 24f:0.35} z={0f:3.2 24f:2}")).expect("the car");
    assert!(aside.across > 0.05 && aside.across < 0.25, "{aside:?}");
    assert!(aside.down.abs() < 0.03, "{aside:?}");
    assert!((aside.off - aside.across).abs() < 0.03, "{aside:?}");
    assert!(aside.worst.1 >= aside.off, "{aside:?}");
    let nearer = placed(&guide, &scene("x={0f:-0.25 24f:0.25} z={0f:2.2 24f:1.4}")).expect("the car");
    assert!(nearer.size > 1.3, "{nearer:?}");
    // The other way is the other sign.
    let left = placed(&guide, &scene("x={0f:-0.35 24f:0.15} z={0f:3.2 24f:2}")).expect("the car");
    assert!(left.across < -0.05, "{left:?}");
}

#[test]
fn a_clip_the_guide_does_not_show_is_refused() {
    let guide = scene(DRIVES);
    let none = Placing::new(&guide, None, "truck").err().expect("no truck");
    assert!(none.to_string().contains("draws no clip `truck`"), "{none}");
    // A camera draws nothing, so the guide does not say where it is.
    assert!(Placing::new(&guide, None, "cam").is_err());
}
