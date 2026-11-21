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
