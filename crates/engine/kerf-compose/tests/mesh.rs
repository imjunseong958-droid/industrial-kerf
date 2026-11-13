//! A clip whose picture is a mesh: a thing with every side it has, placed and seen as a clip in
//! depth is, in the picture and in its passes.

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

fn frame_at(body: &str, pass: Pass, frame: i64) -> Frame {
    compositor(body, pass).frame(Rate::FPS_24.time_of(frame), 1.0)
}

fn frame(body: &str, pass: Pass) -> Frame {
    frame_at(body, pass, 3)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

fn lit(frame: &Frame, x: u32, y: u32) -> bool {
    pixel(frame, x, y).iter().any(|channel| *channel > 8)
}

/// Where what is drawn reaches: left, top, right, bottom, the last two past its last pixel.
fn reach(frame: &Frame) -> [u32; 4] {
    let (mut left, mut top, mut right, mut bottom) = (frame.width, frame.height, 0, 0);
    for y in 0..frame.height {
        for x in 0..frame.width {
            if lit(frame, x, y) {
                (left, top) = (left.min(x), top.min(y));
                (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
            }
        }
    }
    [left, top, right, bottom]
}

/// Whether `reach` is `wanted`, to within a pixel either way.
fn reaches(reach: [u32; 4], wanted: [u32; 4]) -> bool {
    reach.iter().zip(wanted).all(|(a, b)| a.abs_diff(b) <= 1)
}

fn mesh(id: &str, track: u32, set: &str) -> String {
    format!("  track V{track} video\n    clip {id} gen=mesh@1 at=0f dur=24f\n      set {set}\n")
}

#[test]
fn a_mesh_is_as_large_as_it_says_where_its_clip_is() {
    // Thin as paper, its front is at z=0, where a frame height is the frame's: half the frame's
    // height across and a quarter down, about the middle.
    let thin = "kind=box width=0.5 height=0.25 length=0.0001 color=#ffffff";
    let there = reach(&frame(&mesh("thing", 1, thin), Pass::Picture));
    assert!(reaches(there, [115, 67, 205, 113]), "{there:?}");
    // Moved as a clip is, by shares of the frame, and scaled as one is.
    let moved = frame(
        &mesh("thing", 1, &format!("{thin} x=0.25 y=-0.25 scale=0.5")),
        Pass::Picture,
    );
    assert!(
        reaches(reach(&moved), [217, 33, 263, 56]),
        "{:?}",
        reach(&moved)
    );
    // Pushed back by as far as the default camera stands off, it is half the size.
    let back = frame(
        &mesh("thing", 1, &format!("{thin} z=1.3737387097273113")),
        Pass::Picture,
    );
    let [left, top, right, bottom] = reach(&back);
    assert!(
        (right - left).abs_diff(45) <= 1 && (bottom - top).abs_diff(23) <= 1,
        "{:?}",
        reach(&back)
    );
    // A ball is round, and a cylinder stands on its end with straight sides.
    let ball = frame(
        &mesh("thing", 1, "kind=ball width=0.5 height=0.5 length=0.5"),
        Pass::Picture,
    );
    assert!(lit(&ball, 160, 90) && lit(&ball, 160, 50) && lit(&ball, 120, 90));
    assert!(!lit(&ball, 125, 55), "a ball has no corner");
    let can = frame(
        &mesh("thing", 1, "kind=cylinder width=0.5 height=0.5 length=0.5"),
        Pass::Picture,
    );
    assert!(lit(&can, 125, 55), "a cylinder's side goes straight up");
}

#[test]
fn a_mesh_has_more_than_one_side() {
    // Turned a quarter round, a flat clip is edge on and gone. A box shows its side.
    let flat = "  track V1 video\n    clip thing gen=solid@1 at=0f dur=24f\n      set color=#ffffff three_d=true scale=0.5 rotate_y=90\n";
    assert_eq!(reach(&frame(flat, Pass::Picture))[2], 0);
    let turned = mesh(
        "thing",
        1,
        "kind=box width=0.5 height=0.25 length=0.2 rotate_y=90",
    );
    let [left, _, right, _] = reach(&frame(&turned, Pass::Picture));
    // Its length is now across the frame, its nearer face a little larger for being nearer.
    assert!((36..=50).contains(&(right - left)), "{left} to {right}");
    // Turned part of the way under a light from the left, its two sides are lit apart.
    let lit_from_left = format!(
        "{}  track V2 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=1 y=0 z=0.3\n",
        mesh("thing", 1, "kind=box width=0.3 height=0.3 length=0.3 rotate_y=45 color=#ffffff")
    );
    let picture = frame(&lit_from_left, Pass::Picture);
    let (left_side, right_side) = (pixel(&picture, 140, 90)[0], pixel(&picture, 180, 90)[0]);
    assert!(left_side > right_side + 40, "{left_side} and {right_side}");
    // With no light it is its color, flat.
    let unlit = frame(
        &mesh(
            "thing",
            1,
            "kind=box width=0.3 height=0.3 length=0.3 rotate_y=45 color=#ff8000",
        ),
        Pass::Picture,
    );
    assert_eq!(pixel(&unlit, 140, 90), pixel(&unlit, 180, 90));
    let [r, g, b] = pixel(&unlit, 140, 90);
    assert!(r >= 254 && g.abs_diff(128) <= 1 && b == 0, "{r} {g} {b}");
}

#[test]
fn a_camera_going_round_a_mesh_sees_its_other_sides() {
    // A box three times as long as it is wide, twice as far off as a clip at z=0, and a camera
    // that goes a quarter round it, as far from it all the way: a camera's `x` is in frame widths
    // and its `z` in frame heights.
    let body = concat!(
        "  track V1 video\n    clip thing gen=mesh@1 at=0f dur=24f\n",
        "      set kind=box width=0.2 height=0.2 length=0.6 z=1.3737387097273113\n",
        "  track V2 video\n    clip cam gen=camera@1 at=0f dur=24f\n",
        "      set x={0f:0 12f:1.5454560484432252} z={0f:0 12f:2.7474774194546226} pan={0f:0 12f:-90}\n",
    );
    let width = |at: i64| {
        let [left, _, right, _] = reach(&frame_at(body, Pass::Ids, at));
        right - left
    };
    // From the front it is its width across, from the side its length: about three times as wide.
    let (front, side) = (width(0), width(12));
    assert!((16..=24).contains(&front), "{front}");
    assert!(
        (front * 5 / 2..=front * 7 / 2).contains(&side),
        "{front} then {side}"
    );
}

/// A wall turned about its middle, its right side toward the camera, and a wide flat box across
/// it at z=0: the box is before the wall on the left, and behind it on the right.
fn across(wall_track: u32, box_track: u32) -> String {
    format!(
        concat!(
            "  track V{wall} video\n    clip wall gen=solid@1 at=0f dur=24f\n",
            "      set color=#c0c0c0 three_d=true scale=0.9 rotate_y=30\n",
            "  track V{thing} video\n    clip thing gen=mesh@1 at=0f dur=24f\n",
            "      set kind=box width=1 height=0.2 length=0.05 color=#ff0000\n",
        ),
        wall = wall_track,
        thing = box_track,
    )
}

#[test]
fn a_mesh_and_a_clip_in_depth_are_cut_where_they_meet() {
    let same = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
    // Whichever of the two is on the track above, the same is seen: it goes by depth at each
    // pixel, not by track.
    for (wall, thing) in [(1, 2), (2, 1)] {
        let drawn = compositor(&across(wall, thing), Pass::Ids);
        let ids = drawn.ids();
        let color = |id: &str| ids.iter().find(|(told, _)| told == id).unwrap().1;
        let frame = drawn.frame(Rate::FPS_24.time_of(3), 1.0);
        assert!(
            same(pixel(&frame, 115, 90), color("thing")),
            "{wall} {thing}"
        );
        assert!(
            same(pixel(&frame, 205, 90), color("wall")),
            "{wall} {thing}"
        );
        // Above the box there is only the wall.
        assert!(
            same(pixel(&frame, 115, 40), color("wall")),
            "{wall} {thing}"
        );
        // The picture is cut the same way.
        let picture =
            compositor(&across(wall, thing), Pass::Picture).frame(Rate::FPS_24.time_of(3), 1.0);
        let [r, g, _] = pixel(&picture, 115, 90);
        assert!(r > 200 && g < 40, "{wall} {thing}: {r} {g}");
        let [r, g, _] = pixel(&picture, 205, 90);
        assert!(r.abs_diff(g) <= 2, "{wall} {thing}: {r} {g}");
    }
}

#[test]
fn a_depth_pass_has_a_mesh_by_the_pixel() {
    let grey = |frame: &Frame, x: u32, y: u32| {
        let [r, g, b] = pixel(frame, x, y);
        assert!(
            r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
            "{r} {g} {b} is not grey"
        );
        r
    };
    // A ball about z=0: its middle bulges toward the camera, nearer than a clip at z=0 would be,
    // and it falls away to its rim, with nothing around it.
    let ball = frame(
        &mesh("thing", 1, "kind=ball width=0.6 height=0.6 length=0.6"),
        Pass::Depth,
    );
    let greys: Vec<u8> = (0..3)
        .map(|step| grey(&ball, 160 + step * 24, 90))
        .collect();
    assert!(greys[0] > 128 + 10, "{greys:?}");
    assert!(greys.windows(2).all(|pair| pair[1] < pair[0]), "{greys:?}");
    assert_eq!(grey(&ball, 10, 10), 0);
    // The box across the wall: on the left the box is nearer than the wall above it, and on the
    // right the wall, which hides it there, is nearer than a clip at z=0.
    let through = frame(&across(1, 2), Pass::Depth);
    assert!(grey(&through, 115, 90) > grey(&through, 115, 40));
    assert!(grey(&through, 205, 90) > 128);
    // The same pass twice is the same to the byte, and the GPU is not asked for a mesh.
    let at = Rate::FPS_24.time_of(3);
    let drawn = compositor(&across(1, 2), Pass::Depth);
    assert_eq!(drawn.frame(at, 1.0), drawn.frame(at, 1.0));
    let picture = compositor(&across(1, 2), Pass::Picture);
    assert!(!picture.draws_on_gpu(at, 1.0));
}

#[test]
fn a_mesh_is_keyed_and_parented_as_a_clip_is() {
    // Keyed across the frame, and carried by a null it is parented to.
    let keyed = mesh(
        "thing",
        1,
        "kind=ball width=0.2 height=0.2 length=0.2 x={0f:-0.25 12f:0.25}",
    );
    assert!(reach(&frame_at(&keyed, Pass::Ids, 0))[2] < 160);
    assert!(reach(&frame_at(&keyed, Pass::Ids, 12))[0] > 160);
    let carried = concat!(
        "  track V1 video\n    clip rig gen=null@1 at=0f dur=24f\n      set x=0.25\n",
        "  track V2 video\n    clip thing gen=mesh@1 at=0f dur=24f parent=rig\n",
        "      set kind=ball width=0.2 height=0.2 length=0.2\n",
    );
    assert!(reach(&frame(carried, Pass::Ids))[0] > 160);
    // Its opacity is its own, and at none it is not there.
    let clear = mesh("thing", 1, "kind=box opacity=0");
    assert_eq!(reach(&frame(&clear, Pass::Picture))[2], 0);
}

#[test]
fn a_clip_in_depth_casts_its_shadow_on_a_mesh() {
    // A light shining from the left, a box turned to show it a side, and a clip edge on to the
    // camera standing between the two.
    let scene = |shadow: &str| {
        format!(
            concat!(
                "{}  track V2 video\n    clip screen gen=solid@1 at=0f dur=24f\n",
                "      set color=#ffffff three_d=true rotate_y=90 x=-0.3 scale=0.6\n",
                "  track V3 video\n    clip sun gen=light@1 at=0f dur=24f\n",
                "      set kind=directional x=1 y=0 z=0 shadow={}\n",
            ),
            mesh(
                "thing",
                1,
                "kind=box width=0.3 height=0.3 length=0.3 rotate_y=45 color=#ffffff"
            ),
            shadow
        )
    };
    let lit = pixel(&frame(&scene("0"), Pass::Picture), 140, 90)[0];
    let shadowed = pixel(&frame(&scene("1"), Pass::Picture), 140, 90)[0];
    assert!(
        lit > 180 && shadowed < 20,
        "{lit} lit, {shadowed} in shadow"
    );
}

#[test]
fn a_mesh_takes_no_effect_or_mask() {
    let refused = |body: &str| {
        let project = parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n{body}"
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default())
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    };
    let blurred = format!(
        "{}      fx blur.gaussian@1 radius=4\n",
        mesh("thing", 1, "kind=box")
    );
    assert!(
        refused(&blurred).contains("an effect on a mesh"),
        "{}",
        refused(&blurred)
    );
    let masked = format!(
        "{}      mask k1 ellipse=0.5,0.5,0.3,0.3\n",
        mesh("thing", 1, "kind=box")
    );
    assert!(
        refused(&masked).contains("a mask on a mesh"),
        "{}",
        refused(&masked)
    );
}

/// A glTF model of two flat parts, written three ways: a red sheet 2 across and 1 up, standing on
/// the ground, with no normals and with indices; and on a node under it, moved up by 1 and half
/// the size, a green sheet of the same shape with neither. `.glb`, `.gltf` with its bytes beside
/// it, and `.gltf` with them inside.
fn model(folder: &std::path::Path, kind: &str) -> std::path::PathBuf {
    let sheet: [[f32; 3]; 4] = [
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let mut bytes = Vec::new();
    for corner in sheet {
        bytes.extend(corner.iter().flat_map(|value| value.to_le_bytes()));
    }
    for index in [0u16, 1, 2, 0, 2, 3] {
        bytes.extend(index.to_le_bytes());
    }
    for index in [0, 1, 2, 0, 2, 3] {
        bytes.extend(sheet[index].iter().flat_map(|value| value.to_le_bytes()));
    }
    let buffer = match kind {
        "glb" => String::new(),
        "beside" => "\"uri\":\"model.bin\",".to_owned(),
        _ => {
            const LETTERS: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut text = String::new();
            for three in bytes.chunks(3) {
                let held = three
                    .iter()
                    .fold(0u32, |held, byte| (held << 8) | u32::from(*byte))
                    << (8 * (3 - three.len()));
                for place in 0..4 {
                    text.push(if place <= three.len() {
                        LETTERS[((held >> (18 - 6 * place)) & 63) as usize] as char
                    } else {
                        '='
                    });
                }
            }
            format!("\"uri\":\"data:application/octet-stream;base64,{text}\",")
        }
    };
    let json = format!(
        concat!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
            "\"nodes\":[{{\"mesh\":0,\"children\":[1]}},",
            "{{\"mesh\":1,\"translation\":[0,1,0],\"scale\":[0.5,0.5,0.5]}}],",
            "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"material\":0}}]}},",
            "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":2}},\"material\":1}}]}}],",
            "\"materials\":[{{\"pbrMetallicRoughness\":{{\"baseColorFactor\":[1,0,0,1]}}}},",
            "{{\"pbrMetallicRoughness\":{{\"baseColorFactor\":[0,1,0,1]}}}}],",
            "\"buffers\":[{{{buffer}\"byteLength\":132}}],",
            "\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":48}},",
            "{{\"buffer\":0,\"byteOffset\":48,\"byteLength\":12}},",
            "{{\"buffer\":0,\"byteOffset\":60,\"byteLength\":72}}],",
            "\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":4,\"type\":\"VEC3\"}},",
            "{{\"bufferView\":1,\"componentType\":5123,\"count\":6,\"type\":\"SCALAR\"}},",
            "{{\"bufferView\":2,\"componentType\":5126,\"count\":6,\"type\":\"VEC3\"}}]}}",
        ),
        buffer = buffer
    );
    if kind == "glb" {
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
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
        let path = folder.join("model.glb");
        std::fs::write(&path, file).unwrap();
        return path;
    }
    if kind == "beside" {
        std::fs::write(folder.join("model.bin"), &bytes).unwrap();
    }
    let path = folder.join(format!("{kind}.gltf"));
    std::fs::write(&path, json).unwrap();
    path
}

/// A sequence with one mesh read from the file `name` of the project's folder.
fn from_file(
    folder: &std::path::Path,
    name: &str,
    set: &str,
    pass: Pass,
) -> Result<Compositor, String> {
    let mut project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip thing gen=mesh@1 at=0f dur=24f\n      set file=\"{name}\" {set}\n"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    // A document names its files from its own folder.
    project.resolve_paths(Catalog::builtin(), folder);
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            pass,
            ..ComposeOptions::default()
        },
    )
    .map_err(|error| error.to_string())
}

#[test]
fn a_mesh_is_read_from_a_gltf_file_a_color_a_part() {
    let folder = tempfile::tempdir().expect("a folder");
    let at = Rate::FPS_24.time_of(3);
    let mut frames = Vec::new();
    for kind in ["glb", "beside", "inside"] {
        let path = model(folder.path(), kind);
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let drawn = from_file(folder.path(), &name, "height=0.6", Pass::Picture).expect(kind);
        frames.push(drawn.frame(at, 1.0));
    }
    // The three ways a file is written are one model.
    assert!(frames[1] == frames[0] && frames[2] == frames[0]);
    let frame = &frames[0];
    // 1.5 tall in its own units and 0.6 of the frame's height here, about the middle: the red
    // sheet below, 144 across and 72 down, and the green on top of it, half the size, each in
    // its material's color, where its node put it and the right way up.
    assert!(
        reaches(reach(frame), [88, 36, 232, 144]),
        "{:?}",
        reach(frame)
    );
    assert_eq!(pixel(frame, 160, 110), [255, 0, 0]);
    assert_eq!(pixel(frame, 100, 140), [255, 0, 0]);
    assert_eq!(pixel(frame, 160, 55), [0, 255, 0]);
    assert!(!lit(frame, 100, 55) && lit(frame, 130, 55));
    // It is a clip like any other: one color in an ids pass, turned and moved as a clip is.
    let ids = from_file(folder.path(), "model.glb", "height=0.6", Pass::Ids).unwrap();
    let color = ids.ids()[0].1;
    let frame = ids.frame(at, 1.0);
    assert_eq!(pixel(&frame, 160, 110), pixel(&frame, 160, 55));
    assert!(pixel(&frame, 160, 110)
        .iter()
        .zip(color)
        .all(|(a, b)| a.abs_diff(b) <= 1));
    let turned = from_file(
        folder.path(),
        "model.glb",
        "height=0.6 rotate_y=60 x=0.2",
        Pass::Ids,
    )
    .unwrap();
    let [left, _, right, _] = reach(&turned.frame(at, 1.0));
    assert!(left > 160 && (right - left) < 130, "{left} to {right}");
}

#[test]
fn a_file_that_is_no_mesh_is_said_when_the_sequence_is_planned() {
    let folder = tempfile::tempdir().expect("a folder");
    let missing = from_file(folder.path(), "nowhere.glb", "", Pass::Picture)
        .err()
        .unwrap();
    assert!(
        missing.contains("clip `thing`") && missing.contains("cannot be read"),
        "{missing}"
    );
    std::fs::write(
        folder.path().join("empty.gltf"),
        "{\"asset\":{\"version\":\"2.0\"}}",
    )
    .unwrap();
    let empty = from_file(folder.path(), "empty.gltf", "", Pass::Picture)
        .err()
        .unwrap();
    assert!(empty.contains("it has no triangles"), "{empty}");
    std::fs::write(folder.path().join("broken.glb"), b"glTF\x02\0\0\0").unwrap();
    assert!(from_file(folder.path(), "broken.glb", "", Pass::Picture).is_err());
}
