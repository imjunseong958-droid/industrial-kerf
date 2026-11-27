//! The triangles of a glTF file, for a mesh read from one (`gen=mesh file=`).
//!
//! What a guide wants of a model is its shape at rest and its own colors: the triangles of every
//! mesh its scene's nodes hold, each where its node puts it, each material's base color, the
//! picture a material lays over its part, and the colors its corners are given. And how it
//! moves: its animations of where its nodes are, and the skins by which a mesh bends with the
//! nodes that are its bones, so that it can be posed at any time of one ([`Read::posed`]). And
//! its cameras, each where its node is at any time of one ([`Read::seen_from`]). Morph targets are read as how far they take each corner, and not as how they turn the
//! way it faces. Lights in the file are not read, and of a material nothing but its base
//! color: how a thing shines is a model's to draw. A `.gltf` with its buffers and
//! pictures beside it or inside it, and a `.glb`, are read.
//!
//! glTF's x runs right, its y up, and its z toward the viewer. Kerf's y runs down and its z away,
//! so a model is turned half round about x, which keeps it the way round it was.

use std::path::Path;

use kerf_draw::Affine3;
use serde::Deserialize;
use serde_json::Value as Json;

use super::texture::{Texture, Wrap};

type Vector = [f64; 3];

/// A file's triangles, in Kerf's axes and the file's own units.
pub(super) struct Read {
    pub points: Vec<Vector>,
    pub normals: Vec<Vector>,
    /// Where on its part's picture each corner is, as shares of it from its top left.
    pub uvs: Vec<[f32; 2]>,
    /// The color each corner is given, in linear light: white where it is given none.
    pub colors: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub parts: Vec<Part>,
    pub textures: Vec<Texture>,
    /// The ways the file says it moves.
    pub animations: Vec<Animation>,
    /// The cameras its nodes hold.
    pub cameras: Vec<Camera>,
    rig: Rig,
    /// Whether its surface closes round what it holds, once that has been asked.
    closed: std::sync::OnceLock<bool>,
}

/// Whether `triangles` close round what is in them: every edge between two different places
/// is one of two triangles. An edge is told by where its ends are, to a millionth of how
/// large the whole is, since a corner is told again for each way it faces and each part it
/// is a corner of. A triangle with no size has no edge, and nothing closes round nothing.
pub(super) fn closes(points: &[Vector], triangles: &[[u32; 3]]) -> bool {
    let (mut least, mut most) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for point in points {
        for axis in 0..3 {
            least[axis] = least[axis].min(point[axis]);
            most[axis] = most[axis].max(point[axis]);
        }
    }
    let large = (0..3)
        .map(|axis| most[axis] - least[axis])
        .fold(0.0, f64::max);
    if !(large > 0.0 && large.is_finite()) {
        return false;
    }
    let place = |corner: u32| {
        points.get(corner as usize).map(|point| {
            [0, 1, 2].map(|axis| ((point[axis] - least[axis]) / large * 1e6).round() as i64)
        })
    };
    let mut edges = Vec::with_capacity(triangles.len() * 3);
    for triangle in triangles {
        for side in 0..3 {
            let (Some(from), Some(to)) = (place(triangle[side]), place(triangle[(side + 1) % 3]))
            else {
                return false;
            };
            if from != to {
                edges.push((from.min(to), from.max(to)));
            }
        }
    }
    edges.sort_unstable();
    !edges.is_empty()
        && edges
            .chunk_by(|one, other| one == other)
            .all(|same| same.len() == 2)
}

/// A camera of a file: a node that holds one.
pub(super) struct Camera {
    /// What the file calls the camera, or its node: nothing where it calls neither anything.
    pub name: String,
    node: usize,
    /// How much it sees from the top of its picture to the bottom, in degrees: none for a
    /// camera with no perspective.
    pub fov: Option<f64>,
    /// How wide its picture is for its height, where the file says.
    pub aspect: Option<f64>,
}

/// Where a camera is and how it is turned, in Kerf's axes and the file's own units.
pub(super) struct Seen {
    pub eye: Vector,
    pub right: Vector,
    pub forward: Vector,
}

impl Read {
    /// Whether the file's surface closes round what it holds, so that a line that goes into
    /// it comes out of it again ([`closes`], of its corners at rest): a thing modelled whole
    /// does, and a sheet, a set rebuilt from a shot, or a thing left open underneath does
    /// not. How it moves opens and closes nothing, so it is found once.
    pub(super) fn closed(&self) -> bool {
        *self
            .closed
            .get_or_init(|| closes(&self.points, &self.triangles))
    }

    /// Where camera `which` is: at rest, or `seconds` into the animation numbered. A glTF camera
    /// looks along its node's z backward, with its node's y up.
    pub(super) fn seen_from(&self, which: usize, animation: Option<(usize, f64)>) -> Option<Seen> {
        let camera = self.cameras.get(which)?;
        let animation =
            animation.and_then(|(which, seconds)| Some((self.animations.get(which)?, seconds)));
        let place = *self.rig.places(animation).get(camera.node)?;
        // A node's size is not its camera's: only where its axes point.
        let axis = |way: Vector, sign: f64| {
            let length = (way[0] * way[0] + way[1] * way[1] + way[2] * way[2])
                .sqrt()
                .max(1e-300);
            [
                sign * way[0] / length,
                -sign * way[1] / length,
                -sign * way[2] / length,
            ]
        };
        Some(Seen {
            eye: [place.t[0], -place.t[1], -place.t[2]],
            right: axis(place.x, 1.0),
            forward: axis(place.z, -1.0),
        })
    }

    /// Where every corner is and the way it faces, in Kerf's axes and the file's own units:
    /// at rest, or `seconds` into the animation numbered.
    pub(super) fn posed(&self, animation: Option<(usize, f64)>) -> (Vec<Vector>, Vec<Vector>) {
        let rig = &self.rig;
        let animation =
            animation.and_then(|(which, seconds)| Some((self.animations.get(which)?, seconds)));
        let places = rig.places(animation);
        let mut points = Vec::with_capacity(rig.points.len());
        let mut normals = Vec::with_capacity(rig.points.len());
        for run in &rig.runs {
            let whole = Placed::by(places[run.node]);
            // A bone moves a corner from where the bone was when the mesh was bound to it.
            let bones: Option<Vec<Affine3>> = run.skin.map(|skin| {
                let skin = &rig.skins[skin];
                skin.joints
                    .iter()
                    .zip(&skin.inverse)
                    .map(|(joint, inverse)| match places.get(*joint) {
                        Some(place) => then(place, inverse),
                        None => Affine3::IDENTITY,
                    })
                    .collect()
            });
            // How much of each morph target it has: as the animation says, or at rest.
            let weights = animation
                .and_then(|(animation, seconds)| {
                    let told = animation.morphs.iter().find(|told| told.node == run.node)?;
                    Some(told.at(seconds))
                })
                .unwrap_or_else(|| run.weights.clone());
            for corner in run.corners.0..run.corners.1 {
                let placed = match &bones {
                    Some(bones) => {
                        let (joints, weights) = rig.bones[corner];
                        let mut sum = [[0.0; 3]; 4];
                        let mut all = 0.0;
                        for (joint, weight) in joints.iter().zip(weights) {
                            let Some(bone) = bones.get(*joint as usize) else {
                                continue;
                            };
                            let weight = f64::from(weight);
                            all += weight;
                            let bone = [bone.x, bone.y, bone.z, bone.t];
                            for (sum, bone) in sum.iter_mut().flatten().zip(bone.iter().flatten()) {
                                *sum += bone * weight;
                            }
                        }
                        let [x, y, z, t] = sum;
                        let sum = Affine3 { x, y, z, t };
                        if all > 1e-9 {
                            Placed::by(sum)
                        } else {
                            whole
                        }
                    }
                    None => whole,
                };
                let mut at = rig.points[corner];
                for (morph, weight) in run.morphs.iter().zip(&weights) {
                    if *weight != 0.0 {
                        let by = morph[corner - run.corners.0];
                        at = [0, 1, 2].map(|axis| at[axis] + by[axis] * weight);
                    }
                }
                points.push(placed.point(at));
                normals.push(placed.facing(rig.normals[corner]));
            }
            if run.flat {
                for three in &self.triangles[run.triangles.0..run.triangles.1] {
                    let [a, b, c] = three.map(|corner| points[corner as usize]);
                    let (ab, ac) = (
                        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                    );
                    let across = [
                        ab[1] * ac[2] - ab[2] * ac[1],
                        ab[2] * ac[0] - ab[0] * ac[2],
                        ab[0] * ac[1] - ab[1] * ac[0],
                    ];
                    let length =
                        (across[0] * across[0] + across[1] * across[1] + across[2] * across[2])
                            .sqrt()
                            .max(1e-300);
                    for corner in three {
                        normals[*corner as usize] = across.map(|value| value / length);
                    }
                }
            }
        }
        (points, normals)
    }
}

/// One of a file's animations.
pub(super) struct Animation {
    /// What the file calls it: nothing where it calls it nothing.
    pub name: String,
    /// How long it is, in seconds: when the last of its keys is.
    pub length: f64,
    channels: Vec<Channel>,
    /// What it says of the morph targets of nodes' meshes.
    morphs: Vec<Weighed>,
}

/// What an animation says of how much of each morph target a node's mesh has, over time.
struct Weighed {
    node: usize,
    how: How,
    /// How many targets a value is of.
    across: usize,
    times: Vec<f64>,
    values: Vec<f64>,
}

impl Weighed {
    /// The weights `seconds` in: the first before its first key, and the last after its
    /// last. With curves it goes straight from key to key, which the keys are still on.
    fn at(&self, seconds: f64) -> Vec<f64> {
        let each = self.across * if self.how == How::Cubic { 3 } else { 1 };
        let value = |key: usize| {
            let from = key * each
                + if self.how == How::Cubic {
                    self.across
                } else {
                    0
                };
            &self.values[from..from + self.across]
        };
        let after = self.times.partition_point(|time| *time <= seconds);
        if after == 0 {
            return value(0).to_vec();
        }
        if after == self.times.len() {
            return value(after - 1).to_vec();
        }
        if self.how == How::Step {
            return value(after - 1).to_vec();
        }
        let (from, to) = (self.times[after - 1], self.times[after]);
        let share = (seconds - from) / (to - from).max(1e-12);
        value(after - 1)
            .iter()
            .zip(value(after))
            .map(|(a, b)| a + (b - a) * share)
            .collect()
    }
}

/// What an animation says of one node over time.
struct Channel {
    node: usize,
    what: What,
    how: How,
    /// How many numbers a value is.
    across: usize,
    times: Vec<f64>,
    values: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum What {
    Moved,
    Turned,
    Sized,
}

/// How an animation goes from one key to the next, as glTF names it: straight, where it names
/// no other way.
#[derive(Clone, Copy, Default, PartialEq, Deserialize)]
enum How {
    #[serde(rename = "STEP")]
    Step,
    #[default]
    #[serde(rename = "LINEAR")]
    Linear,
    #[serde(rename = "CUBICSPLINE")]
    Cubic,
}

impl Channel {
    /// Its value `seconds` in: the first before its first key, and the last after its last.
    fn at(&self, seconds: f64) -> [f64; 4] {
        let each = self.across * if self.how == How::Cubic { 3 } else { 1 };
        // A key's value: with curves, it is between the way in and the way out.
        let part = |key: usize, which: usize| {
            let from = key * each + which * self.across;
            let mut out = [0.0; 4];
            out[..self.across].copy_from_slice(&self.values[from..from + self.across]);
            out
        };
        let value = |key: usize| part(key, usize::from(self.how == How::Cubic));
        let after = self.times.partition_point(|time| *time <= seconds);
        if after == 0 {
            return value(0);
        }
        if after == self.times.len() {
            return value(after - 1);
        }
        let (from, to) = (self.times[after - 1], self.times[after]);
        let share = (seconds - from) / (to - from).max(1e-12);
        let (a, b) = (value(after - 1), value(after));
        let mut out = match self.how {
            How::Step => a,
            How::Linear if self.what == What::Turned => turned_between(a, b, share),
            How::Linear => [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * share),
            How::Cubic => {
                let (out_of, into) = (part(after - 1, 2), part(after, 0));
                let (t, long) = (share, to - from);
                let (t2, t3) = (t * t, t * t * t);
                [0, 1, 2, 3].map(|k| {
                    (2.0 * t3 - 3.0 * t2 + 1.0) * a[k]
                        + (t3 - 2.0 * t2 + t) * long * out_of[k]
                        + (-2.0 * t3 + 3.0 * t2) * b[k]
                        + (t3 - t2) * long * into[k]
                })
            }
        };
        if self.what == What::Turned {
            let length = out.iter().map(|value| value * value).sum::<f64>().sqrt();
            out = out.map(|value| value / length.max(1e-300));
        }
        out
    }
}

/// The turn `share` of the way from `a` to `b`, the short way round and at an even speed.
fn turned_between(a: [f64; 4], mut b: [f64; 4], share: f64) -> [f64; 4] {
    let mut alike: f64 = a.iter().zip(&b).map(|(a, b)| a * b).sum();
    if alike < 0.0 {
        (b, alike) = (b.map(|value| -value), -alike);
    }
    // Nearly one turn: the straight way between them is as good, and divides by nothing.
    if alike > 0.9995 {
        return [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * share);
    }
    let angle = alike.acos();
    let (from, to) = (
        ((1.0 - share) * angle).sin() / angle.sin(),
        (share * angle).sin() / angle.sin(),
    );
    [0, 1, 2, 3].map(|k| a[k] * from + b[k] * to)
}

/// A node's own place as what it is moved, turned, and sized by.
#[derive(Clone, Copy)]
struct Told {
    moved: Vector,
    turned: [f64; 4],
    sized: Vector,
}

impl Told {
    fn of(node: &json::Node) -> Self {
        Self {
            moved: node.translation.unwrap_or([0.0; 3]),
            turned: node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]),
            sized: node.scale.unwrap_or([1.0; 3]),
        }
    }

    fn place(&self) -> Affine3 {
        let (t, s) = (self.moved, self.sized);
        let length = self
            .turned
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt()
            .max(1e-300);
        let [x, y, z, w] = self.turned.map(|value| value / length);
        let turn = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        let axis = |column: usize| [0, 1, 2].map(|row| turn[row][column] * s[column]);
        Affine3 {
            x: axis(0),
            y: axis(1),
            z: axis(2),
            t,
        }
    }
}

struct Node {
    /// The node it is under.
    parent: Option<usize>,
    /// Its place where the file gives it whole, which no animation moves.
    matrix: Option<Affine3>,
    told: Told,
}

/// Which bones move a corner, and how much each.
type Bones = ([u16; 4], [f32; 4]);

/// A run of corners that one thing places: a primitive of one node's mesh.
struct Run {
    /// Its corners and its triangles: the first, and the one after the last.
    corners: (usize, usize),
    triangles: (usize, usize),
    node: usize,
    /// The skin whose bones move it, in place of its node.
    skin: Option<usize>,
    /// Whether the file told no way its corners face, so that each triangle is flat.
    flat: bool,
    /// Its morph targets: for each, how far it takes each of the run's corners from where
    /// it is, at a weight of 1.
    morphs: Vec<Vec<Vector>>,
    /// How much of each target it has at rest.
    weights: Vec<f64>,
}

struct Skin {
    /// The nodes that are its bones.
    joints: Vec<usize>,
    /// For each bone, the way back from where the mesh was bound to it.
    inverse: Vec<Affine3>,
}

/// What places a file's corners: its nodes, and the corners as their meshes have them.
struct Rig {
    nodes: Vec<Node>,
    /// Every node, each after the one it is under.
    order: Vec<usize>,
    /// Each corner in its mesh's own space, in the file's axes.
    points: Vec<Vector>,
    normals: Vec<Vector>,
    bones: Vec<Bones>,
    runs: Vec<Run>,
    skins: Vec<Skin>,
}

impl Rig {
    /// Where each node is in the file's world: at rest, or with an animation some seconds in.
    fn places(&self, animation: Option<(&Animation, f64)>) -> Vec<Affine3> {
        let mut told: Vec<Told> = self.nodes.iter().map(|node| node.told).collect();
        let mut moved = vec![false; told.len()];
        if let Some((animation, seconds)) = animation {
            for channel in &animation.channels {
                let value = channel.at(seconds);
                let three = [value[0], value[1], value[2]];
                moved[channel.node] = true;
                match channel.what {
                    What::Moved => told[channel.node].moved = three,
                    What::Turned => told[channel.node].turned = value,
                    What::Sized => told[channel.node].sized = three,
                }
            }
        }
        let mut places = vec![Affine3::IDENTITY; told.len()];
        for index in &self.order {
            let node = &self.nodes[*index];
            let own = match node.matrix {
                Some(matrix) if !moved[*index] => matrix,
                _ => told[*index].place(),
            };
            places[*index] = match node.parent {
                Some(parent) => then(&places[parent], &own),
                None => own,
            };
        }
        places
    }
}

/// What moves a corner into the file's world, and from there into Kerf's axes: y down, z away.
#[derive(Clone, Copy)]
struct Placed {
    place: Affine3,
    /// What turns the way a side faces: the cofactors of what moved it, which is its inverse
    /// turned over, but for its size.
    turn: [[f64; 3]; 3],
}

impl Placed {
    fn by(place: Affine3) -> Self {
        let axes = [place.x, place.y, place.z];
        let cofactor = |row: usize, column: usize| {
            let (r1, r2) = ((row + 1) % 3, (row + 2) % 3);
            let (c1, c2) = ((column + 1) % 3, (column + 2) % 3);
            axes[c1][r1] * axes[c2][r2] - axes[c2][r1] * axes[c1][r2]
        };
        Self {
            place,
            turn: [0, 1, 2].map(|row| [0, 1, 2].map(|column| cofactor(row, column))),
        }
    }

    fn point(&self, point: Vector) -> Vector {
        let at = self.place.apply(point);
        [at[0], -at[1], -at[2]]
    }

    fn facing(&self, normal: Vector) -> Vector {
        let at = [0, 1, 2].map(|row| {
            self.turn[row][0] * normal[0]
                + self.turn[row][1] * normal[1]
                + self.turn[row][2] * normal[2]
        });
        let length = (at[0] * at[0] + at[1] * at[1] + at[2] * at[2])
            .sqrt()
            .max(1e-300);
        [at[0] / length, -at[1] / length, -at[2] / length]
    }
}

/// A run of triangles of one material.
pub(super) struct Part {
    /// The triangle after its last.
    pub end: usize,
    /// Its material's color in linear light, where it has a material.
    pub color: Option<[f32; 3]>,
    /// The picture laid over it: which of the file's, and what it does past its edges.
    pub texture: Option<(usize, [Wrap; 2])>,
    /// Whether its corners are given colors of their own.
    pub colored: bool,
}

/// `inner`, and then `outer`.
fn then(outer: &Affine3, inner: &Affine3) -> Affine3 {
    let both = outer.then_apply(inner);
    // Where an axis comes to nothing it is 0.0 and never -0.0, as it has been since it was
    // summed here. The sign of such a zero is read where a file's camera is placed, as a pan of
    // 180 degrees one way or the other.
    let plain = |axis: Vector| axis.map(|along| along + 0.0);
    Affine3 {
        x: plain(both.x),
        y: plain(both.y),
        z: plain(both.z),
        t: both.t,
    }
}

/// A place written whole, a column at a time: sixteen numbers, the last of each column left out.
fn whole(m: &[f64; 16]) -> Affine3 {
    let column = |column: usize| [0, 1, 2].map(|row| m[column * 4 + row]);
    Affine3 {
        x: column(0),
        y: column(1),
        z: column(2),
        t: column(3),
    }
}

fn decoded(base64: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(base64.len() * 3 / 4);
    let (mut held, mut bits) = (0u32, 0u32);
    for byte in base64.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        held = (held << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((held >> bits) as u8);
        }
    }
    Some(out)
}

/// The file's JSON, as much of it as is read, and, in a `.glb`, the bytes that come with it.
fn parts_of(bytes: &[u8]) -> Result<(json::Gltf, Option<&[u8]>), String> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|four| u32::from_le_bytes([four[0], four[1], four[2], four[3]]) as usize)
            .ok_or("it ends early")
    };
    // Read as JSON first, and only then as the types: JSON says what is there, and a type
    // reads it a field at a time, as the loader did by key ([`json`]).
    let typed = |json: &[u8]| {
        serde_json::from_slice::<Json>(json)
            .map(json::Gltf::of)
            .map_err(|error| format!("its JSON: {error}"))
    };
    if !bytes.starts_with(b"glTF") {
        return Ok((typed(bytes)?, None));
    }
    let (mut at, mut json, mut binary) = (12, None, None);
    while at + 8 <= bytes.len() {
        let (length, kind) = (word(at)?, word(at + 4)?);
        let chunk = bytes.get(at + 8..at + 8 + length).ok_or("it ends early")?;
        match kind {
            0x4E4F_534A => json = Some(chunk),
            0x004E_4942 => binary = Some(chunk),
            _ => {}
        }
        at += 8 + length;
    }
    let json = json.ok_or("it has no JSON")?;
    Ok((typed(json)?, binary))
}

struct File<'a> {
    json: &'a json::Gltf,
    buffers: Vec<Vec<u8>>,
    /// Where the file is, for what it names beside it.
    folder: &'a Path,
    /// The pictures read so far: which of the file's, and where among what was read.
    pictures: std::cell::RefCell<Vec<(usize, usize)>>,
}

/// A material's picture on its part.
struct Laid {
    texture: usize,
    wrap: [Wrap; 2],
    /// Which of a corner's places on a picture it is read at.
    set: u64,
    offset: [f64; 2],
    turn: f64,
    scale: [f64; 2],
}

impl Laid {
    /// Where on the picture a corner told `[u, v]` is: sized, turned, and then moved.
    fn at(&self, [u, v]: [f64; 2]) -> [f32; 2] {
        let (u, v) = (u * self.scale[0], v * self.scale[1]);
        let (sin, cos) = self.turn.sin_cos();
        [
            (cos * u + sin * v + self.offset[0]) as f32,
            (-sin * u + cos * v + self.offset[1]) as f32,
        ]
    }
}

/// A name in a URI as a file's: `%20` a space.
fn unescaped(uri: &str) -> String {
    let mut out = Vec::with_capacity(uri.len());
    let bytes = uri.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let byte = std::str::from_utf8(bytes.get(at + 1..at + 3).unwrap_or(&[]))
            .ok()
            .filter(|_| bytes[at] == b'%')
            .and_then(|two| u8::from_str_radix(two, 16).ok());
        match byte {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl File<'_> {
    /// The bytes of each element of accessor `index`, its component type, and how many
    /// components an element has.
    fn accessor(&self, index: json::Named) -> Result<(Vec<&[u8]>, u64, usize), String> {
        let accessor = index
            .0
            .and_then(|index| self.json.accessors.get(index))
            .ok_or("an accessor is missing")?;
        if accessor.sparse.is_some() {
            return Err("a sparse accessor is not read".into());
        }
        let component = accessor.component_type;
        let width = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err("an accessor has no component type".into()),
        };
        let across = match accessor.shape {
            Some(json::Shape::Scalar) => 1,
            Some(json::Shape::Vec2) => 2,
            Some(json::Shape::Vec3) => 3,
            Some(json::Shape::Vec4) => 4,
            Some(json::Shape::Mat4) => 16,
            None => return Err("an accessor is of a type that is not read".into()),
        };
        let count = accessor.count;
        let view = accessor
            .buffer_view
            .and_then(|view| self.json.buffer_views.get(view))
            .ok_or("an accessor has no buffer view")?;
        let buffer = view
            .buffer
            .and_then(|buffer| self.buffers.get(buffer))
            .ok_or("a buffer is missing")?;
        let start = view.byte_offset + accessor.byte_offset;
        let size = width * across;
        let stride = view.byte_stride.max(size);
        (0..count)
            .map(|element| {
                let at = start + element * stride;
                buffer
                    .get(at..at + size)
                    .ok_or("a buffer ends early".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|elements| (elements, component, across))
    }

    fn threes(&self, index: json::Named) -> Result<Vec<Vector>, String> {
        Ok(self.numbers(index, 3)?.as_chunks::<3>().0.to_vec())
    }

    /// The numbers of accessor `index`, `across` to an element: floats as they are, and whole
    /// numbers as shares of the most they can hold where the accessor says they are.
    fn numbers(&self, index: json::Named, across: usize) -> Result<Vec<f64>, String> {
        let (elements, component, told) = self.accessor(index)?;
        if told != across {
            return Err(format!(
                "an accessor read {across} to an element has {told}"
            ));
        }
        let shares = index
            .0
            .and_then(|index| self.json.accessors.get(index))
            .is_some_and(|accessor| accessor.normalized);
        let width = elements.first().map_or(1, |element| element.len() / across);
        let mut out = Vec::with_capacity(elements.len() * across);
        for element in elements {
            for bytes in element.chunks(width) {
                let (value, most) = match component {
                    5120 => (f64::from(bytes[0] as i8), 127.0),
                    5121 => (f64::from(bytes[0]), 255.0),
                    5122 => (f64::from(i16::from_le_bytes([bytes[0], bytes[1]])), 32767.0),
                    5123 => (f64::from(u16::from_le_bytes([bytes[0], bytes[1]])), 65535.0),
                    5125 => {
                        let four = [bytes[0], bytes[1], bytes[2], bytes[3]];
                        (f64::from(u32::from_le_bytes(four)), 4294967295.0)
                    }
                    _ => {
                        let four = [bytes[0], bytes[1], bytes[2], bytes[3]];
                        (f64::from(f32::from_le_bytes(four)), 1.0)
                    }
                };
                out.push(if shares && component != 5126 {
                    (value / most).max(-1.0)
                } else {
                    value
                });
            }
        }
        Ok(out)
    }

    /// The bytes of picture `index`: inside the file, or beside it.
    fn picture(&self, index: usize) -> Result<Texture, String> {
        let image = self.json.images.get(index).ok_or("a picture is missing")?;
        let named = |why: String| format!("its picture {index}: {why}");
        if let Some(uri) = &image.uri {
            let bytes = self.beside(uri).map_err(named)?;
            return Texture::read(&bytes).map_err(named);
        }
        let view = image
            .buffer_view
            .and_then(|view| self.json.buffer_views.get(view))
            .ok_or("a picture is nowhere")?;
        let (start, length) = (view.byte_offset, view.byte_length);
        let bytes = view
            .buffer
            .and_then(|buffer| self.buffers.get(buffer))
            .and_then(|buffer| buffer.get(start..start + length))
            .ok_or("a picture's bytes are missing")?;
        Texture::read(bytes).map_err(named)
    }

    /// What `uri` names: bytes written in it, or a file beside this one.
    fn beside(&self, uri: &str) -> Result<Vec<u8>, String> {
        if uri.starts_with("data:") {
            return uri
                .split_once(',')
                .and_then(|(_, data)| decoded(data))
                .ok_or("what is inside it is not base64".into());
        }
        let name = unescaped(uri);
        std::fs::read(self.folder.join(&name))
            .map_err(|error| format!("`{name}` beside it: {error}"))
    }

    /// The picture a material lays over its part, as a place among `out`'s, what it does past
    /// its edges, which of a corner's places it is read at, and how those are moved on it.
    fn base_picture(
        &self,
        material: &json::Material,
        out: &mut Read,
        read: &mut Vec<(usize, usize)>,
    ) -> Result<Option<Laid>, String> {
        let told = &material.pbr_metallic_roughness.base_color_texture;
        let Some(texture) = told.index.and_then(|index| self.json.textures.get(index)) else {
            return Ok(None);
        };
        let Some(image) = texture
            .source
            .or(texture.extensions.ext_texture_webp.source)
        else {
            return Ok(None);
        };
        let place = match read.iter().find(|(known, _)| *known == image) {
            Some((_, place)) => *place,
            None => {
                out.textures.push(self.picture(image)?);
                read.push((image, out.textures.len() - 1));
                out.textures.len() - 1
            }
        };
        let sampler = texture
            .sampler
            .and_then(|sampler| self.json.samplers.get(sampler));
        let moved = &told.extensions.khr_texture_transform;
        Ok(Some(Laid {
            texture: place,
            wrap: [
                Wrap::of(sampler.and_then(|sampler| sampler.wrap_s)),
                Wrap::of(sampler.and_then(|sampler| sampler.wrap_t)),
            ],
            set: moved.tex_coord.or(told.tex_coord).unwrap_or(0),
            offset: moved.offset.unwrap_or([0.0; 2]),
            turn: moved.rotation.unwrap_or(0.0),
            scale: moved.scale.unwrap_or([1.0; 2]),
        }))
    }

    fn indices(&self, index: json::Named) -> Result<Vec<u32>, String> {
        let (elements, component, across) = self.accessor(index)?;
        if across != 1 || !matches!(component, 5121 | 5123 | 5125) {
            return Err("indices are read as whole numbers".into());
        }
        Ok(elements
            .iter()
            .map(|element| match element.len() {
                1 => u32::from(element[0]),
                2 => u32::from(u16::from_le_bytes([element[0], element[1]])),
                _ => u32::from_le_bytes([element[0], element[1], element[2], element[3]]),
            })
            .collect())
    }

    /// The triangles of a primitive of the mesh of node `node`, as they are in the mesh's own
    /// space, with what places them: the node, or the bones of `skin`.
    fn primitive(
        &self,
        primitive: &json::Primitive,
        node: usize,
        skin: Option<usize>,
        weights: &[f64],
        out: &mut Read,
    ) -> Result<(), String> {
        let mode = primitive.mode.unwrap_or(4);
        if !matches!(mode, 4..=6) {
            // Points and lines have no sides.
            return Ok(());
        }
        let attributes = &primitive.attributes;
        let Some(position) = attributes.position else {
            return Ok(());
        };
        let points = self.threes(position)?;
        let normals = match attributes.normal {
            None => None,
            Some(normal) => {
                Some(self.threes(normal)?).filter(|normals| normals.len() == points.len())
            }
        };
        let indices = match primitive.indices {
            None => (0..points.len() as u32).collect(),
            Some(indices) => self.indices(indices)?,
        };
        if indices.iter().any(|index| *index as usize >= points.len()) {
            return Err("an index is past its points".into());
        }
        let corners: Vec<[u32; 3]> = match mode {
            4 => indices.as_chunks::<3>().0.to_vec(),
            5 => indices
                .windows(3)
                .map(|three| [three[0], three[1], three[2]])
                .collect(),
            _ => indices
                .windows(2)
                .skip(1)
                .map(|pair| [indices[0], pair[0], pair[1]])
                .collect(),
        };
        // Which bones move each corner, and how much each: the first four it names.
        let bones: Option<Vec<Bones>> = match (skin, attributes.joints, attributes.weights) {
            (Some(_), Some(joints), Some(weights)) => {
                let (joints, weights) = (self.numbers(joints, 4)?, self.numbers(weights, 4)?);
                Some(
                    joints
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(weights.as_chunks::<4>().0)
                        .map(|(joints, weights)| {
                            let all: f64 = weights.iter().sum();
                            (
                                joints.map(|joint| joint as u16),
                                weights.map(|weight| (weight / all.max(1e-12)) as f32),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .filter(|bones| bones.len() == points.len())
            }
            _ => None,
        };
        let skin = skin.filter(|_| bones.is_some());
        let bone = |corner: u32| {
            bones
                .as_ref()
                .map_or(([0; 4], [0.0; 4]), |bones| bones[corner as usize])
        };
        let material = primitive
            .material
            .and_then(|material| self.json.materials.get(material));
        let laid = match material {
            Some(material) => {
                let mut read = self.pictures.borrow_mut();
                self.base_picture(material, out, &mut read)?
            }
            None => None,
        };
        // Where each corner is on the picture, where the part has one and says where.
        let uvs: Option<Vec<[f32; 2]>> = match &laid {
            Some(laid) => match attributes.texcoord(laid.set) {
                None => None,
                Some(told) => Some(
                    self.numbers(told, 2)?
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| laid.at(*pair))
                        .collect::<Vec<_>>(),
                )
                .filter(|uvs| uvs.len() == points.len()),
            },
            None => None,
        };
        let laid = laid.filter(|_| uvs.is_some());
        // A corner's color is three numbers or, with how clear it is, four.
        let colors: Option<Vec<[f32; 3]>> = match attributes.color {
            None => None,
            Some(told) => {
                let across = match self.accessor(told)?.2 {
                    4 => 4,
                    _ => 3,
                };
                Some(
                    self.numbers(told, across)?
                        .chunks(across)
                        .map(|color| [color[0] as f32, color[1] as f32, color[2] as f32])
                        .collect::<Vec<_>>(),
                )
                .filter(|colors| colors.len() == points.len())
            }
        };
        let uv = |corner: u32| uvs.as_ref().map_or([0.0; 2], |uvs| uvs[corner as usize]);
        let colored = |corner: u32| {
            colors
                .as_ref()
                .map_or([1.0; 3], |colors| colors[corner as usize])
        };
        // Its morph targets: how far each takes each point. One that does not say, or says
        // it of another number of points, takes none anywhere.
        let targets: Vec<Vec<Vector>> = primitive
            .targets
            .iter()
            .map(|target| match target.position {
                None => Ok(vec![[0.0; 3]; points.len()]),
                Some(told) => self.threes(told).map(|moved| {
                    if moved.len() == points.len() {
                        moved
                    } else {
                        vec![[0.0; 3]; points.len()]
                    }
                }),
            })
            .collect::<Result<_, String>>()?;
        let rig = &mut out.rig;
        let first = rig.points.len();
        let first_triangle = out.triangles.len();
        let mut morphs: Vec<Vec<Vector>> = vec![Vec::new(); targets.len()];
        match &normals {
            Some(normals) => {
                rig.points.extend(&points);
                rig.normals.extend(normals);
                morphs.clone_from(&targets);
                rig.bones.extend((0..points.len() as u32).map(bone));
                out.uvs.extend((0..points.len() as u32).map(uv));
                out.colors.extend((0..points.len() as u32).map(colored));
                out.triangles.extend(
                    corners
                        .iter()
                        .map(|three| three.map(|corner| first as u32 + corner)),
                );
            }
            // With no normals told, each triangle is flat, and has corners of its own.
            None => {
                for three in &corners {
                    let at = rig.points.len() as u32;
                    rig.points
                        .extend(three.map(|corner| points[corner as usize]));
                    for (morph, target) in morphs.iter_mut().zip(&targets) {
                        morph.extend(three.map(|corner| target[corner as usize]));
                    }
                    rig.normals.extend([[0.0; 3]; 3]);
                    rig.bones.extend(three.map(bone));
                    out.uvs.extend(three.map(uv));
                    out.colors.extend(three.map(colored));
                    out.triangles.push([at, at + 1, at + 2]);
                }
            }
        }
        rig.runs.push(Run {
            corners: (first, rig.points.len()),
            triangles: (first_triangle, out.triangles.len()),
            node,
            skin,
            flat: normals.is_none(),
            weights: (0..morphs.len())
                .map(|target| weights.get(target).copied().unwrap_or(0.0))
                .collect(),
            morphs,
        });
        let color = material.map(|material| {
            match material.pbr_metallic_roughness.base_color_factor.as_deref() {
                Some([red, green, blue, ..]) => [*red as f32, *green as f32, *blue as f32],
                _ => [1.0; 3],
            }
        });
        out.parts.push(Part {
            end: out.triangles.len(),
            color,
            texture: laid.map(|laid| (laid.texture, laid.wrap)),
            colored: colors.is_some(),
        });
        Ok(())
    }

    /// The file's skins: the nodes that are each one's bones, and where each bone was when the
    /// mesh was bound to it, undone.
    fn skins(&self) -> Result<Vec<Skin>, String> {
        let mut skins = Vec::new();
        for skin in &self.json.skins {
            let joints: Vec<usize> = skin.joints.iter().flatten().copied().collect();
            let inverse = match skin.inverse_bind_matrices {
                None => vec![Affine3::IDENTITY; joints.len()],
                Some(told) => self
                    .numbers(told, 16)?
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .map(whole)
                    .collect(),
            };
            if inverse.len() != joints.len() {
                return Err("a skin's bones and where they were bound do not match".into());
            }
            skins.push(Skin { joints, inverse });
        }
        Ok(skins)
    }

    /// The file's animations: what each says of a node's place over time, and of how much
    /// of each morph target its mesh has.
    fn animations(&self, nodes: usize) -> Result<Vec<Animation>, String> {
        let mut animations = Vec::new();
        for animation in &self.json.animations {
            let mut channels = Vec::new();
            let mut morphs = Vec::new();
            let sampler_of = |channel: &json::Channel| {
                channel
                    .sampler
                    .and_then(|sampler| animation.samplers.get(sampler))
                    .ok_or("an animation's sampler is missing")
            };
            for channel in &animation.channels {
                if channel.target.path == Some(json::Animated::Weights) {
                    let node = channel.target.node;
                    let sampler = sampler_of(channel)?;
                    let how = sampler.interpolation;
                    let times = self.numbers(sampler.input, 1)?;
                    let values = self.numbers(sampler.output, 1)?;
                    let each = times.len() * if how == How::Cubic { 3 } else { 1 };
                    if let Some(node) = node.filter(|node| *node < nodes) {
                        if times.is_empty() || values.len() % each != 0 || values.is_empty() {
                            return Err("an animation's times and weights do not match".into());
                        }
                        morphs.push(Weighed {
                            node,
                            how,
                            across: values.len() / each,
                            times,
                            values,
                        });
                    }
                    continue;
                }
                let (what, across) = match channel.target.path {
                    Some(json::Animated::Translation) => (What::Moved, 3),
                    Some(json::Animated::Rotation) => (What::Turned, 4),
                    Some(json::Animated::Scale) => (What::Sized, 3),
                    _ => continue,
                };
                let Some(node) = channel.target.node.filter(|node| *node < nodes) else {
                    continue;
                };
                let sampler = sampler_of(channel)?;
                let how = sampler.interpolation;
                let times = self.numbers(sampler.input, 1)?;
                let values = self.numbers(sampler.output, across)?;
                let each = across * if how == How::Cubic { 3 } else { 1 };
                if times.is_empty() || values.len() != times.len() * each {
                    return Err("an animation's times and values do not match".into());
                }
                channels.push(Channel {
                    node,
                    what,
                    how,
                    across,
                    times,
                    values,
                });
            }
            let length = channels
                .iter()
                .filter_map(|channel| channel.times.last())
                .chain(morphs.iter().filter_map(|told| told.times.last()))
                .fold(0.0f64, |length, last| length.max(*last));
            animations.push(Animation {
                name: animation.name.clone(),
                length,
                channels,
                morphs,
            });
        }
        Ok(animations)
    }
}

/// The glTF file at `path`: its triangles at rest, and what moves them.
pub(super) fn read(path: &Path) -> Result<Read, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let (json, binary) = parts_of(&bytes)?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut file = File {
        json: &json,
        buffers: Vec::new(),
        folder,
        pictures: std::cell::RefCell::new(Vec::new()),
    };
    for (index, buffer) in json.buffers.iter().enumerate() {
        let bytes = match &buffer.uri {
            Some(uri) => file.beside(uri).map_err(|why| format!("a buffer: {why}"))?,
            None if index == 0 => binary.ok_or("its first buffer is nowhere")?.to_vec(),
            None => return Err("a buffer is nowhere".into()),
        };
        file.buffers.push(bytes);
    }
    let told = &json.nodes;
    let mut nodes: Vec<Node> = told
        .iter()
        .map(|node| Node {
            parent: None,
            matrix: node.matrix.as_ref().map(whole),
            told: Told::of(node),
        })
        .collect();
    let under = |node: &json::Node| -> Vec<usize> {
        node.children
            .iter()
            .flatten()
            .copied()
            .filter(|child| *child < told.len())
            .collect()
    };
    for (index, node) in told.iter().enumerate() {
        for child in under(node) {
            nodes[child].parent = Some(index);
        }
    }
    // Every node after the one it is under, from those under none.
    let mut order: Vec<usize> = (0..nodes.len())
        .filter(|node| nodes[*node].parent.is_none())
        .collect();
    let mut next = 0;
    while next < order.len() && order.len() <= nodes.len() {
        order.extend(under(&told[order[next]]));
        next += 1;
    }
    if order.len() != nodes.len() {
        return Err("its nodes go round in a ring".into());
    }
    // The nodes of the scene it opens on, or, where it names no scene, those under no other.
    let scene = json.scene.unwrap_or(0);
    let mut shown: Vec<usize> = match json
        .scenes
        .get(scene)
        .and_then(|scene| scene.nodes.as_ref())
    {
        Some(roots) => roots
            .iter()
            .copied()
            .filter(|root| *root < nodes.len())
            .collect(),
        None => (0..nodes.len())
            .filter(|node| nodes[*node].parent.is_none())
            .collect(),
    };
    let mut next = 0;
    while next < shown.len() && shown.len() <= nodes.len() {
        shown.extend(under(&told[shown[next]]));
        next += 1;
    }
    let mut out = Read {
        points: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        colors: Vec::new(),
        triangles: Vec::new(),
        parts: Vec::new(),
        textures: Vec::new(),
        animations: file.animations(nodes.len())?,
        cameras: told
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let camera = json.cameras.get(node.camera?)?;
                let name = camera.name.as_deref().or(node.name.as_deref());
                Some(Camera {
                    name: name.unwrap_or("").to_owned(),
                    node: index,
                    fov: camera.perspective.yfov.map(f64::to_degrees),
                    aspect: camera.perspective.aspect_ratio,
                })
            })
            .collect(),
        rig: Rig {
            nodes,
            order,
            points: Vec::new(),
            normals: Vec::new(),
            bones: Vec::new(),
            runs: Vec::new(),
            skins: file.skins()?,
        },
        closed: std::sync::OnceLock::new(),
    };
    for index in shown {
        let node = &told[index];
        let Some(mesh) = node.mesh.and_then(|mesh| json.meshes.get(mesh)) else {
            continue;
        };
        let skin = node.skin.filter(|skin| *skin < out.rig.skins.len());
        // How much of each morph target it has at rest: as its node says, or else its mesh.
        let weights = node
            .weights
            .as_ref()
            .or(mesh.weights.as_ref())
            .cloned()
            .unwrap_or_default();
        for primitive in &mesh.primitives {
            file.primitive(primitive, index, skin, &weights, &mut out)?;
        }
    }
    if out.triangles.is_empty() {
        return Err("it has no triangles".into());
    }
    (out.points, out.normals) = out.posed(None);
    Ok(out)
}

/// What of a glTF file's JSON is read, as glTF names it: nothing else of it is.
///
/// Each field reads on its own, as when the JSON was read by key. A field the file leaves out,
/// says as `null`, or says as a value of another kind (`"mesh": "0"`) reads as one left out; an
/// object said as anything but an object says nothing of what it would hold; and in a list,
/// each item reads on its own. So what a file says wrong fails it where it is used, as it
/// always has, and not where it is read: a part no node shows fails nothing.
mod json {
    use std::collections::BTreeMap;

    use serde::de::{DeserializeOwned, Deserializer, IgnoredAny};
    use serde::Deserialize;
    use serde_json::Value as Json;

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Gltf {
        /// The scene it opens on.
        #[serde(deserialize_with = "as_said")]
        pub scene: Option<usize>,
        #[serde(deserialize_with = "each_said")]
        pub scenes: Vec<Scene>,
        #[serde(deserialize_with = "each_said")]
        pub nodes: Vec<Node>,
        #[serde(deserialize_with = "each_said")]
        pub meshes: Vec<Mesh>,
        #[serde(deserialize_with = "each_said")]
        pub accessors: Vec<Accessor>,
        #[serde(deserialize_with = "each_said")]
        pub buffer_views: Vec<BufferView>,
        #[serde(deserialize_with = "each_said")]
        pub buffers: Vec<Buffer>,
        #[serde(deserialize_with = "each_said")]
        pub materials: Vec<Material>,
        #[serde(deserialize_with = "each_said")]
        pub textures: Vec<Texture>,
        #[serde(deserialize_with = "each_said")]
        pub images: Vec<Image>,
        #[serde(deserialize_with = "each_said")]
        pub samplers: Vec<Sampler>,
        #[serde(deserialize_with = "each_said")]
        pub cameras: Vec<Camera>,
        #[serde(deserialize_with = "each_said")]
        pub skins: Vec<Skin>,
        #[serde(deserialize_with = "each_said")]
        pub animations: Vec<Animation>,
    }

    impl Gltf {
        /// A file's JSON as these types: JSON that is no object holds none of them.
        pub(super) fn of(json: Json) -> Self {
            object(json)
        }
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Scene {
        /// The nodes it shows with all they hold, where it says.
        #[serde(deserialize_with = "indices_said")]
        pub nodes: Option<Vec<usize>>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Node {
        /// Its place, a column at a time, where it gives it whole.
        #[serde(deserialize_with = "as_said")]
        pub matrix: Option<[f64; 16]>,
        #[serde(deserialize_with = "as_said")]
        pub translation: Option<[f64; 3]>,
        /// A turn as a quaternion: x, y, z, w.
        #[serde(deserialize_with = "as_said")]
        pub rotation: Option<[f64; 4]>,
        #[serde(deserialize_with = "as_said")]
        pub scale: Option<[f64; 3]>,
        #[serde(deserialize_with = "indices_said")]
        pub children: Option<Vec<usize>>,
        #[serde(deserialize_with = "as_said")]
        pub mesh: Option<usize>,
        #[serde(deserialize_with = "as_said")]
        pub skin: Option<usize>,
        /// How much of each morph target its mesh has at rest.
        #[serde(deserialize_with = "as_said")]
        pub weights: Option<Vec<f64>>,
        #[serde(deserialize_with = "as_said")]
        pub camera: Option<usize>,
        #[serde(deserialize_with = "as_said")]
        pub name: Option<String>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Mesh {
        #[serde(deserialize_with = "each_said")]
        pub primitives: Vec<Primitive>,
        /// How much of each morph target it has at rest, where its node does not say.
        #[serde(deserialize_with = "as_said")]
        pub weights: Option<Vec<f64>>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Primitive {
        /// What its corners make: 4 triangles, 5 a strip of them, 6 a fan, and less, points
        /// and lines.
        #[serde(deserialize_with = "as_said")]
        pub mode: Option<u64>,
        #[serde(deserialize_with = "object_said")]
        pub attributes: Attributes,
        pub indices: Option<Named>,
        #[serde(deserialize_with = "as_said")]
        pub material: Option<usize>,
        #[serde(deserialize_with = "each_said")]
        pub targets: Vec<Morph>,
    }

    /// What a primitive's corners have, each in the accessor named.
    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Attributes {
        #[serde(rename = "POSITION")]
        pub position: Option<Named>,
        #[serde(rename = "NORMAL")]
        pub normal: Option<Named>,
        /// The first four bones that move each corner.
        #[serde(rename = "JOINTS_0")]
        pub joints: Option<Named>,
        /// How much each of those bones moves it.
        #[serde(rename = "WEIGHTS_0")]
        pub weights: Option<Named>,
        #[serde(rename = "COLOR_0")]
        pub color: Option<Named>,
        /// The rest, by name: among them each corner's places on a picture, `TEXCOORD_0` and on.
        #[serde(flatten)]
        rest: BTreeMap<String, Option<Named>>,
    }

    impl Attributes {
        /// Where each corner is on a picture, by which of its places that is.
        pub(super) fn texcoord(&self, set: u64) -> Option<Named> {
            self.rest.get(&format!("TEXCOORD_{set}")).copied().flatten()
        }
    }

    /// A morph target: how far it takes each corner of its primitive.
    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Morph {
        #[serde(rename = "POSITION")]
        pub position: Option<Named>,
    }

    /// An accessor as something names it: by its number, where it names it by one. Named by
    /// anything else, it is one that is missing, which is not the same as one not named.
    #[derive(Clone, Copy, Default)]
    pub(super) struct Named(pub Option<usize>);

    impl<'de> Deserialize<'de> for Named {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let told = Json::deserialize(deserializer)?;
            Ok(Self(told.as_u64().map(|index| index as usize)))
        }
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Accessor {
        #[serde(deserialize_with = "as_said")]
        pub buffer_view: Option<usize>,
        #[serde(deserialize_with = "as_said")]
        pub byte_offset: usize,
        /// What each number is, as OpenGL numbers its kinds: 5126 a float.
        #[serde(deserialize_with = "as_said")]
        pub component_type: u64,
        /// How many numbers an element is: none for a type that is not read.
        #[serde(rename = "type", deserialize_with = "text_said")]
        pub shape: Option<Shape>,
        #[serde(deserialize_with = "as_said")]
        pub count: usize,
        /// Whether its whole numbers are shares of the most they can hold.
        #[serde(deserialize_with = "as_said")]
        pub normalized: bool,
        /// What it changes of its view's numbers, which is not read: anything but `null`.
        pub sparse: Option<IgnoredAny>,
    }

    #[derive(Clone, Copy, Deserialize)]
    #[serde(rename_all = "UPPERCASE")]
    pub(super) enum Shape {
        Scalar,
        Vec2,
        Vec3,
        Vec4,
        Mat4,
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct BufferView {
        #[serde(deserialize_with = "as_said")]
        pub buffer: Option<usize>,
        #[serde(deserialize_with = "as_said")]
        pub byte_offset: usize,
        #[serde(deserialize_with = "as_said")]
        pub byte_length: usize,
        /// How far apart its elements are, where that is more than an element's size.
        #[serde(deserialize_with = "as_said")]
        pub byte_stride: usize,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Buffer {
        /// Where its bytes are, or the bytes themselves: in a `.glb`, the first has none.
        #[serde(deserialize_with = "as_said")]
        pub uri: Option<String>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Material {
        #[serde(deserialize_with = "object_said")]
        pub pbr_metallic_roughness: MetallicRoughness,
    }

    /// What a material says of its color.
    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct MetallicRoughness {
        /// Red, green, blue, and how clear it is, in linear light.
        #[serde(deserialize_with = "as_said")]
        pub base_color_factor: Option<Vec<f64>>,
        #[serde(deserialize_with = "object_said")]
        pub base_color_texture: TextureInfo,
    }

    /// The picture a material lays over its part.
    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct TextureInfo {
        /// Which of the file's textures it is.
        #[serde(deserialize_with = "as_said")]
        pub index: Option<usize>,
        /// Which of a corner's places on a picture it is read at.
        #[serde(deserialize_with = "as_said")]
        pub tex_coord: Option<u64>,
        #[serde(deserialize_with = "object_said")]
        pub extensions: TextureInfoExtensions,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct TextureInfoExtensions {
        #[serde(rename = "KHR_texture_transform", deserialize_with = "object_said")]
        pub khr_texture_transform: TextureTransform,
    }

    /// How a corner's places on a picture are moved on it: sized, turned, and then moved.
    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct TextureTransform {
        #[serde(deserialize_with = "as_said")]
        pub offset: Option<[f64; 2]>,
        /// How far it is turned, in radians.
        #[serde(deserialize_with = "as_said")]
        pub rotation: Option<f64>,
        #[serde(deserialize_with = "as_said")]
        pub scale: Option<[f64; 2]>,
        /// Which of a corner's places it is read at, in place of the one its picture says.
        #[serde(deserialize_with = "as_said")]
        pub tex_coord: Option<u64>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Texture {
        /// Which of the file's images it is.
        #[serde(deserialize_with = "as_said")]
        pub source: Option<usize>,
        #[serde(deserialize_with = "as_said")]
        pub sampler: Option<usize>,
        #[serde(deserialize_with = "object_said")]
        pub extensions: TextureExtensions,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct TextureExtensions {
        #[serde(rename = "EXT_texture_webp", deserialize_with = "object_said")]
        pub ext_texture_webp: WebpTexture,
    }

    /// The image that is a texture as a WebP, where it has no other.
    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct WebpTexture {
        #[serde(deserialize_with = "as_said")]
        pub source: Option<usize>,
    }

    /// What a picture does past its edges, across and down, as glTF numbers it ([`super::Wrap`]).
    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Sampler {
        #[serde(deserialize_with = "as_said")]
        pub wrap_s: Option<u64>,
        #[serde(deserialize_with = "as_said")]
        pub wrap_t: Option<u64>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Image {
        /// Where it is, or its bytes themselves.
        #[serde(deserialize_with = "as_said")]
        pub uri: Option<String>,
        /// Where its bytes are, where it has no `uri`.
        #[serde(deserialize_with = "as_said")]
        pub buffer_view: Option<usize>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Camera {
        #[serde(deserialize_with = "as_said")]
        pub name: Option<String>,
        #[serde(deserialize_with = "object_said")]
        pub perspective: Perspective,
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Perspective {
        /// How much it sees from the top of its picture to the bottom, in radians.
        #[serde(deserialize_with = "as_said")]
        pub yfov: Option<f64>,
        #[serde(deserialize_with = "as_said")]
        pub aspect_ratio: Option<f64>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    pub(super) struct Skin {
        /// The nodes that are its bones.
        #[serde(deserialize_with = "indices_said")]
        pub joints: Option<Vec<usize>>,
        /// Where each bone was when the mesh was bound to it, undone.
        pub inverse_bind_matrices: Option<Named>,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Animation {
        #[serde(deserialize_with = "as_said")]
        pub name: String,
        #[serde(deserialize_with = "each_said")]
        pub channels: Vec<Channel>,
        #[serde(deserialize_with = "each_said")]
        pub samplers: Vec<AnimationSampler>,
    }

    /// What an animation moves, and by which of its samplers.
    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Channel {
        #[serde(deserialize_with = "as_said")]
        pub sampler: Option<usize>,
        #[serde(deserialize_with = "object_said")]
        pub target: Target,
    }

    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct Target {
        #[serde(deserialize_with = "as_said")]
        pub node: Option<usize>,
        /// What of its node it moves: none for what is not read.
        #[serde(deserialize_with = "text_said")]
        pub path: Option<Animated>,
    }

    #[derive(Clone, Copy, PartialEq, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub(super) enum Animated {
        Translation,
        Rotation,
        Scale,
        /// How much of each morph target its mesh has.
        Weights,
    }

    /// An animation's keys: when each is, in `input`, and what it is, in `output`.
    #[derive(Default, Deserialize)]
    #[serde(default)]
    pub(super) struct AnimationSampler {
        pub input: Named,
        pub output: Named,
        #[serde(deserialize_with = "text_said")]
        pub interpolation: super::How,
    }

    /// A field as said, or its default where what is said is not of its type.
    fn as_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        Ok(serde_json::from_value(Json::deserialize(deserializer)?).unwrap_or_default())
    }

    /// A field said by name, as [`as_said`] reads one, but only from a text: a name read from
    /// anything else, as from an object of one key (`{"STEP": null}`), is no name.
    fn text_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        Ok(match Json::deserialize(deserializer)? {
            text @ Json::String(_) => serde_json::from_value(text).unwrap_or_default(),
            _ => T::default(),
        })
    }

    /// An object, each of its fields read on its own ([`object`]).
    fn object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        Ok(object(Json::deserialize(deserializer)?))
    }

    /// A list of objects, each read on its own ([`object`]): an item said otherwise is still
    /// one, of its default. What is not a list is empty.
    fn each_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
        deserializer: D,
    ) -> Result<Vec<T>, D::Error> {
        Ok(match Json::deserialize(deserializer)? {
            Json::Array(items) => items.into_iter().map(object).collect(),
            _ => Vec::new(),
        })
    }

    /// What a list names by number, leaving out what it names otherwise: none where it is no
    /// list.
    fn indices_said<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<usize>>, D::Error> {
        Ok(match Json::deserialize(deserializer)? {
            Json::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Json::as_u64)
                    .map(|index| index as usize)
                    .collect(),
            ),
            _ => None,
        })
    }

    /// An object's fields, each read on its own. What is no object says none of them: a type
    /// would otherwise read a list as its fields in order.
    fn object<T: DeserializeOwned + Default>(value: Json) -> T {
        match value {
            Json::Object(_) => serde_json::from_value(value).unwrap_or_default(),
            _ => T::default(),
        }
    }
}
