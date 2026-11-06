//! Shots whose camera is known: points projected straight into paths, with a known noise, and pictures
//! of a scene filmed by that camera, through the real tracker.

use super::follow::Path;
use super::math::*;
use super::scene::{self, Shot};
use super::views::Draws;
use super::*;

const W: f64 = 1280.0;
const H: f64 = 720.0;

/// What points look like through `shots`: a path ends when its point leaves the frame.
fn paths_of(points: &[Vector], shots: &[Shot], noise: f64, seed: u64) -> Followed {
    let mut draws = Draws::new(seed);
    let mut paths = Vec::new();
    for point in points {
        let mut current: Option<Path> = None;
        for (frame, shot) in shots.iter().enumerate() {
            let shown = shot
                .shows(*point)
                .map(|at| [at[0] + W / 2.0, at[1] + H / 2.0])
                .filter(|at| at[0] >= 2.0 && at[1] >= 2.0 && at[0] < W - 2.0 && at[1] < H - 2.0);
            match shown {
                Some(at) => {
                    // A sum of uniforms is near enough to Gaussian, of deviation `noise`.
                    let mut wander = |_: f64| (0..12).map(|_| draws.part()).sum::<f64>() * noise;
                    let at = [at[0] + wander(0.0), at[1] + wander(0.0)];
                    match &mut current {
                        Some(path) => path.at.push(at),
                        None => {
                            current = Some(Path {
                                start: frame,
                                at: vec![at],
                            })
                        }
                    }
                }
                None => {
                    if let Some(path) = current.take() {
                        paths.push(path);
                    }
                }
            }
        }
        if let Some(path) = current.take() {
            paths.push(path);
        }
    }
    paths.retain(|path| path.at.len() >= 3);
    Followed {
        width: W,
        height: H,
        frames: shots.len(),
        paths,
        ended_at: None,
    }
}

fn scattered(count: usize, seed: u64, middle: Vector, span: Vector) -> Vec<Vector> {
    let mut draws = Draws::new(seed);
    (0..count)
        .map(|_| {
            [
                middle[0] + draws.part() * span[0],
                middle[1] + draws.part() * span[1],
                middle[2] + draws.part() * span[2],
            ]
        })
        .collect()
}

/// A camera going sideways and forward, turning to keep the scene in view, with a little roll.
fn moving(frames: usize, focal: f64) -> Vec<Shot> {
    (0..frames)
        .map(|frame| {
            let t = frame as f64 / (frames - 1) as f64;
            let place = [
                -3.0 + 6.0 * t,
                -0.6 + 0.8 * t + 0.5 * (std::f64::consts::PI * t).sin(),
                -1.0 + 2.0 * t - 1.5 * (t - 0.5) * (t - 0.5),
            ];
            let target = [0.3 * t, 0.2, 10.0];
            Shot::looking(place, target, 0.05 * (t * 6.0).sin(), focal)
        })
        .collect()
}

/// How far the solved way is from the true one, as a share of how far the camera went, once scaled:
/// the solve cannot know how big the world is. And the worst it is turned off, in degrees.
fn way_off(shot: &SolvedShot, truth: &[Shot]) -> (f64, f64) {
    let first = &truth[shot.cameras[0].frame];
    let (mut solved, mut known) = (Vec::new(), Vec::new());
    let start = &shot.cameras[0];
    for camera in &shot.cameras {
        solved.push([
            (camera.x - start.x) * shot.width,
            (camera.y - start.y) * shot.height,
            (camera.z - start.z) * shot.height,
        ]);
        known.push(apply(
            &first.turn,
            minus(truth[camera.frame].place, first.place),
        ));
    }
    let across: f64 = solved.iter().zip(&known).map(|(s, k)| dot(*s, *k)).sum();
    let along: f64 = known.iter().map(|k| dot(*k, *k)).sum();
    let scale = across / along.max(1e-12);
    let squares: f64 = solved
        .iter()
        .zip(&known)
        .map(|(s, k)| {
            let off = minus(*s, times(*k, scale));
            dot(off, off)
        })
        .sum();
    let rms = (squares / solved.len() as f64).sqrt();
    let went = known
        .iter()
        .map(|k| length(*k) * scale.abs())
        .fold(0.0, f64::max);
    let mut worst = 0.0f64;
    for camera in &shot.cameras {
        let found = turn_of(camera);
        let true_turn = product(&truth[camera.frame].turn, &transposed(&first.turn));
        let off = length(axis_of(&product(&found, &transposed(&true_turn))));
        worst = worst.max(off.to_degrees());
    }
    (rms / went.max(1e-12), worst)
}

fn fov_of(focal: f64, height: f64) -> f64 {
    (2.0 * (height / 2.0 / focal).atan()).to_degrees()
}

#[test]
fn paths_without_noise_give_the_camera_and_nothing_off() {
    let shots = moving(40, 1100.0);
    let points = scattered(400, 1, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let shot = solve_followed(&paths_of(&points, &shots, 0.0, 2), None).expect("solves");
    assert_eq!(shot.cameras.len(), 40);
    assert!(shot.lens_known);
    assert!(shot.error < 0.01, "{}", shot.error);
    assert!(
        (shot.fov - fov_of(1100.0, H)).abs() < 0.05,
        "{} not {}",
        shot.fov,
        fov_of(1100.0, H)
    );
    let (way, turned) = way_off(&shot, &shots);
    assert!(way < 0.001, "{way}");
    assert!(turned < 0.01, "{turned}");
    // Its first frame is the 2D picture.
    let first = shot.cameras[0];
    assert!(first.x.abs() < 1e-9 && first.y.abs() < 1e-9);
    assert!(first.pan.abs() < 1e-9 && first.tilt.abs() < 1e-9 && first.roll.abs() < 1e-9);
    let rest = focal_of_fov(H, REST_FOV);
    let focal = focal_of_fov(H, shot.fov);
    assert!((first.z - (rest - focal) / H).abs() < 1e-9, "{}", first.z);
}

#[test]
fn known_noise_comes_back_as_the_error_it_should_be() {
    let shots = moving(40, 1100.0);
    let points = scattered(400, 3, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    // The mean distance of 2D Gaussian noise of deviation s is 1.2533 s, less what the fit takes up.
    let deviation = 0.2;
    let shot = solve_followed(&paths_of(&points, &shots, deviation, 4), None).expect("solves");
    let expected = 1.2533 * deviation;
    assert!(
        (shot.error - expected).abs() < 0.03,
        "{} not {expected}",
        shot.error
    );
    assert!((shot.fov - fov_of(1100.0, H)).abs() < 0.4, "{}", shot.fov);
    let (way, turned) = way_off(&shot, &shots);
    assert!(way < 0.01, "{way}");
    assert!(turned < 0.2, "{turned}");
}

#[test]
fn a_flat_scene_solves_from_its_homography() {
    let focal = 1000.0;
    let shots: Vec<Shot> = (0..40)
        .map(|frame| {
            let t = frame as f64 / 39.0;
            let place = [
                -2.0 + 4.0 * t,
                -0.3 * (std::f64::consts::PI * t).sin(),
                2.0 * t,
            ];
            let way = unit([0.1 - 0.2 * t, 0.45, 1.0]);
            Shot::looking(place, add(place, way), 0.0, focal)
        })
        .collect();
    let mut draws = Draws::new(11);
    let floor: Vec<Vector> = (0..500)
        .map(|_| [draws.part() * 20.0, 2.0, 12.0 + draws.part() * 20.0])
        .collect();
    let shot = solve_followed(&paths_of(&floor, &shots, 0.1, 12), None).expect("solves");
    assert!(shot.error < 0.5, "{}", shot.error);
    let (way, _) = way_off(&shot, &shots);
    assert!(way < 0.02, "{way}");
}

#[test]
fn a_camera_that_only_turns_is_said_to() {
    let shots: Vec<Shot> = (0..40)
        .map(|frame| {
            let a = -0.4 + 0.8 * frame as f64 / 39.0;
            let target = [10.0 * a.sin(), 0.2 * a, 10.0 * a.cos()];
            Shot::looking([0.0; 3], target, 0.02, 1200.0)
        })
        .collect();
    let around: Vec<Vector> = scattered(900, 9, [0.0, 0.0, 30.0], [60.0, 20.0, 50.0]);
    assert_eq!(
        solve_followed(&paths_of(&around, &shots, 0.1, 10), None),
        Err(Unsolved::OnlyTurns)
    );
}

#[test]
fn a_long_pan_is_not_taken_for_a_move_by_the_trackers_wander() {
    let count = 480;
    let shots: Vec<Shot> = (0..count)
        .map(|frame| {
            let a = -0.35 + 0.7 * frame as f64 / (count - 1) as f64;
            Shot::looking([0.0; 3], [10.0 * a.sin(), 0.0, 10.0 * a.cos()], 0.0, 760.0)
        })
        .collect();
    let around: Vec<Vector> = scattered(900, 15, [0.0, 0.0, 30.0], [60.0, 20.0, 50.0]);
    let mut followed = paths_of(&around, &shots, 0.05, 16);
    // Each path lags the way the picture goes by a little more each frame, as a tracker's do, some
    // more than others.
    let mut draws = Draws::new(17);
    for path in &mut followed.paths {
        let rate = 0.0047 * (1.0 + draws.part());
        for (step, at) in path.at.iter_mut().enumerate() {
            at[0] += rate * step as f64;
        }
    }
    assert_eq!(solve_followed(&followed, None), Err(Unsolved::OnlyTurns));
}

#[test]
fn a_camera_that_holds_still_is_said_to() {
    let shots = vec![Shot::looking([0.0; 3], [0.0, 0.0, 10.0], 0.0, 1000.0); 30];
    let points = scattered(300, 5, [0.0, 0.0, 10.0], [10.0, 6.0, 8.0]);
    assert_eq!(
        solve_followed(&paths_of(&points, &shots, 0.05, 6), None),
        Err(Unsolved::Still)
    );
}

#[test]
fn a_picture_of_nothing_is_said_to_be() {
    let flat = vec![128u8; 320 * 180 * 4];
    assert_eq!(
        solve(24, None, |_| Some((320, 180, flat.clone()))),
        Err(Unsolved::NoDetail)
    );
}

#[test]
fn a_lens_that_changes_reads_as_a_move_and_shows_in_the_error() {
    let points = scattered(400, 7, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let steady =
        solve_followed(&paths_of(&points, &moving(40, 1000.0), 0.1, 8), None).expect("solves");
    let mut shots = moving(40, 1000.0);
    for (frame, shot) in shots.iter_mut().enumerate() {
        shot.focal = 700.0 + 20.0 * frame as f64;
    }
    let zooming = solve_followed(&paths_of(&points, &shots, 0.1, 8), None).expect("solves");
    // One lens cannot say it, so a push in stands for the zoom, and fits less well.
    assert!(
        zooming.error > 2.0 * steady.error,
        "{} and {}",
        zooming.error,
        steady.error
    );
}

#[test]
fn points_going_their_own_ways_do_not_fit() {
    let mut draws = Draws::new(51);
    let paths = (0..300)
        .map(|_| {
            let (x, y) = (640.0 + draws.part() * 1200.0, 360.0 + draws.part() * 660.0);
            let (dx, dy) = (draws.part() * 16.0, draws.part() * 16.0);
            let (bend, phase) = (draws.part() * 40.0, draws.part() * 6.0);
            Path {
                start: 0,
                at: (0..40)
                    .map(|frame| {
                        let t = frame as f64;
                        [x + dx * t, y + dy * t + bend * (t * 0.3 + phase).sin()]
                    })
                    .collect(),
            }
        })
        .collect();
    let followed = Followed {
        width: W,
        height: H,
        frames: 40,
        paths,
        ended_at: None,
    };
    assert_eq!(solve_followed(&followed, None), Err(Unsolved::DoesNotFit));
}

#[test]
fn a_camera_that_never_turns_fits_any_lens_and_says_so() {
    let shots: Vec<Shot> = (0..40)
        .map(|frame| {
            let t = frame as f64 / 39.0;
            let place = [-3.0 + 6.0 * t, 0.0, 0.0];
            Shot::looking(place, add(place, [0.0, 0.0, 1.0]), 0.0, 900.0)
        })
        .collect();
    let points = scattered(500, 21, [0.0, 0.0, 12.0], [30.0, 12.0, 12.0]);
    let followed = paths_of(&points, &shots, 0.1, 22);
    let shot = solve_followed(&followed, None).expect("solves");
    assert!(!shot.lens_known);
    assert!((shot.fov - REST_FOV).abs() < 1e-9, "{}", shot.fov);
    // What it finds still lands where things show.
    assert!(shot.error < 0.3, "{}", shot.error);
    // Told the lens, it keeps it, and the way is as true.
    let told = solve_followed(&followed, Some(fov_of(900.0, H))).expect("solves");
    assert!(told.lens_known);
    assert!((told.fov - fov_of(900.0, H)).abs() < 1e-9);
    let (way, turned) = way_off(&told, &shots);
    assert!(way < 0.01 && turned < 0.2, "{way} {turned}");
}

#[test]
fn what_moves_by_itself_falls_out() {
    let shots = moving(40, 1000.0);
    let points = scattered(400, 31, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let mut followed = paths_of(&points, &shots, 0.1, 32);
    // A fifth as many again, going their own way across the frame.
    let mut draws = Draws::new(33);
    for _ in 0..80 {
        let (x, y) = (200.0 + draws.part() * 300.0, 300.0 + draws.part() * 300.0);
        followed.paths.push(Path {
            start: 0,
            at: (0..40)
                .map(|frame| [x + 9.0 * frame as f64, y - 3.0 * frame as f64])
                .collect(),
        });
    }
    let shot = solve_followed(&followed, None).expect("solves");
    let (way, turned) = way_off(&shot, &shots);
    assert!(way < 0.015, "{way}");
    assert!(turned < 0.3, "{turned}");
    assert!(shot.error < 0.4, "{}", shot.error);
    assert!(shot.agreeing < 0.9, "{}", shot.agreeing);
}

#[test]
fn few_keyframes_keep_the_way_within_their_tolerance() {
    let shots = moving(40, 1100.0);
    let points = scattered(400, 41, [0.0, 0.0, 10.0], [14.0, 8.0, 8.0]);
    let shot = solve_followed(&paths_of(&points, &shots, 0.0, 42), None).expect("solves");
    let kept = shot.simplified(0.5);
    assert!(kept.len() < 40, "{}", kept.len());
    assert_eq!(kept[0], 0);
    assert_eq!(kept[kept.len() - 1], 39);
    for pair in kept.windows(2) {
        let (a, b) = (shot.cameras[pair[0]], shot.cameras[pair[1]]);
        for index in pair[0] + 1..pair[1] {
            let t = (index - pair[0]) as f64 / (pair[1] - pair[0]) as f64;
            let mix = |p: f64, q: f64| p + (q - p) * t;
            let written = SolvedCamera {
                frame: index,
                x: mix(a.x, b.x),
                y: mix(a.y, b.y),
                z: mix(a.z, b.z),
                pan: mix(a.pan, b.pan),
                tilt: mix(a.tilt, b.tilt),
                roll: mix(a.roll, b.roll),
            };
            for point in shot.points.iter().step_by(25) {
                if let (Some(p), Some(q)) = (
                    shot.shows_at(&shot.cameras[index], point),
                    shot.shows_at(&written, point),
                ) {
                    // Those it watched, and a spread of the rest near enough.
                    assert!((p[0] - q[0]).hypot(p[1] - q[1]) < 1.0);
                }
            }
        }
    }
}

#[test]
fn a_camera_that_holds_and_then_moves_solves_all_of_it() {
    let moves = moving(40, 1000.0);
    // Held on its first place for as long again, and for a while at its last.
    let mut shots = vec![moves[0]; 40];
    shots.extend(moves.iter().copied());
    shots.extend(vec![moves[39]; 20]);
    let points = scattered(500, 71, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let shot = solve_followed(&paths_of(&points, &shots, 0.1, 72), None).expect("solves");
    assert_eq!(shot.cameras.len(), shots.len());
    assert!(shot.error < 0.3, "{}", shot.error);
    let (way, turned) = way_off(&shot, &shots);
    assert!(way < 0.01, "{way}");
    assert!(turned < 0.2, "{turned}");
}

#[test]
fn it_says_where_it_could_go_no_further() {
    let shots = moving(40, 1000.0);
    let points = scattered(400, 81, [0.0, 0.0, 10.0], [16.0, 9.0, 8.0]);
    let mut followed = paths_of(&points, &shots, 0.1, 82);
    // Nothing is seen after frame 29.
    followed.paths.retain_mut(|path| {
        path.at.truncate(30usize.saturating_sub(path.start));
        path.at.len() >= 3
    });
    let shot = solve_followed(&followed, None).expect("solves");
    assert_eq!(shot.cameras.last().map(|camera| camera.frame), Some(29));
    assert_eq!(shot.ended_at, Some(30));
}

#[test]
fn a_long_way_stays_within_what_a_document_can_say() {
    // Down a long street, many times further than anything it sees is away.
    let shots: Vec<Shot> = (0..120)
        .map(|frame| {
            let t = frame as f64 / 119.0;
            let place = [400.0 * t, -0.2 * (t * 9.0).sin(), 0.0];
            let target = [400.0 * t + 8.0, 0.1 * (t * 7.0).cos(), 10.0];
            Shot::looking(place, target, 0.0, 800.0)
        })
        .collect();
    let mut draws = Draws::new(61);
    let points: Vec<Vector> = (0..4000)
        .map(|_| {
            [
                draws.part() * 460.0 + 200.0,
                draws.part() * 8.0,
                10.0 + draws.part() * 10.0,
            ]
        })
        .collect();
    let shot = solve_followed(&paths_of(&points, &shots, 0.1, 62), None).expect("solves");
    assert!(shot.error < 0.4, "{}", shot.error);
    assert_eq!(shot.cameras.len(), 120);
    for camera in &shot.cameras {
        assert!(camera.x.abs() <= 10.0 + 1e-9, "{}", camera.x);
        assert!(camera.y.abs() <= 10.0 + 1e-9, "{}", camera.y);
    }
    assert!(!shot.points.is_empty());
    // A long way adds up what is off a little at a time, with nothing seen twice to hold it to.
    let (way, _) = way_off(&shot, &shots);
    assert!(way < 0.1, "{way}");
}

#[test]
fn the_camera_angles_are_the_compositors() {
    for (pan, tilt, roll) in [
        (0.0, 0.0, 0.0),
        (25.0, -10.0, 5.0),
        (-70.0, 30.0, -40.0),
        (170.0, 20.0, 160.0),
    ] {
        let camera = SolvedCamera {
            frame: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            pan,
            tilt,
            roll,
        };
        let turn = turn_of(&camera);
        assert!((determinant(&turn) - 1.0).abs() < 1e-12);
        let [a, b, c] = angles_of(&turn);
        assert!(
            (a - pan).abs() < 1e-9 && (b - tilt).abs() < 1e-9 && (c - roll).abs() < 1e-9,
            "{pan} {tilt} {roll}: {a} {b} {c}"
        );
    }
}

#[test]
fn pictures_of_a_room_give_the_camera_that_filmed_them() {
    let (width, height) = (480usize, 270usize);
    let focal = 380.0;
    let shots = scene::across_the_room(36, focal);
    let room = scene::room();
    let shot = solve(shots.len(), None, |frame| {
        let picture = scene::picture(&room, &shots[frame], width, height);
        Some((width, height, picture))
    })
    .expect("solves");
    assert_eq!(shot.cameras.len(), 36);
    assert_eq!(shot.ended_at, None);
    assert!(shot.lens_known);
    assert!(shot.error < 0.3, "{}", shot.error);
    assert!(shot.worst < 0.8, "{}", shot.worst);
    let true_fov = fov_of(focal, height as f64);
    assert!(
        (shot.fov - true_fov).abs() < 1.0,
        "{} not {true_fov}",
        shot.fov
    );
    let (way, turned) = way_off(&shot, &shots);
    assert!(way < 0.02, "{way}");
    assert!(turned < 0.4, "{turned}");
    // What it found stands where the scene does: how far from the first camera, against what the true
    // camera sees there, all but the world's one scale.
    let focal_found = focal_of_fov(height as f64, shot.fov);
    let mut ratios: Vec<(f64, f64)> = Vec::new();
    for point in shot.points.iter().filter(|point| point.frame == 0) {
        let at = [
            (point.seen_at[0] - 0.5) * width as f64,
            (point.seen_at[1] - 0.5) * height as f64,
        ];
        let Some(there) = scene::point_at(&room, &shots[0], at) else {
            continue;
        };
        let known = apply(&shots[0].turn, minus(there, shots[0].place))[2];
        ratios.push((point.z * height as f64 + focal_found, known));
    }
    assert!(ratios.len() > 50, "{}", ratios.len());
    let mut scales: Vec<f64> = ratios.iter().map(|(found, known)| found / known).collect();
    scales.sort_by(f64::total_cmp);
    let scale = scales[scales.len() / 2];
    let mut offs: Vec<f64> = ratios
        .iter()
        .map(|(found, known)| (found / (known * scale) - 1.0).abs())
        .collect();
    offs.sort_by(f64::total_cmp);
    let off = offs[offs.len() * 9 / 10];
    assert!(off < 0.05, "nine in ten within {off} of their depth");
}

#[test]
fn pictures_of_a_pan_are_said_to_be_one() {
    let (width, height) = (480usize, 270usize);
    let room = scene::room();
    let shots: Vec<Shot> = (0..30)
        .map(|frame| {
            let a = -0.3 + 0.6 * frame as f64 / 29.0;
            Shot::looking(
                [0.0, -0.5, -2.0],
                [10.0 * a.sin(), 0.0, 10.0 * a.cos() - 2.0],
                0.0,
                380.0,
            )
        })
        .collect();
    let found = solve(shots.len(), None, |frame| {
        Some((
            width,
            height,
            scene::picture(&room, &shots[frame], width, height),
        ))
    });
    assert_eq!(found, Err(Unsolved::OnlyTurns));
}

#[test]
fn a_cut_ends_the_shot_where_it_falls() {
    let (width, height) = (480usize, 270usize);
    let shots = scene::across_the_room(30, 380.0);
    let (room, wall) = (scene::room(), scene::wall());
    let other = Shot::looking([40.0, 0.0, 0.0], [40.0, 0.0, 10.0], 0.0, 200.0);
    let shot = solve(shots.len(), None, |frame| {
        Some(if frame < 24 {
            (
                width,
                height,
                scene::picture(&room, &shots[frame], width, height),
            )
        } else {
            (width, height, scene::picture(&wall, &other, width, height))
        })
    })
    .expect("solves");
    assert_eq!(shot.ended_at, Some(24));
    assert_eq!(shot.cameras.len(), 24);
}

/// A camera wandering about a room at one speed, however long the shot.
fn wandering(frames: usize, focal: f64) -> Vec<Shot> {
    (0..frames)
        .map(|frame| {
            let t = frame as f64 / 120.0;
            let place = [
                2.5 * t.sin(),
                -0.5 + 0.3 * (t * 0.7).cos(),
                -2.0 + 1.2 * (t * 0.4).sin(),
            ];
            let target = [0.8 * (t * 1.3).cos(), 0.1, 9.0];
            Shot::looking(place, target, 0.03 * (t * 3.0).sin(), focal)
        })
        .collect()
}

/// `picture` with a little of the grain a camera adds to each frame.
fn grainy(mut picture: Vec<u8>, frame: usize, amount: f64) -> Vec<u8> {
    let mut draws = Draws::new(frame as u64 * 7919 + 13);
    for pixel in picture.chunks_exact_mut(4) {
        let value = f64::from(pixel[0]) + (draws.part() + draws.part()) * amount;
        pixel[..3].fill(value.clamp(0.0, 255.0) as u8);
    }
    picture
}

/// Accuracy and time at the size `look` reads, with what filming the pictures takes left out:
/// `cargo test --release -p kerf-track measure_ -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore]
fn measure_shots_of_a_room() {
    let (width, height) = (960usize, 540usize);
    let focal = 760.0;
    let room = scene::room();
    for (name, grain, count) in [
        ("clean", 0.0, 48usize),
        ("clean", 0.0, 240),
        ("clean", 0.0, 960),
        ("clean", 0.0, 1800),
        ("grainy", 12.0, 240),
    ] {
        let shots = wandering(count, focal);
        let mut filming = std::time::Duration::ZERO;
        let started = std::time::Instant::now();
        let followed = follow::follow(count, |frame| {
            let filmed = std::time::Instant::now();
            let picture = scene::picture(&room, &shots[frame], width, height);
            let picture = grainy(picture, frame, grain);
            filming += filmed.elapsed();
            Some((width, height, picture))
        });
        let tracked = started.elapsed() - filming;
        let started = std::time::Instant::now();
        let shot = solve_followed(&followed, None).expect("solves");
        let solved = started.elapsed();
        let (way, turned) = way_off(&shot, &shots);
        eprintln!(
            "{name} {count} frames at {width}x{height}: {:.1} ms a frame following, {:.1} ms a frame \
             solving; error {:.3} px, worst {:.3}, agreeing {:.2}; way off {:.3}%, turned off {:.3} \
             deg, fov {:.2} (true {:.2}); {} keyframes",
            tracked.as_secs_f64() * 1000.0 / count as f64,
            solved.as_secs_f64() * 1000.0 / count as f64,
            shot.error,
            shot.worst,
            shot.agreeing,
            way * 100.0,
            turned,
            shot.fov,
            fov_of(focal, height as f64),
            shot.simplified(0.5).len(),
        );
    }
}

/// Whether long and grainy pans are still taken for pans.
#[test]
#[ignore]
fn measure_pans_of_a_room() {
    let (width, height) = (960usize, 540usize);
    let room = scene::room();
    for (grain, count) in [(0.0, 120usize), (12.0, 120), (0.0, 480), (24.0, 120)] {
        let shots: Vec<Shot> = (0..count)
            .map(|frame| {
                let a = -0.35 + 0.7 * frame as f64 / (count - 1) as f64;
                let target = [10.0 * a.sin(), 0.3 * (a * 9.0).sin(), 10.0 * a.cos() - 2.0];
                Shot::looking([0.0, -0.5, -2.0], target, 0.0, 760.0)
            })
            .collect();
        let found = solve(count, None, |frame| {
            let picture = scene::picture(&room, &shots[frame], width, height);
            Some((width, height, grainy(picture, frame, grain)))
        });
        match found {
            Ok(shot) => eprintln!(
                "grain {grain}, {count} frames: solved, error {}",
                shot.error
            ),
            Err(why) => eprintln!("grain {grain}, {count} frames: {why:?}"),
        }
    }
}
