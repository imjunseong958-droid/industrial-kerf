//! What the picture's overlays are drawn from: where a clip's picture lands on
//! the canvas, its masks' outlines, and the curve of its motion path. Where it
//! lands is the compositor's own answer: a clip's numbers are read here, from
//! the snapshot's timeline as the bridge sends it, and placed by the sums the
//! compositor draws by (kerf-geom's `place`), on the curve it follows
//! (kerf-model's `MotionPath`). Pure functions of that timeline, so that they
//! can be held to the compositor's own numbers.

use editor_state::model::{
    Clip, EditKey, EditParam, Interp, Timeline, TimelineShow, TrackKind, ViewParam,
};
use kerf_bridge::event::{Finding, ViewMask};
use kerf_geom::place::{lifted, Camera, Geometry, Lens, Parent, Taken};
use kerf_geom::{Affine3, Mat3, Vec2};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// A map of the plane that keeps lines straight, as the compositor has one.
pub use kerf_geom::Affine;

/// A clip's properties by name, as the timeline says them: each a value, or its
/// keyframes.
pub type Props = BTreeMap<String, ViewParam>;

/// The text `name` of a clip's `props` says, when it says one.
fn text_of<'a>(props: &'a Props, name: &str) -> Option<&'a str> {
    props
        .get(name)
        .and_then(ViewParam::value)
        .and_then(Value::as_str)
}

/// Whether `name` of a clip's `props` is on.
fn is_on(props: &Props, name: &str) -> bool {
    props
        .get(name)
        .and_then(ViewParam::value)
        .and_then(Value::as_bool)
        == Some(true)
}

/// A point: across, then down.
pub type Pt = (f64, f64);

fn vec2((x, y): Pt) -> Vec2 {
    Vec2::new(x, y)
}

fn pt(at: Vec2) -> Pt {
    (at.x, at.y)
}

/// The nearest a camera sees, in frame heights: what is nearer than this
/// along its way, or behind it, is out of sight.
pub const NEAREST: f64 = Camera::NEAR;

/// What `w` is held to at least as a point is mapped: a point on or behind
/// the camera still maps somewhere.
const NEAR_CLIP: f64 = 1e-6;

/// A map of the plane as a camera may see it, in perspective (kerf-geom's
/// `Mat3`), with what of the plane is in sight: `(x, y)` goes across and
/// down over `w`. For a placement in depth `w` is how far along the camera's
/// way the point is, in frame heights; for a flat one it is 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projective(pub Mat3);

impl From<Affine> for Projective {
    fn from(map: Affine) -> Self {
        Self(map.into())
    }
}

impl Projective {
    /// What a placement is when there is nothing to place on: no map back.
    pub const UNSEEN: Self = Self(Mat3([[0.0; 3]; 3]));

    /// This map, and then `next`.
    pub fn then(self, next: impl Into<Self>) -> Self {
        Self(next.into().0 * self.0)
    }

    /// How far along the camera's way `at` is: its `w`.
    fn depth(self, at: Pt) -> f64 {
        self.0.undivided(vec2(at))[2]
    }

    /// Where `at` goes. One on or behind the camera goes somewhere
    /// far off; [`Self::in_sight`] says which are seen.
    pub fn apply(self, at: Pt) -> Pt {
        let [across, down, w] = self.0.undivided(vec2(at));
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
        let det = self.0.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        self.0.inverse().map(Self)
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

/// The number `name` of a clip's `props` at `clip_time` seconds into
/// it: a value, or its keyframes between; `fallback` when it has none.
pub fn property_at(props: &Props, name: &str, clip_time: f64, fallback: f64) -> f64 {
    props
        .get(name)
        .and_then(|param| param.number_at(clip_time))
        .unwrap_or(fallback)
}

/// The least of a picture a crop is taken to leave, as a share of it. The
/// compositor draws no program with a clip cropped away entirely, and what
/// is drawn over that clip's picture still has to be somewhere.
const LEAST_KEPT: f64 = 1e-6;

/// A clip's geometry at `clip_time` seconds into it, as its `props` say it
/// (kerf-geom's `Geometry`): a number it does not say is the one it rests at.
fn geometry_at(props: &Props, clip_time: f64) -> Geometry {
    let mut geometry = Geometry::of(Geometry::NAMES.map(|name| {
        let unsaid = match name {
            "scale" | "scale_x" | "scale_y" => 1.0,
            _ => 0.0,
        };
        property_at(props, name, clip_time, unsaid)
    }));
    let [l, r, t, b] = geometry.crop;
    geometry.crop = [
        l,
        r.min(1.0 - l - LEAST_KEPT),
        t,
        b.min(1.0 - t - LEAST_KEPT),
    ];
    geometry
}

/// The same with the clip's motion path added in: the path adds to
/// where it would be, and `orient` to how it is turned.
fn moved_geometry_at(props: &Props, canvas: Pt, clip_time: f64) -> Geometry {
    let geometry = geometry_at(props, clip_time);
    let Some(points) = text_of(props, "path").and_then(parse_path) else {
        return geometry;
    };
    let on =
        MotionPath::new(points, aspect(canvas)).at(property_at(props, "along", clip_time, 0.0));
    geometry.along(
        Vec2::new(on.x, on.y),
        is_on(props, "orient").then_some(on.heading),
    )
}

fn aspect((width, height): Pt) -> f64 {
    width.max(1.0) / height.max(1.0)
}

/// The canvas a timeline is drawn on, in pixels ([`TimelineShow::canvas`]).
pub fn canvas_of(timeline: &Timeline) -> Pt {
    let (width, height) = timeline.canvas();
    (f64::from(width), f64::from(height))
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

/// A clip's own numbers in depth at `clip_time`: its `z`, in frame heights,
/// and how it is turned about x and about y, in degrees.
fn depth_at(props: &Props, clip_time: f64) -> [f64; 3] {
    ["z", "rotate_x", "rotate_y"].map(|name| property_at(props, name, clip_time, 0.0))
}

/// The world a clip in depth is placed in is the canvas's own pixels: one of
/// them to each of the canvas's, across and down.
const PER_PIXEL: Pt = (1.0, 1.0);

/// How the parents of `clip` take it at `position` seconds into the
/// program, the one it names first (kerf-geom's `Taken`): across the
/// canvas, and those of them in depth then in depth. Each is read here,
/// and the walk ends at one that is not there or draws no picture, and
/// at one met before.
fn taken_by_parents(timeline: &Timeline, clip: &Clip, canvas: Pt, position: f64) -> Taken {
    let mut met: Vec<&str> = Vec::new();
    let mut parent = clip.parent.as_deref();
    let parents = std::iter::from_fn(|| {
        let id = parent.filter(|id| !id.is_empty() && !met.contains(id))?;
        let (rig, track) = clip_in(timeline, id)?;
        let (size, fit) = picture_of(timeline, rig, track, canvas)?;
        met.push(id);
        parent = rig.parent.as_deref();
        let clip_time = position - rig.timeline_start;
        Some(Parent {
            geometry: moved_geometry_at(&rig.props, canvas, clip_time),
            size,
            fit,
            depth: leads_in_depth(rig).then(|| depth_at(&rig.props, clip_time)),
        })
    });
    Taken::by(parents, canvas)
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
        self.path.shows()
            && MotionPath::new(points.to_vec(), aspect(self.canvas))
                .samples()
                .iter()
                .any(|at| self.path.in_sight(*at))
    }
}

/// What a clip is placed with at a moment: its picture's size, whether
/// it is fitted to the canvas, the canvas, seconds into it, how its
/// parents have moved it on the canvas and how those of them in depth
/// carry it there, and where it starts.
#[derive(Clone, Copy, Debug)]
struct Scene {
    picture: Pt,
    fit: bool,
    canvas: Pt,
    clip_time: f64,
    parents: Affine,
    carried: Option<Affine3>,
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
    let taken = taken_by_parents(timeline, clip, canvas, start + clip_time);
    Some((
        clip,
        Scene {
            picture,
            fit,
            canvas,
            clip_time,
            parents: taken.moved,
            carried: taken.carried(canvas, PER_PIXEL, canvas.1),
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

/// Fractions of a clip's picture to canvas pixels, flat: where its numbers
/// put it and then where its parents move it, as the compositor places it.
/// As the document says it, and not on the whole pixels the compositor
/// draws a clip that holds still on, which are under a pixel from here.
fn flat_placement(clip: &Clip, scene: &Scene) -> Affine {
    let geometry = moved_geometry_at(&clip.props, scene.canvas, scene.clip_time);
    let (own, _) = geometry.placed(scene.picture, scene.fit, false, scene.canvas);
    scene
        .parents
        .then_apply(&own)
        .then_apply(&Affine::scale(scene.picture.0, scene.picture.1))
}

/// Where a clip in depth is in the world, by fractions of its picture,
/// which `flat` places on the canvas: taken into depth as the compositor
/// takes it (kerf-geom's `lifted`), held there by its own numbers about
/// its middle, and then as `carried` has its parents in depth take it.
fn in_the_world(clip: &Clip, scene: &Scene, flat: &Affine, carried: Option<&Affine3>) -> Affine3 {
    let canvas = scene.canvas;
    let [z, pitch, yaw] = depth_at(&clip.props, scene.clip_time);
    let numbers = [z * canvas.1, pitch, yaw];
    let center = (canvas.0 / 2.0, canvas.1 / 2.0);
    lifted(flat, (1.0, 1.0), center, numbers, carried).0
}

/// `timeline`'s topmost camera showing at `at` seconds, or the resting
/// one, which sees the plane z = 0 as the flat picture: kerf-geom's
/// `Camera`, of its numbers as they are read here.
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
    let Some(clip) = shown else {
        return Camera::of(&Lens::RESTING, canvas, None);
    };
    let clip_time = at - clip.timeline_start;
    let read = |name: &str, unsaid: f64| property_at(&clip.props, name, clip_time, unsaid);
    let lens = Lens {
        at: [read("x", 0.0), read("y", 0.0), read("z", 0.0)],
        tilt: read("tilt", 0.0),
        pan: read("pan", 0.0),
        roll: read("roll", 0.0),
        fov: read("fov", Camera::RESTING_FOV),
    };
    // A camera with a parent is where the parent takes it, looking as
    // the parent turns it.
    let rigged = clip
        .parent
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        .then(|| taken_by_parents(timeline, clip, canvas, at).rigged(canvas, PER_PIXEL, canvas.1));
    Camera::of(&lens, canvas, rigged.as_ref())
}

/// How `timeline`'s camera sees the plane `place` puts in the world at the
/// scene's moment, by the plane's own terms; [`Projective::UNSEEN`] for one
/// seen edge on.
fn seen(timeline: &Timeline, scene: &Scene, place: &Affine3) -> Projective {
    camera_at(timeline, scene.canvas, scene.at())
        .sees(place)
        .map_or(Projective::UNSEEN, Projective)
}

/// Fractions of `clip`'s picture to canvas pixels: flat, or in depth
/// as the camera sees it. [`Projective::UNSEEN`] when none of its corners
/// is in sight.
fn picture_placement(timeline: &Timeline, clip: &Clip, scene: &Scene) -> Projective {
    let flat = flat_placement(clip, scene);
    if !in_depth(clip) {
        return flat.into();
    }
    let place = in_the_world(clip, scene, &flat, scene.carried.as_ref());
    let seen = seen(timeline, scene, &place);
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    if corners.iter().any(|corner| seen.in_sight(*corner)) {
        seen
    } else {
        Projective::UNSEEN
    }
}

/// A point of `clip`'s motion path to fractions of the canvas. In depth
/// the path is on the plane at the clip's `z`, where the camera sees it;
/// [`Projective::UNSEEN`] for a clip that faces along its path
/// (`orient`) from an anchor off its middle, which no one map shows.
fn path_placement(timeline: &Timeline, clip: &Clip, scene: &Scene) -> Projective {
    let (canvas, picture) = (scene.canvas, scene.picture);
    // The path says where the clip's anchor goes: from where it lands with
    // no path, as far as a point of the path says, in canvas widths and
    // heights, and then as the clip's parents move it.
    let geometry = geometry_at(&clip.props, scene.clip_time);
    let (own, _) = geometry.placed(picture, scene.fit, false, canvas);
    let from = own.apply(geometry.anchor_in(picture));
    let on_canvas = scene
        .parents
        .then_apply(&Affine::translate(from.x, from.y))
        .then_apply(&Affine::scale(canvas.0, canvas.1));
    let to_shares = Affine::scale(1.0 / canvas.0, 1.0 / canvas.1);
    if !in_depth(clip) {
        return to_shares.then_apply(&on_canvas).into();
    }
    // In depth the clip turns about its middle, and its anchor goes round
    // with it: as far from where it is on the canvas as being held in
    // depth takes it.
    let flat = flat_placement(clip, scene);
    let anchor = geometry.anchor_in((1.0, 1.0));
    let held = in_the_world(clip, scene, &flat, None).apply([anchor.x, anchor.y, 0.0]);
    let on = flat.apply(anchor);
    let center = (canvas.0 / 2.0, canvas.1 / 2.0);
    let moved = [
        held[0] - (on.x - center.0),
        held[1] - (on.y - center.1),
        held[2],
    ];
    // Past being moved back by its `z`, that is another way at each point
    // of a path the clip faces along.
    let [z, ..] = depth_at(&clip.props, scene.clip_time);
    let swung = [moved[0], moved[1], moved[2] - z * canvas.1];
    let off: f64 = swung.iter().map(|part| part * part).sum();
    if off > 1e-12 && is_on(&clip.props, "orient") {
        return Projective::UNSEEN;
    }
    // The canvas's plane moved by as much, and where the clip's parents in
    // depth take it, the path's plane with it.
    let plane = Affine3::IDENTITY.moved([moved[0] - center.0, moved[1] - center.1, moved[2]]);
    let plane = match &scene.carried {
        Some(carried) => carried.then_apply(&plane),
        None => plane,
    };
    Projective::from(on_canvas)
        .then(seen(timeline, scene, &plane))
        .then(to_shares)
}

/// Where a clip's anchor is in its picture, in fractions of it: what
/// its motion path takes along.
fn anchor_of(geometry: &Geometry) -> Pt {
    pt(geometry.anchor_in((1.0, 1.0)))
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
    moved.props.insert("path".into(), json!(text).into());
    if let Some(along) = along {
        moved.props.insert("along".into(), json!(along).into());
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

/// How many samples each span between two points of a path has.
pub const SAMPLES_PER_SPAN: usize = kerf_model::MotionPath::SAMPLES;
/// How far out a path's point can be, either way, in frames.
const PATH_LIMIT: f64 = 10.0;

/// The points of a clip's `path=`, `x,y x,y`; none when it does not
/// read as the document requires.
pub fn parse_path(text: &str) -> Option<Vec<Pt>> {
    let points = kerf_model::parse_path_points(text).ok()?;
    Some(points.into_iter().map(|[x, y]| (x, y)).collect())
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
    if props
        .get("along")
        .is_none_or(|along| along.value().is_some_and(Value::is_null))
    {
        let last = (clip.duration_frames - 1).max(1);
        let key = |at, value| EditKey {
            at,
            value: json!(value),
            interp: Interp::Linear,
        };
        set.insert(
            "along".into(),
            json!(EditParam::Keyed(vec![key(0, 0.0), key(last, 1.0)])),
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

/// The curve through a clip's path points on a canvas `aspect` times as
/// wide as it is high, which the compositor moves the clip along
/// (kerf-model's `MotionPath`), by points as the overlays have them.
#[derive(Clone, Debug)]
pub struct MotionPath(kerf_model::MotionPath);

impl MotionPath {
    /// From two points up.
    pub fn new(points: Vec<Pt>, aspect: f64) -> Self {
        let points = points.into_iter().map(|(x, y)| [x, y]).collect();
        Self(kerf_model::MotionPath::new(points, aspect))
    }

    /// On the curve, `u` spans along.
    pub fn curve(&self, u: f64) -> Pt {
        let [x, y] = self.0.curve(u);
        (x, y)
    }

    /// How far along it is at its point `index`, as [`Self::at`] takes it.
    pub fn along_of(&self, index: usize) -> f64 {
        self.0.along_of(index)
    }

    /// Where a clip `along` of the way along is, from 0 to 1 of the
    /// distance, and the way it heads there, in degrees clockwise from
    /// the right.
    pub fn at(&self, along: f64) -> kerf_model::PathPoint {
        self.0.at(along)
    }

    /// The curve as a line, [`SAMPLES_PER_SPAN`] samples to each span.
    pub fn samples(&self) -> Vec<Pt> {
        let samples = self.0.samples();
        samples.into_iter().map(|[x, y]| (x, y)).collect()
    }
}
