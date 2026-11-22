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
        let model = sequence
            .tracks
            .iter()
            .find(|track| track.clips.iter().any(|clip| clip.id.as_str() == self.clip))?;
        let track = |id: &str, clip: Clip| Track {
            id: Id::new(id).expect("called only with \"V1\" and \"V2\", which are valid ids"),
            clips: vec![clip],
            transitions: Vec::new(),
            ..model.clone()
        };
        let mut tracks = vec![track("V1", mesh.clone())];
        if let Some((place, fov)) = place {
            let mut camera = Clip::new(
                Id::new("fitcamera").expect("\"fitcamera\" is a valid id"),
                Content::Generator {
                    fx: Id::new("camera").expect("\"camera\" is a valid id"),
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
    for (one, other) in drawn
        .as_chunks::<4>()
        .0
        .iter()
        .zip(footage.as_chunks::<4>().0)
    {
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

/// How many times further a set is along the way it was seen than a path of the same shot
/// takes it to be, the set's model having taken the frame to be through a lens of `set`
/// degrees and the path's through `path`: a picture through a narrower lens is of things
/// further off, by the halves of the two lenses.
pub(crate) fn deeper(path: f64, set: f64) -> f64 {
    let half = |fov: f64| (fov.clamp(1.0, 179.0).to_radians() / 2.0).tan();
    half(path) / half(set)
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
        file_in_sequence(project, sequence, id)
            .ok_or(format!("`{id}`: it is no `gen=mesh@1` clip with a `file`"))
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
