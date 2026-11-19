//! What shows of each clip on a frame, read without an eye.

use kerf_engine::shows::{showing, Shown, MOST_CLIPS};
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
#[test]
fn a_clip_a_parent_takes_into_depth_is_read_where_it_is_drawn() {
    // A box on a null: where the null is in depth is where the box is read.
    let rigged = |rig: &str, set: &str| {
        format!(
            concat!(
                "  track V1 video\n    clip rig gen=null@1 at=0f dur=24f\n      set {rig}\n",
                "  track V2 video\n    clip box gen=mesh@1 at=0f dur=24f parent=rig\n",
                "      set kind=box width=0.4 height=0.4 length=0.4 color=#ff0000 {set}\n",
            ),
            rig = rig,
            set = set
        )
    };
    let at = |body: &str| {
        let shown = read(body);
        (clip(&shown, "box").at.expect("it is in the frame"), clip(&shown, "box").area)
    };
    let (near, near_area) = at(&rigged("x=0", ""));
    // Moved back with its null it is as small as its own `z` would have it, and smaller
    // than it was.
    let (far, far_area) = at(&rigged("z=2", ""));
    assert_eq!((far, far_area), at(&rigged("x=0", "z=2")));
    assert!(far_area < near_area / 4.0, "{far_area} of {near_area}");
    assert!(far[2] - far[0] < (near[2] - near[0]) / 2.0, "{far:?} of {near:?}");
    // Swung a quarter round its null from the right of it, it is in the middle of the
    // frame and further off; the other way, it has come past the camera and is not read.
    let (swung, _) = at(&rigged("rotate_y=-90", "x=0.5625"));
    assert!((swung[0] + swung[2] - 1.0).abs() < 0.02, "{swung:?}");
    assert!(swung[2] - swung[0] < near[2] - near[0], "{swung:?} of {near:?}");
    let shown = read(&rigged("rotate_y=90", "x=1.125"));
    assert!(clip(&shown, "box").at.is_none(), "{shown:?}");
}

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

/// A solid `kind` named `id`, as large as `size` across, down, and back and with its middle
/// at `place`, both in frame heights, of a frame of 16 by 9.
fn solid(id: &str, kind: &str, size: [f64; 3], place: [f64; 3], color: &str) -> (String, String) {
    let ([w, h, l], [x, y, z]) = (size, place);
    let x = x * 9.0 / 16.0;
    let shape = format!("kind={kind} width={w:.3} height={h:.3} length={l:.3} color={color}");
    let clip = format!("gen=mesh@1 at=0f dur=96f\n      set {shape} x={x:.4} y={y:.3} z={z:.3}");
    (id.to_owned(), clip)
}

/// A clip with more set on it.
fn with((id, clip): (String, String), more: &str) -> (String, String) {
    (id, clip + more)
}

/// A cart of thirteen solids, named `<name>_…`: a bed, two sides, two shafts, two ends, and
/// two axles with four wheels.
fn cart(name: &str, x: f64, z: f64) -> Vec<(String, String)> {
    let wood = |part: &str, size, place| {
        solid(&format!("{name}_{part}"), "box", size, place, "#906838")
    };
    let round = |part: &str, size, place, color| {
        let part = solid(&format!("{name}_{part}"), "cylinder", size, place, color);
        with(part, " rotate_x=90")
    };
    vec![
        wood("bed", [0.5, 0.04, 0.3], [x, 0.34, z]),
        wood("side1", [0.5, 0.1, 0.02], [x, 0.27, z - 0.14]),
        wood("shaft1", [0.3, 0.02, 0.02], [x + 0.4, 0.34, z - 0.098]),
        wood("side2", [0.5, 0.1, 0.02], [x, 0.27, z + 0.14]),
        wood("shaft2", [0.3, 0.02, 0.02], [x + 0.4, 0.34, z + 0.098]),
        wood("end1", [0.02, 0.1, 0.3], [x - 0.24, 0.27, z]),
        wood("end2", [0.02, 0.1, 0.3], [x + 0.24, 0.27, z]),
        round("axle1", [0.03, 0.36, 0.03], [x - 0.17, 0.38, z], "#403020"),
        round("wheel1", [0.24, 0.03, 0.24], [x - 0.17, 0.38, z - 0.17], "#604020"),
        round("wheel2", [0.24, 0.03, 0.24], [x - 0.17, 0.38, z + 0.17], "#604020"),
        round("axle2", [0.03, 0.36, 0.03], [x + 0.17, 0.38, z], "#403020"),
        round("wheel3", [0.24, 0.03, 0.24], [x + 0.17, 0.38, z - 0.17], "#604020"),
        round("wheel4", [0.24, 0.03, 0.24], [x + 0.17, 0.38, z + 0.17], "#604020"),
    ]
}

/// A street blocked out of solids, as a guide has one, on a frame of `size`: a sky out of
/// depth, a ground, seven houses each with a roof and a door, three people of two solids, a
/// cart parked, and the cart that is the subject, written last and so on the top tracks under
/// the lights and the camera. Fifty-five clips draw, the cart's thirteen the highest.
fn street(size: &str) -> Project {
    let flat = |id: &str, set: &str| {
        let clip = format!("gen=solid@1 at=0f dur=96f\n      set {set}");
        (id.to_owned(), clip)
    };
    let mut clips = vec![
        flat("sky", "color=#88aaff"),
        flat("ground", "color=#a09880 three_d=true rotate_x=-90 y=0.5 z=4 scale=16"),
    ];
    let houses = [
        (-1.15, 0.4, "#c8b090"),
        (1.15, 1.0, "#b0b8c0"),
        (-1.15, 1.6, "#d0c0a0"),
        (1.15, 2.2, "#c0a890"),
        (-1.15, 2.8, "#a8b0a0"),
        (1.15, 3.4, "#d8c8b0"),
        (-1.15, 4.0, "#b8a8a0"),
    ];
    for (at, (x, z, color)) in houses.into_iter().enumerate() {
        let door = [x + if x < 0.0 { 0.47 } else { -0.47 }, 0.3, z];
        let roof = solid(&format!("roof{at}"), "box", [0.64, 0.64, 0.9], [x, -0.3, z], "#804030");
        clips.push(solid(&format!("house{at}"), "box", [0.9, 0.8, 0.9], [x, 0.1, z], color));
        clips.push(with(roof, " rotation=45"));
        clips.push(solid(&format!("door{at}"), "box", [0.04, 0.4, 0.2], door, "#503020"));
    }
    let people = [(0.25, 0.7, "#3050a0"), (-0.2, 1.9, "#a03040"), (0.1, 3.2, "#308050")];
    for (at, (x, z, color)) in people.into_iter().enumerate() {
        clips.push(solid(&format!("body{at}"), "cylinder", [0.12, 0.3, 0.12], [x, 0.35, z], color));
        clips.push(solid(&format!("head{at}"), "ball", [0.11; 3], [x, 0.145, z], "#e0b090"));
    }
    clips.extend(cart("parked", 0.35, 2.5));
    clips.extend(cart("cart", -0.3, 1.1));
    let camera = "y=-0.3 tilt=-9 z={0f:-0.3 96f:0.5} pan={0f:-5 96f:5}";
    for (id, kind, set) in [
        ("sun", "light", "kind=directional x=0.35 y=1 z=0.45"),
        ("air", "light", "kind=ambient intensity=0.35"),
        ("cam", "camera", camera),
    ] {
        let clip = format!("gen={kind}@1 at=0f dur=96f\n      set {set}");
        clips.push((id.to_owned(), clip));
    }
    let mut text =
        format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={size} audio=48000\n");
    for (track, (id, clip)) in clips.iter().enumerate() {
        text += &format!("  track V{} video\n    clip {id} {clip}\n", track + 1);
    }
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

#[test]
fn a_scene_of_many_clips_has_those_on_its_upper_tracks_read() {
    let whole = showing(&street("320x180"), None, Rate::FPS_24.time_of(40)).expect("reads");
    // Every clip that draws is read, from the lowest track up: none is left out.
    assert_eq!(whole.unread, Vec::<String>::new());
    assert_eq!(whole.clips.len(), 55, "{:?}", whole.clips);
    assert_eq!(
        (whole.clips[0].clip.as_str(), whole.clips[1].clip.as_str()),
        ("sky", "ground")
    );
    // The subject is the highest thirteen, and each part of it is somewhere in the frame.
    let cart = &whole.clips[42..];
    for part in cart {
        assert!(part.clip.starts_with("cart_"), "{part:?}");
        assert!(part.at.is_some() && part.area > 0.0, "{part:?}");
        assert!(part.edges.is_empty(), "{part:?}");
    }
    // A wheel on the near side shows whole, and the one across from it is behind the cart.
    let near = clip(&whole.clips, "cart_wheel1");
    assert!(near.hidden < 0.05 && near.shows > 0.0, "{near:?}");
    let far = clip(&whole.clips, "cart_wheel2");
    assert!(far.hidden > 0.9, "{far:?}");
    assert_eq!(far.by[0].0, "cart_wheel1", "{far:?}");
    assert!(
        far.by.iter().all(|(by, _)| by.starts_with("cart_")),
        "{far:?}"
    );
    // With every clip read, every pixel of the frame is some clip's or no clip's.
    let had: f64 = whole.clips.iter().map(|shown| shown.shows).sum();
    assert!(
        (had + whole.empty - 1.0).abs() < 0.02,
        "{had} {}",
        whole.empty
    );
}

#[test]
fn of_more_clips_than_a_frame_tells_apart_the_lowest_are_left_out_and_named() {
    // More boxes than are read on one frame, side by side in rows, each on a track of its
    // own, and on the lowest track a wall nearer than them all, before the last of them.
    let boxes = MOST_CLIPS + 2;
    let place = |at: usize| ((at % 24) as f64 / 24.0 - 0.48, (at / 24) as f64 / 12.0 - 0.45);
    let clip = |track: usize, id: &str, side: f64, (x, y): (f64, f64), more: &str| {
        let set = format!("kind=box width={side} height={side} length=0.02 x={x:.3} y={y:.3}{more}");
        format!("  track V{track} video\n    clip {id} gen=mesh@1 at=0f dur=24f\n      set {set}\n")
    };
    let mut text =
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n".to_owned();
    text += &clip(1, "wall", 0.2, place(boxes), " z=-0.1");
    for at in 1..=boxes {
        text += &clip(at + 1, &format!("b{at}"), 0.04, place(at), "");
    }
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let whole = showing(&project, None, Rate::FPS_24.time_of(3)).expect("reads");
    // The highest tracks' are the ones read, and what is left out is said, from the lowest
    // track up: every clip is one or the other.
    assert_eq!(whole.unread[0], "wall", "{:?}", whole.unread);
    assert_eq!(whole.clips.last().unwrap().clip, format!("b{boxes}"));
    assert_eq!(whole.clips.len() + whole.unread.len(), boxes + 1);
    // The ids pass has a color for each of them, so as many are read as are read at most,
    // and the lowest are the ones left out.
    assert_eq!(whole.clips.len(), MOST_CLIPS);
    assert_eq!(whole.unread, ["wall", "b1", "b2"]);
    // What is left out is out of the frame that is read: a box is where it is with nothing
    // before it, the one behind the wall with the rest.
    for shown in &whole.clips {
        assert!(shown.at.is_some() && shown.area > 0.0, "{shown:?}");
        assert!(shown.hidden < 0.03 && shown.by.is_empty(), "{shown:?}");
    }
}

#[test]
fn a_clip_is_not_hidden_by_one_of_its_color_at_another_moment() {
    // A program of more clips than the ids pass has colors: a clip for each color, a frame
    // each and one after another, and over them all one that stays, which has the first's.
    let colors = kerf_compose::ID_COLORS;
    let mut text =
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n".to_owned();
    text += "  track V1 video\n";
    for at in 0..colors {
        text += &format!("    clip c{at} gen=solid@1 at={at}f dur=1f\n      set scale=0.5\n");
    }
    text += &format!(
        "  track V2 video\n    clip stays gen=solid@1 at=0f dur={}f\n      set scale=0.5\n",
        colors + 56
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    let ids = kerf_compose::Compositor::new(
        &project,
        Catalog::builtin(),
        &kerf_compose::ComposeOptions {
            pass: kerf_compose::Pass::Ids,
            ..kerf_compose::ComposeOptions::default()
        },
    )
    .expect("composes")
    .ids();
    assert_eq!((ids[0].0.as_str(), ids[colors].0.as_str()), ("c0", "stays"));
    assert_eq!(ids[0].1, ids[colors].1);
    // Long after the first has ended, the one that stays is read as itself, with nothing
    // before it.
    let alone = |whole: &[Shown]| {
        assert_eq!(whole.len(), 1, "{whole:?}");
        let stays = clip(whole, "stays");
        assert!((stays.area - 0.25).abs() < 0.01, "{stays:?}");
        assert!(stays.hidden < 0.03 && stays.by.is_empty(), "{stays:?}");
        assert!((stays.shows - stays.area).abs() <= 0.001, "{stays:?}");
    };
    let later = showing(&project, None, Rate::FPS_24.time_of(colors as i64 + 6)).expect("reads");
    assert!(later.unread.is_empty(), "{:?}", later.unread);
    alone(&later.clips);
    // On the frame they share, a frame cannot tell the two apart: the lower is left out and
    // named, and the higher is read as if it were not there.
    let shared = showing(&project, None, Rate::FPS_24.time_of(0)).expect("reads");
    assert_eq!(shared.unread, ["c0"]);
    alone(&shared.clips);
    // Between the two, a clip under the one that stays has a color of its own, and is read
    // as all hidden by it.
    let between = showing(&project, None, Rate::FPS_24.time_of(144)).expect("reads");
    assert!(between.unread.is_empty(), "{:?}", between.unread);
    let under = clip(&between.clips, "c144");
    assert!(under.hidden > 0.97, "{under:?}");
    assert_eq!(under.by[0].0, "stays", "{under:?}");
}

#[test]
#[ignore = "a measurement, not a check: cargo test -p kerf-engine --test shows --release -- --ignored --nocapture"]
fn what_reading_a_frame_costs() {
    let street = street("1920x1080");
    let at = Rate::FPS_24.time_of(40);
    let read = showing(&street, None, at).expect("reads");
    let mut times: Vec<f64> = (0..9)
        .map(|_| {
            let started = std::time::Instant::now();
            drop(showing(&street, None, at).expect("reads"));
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    println!(
        "the street, a frame: {} clips read and {} left out in {:.0} ms",
        read.clips.len(),
        read.unread.len(),
        times[4]
    );
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
