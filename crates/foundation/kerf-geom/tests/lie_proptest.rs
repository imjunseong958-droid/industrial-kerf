//! Property tests for `kerf_geom::lie` ($SO(3)$, $SE(3)$, $\mathfrak{se}(3)$ Twist, Adjoint,
//! geodesic interpolation, `Camera`/`Lens`/`Affine3` bridges, analytical optical flow, and
//! `Ray3`/`Plane3` geometry).

use kerf_geom::lie::{screen_velocity, Plane3, Quat, Ray3, Se3, Twist};
use kerf_geom::place::{Camera, Lens};
use proptest::prelude::*;

fn small_vec(bound: f64) -> impl Strategy<Value = [f64; 3]> {
    [-bound..bound, -bound..bound, -bound..bound]
}

fn twist_strategy() -> impl Strategy<Value = Twist> {
    // Keep ||omega|| < 2.8 rad so log(exp(omega)) stays strictly inside the unique cut locus (< pi).
    (small_vec(1.5), small_vec(50.0)).prop_map(|(omega, v)| Twist::new(omega, v))
}

fn se3_strategy() -> impl Strategy<Value = Se3> {
    twist_strategy().prop_map(Se3::exp)
}

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps * (1.0 + a.abs().max(b.abs()))
}

fn vec_close(a: [f64; 3], b: [f64; 3], eps: f64) -> bool {
    (0..3).all(|i| close(a[i], b[i], eps))
}

fn se3_close(a: &Se3, b: &Se3, eps: f64) -> bool {
    let dot_q =
        (a.rot.w * b.rot.w + a.rot.x * b.rot.x + a.rot.y * b.rot.y + a.rot.z * b.rot.z).abs();
    (1.0 - dot_q) <= eps && vec_close(a.trans, b.trans, eps)
}

proptest! {
    #[test]
    fn se3_exp_log_round_trip(xi in twist_strategy()) {
        let t = Se3::exp(xi);
        let recovered = t.log();
        prop_assert!(
            vec_close(xi.omega, recovered.omega, 1e-9),
            "omega mismatch: {xi:?} vs {recovered:?}"
        );
        prop_assert!(
            vec_close(xi.v, recovered.v, 1e-9),
            "v mismatch: {xi:?} vs {recovered:?}"
        );
        let t2 = Se3::exp(recovered);
        prop_assert!(se3_close(&t, &t2, 1e-10));
    }

    #[test]
    fn se3_group_axioms_and_affine3_bridge(a in se3_strategy(), b in se3_strategy(), p in small_vec(100.0)) {
        // Composition matches sequential point application.
        let ab = a.compose(&b);
        prop_assert!(vec_close(ab.apply_point(p), a.apply_point(b.apply_point(p)), 1e-9));

        // Inverse recovers identity.
        let id = a.compose(&a.inverse());
        prop_assert!(se3_close(&id, &Se3::IDENTITY, 1e-10));
        prop_assert!(vec_close(a.inverse().apply_point(a.apply_point(p)), p, 1e-9));

        // Lossless Affine3 round-trip.
        let aff = ab.to_affine3();
        prop_assert!(vec_close(aff.apply(p), ab.apply_point(p), 1e-9));
        let back = Se3::try_from_affine3(&aff).expect("rigid affine3 converts back to Se3");
        prop_assert!(se3_close(&ab, &back, 1e-9));
    }

    #[test]
    fn adjoint_homomorphism_and_conjugation(a in se3_strategy(), b in se3_strategy(), xi in twist_strategy()) {
        // Homomorphism: Ad_{AB}(xi) == Ad_A(Ad_B(xi)).
        let lhs = a.compose(&b).adjoint(xi);
        let rhs = a.adjoint(b.adjoint(xi));
        prop_assert!(vec_close(lhs.omega, rhs.omega, 1e-9));
        prop_assert!(vec_close(lhs.v, rhs.v, 1e-9));

        // Conjugation: A * exp(xi) * A^{-1} == exp(Ad_A(xi)) (scale xi down to stay inside cut locus).
        let small = xi.scale(0.15);
        let conj = a.compose(&Se3::exp(small)).compose(&a.inverse());
        let adj_exp = Se3::exp(a.adjoint(small));
        prop_assert!(se3_close(&conj, &adj_exp, 1e-9));

        // Matrix form matches functional form.
        let m = a.adjoint_matrix();
        let x = [xi.omega[0], xi.omega[1], xi.omega[2], xi.v[0], xi.v[1], xi.v[2]];
        let mut y = [0.0; 6];
        for r in 0..6 {
            for c in 0..6 {
                y[r] += m[r][c] * x[c];
            }
        }
        let direct = a.adjoint(xi);
        prop_assert!(vec_close([y[0], y[1], y[2]], direct.omega, 1e-9));
        prop_assert!(vec_close([y[3], y[4], y[5]], direct.v, 1e-9));
    }

    #[test]
    fn geodesic_interpolation_is_left_invariant_with_constant_twist(
        a in se3_strategy(),
        b in se3_strategy(),
        g in se3_strategy(),
        t in 0.0f64..=1.0,
    ) {
        prop_assert!(se3_close(&a.interpolate(&b, 0.0), &a, 1e-10));
        prop_assert!(se3_close(&a.interpolate(&b, 1.0), &b, 1e-10));
        // Left invariance: g * interp(a, b, t) == interp(g * a, g * b, t).
        let left = g.compose(&a.interpolate(&b, t));
        let right = g.compose(&a).interpolate(&g.compose(&b), t);
        prop_assert!(se3_close(&left, &right, 1e-8));
    }

    #[test]
    fn lens_and_camera_round_trip(
        ax in -2.0f64..2.0,
        ay in -2.0f64..2.0,
        az in -2.0f64..2.0,
        tilt in -75.0f64..75.0,
        pan in -170.0f64..170.0,
        roll in -170.0f64..170.0,
        fov in 20.0f64..110.0,
    ) {
        let size = (1920.0, 1080.0);
        let lens = Lens {
            at: [ax, ay, az],
            tilt,
            pan,
            roll,
            fov,
        };
        let cam = Camera::of(&lens, size, None);
        let pose = Se3::from_camera(&cam);
        let from_lens = Se3::from_lens(&lens, size);
        prop_assert!(se3_close(&pose, &from_lens, 1e-10));
        let back_lens = pose.to_lens(size, fov);
        let back_cam = Camera::of(&back_lens, size, None);
        prop_assert!(vec_close(cam.eye, back_cam.eye, 1e-8));
        prop_assert!(vec_close(cam.right, back_cam.right, 1e-8));
        prop_assert!(vec_close(cam.down, back_cam.down, 1e-8));
        prop_assert!(vec_close(cam.forward, back_cam.forward, 1e-8));
    }

    #[test]
    fn analytical_screen_velocity_matches_central_finite_differences(
        cam_twist in (small_vec(0.4), small_vec(40.0)).prop_map(|(w, v)| Twist::new(w, v)),
        obj_twist in (small_vec(0.4), small_vec(40.0)).prop_map(|(w, v)| Twist::new(w, v)),
        tilt in -30.0f64..30.0,
        pan in -30.0f64..30.0,
        roll in -30.0f64..30.0,
        local_x in -300.0f64..300.0,
        local_y in -200.0f64..200.0,
        local_z in 400.0f64..2000.0,
    ) {
        let size = (1280.0, 720.0);
        let lens = Lens {
            at: [0.05, -0.04, 0.1],
            tilt,
            pan,
            roll,
            fov: 50.0,
        };
        let cam = Camera::of(&lens, size, None);
        let cam_pose = Se3::from_camera(&cam);
        let world_p = cam_pose.apply_point([local_x, local_y, local_z]);
        let world_v = obj_twist.point_velocity(world_p);
        let (du, dv) = screen_velocity(&cam, cam_twist, world_p, world_v)
            .expect("point in front of camera projects");

        let dt = 1e-6;
        let eval_at = |h: f64| {
            let pose_h = Se3::exp(cam_twist.scale(h)).compose(&cam_pose);
            let cam_h = pose_h.to_camera(cam.focal, cam.center, cam.height);
            let p_h = Se3::exp(obj_twist.scale(h)).apply_point(world_p);
            cam_h.on_screen(p_h).expect("perturbed point projects")
        };
        let (u_plus, v_plus) = eval_at(dt);
        let (u_minus, v_minus) = eval_at(-dt);
        let fd_u = (u_plus - u_minus) / (2.0 * dt);
        let fd_v = (v_plus - v_minus) / (2.0 * dt);
        prop_assert!(
            close(du, fd_u, 1e-5) && close(dv, fd_v, 1e-5),
            "screen_velocity ({du}, {dv}) vs fd ({fd_u}, {fd_v})"
        );
    }
}

#[test]
fn quat_slerp_and_ray_plane_geometry_invariants() {
    let q0 = Quat::IDENTITY;
    let q1 = Quat::from_axis_angle([0.0, 1.0, 0.0], std::f64::consts::FRAC_PI_2);
    let mid = q0.slerp(q1, 0.5);
    let expected = Quat::from_axis_angle([0.0, 1.0, 0.0], std::f64::consts::FRAC_PI_4);
    assert!(vec_close(
        mid.rotate_vec([1.0, 0.0, 0.0]),
        expected.rotate_vec([1.0, 0.0, 0.0]),
        1e-12
    ));

    // Ray from camera pixel hits z=0 plane at the exact pixel world coordinate.
    let cam = Camera::of(&Lens::RESTING, (1920.0, 1080.0), None);
    let plane_z0 = Plane3::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).expect("valid plane");
    let ray = Ray3::from_camera_pixel(&cam, (1200.0, 300.0));
    let (t, hit) = ray.intersect_plane(&plane_z0).expect("hits z=0");
    assert!(t > 0.0);
    let (sx, sy) = cam.on_screen(hit).expect("projects back");
    assert!(close(sx, 1200.0, 1e-9) && close(sy, 300.0, 1e-9));

    // Plane-plane intersection line lies in both planes.
    let p1 = Plane3::new([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]).expect("p1");
    let p2 = Plane3::new([1.0, 2.0, 3.0], [1.0, 1.0, 0.0]).expect("p2");
    let line = p1.intersect_plane(&p2).expect("planes intersect");
    for s in [-10.0, 0.0, 7.5] {
        let pt = line.at(s);
        assert!(p1.signed_distance(pt).abs() < 1e-11);
        assert!(p2.signed_distance(pt).abs() < 1e-11);
    }
}
