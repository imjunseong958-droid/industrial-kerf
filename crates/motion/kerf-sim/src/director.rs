//! Inverse Director Solver: solves agent-declared spatiotemporal goals over the real XPBD
//! simulator and plans smooth $SE(3)$ camera trajectories tracking simulated entities.

use crate::cache::WorldTimeline;
use crate::solver::step_world;
use crate::world::WorldSpec;
use kerf_geom::lie::{Quat, Se3};
use kerf_model::Time;

/// A spatiotemporal arrival goal: body `body` should reach `target_pos` at `arrive_at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrajectoryGoal {
    pub body: usize,
    pub target_pos: [f64; 3],
    pub arrive_at: Time,
}

/// Solution returned by [`solve_launch_velocity`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaunchSolution {
    pub initial_velocity: [f64; 3],
    pub final_pos: [f64; 3],
    pub error: f64,
    pub iterations: usize,
}

fn simulate_body_at(spec: &WorldSpec, body_idx: usize, v0: [f64; 3], arrive_at: Time) -> [f64; 3] {
    let mut local_spec = spec.clone();
    local_spec.bodies[body_idx].twist.v = v0;
    let mut state = local_spec.initial_state();
    let step_flicks = local_spec.step_flicks.max(1);
    let target_step = (arrive_at.flicks().max(0) / step_flicks) as u64;
    while state.step < target_step {
        step_world(&local_spec, &mut state);
    }
    state.bodies[body_idx].pose.trans
}

/// Solve a 3×3 linear system `A x = b` via Gaussian elimination with partial pivoting.
fn solve_3x3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for (row, r_vals) in a.iter().enumerate().skip(col + 1) {
            if r_vals[col].abs() > best {
                best = r_vals[col].abs();
                pivot = row;
            }
        }
        if best < 1e-12 {
            return None;
        }
        if pivot != col {
            a.swap(col, pivot);
            b.swap(col, pivot);
        }
        let diag = a[col][col];
        for row in (col + 1)..3 {
            let factor = a[row][col] / diag;
            for k in col..3 {
                a[row][k] -= factor * a[col][k];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0; 3];
    for i in (0..3).rev() {
        let mut sum = b[i];
        for k in (i + 1)..3 {
            sum -= a[i][k] * x[k];
        }
        x[i] = sum / a[i][i];
    }
    Some(x)
}

/// Solve for the initial linear velocity `v0` of `goal.body` so that under the full XPBD world
/// dynamics (gravity, drag, constraints, and collisions) it arrives at `goal.target_pos` at
/// `goal.arrive_at`. Uses Levenberg-Marquardt damped shooting over the exact simulator.
pub fn solve_launch_velocity(
    spec: &WorldSpec,
    goal: &TrajectoryGoal,
    max_iters: usize,
    tol: f64,
) -> Option<LaunchSolution> {
    if goal.body >= spec.bodies.len() || !spec.bodies[goal.body].is_dynamic() {
        return None;
    }
    let dt = goal.arrive_at.seconds().max(spec.dt_seconds());
    let p0 = spec.bodies[goal.body].pose.trans;
    // Closed-form ballistic guess under constant gravity as warm start.
    let mut v = [
        (goal.target_pos[0] - p0[0]) / dt - 0.5 * spec.gravity[0] * dt,
        (goal.target_pos[1] - p0[1]) / dt - 0.5 * spec.gravity[1] * dt,
        (goal.target_pos[2] - p0[2]) / dt - 0.5 * spec.gravity[2] * dt,
    ];
    let mut damping = 1e-3;
    let eps = 1e-4;
    let mut best_pos = simulate_body_at(spec, goal.body, v, goal.arrive_at);
    let residual = |pos: [f64; 3]| {
        [
            pos[0] - goal.target_pos[0],
            pos[1] - goal.target_pos[1],
            pos[2] - goal.target_pos[2],
        ]
    };
    let err_norm = |r: [f64; 3]| (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    let mut r = residual(best_pos);
    let mut best_err = err_norm(r);

    for iter in 0..max_iters {
        if best_err <= tol {
            return Some(LaunchSolution {
                initial_velocity: v,
                final_pos: best_pos,
                error: best_err,
                iterations: iter,
            });
        }
        // Central-difference 3×3 Jacobian of terminal position w.r.t. initial velocity.
        let mut jac = [[0.0; 3]; 3];
        for col in 0..3 {
            let mut vp = v;
            let mut vm = v;
            vp[col] += eps;
            vm[col] -= eps;
            let pp = simulate_body_at(spec, goal.body, vp, goal.arrive_at);
            let pm = simulate_body_at(spec, goal.body, vm, goal.arrive_at);
            for row in 0..3 {
                jac[row][col] = (pp[row] - pm[row]) / (2.0 * eps);
            }
        }
        // Normal equations: (J^T J + λ I) Δv = -J^T r.
        let mut jtj = [[0.0; 3]; 3];
        let mut neg_jtr = [0.0; 3];
        for i in 0..3 {
            for j in 0..3 {
                let mut s = 0.0;
                for row in &jac {
                    s += row[i] * row[j];
                }
                jtj[i][j] = s;
            }
            jtj[i][i] += damping;
            let mut g = 0.0;
            for (row, rk) in jac.iter().zip(r) {
                g += row[i] * rk;
            }
            neg_jtr[i] = -g;
        }
        let Some(dv) = solve_3x3(jtj, neg_jtr) else {
            damping *= 10.0;
            continue;
        };
        let cand_v = [v[0] + dv[0], v[1] + dv[1], v[2] + dv[2]];
        let cand_pos = simulate_body_at(spec, goal.body, cand_v, goal.arrive_at);
        let cand_r = residual(cand_pos);
        let cand_err = err_norm(cand_r);
        if cand_err < best_err {
            v = cand_v;
            best_pos = cand_pos;
            r = cand_r;
            best_err = cand_err;
            damping = (damping * 0.3).max(1e-9);
        } else {
            damping *= 5.0;
        }
    }

    Some(LaunchSolution {
        initial_velocity: v,
        final_pos: best_pos,
        error: best_err,
        iterations: max_iters,
    })
}

/// Build an orthonormal right-handed Kerf camera pose (`x` right, `y` down, `z` forward) at `eye`
/// looking directly at `target`.
fn look_at_se3(eye: [f64; 3], target: [f64; 3]) -> Se3 {
    let f = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let f_len = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
    if f_len <= 1e-12 {
        return Se3::from_translation(eye);
    }
    let forward = [f[0] / f_len, f[1] / f_len, f[2] / f_len];
    // World down is +y in Kerf coordinates; right = down × forward (since x × y = z => y × z = x).
    let world_down = [0.0, 1.0, 0.0];
    let mut right = [
        world_down[1] * forward[2] - world_down[2] * forward[1],
        world_down[2] * forward[0] - world_down[0] * forward[2],
        world_down[0] * forward[1] - world_down[1] * forward[0],
    ];
    let r_len = (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt();
    if r_len <= 1e-9 {
        right = [1.0, 0.0, 0.0];
    } else {
        right = [right[0] / r_len, right[1] / r_len, right[2] / r_len];
    }
    // down = forward × right (since z × x = y).
    let down = [
        forward[1] * right[2] - forward[2] * right[1],
        forward[2] * right[0] - forward[0] * right[2],
        forward[0] * right[1] - forward[1] * right[0],
    ];
    Se3::new(Quat::from_basis(right, down, forward), eye)
}

/// Plan a smooth gimbal-lock-free $SE(3)$ camera trajectory tracking `target_body` across `times`,
/// positioned at `target + offset_world` and smoothed via geodesic [`Se3::interpolate`] with factor
/// `smoothing ∈ (0, 1]` (`1.0` = instant lock, smaller = smoother damped tracking).
pub fn plan_tracking_camera(
    timeline: &mut WorldTimeline,
    target_body: usize,
    offset_world: [f64; 3],
    times: &[Time],
    smoothing: f64,
) -> Vec<Se3> {
    let alpha = smoothing.clamp(1e-3, 1.0);
    let mut poses = Vec::with_capacity(times.len());
    for &t in times {
        let state = timeline.state_at(t);
        let Some(body) = state.bodies.get(target_body) else {
            poses.push(Se3::IDENTITY);
            continue;
        };
        let target = body.pose.trans;
        let eye = [
            target[0] + offset_world[0],
            target[1] + offset_world[1],
            target[2] + offset_world[2],
        ];
        let desired = look_at_se3(eye, target);
        let pose = match poses.last() {
            Some(prev) => prev.interpolate(&desired, alpha),
            None => desired,
        };
        poses.push(pose);
    }
    poses
}
