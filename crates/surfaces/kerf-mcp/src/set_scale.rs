//! How much of a set one unit of a camera's path is, found from the footage both are of.
//!
//! A set one model rebuilt of a frame and a path another found of the shot are each in units
//! of their own, and neither says how many of one are one of the other. The footage does. The
//! set carries the frame it was rebuilt from as its colors, so seen from where the path has
//! the camera on another frame it is a picture of that frame, where the size is right: at
//! half the size the camera has gone half as far into the set as the footage went into the
//! street, and what it draws is of an earlier moment. So the set is drawn through the path at
//! a run of sizes on a few frames far from the one it is of, each held to the footage's own
//! frame by its pixels, and the size at which they are most alike is the one.
//!
//! It needs no model, and it needs a camera that travels: one that only turns sees the set
//! the same at every size, which is said (`sure`).

use kerf_compose::{ComposeOptions, Compositor, FileView};
use kerf_model::{Catalog, Clip, Content, Id, Param, Project, Time, Track, Value};

/// The sizes tried first: each a quarter of a doubling from the last, from a sixty-fourth
/// to sixty-four.
const STEPS: i32 = 24;

/// How many frames the set is drawn on at each size.
const FRAMES: usize = 4;

/// The share of a frame the set has to cover for the frame to count.
const COVERED: f64 = 0.03;

/// A size found for a path against a set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fit {
    /// How much of the mesh's file one of the path's units is.
    pub scale: f64,
    /// How far the set's picture is from the footage at that size, as a mean over the pixels
    /// it covers, of 255.
    pub error: f64,
    /// The same at the nearer of half and twice that size, over `error`: how much worse a
    /// wrong size is. Near 1, any size fits as well, and the camera did not travel enough
    /// to tell.
    pub sure: f64,
    /// The frames of the path it was read on.
    pub frames: Vec<usize>,
}

/// What draws a mesh clip alone through a camera at a place in its file's space.
pub(crate) struct Seen<'a> {
    /// The project with its media where they are.
    pub project: &'a Project,
    pub sequence: usize,
    pub clip: &'a str,
    /// How much of the sequence's size is drawn.
    pub small: f64,
}

impl Seen<'_> {
    /// The project with the mesh's sequence holding the mesh clip alone, and a camera at
    /// `place` where one is given.
    fn alone(&self, place: Option<(&kerf_compose::CameraPlace, f64)>) -> Option<Project> {
        let mut project = self.project.clone();
        let sequence = project.sequences.get_mut(self.sequence)?;
        let mesh = sequence
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|clip| clip.id.as_str() == self.clip)?
            .clone();
        let model = sequence.tracks.iter().find(|track| {
            track.clips.iter().any(|clip| clip.id.as_str() == self.clip)
        })?;
        let track = |id: &str, clip: Clip| Track {
            id: Id::new(id).expect("a track's name"),
            clips: vec![clip],
            transitions: Vec::new(),
            ..model.clone()
        };
        let mut tracks = vec![track("V1", mesh.clone())];
        if let Some((place, fov)) = place {
            let mut camera = Clip::new(
                Id::new("fitcamera").expect("a clip's name"),
                Content::Generator {
                    fx: Id::new("camera").expect("a generator's name"),
                    version: 1,
                    params: Default::default(),
                },
                mesh.start,
                mesh.duration,
            );
            // Where it is is a clip's, and how it is turned and its lens are a camera's.
            for (name, value) in [("x", place.x), ("y", place.y), ("z", place.z)] {
                camera
                    .props
                    .insert(name.to_owned(), Param::Static(Value::Number(value)));
            }
            let Content::Generator { params, .. } = &mut camera.content else {
                return None;
            };
            for (name, value) in [
                ("pan", place.pan),
                ("tilt", place.tilt),
                ("roll", place.roll),
                ("fov", fov),
            ] {
                params.insert(name.to_owned(), Param::Static(Value::Number(value)));
            }
            tracks.push(track("V2", camera));
        }
        sequence.tracks = tracks;
        Some(project)
    }

    fn compositor(&self, project: &Project) -> Option<Compositor> {
        let sequence = project.sequences.get(self.sequence)?;
        Compositor::new(
            project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some(sequence.id.to_string()),
                ..ComposeOptions::default()
            },
        )
        .ok()
    }

    /// How wide and tall what it draws is.
    pub(crate) fn size(&self) -> Option<(u32, u32)> {
        let project = self.alone(None)?;
        let frame = self.compositor(&project)?.frame(self.start()?, self.small);
        Some((frame.width, frame.height))
    }

    fn start(&self) -> Option<Time> {
        let sequence = self.project.sequences.get(self.sequence)?;
        sequence
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|clip| clip.id.as_str() == self.clip)
            .map(|clip| clip.start)
    }

    /// The set as a camera at `view` in its file's space sees it through a lens of `fov`.
    fn through(&self, bare: &Compositor, view: &FileView, fov: f64) -> Option<Vec<u8>> {
        let at = self.start()?;
        let place = bare.mesh_view(self.clip, view, at)?;
        let project = self.alone(Some((&place, fov)))?;
        Some(self.compositor(&project)?.frame(at, self.small).rgba)
    }
}

/// How far `drawn` is from `footage` over the pixels `drawn` covers, of 255: none where it
/// covers too little to say.
fn apart(drawn: &[u8], footage: &[u8]) -> Option<f64> {
    let (mut sum, mut covered) = (0.0, 0usize);
    let pixels = drawn.len() / 4;
    for (one, other) in drawn.as_chunks::<4>().0.iter().zip(footage.as_chunks::<4>().0) {
        // What the set does not cover is drawn as nothing, which is black to the last bit.
        if one[..3] == [0, 0, 0] {
            continue;
        }
        covered += 1;
        sum += (0..3)
            .map(|channel| f64::from(one[channel].abs_diff(other[channel])))
            .sum::<f64>()
            / 3.0;
    }
    (covered as f64 >= COVERED * pixels as f64).then(|| sum / covered as f64)
}

/// The frames of a path to read a size on: those where the camera is furthest from where
/// it is on frame `origin`, spread over the way there. `eyes` is where it is on each, at
/// any one size.
pub(crate) fn frames_to_read(eyes: &[[f64; 3]], origin: usize) -> Vec<usize> {
    let from = eyes.get(origin).copied().unwrap_or_default();
    let gone = |eye: &[f64; 3]| {
        (0..3)
            .map(|axis| (eye[axis] - from[axis]).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let furthest = eyes.iter().map(gone).fold(0.0, f64::max);
    if furthest <= 0.0 {
        return Vec::new();
    }
    // The nearest frame to each of a few shares of the furthest it goes, either side.
    let mut frames = Vec::new();
    for step in 1..=FRAMES {
        let wanted = furthest * step as f64 / FRAMES as f64;
        for side in [0..origin, origin + 1..eyes.len()] {
            let nearest = side
                .filter(|frame| gone(&eyes[*frame]) >= furthest / (FRAMES as f64 * 2.0))
                .min_by(|a, b| {
                    (gone(&eyes[*a]) - wanted)
                        .abs()
                        .total_cmp(&(gone(&eyes[*b]) - wanted).abs())
                });
            if let Some(frame) = nearest.filter(|frame| !frames.contains(frame)) {
                frames.push(frame);
            }
        }
    }
    // The furthest say most.
    frames.sort_by(|a, b| gone(&eyes[*b]).total_cmp(&gone(&eyes[*a])));
    frames.truncate(FRAMES);
    frames.sort_unstable();
    frames
}

/// The lens through which `seen`, from `view`, is most like `footage`, the footage's picture
/// on the frame `view` is the camera of: degrees down the picture, with how far the set's
/// picture is from the footage there, and how many times worse a lens a quarter narrower
/// or wider is. A set is rebuilt along the rays of the lens its model took the frame to
/// have, and only through that lens is it the frame again; it says nothing of size, the
/// camera being where the set was seen from.
pub(crate) fn lens(seen: &Seen, view: &FileView, footage: &[u8]) -> Option<(f64, f64, f64)> {
    let bare = seen.compositor(&seen.alone(None)?)?;
    let far = |fov: f64| apart(&seen.through(&bare, view, fov)?, footage);
    // From 10 degrees to 115, an eighth of a doubling at a time.
    let step = |index: i32| 10.0 * 2f64.powf(f64::from(index) / 8.0);
    let (best, _) = (0..=28)
        .filter_map(|index| Some((index, far(step(index))?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let (mut low, mut high) = (f64::from(best - 1) / 8.0, f64::from(best + 1) / 8.0);
    for _ in 0..8 {
        let (one, other) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        match (far(10.0 * 2f64.powf(one)), far(10.0 * 2f64.powf(other))) {
            (Some(a), Some(b)) if a <= b => high = other,
            (Some(_), Some(_)) | (None, Some(_)) => low = one,
            (Some(_), None) => high = other,
            (None, None) => break,
        }
    }
    let fov = 10.0 * 2f64.powf((low + high) / 2.0);
    let error = far(fov)?;
    let beside = [far(fov / 1.25), far(fov * 1.25)]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
    Some((fov, error, beside / error.max(1e-9)))
}

/// The size at which `seen`, drawn through `views(scale)` on `frames`, is most like
/// `footage`, which holds the footage's picture on each of those frames at [`Seen::size`].
pub(crate) fn fit(
    seen: &Seen,
    fov: f64,
    frames: &[usize],
    footage: &[Vec<u8>],
    views: impl Fn(f64, usize) -> Option<FileView>,
) -> Result<Fit, String> {
    let bare = seen
        .alone(None)
        .and_then(|project| seen.compositor(&project))
        .ok_or("the set could not be drawn alone")?;
    let far = |scale: f64| -> Option<f64> {
        let mut sum = 0.0;
        for (frame, picture) in frames.iter().zip(footage) {
            let drawn = seen.through(&bare, &views(scale, *frame)?, fov)?;
            sum += apart(&drawn, picture)?;
        }
        Some(sum / frames.len().max(1) as f64)
    };
    let step = |index: i32| 2f64.powf(f64::from(index) / 4.0);
    let tried: Vec<(i32, f64)> = (-STEPS..=STEPS)
        .filter_map(|index| Some((index, far(step(index))?)))
        .collect();
    let Some((best, _)) = tried.iter().min_by(|a, b| a.1.total_cmp(&b.1)).copied() else {
        return Err(
            "the set is on too little of the frames read at any size from 1/64 to 64".to_owned(),
        );
    };
    // Between the sizes either side of the best, by thirds.
    let (mut low, mut high) = (f64::from(best - 1) / 4.0, f64::from(best + 1) / 4.0);
    for _ in 0..8 {
        let (one, other) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        let (at_one, at_other) = (far(2f64.powf(one)), far(2f64.powf(other)));
        match (at_one, at_other) {
            (Some(a), Some(b)) if a <= b => high = other,
            (Some(_), Some(_)) | (None, Some(_)) => low = one,
            (Some(_), None) => high = other,
            (None, None) => break,
        }
    }
    let scale = 2f64.powf((low + high) / 2.0);
    let error = far(scale).ok_or("the set is on too little of the frames read")?;
    let beside = [far(scale / 2.0), far(scale * 2.0)]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
    Ok(Fit {
        scale,
        error,
        sure: if beside.is_finite() && error > 0.0 {
            beside / error
        } else {
            f64::INFINITY
        },
        frames: frames.to_vec(),
    })
}

/// Where the origin of a mesh clip's file is in its sequence, as a camera's place is said,
/// and how many frame heights one of the file's units is.
fn file_in_sequence(project: &Project, sequence: usize, clip: &str) -> Option<([f64; 3], f64)> {
    let planned = project.sequences.get(sequence)?;
    let start = planned
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|found| found.id.as_str() == clip)?
        .start;
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(planned.id.to_string()),
            ..ComposeOptions::default()
        },
    )
    .ok()?;
    let at = |eye: [f64; 3]| {
        let place = compositor.mesh_view(
            clip,
            &FileView {
                eye,
                ..FileView::ORIGIN
            },
            start,
        )?;
        Some([place.x, place.y, place.z])
    };
    let (origin, up) = (at([0.0; 3])?, at([0.0, 1.0, 0.0])?);
    // A place's x is in frame widths, and the rest in heights.
    let wide = f64::from(planned.width) / f64::from(planned.height.max(1));
    let unit = ((up[0] - origin[0]) * wide)
        .hypot(up[1] - origin[1])
        .hypot(up[2] - origin[2]);
    Some((origin, unit))
}

fn clip_of<'a>(project: &'a mut Project, sequence: usize, id: &str) -> Option<&'a mut Clip> {
    project
        .sequences
        .get_mut(sequence)?
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
        .find(|clip| clip.id.as_str() == id)
}

/// The line that puts the file of mesh clip `clip` where the file of `other` is, the two
/// files being in one space: as tall as `other` has a unit of its file, and moved until the
/// two origins are one place.
pub(crate) fn beside(
    project: &Project,
    sequence: usize,
    clip: &str,
    other: &str,
) -> Result<String, String> {
    let mut project = project.clone();
    let number = |param: Option<&Param>, otherwise: f64| match param {
        None => Some(otherwise),
        Some(Param::Static(Value::Number(value))) => Some(*value),
        Some(_) => None,
    };
    for id in [clip, other] {
        let found = clip_of(&mut project, sequence, id).ok_or(format!("no clip `{id}`"))?;
        let turned = ["rotate_x", "rotate_y", "rotation"]
            .iter()
            .any(|name| number(found.props.get(*name), 0.0) != Some(0.0));
        let keyed = ["x", "y", "z"]
            .iter()
            .any(|name| number(found.props.get(*name), 0.0).is_none());
        if turned || keyed || found.parent.is_some() {
            return Err(format!(
                "`{id}` is turned, keyed, or moves with another clip: `beside` places a mesh that stands still and square"
            ));
        }
    }
    let wanted = |project: &Project, id: &str| {
        file_in_sequence(project, sequence, id).ok_or(format!(
            "`{id}`: it is no `gen=mesh@1` clip with a `file`"
        ))
    };
    let (to, unit) = wanted(&project, other)?;
    let (_, own) = wanted(&project, clip)?;
    let default = Catalog::builtin()
        .fx("mesh")
        .and_then(|def| def.find("height"))
        .and_then(|def| def.default.as_number())
        .unwrap_or(1.0);
    let moved = clip_of(&mut project, sequence, clip).ok_or("no clip")?;
    let Content::Generator { params, .. } = &mut moved.content else {
        return Err(format!("`{clip}` is no mesh"));
    };
    let height = number(params.get("height"), default)
        .ok_or(format!("the `height` of `{clip}` is keyed"))?
        * unit
        / own;
    params.insert("height".to_owned(), Param::Static(Value::Number(height)));
    // A place is not quite a straight line of a clip's own, so twice more from where it lands.
    let mut place = [0.0; 3];
    for _ in 0..3 {
        let (from, _) = wanted(&project, clip)?;
        let moved = clip_of(&mut project, sequence, clip).ok_or("no clip")?;
        for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
            place[axis] = number(moved.props.get(name), 0.0).unwrap_or(0.0) + to[axis] - from[axis];
            moved
                .props
                .insert(name.to_owned(), Param::Static(Value::Number(place[axis])));
        }
    }
    let rounded = |value: f64| (value * 1e5).round() / 1e5 + 0.0;
    Ok(format!(
        "set {clip} height={} x={} y={} z={}",
        rounded(height),
        rounded(place[0]),
        rounded(place[1]),
        rounded(place[2])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frames_read_are_the_far_ones_either_side_of_the_sets() {
        // A camera going straight ahead a unit a frame, the set being of the fifth frame.
        let eyes: Vec<[f64; 3]> = (0..11).map(|frame| [0.0, 0.0, -f64::from(frame)]).collect();
        let frames = frames_to_read(&eyes, 4);
        assert_eq!(frames.len(), FRAMES);
        assert!(frames.contains(&10) && !frames.contains(&4), "{frames:?}");
        assert!(frames.iter().all(|frame| *frame < 3 || *frame > 5), "{frames:?}");
        // One that does not move has none.
        assert!(frames_to_read(&[[1.0, 2.0, 3.0]; 5], 0).is_empty());
    }

    /// A set of three walls, each a color, as a `.glb`: a wide one far off, a floor, and a
    /// post near the camera, which is at the file's origin looking along its z backward.
    fn set(folder: &std::path::Path) -> std::path::PathBuf {
        let quad = |corners: [[f32; 3]; 4]| {
            [0, 1, 2, 0, 2, 3].map(|corner| corners[corner]).concat()
        };
        let walls = [
            quad([[-4.0, 2.0, -8.0], [4.0, 2.0, -8.0], [4.0, -1.0, -8.0], [-4.0, -1.0, -8.0]]),
            quad([[-4.0, -1.0, -8.0], [4.0, -1.0, -8.0], [4.0, -1.0, -1.0], [-4.0, -1.0, -1.0]]),
            quad([[0.3, 0.5, -3.0], [0.8, 0.5, -3.0], [0.8, -1.0, -3.0], [0.3, -1.0, -3.0]]),
        ];
        let bytes: Vec<u8> = walls
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let colors = ["0,0,1,1", "0,1,0,1", "1,0,0,1"];
        let part = |index: usize| {
            format!("{{\"attributes\":{{\"POSITION\":{index}}},\"material\":{index}}}")
        };
        let mut json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0}}],\"meshes\":[{{\"primitives\":[{}]}}],",
                "\"materials\":[{}],\"buffers\":[{{\"byteLength\":216}}],",
                "\"bufferViews\":[{}],\"accessors\":[{}]}}"
            ),
            [0, 1, 2].map(part).join(","),
            colors
                .map(|color| format!(
                    "{{\"pbrMetallicRoughness\":{{\"baseColorFactor\":[{color}]}}}}"
                ))
                .join(","),
            [0, 1, 2]
                .map(|index| format!(
                    "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":72}}",
                    index * 72
                ))
                .join(","),
            [0, 1, 2]
                .map(|index| format!(
                    "{{\"bufferView\":{index},\"componentType\":5126,\"count\":6,\"type\":\"VEC3\"}}"
                ))
                .join(","),
        )
        .into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut file = b"glTF".to_vec();
        file.extend(2u32.to_le_bytes());
        file.extend((12 + 8 + json.len() as u32 + 8 + bytes.len() as u32).to_le_bytes());
        file.extend((json.len() as u32).to_le_bytes());
        file.extend(b"JSON");
        file.extend(&json);
        file.extend((bytes.len() as u32).to_le_bytes());
        file.extend(b"BIN\0");
        file.extend(&bytes);
        let path = folder.join("set.glb");
        std::fs::write(&path, file).unwrap();
        path
    }

    #[test]
    fn the_size_a_path_was_filmed_at_is_found_from_its_footage() {
        let folder = tempfile::tempdir().unwrap();
        let file = set(folder.path());
        let project = kerf_lang::parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip other gen=solid@1 at=0f dur=24f\n  track V2 video\n    clip set gen=mesh@1 at=0f dur=24f\n      set file=\"{}\" height=0.5\n",
                file.display()
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        let seen = Seen {
            project: &project,
            sequence: 0,
            clip: "set",
            small: 1.0,
        };
        assert_eq!(seen.size(), Some((160, 90)));
        // A path of five frames in units half the set's: it goes to the right and ahead.
        let eyes: Vec<[f64; 3]> = (0..5)
            .map(|frame| [0.2 * f64::from(frame), 0.0, -0.25 * f64::from(frame)])
            .collect();
        let views = |scale: f64, frame: usize| {
            Some(FileView {
                eye: eyes.get(frame)?.map(|side| side * scale),
                ..FileView::ORIGIN
            })
        };
        let frames = frames_to_read(&eyes, 0);
        assert_eq!(frames, [1, 2, 3, 4]);
        // The footage is what was there to film: the set, from where the camera was.
        let bare = seen.compositor(&seen.alone(None).unwrap()).unwrap();
        let footage: Vec<Vec<u8>> = frames
            .iter()
            .map(|frame| seen.through(&bare, &views(2.0, *frame).unwrap(), 40.0).unwrap())
            .collect();
        // Alone: the solid on the track under it is not in what is drawn.
        assert!(footage[0].as_chunks::<4>().0.iter().any(|pixel| pixel[..3] == [0, 0, 0]));
        let found = fit(&seen, 40.0, &frames, &footage, views).expect("a size");
        assert!((found.scale - 2.0).abs() < 0.05, "{found:?}");
        assert!(found.error < 1.0 && found.sure > 2.0, "{found:?}");
        assert_eq!(found.frames, frames);
        // The lens is found from the frame the set is of, where size says nothing.
        let own = seen.through(&bare, &FileView::ORIGIN, 40.0).unwrap();
        let (fov, error, sure) = lens(&seen, &FileView::ORIGIN, &own).expect("a lens");
        assert!((fov - 40.0).abs() < 0.5 && error < 1.0 && sure > 2.0, "{fov} {error} {sure}");
        // Through another lens nothing fits as well, and it is still the nearest size.
        let other = fit(&seen, 55.0, &frames, &footage, views).expect("a size");
        assert!(other.error > found.error, "{other:?}");
    }

    #[test]
    fn a_mesh_is_put_where_another_of_the_same_space_is() {
        let folder = tempfile::tempdir().unwrap();
        let file = set(folder.path());
        let scene = |more: &str| {
            kerf_lang::parse_document(
                &format!(
                    "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip first gen=mesh@1 at=0f dur=24f\n      set file=\"{file}\" height=0.5 x=0.2 y=-0.1 z=0.3\n  track V2 video\n    clip second gen=mesh@1 at=0f dur=24f\n      set file=\"{file}\" {more}\n",
                    file = file.display()
                ),
                Catalog::builtin(),
            )
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project
        };
        // The same file is in its own space, so it is put exactly where the first is.
        let line = beside(&scene("height=2 x=-1"), 0, "second", "first").expect("a line");
        assert_eq!(line, "set second height=0.5 x=0.2 y=-0.1 z=0.3");
        // One that is turned is not placed, and one that is no mesh is said.
        let turned = beside(&scene("rotate_y=30"), 0, "second", "first").unwrap_err();
        assert!(turned.contains("stands still and square"), "{turned}");
        assert!(beside(&scene(""), 0, "second", "nobody").is_err());
    }

    #[test]
    fn a_picture_is_held_to_another_where_it_covers_enough() {
        let footage = [10u8, 20, 30, 255].repeat(100);
        let mut drawn = [0u8, 0, 0, 255].repeat(100);
        assert_eq!(apart(&drawn, &footage), None);
        for pixel in 0..10 {
            drawn[pixel * 4..pixel * 4 + 3].copy_from_slice(&[13, 20, 24]);
        }
        assert_eq!(apart(&drawn, &footage), Some(3.0));
    }
}
