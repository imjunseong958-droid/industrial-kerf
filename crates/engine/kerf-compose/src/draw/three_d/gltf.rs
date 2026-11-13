//! The triangles of a glTF file, for a mesh read from one (`gen=mesh file=`).
//!
//! What a guide wants of a model is its shape at rest and its own colors: the triangles of every
//! mesh its scene's nodes hold, each where its node puts it, each material's base color, the
//! picture a material lays over its part, and the colors its corners are given. And how it
//! moves: its animations of where its nodes are, and the skins by which a mesh bends with the
//! nodes that are its bones, so that it can be posed at any time of one ([`Read::posed`]). Morph
//! targets, cameras, and lights in the file are not read, and of a material nothing but its base
//! color: how a thing shines is a model's to draw. A `.gltf` with its buffers and
//! pictures beside it or inside it, and a `.glb`, are read.
//!
//! glTF's x runs right, its y up, and its z toward the viewer. Kerf's y runs down and its z away,
//! so a model is turned half round about x, which keeps it the way round it was.

use std::path::Path;

use serde_json::Value as Json;

use super::texture::{Texture, Wrap};

type Vector = [f64; 3];
/// A node's place: rows of three across and what it is moved by.
type Matrix = [[f64; 4]; 3];

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
    rig: Rig,
}

impl Read {
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
            let bones: Option<Vec<Matrix>> = run.skin.map(|skin| {
                let skin = &rig.skins[skin];
                skin.joints
                    .iter()
                    .zip(&skin.inverse)
                    .map(|(joint, inverse)| match places.get(*joint) {
                        Some(place) => then(place, inverse),
                        None => IDENTITY,
                    })
                    .collect()
            });
            for corner in run.corners.0..run.corners.1 {
                let placed = match &bones {
                    Some(bones) => {
                        let (joints, weights) = rig.bones[corner];
                        let mut sum = [[0.0; 4]; 3];
                        let mut all = 0.0;
                        for (joint, weight) in joints.iter().zip(weights) {
                            let Some(bone) = bones.get(*joint as usize) else {
                                continue;
                            };
                            let weight = f64::from(weight);
                            all += weight;
                            for (sum, bone) in sum.iter_mut().flatten().zip(bone.iter().flatten()) {
                                *sum += bone * weight;
                            }
                        }
                        if all > 1e-9 {
                            Placed::by(sum)
                        } else {
                            whole
                        }
                    }
                    None => whole,
                };
                points.push(placed.point(rig.points[corner]));
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

#[derive(Clone, Copy, PartialEq)]
enum How {
    Step,
    Linear,
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
    fn of(node: &Json) -> Self {
        let read = |name: &str, otherwise: &[f64]| {
            numbers(&node[name])
                .filter(|told| told.len() == otherwise.len())
                .unwrap_or_else(|| otherwise.to_vec())
        };
        let (t, q, s) = (
            read("translation", &[0.0; 3]),
            read("rotation", &[0.0, 0.0, 0.0, 1.0]),
            read("scale", &[1.0; 3]),
        );
        Self {
            moved: [t[0], t[1], t[2]],
            turned: [q[0], q[1], q[2], q[3]],
            sized: [s[0], s[1], s[2]],
        }
    }

    fn place(&self) -> Matrix {
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
        [0, 1, 2].map(|row| {
            [
                turn[row][0] * s[0],
                turn[row][1] * s[1],
                turn[row][2] * s[2],
                t[row],
            ]
        })
    }
}

struct Node {
    /// The node it is under.
    parent: Option<usize>,
    /// Its place where the file gives it whole, which no animation moves.
    matrix: Option<Matrix>,
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
}

struct Skin {
    /// The nodes that are its bones.
    joints: Vec<usize>,
    /// For each bone, the way back from where the mesh was bound to it.
    inverse: Vec<Matrix>,
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
    fn places(&self, animation: Option<(&Animation, f64)>) -> Vec<Matrix> {
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
        let mut places = vec![IDENTITY; told.len()];
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
    place: Matrix,
    /// What turns the way a side faces: the cofactors of what moved it, which is its inverse
    /// turned over, but for its size.
    turn: [[f64; 3]; 3],
}

impl Placed {
    fn by(place: Matrix) -> Self {
        let cofactor = |row: usize, column: usize| {
            let (r1, r2) = ((row + 1) % 3, (row + 2) % 3);
            let (c1, c2) = ((column + 1) % 3, (column + 2) % 3);
            place[r1][c1] * place[r2][c2] - place[r1][c2] * place[r2][c1]
        };
        Self {
            place,
            turn: [0, 1, 2].map(|row| [0, 1, 2].map(|column| cofactor(row, column))),
        }
    }

    fn point(&self, point: Vector) -> Vector {
        let at = [0, 1, 2].map(|row| {
            self.place[row][0] * point[0]
                + self.place[row][1] * point[1]
                + self.place[row][2] * point[2]
                + self.place[row][3]
        });
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

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn then(outer: &Matrix, inner: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 3];
    for (row, out) in out.iter_mut().enumerate() {
        for (column, out) in out.iter_mut().enumerate() {
            *out = (0..3)
                .map(|k| outer[row][k] * inner[k][column])
                .sum::<f64>()
                + if column == 3 { outer[row][3] } else { 0.0 };
        }
    }
    out
}

fn numbers(value: &Json) -> Option<Vec<f64>> {
    value.as_array()?.iter().map(Json::as_f64).collect()
}

/// A node's place where the file gives it whole.
fn matrix_of(node: &Json) -> Option<Matrix> {
    let m = numbers(&node["matrix"]).filter(|m| m.len() == 16)?;
    // Written a column at a time.
    Some([0, 1, 2].map(|row| [0, 1, 2, 3].map(|column| m[column * 4 + row])))
}

/// A node's own place: its `matrix`, or its translation, rotation, and scale.
#[cfg(test)]
fn local(node: &Json) -> Matrix {
    matrix_of(node).unwrap_or_else(|| Told::of(node).place())
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

/// The file's JSON and, in a `.glb`, the bytes that come with it.
fn parts_of(bytes: &[u8]) -> Result<(Json, Option<&[u8]>), String> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|four| u32::from_le_bytes([four[0], four[1], four[2], four[3]]) as usize)
            .ok_or("it ends early")
    };
    if !bytes.starts_with(b"glTF") {
        let json = serde_json::from_slice(bytes).map_err(|error| format!("its JSON: {error}"))?;
        return Ok((json, None));
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
    let json = serde_json::from_slice(json).map_err(|error| format!("its JSON: {error}"))?;
    Ok((json, binary))
}

struct File<'a> {
    json: &'a Json,
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
    fn accessor(&self, index: &Json) -> Result<(Vec<&[u8]>, u64, usize), String> {
        let accessor = index
            .as_u64()
            .and_then(|index| self.json["accessors"].get(index as usize))
            .ok_or("an accessor is missing")?;
        if !accessor["sparse"].is_null() {
            return Err("a sparse accessor is not read".into());
        }
        let component = accessor["componentType"].as_u64().unwrap_or(0);
        let width = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err("an accessor has no component type".into()),
        };
        let across = match accessor["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            _ => return Err("an accessor is of a type that is not read".into()),
        };
        let count = accessor["count"].as_u64().unwrap_or(0) as usize;
        let view = accessor["bufferView"]
            .as_u64()
            .and_then(|view| self.json["bufferViews"].get(view as usize))
            .ok_or("an accessor has no buffer view")?;
        let buffer = view["buffer"]
            .as_u64()
            .and_then(|buffer| self.buffers.get(buffer as usize))
            .ok_or("a buffer is missing")?;
        let offset = |of: &Json| of["byteOffset"].as_u64().unwrap_or(0) as usize;
        let start = offset(view) + offset(accessor);
        let size = width * across;
        let stride = (view["byteStride"].as_u64().unwrap_or(0) as usize).max(size);
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

    fn threes(&self, index: &Json) -> Result<Vec<Vector>, String> {
        Ok(self.numbers(index, 3)?.as_chunks::<3>().0.to_vec())
    }

    /// The numbers of accessor `index`, `across` to an element: floats as they are, and whole
    /// numbers as shares of the most they can hold where the accessor says they are.
    fn numbers(&self, index: &Json, across: usize) -> Result<Vec<f64>, String> {
        let (elements, component, told) = self.accessor(index)?;
        if told != across {
            return Err(format!(
                "an accessor read {across} to an element has {told}"
            ));
        }
        let shares = index
            .as_u64()
            .and_then(|index| self.json["accessors"].get(index as usize))
            .is_some_and(|accessor| accessor["normalized"].as_bool() == Some(true));
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
        let image = self.json["images"]
            .get(index)
            .ok_or("a picture is missing")?;
        let named = |why: String| format!("its picture {index}: {why}");
        if let Some(uri) = image["uri"].as_str() {
            let bytes = self.beside(uri).map_err(named)?;
            return Texture::read(&bytes).map_err(named);
        }
        let view = image["bufferView"]
            .as_u64()
            .and_then(|view| self.json["bufferViews"].get(view as usize))
            .ok_or("a picture is nowhere")?;
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"].as_u64().unwrap_or(0) as usize;
        let bytes = view["buffer"]
            .as_u64()
            .and_then(|buffer| self.buffers.get(buffer as usize))
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
        material: &Json,
        out: &mut Read,
        read: &mut Vec<(usize, usize)>,
    ) -> Result<Option<Laid>, String> {
        let told = &material["pbrMetallicRoughness"]["baseColorTexture"];
        let Some(texture) = told["index"]
            .as_u64()
            .and_then(|index| self.json["textures"].get(index as usize))
        else {
            return Ok(None);
        };
        let Some(image) = texture["source"]
            .as_u64()
            .or_else(|| texture["extensions"]["EXT_texture_webp"]["source"].as_u64())
        else {
            return Ok(None);
        };
        let image = image as usize;
        let place = match read.iter().find(|(known, _)| *known == image) {
            Some((_, place)) => *place,
            None => {
                out.textures.push(self.picture(image)?);
                read.push((image, out.textures.len() - 1));
                out.textures.len() - 1
            }
        };
        let sampler = texture["sampler"]
            .as_u64()
            .and_then(|sampler| self.json["samplers"].get(sampler as usize));
        let wrap = |way: &str| Wrap::of(sampler.and_then(|sampler| sampler[way].as_u64()));
        let moved = &told["extensions"]["KHR_texture_transform"];
        let pair = |name: &str, otherwise: f64| {
            numbers(&moved[name])
                .filter(|pair| pair.len() == 2)
                .map_or([otherwise; 2], |pair| [pair[0], pair[1]])
        };
        Ok(Some(Laid {
            texture: place,
            wrap: [wrap("wrapS"), wrap("wrapT")],
            set: moved["texCoord"]
                .as_u64()
                .or(told["texCoord"].as_u64())
                .unwrap_or(0),
            offset: pair("offset", 0.0),
            turn: moved["rotation"].as_f64().unwrap_or(0.0),
            scale: pair("scale", 1.0),
        }))
    }

    fn indices(&self, index: &Json) -> Result<Vec<u32>, String> {
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
        primitive: &Json,
        node: usize,
        skin: Option<usize>,
        out: &mut Read,
    ) -> Result<(), String> {
        let mode = primitive["mode"].as_u64().unwrap_or(4);
        if !matches!(mode, 4..=6) {
            // Points and lines have no sides.
            return Ok(());
        }
        let attributes = &primitive["attributes"];
        if attributes["POSITION"].is_null() {
            return Ok(());
        }
        let points = self.threes(&attributes["POSITION"])?;
        let normals = match &attributes["NORMAL"] {
            Json::Null => None,
            normal => Some(self.threes(normal)?).filter(|normals| normals.len() == points.len()),
        };
        let indices = match &primitive["indices"] {
            Json::Null => (0..points.len() as u32).collect(),
            indices => self.indices(indices)?,
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
        let bones: Option<Vec<Bones>> =
            match (skin, &attributes["JOINTS_0"], &attributes["WEIGHTS_0"]) {
                (None, _, _) | (_, Json::Null, _) | (_, _, Json::Null) => None,
                (Some(_), joints, weights) => {
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
            };
        let skin = skin.filter(|_| bones.is_some());
        let bone = |corner: u32| {
            bones
                .as_ref()
                .map_or(([0; 4], [0.0; 4]), |bones| bones[corner as usize])
        };
        let material = primitive["material"]
            .as_u64()
            .and_then(|material| self.json["materials"].get(material as usize));
        let laid = match material {
            Some(material) => {
                let mut read = self.pictures.borrow_mut();
                self.base_picture(material, out, &mut read)?
            }
            None => None,
        };
        // Where each corner is on the picture, where the part has one and says where.
        let uvs: Option<Vec<[f32; 2]>> = match &laid {
            Some(laid) => match &attributes[format!("TEXCOORD_{}", laid.set).as_str()] {
                Json::Null => None,
                told => Some(
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
        let colors: Option<Vec<[f32; 3]>> = match &attributes["COLOR_0"] {
            Json::Null => None,
            told => {
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
        let rig = &mut out.rig;
        let first = rig.points.len();
        let first_triangle = out.triangles.len();
        match &normals {
            Some(normals) => {
                rig.points.extend(&points);
                rig.normals.extend(normals);
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
        });
        let color = material.map(|material| {
            let told = numbers(&material["pbrMetallicRoughness"]["baseColorFactor"])
                .filter(|told| told.len() >= 3)
                .unwrap_or_else(|| vec![1.0; 3]);
            [told[0] as f32, told[1] as f32, told[2] as f32]
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
        for skin in self.json["skins"].as_array().into_iter().flatten() {
            let joints: Vec<usize> = skin["joints"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|joint| joint.as_u64().map(|joint| joint as usize))
                .collect();
            let inverse = match &skin["inverseBindMatrices"] {
                Json::Null => vec![IDENTITY; joints.len()],
                told => self
                    .numbers(told, 16)?
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .map(|m| [0, 1, 2].map(|row| [0, 1, 2, 3].map(|column| m[column * 4 + row])))
                    .collect(),
            };
            if inverse.len() != joints.len() {
                return Err("a skin's bones and where they were bound do not match".into());
            }
            skins.push(Skin { joints, inverse });
        }
        Ok(skins)
    }

    /// The file's animations: what each says of a node's place over time. What one says of a
    /// mesh's morph targets is not read.
    fn animations(&self, nodes: usize) -> Result<Vec<Animation>, String> {
        let mut animations = Vec::new();
        for animation in self.json["animations"].as_array().into_iter().flatten() {
            let mut channels = Vec::new();
            for channel in animation["channels"].as_array().into_iter().flatten() {
                let (what, across) = match channel["target"]["path"].as_str() {
                    Some("translation") => (What::Moved, 3),
                    Some("rotation") => (What::Turned, 4),
                    Some("scale") => (What::Sized, 3),
                    _ => continue,
                };
                let Some(node) = channel["target"]["node"]
                    .as_u64()
                    .map(|node| node as usize)
                    .filter(|node| *node < nodes)
                else {
                    continue;
                };
                let sampler = channel["sampler"]
                    .as_u64()
                    .and_then(|sampler| animation["samplers"].get(sampler as usize))
                    .ok_or("an animation's sampler is missing")?;
                let how = match sampler["interpolation"].as_str() {
                    Some("STEP") => How::Step,
                    Some("CUBICSPLINE") => How::Cubic,
                    _ => How::Linear,
                };
                let times = self.numbers(&sampler["input"], 1)?;
                let values = self.numbers(&sampler["output"], across)?;
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
                .fold(0.0f64, |length, last| length.max(*last));
            animations.push(Animation {
                name: animation["name"].as_str().unwrap_or("").to_owned(),
                length,
                channels,
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
    for (index, buffer) in json["buffers"].as_array().into_iter().flatten().enumerate() {
        let bytes = match buffer["uri"].as_str() {
            Some(uri) => file.beside(uri).map_err(|why| format!("a buffer: {why}"))?,
            None if index == 0 => binary.ok_or("its first buffer is nowhere")?.to_vec(),
            None => return Err("a buffer is nowhere".into()),
        };
        file.buffers.push(bytes);
    }
    let told: &[Json] = json["nodes"].as_array().map_or(&[], Vec::as_slice);
    let mut nodes: Vec<Node> = told
        .iter()
        .map(|node| Node {
            parent: None,
            matrix: matrix_of(node),
            told: Told::of(node),
        })
        .collect();
    let under = |node: &Json| -> Vec<usize> {
        node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|child| child.as_u64().map(|child| child as usize))
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
    let scene = json["scene"].as_u64().unwrap_or(0) as usize;
    let mut shown: Vec<usize> = match json["scenes"]
        .get(scene)
        .and_then(|scene| scene["nodes"].as_array())
    {
        Some(roots) => roots
            .iter()
            .filter_map(|root| root.as_u64().map(|root| root as usize))
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
        rig: Rig {
            nodes,
            order,
            points: Vec::new(),
            normals: Vec::new(),
            bones: Vec::new(),
            runs: Vec::new(),
            skins: file.skins()?,
        },
    };
    for index in shown {
        let node = &told[index];
        let Some(mesh) = node["mesh"]
            .as_u64()
            .and_then(|mesh| json["meshes"].get(mesh as usize))
        else {
            continue;
        };
        let skin = node["skin"]
            .as_u64()
            .map(|skin| skin as usize)
            .filter(|skin| *skin < out.rig.skins.len());
        for primitive in mesh["primitives"].as_array().into_iter().flatten() {
            file.primitive(primitive, index, skin, &mut out)?;
        }
    }
    if out.triangles.is_empty() {
        return Err("it has no triangles".into());
    }
    (out.points, out.normals) = out.posed(None);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_is_placed_by_its_matrix_or_by_what_it_is_moved_turned_and_sized_by() {
        // Moved by 1, 2, 3 and twice the size, either way it is said.
        let told = serde_json::json!({"translation": [1, 2, 3], "scale": [2, 2, 2]});
        let matrix =
            serde_json::json!({"matrix": [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 1, 2, 3, 1]});
        assert_eq!(local(&told), local(&matrix));
        assert_eq!(local(&told)[1], [0.0, 2.0, 0.0, 2.0]);
        // A quarter turn about z takes x to y.
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let turned = local(&serde_json::json!({"rotation": [0, 0, half, half]}));
        assert!((turned[0][0]).abs() < 1e-12 && (turned[1][0] - 1.0).abs() < 1e-12);
        // One node under another is placed by both, the outer last.
        let both = then(&local(&told), &turned);
        assert!((both[1][0] - 2.0).abs() < 1e-12 && both[0][3] == 1.0);
    }

    #[test]
    fn a_value_between_two_keys_is_as_the_animation_says_it_goes() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let channel = |what: What, how: How, across: usize, values: &[f64]| Channel {
            node: 0,
            what,
            how,
            across,
            times: vec![1.0, 3.0],
            values: values.to_vec(),
        };
        // Evenly, held until the next, and by a curve that leaves and arrives at no speed.
        let moved = [0.0, 0.0, 0.0, 4.0, 0.0, 0.0];
        assert_eq!(channel(What::Moved, How::Linear, 3, &moved).at(1.5)[0], 1.0);
        assert_eq!(channel(What::Moved, How::Step, 3, &moved).at(2.9)[0], 0.0);
        let mut curved = vec![0.0; 18];
        curved[12] = 4.0;
        let curve = channel(What::Moved, How::Cubic, 3, &curved);
        assert_eq!(curve.at(2.0)[0], 2.0);
        assert!(curve.at(1.5)[0] < 1.0 && curve.at(2.5)[0] > 3.0);
        // Before its first key it is the first, and after its last the last.
        assert_eq!(curve.at(0.0)[0], 0.0);
        assert_eq!(curve.at(9.0)[0], 4.0);
        // A turn goes the short way round at an even speed: from none to a quarter turn about z
        // written the long way, an eighth is halfway.
        let turned = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -half, -half];
        let eighth = channel(What::Turned, How::Linear, 4, &turned).at(2.0);
        let (sin, cos) = (std::f64::consts::PI / 8.0).sin_cos();
        assert!((eighth[2] - sin).abs() < 1e-12 && (eighth[3] - cos).abs() < 1e-12);
    }

    #[test]
    fn base64_is_read() {
        assert_eq!(decoded("AAECAwQ=").unwrap(), [0, 1, 2, 3, 4]);
        assert_eq!(decoded("/+7d").unwrap(), [0xff, 0xee, 0xdd]);
        assert!(decoded("a b").is_none());
    }
}
