//! Deterministic 3D rigid-body world built directly from a sequence's `physics=dynamic|static`
//! clips and evaluated through [`kerf_sim::WorldTimeline`].

use super::*;
use kerf_geom::lie::{Quat, Se3, Twist};
use kerf_sim::{Constraint, RigidBody, WorldSpec, WorldTimeline, DEFAULT_STEP_FLICKS};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Physics participation mode declared on a video clip via `physics=none|dynamic|static`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::draw) enum PhysicsMode {
    None,
    Dynamic,
    Static,
}

pub(in crate::draw) fn physics_mode(clip: &Clip) -> PhysicsMode {
    match param(clip, "physics") {
        Param::Static(Value::Enum(kind)) if kind == "dynamic" => PhysicsMode::Dynamic,
        Param::Static(Value::Enum(kind)) if kind == "static" => PhysicsMode::Static,
        _ => PhysicsMode::None,
    }
}

/// Declarative simulation world extracted from a sequence's 3D physics clips.
///
/// Shared between the compositor and the MCP director harness so inverse trajectory solving and
/// live rendering always execute the exact same [`WorldSpec`].
#[derive(Clone, Debug)]
pub struct SequenceWorldSpec {
    pub spec: WorldSpec,
    /// Maps each participating clip id to its index in `spec.bodies`.
    pub body_by_clip: HashMap<Id, usize>,
    /// Clip ids whose poses are driven by the simulation (`physics=dynamic`).
    pub dynamic_clips: Vec<(Id, usize)>,
    /// Sequence width / height aspect ratio (`x` in frame widths = `x * aspect` in frame heights).
    pub aspect: f64,
    /// Sequence canvas size in pixels.
    pub size: (u32, u32),
    /// Sequence frame rate.
    pub rate: Rate,
    /// Content digest covering every physics body & constraint in this sequence.
    pub digest: u64,
}

impl SequenceWorldSpec {
    /// Look up a participating clip's index in `spec.bodies` by clip id.
    pub fn body_index(&self, clip_id: &str) -> Option<usize> {
        self.body_by_clip
            .iter()
            .find(|(id, _)| id.as_str() == clip_id)
            .map(|(_, &idx)| idx)
    }
}

/// Stateful per-sequence timeline cache wrapping [`WorldTimeline`].
pub(in crate::draw) struct PhysicsWorld {
    dynamic_clips: Vec<(Id, usize)>,
    timeline: WorldTimeline,
}

impl PhysicsWorld {
    pub(in crate::draw) fn from_spec(world: SequenceWorldSpec) -> Self {
        Self {
            dynamic_clips: world.dynamic_clips,
            timeline: WorldTimeline::new(world.spec, 12, 128),
        }
    }

    /// Deterministically sample `(pose, twist)` in frame-height world coordinates for every
    /// dynamic clip at sequence time `at`.
    pub(in crate::draw) fn sample(&mut self, at: Time) -> HashMap<Id, (Se3, Twist)> {
        let state = self.timeline.state_at(at);
        let mut out = HashMap::with_capacity(self.dynamic_clips.len());
        for (id, idx) in &self.dynamic_clips {
            if let Some(body) = state.bodies.get(*idx) {
                out.insert(id.clone(), (body.pose, body.twist));
            }
        }
        out
    }
}

/// Initial world-space placement of a clip at `Time::ZERO` in frame heights (`x` right, `y` down,
/// `z` away from camera, centered at the frame middle).
fn initial_pose_and_extents(
    clip: &Clip,
    size: (u32, u32),
    fit: bool,
    canvas: (f64, f64),
) -> (Se3, [f64; 2]) {
    let height = canvas.1.max(1.0);
    let center = (canvas.0 / 2.0, canvas.1 / 2.0);
    let numbers = geometry_numbers(clip, None, Time::ZERO);
    let (to_output, _) = numbers.placed(pixels(size), fit, false, canvas);
    let read = |name: &str| number(&param(clip, name).value_at(Time::ZERO));
    let depth_nums = [read("z") * height, read("rotate_x"), read("rotate_y")];
    let (place, middle) = lifted(&to_output, pixels(size), center, depth_nums, None);
    let across_len = dot(place.x, place.x).sqrt().max(1e-9);
    let down_len = dot(place.y, place.y).sqrt().max(1e-9);
    let rx = times(place.x, 1.0 / across_len);
    let ry = times(place.y, 1.0 / down_len);
    let rz = unit(cross(rx, ry));
    let rot = Quat::from_basis(rx, ry, rz);
    let trans = [middle[0] / height, middle[1] / height, middle[2] / height];
    let ext_x = across_len * f64::from(size.0) / height;
    let ext_y = down_len * f64::from(size.1) / height;
    (Se3::new(rot, trans), [ext_x, ext_y])
}

fn rig_anchor_in_frame_heights(rig: &Rig, canvas: (f64, f64)) -> [f64; 3] {
    let (pose, _) = initial_pose_and_extents(&rig.clip, rig.size, rig.fit, canvas);
    pose.trans
}

/// Extract a [`SequenceWorldSpec`] from planned sequence tracks, or `None` when no clip enables
/// `physics=dynamic|static`.
pub(in crate::draw) fn build_sequence_world(
    tracks: &[TrackPlan],
    rigs: &HashMap<Id, Rig>,
    canvas: (u32, u32),
    rate: Rate,
) -> Option<SequenceWorldSpec> {
    let canvas_f = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let aspect = canvas_f.0 / canvas_f.1;
    let mesh_def = Catalog::builtin().fx("mesh");

    let mut bodies = Vec::new();
    let mut constraints = Vec::new();
    let mut body_by_clip = HashMap::new();
    let mut dynamic_clips = Vec::new();
    let mut world_gravity: Option<f64> = None;
    let mut hasher = DefaultHasher::new();

    for track in tracks.iter().filter(|t| t.shows) {
        for shown in &track.shown {
            let plan = &shown.plan;
            let clip = &plan.clip;
            let mode = physics_mode(clip);
            if mode == PhysicsMode::None || !plan.three_d {
                continue;
            }
            if body_by_clip.contains_key(&clip.id) {
                continue;
            }
            let read = |name: &str| number(&param(clip, name).value_at(Time::ZERO));
            let (pose, [ext_x, ext_y]) =
                initial_pose_and_extents(clip, plan.size, plan.fit, canvas_f);
            let mass = read("mass").max(0.01);
            let restitution = read("restitution").clamp(0.0, 1.0);
            let friction = read("friction").max(0.0);
            let linear_drag = read("linear_drag").max(0.0);
            let vel = [read("vel_x"), read("vel_y"), read("vel_z")];

            if mode == PhysicsMode::Dynamic && world_gravity.is_none() {
                world_gravity = Some(read("world_gravity"));
            }

            let is_mesh = matches!(plan.source, Source::Mesh);
            let mut body = if is_mesh {
                let mread = |name: &str| {
                    generated(mesh_def, clip, name, Time::ZERO)
                        .and_then(|v| v.as_number())
                        .unwrap_or(0.25)
                        .max(1e-3)
                };
                let scale = read("scale").abs().max(1e-3);
                let sx = scale * read("scale_x").abs().max(1e-3);
                let sy = scale * read("scale_y").abs().max(1e-3);
                let w = mread("width") * sx;
                let h = mread("height") * sy;
                let l = mread("length") * scale;
                let is_ball = matches!(
                    generated(mesh_def, clip, "kind", Time::ZERO),
                    Some(Value::Enum(kind)) if kind == "ball"
                );
                let mut b = if is_ball {
                    let r = w.max(h).max(l) * 0.5;
                    RigidBody::dynamic_sphere(pose.trans, vel, r, mass)
                } else {
                    RigidBody::dynamic_box(pose.trans, vel, [w * 0.5, h * 0.5, l * 0.5], mass)
                };
                if mode == PhysicsMode::Static {
                    b.inv_mass = 0.0;
                    b.inv_inertia = [0.0; 3];
                    b.twist = Twist::ZERO;
                }
                b
            } else {
                let depth = read("depth").max(0.0);
                if mode == PhysicsMode::Static && depth <= 1e-6 {
                    // Flat static 3D plane (floor / ramp / wall): orient normal upward (-y) when
                    // non-vertical, or toward camera (-z) when vertical.
                    let [_, _, mut n] = pose.rot.to_basis();
                    if n[1].abs() > 1e-4 {
                        if n[1] > 0.0 {
                            n = times(n, -1.0);
                        }
                    } else if n[2] > 0.0 {
                        n = times(n, -1.0);
                    }
                    let offset = dot(pose.trans, n);
                    RigidBody::static_plane(n, offset)
                } else {
                    let half = [
                        (ext_x * 0.5).max(1e-3),
                        (ext_y * 0.5).max(1e-3),
                        (depth * 0.5).max(0.02),
                    ];
                    let mut b = RigidBody::dynamic_box(pose.trans, vel, half, mass);
                    if mode == PhysicsMode::Static {
                        b.inv_mass = 0.0;
                        b.inv_inertia = [0.0; 3];
                        b.twist = Twist::ZERO;
                    }
                    b
                }
            };

            if !matches!(body.collider, kerf_sim::Collider::Plane { .. }) {
                body.pose = pose;
            }
            body.restitution = restitution;
            body.friction = friction;
            body.linear_damping = linear_drag;

            let idx = bodies.len();
            bodies.push(body);
            body_by_clip.insert(clip.id.clone(), idx);
            if mode == PhysicsMode::Dynamic {
                dynamic_clips.push((clip.id.clone(), idx));
                let pin_dist = read("pin_distance");
                if pin_dist > 0.0 {
                    let anchor = clip
                        .parent
                        .as_ref()
                        .and_then(|pid| rigs.get(pid))
                        .map(|rig| rig_anchor_in_frame_heights(rig, canvas_f))
                        .unwrap_or([pose.trans[0], pose.trans[1] - pin_dist, pose.trans[2]]);
                    constraints.push(Constraint::Pin {
                        body: idx,
                        anchor,
                        distance: pin_dist,
                        bilateral: true,
                        compliance: 1e-5,
                    });
                }
            }

            // Hash every deterministic body property into the sequence physics digest.
            clip.id.hash(&mut hasher);
            (mode as u8).hash(&mut hasher);
            for v in [
                body.pose.trans[0],
                body.pose.trans[1],
                body.pose.trans[2],
                body.pose.rot.w,
                body.pose.rot.x,
                body.pose.rot.y,
                body.pose.rot.z,
                body.twist.v[0],
                body.twist.v[1],
                body.twist.v[2],
                body.inv_mass,
                body.restitution,
                body.friction,
                body.linear_damping,
            ] {
                v.to_bits().hash(&mut hasher);
            }
        }
    }

    if bodies.is_empty() {
        return None;
    }

    let gravity_y = world_gravity.unwrap_or(1.8);
    gravity_y.to_bits().hash(&mut hasher);
    constraints.len().hash(&mut hasher);

    Some(SequenceWorldSpec {
        spec: WorldSpec {
            gravity: [0.0, gravity_y, 0.0],
            substeps: 4,
            step_flicks: DEFAULT_STEP_FLICKS,
            bodies,
            constraints,
            particles: None,
        },
        body_by_clip,
        dynamic_clips,
        aspect,
        size: canvas,
        rate,
        digest: hasher.finish(),
    })
}
