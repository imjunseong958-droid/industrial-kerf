//! What shows of each clip on a frame, read without an eye.

use kerf_engine::shows::{showing, Shown};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Project, Rate};

fn project(body: &str) -> Project {
    parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

fn read(body: &str) -> Vec<Shown> {
    showing(&project(body), None, Rate::FPS_24.time_of(3))
        .expect("reads")
        .clips
}

fn clip<'a>(shown: &'a [Shown], id: &str) -> &'a Shown {
    shown
        .iter()
        .find(|shown| shown.clip == id)
        .unwrap_or_else(|| panic!("no {id} in {shown:?}"))
}

/// A sky out of depth, then in depth a floor, a car standing on it, and a camera.
fn scene(floor: &str, car: &str) -> String {
    format!(
        concat!(
            "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
            "  track V2 video\n    clip floor gen=solid@1 at=0f dur=24f\n",
            "      set color=#404040 three_d=true {floor}\n",
            "  track V3 video\n    clip car gen=solid@1 at=0f dur=24f\n",
            "      set color=#ff0000 three_d=true {car}\n",
            "  track V4 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
            "  track V5 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=ambient\n",
        ),
        floor = floor,
        car = car,
    )
}

#[test]
fn each_clip_is_where_it_is_with_what_is_before_it() {
    let shown = read(&scene("rotate_x=-80 y=0.4 z=1 scale=3", "scale=0.2"));
    // What draws, from the lowest track up; a camera and a light draw nothing.
    assert_eq!(
        shown
            .iter()
            .map(|shown| shown.clip.as_str())
            .collect::<Vec<_>>(),
        ["sky", "floor", "car"]
    );
    // The car is a fifth of the frame each way, in its middle, with nothing before it.
    let car = clip(&shown, "car");
    let [left, top, right, bottom] = car.at.unwrap();
    assert!(
        (left - 0.4).abs() < 0.02 && (right - 0.6).abs() < 0.02,
        "{car:?}"
    );
    assert!(
        (top - 0.4).abs() < 0.02 && (bottom - 0.6).abs() < 0.02,
        "{car:?}"
    );
    assert!((car.area - 0.04).abs() < 0.005, "{car:?}");
    assert!(
        car.hidden < 0.03 && car.by.is_empty() && car.edges.is_empty(),
        "{car:?}"
    );
    assert_eq!(car.opacity, None);
    // The sky fills the frame and is hidden where the floor and the car are.
    let sky = clip(&shown, "sky");
    assert_eq!(sky.at, Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(sky.edges, ["left", "top", "right", "bottom"]);
    assert!(sky.hidden > 0.3, "{sky:?}");
    assert_eq!(sky.by[0].0, "floor");
    assert!(sky.by.iter().any(|(by, _)| by == "car"), "{sky:?}");
    // The floor goes off the frame's foot and sides.
    let floor = clip(&shown, "floor");
    assert!(floor.edges.contains(&"bottom"), "{floor:?}");
}

#[test]
fn a_floor_over_what_stands_on_it_is_said() {
    // The fault a guide once had: a floor higher than the car's foot hides the car.
    let shown = read(&scene(
        "rotate_x=-80 y=0.1 z=0 scale=3",
        "scale=0.2 y=0.2 z=0.5",
    ));
    let car = clip(&shown, "car");
    assert!(car.hidden > 0.5, "{car:?}");
    assert_eq!(car.by[0].0, "floor", "{car:?}");
    assert!(car.shows < car.area, "{car:?}");
}

#[test]
fn what_has_left_the_frame_or_lets_light_through_is_said() {
    // Moved wholly off the frame, a clip is nowhere.
    let shown = read(&scene("rotate_x=-80 y=0.4 z=1 scale=3", "scale=0.2 x=2"));
    let car = clip(&shown, "car");
    assert_eq!((car.at, car.area, car.shows), (None, 0.0, 0.0));
    // Half way off it, it reaches the edge it leaves by.
    let shown = read(&scene("rotate_x=-80 y=0.4 z=1 scale=3", "scale=0.2 x=0.5"));
    let car = clip(&shown, "car");
    assert_eq!(car.edges, ["right"], "{car:?}");
    assert!((car.area - 0.02).abs() < 0.004, "{car:?}");
    // Half clear, it is where it is, and is read as not showing, with why.
    let shown = read(&scene(
        "rotate_x=-80 y=0.4 z=1 scale=3",
        "scale=0.2 opacity=0.5",
    ));
    let car = clip(&shown, "car");
    assert!(car.at.is_some() && car.shows == 0.0, "{car:?}");
    assert_eq!(car.opacity, Some(0.5));
}

#[test]
fn a_mesh_is_read_as_any_clip_is() {
    // The car a box with its sides, standing on the floor: where it is, with nothing before it.
    let car = |floor: &str, set: &str| {
        format!(
            concat!(
                "  track V1 video\n    clip sky gen=solid@1 at=0f dur=24f\n      set color=#88aaff\n",
                "  track V2 video\n    clip floor gen=solid@1 at=0f dur=24f\n",
                "      set color=#404040 three_d=true {floor}\n",
                "  track V3 video\n    clip car gen=mesh@1 at=0f dur=24f\n",
                "      set kind=box width=0.4 height=0.2 length=0.2 color=#ff0000 {set}\n",
                "  track V4 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=ambient\n",
            ),
            floor = floor,
            set = set,
        )
    };
    let shown = read(&car("rotate_x=-80 y=0.4 z=1 scale=3", ""));
    assert_eq!(
        shown
            .iter()
            .map(|shown| shown.clip.as_str())
            .collect::<Vec<_>>(),
        ["sky", "floor", "car"]
    );
    let there = clip(&shown, "car");
    let [left, top, right, bottom] = there.at.unwrap();
    // Its front is nearer than z=0 by half its length, and so a little larger than 0.4 by 0.2
    // of the frame's height.
    assert!(left < 0.5 - 0.1125 && left > 0.34, "{there:?}");
    assert!(right > 0.5 + 0.1125 && right < 0.66, "{there:?}");
    assert!(top < 0.4 && top > 0.34 && bottom > 0.6 && bottom < 0.66, "{there:?}");
    assert!(there.hidden < 0.03 && there.by.is_empty(), "{there:?}");
    // The fault a guide once had, with a mesh for the car: a floor above its foot hides it.
    let shown = read(&car("rotate_x=-80 y=0.1 z=0 scale=3", "y=0.2 z=0.5"));
    let there = clip(&shown, "car");
    assert!(there.hidden > 0.5, "{there:?}");
    assert_eq!(there.by[0].0, "floor", "{there:?}");
    // Turned edge on, a mesh is still there, where a flat clip is gone.
    let shown = read(&car("rotate_x=-80 y=0.4 z=1 scale=3", "rotate_y=90"));
    let there = clip(&shown, "car");
    assert!(there.at.is_some() && there.area > 0.01, "{there:?}");
}

#[test]
fn nothing_is_said_of_a_moment_with_no_clips() {
    let whole = showing(
        &project("  track V1 video\n    clip one gen=solid@1 at=0f dur=2f\n      set scale=0.5\n"),
        None,
        Rate::FPS_24.time_of(1),
    )
    .unwrap();
    assert_eq!(whole.clips.len(), 1);
    assert!((whole.empty - 0.75).abs() < 0.01, "{whole:?}");
    assert!(whole.unread.is_empty());
}
