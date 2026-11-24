//! What the picture's overlays are drawn from: where a clip's picture lands on
//! the canvas (kerf-compose's `placed`), its masks' outlines, and the curve of
//! its motion path (kerf_model's `MotionPath`). Pure functions of the
//! snapshot's timeline as the bridge sends it, so that they can be held to
//! the compositor's own numbers.

use editor_state::model::{Clip, Timeline, TrackKind};
use kerf_bridge::event::{Finding, ViewMask};
use serde_json::{json, Map, Value};

/// A clip's properties by name, as the timeline says them: each a value, or its
/// keyframes.
pub type Props = Map<String, Value>;

/// The text `name` of a clip's `props` says, when it says one.
fn text_of<'a>(props: &'a Props, name: &str) -> Option<&'a str> {
    props.get(name).and_then(Value::as_str)
}

/// Whether `name` of a clip's `props` is on.
fn is_on(props: &Props, name: &str) -> bool {
    props.get(name).and_then(Value::as_bool) == Some(true)
}

/// A point: across, then down.
pub type Pt = (f64, f64);

/// A map of the plane that keeps lines straight: `(x, y)` goes to
/// `(a x + c y + e, b x + d y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f64, y: f64) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    /// Turned `degrees` clockwise, y running down.
    pub fn rotate(degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// This map, and then `next`.
    pub fn then(self, next: Self) -> Self {
        Self {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    pub fn apply(self, (x, y): Pt) -> Pt {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The map back; none when it folds the plane onto a line.
    pub fn inverse(self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Self {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }
}

/// The nearest a camera sees, in frame heights (kerf-compose's `NEAR`):
/// what is nearer than this along its way, or behind it, is out of sight.
pub const NEAREST: f64 = 0.001;

/// What `w` is held to at least as a point is mapped: a point on or behind
/// the camera still maps somewhere.
const NEAR_CLIP: f64 = 1e-6;

/// A map of the plane as a camera may see it, in perspective, a row vector
/// times the matrix: `(x, y)` goes to `(m11 x + m21 y + m31,
/// m12 x + m22 y + m32)` over `w = m13 x + m23 y + m33`, `m[r][c]` being
/// `m(r+1)(c+1)`. For a placement in depth `w` is how far along the
/// camera's way the point is, in frame heights; for a flat one it is 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projective {
    pub m: [[f64; 3]; 3],
}

impl From<Affine> for Projective {
    fn from(map: Affine) -> Self {
        Self {
            m: [
                [map.a, map.b, 0.0],
                [map.c, map.d, 0.0],
                [map.e, map.f, 1.0],
            ],
        }
    }
}

impl Projective {
    /// What a placement is when there is nothing to place on: no map back.
    pub const UNSEEN: Self = Self { m: [[0.0; 3]; 3] };

    /// This map, and then `next`.
    pub fn then(self, next: impl Into<Self>) -> Self {
        let next = next.into();
        let mut m = [[0.0; 3]; 3];
        for (row, out) in m.iter_mut().enumerate() {
            for (column, cell) in out.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.m[row][k] * next.m[k][column]).sum();
            }
        }
        Self { m }
    }

    /// How far along the camera's way `(x, y)` is: its `w`.
    fn depth(self, (x, y): Pt) -> f64 {
        self.m[0][2] * x + self.m[1][2] * y + self.m[2][2]
    }

    /// Where `(x, y)` goes. One on or behind the camera goes somewhere
    /// far off; [`Self::in_sight`] says which are seen.
    pub fn apply(self, (x, y): Pt) -> Pt {
        let across = self.m[0][0] * x + self.m[1][0] * y + self.m[2][0];
        let down = self.m[0][1] * x + self.m[1][1] * y + self.m[2][1];
        let w = self.depth((x, y));
        if w == 1.0 {
            return (across, down);
        }
        let w = w.max(NEAR_CLIP);
        (across / w, down / w)
    }

    /// Whether `(x, y)` is in front of the camera, where it can be seen.
    pub fn in_sight(self, at: Pt) -> bool {
        self.depth(at) >= NEAREST
    }

    /// The map back; none when it folds the plane onto a line, or
    /// there is nothing to place on ([`Self::UNSEEN`]).
    pub fn inverse(self) -> Option<Self> {
        let m = self.m;
        let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let adjugate = [
            [
                cofactor(1, 2, 1, 2),
                -cofactor(0, 2, 1, 2),
                cofactor(0, 1, 1, 2),
            ],
            [
                -cofactor(1, 2, 0, 2),
                cofactor(0, 2, 0, 2),
                -cofactor(0, 1, 0, 2),
            ],
            [
                cofactor(1, 2, 0, 1),
                -cofactor(0, 2, 0, 1),
                cofactor(0, 1, 0, 1),
            ],
        ];
        let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        Some(Self {
            m: adjugate.map(|row| row.map(|cell| cell / det)),
        })
    }

    /// Whether it places anything: it has a map back.
    pub fn shows(self) -> bool {
        self.inverse().is_some()
    }

    /// The point that is seen at `seen`: none past the horizon, where
    /// nothing of the plane is, and none on or behind the camera.
    pub fn point_in_sight(self, seen: Pt) -> Option<Pt> {
        let back = self.inverse()?;
        if back.depth(seen) <= NEAR_CLIP {
            return None;
        }
        let point = back.apply(seen);
        self.in_sight(point).then_some(point)
    }
}

/// How far along a span `interp` is, `t` of the way across
/// (kerf_model's `Interp::curve`).
pub fn curve(interp: &str, t: f64) -> f64 {
    match interp {
        "linear" => t,
        "hold" => 0.0,
        "ease-in" => t * t,
        "ease-out" => 1.0 - (1.0 - t) * (1.0 - t),
        "ease" => t * t * (3.0 - 2.0 * t),
        other => bezier(other, t).unwrap_or(t),
    }
}

/// A curve written as its two handles, `x1,y1,x2,y2`.
fn bezier(interp: &str, t: f64) -> Option<f64> {
    let numbers: Vec<f64> = interp
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [x1, y1, x2, y2] = numbers[..] else {
        return None;
    };
    let cubic = |a: f64, b: f64, s: f64| {
        let r = 1.0 - s;
        3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
    };
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..40 {
        let middle = (low + high) / 2.0;
        if cubic(x1, x2, middle) < t {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(cubic(y1, y2, (low + high) / 2.0))
}

/// The number `name` of a clip's `props` at `clip_time` seconds into
/// it: a value, or its keyframes between; `fallback` when it has none.
pub fn property_at(props: &Props, name: &str, clip_time: f64, fallback: f64) -> f64 {
    match props.get(name) {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(fallback),
        Some(Value::Array(keys)) if !keys.is_empty() => {
            let at = |index: usize| keys[index]["at"].as_f64().unwrap_or(0.0);
            let value = |index: usize| keys[index]["value"].as_f64().unwrap_or(fallback);
            if clip_time <= at(0) {
                return value(0);
            }
            for index in 1..keys.len() {
                if clip_time > at(index) {
                    continue;
                }
                let span = at(index) - at(index - 1);
                let t = if span > 0.0 {
                    (clip_time - at(index - 1)) / span
                } else {
                    1.0
                };
                let interp = keys[index - 1]["interp"].as_str().unwrap_or("linear");
                return value(index - 1) + (value(index) - value(index - 1)) * curve(interp, t);
            }
            value(keys.len() - 1)
        }
        _ => fallback,
    }
}

/// A clip's geometry at a moment: the compositor's `GEOMETRY`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub crop: [f64; 4],
    pub scale: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
    pub anchor: Pt,
    pub at: Pt,
}

pub fn geometry_at(props: &Props, clip_time: f64) -> Geometry {
    let prop = |name: &str, fallback: f64| property_at(props, name, clip_time, fallback);
    Geometry {
        crop: [
            prop("crop_l", 0.0),
            prop("crop_r", 0.0),
            prop("crop_t", 0.0),
            prop("crop_b", 0.0),
        ],
        scale: prop("scale", 1.0),
        scale_x: prop("scale_x", 1.0),
        scale_y: prop("scale_y", 1.0),
        rotation: prop("rotation", 0.0),
        anchor: (prop("anchor_x", 0.0), prop("anchor_y", 0.0)),
        at: (prop("x", 0.0), prop("y", 0.0)),
    }
}

/// The same with the clip's motion path added in: the path adds to
/// where it would be, and `orient` to how it is turned.
pub fn moved_geometry_at(props: &Props, canvas: Pt, clip_time: f64) -> Geometry {
    let mut geometry = geometry_at(props, clip_time);
    if let Some(points) = text_of(props, "path").and_then(parse_path) {
        let on =
            MotionPath::new(points, aspect(canvas)).at(property_at(props, "along", clip_time, 0.0));
        geometry.at.0 += on.x;
        geometry.at.1 += on.y;
        if is_on(props, "orient") {
            geometry.rotation += on.heading;
        }
    }
    geometry
}

fn aspect((width, height): Pt) -> f64 {
    width.max(1.0) / height.max(1.0)
}

/// What a picture of the cropped size fills on the canvas, before the
/// clip's own scale.
fn filled((width, height): Pt, fit: bool, (cw, ch): Pt) -> Pt {
    if !fit {
        return (width, height);
    }
    (
        (ch * width / height).round().min(cw).max(1.0),
        (cw * height / width).round().min(ch).max(1.0),
    )
}

/// Where a source of `picture` pixels lands on the canvas, in canvas
/// pixels (kerf-compose's `placed`).
pub fn placed(g: &Geometry, picture: Pt, fit: bool, canvas: Pt) -> Affine {
    let (sw, sh) = (picture.0.max(1.0), picture.1.max(1.0));
    let [l, r, t, b] = g.crop;
    let width = sw * (1.0 - l - r).max(1e-6);
    let height = sh * (1.0 - t - b).max(1e-6);
    let (fw, fh) = filled((width, height), fit, canvas);
    let (ax, ay) = g.anchor;
    Affine::translate(
        -(sw * l + width * (0.5 + ax)),
        -(sh * t + height * (0.5 + ay)),
    )
    .then(Affine::scale(
        fw / width * g.scale * g.scale_x,
        fh / height * g.scale * g.scale_y,
    ))
    .then(Affine::rotate(g.rotation))
    .then(Affine::translate(
        canvas.0 / 2.0 + g.at.0 * canvas.0 + ax * fw,
        canvas.1 / 2.0 + g.at.1 * canvas.1 + ay * fh,
    ))
}

/// How a parent has moved its children at its clip time: from where it
/// would sit with no position, scale or rotation of its own, to where it
/// is (kerf-compose's `parent_motion`).
pub fn parent_motion(props: &Props, picture: Pt, fit: bool, canvas: Pt, clip_time: f64) -> Affine {
    let now = moved_geometry_at(props, canvas, clip_time);
    let rest = Geometry {
        crop: now.crop,
        scale: 1.0,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation: 0.0,
        anchor: now.anchor,
        at: (0.0, 0.0),
    };
    match placed(&rest, picture, fit, canvas).inverse() {
        Some(back) => back.then(placed(&now, picture, fit, canvas)),
        None => Affine::IDENTITY,
    }
}

/// The canvas a timeline is drawn on, in pixels: its sequence's size,
/// else its largest media's, else 1920 × 1080.
pub fn canvas_of(timeline: &Timeline) -> Pt {
    let size = |width: Option<u32>, height: Option<u32>| {
        let (width, height) = (f64::from(width?), f64::from(height?));
        (width > 0.0 && height > 0.0).then_some((width.max(2.0), height.max(2.0)))
    };
    if let Some(size) = size(timeline.width, timeline.height) {
        return size;
    }
    timeline
        .sources
        .iter()
        .filter_map(|source| size(source.width, source.height))
        .fold(None, |best: Option<Pt>, size| match best {
            Some(best) if best.0 * best.1 >= size.0 * size.1 => Some(best),
            _ => Some(size),
        })
        .unwrap_or((1920.0, 1080.0))
}

/// The clip `id` of `timeline`, and the kind of track it is on.
pub fn clip_in<'a>(timeline: &'a Timeline, id: &str) -> Option<(&'a Clip, TrackKind)> {
    timeline.clip(id).map(|(track, clip)| (clip, track.kind))
}

/// The size of the picture a clip on a `track` track draws, and whether
/// it is fitted to the canvas; none for what draws no picture.
pub fn picture_of(
    timeline: &Timeline,
    clip: &Clip,
    track: TrackKind,
    canvas: Pt,
) -> Option<(Pt, bool)> {
    let kind = clip.kind.as_str();
    if track != TrackKind::Video || kind == "caption" {
        return None;
    }
    if kind != "media" {
        // Generators fill the canvas; a nested sequence is fitted to it.
        return Some((canvas, kind == "sequence"));
    }
    let source = timeline.source(clip.source_id.as_deref()?)?;
    let (width, height) = (f64::from(source.width?), f64::from(source.height?));
    (width > 0.0 && height > 0.0).then_some(((width, height), true))
}

/// Where a flat clip's parents have moved it, in canvas pixels, at
/// `position` seconds into the program.
pub fn parents_motion(timeline: &Timeline, clip: &Clip, canvas: Pt, position: f64) -> Affine {
    let mut moved = Affine::IDENTITY;
    let mut seen: Vec<String> = Vec::new();
    let mut parent = clip.parent.clone();
    while let Some(id) = parent.filter(|id| !id.is_empty() && !seen.contains(id)) {
        let Some((rig, track)) = clip_in(timeline, &id) else {
            break;
        };
        let Some((picture, fit)) = picture_of(timeline, rig, track, canvas) else {
            break;
        };
        moved = moved.then(parent_motion(
            &rig.props,
            picture,
            fit,
            canvas,
            position - rig.timeline_start,
        ));
        seen.push(id);
        parent = rig.parent.clone();
    }
    moved
}

/// Where a picture clip is on the canvas at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Fractions of the clip's picture to canvas pixels: where its
    /// masks are. In depth, as the camera sees it; [`Projective::UNSEEN`]
    /// when it cannot be seen (turned edge on, behind the camera).
    pub picture: Projective,
    /// A point of its motion path to fractions of the canvas;
    /// [`Projective::UNSEEN`] for a path in depth no one map shows
    /// ([`sampled`] shows that one).
    pub path: Projective,
    pub canvas: Pt,
    /// Seconds into the clip.
    pub clip_time: f64,
}

impl Placement {
    /// Whether its masks can be drawn, chosen and dragged on the
    /// picture: its picture can be seen, not turned edge on or behind
    /// the camera (an invertible placement).
    pub fn masks_seen(&self) -> bool {
        self.picture.shows()
    }

    /// Whether its motion path `points` can be seen through its map:
    /// the map shows, and some of its curve is in front of the camera.
    /// A path no one map shows is seen by [`sampled`] instead.
    pub fn path_seen(&self, points: &[Pt]) -> bool {
        let (width, height) = self.canvas;
        self.path.shows()
            && MotionPath::new(points.to_vec(), width.max(1.0) / height.max(1.0))
                .samples()
                .iter()
                .any(|at| self.path.in_sight(*at))
    }
}

/// What a clip is placed with at a moment: its picture's size, whether
/// it is fitted to the canvas, the canvas, seconds into it, how its
/// parents have moved it on the canvas, and where it starts.
#[derive(Clone, Copy, Debug)]
struct Scene {
    picture: Pt,
    fit: bool,
    canvas: Pt,
    clip_time: f64,
    parents: Affine,
    start: f64,
}

impl Scene {
    /// Seconds into the program.
    fn at(&self) -> f64 {
        self.start + self.clip_time
    }
}

/// The clip `id` of `timeline` and what it is placed with at `position`
/// seconds into the program; none for a clip that draws no picture.
fn scene_of<'a>(timeline: &'a Timeline, id: &str, position: f64) -> Option<(&'a Clip, Scene)> {
    let (clip, track) = clip_in(timeline, id)?;
    let canvas = canvas_of(timeline);
    let (picture, fit) = picture_of(timeline, clip, track, canvas)?;
    let start = clip.timeline_start;
    let clip_time = (position - start).clamp(0.0, clip.duration.max(0.0));
    let parents = parents_motion(timeline, clip, canvas, start + clip_time);
    Some((
        clip,
        Scene {
            picture,
            fit,
            canvas,
            clip_time,
            parents,
            start,
        },
    ))
}

/// Where the clip `id` of `timeline` is at `position` seconds into the
/// program, flat or in depth as the camera sees it; none for a clip
/// that draws no picture, and one of which nothing can be seen.
pub fn placement(timeline: &Timeline, id: &str, position: f64) -> Option<Placement> {
    let (clip, scene) = scene_of(timeline, id, position)?;
    let picture = picture_placement(timeline, clip, &scene);
    let path = path_placement(timeline, clip, &scene);
    if !picture.shows() && !path.shows() && sampled(timeline, id, position, None).is_none() {
        return None;
    }
    Some(Placement {
        picture,
        path,
        canvas: scene.canvas,
        clip_time: scene.clip_time,
    })
}

// ------------------------------------------------------------- depth

/// The field of view at which a clip at z = 0 is its flat self, in
/// degrees (kerf-compose's `DEFAULT_FOV`).
const RESTING_FOV: f64 = 40.0;

/// A point or a way in the compositor's world: x right, y down and z
/// away from the viewer, in canvas pixels, 0, 0, 0 the canvas's middle.
type World = [f64; 3];

fn add(a: World, b: World) -> World {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn minus(a: World, b: World) -> World {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn times(a: World, by: f64) -> World {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: World, b: World) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: World, b: World) -> World {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Turned about x by `pitch` and then about y by `yaw`, in degrees.
fn turned(a: World, pitch: f64, yaw: f64) -> World {
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (y, z) = (a[1] * cp - a[2] * sp, a[1] * sp + a[2] * cp);
    [a[0] * cy + z * sy, y, -a[0] * sy + z * cy]
}

/// Whether a clip is placed in depth (`three_d`).
fn in_depth(clip: &Clip) -> bool {
    is_on(&clip.props, "three_d")
}

/// Whether a parent's place in depth is its children's (kerf-compose's
/// `Rig::in_depth`): a null's, and that of a clip in depth, which a mesh
/// is without saying so.
fn leads_in_depth(clip: &Clip) -> bool {
    matches!(clip.fx.as_deref(), Some("null" | "mesh")) || in_depth(clip)
}

/// A clip's own numbers in depth at `clip_time`: how far back, in canvas
/// pixels, and how it is turned about x and about y, in degrees.
struct Depth {
    z: f64,
    pitch: f64,
    yaw: f64,
}

fn depth_at(props: &Props, canvas: Pt, clip_time: f64) -> Depth {
    Depth {
        z: property_at(props, "z", clip_time, 0.0) * canvas.1.max(1.0),
        pitch: property_at(props, "rotate_x", clip_time, 0.0),
        yaw: property_at(props, "rotate_y", clip_time, 0.0),
    }
}

/// How its parents have taken a clip in depth, as one thing with them
/// (kerf-compose's `Carried`): where the world's x, y and z go, and
/// where its middle goes.
#[derive(Clone, Copy, Debug)]
struct Carried {
    ways: [World; 3],
    shift: World,
}

impl Carried {
    const STILL: Self = Self {
        ways: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        shift: [0.0; 3],
    };

    /// This, and then as a parent takes it: turned about `about` by the
    /// parent's `pitch` and `yaw` (degrees), and moved `back`.
    fn then(self, about: World, pitch: f64, yaw: f64, back: f64) -> Self {
        Self {
            ways: self.ways.map(|way| turned(way, pitch, yaw)),
            shift: add(
                add(about, turned(minus(self.shift, about), pitch, yaw)),
                [0.0, 0.0, back],
            ),
        }
    }

    fn way(&self, of: World) -> World {
        add(
            add(times(self.ways[0], of[0]), times(self.ways[1], of[1])),
            times(self.ways[2], of[2]),
        )
    }

    fn point(&self, at: World) -> World {
        add(self.way(at), self.shift)
    }
}

/// Fractions of a clip's picture to canvas pixels, flat: the
/// compositor's geometry with its parents'.
fn flat_placement(clip: &Clip, scene: &Scene) -> Affine {
    let (picture, canvas) = (scene.picture, scene.canvas);
    Affine::scale(picture.0.max(1.0), picture.1.max(1.0))
        .then(placed(
            &moved_geometry_at(&clip.props, canvas, scene.clip_time),
            picture,
            scene.fit,
            canvas,
        ))
        .then(scene.parents)
}

/// How the parents of `clip` have taken it in depth at `at` seconds of
/// `timeline`, the one it names first (kerf-compose's `Scene::at`):
/// each somewhere in depth turns it about its own middle, where that
/// is on the canvas, and moves it back.
fn carried_by(timeline: &Timeline, clip: &Clip, canvas: Pt, at: f64) -> Carried {
    let mut carried = Carried::STILL;
    let mut seen: Vec<String> = Vec::new();
    let mut parent = clip.parent.clone();
    while let Some(id) = parent.filter(|id| !id.is_empty() && !seen.contains(id)) {
        seen.push(id.clone());
        let Some((rig, track)) = clip_in(timeline, &id) else {
            break;
        };
        let Some((picture, fit)) = picture_of(timeline, rig, track, canvas) else {
            break;
        };
        parent = rig.parent.clone();
        if !leads_in_depth(rig) {
            continue;
        }
        let start = rig.timeline_start;
        let depth = depth_at(&rig.props, canvas, at - start);
        if depth.z == 0.0 && depth.pitch == 0.0 && depth.yaw == 0.0 {
            continue;
        }
        let scene = Scene {
            picture,
            fit,
            canvas,
            clip_time: at - start,
            parents: parents_motion(timeline, rig, canvas, at),
            start,
        };
        let middle = flat_placement(rig, &scene).apply((0.5, 0.5));
        let about = [middle.0 - canvas.0 / 2.0, middle.1 - canvas.1 / 2.0, 0.0];
        carried = carried.then(about, depth.pitch, depth.yaw, depth.z);
    }
    carried
}

/// How a sequence is seen at a moment (kerf-compose's `Scene::at`).
struct Camera {
    eye: World,
    right: World,
    down: World,
    forward: World,
    focal: f64,
    width: f64,
    height: f64,
}

impl Camera {
    /// Where `(a, b)` of the plane `origin + a across + b below` lands,
    /// in canvas pixels; its `w` is how far along the camera's way that
    /// is, in frame heights. Edge on, it is a line at most a pixel wide,
    /// which the compositor's rays miss: unseen.
    fn sees(&self, origin: World, across: World, below: World) -> Projective {
        let from = minus(origin, self.eye);
        let perpendicular = cross(across, below);
        let area = dot(perpendicular, perpendicular).sqrt();
        if area <= 0.0 || dot(from, perpendicular).abs() / area < NEAREST * self.height {
            return Projective::UNSEEN;
        }
        let sideways = add(
            times(self.right, self.focal),
            times(self.forward, self.width / 2.0),
        );
        let downwards = add(
            times(self.down, self.focal),
            times(self.forward, self.height / 2.0),
        );
        let row = |of: World| {
            [
                dot(of, sideways) / self.height,
                dot(of, downwards) / self.height,
                dot(of, self.forward) / self.height,
            ]
        };
        Projective {
            m: [row(across), row(below), row(from)],
        }
    }
}

/// `timeline`'s topmost camera showing at `at` seconds, or the resting
/// one, which sees the plane z = 0 as the flat picture.
fn camera_at(timeline: &Timeline, canvas: Pt, at: f64) -> Camera {
    // Times come as seconds in doubles, a hair off the frames they name.
    const SAME_MOMENT: f64 = 1e-9;
    let mut shown: Option<&Clip> = None;
    for track in timeline.tracks.iter().filter(|track| track.enabled) {
        for clip in &track.clips {
            if clip.fx.as_deref() != Some("camera") || !clip.enabled {
                continue;
            }
            let (start, duration) = (clip.timeline_start, clip.duration);
            if start - SAME_MOMENT <= at && at < start + duration - SAME_MOMENT {
                shown = Some(clip);
            }
        }
    }
    // With no camera showing, the resting one: every number at what it is when unsaid.
    let resting = Props::new();
    let props = shown.map_or(&resting, |clip| &clip.props);
    let clip_time = at - shown.map_or(0.0, |clip| clip.timeline_start);
    let read = |name: &str, otherwise: f64| property_at(props, name, clip_time, otherwise);
    let (width, height) = (canvas.0.max(1.0), canvas.1.max(1.0));
    let distance = |fov: f64| height / 2.0 / (fov.clamp(1.0, 170.0).to_radians() / 2.0).tan();
    let mut camera = Camera {
        eye: [
            read("x", 0.0) * width,
            read("y", 0.0) * height,
            read("z", 0.0) * height - distance(RESTING_FOV),
        ],
        right: [1.0, 0.0, 0.0],
        down: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
        focal: distance(read("fov", RESTING_FOV)),
        width,
        height,
    };
    let (pitch, yaw) = (read("tilt", 0.0), read("pan", 0.0));
    let roll = read("roll", 0.0).to_radians();
    camera.forward = turned([0.0, 0.0, 1.0], pitch, yaw);
    let level_right = turned([1.0, 0.0, 0.0], pitch, yaw);
    let level_down = cross(camera.forward, level_right);
    camera.right = add(
        times(level_right, roll.cos()),
        times(level_down, roll.sin()),
    );
    camera.down = minus(
        times(level_down, roll.cos()),
        times(level_right, roll.sin()),
    );
    let Some(camera_clip) =
        shown.filter(|clip| clip.parent.as_deref().is_some_and(|id| !id.is_empty()))
    else {
        return camera;
    };
    // A camera with a parent is where the parent takes it, looking as
    // the parent turns it (kerf-compose's `Rigged`): across the canvas
    // as a clip is moved, and then in depth.
    let moved = parents_motion(timeline, camera_clip, canvas, at);
    let carried = carried_by(timeline, camera_clip, canvas, at);
    let on = moved.apply((width / 2.0 + camera.eye[0], height / 2.0 + camera.eye[1]));
    camera.eye = carried.point([on.0 - width / 2.0, on.1 - height / 2.0, camera.eye[2]]);
    // By as much as its parents turn a clip on the canvas, however they size it.
    let (sin, cos) = moved.b.atan2(moved.a).sin_cos();
    let way =
        |of: World| carried.way([of[0] * cos - of[1] * sin, of[0] * sin + of[1] * cos, of[2]]);
    camera.right = way(camera.right);
    camera.down = way(camera.down);
    camera.forward = way(camera.forward);
    camera
}

/// Fractions of `clip`'s picture to canvas pixels: flat, or in depth
/// as the camera sees it (kerf-compose's `Plane::of`: lifted to its `z`
/// and turned about its middle, then where its parents in depth take
/// it). [`Projective::UNSEEN`] when none of its corners is in sight.
fn picture_placement(timeline: &Timeline, clip: &Clip, scene: &Scene) -> Projective {
    let flat = flat_placement(clip, scene);
    if !in_depth(clip) {
        return flat.into();
    }
    let canvas = scene.canvas;
    let depth = depth_at(&clip.props, canvas, scene.clip_time);
    let middle = flat.apply((0.5, 0.5));
    let across = turned([flat.a, flat.b, 0.0], depth.pitch, depth.yaw);
    let below = turned([flat.c, flat.d, 0.0], depth.pitch, depth.yaw);
    let origin = minus(
        minus(
            [
                middle.0 - canvas.0 / 2.0,
                middle.1 - canvas.1 / 2.0,
                depth.z,
            ],
            times(across, 0.5),
        ),
        times(below, 0.5),
    );
    let carried = carried_by(timeline, clip, canvas, scene.at());
    let seen = camera_at(timeline, canvas, scene.at()).sees(
        carried.point(origin),
        carried.way(across),
        carried.way(below),
    );
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    if corners.iter().any(|corner| seen.in_sight(*corner)) {
        seen
    } else {
        Projective::UNSEEN
    }
}

/// Where the clip's anchor is in its picture, in fractions of it: what
/// its motion path takes along.
fn anchor_of(g: &Geometry) -> Pt {
    let [l, r, t, b] = g.crop;
    (
        l + (1.0 - l - r) * (0.5 + g.anchor.0),
        t + (1.0 - t - b) * (0.5 + g.anchor.1),
    )
}

/// A point of `clip`'s motion path to fractions of the canvas. In depth
/// the path is on the plane at the clip's `z`, where the camera sees it;
/// [`Projective::UNSEEN`] for a clip that faces along its path
/// (`orient`) from an anchor off its middle, which no one map shows.
fn path_placement(timeline: &Timeline, clip: &Clip, scene: &Scene) -> Projective {
    let (canvas, picture) = (scene.canvas, scene.picture);
    let g = geometry_at(&clip.props, scene.clip_time);
    let [l, r, t, b] = g.crop;
    let width = picture.0.max(1.0) * (1.0 - l - r).max(1e-6);
    let height = picture.1.max(1.0) * (1.0 - t - b).max(1e-6);
    let (fw, fh) = filled((width, height), scene.fit, canvas);
    let (cw, ch) = (canvas.0.max(1.0), canvas.1.max(1.0));
    let origin = (
        0.5 + g.at.0 + g.anchor.0 * fw / cw,
        0.5 + g.at.1 + g.anchor.1 * fh / ch,
    );
    let on_canvas = Affine::translate(origin.0, origin.1)
        .then(Affine::scale(cw, ch))
        .then(scene.parents);
    let to_shares = Affine::scale(1.0 / cw, 1.0 / ch);
    if !in_depth(clip) {
        return on_canvas.then(to_shares).into();
    }
    // The path says where the anchor goes, and the clip turns about its middle.
    let flat = flat_placement(clip, scene);
    let (middle, anchor) = (flat.apply((0.5, 0.5)), flat.apply(anchor_of(&g)));
    let to_middle = [middle.0 - anchor.0, middle.1 - anchor.1, 0.0];
    let depth = depth_at(&clip.props, canvas, scene.clip_time);
    let swung = add(
        turned(times(to_middle, -1.0), depth.pitch, depth.yaw),
        to_middle,
    );
    if dot(swung, swung) > 1e-12 && is_on(&clip.props, "orient") {
        return Projective::UNSEEN;
    }
    let plane = add([-cw / 2.0, -ch / 2.0, depth.z], swung);
    // And where the clip's parents in depth take it, the path's plane with it.
    let carried = carried_by(timeline, clip, canvas, scene.at());
    let seen = camera_at(timeline, canvas, scene.at()).sees(
        carried.point(plane),
        carried.way([1.0, 0.0, 0.0]),
        carried.way([0.0, 1.0, 0.0]),
    );
    Projective::from(on_canvas).then(seen).then(to_shares)
}

// ------------------------------------------------------- sampled paths

/// How many steps a path in depth that no one map shows is sampled in.
pub const SAMPLED_STEPS: usize = 96;

/// A path in depth that no one map shows (a clip facing along it from
/// an anchor off its middle), sampled where the clip's anchor goes, in
/// fractions of the canvas: none where that is out of sight.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampled {
    /// The curve, [`SAMPLED_STEPS`] steps along it.
    pub places: Vec<Option<Pt>>,
    /// Where the clip is on it now.
    pub now: Option<Pt>,
    /// Where its anchor is at each of the path's points.
    pub handles: Vec<Option<Pt>>,
}

/// `clip` with its path's points `points` (as `path=` would say them
/// exactly) and, if given, `along`.
fn with_path(clip: &Clip, points: &[Pt], along: Option<f64>) -> Clip {
    let mut moved = clip.clone();
    let text = points
        .iter()
        .map(|(x, y)| format!("{x},{y}"))
        .collect::<Vec<_>>()
        .join(" ");
    moved.props.insert("path".into(), json!(text));
    if let Some(along) = along {
        moved.props.insert("along".into(), json!(along));
    }
    moved
}

/// Where `placement` puts the clip's `anchor`, in fractions of the canvas;
/// none when that cannot be seen.
fn share_of(placement: Projective, anchor: Pt, canvas: Pt) -> Option<Pt> {
    if !placement.shows() || !placement.in_sight(anchor) {
        return None;
    }
    let on = placement.apply(anchor);
    Some((on.0 / canvas.0.max(1.0), on.1 / canvas.1.max(1.0)))
}

/// Where the anchor of `clip`, with its path's points `points`, is when
/// at its point `index`.
fn anchor_at_point(
    timeline: &Timeline,
    clip: &Clip,
    scene: &Scene,
    points: &[Pt],
    index: usize,
) -> Option<Pt> {
    let along = MotionPath::new(points.to_vec(), aspect(scene.canvas)).along_of(index);
    let moved = with_path(clip, points, Some(along));
    let anchor = anchor_of(&geometry_at(&clip.props, scene.clip_time));
    share_of(
        picture_placement(timeline, &moved, scene),
        anchor,
        scene.canvas,
    )
}

/// The clip `id` and what it is placed with, when its path is one no
/// one map shows: in depth, facing along a path it has, from an anchor
/// off its middle.
fn sampled_scene<'a>(
    timeline: &'a Timeline,
    id: &str,
    position: f64,
) -> Option<(&'a Clip, Scene, Vec<Pt>)> {
    let (clip, scene) = scene_of(timeline, id, position)?;
    let props = &clip.props;
    let points = text_of(props, "path").and_then(parse_path)?;
    let facing = in_depth(clip) && is_on(props, "orient");
    (facing && !path_placement(timeline, clip, &scene).shows()).then_some((clip, scene, points))
}

/// The path of the clip `id` at `position` seconds into the program,
/// sampled, when it is one no one map shows; with its points as
/// `points` say in place of the document's (a point being dragged).
/// None for any other clip, and for one whose path is all out of sight.
pub fn sampled(
    timeline: &Timeline,
    id: &str,
    position: f64,
    points: Option<&[Pt]>,
) -> Option<Sampled> {
    let (clip, scene, written) = sampled_scene(timeline, id, position)?;
    let points = points.map_or(written, <[Pt]>::to_vec);
    let moved = with_path(clip, &points, None);
    let anchor = anchor_of(&geometry_at(&clip.props, scene.clip_time));
    let places: Vec<Option<Pt>> = (0..=SAMPLED_STEPS)
        .map(|step| {
            let there = with_path(clip, &points, Some(step as f64 / SAMPLED_STEPS as f64));
            share_of(
                picture_placement(timeline, &there, &scene),
                anchor,
                scene.canvas,
            )
        })
        .collect();
    if places.iter().all(Option::is_none) {
        return None;
    }
    let along = property_at(&moved.props, "along", scene.clip_time, 0.0);
    let now = places[(along.clamp(0.0, 1.0) * SAMPLED_STEPS as f64).round() as usize];
    let handles = (0..points.len())
        .map(|index| anchor_at_point(timeline, clip, &scene, &points, index))
        .collect();
    Some(Sampled {
        places,
        now,
        handles,
    })
}

/// Where point `index` of the path `points` of the clip `id` must be for
/// its anchor to be seen at `seen` (fractions of the canvas), found by
/// solving from the clip's own placement there; none when no point
/// puts it there, as past a horizon.
pub fn sampled_point_for(
    timeline: &Timeline,
    id: &str,
    position: f64,
    points: &[Pt],
    index: usize,
    seen: Pt,
) -> Option<Pt> {
    // Far closer than the document can say a point, a ten-thousandth of the frame.
    const CLOSE: f64 = 1e-7;
    // How far a point is moved to see what that does, and the most a step may take it.
    const NUDGE: f64 = 1e-4;
    const MOST_STEP: f64 = 0.5;
    const MOST_STEPS: usize = 12;
    let (clip, scene, _) = sampled_scene(timeline, id, position)?;
    if index >= points.len() {
        return None;
    }
    let mut trying = points.to_vec();
    let mut landing = |point: Pt| {
        trying[index] = point;
        anchor_at_point(timeline, clip, &scene, &trying, index)
    };
    let mut point = points[index];
    for _ in 0..MOST_STEPS {
        let on = landing(point)?;
        let across = landing((point.0 + NUDGE, point.1))?;
        let down = landing((point.0, point.1 + NUDGE))?;
        let off = (on.0 - seen.0, on.1 - seen.1);
        if off.0.hypot(off.1) < CLOSE {
            return Some(point);
        }
        let (a, b) = ((across.0 - on.0) / NUDGE, (down.0 - on.0) / NUDGE);
        let (c, d) = ((across.1 - on.1) / NUDGE, (down.1 - on.1) / NUDGE);
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let mut by = ((d * off.0 - b * off.1) / det, (a * off.1 - c * off.0) / det);
        let length = by.0.hypot(by.1);
        if length > MOST_STEP {
            by = (by.0 * MOST_STEP / length, by.1 * MOST_STEP / length);
        }
        point = (point.0 - by.0, point.1 - by.1);
    }
    let on = landing(point)?;
    ((on.0 - seen.0).hypot(on.1 - seen.1) < CLOSE).then_some(point)
}

// ------------------------------------------------------------- masks

/// A mask's outline, in fractions of its clip's picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Outline {
    /// Left, top, right, bottom.
    Rect([f64; 4]),
    /// Centre across and down, then the radius across and down.
    Ellipse([f64; 4]),
    Points {
        points: Vec<Pt>,
        closed: bool,
    },
}

/// Points an ellipse is drawn and hit with.
const ELLIPSE_STEPS: usize = 72;

/// Fractions are kept to four places, under half a pixel of a 4K
/// picture, so the document stays readable.
fn fraction(value: f64) -> f64 {
    let rounded = (value * 10000.0).round() / 10000.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

impl Outline {
    /// A mask's outline as the snapshot carries it.
    pub fn of(mask: &ViewMask) -> Option<Self> {
        let outline = &mask.outline;
        if let Some(rect) = outline.rect {
            return Some(Self::Rect(rect));
        }
        if let Some(ellipse) = outline.ellipse {
            return Some(Self::Ellipse(ellipse));
        }
        Some(Self::Points {
            points: outline
                .points
                .as_ref()?
                .iter()
                .map(|[x, y]| (*x, *y))
                .collect(),
            closed: outline.closed.unwrap_or(true),
        })
    }

    /// A rectangle's corners clockwise from its top left, an ellipse's
    /// left, right, top and bottom, or a path's points.
    pub fn handles(&self) -> Vec<Pt> {
        match self {
            Self::Rect([l, t, r, b]) => vec![(*l, *t), (*r, *t), (*r, *b), (*l, *b)],
            Self::Ellipse([x, y, rx, ry]) => {
                vec![(x - rx, *y), (x + rx, *y), (*x, y - ry), (*x, y + ry)]
            }
            Self::Points { points, .. } => points.clone(),
        }
    }

    /// With its handle `index` dragged to `to`: a rectangle keeps the
    /// opposite corner, an ellipse its centre.
    pub fn with_handle(&self, index: usize, to: Pt) -> Self {
        match self {
            Self::Rect(_) => {
                let handles = self.handles();
                let (ox, oy) = handles[(index + 2) % 4];
                Self::Rect([to.0.min(ox), to.1.min(oy), to.0.max(ox), to.1.max(oy)])
            }
            Self::Ellipse([x, y, rx, ry]) => {
                if index < 2 {
                    Self::Ellipse([*x, *y, (to.0 - x).abs(), *ry])
                } else {
                    Self::Ellipse([*x, *y, *rx, (to.1 - y).abs()])
                }
            }
            Self::Points { points, closed } => {
                let mut points = points.clone();
                if let Some(point) = points.get_mut(index) {
                    *point = to;
                }
                Self::Points {
                    points,
                    closed: *closed,
                }
            }
        }
    }

    pub fn moved_by(&self, (dx, dy): Pt) -> Self {
        match self {
            Self::Rect([l, t, r, b]) => Self::Rect([l + dx, t + dy, r + dx, b + dy]),
            Self::Ellipse([x, y, rx, ry]) => Self::Ellipse([x + dx, y + dy, *rx, *ry]),
            Self::Points { points, closed } => Self::Points {
                points: points.iter().map(|(x, y)| (x + dx, y + dy)).collect(),
                closed: *closed,
            },
        }
    }

    /// The outline as a line through points, and whether it closes.
    pub fn line(&self) -> (Vec<Pt>, bool) {
        match self {
            Self::Rect(_) => (self.handles(), true),
            Self::Ellipse([x, y, rx, ry]) => (
                (0..ELLIPSE_STEPS)
                    .map(|step| {
                        let angle = step as f64 / ELLIPSE_STEPS as f64 * std::f64::consts::TAU;
                        (x + rx * angle.cos(), y + ry * angle.sin())
                    })
                    .collect(),
                true,
            ),
            Self::Points { points, closed } => (points.clone(), *closed),
        }
    }

    /// What an operation carries of it, each fraction to four places.
    pub fn fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        match self {
            Self::Rect(rect) => {
                fields.insert("rect".into(), json!(rect.map(fraction)));
            }
            Self::Ellipse(ellipse) => {
                fields.insert("ellipse".into(), json!(ellipse.map(fraction)));
            }
            Self::Points { points, closed } => {
                let points: Vec<[f64; 2]> = points
                    .iter()
                    .map(|(x, y)| [fraction(*x), fraction(*y)])
                    .collect();
                fields.insert("points".into(), json!(points));
                fields.insert("closed".into(), json!(closed));
            }
        }
        fields
    }
}

/// A mask on a clip's picture, as the picture shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub name: String,
    pub mode: String,
    pub outline: Outline,
}

impl Mask {
    /// A path only (mode none): it cuts nothing, and is drawn dashed.
    pub fn path_only(&self) -> bool {
        self.mode == "none"
    }
}

/// A clip's masks, in the order they combine.
pub fn masks_of(clip: &Clip) -> Vec<Mask> {
    named(clip)
        .filter_map(|mask| {
            Some(Mask {
                name: mask.name.clone(),
                // One that says no mode adds.
                mode: match mask.mode.as_str() {
                    "" => "add".to_owned(),
                    mode => mode.to_owned(),
                },
                outline: Outline::of(mask)?,
            })
        })
        .collect()
}

/// A clip's masks that have a name, which is what they are set and removed by.
fn named(clip: &Clip) -> impl Iterator<Item = &ViewMask> {
    clip.masks.iter().filter(|mask| !mask.name.is_empty())
}

/// The edit that gives the mask `name` of `clip` the outline `outline`.
pub fn mask_set(clip: &str, name: &str, outline: &Outline) -> Value {
    let mut op = outline.fields();
    op.insert("op".into(), json!("mask-set"));
    op.insert("clip".into(), json!(clip));
    op.insert("name".into(), json!(name));
    Value::Object(op)
}

/// The names of a clip's masks, whatever their outlines.
pub fn mask_names(clip: &Clip) -> Vec<String> {
    named(clip).map(|mask| mask.name.clone()).collect()
}

/// The name a new mask takes: the first of `k1`, `k2`, … not `taken`.
pub fn new_mask_name(taken: &[String]) -> String {
    (1..)
        .map(|n| format!("k{n}"))
        .find(|name| !taken.contains(name))
        .unwrap_or_default()
}

/// The edit that adds the mask `name` with the outline `outline` to
/// `clip`, after its others.
pub fn mask_add(clip: &str, name: &str, outline: &Outline) -> Value {
    let mut op = outline.fields();
    op.insert("op".into(), json!("mask-add"));
    op.insert("clip".into(), json!(clip));
    op.insert("name".into(), json!(name));
    Value::Object(op)
}

/// A rectangle, or the ellipse inside it, dragged out from `from` to
/// `to` (fractions of the clip's picture), corner to corner.
pub fn dragged(ellipse: bool, from: Pt, to: Pt) -> Outline {
    let (left, right) = (from.0.min(to.0), from.0.max(to.0));
    let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
    if ellipse {
        Outline::Ellipse([
            (left + right) / 2.0,
            (top + bottom) / 2.0,
            (right - left) / 2.0,
            (bottom - top) / 2.0,
        ])
    } else {
        Outline::Rect([left, top, right, bottom])
    }
}

/// Points a drag must go, across or down on the picture, to draw a
/// rectangle or an ellipse rather than be a click.
pub const DRAW_LEAST: f64 = 4.0;

/// The mask a rectangle or ellipse tool draws when its drag from `from`
/// to `to` is let go, `map` taking the clip's picture to the window.
/// None for a click (under [`DRAW_LEAST`] points both across and down),
/// and for an outline too thin to hold an area as the document keeps it.
pub fn drawn(ellipse: bool, from: Pt, to: Pt, map: impl Into<Projective>) -> Option<Outline> {
    let map = map.into();
    let (a, b) = (map.apply(from), map.apply(to));
    if (a.0 - b.0).abs() < DRAW_LEAST && (a.1 - b.1).abs() < DRAW_LEAST {
        return None;
    }
    let outline = dragged(ellipse, from, to);
    let holds = match &outline {
        Outline::Rect([l, t, r, b]) => fraction(*r) > fraction(*l) && fraction(*b) > fraction(*t),
        Outline::Ellipse([_, _, rx, ry]) => fraction(*rx) > 0.0 && fraction(*ry) > 0.0,
        Outline::Points { .. } => true,
    };
    holds.then_some(outline)
}

/// The caption-overlap box a finished check puts over the picture: where
/// (fractions of the frame: left, top, width, height) and what it measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlap {
    pub rect: [f64; 4],
    /// "8 px"; empty when the finding measured nothing.
    pub tag: String,
}

/// A number as a person reads it: four figures at most, no trailing zeros.
fn figures(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        return format!("{value:.0}");
    }
    let digits = (4 - 1 - value.abs().log10().floor() as i32).clamp(0, 6) as usize;
    let text = format!("{value:.digits$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

/// The `text.overlap` finding of `findings` (a `check` event's, done)
/// whose frames the playhead at `playhead` is on, give or take half a
/// frame at `fps`: the first such with an area. A number that is not
/// finite is none, as JSON's `null`.
pub fn overlap_at(findings: &[Finding], playhead: f64, fps: f64) -> Option<Overlap> {
    let slack = 0.5 / fps.max(1.0);
    let finite = |number: f64| number.is_finite().then_some(number);
    findings.iter().find_map(|finding| {
        if finding.code != "text.overlap" {
            return None;
        }
        let rect = finding.rect?;
        let (x, y) = (finite(rect.x)?, finite(rect.y)?);
        let (w, h) = (finite(rect.w)?, finite(rect.h)?);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        if let Some(at) = finding.at.and_then(finite) {
            let end = at + finding.duration.and_then(finite).unwrap_or(0.0).max(0.0);
            if playhead < at - slack || playhead > end + slack {
                return None;
            }
        }
        let tag = match finding.value.and_then(finite) {
            Some(value) => {
                let number = figures(value);
                match finding.unit.as_deref().filter(|unit| !unit.is_empty()) {
                    Some(unit) => format!("{number} {unit}"),
                    None => number,
                }
            }
            None => String::new(),
        };
        Some(Overlap {
            rect: [x, y, w, h],
            tag,
        })
    })
}

/// The distance from `at` to the line through `points`.
pub fn distance_to_line(points: &[Pt], closed: bool, at: Pt) -> f64 {
    let segment = |from: Pt, to: Pt| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 {
            (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (from.0 + dx * t - at.0).hypot(from.1 + dy * t - at.1)
    };
    let mut nearest = f64::INFINITY;
    for pair in points.windows(2) {
        nearest = nearest.min(segment(pair[0], pair[1]));
    }
    if closed && points.len() > 2 {
        nearest = nearest.min(segment(points[points.len() - 1], points[0]));
    }
    if points.len() == 1 {
        nearest = (points[0].0 - at.0).hypot(points[0].1 - at.1);
    }
    nearest
}

/// Whether `at` is inside the closed line through `points`.
pub fn inside(points: &[Pt], at: Pt) -> bool {
    let mut inside = false;
    let mut last = match points.last() {
        Some(last) => *last,
        None => return false,
    };
    for &point in points {
        if (point.1 > at.1) != (last.1 > at.1)
            && at.0 < (last.0 - point.0) * (at.1 - point.1) / (last.1 - point.1) + point.0
        {
            inside = !inside;
        }
        last = point;
    }
    inside
}

// ------------------------------------------------------- motion paths

/// How many samples each span between two points of a path has
/// (kerf_model's `path::SAMPLES`).
pub const SAMPLES_PER_SPAN: usize = 32;
/// How far out a path's point can be, either way, in frames.
const PATH_LIMIT: f64 = 10.0;

/// The points of a clip's `path=`, `x,y x,y`; none when it does not
/// read as the document requires.
pub fn parse_path(text: &str) -> Option<Vec<Pt>> {
    let points = text
        .split_whitespace()
        .map(|point| {
            let (x, y) = point.split_once(',')?;
            let (x, y): (f64, f64) = (x.parse().ok()?, y.parse().ok()?);
            (x.is_finite() && y.is_finite() && x.abs() <= PATH_LIMIT && y.abs() <= PATH_LIMIT)
                .then_some((x, y))
        })
        .collect::<Option<Vec<_>>>()?;
    (points.len() >= 2).then_some(points)
}

/// `points` as `path=` writes them, each held to what it allows.
pub fn path_text(points: &[Pt]) -> String {
    let text = |value: f64| {
        let rounded = (value.clamp(-PATH_LIMIT, PATH_LIMIT) * 10000.0).round() / 10000.0;
        format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
    };
    points
        .iter()
        .map(|(x, y)| format!("{},{}", text(*x), text(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `path=` can say `point` as it is.
pub fn path_holds((x, y): Pt) -> bool {
    x.abs() <= PATH_LIMIT && y.abs() <= PATH_LIMIT
}

/// How near two points of a path are to be the same one: more than
/// `path=` rounds a point by.
const PATH_SAME: f64 = 0.0002;

/// The motion path of the clip `id`, if it has one.
pub fn path_of(timeline: &Timeline, id: &str) -> Option<Vec<Pt>> {
    path_in(clip_in(timeline, id)?.0)
}

/// A clip's motion path as it is written; none for a clip without one, and for one that
/// does not read.
pub fn path_in(clip: &Clip) -> Option<Vec<Pt>> {
    text_of(&clip.props, "path").and_then(parse_path)
}

/// Where the point at `index` of `before` is in `after`: the one at its
/// place, as `path=` keeps it. None when `after` has no point there, as
/// an undo can leave another point at that index.
pub fn follow_point(before: &[Pt], index: usize, after: &[Pt]) -> Option<usize> {
    let (x, y) = *before.get(index)?;
    after
        .iter()
        .position(|(across, down)| (across - x).hypot(down - y) < PATH_SAME)
}

/// The edit that starts a motion path on the clip `id` at `position`
/// seconds into the program: from where the clip is, a quarter of the
/// canvas towards its middle, with `along` keyed from 0 to 1 over the
/// clip unless it says already how far along it is. None for a clip
/// that draws no picture, or has a path already.
pub fn start_path(timeline: &Timeline, id: &str, position: f64) -> Option<Value> {
    let placement = placement(timeline, id, position)?;
    let (clip, _) = clip_in(timeline, id)?;
    let props = &clip.props;
    if text_of(props, "path").is_some_and(|path| !path.trim().is_empty()) {
        return None;
    }
    let (from, _) = placement.path.apply((0.0, 0.0));
    let toward = if from > 0.5 { -0.25 } else { 0.25 };
    let mut set = Map::new();
    set.insert(
        "path".into(),
        json!(path_text(&[(0.0, 0.0), (toward, 0.0)])),
    );
    if props.get("along").is_none_or(Value::is_null) {
        let last = (clip.duration_frames - 1).max(1);
        set.insert(
            "along".into(),
            json!({"keys": [
                {"at": 0, "value": 0.0, "interp": "linear"},
                {"at": last, "value": 1.0, "interp": "linear"}
            ]}),
        );
    }
    Some(json!({"op": "set", "clip": id, "props": set}))
}

/// `points` with one more, on their curve `u` spans along.
pub fn with_point_on(points: &[Pt], aspect: f64, u: f64) -> Vec<Pt> {
    let on = MotionPath::new(points.to_vec(), aspect).curve(u);
    let span = (u.floor().max(0.0) as usize).min(points.len().saturating_sub(2));
    let mut points = points.to_vec();
    points.insert(span + 1, on);
    points
}

/// Where a clip is on its path: a point, and the way it heads, in
/// degrees clockwise from the right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnPath {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
}

/// The curve through a clip's path points (centripetal Catmull-Rom), on
/// a canvas `aspect` times as wide as it is high.
#[derive(Clone, Debug)]
pub struct MotionPath {
    points: Vec<Pt>,
    aspect: f64,
    distances: Vec<f64>,
}

impl MotionPath {
    /// From two points up.
    pub fn new(points: Vec<Pt>, aspect: f64) -> Self {
        let mut path = Self {
            points,
            aspect,
            distances: Vec::new(),
        };
        let total = path.points.len().saturating_sub(1) * SAMPLES_PER_SPAN;
        let mut distances = Vec::with_capacity(total + 1);
        distances.push(0.0);
        let mut last = path.curve(0.0);
        let mut walked = 0.0;
        for sample in 1..=total {
            let at = path.curve(sample as f64 / SAMPLES_PER_SPAN as f64);
            walked += ((at.0 - last.0) * aspect).hypot(at.1 - last.1);
            distances.push(walked);
            last = at;
        }
        path.distances = distances;
        path
    }

    /// On the curve, `u` spans along.
    pub fn curve(&self, u: f64) -> Pt {
        if self.points.len() < 2 {
            return self.points.first().copied().unwrap_or((0.0, 0.0));
        }
        let last = self.points.len() - 1;
        let span = (u.floor().max(0.0) as usize).min(last - 1);
        let t = (u - span as f64).clamp(0.0, 1.0);
        let (p1, p2) = (self.points[span], self.points[span + 1]);
        // Past the ends, a point continuing straight on.
        let p0 = if span == 0 {
            (2.0 * p1.0 - p2.0, 2.0 * p1.1 - p2.1)
        } else {
            self.points[span - 1]
        };
        let p3 = if span + 2 > last {
            (2.0 * p2.0 - p1.0, 2.0 * p2.1 - p1.1)
        } else {
            self.points[span + 2]
        };
        // Knots a square root of the distance apart, on the canvas.
        let gap = |a: Pt, b: Pt| {
            ((a.0 - b.0) * self.aspect)
                .hypot(a.1 - b.1)
                .sqrt()
                .max(1e-9)
        };
        let t0 = 0.0;
        let t1 = t0 + gap(p0, p1);
        let t2 = t1 + gap(p1, p2);
        let t3 = t2 + gap(p2, p3);
        let at = t1 + (t2 - t1) * t;
        let lerp = |a: Pt, b: Pt, low: f64, high: f64| {
            let k = (at - low) / (high - low);
            (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
        };
        // Barry and Goldman's pyramid.
        let a1 = lerp(p0, p1, t0, t1);
        let a2 = lerp(p1, p2, t1, t2);
        let a3 = lerp(p2, p3, t2, t3);
        let b1 = lerp(a1, a2, t0, t2);
        let b2 = lerp(a2, a3, t1, t3);
        lerp(b1, b2, t1, t2)
    }

    fn total(&self) -> f64 {
        self.distances.last().copied().unwrap_or(0.0)
    }

    /// How far along it is at its point `index`, as [`Self::at`] takes it.
    pub fn along_of(&self, index: usize) -> f64 {
        let total = self.total();
        if total <= 0.0 {
            return 0.0;
        }
        let index = index.min(self.points.len().saturating_sub(1)) * SAMPLES_PER_SPAN;
        self.distances.get(index).copied().unwrap_or(total) / total
    }

    fn at_distance(&self, along: f64) -> f64 {
        let total = self.total();
        if total <= 0.0 || self.distances.len() < 2 {
            return 0.0;
        }
        let wanted = along.clamp(0.0, 1.0) * total;
        let found = self
            .distances
            .partition_point(|distance| *distance < wanted);
        let next = found.clamp(1, self.distances.len() - 1);
        let (low, high) = (self.distances[next - 1], self.distances[next]);
        let within = if high > low {
            (wanted - low) / (high - low)
        } else {
            0.0
        };
        ((next - 1) as f64 + within) / SAMPLES_PER_SPAN as f64
    }

    /// Where a clip `along` of the way along is, from 0 to 1 of the distance.
    pub fn at(&self, along: f64) -> OnPath {
        let u = self.at_distance(along);
        let (x, y) = self.curve(u);
        let last = self.points.len().saturating_sub(1) as f64;
        const STEP: f64 = 1e-3;
        let before = self.curve((u - STEP).max(0.0));
        let after = self.curve((u + STEP).min(last));
        let heading = (after.1 - before.1)
            .atan2((after.0 - before.0) * self.aspect)
            .to_degrees();
        OnPath { x, y, heading }
    }

    /// The curve as a line, [`SAMPLES_PER_SPAN`] samples to each span.
    pub fn samples(&self) -> Vec<Pt> {
        let total = self.points.len().saturating_sub(1) * SAMPLES_PER_SPAN;
        (0..=total)
            .map(|sample| self.curve(sample as f64 / SAMPLES_PER_SPAN as f64))
            .collect()
    }
}
