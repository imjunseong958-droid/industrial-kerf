//! Deterministic fixed-step Symplectic + XPBD variational solver on $SE(3)$.

use crate::world::{
    Collider, Constraint, ParticleFieldSpec, RigidBody, SimParticle, WorldSpec, WorldState,
    MAX_SIM_PARTICLES,
};
use kerf_geom::lie::{Quat, Se3, Twist};
use kerf_model::Time;

type Vec3 = [f64; 3];

#[inline]
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

/// Deterministic SplitMix64 hash returning a float in `[0, 1)`.
fn rand01(seed: u64, id: u64, stream: u64) -> f64 {
    let mut z = seed
        .wrapping_add(id.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(stream.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth 2D value noise and its divergence-free curl for deterministic turbulence.
fn value_noise_2d(x: f64, y: f64, seed: u64) -> f64 {
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - ix as f64;
    let fy = y - iy as f64;
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fy * fy * (3.0 - 2.0 * fy);
    let h = |gx: i64, gy: i64| {
        let key = (gx as u64).wrapping_mul(0x517C_C1B7_2722_0A95)
            ^ (gy as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        rand01(seed, key, 7) * 2.0 - 1.0
    };
    let a = h(ix, iy);
    let b = h(ix + 1, iy);
    let c = h(ix, iy + 1);
    let d = h(ix + 1, iy + 1);
    let top = a + (b - a) * u;
    let bot = c + (d - c) * u;
    top + (bot - top) * v
}

fn curl_turbulence_2d(x: f64, y: f64, t: f64, scale_px: f64, seed: u64) -> [f64; 2] {
    let inv = 1.0 / scale_px.max(1.0);
    let sx = x * inv + t * 0.35;
    let sy = y * inv - t * 0.25;
    let eps = 0.05;
    let dpsi_dy =
        (value_noise_2d(sx, sy + eps, seed) - value_noise_2d(sx, sy - eps, seed)) / (2.0 * eps);
    let dpsi_dx =
        (value_noise_2d(sx + eps, sy, seed) - value_noise_2d(sx - eps, sy, seed)) / (2.0 * eps);
    [dpsi_dy, -dpsi_dx]
}

struct Contact {
    a: usize,
    b: usize,
    /// Unit normal pointing from `b` toward `a` (pushing `a` along `+normal`, `b` along `-normal`).
    normal: Vec3,
    penetration: f64,
    /// World contact point (used for angular momentum transfer).
    point: Vec3,
}

fn box_support_depth(
    body: &RigidBody,
    half: Vec3,
    plane_normal: Vec3,
    plane_offset: f64,
) -> (f64, Vec3) {
    let [ax, ay, az] = body.pose.rot.to_basis();
    let axes = [ax, ay, az];
    let mut deepest = body.pose.trans;
    for k in 0..3 {
        let proj = dot(axes[k], plane_normal);
        let sign = if proj > 1e-6 {
            -1.0
        } else if proj < -1e-6 {
            1.0
        } else {
            0.0
        };
        deepest = add(deepest, scale(axes[k], sign * half[k]));
    }
    let dist = dot(deepest, plane_normal) - plane_offset;
    (-dist, deepest)
}

fn detect_contacts(bodies: &[RigidBody]) -> Vec<Contact> {
    let mut contacts = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (a, b) = (&bodies[i], &bodies[j]);
            if !a.is_dynamic() && !b.is_dynamic() {
                continue;
            }
            match (a.collider, b.collider) {
                (Collider::Sphere { radius: ra }, Collider::Sphere { radius: rb }) => {
                    let delta = sub(a.pose.trans, b.pose.trans);
                    let dist = norm(delta);
                    let sum = ra + rb;
                    if dist < sum {
                        let normal = if dist > 1e-12 {
                            scale(delta, 1.0 / dist)
                        } else {
                            [0.0, -1.0, 0.0]
                        };
                        let point = add(b.pose.trans, scale(normal, rb));
                        contacts.push(Contact {
                            a: i,
                            b: j,
                            normal,
                            penetration: sum - dist,
                            point,
                        });
                    }
                }
                (Collider::Sphere { radius }, Collider::Plane { normal, offset }) => {
                    let dist = dot(a.pose.trans, normal) - offset;
                    if dist < radius {
                        let point = sub(a.pose.trans, scale(normal, dist));
                        contacts.push(Contact {
                            a: i,
                            b: j,
                            normal,
                            penetration: radius - dist,
                            point,
                        });
                    }
                }
                (Collider::Plane { normal, offset }, Collider::Sphere { radius }) => {
                    let dist = dot(b.pose.trans, normal) - offset;
                    if dist < radius {
                        let point = sub(b.pose.trans, scale(normal, dist));
                        contacts.push(Contact {
                            a: j,
                            b: i,
                            normal,
                            penetration: radius - dist,
                            point,
                        });
                    }
                }
                (Collider::Box { half_extents }, Collider::Plane { normal, offset }) => {
                    let (pen, point) = box_support_depth(a, half_extents, normal, offset);
                    if pen > 0.0 {
                        contacts.push(Contact {
                            a: i,
                            b: j,
                            normal,
                            penetration: pen,
                            point,
                        });
                    }
                }
                (Collider::Plane { normal, offset }, Collider::Box { half_extents }) => {
                    let (pen, point) = box_support_depth(b, half_extents, normal, offset);
                    if pen > 0.0 {
                        contacts.push(Contact {
                            a: j,
                            b: i,
                            normal,
                            penetration: pen,
                            point,
                        });
                    }
                }
                (Collider::Sphere { radius }, Collider::Box { half_extents }) => {
                    // Transform sphere center into box local frame.
                    let local = b.pose.inverse().apply_point(a.pose.trans);
                    let clamped = [
                        local[0].clamp(-half_extents[0], half_extents[0]),
                        local[1].clamp(-half_extents[1], half_extents[1]),
                        local[2].clamp(-half_extents[2], half_extents[2]),
                    ];
                    let world_closest = b.pose.apply_point(clamped);
                    let delta = sub(a.pose.trans, world_closest);
                    let dist = norm(delta);
                    if dist < radius {
                        let normal = if dist > 1e-12 {
                            scale(delta, 1.0 / dist)
                        } else {
                            [0.0, -1.0, 0.0]
                        };
                        contacts.push(Contact {
                            a: i,
                            b: j,
                            normal,
                            penetration: radius - dist,
                            point: world_closest,
                        });
                    }
                }
                (Collider::Box { half_extents }, Collider::Sphere { radius }) => {
                    let local = a.pose.inverse().apply_point(b.pose.trans);
                    let clamped = [
                        local[0].clamp(-half_extents[0], half_extents[0]),
                        local[1].clamp(-half_extents[1], half_extents[1]),
                        local[2].clamp(-half_extents[2], half_extents[2]),
                    ];
                    let world_closest = a.pose.apply_point(clamped);
                    let delta = sub(b.pose.trans, world_closest);
                    let dist = norm(delta);
                    if dist < radius {
                        let normal = if dist > 1e-12 {
                            scale(delta, 1.0 / dist)
                        } else {
                            [0.0, -1.0, 0.0]
                        };
                        contacts.push(Contact {
                            a: j,
                            b: i,
                            normal,
                            penetration: radius - dist,
                            point: world_closest,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    contacts
}

/// Effective rotational inverse inertia scalar for an impulse along `dir` at lever arm `r`.
fn rotational_term(body: &RigidBody, r: Vec3, dir: Vec3) -> f64 {
    if !body.is_dynamic() {
        return 0.0;
    }
    let rxn = cross(r, dir);
    let local = body.pose.rot.inverse().rotate_vec(rxn);
    local[0] * local[0] * body.inv_inertia[0]
        + local[1] * local[1] * body.inv_inertia[1]
        + local[2] * local[2] * body.inv_inertia[2]
}

fn apply_angular_impulse(body: &mut RigidBody, r: Vec3, impulse: Vec3) {
    if !body.is_dynamic() {
        return;
    }
    let torque = cross(r, impulse);
    let local_t = body.pose.rot.inverse().rotate_vec(torque);
    let local_dw = [
        local_t[0] * body.inv_inertia[0],
        local_t[1] * body.inv_inertia[1],
        local_t[2] * body.inv_inertia[2],
    ];
    let world_dw = body.pose.rot.rotate_vec(local_dw);
    body.twist.omega = add(body.twist.omega, world_dw);
}

fn project_constraints(constraints: &[Constraint], bodies: &mut [RigidBody], h: f64) {
    let inv_h2 = 1.0 / (h * h).max(1e-18);
    for constraint in constraints {
        match *constraint {
            Constraint::Distance {
                a,
                b,
                rest_length,
                compliance,
            } => {
                if a >= bodies.len() || b >= bodies.len() || a == b {
                    continue;
                }
                let wa = bodies[a].inv_mass;
                let wb = bodies[b].inv_mass;
                let w_sum = wa + wb;
                if w_sum <= 0.0 {
                    continue;
                }
                let delta = sub(bodies[b].pose.trans, bodies[a].pose.trans);
                let dist = norm(delta);
                if dist <= 1e-12 {
                    continue;
                }
                let c = dist - rest_length;
                let alpha_tilde = compliance.max(0.0) * inv_h2;
                let lambda = -c / (w_sum + alpha_tilde);
                let corr = scale(delta, lambda / dist);
                bodies[a].pose.trans = sub(bodies[a].pose.trans, scale(corr, wa));
                bodies[b].pose.trans = add(bodies[b].pose.trans, scale(corr, wb));
            }
            Constraint::Pin {
                body,
                anchor,
                distance,
                bilateral,
                compliance,
            } => {
                if body >= bodies.len() {
                    continue;
                }
                let w = bodies[body].inv_mass;
                if w <= 0.0 {
                    continue;
                }
                let delta = sub(bodies[body].pose.trans, anchor);
                let dist = norm(delta);
                if !bilateral && dist <= distance {
                    continue;
                }
                if dist <= 1e-12 {
                    continue;
                }
                let c = dist - distance;
                let alpha_tilde = compliance.max(0.0) * inv_h2;
                let lambda = -c / (w + alpha_tilde);
                let corr = scale(delta, lambda * w / dist);
                bodies[body].pose.trans = add(bodies[body].pose.trans, corr);
            }
        }
    }
}

fn collide_particle(
    p: &mut SimParticle,
    pf: &ParticleFieldSpec,
    bodies: &[RigidBody],
    bounce: f64,
) {
    // Floor collision (Kerf +y is downward).
    if let Some(floor_y) = pf.floor_y
        && p.pos[1] > floor_y
    {
        p.pos[1] = floor_y;
        if p.vel[1] > 0.0 {
            p.vel[1] = -p.vel[1] * bounce;
            p.vel[0] *= 1.0 - 0.15 * (1.0 - bounce);
        }
    }
    // World rigid-body sphere/plane collisions.
    for body in bodies {
        match body.collider {
            Collider::Plane { normal, offset } => {
                let dist = dot(p.pos, normal) - offset;
                if dist < 0.0 {
                    p.pos = sub(p.pos, scale(normal, dist));
                    let vn = dot(p.vel, normal);
                    if vn < 0.0 {
                        p.vel = sub(p.vel, scale(normal, (1.0 + bounce) * vn));
                    }
                }
            }
            Collider::Sphere { radius } => {
                let delta = sub(p.pos, body.pose.trans);
                let dist = norm(delta);
                if dist < radius && dist > 1e-12 {
                    let normal = scale(delta, 1.0 / dist);
                    p.pos = add(body.pose.trans, scale(normal, radius));
                    let vn = dot(p.vel, normal);
                    if vn < 0.0 {
                        p.vel = sub(p.vel, scale(normal, (1.0 + bounce) * vn));
                    }
                }
            }
            Collider::Box { .. } => {}
        }
    }
}

fn step_particles(
    pf: &ParticleFieldSpec,
    bodies: &[RigidBody],
    particles: &mut Vec<SimParticle>,
    next_id: &mut u64,
    t_prev: f64,
    t_now: f64,
    h: f64,
) {
    let drag_factor = (-pf.drag.max(0.0) * h).exp();
    let bounce = pf.bounce.clamp(0.0, 1.0);

    // 1. Integrate already-alive particles by one full substep `h` and compact expired ones.
    let mut write = 0;
    for read in 0..particles.len() {
        let mut p = particles[read];
        p.age += h;
        if p.age >= p.life {
            continue;
        }
        p.vel[1] += pf.gravity * h;
        if pf.turbulence > 0.0 {
            let curl = curl_turbulence_2d(p.pos[0], p.pos[1], t_now, pf.turbulence_scale, pf.seed);
            p.vel[0] += curl[0] * pf.turbulence * h;
            p.vel[1] += curl[1] * pf.turbulence * h;
        }
        p.vel[0] *= drag_factor;
        p.vel[1] *= drag_factor;
        p.vel[2] *= drag_factor;
        p.pos[0] += p.vel[0] * h;
        p.pos[1] += p.vel[1] * h;
        p.pos[2] += p.vel[2] * h;
        collide_particle(&mut p, pf, bodies, bounce);
        particles[write] = p;
        write += 1;
    }
    particles.truncate(write);

    // 2. Spawn particles whose deterministic birth time falls in [t_prev, t_now] and advance them
    //    strictly by their fractional age `dt = t_now - born` inside this substep.
    if pf.rate > 0.0 && pf.life > 0.0 {
        let rate = pf.rate.min(MAX_SIM_PARTICLES as f64 / pf.life.max(1e-6));
        let target_spawned = (t_now * rate).floor() as u64 + 1;
        while *next_id < target_spawned && particles.len() < MAX_SIM_PARTICLES {
            let id = *next_id;
            *next_id += 1;
            let born = id as f64 / rate;
            let age = (t_now - born).max(0.0);
            if age >= pf.life {
                continue;
            }
            let turn = rand01(pf.seed, id, 0) - 0.5;
            let angle = (pf.direction_deg + turn * pf.spread_deg).to_radians();
            let speed = pf.velocity * (1.0 + pf.vary * (rand01(pf.seed, id, 1) - 0.5) * 2.0);
            let mut pos = [
                pf.origin[0] + (rand01(pf.seed, id, 2) - 0.5) * pf.area[0],
                pf.origin[1] + (rand01(pf.seed, id, 3) - 0.5) * pf.area[1],
                0.0,
            ];
            let mut vel = [angle.cos() * speed, angle.sin() * speed, 0.0];
            let dt = (t_now - born.max(t_prev)).clamp(0.0, h);
            if dt > 0.0 {
                let frac_drag = (-pf.drag.max(0.0) * dt).exp();
                vel[1] += pf.gravity * dt;
                if pf.turbulence > 0.0 {
                    let curl =
                        curl_turbulence_2d(pos[0], pos[1], t_now, pf.turbulence_scale, pf.seed);
                    vel[0] += curl[0] * pf.turbulence * dt;
                    vel[1] += curl[1] * pf.turbulence * dt;
                }
                vel[0] *= frac_drag;
                vel[1] *= frac_drag;
                vel[2] *= frac_drag;
                pos[0] += vel[0] * dt;
                pos[1] += vel[1] * dt;
                pos[2] += vel[2] * dt;
            }
            let mut p = SimParticle {
                id,
                pos,
                vel,
                age,
                life: pf.life,
            };
            collide_particle(&mut p, pf, bodies, bounce);
            particles.push(p);
        }
    }
}

fn apply_contact_projection(bodies: &mut [RigidBody], contact: &Contact) {
    let wa = bodies[contact.a].inv_mass;
    let wb = bodies[contact.b].inv_mass;
    let w_sum = wa + wb;
    if w_sum <= 0.0 || contact.penetration <= 0.0 {
        return;
    }
    let corr = scale(contact.normal, contact.penetration / w_sum);
    bodies[contact.a].pose.trans = add(bodies[contact.a].pose.trans, scale(corr, wa));
    bodies[contact.b].pose.trans = sub(bodies[contact.b].pose.trans, scale(corr, wb));
}

/// Advance `state` by exactly ONE fixed tick (`spec.step_flicks`) using deterministic Symplectic
/// + XPBD substeps.
pub fn step_world(spec: &WorldSpec, state: &mut WorldState) {
    let substeps = spec.substeps.max(1);
    let dt_total = spec.dt_seconds();
    let h = dt_total / f64::from(substeps);
    let t_start = state.time.seconds();

    for sub_idx in 0..substeps {
        let t_sub_prev = t_start + f64::from(sub_idx) * h;
        let t_sub_now = t_start + f64::from(sub_idx + 1) * h;

        let prev_poses: Vec<Se3> = state.bodies.iter().map(|b| b.pose).collect();
        let prev_vel: Vec<Twist> = state.bodies.iter().map(|b| b.twist).collect();

        // 1. Semi-implicit symplectic prediction on SE(3).
        for body in &mut state.bodies {
            if !body.is_dynamic() {
                continue;
            }
            let lin_damp = (-body.linear_damping.max(0.0) * h).exp();
            let ang_damp = (-body.angular_damping.max(0.0) * h).exp();
            body.twist.v = scale(add(body.twist.v, scale(spec.gravity, h)), lin_damp);
            body.twist.omega = scale(body.twist.omega, ang_damp);
            body.pose.trans = add(body.pose.trans, scale(body.twist.v, h));
            let dq = Quat::exp(scale(body.twist.omega, h));
            body.pose.rot = (dq * body.pose.rot).normalized();
        }

        // 2 & 3. XPBD constraint and multi-contact non-penetration positional projection.
        // Resolve dynamic-dynamic contacts first, then re-detect and enforce static boundary
        // contacts so stacked bodies can never be pushed through a static plane.
        let mut active_contacts: Vec<Contact> = Vec::new();
        for pass in 0..2 {
            project_constraints(&spec.constraints, &mut state.bodies, h);
            let dyn_contacts = detect_contacts(&state.bodies);
            for contact in &dyn_contacts {
                if state.bodies[contact.a].is_dynamic() && state.bodies[contact.b].is_dynamic() {
                    apply_contact_projection(&mut state.bodies, contact);
                }
            }
            let static_contacts = detect_contacts(&state.bodies);
            for contact in &static_contacts {
                if !state.bodies[contact.a].is_dynamic() || !state.bodies[contact.b].is_dynamic() {
                    apply_contact_projection(&mut state.bodies, contact);
                }
            }
            if pass == 0 {
                for c in dyn_contacts {
                    if state.bodies[c.a].is_dynamic() && state.bodies[c.b].is_dynamic() {
                        active_contacts.push(c);
                    }
                }
                for sc in static_contacts {
                    if !state.bodies[sc.a].is_dynamic() || !state.bodies[sc.b].is_dynamic() {
                        active_contacts.push(sc);
                    }
                }
            }
        }
        let contacts = active_contacts;

        // 4. XPBD velocity recovery from pose deltas.
        let inv_h = 1.0 / h;
        for (idx, body) in state.bodies.iter_mut().enumerate() {
            if !body.is_dynamic() {
                continue;
            }
            body.twist.v = scale(sub(body.pose.trans, prev_poses[idx].trans), inv_h);
            let dq = body.pose.rot * prev_poses[idx].rot.inverse();
            body.twist.omega = scale(dq.log(), inv_h);
        }

        // 5. Restitution and Coulomb friction velocity impulses at active contacts.
        for contact in &contacts {
            let (ia, ib) = (contact.a, contact.b);
            let ra = sub(contact.point, state.bodies[ia].pose.trans);
            let rb = sub(contact.point, state.bodies[ib].pose.trans);
            let va_pre = add(prev_vel[ia].v, cross(prev_vel[ia].omega, ra));
            let vb_pre = add(prev_vel[ib].v, cross(prev_vel[ib].omega, rb));
            let vn_pre = dot(sub(va_pre, vb_pre), contact.normal);

            let va = add(
                state.bodies[ia].twist.v,
                cross(state.bodies[ia].twist.omega, ra),
            );
            let vb = add(
                state.bodies[ib].twist.v,
                cross(state.bodies[ib].twist.omega, rb),
            );
            let v_rel = sub(va, vb);
            let vn = dot(v_rel, contact.normal);

            let e = state.bodies[ia]
                .restitution
                .max(state.bodies[ib].restitution)
                .clamp(0.0, 1.0);
            // Suppress micro-bouncing below a small threshold.
            let target_vn = if vn_pre < -0.15 { -e * vn_pre } else { 0.0 };
            let denom = state.bodies[ia].inv_mass
                + state.bodies[ib].inv_mass
                + rotational_term(&state.bodies[ia], ra, contact.normal)
                + rotational_term(&state.bodies[ib], rb, contact.normal);
            if denom <= 1e-12 {
                continue;
            }
            let jn = ((target_vn - vn) / denom).max(0.0);
            let imp_n = scale(contact.normal, jn);
            let wa = state.bodies[ia].inv_mass;
            let wb = state.bodies[ib].inv_mass;
            state.bodies[ia].twist.v = add(state.bodies[ia].twist.v, scale(imp_n, wa));
            state.bodies[ib].twist.v = sub(state.bodies[ib].twist.v, scale(imp_n, wb));
            apply_angular_impulse(&mut state.bodies[ia], ra, imp_n);
            apply_angular_impulse(&mut state.bodies[ib], rb, scale(imp_n, -1.0));

            // Tangential Coulomb friction.
            let vt_vec = sub(v_rel, scale(contact.normal, vn));
            let vt_len = norm(vt_vec);
            if vt_len > 1e-9 && jn > 0.0 {
                let t_dir = scale(vt_vec, 1.0 / vt_len);
                let mu = (state.bodies[ia].friction * state.bodies[ib].friction).sqrt();
                let denom_t = wa
                    + wb
                    + rotational_term(&state.bodies[ia], ra, t_dir)
                    + rotational_term(&state.bodies[ib], rb, t_dir);
                let jt = (-vt_len / denom_t.max(1e-12)).clamp(-mu * jn, mu * jn);
                let imp_t = scale(t_dir, jt);
                state.bodies[ia].twist.v = add(state.bodies[ia].twist.v, scale(imp_t, wa));
                state.bodies[ib].twist.v = sub(state.bodies[ib].twist.v, scale(imp_t, wb));
                apply_angular_impulse(&mut state.bodies[ia], ra, imp_t);
                apply_angular_impulse(&mut state.bodies[ib], rb, scale(imp_t, -1.0));
            }
        }

        // 6. Step deterministic particle field if present.
        if let Some(pf) = &spec.particles {
            step_particles(
                pf,
                &state.bodies,
                &mut state.particles,
                &mut state.next_particle_id,
                t_sub_prev,
                t_sub_now,
                h,
            );
        }
    }

    state.step += 1;
    state.time = Time::from_flicks(state.step as i64 * spec.step_flicks);
}

/// Geodesic $SE(3)$ interpolation between two consecutive tick states `a` and `b` at `alpha ∈ [0, 1]`.
pub fn interpolate_state(a: &WorldState, b: &WorldState, alpha: f64) -> WorldState {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return a.clone();
    }
    if alpha >= 1.0 {
        return b.clone();
    }
    let bodies = a
        .bodies
        .iter()
        .zip(&b.bodies)
        .map(|(ba, bb)| RigidBody {
            pose: ba.pose.interpolate(&bb.pose, alpha),
            twist: Twist::new(
                add(
                    ba.twist.omega,
                    scale(sub(bb.twist.omega, ba.twist.omega), alpha),
                ),
                add(ba.twist.v, scale(sub(bb.twist.v, ba.twist.v), alpha)),
            ),
            ..*ba
        })
        .collect();
    let particles = a
        .particles
        .iter()
        .map(|pa| {
            if let Some(pb) = b.particles.iter().find(|pb| pb.id == pa.id) {
                SimParticle {
                    id: pa.id,
                    pos: add(pa.pos, scale(sub(pb.pos, pa.pos), alpha)),
                    vel: add(pa.vel, scale(sub(pb.vel, pa.vel), alpha)),
                    age: pa.age + (pb.age - pa.age) * alpha,
                    life: pa.life,
                }
            } else {
                *pa
            }
        })
        .collect();
    let flicks =
        a.time.flicks() + ((b.time.flicks() - a.time.flicks()) as f64 * alpha).round() as i64;
    WorldState {
        step: a.step,
        time: Time::from_flicks(flicks),
        bodies,
        particles,
        next_particle_id: b.next_particle_id,
    }
}
