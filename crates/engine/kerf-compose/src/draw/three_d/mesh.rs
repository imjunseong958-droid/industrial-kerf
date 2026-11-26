//! A clip whose picture is a mesh: a thing with more than one side, for a guide.
//!
//! A clip in depth is a plane, or with a `depth` its outline pushed straight back, and each output
//! pixel's ray meets it where the plane says. That does not do for a mesh, which wants triangles
//! and a depth for each pixel. So a mesh is drawn as EffectCraft's `render/three_d/adv/raster.rs`
//! drew one, carried into Kerf's terms and without its materials: triangles are cut where they
//! come nearer than the camera sees, put on the screen, and scanned with edge functions (pixel
//! centers, the top and left edges a triangle's own), keeping at each place the nearest, with
//! weights that are right in perspective. What comes of it is what a plane's rays give, a picture
//! and how far along the camera's way each pixel is, so that a mesh goes down among the other
//! clips in depth, and into the ids and depth passes, as they do.
//!
//! A part in one flat color is lit by the scene's lights by the way each place faces. A part
//! with colors of its own, a picture its file lays over it or a color at each corner, is drawn in
//! them as they are, with no light: they are what the thing is, and a set rebuilt from a shot
//! has the shot's light in them already. Its triangles are drawn on the CPU, and where the
//! rest of a frame is the GPU's the picture and how far off it is are handed over to be laid
//! with it (`Drawing::gpu_crossing`), but for a frame whose lights cast shadows or shine
//! from a picture.
//!
//! Its clip's masks, effects, and wipe are done to that picture, the mesh as the camera sees
//! it on the canvas (`Drawing::mesh`), there being no flat picture of it before that.
//!
//! It stands in a light's way as a clip in depth does ([`Standing`]): a ray from a place to a
//! light is stopped by the first of its triangles it meets, found through a tree of boxes
//! about them, and not by the mesh the place is on. The meshes of a scene are in such a tree
//! themselves ([`InTheWay`]), so that a ray asks the few it comes to. A light with a size
//! sends many rays from each place, and a mesh is asked for them only where one of its
//! triangles is between the four rays around the light ([`Beam`]): a place in plain light
//! walks the trees once and sends no ray.
//!
//! It is one of a few solids, a box, a ball, a cylinder, or the triangles of a glTF file
//! ([`super::gltf`]), as tall as the clip says and in the file's own proportions. A file's
//! animation moves it where the clip names one: its nodes, and the bones its skin bends with, are
//! where the animation has them as many seconds in as the clip has run, or as its `seconds` says.
//!
//! A mesh stands where its clip's 2D geometry puts the middle of the frame, turned and pushed back
//! by `z`, `rotate_x`, and `rotate_y` as a plane is. Its own space runs x right, y down, and z away
//! from the viewer, in frame heights.

use super::texture::Wrap;
use super::*;

/// How many places across and down a pixel are read, for an edge that is not jagged.
const SAMPLES: usize = 2;
/// How many sides a ball or a cylinder has around, and how many bands a ball has from top to bottom.
const AROUND: usize = 48;
const BANDS: usize = 24;
/// No triangle.
const NONE: u32 = u32::MAX;

/// Triangles, with the way each corner faces, in runs of one color.
pub(in crate::draw) struct Mesh {
    points: Vec<Vector>,
    normals: Vec<Vector>,
    triangles: Vec<[u32; 3]>,
    parts: Vec<Part>,
    /// The file it was read from, which has its parts' pictures and its corners' colors and
    /// places on them, corner for corner.
    file: Option<Arc<super::gltf::Read>>,
}

/// A run of triangles of one material.
#[derive(Debug, PartialEq)]
struct Part {
    /// The triangle after its last.
    end: usize,
    /// Its color, in linear light.
    color: [f32; 3],
    /// The picture laid over it: which of its file's, and what it does past its edges.
    texture: Option<(usize, [Wrap; 2])>,
    /// Whether it is drawn in its own colors, with no light.
    own: bool,
}

/// What a mesh is drawn as.
pub(in crate::draw) enum Shade<'a> {
    /// Its colors, lit as the clip `0` is.
    Lit(&'a Id),
    /// One color all over, as an ids pass has it, with a hard edge.
    Flat([f32; 3]),
    /// As grey as it is near, as a depth pass has it.
    Near,
    /// Colored by the way each place faces, as a normals pass has it.
    Facing,
    /// Colored by how far each place goes on the screen in a frame, as a motion pass has it:
    /// to where the scene `0` sees it a frame on, the clip placed as `1` has it then, a frame
    /// being `2` long, by a frame `3` tall.
    Moving(&'a Scene, &'a Affine, Time, f64),
}

impl Mesh {
    fn new() -> Self {
        Self {
            points: Vec::new(),
            normals: Vec::new(),
            triangles: Vec::new(),
            parts: Vec::new(),
            file: None,
        }
    }

    fn corner(&mut self, point: Vector, normal: Vector) -> u32 {
        self.points.push(point);
        self.normals.push(normal);
        (self.points.len() - 1) as u32
    }

    /// Four corners in order round a flat side, as two triangles.
    fn side(&mut self, corners: [Vector; 4], normal: Vector) {
        let [a, b, c, d] = corners.map(|corner| self.corner(corner, normal));
        self.triangles.push([a, b, c]);
        self.triangles.push([a, c, d]);
    }

    fn finished(mut self, color: [f32; 3]) -> Self {
        self.parts.push(Part {
            end: self.triangles.len(),
            color,
            texture: None,
            own: false,
        });
        self
    }

    /// A box `size` across, down, and back, about its middle.
    fn a_box([w, h, l]: Vector, color: [f32; 3]) -> Self {
        let (x, y, z) = (w / 2.0, h / 2.0, l / 2.0);
        let mut mesh = Self::new();
        // Front and back, left and right, top and bottom.
        mesh.side(
            [[-x, -y, -z], [x, -y, -z], [x, y, -z], [-x, y, -z]],
            [0.0, 0.0, -1.0],
        );
        mesh.side(
            [[x, -y, z], [-x, -y, z], [-x, y, z], [x, y, z]],
            [0.0, 0.0, 1.0],
        );
        mesh.side(
            [[-x, -y, z], [-x, -y, -z], [-x, y, -z], [-x, y, z]],
            [-1.0, 0.0, 0.0],
        );
        mesh.side(
            [[x, -y, -z], [x, -y, z], [x, y, z], [x, y, -z]],
            [1.0, 0.0, 0.0],
        );
        mesh.side(
            [[-x, -y, z], [x, -y, z], [x, -y, -z], [-x, -y, -z]],
            [0.0, -1.0, 0.0],
        );
        mesh.side(
            [[-x, y, -z], [x, y, -z], [x, y, z], [-x, y, z]],
            [0.0, 1.0, 0.0],
        );
        mesh.finished(color)
    }

    /// A ball that fills a box of `size`: round when its three sizes are one.
    fn a_ball([w, h, l]: Vector, color: [f32; 3]) -> Self {
        let (a, b, c) = (
            (w / 2.0).max(1e-9),
            (h / 2.0).max(1e-9),
            (l / 2.0).max(1e-9),
        );
        let mut mesh = Self::new();
        for band in 0..=BANDS {
            let (ring, up) = (band as f64 / BANDS as f64 * std::f64::consts::PI).sin_cos();
            for step in 0..=AROUND {
                let (sin, cos) = (step as f64 / AROUND as f64 * std::f64::consts::TAU).sin_cos();
                let way = [ring * cos, -up, ring * sin];
                // A squashed ball faces more along the way it is squashed.
                let normal = unit([way[0] / a, way[1] / b, way[2] / c]);
                mesh.corner([way[0] * a, way[1] * b, way[2] * c], normal);
            }
        }
        let row = (AROUND + 1) as u32;
        for band in 0..BANDS as u32 {
            for step in 0..AROUND as u32 {
                let (here, below) = (band * row + step, (band + 1) * row + step);
                if band > 0 {
                    mesh.triangles.push([here, below, here + 1]);
                }
                if band + 1 < BANDS as u32 {
                    mesh.triangles.push([here + 1, below, below + 1]);
                }
            }
        }
        mesh.finished(color)
    }

    /// A cylinder standing in a box of `size`, its ends at the top and the bottom.
    fn a_cylinder([w, h, l]: Vector, color: [f32; 3]) -> Self {
        let (a, y, c) = ((w / 2.0).max(1e-9), h / 2.0, (l / 2.0).max(1e-9));
        let mut mesh = Self::new();
        let round: Vec<(f64, f64)> = (0..=AROUND)
            .map(|step| (step as f64 / AROUND as f64 * std::f64::consts::TAU).sin_cos())
            .collect();
        for pair in round.windows(2) {
            let [(sin, cos), (next_sin, next_cos)] = [pair[0], pair[1]];
            let facing = |cos: f64, sin: f64| unit([cos / a, 0.0, sin / c]);
            let top = mesh.corner([cos * a, -y, sin * c], facing(cos, sin));
            let bottom = mesh.corner([cos * a, y, sin * c], facing(cos, sin));
            let next_top =
                mesh.corner([next_cos * a, -y, next_sin * c], facing(next_cos, next_sin));
            let next_bottom =
                mesh.corner([next_cos * a, y, next_sin * c], facing(next_cos, next_sin));
            mesh.triangles.push([top, bottom, next_top]);
            mesh.triangles.push([next_top, bottom, next_bottom]);
            for (end, normal) in [(-y, [0.0, -1.0, 0.0]), (y, [0.0, 1.0, 0.0])] {
                let middle = mesh.corner([0.0, end, 0.0], normal);
                let here = mesh.corner([cos * a, end, sin * c], normal);
                let next = mesh.corner([next_cos * a, end, next_sin * c], normal);
                mesh.triangles.push([middle, here, next]);
            }
        }
        mesh.finished(color)
    }

    /// What was read of a file, about the middle of the box it fills at rest and `height` tall
    /// there, with `color` where a part has no material: at rest, or as `pose` has it, an
    /// animation of the file's and how many seconds into it.
    fn fitted(
        read: &Arc<super::gltf::Read>,
        height: f64,
        color: [f32; 3],
        pose: Option<(usize, f64)>,
    ) -> Self {
        let (middle, _, tall) = fit(read);
        let (points, normals) = match pose {
            Some(pose) => read.posed(Some(pose)),
            None => (read.points.clone(), read.normals.clone()),
        };
        Self {
            points: points
                .iter()
                .map(|point| times(minus(*point, middle), height / tall))
                .collect(),
            normals,
            triangles: read.triangles.clone(),
            parts: read
                .parts
                .iter()
                .map(|part| Part {
                    end: part.end,
                    // A picture or a corner's colors with no material are as they are.
                    color: part
                        .color
                        .unwrap_or(if part.texture.is_some() || part.colored {
                            [1.0; 3]
                        } else {
                            color
                        }),
                    texture: part.texture,
                    own: part.texture.is_some() || part.colored,
                })
                .collect(),
            file: Some(read.clone()),
        }
    }

    /// The mesh of a `gen=mesh` clip at sequence time `at`.
    pub(in crate::draw) fn of(clip: &Clip, at: Time) -> Self {
        let def = Catalog::builtin().fx("mesh");
        let read = |name: &str| {
            generated(def, clip, name, at)
                .and_then(|value| value.as_number())
                .unwrap_or(0.0)
                .max(0.0)
        };
        let size = [read("width"), read("height"), read("length")];
        let color = match generated(def, clip, "color", at) {
            Some(Value::Color(color)) => color,
            _ => Rgba::WHITE,
        };
        let [r, g, b, _] = premultiplied(Rgba { a: 255, ..color });
        if let Some(path) = file_of(clip) {
            // A file that cannot be read was said when the sequence was planned.
            return match loaded(&path) {
                Ok(read) => {
                    let pose = animation_of(clip, &read)
                        .and_then(|which| which.ok())
                        .map(|which| (which, seconds_into(clip, def, &read, which, at)));
                    Self::fitted(&read, size[1], [r, g, b], pose)
                }
                Err(_) => Self::new(),
            };
        }
        match generated(def, clip, "kind", at) {
            Some(Value::Enum(kind)) if kind == "ball" => Self::a_ball(size, [r, g, b]),
            Some(Value::Enum(kind)) if kind == "cylinder" => Self::a_cylinder(size, [r, g, b]),
            _ => Self::a_box(size, [r, g, b]),
        }
    }

    /// Whether its surface closes round it: a file's mesh none of whose edges is open, and a
    /// box, a ball, or a cylinder, which are made closed, unless it has no size one way and
    /// so nothing inside it.
    fn closed(&self) -> bool {
        match &self.file {
            Some(read) => read.closed(),
            None => (0..3).all(|axis| {
                let along = self.points.iter().map(|point| point[axis]);
                along.clone().fold(f64::NEG_INFINITY, f64::max)
                    > along.fold(f64::INFINITY, f64::min)
            }),
        }
    }

    fn part_of(&self, triangle: usize) -> Option<&Part> {
        let part = self.parts.partition_point(|part| part.end <= triangle);
        self.parts.get(part)
    }

    /// The color of `part` where its `triangle` is `shares` of each of its corners: its picture
    /// there, at the size `sizes` has for the triangle, times its corners' colors and its own.
    fn own_color(&self, part: &Part, triangle: usize, shares: [f64; 3], sizes: &[f32]) -> [f32; 3] {
        let Some(file) = &self.file else {
            return part.color;
        };
        let mut color = part.color;
        let corners = self.triangles[triangle];
        for (channel, color) in color.iter_mut().enumerate() {
            *color *= corners
                .iter()
                .zip(shares)
                .map(|(corner, share)| file.colors[*corner as usize][channel] * share as f32)
                .sum::<f32>();
        }
        if let Some((texture, wrap)) = part.texture {
            let places = self.places(triangle);
            let place = [0, 1].map(|axis| {
                (0..3)
                    .map(|corner| shares[corner] * places[corner][axis])
                    .sum::<f64>()
            });
            let size = sizes.get(triangle).copied().unwrap_or(0.0);
            let read = file.textures[texture].at(place, size, wrap);
            for (color, read) in color.iter_mut().zip(read) {
                *color *= read;
            }
        }
        color
    }

    /// Where on its part's picture each corner of `triangle` is.
    fn places(&self, triangle: usize) -> [[f64; 2]; 3] {
        self.triangles[triangle].map(|corner| {
            self.file
                .as_ref()
                .map_or([0.0; 2], |file| file.uvs[corner as usize].map(f64::from))
        })
    }
}

/// The box a file's mesh fills at rest: its middle, its size, and how tall it is taken to be.
fn fit(read: &super::gltf::Read) -> (Vector, Vector, f64) {
    let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for point in &read.points {
        for axis in 0..3 {
            least[axis] = least[axis].min(point[axis]);
            most[axis] = most[axis].max(point[axis]);
        }
    }
    let size = minus(most, least);
    // Flat as a sheet lying down, it is as wide as it was to be tall.
    let tall = if size[1] > 1e-12 {
        size[1]
    } else {
        size[0].max(size[2]).max(1e-12)
    };
    (times(add(least, most), 0.5), size, tall)
}

/// What a `gen=mesh` clip's file holds besides its triangles, for one who is to write of it.
pub(in crate::draw) fn holds(clip: &Clip) -> std::result::Result<crate::MeshFile, String> {
    let path = file_of(clip).ok_or("it is of no file")?;
    let read = loaded(&path).map_err(|why| format!("`{path}` cannot be read: {why}"))?;
    let (_, size, tall) = fit(&read);
    Ok(crate::MeshFile {
        triangles: read.triangles.len(),
        size: size.map(|side| side / tall),
        animations: read
            .animations
            .iter()
            .map(|animation| (animation.name.clone(), animation.length))
            .collect(),
        playing: animation_of(clip, &read).and_then(|which| which.ok()),
        cameras: read
            .cameras
            .iter()
            .map(|camera| crate::MeshCamera {
                name: camera.name.clone(),
                fov: camera.fov,
                aspect: camera.aspect,
            })
            .collect(),
    })
}

/// Where camera `which` of the file of the mesh clip of `plan` is at clip time `time`, as a
/// `gen=camera@1` clip of the same sequence says it: the mesh being where its clip puts it, and
/// its file's animation as far in as the clip has it.
pub(in crate::draw) fn camera(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
    which: usize,
) -> Option<crate::CameraPlace> {
    let clip = &plan.clip;
    let read = loaded(&file_of(clip)?).ok()?;
    let def = Catalog::builtin().fx("mesh");
    let at = clip.start + time;
    let pose = animation_of(clip, &read)
        .and_then(|which| which.ok())
        .map(|which| (which, seconds_into(clip, def, &read, which, at)));
    let seen = read.seen_from(which, pose)?;
    let fov = read.cameras[which].fov;
    Some(placed(scene, plan, to_output, time, &read, &seen, fov))
}

/// Where a camera that is at `view` in the space of the file of the mesh clip of `plan` is at
/// clip time `time`, as a `gen=camera@1` clip of the same sequence says it. It is a camera the
/// file does not hold: the one a set was seen by, or one found from a shot's frames.
pub(in crate::draw) fn view(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
    view: &crate::FileView,
) -> Option<crate::CameraPlace> {
    let read = loaded(&file_of(&plan.clip)?).ok()?;
    // glTF's y is up and its z toward the viewer, as the file's own points were.
    let turned = |of: Vector| [of[0], -of[1], -of[2]];
    let seen = super::gltf::Seen {
        eye: turned(view.eye),
        right: turned(view.right),
        forward: turned(view.forward),
    };
    Some(placed(scene, plan, to_output, time, &read, &seen, None))
}

/// A camera `seen` in the space of a mesh clip's file, as a camera of the clip's sequence.
fn placed(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
    read: &super::gltf::Read,
    seen: &super::gltf::Seen,
    fov: Option<f64>,
) -> crate::CameraPlace {
    let clip = &plan.clip;
    let def = Catalog::builtin().fx("mesh");
    let at = clip.start + time;
    let height = generated(def, clip, "height", at)
        .and_then(|value| value.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let (middle, _, tall) = fit(read);
    let size = (f64::from(plan.size.0), f64::from(plan.size.1));
    let plane = Plane::of(scene, plan, size, to_output, time);
    let ways = [
        times(plane.across, size.1),
        times(plane.down, size.1),
        times(plane.normal, plane.pixel_size() * size.1),
    ];
    let along = |of: Vector| {
        add(
            add(times(ways[0], of[0]), times(ways[1], of[1])),
            times(ways[2], of[2]),
        )
    };
    let eye = add(
        plane.middle,
        along(times(minus(seen.eye, middle), height / tall)),
    );
    let (right, forward) = (unit(along(seen.right)), unit(along(seen.forward)));
    // As `Scene::at` turns a camera: tilted, then panned, then rolled about its own line.
    let tilt = (-forward[1]).clamp(-1.0, 1.0).asin();
    let pan = forward[0].atan2(forward[2]);
    let level_right = turned([1.0, 0.0, 0.0], tilt, pan);
    let level_down = cross(forward, level_right);
    let roll = dot(right, level_down).atan2(dot(right, level_right));
    let (wide, tall) = (scene.center.0 * 2.0, scene.height);
    let back = tall / 2.0 / (DEFAULT_FOV.to_radians() / 2.0).tan();
    crate::CameraPlace {
        x: eye[0] / wide,
        y: eye[1] / tall,
        z: (eye[2] + back) / tall,
        pan: pan.to_degrees(),
        tilt: tilt.to_degrees(),
        roll: roll.to_degrees(),
        fov,
    }
}

/// The file a `gen=mesh` clip reads its triangles from, when it names one.
fn file_of(clip: &Clip) -> Option<String> {
    let Content::Generator { params, .. } = &clip.content else {
        return None;
    };
    match params.get("file") {
        Some(Param::Static(Value::Text(path))) if !path.is_empty() => Some(path.clone()),
        _ => None,
    }
}

/// Which of its file's animations a `gen=mesh` clip plays, where it names one: by what the file
/// calls it, or by its number from 0. An error says which the file has.
fn animation_of(
    clip: &Clip,
    read: &super::gltf::Read,
) -> Option<std::result::Result<usize, String>> {
    let Content::Generator { params, .. } = &clip.content else {
        return None;
    };
    let Some(Param::Static(Value::Text(name))) = params.get("animation") else {
        return None;
    };
    if name.is_empty() {
        return None;
    }
    let named = read
        .animations
        .iter()
        .position(|animation| animation.name == *name);
    let numbered = name
        .parse::<usize>()
        .ok()
        .filter(|number| *number < read.animations.len());
    Some(named.or(numbered).ok_or_else(|| {
        let has: Vec<String> = read
            .animations
            .iter()
            .enumerate()
            .map(|(number, animation)| match animation.name.as_str() {
                "" => format!("`{number}`"),
                name => format!("`{name}`"),
            })
            .collect();
        match has.len() {
            0 => format!("has no animation, and `{name}` is asked for"),
            _ => format!("has no animation `{name}`: it has {}", has.join(", ")),
        }
    }))
}

/// How many seconds into animation `which` of its file a `gen=mesh` clip is at sequence time
/// `at`: what its `seconds` says, or with none as long as the clip has run; from the start again
/// past its end where it loops, and held at its end where it does not.
fn seconds_into(
    clip: &Clip,
    def: Option<&FxDef>,
    read: &super::gltf::Read,
    which: usize,
    at: Time,
) -> f64 {
    let told = match &clip.content {
        Content::Generator { params, .. } => params.get("seconds"),
        _ => None,
    };
    let seconds = match told {
        Some(told) => told.value_at(at - clip.start).as_number().unwrap_or(0.0),
        None => (at - clip.start).seconds(),
    };
    let length = read.animations[which].length;
    let loops = !matches!(generated(def, clip, "loop", at), Some(Value::Bool(false)));
    if loops && length > 0.0 && !(0.0..=length).contains(&seconds) {
        seconds.rem_euclid(length)
    } else {
        seconds.clamp(0.0, length)
    }
}

/// What a file holds, read once for as long as the file is what it was.
fn loaded(path: &str) -> std::result::Result<Arc<super::gltf::Read>, String> {
    type Kept = (u64, Option<std::time::SystemTime>, Arc<super::gltf::Read>);
    static FILES: std::sync::Mutex<Vec<(String, Kept)>> = std::sync::Mutex::new(Vec::new());
    let file = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let now = (file.len(), file.modified().ok());
    let mut files = FILES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, (length, modified, read))) = files.iter().find(|(kept, _)| kept == path)
        && (*length, *modified) == now
    {
        return Ok(read.clone());
    }
    let read = Arc::new(super::gltf::read(std::path::Path::new(path))?);
    files.retain(|(kept, _)| kept != path);
    files.push((path.to_owned(), (now.0, now.1, read.clone())));
    Ok(read)
}

/// Whether the file a `gen=mesh` clip names can be read, and why not.
pub(in crate::draw) fn check(clip: &Clip) -> std::result::Result<(), String> {
    match file_of(clip) {
        Some(path) => {
            let read = loaded(&path).map_err(|why| format!("`{path}` cannot be read: {why}"))?;
            match animation_of(clip, &read) {
                Some(Err(why)) => Err(format!("`{path}` {why}")),
                _ => Ok(()),
            }
        }
        None => Ok(()),
    }
}

/// A corner of a triangle cut by what the camera sees: where it is before the camera, and how much
/// of each of the triangle's own corners it is.
#[derive(Clone, Copy)]
struct Cut {
    seen: Vector,
    of: [f64; 3],
}

/// `corners`, less what of them is nearer than `near` along the camera's way.
fn cut_near(corners: &[Cut], near: f64) -> Vec<Cut> {
    let mut out = Vec::with_capacity(corners.len() + 1);
    for (index, a) in corners.iter().enumerate() {
        let b = &corners[(index + 1) % corners.len()];
        let (ahead_a, ahead_b) = (a.seen[2] - near, b.seen[2] - near);
        if ahead_a >= 0.0 {
            out.push(*a);
        }
        if (ahead_a >= 0.0) != (ahead_b >= 0.0) {
            let along = ahead_a / (ahead_a - ahead_b);
            let between = |a: f64, b: f64| a + (b - a) * along;
            out.push(Cut {
                seen: [0, 1, 2].map(|axis| between(a.seen[axis], b.seen[axis])),
                of: [0, 1, 2].map(|corner| between(a.of[corner], b.of[corner])),
            });
        }
    }
    out
}

/// What is nearest at each place read: its triangle, how far ahead it is, and how much of the
/// triangle's first two corners it is.
struct Nearest {
    across: usize,
    triangle: Vec<u32>,
    ahead: Vec<f32>,
    of: Vec<[f32; 2]>,
}

impl Nearest {
    /// The triangle `corners` on the screen, each a place and how far ahead it is, kept where it is
    /// the nearest yet.
    fn take(&mut self, index: u32, corners: [(f64, f64, Cut); 3], down: usize) {
        let [(ax, ay, a), (bx, by, b), (cx, cy, c)] = corners;
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        // Wound one way round, so that inside is where every edge is positive.
        let [(bx, by, b), (cx, cy, c)] = if area < 0.0 {
            [(cx, cy, c), (bx, by, b)]
        } else {
            [(bx, by, b), (cx, cy, c)]
        };
        let area = area.abs();
        let reach = |least: f64, most: f64, size: usize| {
            let from = least.floor().max(0.0) as usize;
            let to = (most.ceil().max(0.0) as usize).min(size);
            (from, to)
        };
        let (left, right) = reach(ax.min(bx).min(cx), ax.max(bx).max(cx), self.across);
        let (top, bottom) = reach(ay.min(by).min(cy), ay.max(by).max(cy), down);
        // An edge is a triangle's own when it is its top, or its left.
        let own = |(px, py): (f64, f64), (qx, qy): (f64, f64)| {
            let (dx, dy) = (qx - px, qy - py);
            dy < 0.0 || (dy == 0.0 && dx > 0.0)
        };
        let owns = [
            own((bx, by), (cx, cy)),
            own((cx, cy), (ax, ay)),
            own((ax, ay), (bx, by)),
        ];
        let inverse = [1.0 / a.seen[2], 1.0 / b.seen[2], 1.0 / c.seen[2]];
        for y in top..bottom {
            let py = y as f64 + 0.5;
            for x in left..right {
                let px = x as f64 + 0.5;
                let edges = [
                    (cx - bx) * (py - by) - (cy - by) * (px - bx),
                    (ax - cx) * (py - cy) - (ay - cy) * (px - cx),
                    (bx - ax) * (py - ay) - (by - ay) * (px - ax),
                ];
                let inside = edges
                    .iter()
                    .zip(owns)
                    .all(|(edge, own)| *edge > 0.0 || (*edge == 0.0 && own));
                if !inside {
                    continue;
                }
                // One over the distance goes evenly across the screen, and so do a corner's
                // shares over it.
                let shares = [0, 1, 2].map(|corner| edges[corner] / area * inverse[corner]);
                let sum = shares[0] + shares[1] + shares[2];
                if sum <= 0.0 {
                    continue;
                }
                let ahead = (1.0 / sum) as f32;
                let at = y * self.across + x;
                if ahead >= self.ahead[at] {
                    continue;
                }
                let shares = shares.map(|share| share / sum);
                let of = [0, 1].map(|corner| {
                    (shares[0] * a.of[corner] + shares[1] * b.of[corner] + shares[2] * c.of[corner])
                        as f32
                });
                self.triangle[at] = index;
                self.ahead[at] = ahead;
                self.of[at] = of;
            }
        }
    }
}

/// The corners of `mesh` in the world, its clip being where `plane` is.
fn in_world(mesh: &Mesh, plane: &Plane, size: (f64, f64)) -> (Vec<Vector>, [Vector; 3]) {
    // The mesh's own space in the world: a frame height of it is the picture's height along each
    // of the plane's ways, and as much back as a pixel of it is wide.
    let ways = [
        times(plane.across, size.1),
        times(plane.down, size.1),
        times(plane.normal, plane.pixel_size() * size.1),
    ];
    let points = mesh
        .points
        .iter()
        .map(|point| {
            let off = add(
                add(times(ways[0], point[0]), times(ways[1], point[1])),
                times(ways[2], point[2]),
            );
            add(plane.middle, off)
        })
        .collect();
    (points, ways)
}

/// A mesh in the world at a moment.
pub(in crate::draw) struct InWorld {
    /// Its corners, as its file's animation has them then.
    pub points: Vec<Vector>,
    /// Its triangles, by their corners.
    pub triangles: Vec<[u32; 3]>,
    /// Whether its surface closes round it ([`Mesh::closed`]).
    pub closed: bool,
    /// Whether it is a box, a ball, or a cylinder, which bulge nowhere inward: what is
    /// between any of its corners is inside it. Of a file's mesh that is not known.
    pub convex: bool,
}

/// The mesh of the clip of `plan` at clip time `time` in the world, where `to_output` puts its
/// clip.
pub(in crate::draw) fn in_the_world(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
) -> InWorld {
    let mesh = Mesh::of(&plan.clip, plan.clip.start + time);
    let size = (f64::from(plan.size.0), f64::from(plan.size.1));
    let plane = Plane::of(scene, plan, size, to_output, time);
    let (points, _) = in_world(&mesh, &plane, size);
    InWorld {
        points,
        closed: mesh.closed(),
        convex: mesh.file.is_none(),
        triangles: mesh.triangles,
    }
}

/// A mesh as what may stand in a light's way: its triangles in the world, in a tree of boxes
/// so that a ray asks few of them.
pub(in crate::draw) struct Standing {
    pub clip: Id,
    opacity: f32,
    /// Each triangle as a corner and the two sides from it, in the order of the tree's boxes.
    triangles: Vec<[Vector; 3]>,
    tree: Tree,
}

/// Boxes about things in a row, a mesh's triangles or a scene's meshes, each box halved while
/// it holds more than a few. The root is first, and after each box come all those in it, so
/// that one who walks it keeps no list of where to go next: into a box is the box after it,
/// and past a box is where the box says.
#[derive(Default)]
struct Tree {
    boxes: Vec<Boxed>,
}

struct Boxed {
    least: Vector,
    most: Vector,
    /// The run of things it holds, where it is not halved.
    run: Option<(usize, usize)>,
    /// The box after it and all that are in it.
    after: usize,
}

/// The most triangles a box of a mesh's tree holds without being halved.
const LEAF: usize = 8;

/// How far along a ray to a light a triangle starts being in its way: not from the very place
/// the ray starts, which may be on a triangle of another clip touching this one, but a
/// ten-thousandth of a pixel on, the world being in pixels.
const STARTS: f64 = 1e-4;

impl Tree {
    /// Over `things`, which it puts in the order of its boxes, no box left unhalved with more
    /// than `few` of them. `about` is the box about a thing, and `middle` where along an axis
    /// the thing is taken to be, which says the half it goes in.
    fn over<T>(
        things: &mut [T],
        few: usize,
        about: impl Fn(&T) -> (Vector, Vector),
        middle: impl Fn(&T, usize) -> f64,
    ) -> Self {
        let mut boxes = Vec::new();
        let all = (0, things.len());
        Self::halve(things, all, few.max(1), &(about, middle), &mut boxes);
        Self { boxes }
    }

    /// Put `things[from..to]` in a box at the end of `boxes`, halved along its longest side
    /// while it holds more than `few`.
    fn halve<T, A: Fn(&T) -> (Vector, Vector), M: Fn(&T, usize) -> f64>(
        things: &mut [T],
        (from, to): (usize, usize),
        few: usize,
        of: &(A, M),
        boxes: &mut Vec<Boxed>,
    ) {
        let (about, middle) = of;
        let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for thing in &things[from..to] {
            let (low, high) = about(thing);
            for axis in 0..3 {
                least[axis] = least[axis].min(low[axis]);
                most[axis] = most[axis].max(high[axis]);
            }
        }
        let own = boxes.len();
        boxes.push(Boxed {
            least,
            most,
            run: Some((from, to)),
            after: 0,
        });
        if to - from > few {
            let axis = (0..3)
                .max_by(|a, b| (most[*a] - least[*a]).total_cmp(&(most[*b] - least[*b])))
                .unwrap_or(0);
            let half = from + (to - from) / 2;
            things[from..to].select_nth_unstable_by(half - from, |a, b| {
                middle(a, axis).total_cmp(&middle(b, axis))
            });
            // The second half is walked before the first, as it was when a walk kept a list
            // and took the last thing put on it: which of two triangles met equally far off is
            // the first met stays as it was.
            Self::halve(things, (half, to), few, of, boxes);
            Self::halve(things, (from, half), few, of, boxes);
            boxes[own].run = None;
        }
        boxes[own].after = boxes.len();
    }

    /// Walk it from the root. `into` says whether a box is gone into, and `run` is given the
    /// things of each box gone into that is not halved, and says whether the walk is over.
    /// Whether a run ended it.
    #[inline]
    fn walk(
        &self,
        mut into: impl FnMut(&Boxed) -> bool,
        mut run: impl FnMut(usize, usize) -> bool,
    ) -> bool {
        let mut index = 0;
        while let Some(boxed) = self.boxes.get(index) {
            index = match (into(boxed), boxed.run) {
                (false, _) => boxed.after,
                (true, None) => index + 1,
                (true, Some((from, to))) => {
                    if run(from, to) {
                        return true;
                    }
                    boxed.after
                }
            };
        }
        false
    }
}

impl Boxed {
    /// Whether a ray from `at` is in it anywhere from `from` to `to` along itself, `over` being
    /// one over how far the ray goes along each axis ([`over`]).
    #[inline]
    fn crossed(&self, at: &Vector, over: &Vector, from: f64, to: f64) -> bool {
        let (mut enters, mut leaves) = (from, to);
        for axis in 0..3 {
            let (one, other) = (
                (self.least[axis] - at[axis]) * over[axis],
                (self.most[axis] - at[axis]) * over[axis],
            );
            enters = enters.max(one.min(other));
            leaves = leaves.min(one.max(other));
        }
        enters <= leaves
    }
}

/// One over how far a ray along `way` goes along each axis, and a great deal where it does not.
#[inline]
fn over(way: Vector) -> Vector {
    way.map(|side| 1.0 / if side.abs() < 1e-300 { 1e-300 } else { side })
}

/// How far along `way` from `at` a line meets a triangle, a corner and the two sides from it:
/// before `at` as well as after it, and not at all where it passes the triangle or runs along it.
#[inline]
fn along(at: Vector, way: Vector, [corner, one, other]: &[Vector; 3]) -> Option<f64> {
    let across = cross(way, *other);
    let facing = dot(*one, across);
    if facing.abs() < 1e-18 {
        return None;
    }
    let from = minus(at, *corner);
    let u = dot(from, across) / facing;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let beside = cross(from, *one);
    let v = dot(way, beside) / facing;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(*other, beside) / facing)
}

/// Where the rays from a place to a light with a size go: ahead of the place, between the
/// four rays around the light, and no further than the light. What is nowhere in it, none of
/// them can meet.
pub(in crate::draw) struct Beam {
    from: Vector,
    /// The ways of the four rays around the light, in order round it.
    around: [Vector; 4],
    /// The way into it across each of its four sides, the planes those rays lie in by twos,
    /// and the way ahead from where it starts.
    inward: [Vector; 5],
    /// How far ahead the light is.
    reach: f64,
}

impl Beam {
    /// The beam from `from` toward a light whose middle lies along `way`, a unit long, which
    /// the rays `around` go around, each a way and a reach, in order round it.
    pub fn between(from: Vector, way: Vector, around: &[(Vector, f64); 4]) -> Self {
        let mut inward = [way; 5];
        let mut reach = 0.0f64;
        for (corner, (ray, length)) in around.iter().enumerate() {
            // Square to the plane two neighbors lie in, toward the side the middle is on.
            let side = cross(*ray, around[(corner + 1) % 4].0);
            inward[corner] = if dot(side, way) < 0.0 {
                times(side, -1.0)
            } else {
                side
            };
            reach = reach.max(length * dot(*ray, way));
        }
        Self {
            from,
            around: around.map(|(ray, _)| ray),
            inward,
            reach,
        }
    }

    /// Whether the box from `least` to `most` is wholly outside it: past one of its sides,
    /// behind where it starts, or beyond the light. A box that is not may still be met by none
    /// of its rays.
    #[inline]
    fn clears_box(&self, least: &Vector, most: &Vector) -> bool {
        let (least, most) = (minus(*least, self.from), minus(*most, self.from));
        // How far along a way the box reaches at the most, or with `nearest` at the least: its
        // corner there is of `least` or of `most` along each axis, whichever goes further.
        let furthest = |way: &Vector, nearest: bool| {
            let mut sum = 0.0;
            for axis in 0..3 {
                let (one, other) = (least[axis] * way[axis], most[axis] * way[axis]);
                sum += if (one > other) != nearest { one } else { other };
            }
            sum
        };
        self.inward.iter().any(|way| furthest(way, false) < 0.0)
            || furthest(&self.inward[4], true) > self.reach
    }

    /// Whether a triangle, a corner and the two sides from it, is wholly outside it: past one
    /// of its sides, behind where it starts, or beyond the light, as a box is, or on the far
    /// side of one of its own edges from every ray around the light, as a large triangle
    /// lying askew of a narrow beam is of no side of the beam.
    #[inline]
    fn clears_triangle(&self, [corner, one, other]: &[Vector; 3]) -> bool {
        let first = minus(*corner, self.from);
        let corners = [first, add(first, *one), add(first, *other)];
        let along = |way: &Vector| corners.map(|corner| dot(corner, *way));
        let ahead = along(&self.inward[4]);
        if ahead.iter().all(|ahead| *ahead < 0.0)
            || ahead.iter().all(|ahead| *ahead > self.reach)
            || self.inward[..4]
                .iter()
                .any(|way| along(way).iter().all(|along| *along < 0.0))
        {
            return true;
        }
        // The plane an edge lies in with where the beam starts has the triangle on one side
        // of it. With the four rays around the light all on the other, no ray comes to it.
        (0..3).any(|edge| {
            let side = cross(corners[edge], corners[(edge + 1) % 3]);
            let third = dot(corners[(edge + 2) % 3], side);
            let rays = self.around.map(|ray| dot(ray, side));
            (third >= 0.0 && rays.iter().all(|ray| *ray < 0.0))
                || (third <= 0.0 && rays.iter().all(|ray| *ray > 0.0))
        })
    }
}

impl Standing {
    /// The mesh clip `plan` at clip time `time`, where `to_output` puts it, as opaque as
    /// `opacity`.
    pub fn of(
        scene: &Scene,
        plan: &ClipPlan,
        to_output: &Affine,
        time: Time,
        opacity: f32,
    ) -> Self {
        let InWorld {
            points, triangles, ..
        } = in_the_world(scene, plan, to_output, time);
        let corners = triangles
            .iter()
            .map(|triangle| triangle.map(|corner| points[corner as usize]))
            .collect();
        Self::over(plan.clip.id.clone(), opacity, corners)
    }

    /// `corners`, triangles in the world, whichever clip they are of: a mesh's, or the two
    /// of a flat clip.
    pub fn over(clip: Id, opacity: f32, mut corners: Vec<[Vector; 3]>) -> Self {
        let tree = Tree::over(
            &mut corners,
            LEAF,
            |triangle| {
                let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for corner in triangle {
                    for axis in 0..3 {
                        least[axis] = least[axis].min(corner[axis]);
                        most[axis] = most[axis].max(corner[axis]);
                    }
                }
                (least, most)
            },
            |triangle, axis| triangle[0][axis] + triangle[1][axis] + triangle[2][axis],
        );
        Self {
            clip,
            opacity,
            triangles: corners
                .into_iter()
                .map(|[a, b, c]| [a, minus(b, a), minus(c, a)])
                .collect(),
            tree,
        }
    }

    /// The box about all of it: nowhere, for a mesh of no triangles.
    fn about(&self) -> (Vector, Vector) {
        self.tree
            .boxes
            .first()
            .map_or(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |boxed| {
                (boxed.least, boxed.most)
            })
    }

    /// How much of a light gets past it to `at`, going along `way`, a unit long, for `reach`,
    /// `over` being [`over`] of the way.
    #[inline]
    fn lets_through(&self, at: Vector, way: Vector, over: &Vector, reach: f64) -> f32 {
        match self.meets(at, way, over, reach) {
            Some(_) => 1.0 - self.opacity,
            None => 1.0,
        }
    }

    /// How much of a light gets past it to `at`, going along `way`, a unit long, for `reach`,
    /// for one ray after another from one place toward one light, as the rays to a light
    /// with a size are. `met` is the triangle that stopped the last of them to be stopped,
    /// and is asked first: where a mesh hides all of a light it mostly stops the next ray
    /// too, and the tree is not walked.
    pub fn lets_through_after(
        &self,
        at: Vector,
        way: Vector,
        reach: f64,
        met: &mut Option<usize>,
    ) -> f32 {
        let again = met.is_some_and(|triangle| self.stops(triangle, at, way, reach));
        if !again {
            *met = self.meets(at, way, &over(way), reach);
        }
        match met {
            Some(_) => 1.0 - self.opacity,
            None => 1.0,
        }
    }

    /// Whether the triangle `which` is in the way of a ray from `at` along `way` within `reach`.
    #[inline]
    fn stops(&self, which: usize, at: Vector, way: Vector, reach: f64) -> bool {
        along(at, way, &self.triangles[which]).is_some_and(|along| along > STARTS && along < reach)
    }

    /// The first triangle found in the way of a ray from `at` along `way` within `reach`,
    /// which need not be the nearest.
    #[inline]
    fn meets(&self, at: Vector, way: Vector, over: &Vector, reach: f64) -> Option<usize> {
        let mut met = None;
        self.tree.walk(
            |boxed| boxed.crossed(&at, over, STARTS, reach),
            |from, to| {
                met = (from..to).find(|which| self.stops(*which, at, way, reach));
                met.is_some()
            },
        );
        met
    }

    /// Whether any ray of `beam` may meet it: whether a triangle of it is not wholly outside
    /// the beam, which one walk of its tree says for all of the beam's rays.
    fn reached_by(&self, beam: &Beam) -> bool {
        self.tree.walk(
            |boxed| !beam.clears_box(&boxed.least, &boxed.most),
            |from, to| {
                self.triangles[from..to]
                    .iter()
                    .any(|triangle| !beam.clears_triangle(triangle))
            },
        )
    }

    /// How far along `way`, a unit long, a ray from `at` first meets a triangle, no further
    /// than `reach`, and the way that triangle faces, a unit long. From the very place it
    /// starts on: one who measures how far off a surface is wants a surface touched said.
    pub fn first(&self, at: Vector, way: Vector, reach: f64) -> Option<(f64, Vector)> {
        let over = over(way);
        // Read by the one who says which boxes are gone into, and set by the one given their
        // triangles.
        let nearest: std::cell::Cell<Option<(f64, Vector)>> = std::cell::Cell::new(None);
        self.tree.walk(
            |boxed| {
                let within = nearest.get().map_or(reach, |(along, _)| along);
                boxed.crossed(&at, &over, 0.0, within)
            },
            |from, to| {
                for triangle in &self.triangles[from..to] {
                    let Some(along) = along(at, way, triangle) else {
                        continue;
                    };
                    if along >= 0.0
                        && along <= reach
                        && nearest.get().is_none_or(|(least, _)| along < least)
                    {
                        let [_, one, other] = triangle;
                        nearest.set(Some((along, unit(cross(*one, *other)))));
                    }
                }
                false
            },
        );
        nearest.get()
    }
}

/// The meshes that may stand in a light's way, in a tree of the boxes about them, so that a
/// ray asks those it comes to and not every mesh of the scene.
#[derive(Default)]
pub(in crate::draw) struct InTheWay {
    /// In the order of the tree's boxes.
    meshes: Vec<Standing>,
    tree: Tree,
}

impl InTheWay {
    /// `meshes`, each in a box of the tree by itself.
    pub fn of(mut meshes: Vec<Standing>) -> Self {
        let tree = Tree::over(
            &mut meshes,
            1,
            |mesh| mesh.about(),
            |mesh, axis| {
                let (least, most) = mesh.about();
                least[axis] + most[axis]
            },
        );
        Self { meshes, tree }
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    /// How much of a light gets past all of them but that of the clip `receiver` to `at`,
    /// going along `way`, a unit long, for `reach`.
    pub fn lets_through(&self, receiver: &Id, at: Vector, way: Vector, reach: f64) -> f32 {
        let over = over(way);
        let mut through = 1.0f32;
        self.tree.walk(
            |boxed| boxed.crossed(&at, &over, STARTS, reach),
            |from, to| {
                for mesh in &self.meshes[from..to] {
                    if mesh.clip != *receiver {
                        through *= mesh.lets_through(at, way, &over, reach);
                    }
                }
                // Past one that lets nothing by, the rest take nothing more.
                through == 0.0
            },
        );
        through
    }

    /// Give `each` every mesh but that of the clip `receiver` that a ray of `beam` may meet,
    /// until it says that no more are wanted.
    pub fn reached(&self, receiver: &Id, beam: &Beam, mut each: impl FnMut(&Standing) -> bool) {
        self.tree.walk(
            |boxed| !beam.clears_box(&boxed.least, &boxed.most),
            |from, to| {
                self.meshes[from..to]
                    .iter()
                    .any(|mesh| mesh.clip != *receiver && mesh.reached_by(beam) && each(mesh))
            },
        );
    }
}

/// The mesh of the clip of `plan` at clip time `time` as `scene` sees it on a picture of `(w, h)`,
/// and how far along the camera's way it is at each pixel: infinite where it is not.
pub(in crate::draw) fn draw(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
    (w, h): (usize, usize),
    shade: Shade,
) -> (Image, Vec<f32>) {
    let mesh = Mesh::of(&plan.clip, plan.clip.start + time);
    let size = (f64::from(plan.size.0), f64::from(plan.size.1));
    let plane = Plane::of(scene, plan, size, to_output, time);
    let (points, ways) = in_world(&mesh, &plane, size);
    // The ways are square to each other, so a side faces along each by its length less.
    let normals: Vec<Vector> = mesh
        .normals
        .iter()
        .map(|normal| {
            let along = |axis: usize| {
                times(
                    ways[axis],
                    normal[axis] / dot(ways[axis], ways[axis]).max(1e-18),
                )
            };
            unit(add(add(along(0), along(1)), along(2)))
        })
        .collect();
    let seen: Vec<Vector> = points
        .iter()
        .map(|point| {
            let from = minus(*point, scene.eye);
            [
                dot(from, scene.right),
                dot(from, scene.down),
                dot(from, scene.forward),
            ]
        })
        .collect();

    let (across, down) = (w * SAMPLES, h * SAMPLES);
    let mut nearest = Nearest {
        across,
        triangle: vec![NONE; across * down],
        ahead: vec![f32::INFINITY; across * down],
        of: vec![[0.0; 2]; across * down],
    };
    let near = NEAR * scene.height;
    let onto = |cut: Cut| {
        let by = scene.focal / cut.seen[2] * SAMPLES as f64;
        (
            scene.center.0 * SAMPLES as f64 + cut.seen[0] * by,
            scene.center.1 * SAMPLES as f64 + cut.seen[1] * by,
            cut,
        )
    };
    // Which size of its part's picture each triangle reads, where it has one: the size at which
    // a texel is as large as a place read, by how much of the picture and of the screen the
    // triangle covers.
    let pictured = mesh.parts.iter().any(|part| part.texture.is_some());
    let mut sizes = vec![0.0f32; if pictured { mesh.triangles.len() } else { 0 }];
    for (index, triangle) in mesh.triangles.iter().enumerate() {
        let corners = [0, 1, 2].map(|corner| Cut {
            seen: seen[triangle[corner] as usize],
            of: [0, 1, 2].map(|other| if other == corner { 1.0 } else { 0.0 }),
        });
        let cut = cut_near(&corners, near);
        for last in 2..cut.len() {
            nearest.take(
                index as u32,
                [onto(cut[0]), onto(cut[last - 1]), onto(cut[last])],
                down,
            );
        }
        let texture = mesh
            .part_of(index)
            .and_then(|part| part.texture)
            .and_then(|(texture, _)| mesh.file.as_ref()?.textures.get(texture));
        if let (Some(texture), true) = (texture, cut.len() >= 3) {
            let places = mesh.places(index);
            let on = [cut[0], cut[1], cut[2]].map(|cut| {
                let (x, y, _) = onto(cut);
                let place = [0, 1].map(|axis| {
                    (0..3)
                        .map(|corner| cut.of[corner] * places[corner][axis])
                        .sum::<f64>()
                });
                ((x, y), place)
            });
            let area = |[a, b, c]: [(f64, f64); 3]| {
                ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs()
            };
            let (wide, tall) = texture.size();
            let texels = area(on.map(|(_, place)| (place[0], place[1]))) * (wide * tall) as f64;
            let read = area(on.map(|(screen, _)| screen));
            if texels > 0.0 && read > 0.0 {
                sizes[index] = (0.5 * (texels / read).log2()).max(0.0) as f32;
            }
        }
    }

    // Where each point of the mesh is a frame on, for a motion pass. A file's animation that
    // gives it other points then tells nothing of where each went, and it is said to hold still.
    let then: Option<(&Scene, Vec<Vector>)> = match &shade {
        Shade::Moving(next, to_output, step, _) => {
            let later = Mesh::of(&plan.clip, plan.clip.start + time + *step);
            let plane = Plane::of(next, plan, size, to_output, time + *step);
            let (later, _) = in_world(&later, &plane, size);
            (later.len() == points.len()).then_some((*next, later))
        }
        _ => None,
    };
    // A place of a triangle among `all`, the points of the mesh, by its shares of two corners.
    let place = |triangle: u32, of: [f32; 2], all: &[Vector]| -> Vector {
        let corners = mesh.triangles[triangle as usize];
        let shares = [
            f64::from(of[0]),
            f64::from(of[1]),
            f64::from(1.0 - of[0] - of[1]),
        ];
        let mut sum = [0.0; 3];
        for (corner, share) in corners.iter().zip(shares) {
            sum = add(sum, times(all[*corner as usize], share));
        }
        sum
    };

    let colored = |triangle: u32, of: [f32; 2], ahead: f32| -> [f32; 3] {
        match &shade {
            Shade::Flat(color) => *color,
            Shade::Moving(_, _, _, height) => {
                let moved = then.as_ref().and_then(|(next, later)| {
                    let from = scene.on_screen(place(triangle, of, &points))?;
                    let to = next.on_screen(place(triangle, of, later))?;
                    Some((to.0 - from.0, to.1 - from.1))
                });
                motion_color(moved.unwrap_or((0.0, 0.0)), *height)
            }
            Shade::Near => [kerf_draw::color::to_linear(scene.nearness(f64::from(ahead))); 3],
            Shade::Facing => {
                let corners = mesh.triangles[triangle as usize];
                let shares = [
                    f64::from(of[0]),
                    f64::from(of[1]),
                    f64::from(1.0 - of[0] - of[1]),
                ];
                let mixed = |all: &[Vector]| {
                    let mut sum = [0.0; 3];
                    for (corner, share) in corners.iter().zip(shares) {
                        sum = add(sum, times(all[*corner as usize], share));
                    }
                    sum
                };
                scene.facing(mixed(&points), unit(mixed(&normals)))
            }
            Shade::Lit(receiver) => {
                let corners = mesh.triangles[triangle as usize];
                let shares = [
                    f64::from(of[0]),
                    f64::from(of[1]),
                    f64::from(1.0 - of[0] - of[1]),
                ];
                let Some(part) = mesh.part_of(triangle as usize) else {
                    return [1.0; 3];
                };
                if part.own {
                    return mesh.own_color(part, triangle as usize, shares, &sizes);
                }
                let mixed = |all: &[Vector]| {
                    let mut sum = [0.0; 3];
                    for (corner, share) in corners.iter().zip(shares) {
                        sum = add(sum, times(all[*corner as usize], share));
                    }
                    sum
                };
                let at = mixed(&points);
                let mut normal = unit(mixed(&normals));
                // Seen from behind, a side faces whoever sees it.
                if dot(minus(scene.eye, at), normal) < 0.0 {
                    normal = times(normal, -1.0);
                }
                let color = part.color;
                match scene.lit_outside(receiver, at, normal) {
                    Some(light) => [0, 1, 2].map(|channel| color[channel] * light[channel]),
                    None => color,
                }
            }
        }
    };
    let mut out = Image::new(w, h);
    let mut depth = vec![f32::INFINITY; w * h];
    let share = 1.0 / (SAMPLES * SAMPLES) as f32;
    out.px
        .par_chunks_mut(w.max(1) * 4)
        .zip(depth.par_chunks_mut(w.max(1)))
        .enumerate()
        .for_each(|(y, (row, depth))| {
            for x in 0..w {
                let mut sum = [0.0f32; 4];
                // A pixel is mostly of one triangle, which is then lit once.
                let mut last: Option<(u32, [f32; 3])> = None;
                for place in 0..SAMPLES * SAMPLES {
                    let at =
                        (y * SAMPLES + place / SAMPLES) * across + x * SAMPLES + place % SAMPLES;
                    let triangle = nearest.triangle[at];
                    if triangle == NONE {
                        continue;
                    }
                    let ahead = nearest.ahead[at];
                    depth[x] = depth[x].min(ahead);
                    // Its own colors change within a pixel, and are read at each place.
                    let once = match shade {
                        Shade::Near | Shade::Facing | Shade::Moving(..) => false,
                        Shade::Flat(_) => true,
                        Shade::Lit(_) => !mesh.part_of(triangle as usize).is_some_and(|p| p.own),
                    };
                    let color = match last {
                        Some((lit, color)) if lit == triangle && once => color,
                        _ => colored(triangle, nearest.of[at], ahead),
                    };
                    last = Some((triangle, color));
                    for (sum, channel) in sum.iter_mut().zip(color) {
                        *sum += channel * share;
                    }
                    sum[3] += share;
                }
                // An ids pass says which clip a pixel is of, and a pixel half of one is of none:
                // there it is the mesh's or it is not.
                if let Shade::Flat(color) = &shade {
                    sum = if sum[3] >= 0.5 {
                        [color[0], color[1], color[2], 1.0]
                    } else {
                        depth[x] = f32::INFINITY;
                        [0.0; 4]
                    };
                }
                row[x * 4..x * 4 + 4].copy_from_slice(&sum);
            }
        });
    if scene.aperture > 0.0 {
        let mut radii = scene.blur_radii(&depth);
        for (radius, depth) in radii.iter_mut().zip(&depth) {
            if !depth.is_finite() {
                *radius = 0.0;
            }
        }
        out = super::super::focus::blurred(&out, &radii);
    }
    (out, depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solids_are_closed_and_face_outward() {
        for mesh in [
            Mesh::a_box([1.0, 2.0, 3.0], [1.0; 3]),
            Mesh::a_ball([1.0, 2.0, 3.0], [1.0; 3]),
            Mesh::a_cylinder([1.0, 2.0, 3.0], [1.0; 3]),
        ] {
            assert_eq!(mesh.parts.len(), 1);
            assert_eq!(mesh.parts[0].end, mesh.triangles.len());
            // Every corner faces away from the middle, and is within the box it was asked to fill.
            for (point, normal) in mesh.points.iter().zip(&mesh.normals) {
                assert!((dot(*normal, *normal) - 1.0).abs() < 1e-9);
                assert!(dot(*point, *normal) >= -1e-9, "{point:?} faces {normal:?}");
                assert!(
                    point[0].abs() <= 0.5 + 1e-9
                        && point[1].abs() <= 1.0 + 1e-9
                        && point[2].abs() <= 1.5 + 1e-9
                );
            }
            // Closed: every edge between two different places is one of two triangles, counted by
            // where its ends are, since a corner is told again for each way it faces.
            assert!(
                super::super::gltf::closes(&mesh.points, &mesh.triangles),
                "an edge is open"
            );
            assert!(mesh.closed());
        }
        // One with no size one way has nothing inside it, and one with a side taken off is open.
        assert!(!Mesh::a_box([1.0, 0.0, 3.0], [1.0; 3]).closed());
        let mut open = Mesh::a_box([1.0, 2.0, 3.0], [1.0; 3]);
        open.triangles.truncate(open.triangles.len() - 2);
        assert!(!super::super::gltf::closes(&open.points, &open.triangles));
        assert!(!super::super::gltf::closes(&[], &[]));
    }

    /// The triangles of `mesh` in a world of pixels, turned `turn` degrees about the way down
    /// and with its middle at `place`, as what stands in a light's way.
    fn standing(clip: &str, opacity: f32, mesh: &Mesh, turn: f64, place: Vector) -> Standing {
        let (sin, cos) = turn.to_radians().sin_cos();
        let corners = mesh
            .triangles
            .iter()
            .map(|triangle| {
                triangle.map(|corner| {
                    let [x, y, z] = mesh.points[corner as usize];
                    add([x * cos + z * sin, y, z * cos - x * sin], place)
                })
            })
            .collect();
        Standing::over(Id::new(clip).unwrap(), opacity, corners)
    }

    #[test]
    fn a_ray_through_the_tree_meets_what_it_meets_of_every_triangle() {
        // A ball has triangles enough for a tree some levels deep.
        let ball = Mesh::a_ball([100.0, 140.0, 80.0], [1.0; 3]);
        let ball = standing("ball", 1.0, &ball, 0.0, [10.0, -20.0, 30.0]);
        assert!(ball.tree.boxes.len() > 100);
        let (mut met, mut passed) = (0, 0);
        for step in 0..400 {
            // From all around it, toward it and beside it.
            let turn = f64::from(step) * 0.7;
            let at = [
                150.0 * turn.cos(),
                120.0 * (turn * 0.37).sin(),
                150.0 * turn.sin(),
            ];
            let toward = [
                10.0 + 70.0 * (turn * 1.3).sin(),
                -20.0 + 90.0 * (turn * 0.9).cos(),
                30.0 + 60.0 * (turn * 0.53).sin(),
            ];
            let way = unit(minus(toward, at));
            for reach in [f64::INFINITY, 140.0] {
                let every =
                    (0..ball.triangles.len()).any(|which| ball.stops(which, at, way, reach));
                let found = ball.meets(at, way, &over(way), reach);
                assert_eq!(found.is_some(), every, "from {at:?} along {way:?}");
                if every {
                    met += 1;
                } else {
                    passed += 1;
                }
                // And the first it meets is the nearest of them all.
                let nearest = ball
                    .triangles
                    .iter()
                    .filter_map(|triangle| along(at, way, triangle))
                    .filter(|along| *along >= 0.0 && *along <= reach)
                    .min_by(f64::total_cmp);
                let first = ball.first(at, way, reach).map(|(along, _)| along);
                assert_eq!(first, nearest, "from {at:?} along {way:?}");
            }
        }
        assert!(met > 100 && passed > 100, "{met} {passed}");
    }
}
