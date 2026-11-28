//! Deterministic world simulation engine for Industrial Kerf.
//!
//! Combines:
//! - [`world`]: 3D rigid bodies on $SE(3)$ ([`kerf_geom::lie::Se3`]), colliders, XPBD variational
//!   constraints, and physics-driven particle fields.
//! - [`solver`]: `flicks`-quantized fixed-step Symplectic + XPBD solver with exact $SE(3)$
//!   geodesic sub-step interpolation.
//! - [`cache`]: [`WorldTimeline`] temporal checkpoint cache with causal invalidation, guaranteeing
//!   bit-identical random-access seeking vs sequential stepping.
//! - [`director`]: inverse goal-constraint shooting solver and smooth $SE(3)$ tracking-camera
//!   planner for agent-directed scenes.

pub mod cache;
pub mod director;
pub mod solver;
pub mod world;

pub use cache::WorldTimeline;
pub use director::{plan_tracking_camera, solve_launch_velocity, LaunchSolution, TrajectoryGoal};
pub use solver::{interpolate_state, step_world};
pub use world::{
    Collider, Constraint, ParticleFieldSpec, RigidBody, SimParticle, WorldSpec, WorldState,
    DEFAULT_STEP_FLICKS, MAX_SIM_PARTICLES,
};
