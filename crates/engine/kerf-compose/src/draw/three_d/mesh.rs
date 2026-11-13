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
//! has the shot's light in them already. It casts no shadow, and is drawn on the CPU.
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
        let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for point in &read.points {
            for axis in 0..3 {
                least[axis] = least[axis].min(point[axis]);
                most[axis] = most[axis].max(point[axis]);
            }
        }
        let middle = times(add(least, most), 0.5);
        let size = minus(most, least);
        // Flat as a sheet lying down, it is as wide as it was to be tall.
        let tall = if size[1] > 1e-12 {
            size[1]
        } else {
            size[0].max(size[2]).max(1e-12)
        };
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
    if let Some((_, (length, modified, read))) = files.iter().find(|(kept, _)| kept == path) {
        if (*length, *modified) == now {
            return Ok(read.clone());
        }
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
    // The mesh's own space in the world: a frame height of it is the picture's height along each
    // of the plane's ways, and as much back as a pixel of it is wide.
    let ways = [
        times(plane.across, size.1),
        times(plane.down, size.1),
        times(plane.normal, plane.pixel_size() * size.1),
    ];
    let points: Vec<Vector> = mesh
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

    let colored = |triangle: u32, of: [f32; 2], ahead: f32| -> [f32; 3] {
        match &shade {
            Shade::Flat(color) => *color,
            Shade::Near => [kerf_draw::color::to_linear(scene.nearness(f64::from(ahead))); 3],
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
                        Shade::Near => false,
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
            let key =
                |index: u32| mesh.points[index as usize].map(|value| (value * 1e6).round() as i64);
            let mut edges = std::collections::BTreeMap::new();
            for triangle in &mesh.triangles {
                for side in 0..3 {
                    let (a, b) = (key(triangle[side]), key(triangle[(side + 1) % 3]));
                    if a != b {
                        *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                    }
                }
            }
            assert!(edges.values().all(|count| *count == 2), "an edge is open");
        }
    }

    #[test]
    fn a_triangle_cut_by_the_camera_keeps_what_is_ahead() {
        let corner = |x: f64, ahead: f64, of: [f64; 3]| Cut {
            seen: [x, 0.0, ahead],
            of,
        };
        let corners = [
            corner(0.0, 3.0, [1.0, 0.0, 0.0]),
            corner(1.0, -1.0, [0.0, 1.0, 0.0]),
            corner(-1.0, 3.0, [0.0, 0.0, 1.0]),
        ];
        let cut = cut_near(&corners, 1.0);
        // One corner behind: four are left, the new two where its edges come to what is seen.
        assert_eq!(cut.len(), 4);
        assert!(cut.iter().all(|corner| corner.seen[2] >= 1.0 - 1e-12));
        assert!((cut[1].seen[0] - 0.5).abs() < 1e-12 && (cut[1].of[1] - 0.5).abs() < 1e-12);
        // All of it behind, and none is left; all of it ahead, and it is as it was.
        assert!(cut_near(&corners, 4.0).is_empty());
        assert_eq!(cut_near(&corners, -2.0).len(), 3);
    }
}
