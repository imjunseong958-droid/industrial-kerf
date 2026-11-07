//! Bundle adjustment: Levenberg-Marquardt over cameras and points, with the Schur complement taking
//! the points out of the normal equations (Triggs et al., "Bundle Adjustment: A Modern Synthesis",
//! 2000; Hartley & Zisserman, Appendix 6), and a Huber loss so that what does not fit pulls only so
//! hard. EffectCraft's `track/src/camtrack/bundle.rs`, with no lens distortion.
//!
//! A camera turns by small rotations from where it is, and all cameras share one focal length when it
//! is let go, or have one each for a lens that zooms, adjusted by how much it grows.

use super::math::*;
use super::views::View;
use rayon::prelude::*;

/// A camera in the adjustment. Its focal length is in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub view: View,
    pub focal: f64,
}

/// Camera `camera` sees point `point` at `at`, in pixels from the middle of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sighting {
    pub camera: u32,
    pub point: u32,
    pub at: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Whether the shared focal length moves.
    pub free_focal: bool,
    /// Whether each camera has a focal length of its own, which moves.
    pub own_focals: bool,
    /// Whether the points stay, for a camera found from them.
    pub fixed_points: bool,
    /// Which cameras stay, by index; one must for the world to hold still.
    pub fixed: Vec<bool>,
    /// Where the Huber loss turns from square to straight, in pixels.
    pub huber: f64,
    pub iterations: usize,
}

/// What a sighting behind its camera costs.
const BEHIND: f64 = 1e6;

fn huber(square: f64, limit: f64) -> f64 {
    if square <= limit * limit {
        square
    } else {
        2.0 * limit * square.sqrt() - limit * limit
    }
}

/// How far `point` lands from `at`, in pixels: none behind the camera.
pub fn off(camera: &Camera, point: Vector, at: [f64; 2]) -> Option<[f64; 2]> {
    let seen = camera.view.seen(point);
    if seen[2] <= 1e-9 {
        return None;
    }
    Some([
        camera.focal * seen[0] / seen[2] - at[0],
        camera.focal * seen[1] / seen[2] - at[1],
    ])
}

/// The cost of it all, robustly.
fn cost(cameras: &[Camera], points: &[Vector], sightings: &[Sighting], options: &Options) -> f64 {
    sightings
        .par_iter()
        .map(|sighting| {
            let camera = &cameras[sighting.camera as usize];
            match off(camera, points[sighting.point as usize], sighting.at) {
                Some(r) => huber(r[0] * r[0] + r[1] * r[1], options.huber),
                None => BEHIND,
            }
        })
        .sum()
}

/// A point's part of the normal equations.
struct PointSystem {
    v: Matrix,
    b: Vector,
    /// What ties it to each camera parameter.
    w: Vec<(usize, Vector)>,
}

/// Which of the system's unknowns are whose.
struct Layout {
    /// A camera's first parameter, or none when it stays.
    start: Vec<Option<usize>>,
    focal: Option<usize>,
    /// Each camera's own focal length.
    own: Vec<Option<usize>>,
    size: usize,
}

impl Layout {
    fn of(count: usize, options: &Options) -> Self {
        let mut size = 0;
        let start = (0..count)
            .map(|index| {
                if options.fixed.get(index).copied().unwrap_or(false) {
                    return None;
                }
                size += 6;
                Some(size - 6)
            })
            .collect();
        let focal = options.free_focal.then(|| {
            size += 1;
            size - 1
        });
        let own = (0..count)
            .map(|_| {
                options.own_focals.then(|| {
                    size += 1;
                    size - 1
                })
            })
            .collect();
        Self {
            start,
            focal,
            own,
            size,
        }
    }
}

/// The normal equations' terms for point `index`, adding to the camera system `s` and `b`.
fn point_system(
    index: usize,
    (cameras, points, sightings, by_point): (&[Camera], &[Vector], &[Sighting], &[Vec<u32>]),
    (layout, options): (&Layout, &Options),
    s: &mut [f64],
    b: &mut [f64],
) -> PointSystem {
    let n = layout.size;
    let mut system = PointSystem {
        v: [[0.0; 3]; 3],
        b: [0.0; 3],
        w: Vec::new(),
    };
    let mut camera_terms: Vec<(usize, [f64; 2])> = Vec::with_capacity(8);
    for sighting in &by_point[index] {
        let sighting = &sightings[*sighting as usize];
        let which = sighting.camera as usize;
        let camera = &cameras[which];
        let seen = camera.view.seen(points[index]);
        if seen[2] <= 1e-9 {
            continue;
        }
        let over_z = 1.0 / seen[2];
        let landed = [
            camera.focal * seen[0] * over_z,
            camera.focal * seen[1] * over_z,
        ];
        let r = [landed[0] - sighting.at[0], landed[1] - sighting.at[1]];
        let error = r[0].hypot(r[1]);
        let weight = if error <= options.huber {
            1.0
        } else {
            options.huber / error
        };
        let a = [
            [
                camera.focal * over_z,
                0.0,
                -camera.focal * seen[0] * over_z * over_z,
            ],
            [
                0.0,
                camera.focal * over_z,
                -camera.focal * seen[1] * over_z * over_z,
            ],
        ];
        camera_terms.clear();
        if let Some(start) = layout.start[which] {
            // A small turn moves what it sees by -[seen]x.
            let k = skew(seen);
            for axis in 0..3 {
                let g = [-k[0][axis], -k[1][axis], -k[2][axis]];
                camera_terms.push((start + axis, [dot(a[0], g), dot(a[1], g)]));
            }
            // A step of the camera moves what it sees by -R.
            let turn = &camera.view.turn;
            for axis in 0..3 {
                let g = [-turn[0][axis], -turn[1][axis], -turn[2][axis]];
                camera_terms.push((start + 3 + axis, [dot(a[0], g), dot(a[1], g)]));
            }
        }
        if let Some(focal) = layout.focal.or(layout.own[which]) {
            camera_terms.push((focal, landed));
        }
        let of_point = if options.fixed_points {
            [[0.0; 3]; 2]
        } else {
            [
                apply_back(&camera.view.turn, a[0]),
                apply_back(&camera.view.turn, a[1]),
            ]
        };
        for (i, ji) in &camera_terms {
            b[*i] -= weight * (ji[0] * r[0] + ji[1] * r[1]);
            for (j, jj) in &camera_terms {
                s[i * n + j] += weight * (ji[0] * jj[0] + ji[1] * jj[1]);
            }
        }
        if options.fixed_points {
            continue;
        }
        for i in 0..3 {
            system.b[i] -= weight * (of_point[0][i] * r[0] + of_point[1][i] * r[1]);
            for j in 0..3 {
                system.v[i][j] +=
                    weight * (of_point[0][i] * of_point[0][j] + of_point[1][i] * of_point[1][j]);
            }
        }
        for (i, ji) in &camera_terms {
            let tie: Vector =
                std::array::from_fn(|k| weight * (ji[0] * of_point[0][k] + ji[1] * of_point[1][k]));
            // Each sighting of a point is from another camera: only the shared focal length repeats.
            let repeat = (Some(*i) == layout.focal)
                .then(|| system.w.iter_mut().find(|entry| entry.0 == *i))
                .flatten();
            match repeat {
                Some(entry) => entry.1 = add(entry.1, tie),
                None => system.w.push((*i, tie)),
            }
        }
    }
    system
}

/// Levenberg-Marquardt over `cameras` and `points`, in place: the final cost.
pub fn adjust(
    cameras: &mut [Camera],
    points: &mut [Vector],
    sightings: &[Sighting],
    options: &Options,
) -> f64 {
    let layout = Layout::of(cameras.len(), options);
    let n = layout.size;
    let count = points.len();
    let mut by_point: Vec<Vec<u32>> = vec![Vec::new(); count];
    for (index, sighting) in sightings.iter().enumerate() {
        by_point[sighting.point as usize].push(index as u32);
    }
    let mut now = cost(cameras, points, sightings, options);
    if n == 0 && options.fixed_points {
        return now;
    }
    let mut damping = 1e-4;
    for _ in 0..options.iterations {
        let tied = (&*cameras, &*points, sightings, by_point.as_slice());
        // Build by chunks of points when a copy of the camera system for each is small enough.
        let (base_s, base_b, systems) = if n <= 700 && count > 64 {
            let chunks = rayon::current_num_threads().clamp(1, 16);
            let per = count.div_ceil(chunks);
            let parts: Vec<(Vec<f64>, Vec<f64>, Vec<PointSystem>)> = (0..chunks)
                .into_par_iter()
                .map(|chunk| {
                    let (mut s, mut b) = (vec![0.0; n * n], vec![0.0; n]);
                    let systems = (chunk * per..((chunk + 1) * per).min(count))
                        .map(|index| point_system(index, tied, (&layout, options), &mut s, &mut b))
                        .collect();
                    (s, b, systems)
                })
                .collect();
            let (mut s, mut b) = (vec![0.0; n * n], vec![0.0; n]);
            let mut systems = Vec::with_capacity(count);
            for (part_s, part_b, part) in parts {
                s.iter_mut().zip(part_s).for_each(|(x, y)| *x += y);
                b.iter_mut().zip(part_b).for_each(|(x, y)| *x += y);
                systems.extend(part);
            }
            (s, b, systems)
        } else {
            let (mut s, mut b) = (vec![0.0; n * n], vec![0.0; n]);
            let systems = (0..count)
                .map(|index| point_system(index, tied, (&layout, options), &mut s, &mut b))
                .collect();
            (s, b, systems)
        };
        let mean = ((0..n).map(|i| base_s[i * n + i]).sum::<f64>() / n.max(1) as f64).max(1e-12);
        let mut improved = false;
        for _ in 0..12 {
            let mut s = base_s.clone();
            let mut rhs = base_b.clone();
            for i in 0..n {
                let d = s[i * n + i];
                s[i * n + i] = d + damping * d.max(1e-6 * mean);
            }
            let inverses: Vec<Option<Matrix>> = if options.fixed_points {
                vec![None; count]
            } else {
                systems
                    .par_iter()
                    .map(|system| {
                        let mut v = system.v;
                        let mean = (v[0][0] + v[1][1] + v[2][2]) / 3.0;
                        for k in 0..3 {
                            v[k][k] += damping * v[k][k].max(1e-6 * mean.max(1e-12)) + 1e-12;
                        }
                        inverse(&v)
                    })
                    .collect()
            };
            if !options.fixed_points {
                let reduce = |range: std::ops::Range<usize>, s: &mut [f64], rhs: &mut [f64]| {
                    for j in range {
                        let (Some(vi), system) = (&inverses[j], &systems[j]) else {
                            continue;
                        };
                        let vb = apply(vi, system.b);
                        let wv: Vec<Vector> = system.w.iter().map(|(_, w)| apply(vi, *w)).collect();
                        for (a, (ia, wa)) in system.w.iter().enumerate() {
                            rhs[*ia] -= dot(*wa, vb);
                            for (ib, wb) in &system.w[a..] {
                                let d = dot(wv[a], *wb);
                                s[ia * n + ib] -= d;
                                if ia != ib {
                                    s[ib * n + ia] -= d;
                                }
                            }
                        }
                    }
                };
                let chunks = rayon::current_num_threads().clamp(1, 8);
                if n <= 1200 && count >= 64 && chunks > 1 {
                    let per = count.div_ceil(chunks);
                    let parts: Vec<(Vec<f64>, Vec<f64>)> = (0..chunks)
                        .into_par_iter()
                        .map(|chunk| {
                            let (mut ds, mut dr) = (vec![0.0; n * n], vec![0.0; n]);
                            reduce(
                                chunk * per..((chunk + 1) * per).min(count),
                                &mut ds,
                                &mut dr,
                            );
                            (ds, dr)
                        })
                        .collect();
                    for (ds, dr) in parts {
                        s.par_iter_mut()
                            .zip(ds.par_iter())
                            .for_each(|(x, y)| *x += y);
                        rhs.iter_mut().zip(dr).for_each(|(x, y)| *x += y);
                    }
                } else {
                    reduce(0..count, &mut s, &mut rhs);
                }
            }
            let mut step = rhs;
            if n > 0 && !solve_positive(&mut s, n, &mut step) {
                damping *= 10.0;
                continue;
            }
            let mut moved: Vec<Camera> = cameras.to_vec();
            for (index, camera) in moved.iter_mut().enumerate() {
                if let Some(start) = layout.start[index] {
                    let turn = [step[start], step[start + 1], step[start + 2]];
                    camera.view.turn = product(&rotation(turn), &camera.view.turn);
                    camera.view.place = add(
                        camera.view.place,
                        [step[start + 3], step[start + 4], step[start + 5]],
                    );
                }
                if let Some(focal) = layout.focal.or(layout.own[index]) {
                    camera.focal *= step[focal].clamp(-0.5, 0.5).exp();
                }
            }
            let mut placed: Vec<Vector> = points.to_vec();
            if !options.fixed_points {
                placed.par_iter_mut().enumerate().for_each(|(j, point)| {
                    let Some(vi) = &inverses[j] else {
                        return;
                    };
                    let system = &systems[j];
                    let mut b = system.b;
                    for (i, w) in &system.w {
                        b = minus(b, times(*w, step[*i]));
                    }
                    *point = add(*point, apply(vi, b));
                });
            }
            let then = cost(&moved, &placed, sightings, options);
            if then.is_finite() && then < now {
                cameras.copy_from_slice(&moved);
                points.copy_from_slice(&placed);
                let gain = now - then;
                now = then;
                damping = (damping / 3.0).max(1e-9);
                improved = gain > 1e-6 * now.max(1e-12);
                if !improved {
                    return now;
                }
                break;
            }
            damping *= 4.0;
        }
        if !improved {
            break;
        }
    }
    now
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::views::Draws;

    /// Parts of a span, from a seeded draw.
    fn parts(seed: u64) -> impl FnMut() -> f64 {
        let mut state = Draws::new(seed);
        move || state.part()
    }

    #[test]
    fn it_finds_its_way_back_from_noise() {
        let mut part = parts(99);
        let truth: Vec<Vector> = (0..80)
            .map(|_| [part() * 6.0, part() * 4.0, 6.0 + part() * 4.0])
            .collect();
        let focal = 900.0;
        let true_cameras: Vec<Camera> = (0..6)
            .map(|index| {
                let i = index as f64;
                Camera {
                    view: View {
                        turn: rotation([0.0, -0.05 * i, 0.01 * i]),
                        place: [0.3 * i, 0.05 * i, 0.0],
                    },
                    focal,
                }
            })
            .collect();
        let mut sightings = Vec::new();
        for (c, camera) in true_cameras.iter().enumerate() {
            for (p, point) in truth.iter().enumerate() {
                sightings.push(Sighting {
                    camera: c as u32,
                    point: p as u32,
                    at: off(camera, *point, [0.0, 0.0]).unwrap(),
                });
            }
        }
        let mut cameras = true_cameras.clone();
        for (index, camera) in cameras.iter_mut().enumerate().skip(2) {
            let i = index as f64;
            camera.view.turn = product(&rotation([0.01, -0.003 * i, 0.005]), &camera.view.turn);
            camera.view.place = add(camera.view.place, [0.02, -0.03, 0.01]);
        }
        let mut points: Vec<Vector> = truth
            .iter()
            .map(|point| add(*point, [part() * 0.1, part() * 0.1, part() * 0.2]))
            .collect();
        let cost = adjust(
            &mut cameras,
            &mut points,
            &sightings,
            &Options {
                free_focal: false,
                own_focals: false,
                fixed_points: false,
                fixed: vec![true, true, false, false, false, false],
                huber: 2.0,
                iterations: 50,
            },
        );
        assert!((cost / sightings.len() as f64).sqrt() < 1e-4, "{cost}");
    }

    #[test]
    fn it_finds_a_focal_length_for_each_camera() {
        let mut part = parts(5);
        let points: Vec<Vector> = (0..120)
            .map(|_| [part() * 6.0, part() * 4.0, 7.0 + part() * 5.0])
            .collect();
        let camera = |index: usize, focal: f64| Camera {
            view: View {
                turn: rotation([0.02 * index as f64, -0.06 * index as f64, 0.0]),
                place: [0.4 * index as f64, 0.05 * index as f64, 0.2 * index as f64],
            },
            focal,
        };
        let true_focal = |index: usize| 800.0 + 60.0 * index as f64;
        let mut sightings = Vec::new();
        for c in 0..6 {
            for (p, point) in points.iter().enumerate() {
                sightings.push(Sighting {
                    camera: c as u32,
                    point: p as u32,
                    at: off(&camera(c, true_focal(c)), *point, [0.0, 0.0]).unwrap(),
                });
            }
        }
        let mut cameras: Vec<Camera> = (0..6).map(|c| camera(c, true_focal(c) * 0.97)).collect();
        cameras[0].focal = true_focal(0);
        let mut moved = points.clone();
        adjust(
            &mut cameras,
            &mut moved,
            &sightings,
            &Options {
                free_focal: false,
                own_focals: true,
                fixed_points: false,
                fixed: vec![true],
                huber: 2.0,
                iterations: 60,
            },
        );
        for (index, camera) in cameras.iter().enumerate() {
            let off = (camera.focal - true_focal(index)).abs();
            assert!(off < 2.0, "{index}: {}", camera.focal);
        }
    }

    #[test]
    fn it_finds_a_shared_focal_length() {
        let mut part = parts(5);
        let points: Vec<Vector> = (0..80)
            .map(|_| [part() * 6.0, part() * 4.0, 6.0 + part() * 4.0])
            .collect();
        let camera = |index: usize, focal: f64| Camera {
            view: View {
                turn: rotation([0.02 * index as f64, -0.06 * index as f64, 0.0]),
                place: [0.4 * index as f64, 0.05 * index as f64, 0.2 * index as f64],
            },
            focal,
        };
        let mut sightings = Vec::new();
        for c in 0..6 {
            for (p, point) in points.iter().enumerate() {
                sightings.push(Sighting {
                    camera: c as u32,
                    point: p as u32,
                    at: off(&camera(c, 1000.0), *point, [0.0, 0.0]).unwrap(),
                });
            }
        }
        let mut cameras: Vec<Camera> = (0..6).map(|c| camera(c, 950.0)).collect();
        let mut moved = points.clone();
        adjust(
            &mut cameras,
            &mut moved,
            &sightings,
            &Options {
                free_focal: true,
                own_focals: false,
                fixed_points: false,
                fixed: vec![true],
                huber: 2.0,
                iterations: 60,
            },
        );
        assert!(
            (cameras[3].focal - 1000.0).abs() < 1.0,
            "{}",
            cameras[3].focal
        );
    }
}
