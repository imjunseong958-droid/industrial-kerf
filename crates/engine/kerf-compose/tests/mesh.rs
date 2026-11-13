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

/// A glTF file as it is written: its bytes, and the views and accessors onto them.
#[derive(Default)]
struct Writing {
    bytes: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

impl Writing {
    /// `bytes` as a view of the file's buffer, and which view it is.
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        self.views.push(format!(
            "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{}}}",
            self.bytes.len(),
            bytes.len()
        ));
        self.bytes.extend(bytes);
        self.views.len() - 1
    }

    fn accessor(&mut self, bytes: &[u8], component: u32, across: usize, more: &str) -> usize {
        let view = self.view(bytes);
        let kind = match across {
            1 => "SCALAR",
            2 => "VEC2",
            3 => "VEC3",
            4 => "VEC4",
            _ => "MAT4",
        };
        let width = match component {
            5121 => 1,
            5123 => 2,
            _ => 4,
        };
        self.accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":{component},\"count\":{},\"type\":\"{kind}\"{more}}}",
            bytes.len() / width / across
        ));
        self.accessors.len() - 1
    }

    /// `values` as an accessor of floats, `across` to an element, and which accessor it is.
    fn floats(&mut self, across: usize, values: &[f32]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        self.accessor(&bytes, 5126, across, "")
    }

    fn shorts(&mut self, values: &[u16]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        self.accessor(&bytes, 5123, 1, "")
    }

    /// Bytes from 0 to 255 read as shares of 255, `across` to an element.
    fn shares(&mut self, across: usize, values: &[u8]) -> usize {
        self.accessor(values, 5121, across, ",\"normalized\":true")
    }

    /// The file's JSON: `rest`, its other members, and what was written.
    fn json(&self, rest: &str, uri: &str) -> String {
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},{rest},\"buffers\":[{{{uri}\"byteLength\":{}}}],\"bufferViews\":[{}],\"accessors\":[{}]}}",
            self.bytes.len(),
            self.views.join(","),
            self.accessors.join(",")
        )
    }

    /// The file as a `.glb` named `name` in `folder`.
    fn glb(&self, folder: &std::path::Path, name: &str, rest: &str) {
        let mut json = self.json(rest, "").into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bytes = self.bytes.clone();
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
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
        std::fs::write(folder.join(name), file).unwrap();
    }

    /// The file as a `.gltf` named `name` in `folder`, with its bytes beside it.
    fn gltf(&self, folder: &std::path::Path, name: &str, rest: &str) {
        std::fs::write(folder.join(format!("{name}.bin")), &self.bytes).unwrap();
        let json = self.json(rest, &format!("\"uri\":\"{name}.bin\","));
        std::fs::write(folder.join(name), json).unwrap();
    }

    /// A sheet 2 across and 1 up, standing on the ground and facing the viewer, its corners from
    /// the top left round by the top right: the accessors of where they are, and of its indices.
    fn sheet(&mut self) -> (usize, usize) {
        let corners = [-1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0];
        (self.floats(3, &corners), self.shorts(&[0, 1, 2, 0, 2, 3]))
    }
}

/// A PNG file of `width` by `height`, each pixel what `color` says, written with no packing.
fn png(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (!(crc & 1)).wrapping_add(1));
            }
        }
        !crc
    }
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend(color(x, y));
        }
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    let mut packed = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (index, block) in blocks.iter().enumerate() {
        packed.push(u8::from(index + 1 == blocks.len()));
        packed.extend((block.len() as u16).to_le_bytes());
        packed.extend((!(block.len() as u16)).to_le_bytes());
        packed.extend(*block);
    }
    packed.extend(((b << 16) | a).to_be_bytes());
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        file.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        file.extend(&body);
        file.extend(crc(&body).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    file
}

/// A sequence with a mesh read from the file `name` of `folder`, `height` 0.5, and `more` tracks.
fn sheet_of(folder: &std::path::Path, name: &str, more: &str, pass: Pass) -> Frame {
    let mut project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n  track V1 video\n    clip thing gen=mesh@1 at=0f dur=24f\n      set file=\"{name}\" height=0.5\n{more}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    project.resolve_paths(Catalog::builtin(), folder);
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            pass,
            ..ComposeOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{error}"))
    .frame(Rate::FPS_24.time_of(3), 1.0)
}

const SUN: &str = "  track V2 video\n    clip sun gen=light@1 at=0f dur=24f\n      set kind=directional x=1 y=0 z=0.3\n";
const SCENE: &str = "\"scene\":0,\"scenes\":[{\"nodes\":[0]}],\"nodes\":[{\"mesh\":0}]";

#[test]
fn a_part_with_a_picture_is_drawn_in_it_with_no_light() {
    let folder = tempfile::tempdir().expect("a folder");
    // A picture in four: red and green above, blue and white below.
    let quarters = png(8, 8, |x, y| match (x < 4, y < 4) {
        (true, true) => [255, 0, 0],
        (false, true) => [0, 255, 0],
        (true, false) => [0, 0, 255],
        (false, false) => [255, 255, 255],
    });
    let mut pictured = Writing::default();
    let (points, indices) = pictured.sheet();
    let normals = pictured.floats(3, &[0.0, 0.0, 1.0].repeat(4));
    let places = pictured.floats(2, &[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
    let picture = pictured.view(&quarters);
    pictured.glb(
        folder.path(),
        "pictured.glb",
        &format!(
            concat!(
                "{},\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{},\"NORMAL\":{},\"TEXCOORD_0\":{}}},",
                "\"indices\":{},\"material\":0}}]}}],",
                "\"materials\":[{{\"pbrMetallicRoughness\":{{\"baseColorTexture\":{{\"index\":0}}}}}}],",
                "\"textures\":[{{\"source\":0,\"sampler\":0}}],\"samplers\":[{{\"wrapS\":33071,\"wrapT\":33071}}],",
                "\"images\":[{{\"bufferView\":{},\"mimeType\":\"image/png\"}}]",
            ),
            SCENE, points, normals, places, indices, picture
        ),
    );
    // The same sheet with no picture, in its material's white.
    let mut plain = Writing::default();
    let (points, indices) = plain.sheet();
    let normals = plain.floats(3, &[0.0, 0.0, 1.0].repeat(4));
    plain.glb(
        folder.path(),
        "plain.glb",
        &format!(
            "{SCENE},\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{points},\"NORMAL\":{normals}}},\"indices\":{indices},\"material\":0}}]}}],\"materials\":[{{}}]"
        ),
    );

    // 180 across and 90 down about the middle, each quarter of the picture where it was laid.
    let frame = sheet_of(folder.path(), "pictured.glb", "", Pass::Picture);
    assert!(reaches(reach(&frame), [70, 45, 250, 135]), "{:?}", reach(&frame));
    assert_eq!(pixel(&frame, 115, 67), [255, 0, 0]);
    assert_eq!(pixel(&frame, 205, 67), [0, 255, 0]);
    assert_eq!(pixel(&frame, 115, 112), [0, 0, 255]);
    assert_eq!(pixel(&frame, 205, 112), [255, 255, 255]);
    // A light from the side dims the plain sheet, and leaves the picture as it is.
    let lit = sheet_of(folder.path(), "pictured.glb", SUN, Pass::Picture);
    assert!(lit == frame);
    let plain_lit = sheet_of(folder.path(), "plain.glb", SUN, Pass::Picture);
    assert_eq!(pixel(&sheet_of(folder.path(), "plain.glb", "", Pass::Picture), 115, 67), [255; 3]);
    assert!(pixel(&plain_lit, 115, 67)[0] < 200, "{:?}", pixel(&plain_lit, 115, 67));
    // In its passes it is the shape it is, whatever is laid over it.
    for pass in [Pass::Ids, Pass::Depth] {
        let with = sheet_of(folder.path(), "pictured.glb", "", pass);
        assert!(with == sheet_of(folder.path(), "plain.glb", "", pass));
        assert_eq!(pixel(&with, 115, 67), pixel(&with, 205, 112));
    }
}

/// A JPEG of 16 by 16, all of it one orange: 224, 128, 32.
const ORANGE: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x02, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xfe, 0x00, 0x0f, 0x4c, 0x61, 0x76, 0x63, 0x36, 0x33, 0x2e, 0x31,
    0x2e, 0x31, 0x30, 0x32, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x08, 0x08, 0x08, 0x07, 0x07, 0x07, 0x06, 0x06, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x09, 0x09, 0x09, 0x08, 0x08, 0x08, 0x08, 0x09, 0x09, 0x0a, 0x0a, 0x0a,
    0x0c, 0x0c, 0x0b, 0x0b, 0x0e, 0x0e, 0x0e, 0x11, 0x11, 0x14, 0xff, 0xc4, 0x00, 0x4c, 0x00, 0x01,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x05, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x06, 0x07, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00,
    0x10, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xda, 0x00, 0x0c, 0x03,
    0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0xa4, 0x02, 0x7e, 0x6e, 0xff, 0xd9,
];

#[test]
fn a_picture_beside_the_file_is_read_and_one_far_off_is_read_small() {
    let folder = tempfile::tempdir().expect("a folder");
    std::fs::write(folder.path().join("an orange.jpg"), ORANGE).unwrap();
    // Squares of black and white, a texel each: from far off they are an even grey.
    let checks = png(256, 256, |x, y| [if (x + y) % 2 == 0 { 255 } else { 0 }; 3]);
    std::fs::write(folder.path().join("checks.png"), checks).unwrap();
    for (picture, name) in [("an%20orange.jpg", "orange.gltf"), ("checks.png", "checks.gltf")] {
        let mut file = Writing::default();
        let (points, indices) = file.sheet();
        // With no normals told, and the picture laid twice across and twice down: it starts
        // again.
        let places = file.floats(2, &[0.0, 0.0, 2.0, 0.0, 2.0, 2.0, 0.0, 2.0]);
        file.gltf(
            folder.path(),
            name,
            &format!(
                concat!(
                    "{},\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{},\"TEXCOORD_0\":{}}},",
                    "\"indices\":{},\"material\":0}}]}}],",
                    "\"materials\":[{{\"pbrMetallicRoughness\":{{\"baseColorTexture\":{{\"index\":0}}}}}}],",
                    "\"textures\":[{{\"source\":0}}],\"images\":[{{\"uri\":\"{}\"}}]",
                ),
                SCENE, points, places, indices, picture
            ),
        );
    }
    let orange = sheet_of(folder.path(), "orange.gltf", "", Pass::Picture);
    for (x, y) in [(75, 50), (160, 90), (245, 130)] {
        let [r, g, b] = pixel(&orange, x, y);
        assert!(
            r.abs_diff(224) <= 3 && g.abs_diff(128) <= 3 && b.abs_diff(32) <= 3,
            "{:?}",
            pixel(&orange, x, y)
        );
    }
    // 512 texels across 180 pixels and down 90 are noise if each place reads the picture as it
    // is; read at the size they are on the screen they are grey, and half the light of white is
    // 188.
    let checks = sheet_of(folder.path(), "checks.gltf", "", Pass::Picture);
    for y in 50..130 {
        for x in 75..245 {
            let grey = pixel(&checks, x, y)[0];
            assert!(grey.abs_diff(188) <= 6, "{grey} at {x}, {y}");
        }
    }
    // A picture that is none is said, with which.
    std::fs::write(folder.path().join("checks.png"), b"\x89PNG and no more").unwrap();
    std::fs::write(
        folder.path().join("again.gltf"),
        std::fs::read(folder.path().join("checks.gltf")).unwrap(),
    )
    .unwrap();
    let said = from_file(folder.path(), "again.gltf", "", Pass::Picture)
        .err()
        .expect("no picture");
    assert!(said.contains("its picture 0"), "{said}");
}

#[test]
fn corners_given_colors_are_drawn_in_them_with_no_light() {
    let folder = tempfile::tempdir().expect("a folder");
    let mut file = Writing::default();
    let (points, indices) = file.sheet();
    // Red on the left and blue on the right, as bytes, with how clear each is.
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    let colors = file.shares(4, &[red, blue, blue, red].concat());
    file.glb(
        folder.path(),
        "colored.glb",
        &format!(
            "{SCENE},\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{points},\"COLOR_0\":{colors}}},\"indices\":{indices}}}]}}]"
        ),
    );
    let frame = sheet_of(folder.path(), "colored.glb", SUN, Pass::Picture);
    let [r, g, b] = pixel(&frame, 72, 90);
    assert!(r > 250 && g == 0 && b < 40, "{:?}", pixel(&frame, 72, 90));
    let [r, g, b] = pixel(&frame, 247, 90);
    assert!(r < 40 && g == 0 && b > 250, "{:?}", pixel(&frame, 247, 90));
    // Halfway it is half of each, in light: 188 of 255.
    let [r, _, b] = pixel(&frame, 160, 90);
    assert!(r.abs_diff(188) <= 2 && b.abs_diff(188) <= 2, "{r} {b}");
    // The clip's color is for a part with none of its own.
    let tinted = sheet_of(folder.path(), "colored.glb", "", Pass::Picture);
    assert!(tinted == sheet_of(folder.path(), "colored.glb", SUN, Pass::Picture));
}
