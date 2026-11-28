//! Property and invariant tests for `kerf-sim`:
//! - Bit-identical random-access scrubbing vs fresh sequential stepping.
//! - Causal edit invalidation preserving pre-edit checkpoints.
//! - Rigid-body non-penetration, elastic momentum/energy conservation, and XPBD constraints.
//! - Particle bounce/drag/turbulence invariants.
//! - Inverse Director shooting solver and $SE(3)$ tracking camera bridge to `kerf_geom::Camera`.

use kerf_geom::lie::{Quat, Se3};
use kerf_geom::place::Camera;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::Time;
use kerf_sim::{
    interpolate_state, plan_tracking_camera, solve_launch_velocity, step_world, Constraint,
    ParticleFieldSpec, RigidBody, TrajectoryGoal, WorldSpec, WorldTimeline, DEFAULT_STEP_FLICKS,
};
use proptest::prelude::*;

fn reference_state_at(spec: &WorldSpec, time: Time) -> kerf_sim::WorldState {
    let flicks = time.flicks();
    let mut state = spec.initial_state();
    if flicks <= 0 {
        return state;
    }
    let step_flicks = spec.step_flicks.max(1);
    let step_lo = (flicks / step_flicks) as u64;
    let rem = flicks % step_flicks;
    while state.step < step_lo {
        step_world(spec, &mut state);
    }
    if rem == 0 {
        return state;
    }
    let mut hi = state.clone();
    step_world(spec, &mut hi);
    let alpha = rem as f64 / step_flicks as f64;
    interpolate_state(&state, &hi, alpha)
}

fn demo_world(seed: u64, gravity_y: f64, restitution: f64, drag: f64) -> WorldSpec {
    let mut sphere = RigidBody::dynamic_sphere([-0.4, -0.8, 0.2], [0.6, -0.3, 0.1], 0.15, 1.5);
    sphere.restitution = restitution;
    sphere.linear_damping = drag;

    let mut box_body = RigidBody::dynamic_box([0.35, -0.9, -0.1], [-0.4, 0.1, 0.2], [0.12; 3], 2.0);
    box_body.pose.rot = Quat::exp([0.3, -0.4, 0.2]);
    box_body.restitution = restitution;

    // Static floor at y = 0.5 with upward normal [0, -1, 0] (Kerf +y is downward):
    // dot(p, [0, -1, 0]) >= -0.5  <=>  p.y <= 0.5.
    let floor = RigidBody::static_plane([0.0, -1.0, 0.0], -0.5);

    WorldSpec {
        gravity: [0.0, gravity_y, 0.0],
        substeps: 3,
        step_flicks: DEFAULT_STEP_FLICKS,
        bodies: vec![sphere, box_body, floor],
        constraints: vec![Constraint::Distance {
            a: 0,
            b: 1,
            rest_length: 0.85,
            compliance: 1e-4,
        }],
        particles: Some(ParticleFieldSpec {
            rate: 30.0,
            life: 1.2,
            velocity: 1.5,
            vary: 0.25,
            direction_deg: 90.0,
            spread_deg: 45.0,
            gravity: gravity_y,
            drag,
            bounce: restitution,
            floor_y: Some(0.5),
            turbulence: 1.2,
            turbulence_scale: 0.5,
            origin: [0.0, -0.4],
            area: [0.2, 0.1],
            seed,
        }),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Invariant: Random-access scrubbing in any order (including sub-tick flicks and aggressive
    /// checkpoint eviction) yields bit-for-bit identical world states to sequential stepping from 0.
    #[test]
    fn random_seek_is_bit_identical_to_sequential(
        seed in 0u64..10_000,
        gravity_y in 2.0f64..15.0,
        restitution in 0.1f64..0.95,
        drag in 0.0f64..2.0,
        stride in 2u64..10,
        max_cp in 2usize..7,
        queries in prop::collection::vec(0i64..(FLICKS_PER_SECOND * 3 / 2), 4..14),
    ) {
        let spec = demo_world(seed, gravity_y, restitution, drag);
        let mut timeline = WorldTimeline::new(spec.clone(), stride, max_cp);
        for flicks in queries {
            let t = Time::from_flicks(flicks);
            let cached = timeline.state_at(t);
            let direct = reference_state_at(&spec, t);
            prop_assert_eq!(cached, direct, "mismatch at flicks={}", flicks);
        }
        prop_assert!(timeline.cached_checkpoints() <= max_cp);
    }

    /// Invariant: Sphere and oriented box colliders dropped onto a static half-space floor never
    /// penetrate beneath the plane after stepping.
    #[test]
    fn rigid_bodies_never_penetrate_static_floor(
        y0 in -1.5f64..-0.2,
        vy0 in -1.0f64..4.0,
        rot_vec in prop::array::uniform3(-1.5f64..1.5),
        restitution in 0.0f64..0.9,
    ) {
        let floor_y = 0.5;
        let radius = 0.14;
        let half = [0.1, 0.12, 0.08];
        let mut sphere = RigidBody::dynamic_sphere([-0.3, y0, 0.0], [0.2, vy0, 0.0], radius, 1.0);
        sphere.restitution = restitution;
        let mut box_b = RigidBody::dynamic_box([0.3, y0, 0.0], [-0.1, vy0, 0.0], half, 1.5);
        box_b.pose.rot = Quat::exp(rot_vec);
        box_b.restitution = restitution;
        let floor = RigidBody::static_plane([0.0, -1.0, 0.0], -floor_y);

        let spec = WorldSpec {
            gravity: [0.0, 9.81, 0.0],
            substeps: 4,
            step_flicks: DEFAULT_STEP_FLICKS,
            bodies: vec![sphere, box_b, floor],
            constraints: Vec::new(),
            particles: None,
        };
        let mut state = spec.initial_state();
        for _ in 0..180 {
            step_world(&spec, &mut state);
            // Sphere bottom: trans.y + radius <= floor_y
            prop_assert!(
                state.bodies[0].pose.trans[1] + radius <= floor_y + 1e-6,
                "sphere penetrated floor: y={}, r={radius}, floor={floor_y}",
                state.bodies[0].pose.trans[1]
            );
            // Box lowest corner: max_corner_y <= floor_y
            let [ax, ay, az] = state.bodies[1].pose.rot.to_basis();
            let extent_y = ax[1].abs() * half[0] + ay[1].abs() * half[1] + az[1].abs() * half[2];
            prop_assert!(
                state.bodies[1].pose.trans[1] + extent_y <= floor_y + 1e-6,
                "box penetrated floor: y={}, extent_y={extent_y}, floor={floor_y}",
                state.bodies[1].pose.trans[1]
            );
        }
    }
}

#[test]
fn causal_invalidation_keeps_past_checkpoints_and_recomputes_future() {
    let spec = demo_world(42, 9.81, 0.6, 0.2);
    let mut timeline = WorldTimeline::new(spec.clone(), 10, 64);

    // Populate checkpoints across [0, 1.0s] (120 ticks -> checkpoints at 0, 10, 20, ..., 120).
    let t_before = Time::from_flicks(DEFAULT_STEP_FLICKS * 30);
    let t_edit = Time::from_flicks(DEFAULT_STEP_FLICKS * 50);
    let t_after = Time::from_flicks(DEFAULT_STEP_FLICKS * 90);
    let before_old = timeline.state_at(t_before);
    let after_old = timeline.state_at(t_after);

    // Edit gravity starting at t_edit = step 50.
    let mut edited_spec = spec;
    edited_spec.gravity = [0.0, 2.0, 0.0];
    timeline.update_spec(edited_spec, t_edit);

    // Seeking to t_before = step 30 must hit the preserved checkpoint with ZERO replay steps.
    let replays_before = timeline.replay_steps();
    let before_new = timeline.state_at(t_before);
    assert_eq!(before_new, before_old);
    assert_eq!(
        timeline.replay_steps(),
        replays_before,
        "pre-edit checkpoint must survive causal invalidation"
    );

    // Seeking to t_after must recompute under the new gravity and diverge from the old future.
    let after_new = timeline.state_at(t_after);
    assert_ne!(
        after_new.bodies[0].pose.trans,
        after_old.bodies[0].pose.trans
    );
    assert!(timeline.replay_steps() > replays_before);
}

#[test]
fn elastic_head_on_collision_conserves_momentum_and_energy() {
    let mut a = RigidBody::dynamic_sphere([-0.5, 0.0, 0.0], [2.0, 0.0, 0.0], 0.15, 1.0);
    a.restitution = 1.0;
    a.friction = 0.0;
    let mut b = RigidBody::dynamic_sphere([0.5, 0.0, 0.0], [-1.0, 0.0, 0.0], 0.15, 2.0);
    b.restitution = 1.0;
    b.friction = 0.0;

    let spec = WorldSpec {
        gravity: [0.0, 0.0, 0.0],
        substeps: 4,
        step_flicks: DEFAULT_STEP_FLICKS,
        bodies: vec![a, b],
        constraints: Vec::new(),
        particles: None,
    };
    let mut state = spec.initial_state();
    let p0 = state.total_momentum();
    let ke0 = state.kinetic_energy();

    for _ in 0..80 {
        step_world(&spec, &mut state);
    }

    // Confirm collision happened (a bounced left, b moved right).
    assert!(state.bodies[0].twist.v[0] < 0.0, "body a must rebound");
    assert!(state.bodies[1].twist.v[0] > 0.0, "body b must rebound");

    let p1 = state.total_momentum();
    let ke1 = state.kinetic_energy();
    for k in 0..3 {
        assert!(
            (p1[k] - p0[k]).abs() < 1e-9,
            "momentum axis {k}: before={}, after={}",
            p0[k],
            p1[k]
        );
    }
    assert!(
        (ke1 - ke0).abs() < 1e-4,
        "elastic kinetic energy must hold: before={ke0}, after={ke1}"
    );
}

#[test]
fn xpbd_pin_and_distance_constraints_hold_length() {
    let a = RigidBody::dynamic_sphere([0.6, 0.0, 0.0], [0.0, 0.0, 0.0], 0.08, 1.0);
    let b = RigidBody::dynamic_sphere([1.2, 0.0, 0.0], [0.0, 0.0, 0.0], 0.08, 1.0);
    let spec = WorldSpec {
        gravity: [0.0, 9.81, 0.0],
        substeps: 6,
        step_flicks: DEFAULT_STEP_FLICKS,
        bodies: vec![a, b],
        constraints: vec![
            Constraint::Pin {
                body: 0,
                anchor: [0.0, 0.0, 0.0],
                distance: 0.6,
                bilateral: true,
                compliance: 0.0,
            },
            Constraint::Distance {
                a: 0,
                b: 1,
                rest_length: 0.6,
                compliance: 0.0,
            },
        ],
        particles: None,
    };
    let mut state = spec.initial_state();
    for step in 0..240 {
        step_world(&spec, &mut state);
        let pa = state.bodies[0].pose.trans;
        let pb = state.bodies[1].pose.trans;
        let d_pin = (pa[0] * pa[0] + pa[1] * pa[1] + pa[2] * pa[2]).sqrt();
        let d_ab =
            ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2) + (pb[2] - pa[2]).powi(2)).sqrt();
        assert!(
            (d_pin - 0.6).abs() < 2e-3,
            "step {step}: pin length drifted to {d_pin}"
        );
        assert!(
            (d_ab - 0.6).abs() < 2e-3,
            "step {step}: link length drifted to {d_ab}"
        );
    }
}

#[test]
fn particle_floor_drag_and_turbulence_invariants() {
    let base = ParticleFieldSpec {
        rate: 50.0,
        life: 1.5,
        velocity: 100.0,
        vary: 0.0,
        direction_deg: 90.0, // straight down
        spread_deg: 0.0,
        gravity: 300.0,
        drag: 0.0,
        bounce: 0.8,
        floor_y: Some(200.0),
        turbulence: 0.0,
        turbulence_scale: 80.0,
        origin: [100.0, 50.0],
        area: [0.0, 0.0],
        seed: 7,
    };
    let with_drag = ParticleFieldSpec { drag: 2.5, ..base };
    let with_turb = ParticleFieldSpec {
        turbulence: 180.0,
        ..base
    };

    let t = Time::from_flicks(FLICKS_PER_SECOND);
    let s_base = reference_state_at(
        &WorldSpec {
            gravity: [0.0, 300.0, 0.0],
            particles: Some(base),
            ..WorldSpec::default()
        },
        t,
    );
    let s_drag = reference_state_at(
        &WorldSpec {
            gravity: [0.0, 300.0, 0.0],
            particles: Some(with_drag),
            ..WorldSpec::default()
        },
        t,
    );
    let s_turb = reference_state_at(
        &WorldSpec {
            gravity: [0.0, 300.0, 0.0],
            particles: Some(with_turb),
            ..WorldSpec::default()
        },
        t,
    );

    assert!(!s_base.particles.is_empty());
    for p in &s_base.particles {
        assert!(
            p.pos[1] <= 200.0 + 1e-9,
            "particle passed floor: {}",
            p.pos[1]
        );
        // Zero spread, zero turbulence -> strictly stays at x = 100.
        assert!((p.pos[0] - 100.0).abs() < 1e-9);
    }

    let avg_speed = |s: &kerf_sim::WorldState| {
        s.particles
            .iter()
            .map(|p| (p.vel[0].powi(2) + p.vel[1].powi(2)).sqrt())
            .sum::<f64>()
            / s.particles.len() as f64
    };
    assert!(
        avg_speed(&s_drag) < avg_speed(&s_base) * 0.7,
        "drag must reduce particle speed: drag={}, base={}",
        avg_speed(&s_drag),
        avg_speed(&s_base)
    );

    let max_dx = s_turb
        .particles
        .iter()
        .map(|p| (p.pos[0] - 100.0).abs())
        .fold(0.0f64, f64::max);
    assert!(
        max_dx > 5.0,
        "curl turbulence must displace particles laterally: max_dx={max_dx}"
    );
}

#[test]
fn director_shooting_solver_hits_target_under_gravity_and_drag() {
    let mut ball = RigidBody::dynamic_sphere([-1.2, -0.3, 0.4], [0.0; 3], 0.1, 1.0);
    ball.linear_damping = 0.65;
    let spec = WorldSpec {
        gravity: [0.0, 9.81, 0.0],
        substeps: 4,
        step_flicks: DEFAULT_STEP_FLICKS,
        bodies: vec![ball],
        constraints: Vec::new(),
        particles: None,
    };
    let goal = TrajectoryGoal {
        body: 0,
        target_pos: [1.3, 0.25, -0.5],
        arrive_at: Time::from_flicks(FLICKS_PER_SECOND * 4 / 5), // 0.8s
    };

    let sol = solve_launch_velocity(&spec, &goal, 25, 1e-5).expect("solution");
    assert!(
        sol.error < 1e-4,
        "shooting solver should hit target tightly: err={}, iters={}",
        sol.error,
        sol.iterations
    );

    // Independently replay through WorldTimeline and confirm arrival.
    let mut verified_spec = spec;
    verified_spec.bodies[0].twist.v = sol.initial_velocity;
    let mut timeline = WorldTimeline::new(verified_spec, 12, 32);
    let landed = timeline.state_at(goal.arrive_at).bodies[0].pose.trans;
    let dist = ((landed[0] - goal.target_pos[0]).powi(2)
        + (landed[1] - goal.target_pos[1]).powi(2)
        + (landed[2] - goal.target_pos[2]).powi(2))
    .sqrt();
    assert!(
        dist < 1e-4,
        "replayed arrival error {dist}, landed={landed:?}"
    );
}

#[test]
fn tracking_camera_keeps_dynamic_body_centered_through_kerf_camera() {
    let mut ball = RigidBody::dynamic_sphere([-0.8, -0.5, 0.0], [1.6, -1.2, 0.4], 0.1, 1.0);
    ball.restitution = 0.7;
    let floor = RigidBody::static_plane([0.0, -1.0, 0.0], -0.4);
    let spec = WorldSpec {
        gravity: [0.0, 4.0, 0.0],
        substeps: 4,
        step_flicks: DEFAULT_STEP_FLICKS,
        bodies: vec![ball, floor],
        constraints: Vec::new(),
        particles: None,
    };
    let mut timeline = WorldTimeline::new(spec, 12, 64);
    let times: Vec<Time> = (0..30)
        .map(|f| Time::from_flicks(f * FLICKS_PER_SECOND / 24))
        .collect();

    // Instant lock (smoothing = 1.0) must project target right onto screen center (960, 540).
    let locked = plan_tracking_camera(&mut timeline, 0, [0.0, -0.2, -2.5], &times, 1.0);
    assert_eq!(locked.len(), times.len());
    let focal = 540.0 / (45.0_f64.to_radians() / 2.0).tan();
    for (&t, pose) in times.iter().zip(&locked) {
        let target = timeline.state_at(t).bodies[0].pose.trans;
        let cam: Camera = pose.to_camera(focal, (960.0, 540.0), 1080.0);
        let screen = cam.on_screen(target).expect("target in front of camera");
        assert!(
            (screen.0 - 960.0).abs() < 1e-4 && (screen.1 - 540.0).abs() < 1e-4,
            "locked camera drifted off center at {:?}: screen={screen:?}",
            t
        );
    }

    // Smooth tracking (smoothing = 0.25) keeps poses unit-length and reduces step-to-step deltas.
    let smooth = plan_tracking_camera(&mut timeline, 0, [0.0, -0.2, -2.5], &times, 0.25);
    let step_motion = |poses: &[Se3]| -> f64 {
        poses
            .windows(2)
            .map(|w| {
                let d = w[0].inverse().compose(&w[1]).log();
                (d.v[0].powi(2) + d.v[1].powi(2) + d.v[2].powi(2)).sqrt()
            })
            .sum()
    };
    assert!(
        step_motion(&smooth) < step_motion(&locked),
        "smoothed camera path should have lower total twist motion"
    );
}
