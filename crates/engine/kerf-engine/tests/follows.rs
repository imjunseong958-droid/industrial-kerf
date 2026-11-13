//! A take's depth held to its guide, clip by clip.

use kerf_compose::{ComposeOptions, Compositor, Pass};
use kerf_engine::follows::{Followed, Following};
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
            "      set kind=rect width=0.5 height=0.3 fill=#c02020 three_d=true depth=0.4 y=0.8 z=2 {car}\n",
            "  track V4 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
        ),
        car = car,
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

/// The guide held to a take whose depth is what `take` draws: a model that read it right.
fn held(guide: &Project, take: &Project) -> Followed {
    let mut following = Following::new(guide, None).expect("a guide");
    let drawn = Compositor::new(
        take,
        Catalog::builtin(),
        &ComposeOptions {
            pass: Pass::Depth,
            ..ComposeOptions::default()
        },
    )
    .expect("a take");
    let (width, height) = following.size();
    for frame in 0..24 {
        let at = Rate::FPS_24.time_of(frame);
        let picture = drawn.frame(at, f64::from(height) / 180.0);
        assert_eq!((picture.width, picture.height), (width, height));
        // A model stretches a frame to its own nearest and farthest, and is not exact.
        let grey: Vec<u8> = picture
            .rgba
            .chunks_exact(4)
            .map(|pixel| (f64::from(pixel[0]) * 0.8 + 20.0) as u8)
            .collect();
        following.frame(at, &grey);
    }
    following.done()
}

fn clip<'a>(followed: &'a Followed, id: &str) -> &'a kerf_engine::follows::ClipFollowed {
    followed
        .clips
        .iter()
        .find(|clip| clip.clip == id)
        .unwrap_or_else(|| panic!("no {id} in {followed:?}"))
}

#[test]
fn a_take_with_its_subject_elsewhere_is_off_on_that_clip() {
    let guide = scene("x={0f:-0.3 24f:0.3}");
    // The take that followed, and one that drove the car the other way.
    let right = held(&guide, &guide);
    let wrong = held(&guide, &scene("x={0f:0.3 24f:-0.3}"));
    assert_eq!(right.frames, 24);
    // What is in depth is read, and a sky that is not has nothing to hold to.
    let names: Vec<&str> = right.clips.iter().map(|clip| clip.clip.as_str()).collect();
    assert_eq!(names, ["road", "car"]);
    // Over the whole frame the wrong take is nearly as good as the right one: the road is
    // where it was.
    assert!(right.order > 0.999, "{right:?}");
    assert!(wrong.order > 0.9, "{wrong:?}");
    // By the clip it is not. The right take has the car where the guide has it; the wrong
    // one has there the road behind it, which is farther.
    let (car, lost) = (clip(&right, "car"), clip(&wrong, "car"));
    assert!(car.off < 0.01, "{car:?}");
    assert!(lost.off > 10.0 * car.off.max(0.005), "{lost:?} beside {car:?}");
    assert!(lost.farther > 0.0, "{lost:?}");
    assert!(lost.off > 3.0 * clip(&wrong, "road").off, "{wrong:?}");
    // It is worst where the two cars are furthest apart, at an end, and least where they
    // cross in the middle.
    assert!(lost.worst.0 < 4 || lost.worst.0 > 19, "{lost:?}");
    assert!(car.shows > 0.01 && car.frames == 24, "{car:?}");
}
