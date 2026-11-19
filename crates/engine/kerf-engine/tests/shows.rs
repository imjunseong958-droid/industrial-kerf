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
    // Half clear, it is where it is, with half of each pixel it is on, and why is said. What
    // shows through it is hidden by that half, and the car by nothing.
    let shown = read(&scene(
        "rotate_x=-80 y=0.4 z=1 scale=3",
        "scale=0.2 opacity=0.5",
    ));
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
    assert!((car.area - 0.02).abs() < 0.003, "{car:?}");
    assert!((car.shows - car.area).abs() <= 0.001, "{car:?}");
    assert!(car.hidden < 0.03 && car.by.is_empty(), "{car:?}");
    assert_eq!(car.opacity, Some(0.5));
    let opaque = read(&scene("rotate_x=-80 y=0.4 z=1 scale=3", "scale=0.2"));
    let behind = |shown: &[Shown]| clip(shown, "sky").shows + clip(shown, "floor").shows;
    assert!(
        (behind(&shown) - behind(&opaque) - car.shows).abs() <= 0.002,
        "{shown:?} {opaque:?}"
    );
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

/// A sky, a floor in depth, and over the floor a title in depth, on a frame of 1920 by 1080:
/// read a fifth as tall, the title's strokes are about a pixel wide.
fn word(set: &str) -> Vec<Shown> {
    let text = format!(
        concat!(
            "kerf 1\nproject name=\"t\"\nsequence set rate=24 size=1920x1080 audio=48000\n",
            "  track V1 video\n    clip sky gen=solid@1 at=0f dur=96f\n      set color=#7fa8d8\n",
            "  track V2 video\n    clip floor gen=solid@1 at=0f dur=96f\n",
            "      set color=#50545a three_d=true rotate_x=-84 y=0.75 z=4 scale=16\n",
            "  track V3 video\n    clip word gen=title@1 at=0f dur=96f\n",
            "      set text=\"KERF\" size=0.25 three_d=true {set}\n",
        ),
        set = set,
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    showing(&project, None, Rate::FPS_24.time_of(0))
        .expect("reads")
        .clips
}

#[test]
fn thin_strokes_are_not_hidden_by_what_they_are_drawn_over() {
    // Every pixel of the title is part the title's color and part the floor's, and the title
    // is before the floor on all of them.
    let shown = word("z=2.6 y=0.2 depth=0.1 bevel=0.03");
    let title = clip(&shown, "word");
    assert!(title.at.is_some() && title.area > 0.0, "{title:?}");
    assert!(title.hidden < 0.05 && title.by.is_empty(), "{title:?}");
    assert!((title.shows - title.area).abs() <= 0.001, "{title:?}");
    // What it says of a clip agrees with itself: what shows is what is not hidden, and what is
    // hidden is hidden by the clips it names.
    for shown in &shown {
        let by: f64 = shown.by.iter().map(|(_, share)| share).sum();
        assert!(by <= shown.hidden + 0.002, "{shown:?}");
        assert!(shown.hidden - by < 0.06, "{shown:?}");
        assert!(
            (shown.shows - shown.area * (1.0 - shown.hidden)).abs() <= 0.002,
            "{shown:?}"
        );
    }
    // Sunk under the floor, the same strokes are hidden, and by the floor.
    let shown = word("z=2.6 y=1.2 depth=0.1 bevel=0.03");
    let title = clip(&shown, "word");
    assert!(title.hidden > 0.9 && title.shows < 0.001, "{title:?}");
    assert_eq!(title.by[0].0, "floor", "{title:?}");
    assert!(title.by[0].1 > 0.9, "{title:?}");
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
