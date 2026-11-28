//! World specification and state types for deterministic rigid-body, constraint, and particle
//! simulation on $SE(3)$.

use kerf_geom::lie::{Quat, Se3, Twist};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::Time;

/// Exact 120 Hz fixed-step quantum in Kerf flicks (`705_600_000 / 120 = 5_880_000`), dividing
/// 24, 30, 60, and 120 fps cleanly with zero rounding drift.
pub const DEFAULT_STEP_FLICKS: i64 = FLICKS_PER_SECOND / 120;

/// Safety cap on concurrently alive simulated particles per field.
pub const MAX_SIM_PARTICLES: usize = 10_000;

/// Collision shape of a [`RigidBody`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    Sphere {
        radius: f64,
    },
    /// Static half-space `{ p : dot(p, normal) >= offset }` (normal should be unit length).
    Plane {
        normal: [f64; 3],
        offset: f64,
    },
    /// Oriented box centered at `pose.trans` with local half-extents.
    Box {
        half_extents: [f64; 3],
    },
}

/// A rigid body evolving on $SE(3)$.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidBody {
    pub pose: Se3,
    /// Center-of-mass angular (`omega`, rad/s) and linear (`v`, world units/s) velocity.
    pub twist: Twist,
    /// Inverse mass (`0.0` for static/immovable bodies).
    pub inv_mass: f64,
    /// Body-frame diagonal principal inverse inertia (`[0.0; 3]` for static bodies).
    pub inv_inertia: [f64; 3],
    pub collider: Collider,
    pub restitution: f64,
    pub friction: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
}

impl RigidBody {
    /// Dynamic solid sphere of uniform density (`I = 2/5 m r^2`).
    pub fn dynamic_sphere(pos: [f64; 3], vel: [f64; 3], radius: f64, mass: f64) -> Self {
        let r = radius.max(1e-6);
        let m = mass.max(1e-6);
        let inv_i = 1.0 / (0.4 * m * r * r);
        Self {
            pose: Se3::from_translation(pos),
            twist: Twist::linear(vel),
            inv_mass: 1.0 / m,
            inv_inertia: [inv_i; 3],
            collider: Collider::Sphere { radius: r },
            restitution: 0.4,
            friction: 0.3,
            linear_damping: 0.0,
            angular_damping: 0.0,
        }
    }

    /// Dynamic solid box of uniform density (`I_x = m (h_y^2 + h_z^2) / 3` for half-extents `h`).
    pub fn dynamic_box(pos: [f64; 3], vel: [f64; 3], half_extents: [f64; 3], mass: f64) -> Self {
        let h = half_extents.map(|e| e.max(1e-6));
        let m = mass.max(1e-6);
        let ix = m * (h[1] * h[1] + h[2] * h[2]) / 3.0;
        let iy = m * (h[0] * h[0] + h[2] * h[2]) / 3.0;
        let iz = m * (h[0] * h[0] + h[1] * h[1]) / 3.0;
        Self {
            pose: Se3::from_translation(pos),
            twist: Twist::linear(vel),
            inv_mass: 1.0 / m,
            inv_inertia: [1.0 / ix, 1.0 / iy, 1.0 / iz],
            collider: Collider::Box { half_extents: h },
            restitution: 0.35,
            friction: 0.4,
            linear_damping: 0.0,
            angular_damping: 0.0,
        }
    }

    /// Immovable half-space plane `{ p : dot(p, normal) >= offset }`.
    pub fn static_plane(normal: [f64; 3], offset: f64) -> Self {
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
            .sqrt()
            .max(1e-12);
        let unit = [normal[0] / len, normal[1] / len, normal[2] / len];
        Self {
            pose: Se3::new(
                Quat::IDENTITY,
                [unit[0] * offset, unit[1] * offset, unit[2] * offset],
            ),
            twist: Twist::ZERO,
            inv_mass: 0.0,
            inv_inertia: [0.0; 3],
            collider: Collider::Plane {
                normal: unit,
                offset,
            },
            restitution: 0.5,
            friction: 0.35,
            linear_damping: 0.0,
            angular_damping: 0.0,
        }
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        self.inv_mass > 0.0
    }
}

/// Variational XPBD constraint with inverse stiffness (`compliance >= 0`; `0` = infinitely stiff).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constraint {
    /// Maintains distance `rest_length` between body `a` and body `b`.
    Distance {
        a: usize,
        b: usize,
        rest_length: f64,
        compliance: f64,
    },
    /// Keeps `body` within `max_distance` of a world-space `anchor` (exact rod when
    /// `bilateral == true`, tether when `false`).
    Pin {
        body: usize,
        anchor: [f64; 3],
        distance: f64,
        bilateral: bool,
        compliance: f64,
    },
}

/// A simulated particle with deterministic birth ID and state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParticle {
    pub id: u64,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub age: f64,
    pub life: f64,
}

/// Specification for a deterministic physics-driven particle field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleFieldSpec {
    pub rate: f64,
    pub life: f64,
    pub velocity: f64,
    pub vary: f64,
    pub direction_deg: f64,
    pub spread_deg: f64,
    pub gravity: f64,
    pub drag: f64,
    pub bounce: f64,
    /// Optional 2D horizontal floor plane in pixel y-coordinates (Kerf y grows downward: particles
    /// with `pos[1] > floor_y` reflect upward with restitution `bounce`).
    pub floor_y: Option<f64>,
    pub turbulence: f64,
    pub turbulence_scale: f64,
    pub origin: [f64; 2],
    pub area: [f64; 2],
    pub seed: u64,
}

/// Immutable declarative specification of a simulated world at `t = 0`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSpec {
    /// World gravity acceleration vector (in Kerf coordinates, `+y` is downward).
    pub gravity: [f64; 3],
    /// XPBD substeps per tick (higher improves stiff-constraint convergence).
    pub substeps: u32,
    /// Exact fixed-step quantum in flicks.
    pub step_flicks: i64,
    /// Initial rigid bodies at `t = 0`.
    pub bodies: Vec<RigidBody>,
    /// Variational constraints acting on `bodies`.
    pub constraints: Vec<Constraint>,
    /// Optional physics-driven particle emitter coupled to the world's colliders.
    pub particles: Option<ParticleFieldSpec>,
}

impl Default for WorldSpec {
    fn default() -> Self {
        Self {
            gravity: [0.0, 9.81, 0.0],
            substeps: 4,
            step_flicks: DEFAULT_STEP_FLICKS,
            bodies: Vec::new(),
            constraints: Vec::new(),
            particles: None,
        }
    }
}

impl WorldSpec {
    /// Construct the initial [`WorldState`] at `step = 0` (`Time::ZERO`).
    pub fn initial_state(&self) -> WorldState {
        WorldState {
            step: 0,
            time: Time::ZERO,
            bodies: self.bodies.clone(),
            particles: Vec::new(),
            next_particle_id: 0,
        }
    }

    /// Exact step duration in seconds.
    #[inline]
    pub fn dt_seconds(&self) -> f64 {
        self.step_flicks.max(1) as f64 / FLICKS_PER_SECOND as f64
    }
}

/// Complete deterministic snapshot of a simulated world at a discrete tick.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldState {
    pub step: u64,
    pub time: Time,
    pub bodies: Vec<RigidBody>,
    pub particles: Vec<SimParticle>,
    pub next_particle_id: u64,
}

impl WorldState {
    /// Total translational + rotational kinetic energy of all dynamic bodies.
    pub fn kinetic_energy(&self) -> f64 {
        self.bodies
            .iter()
            .filter(|b| b.is_dynamic())
            .map(|b| {
                let m = 1.0 / b.inv_mass;
                let v = b.twist.v;
                let v2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                let w_local = b.pose.rot.inverse().rotate_vec(b.twist.omega);
                let rot_e = (0..3)
                    .map(|k| {
                        if b.inv_inertia[k] > 0.0 {
                            0.5 * (1.0 / b.inv_inertia[k]) * w_local[k] * w_local[k]
                        } else {
                            0.0
                        }
                    })
                    .sum::<f64>();
                0.5 * m * v2 + rot_e
            })
            .sum()
    }

    /// Total linear momentum of all dynamic bodies.
    pub fn total_momentum(&self) -> [f64; 3] {
        let mut p = [0.0; 3];
        for b in self.bodies.iter().filter(|b| b.is_dynamic()) {
            let m = 1.0 / b.inv_mass;
            for (slot, v) in p.iter_mut().zip(b.twist.v) {
                *slot += m * v;
            }
        }
        p
    }
}
