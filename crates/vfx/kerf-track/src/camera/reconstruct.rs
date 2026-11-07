//! From paths on the picture to cameras and points in space: structure from motion, as EffectCraft's
//! `track/src/camtrack/solver.rs` does it. Key frames are chosen by how far things moved; two of them
//! start a reconstruction, from the essential matrix or, for a scene that is mostly flat, a homography;
//! the others are added one at a time, each found from the points already placed, which then place
//! more; bundle adjustment settles it all; and the frames between are found last. The focal length is
//! searched for, by how well a short reconstruction fits at each.
//!
//! A lens that zooms ([`solve`]) is one more reconstruction, of the paths with how much the picture grew
//! taken out of them, whose cameras then each get the focal length that grew and adjust it. It stands
//! only where it fits clearly better than one lens does, so a shot that one lens explains comes out as
//! it would have.
//!
//! Nothing here comes out of a doubtful shot: a picture that does not move, a camera that only turns,
//! and a reconstruction that does not fit are each said to be so ([`Unsolved`]).

use super::adjust::{self, adjust, Camera, Options, Sighting};
use super::follow::Followed;
use super::math::*;
use super::views::{self, View};
use super::Unsolved;
use crate::planar::fit_robustly;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Paths from the middle of the frame, in pixels, as a solve reads them.
struct Seen {
    frames: usize,
    width: f64,
    /// Each path's first frame, and where it goes.
    paths: Vec<(usize, Vec<[f64; 2]>)>,
    /// What each frame sees: which path, where.
    by_frame: Vec<Vec<(u32, [f64; 2])>>,
    /// How far a path is taken to wander, in pixels. It starts high, and is measured once the key
    /// frames are solved, so that what moves by itself then falls out.
    noise: f64,
}

impl Seen {
    fn new(followed: &Followed) -> Self {
        let middle = [followed.width * 0.5, followed.height * 0.5];
        let mut paths = Vec::with_capacity(followed.paths.len());
        let mut by_frame = vec![Vec::new(); followed.frames];
        for (index, path) in followed.paths.iter().enumerate() {
            let at: Vec<[f64; 2]> = path
                .at
                .iter()
                .map(|at| [at[0] - middle[0], at[1] - middle[1]])
                .collect();
            for (step, at) in at.iter().enumerate() {
                if let Some(frame) = by_frame.get_mut(path.start + step) {
                    frame.push((index as u32, *at));
                }
            }
            paths.push((path.start, at));
        }
        Self {
            frames: followed.frames,
            width: followed.width,
            paths,
            by_frame,
            noise: 0.5,
        }
    }

    fn at(&self, path: u32, frame: usize) -> Option<[f64; 2]> {
        let (start, at) = &self.paths[path as usize];
        frame
            .checked_sub(*start)
            .and_then(|step| at.get(step).copied())
    }

    fn tolerance(&self) -> f64 {
        4.0 * self.noise
    }

    /// What both frames see: which path, where in the first, where in the second.
    fn shared(&self, a: usize, b: usize) -> Vec<(u32, [f64; 2], [f64; 2])> {
        self.by_frame[a]
            .iter()
            .filter_map(|(path, at)| self.at(*path, b).map(|there| (*path, *at, there)))
            .collect()
    }
}

/// How few shared points a key frame may keep with the last.
const FEWEST_SHARED: usize = 24;
/// How far things move between key frames, as a share of the width, at most.
const KEY_MOTION: f64 = 0.015;
/// How many frames apart key frames are, at most.
const KEY_SPACING: usize = 12;
/// How few points a frame must see to count.
const FEWEST_SEEN: usize = 8;

/// A new key frame when what it sees has moved far enough since the last, or too little of that is
/// left, or it has been a while. The first and last frames that see enough are among them.
fn key_frames(seen: &Seen) -> Vec<usize> {
    let Some(first) = (0..seen.frames).find(|k| seen.by_frame[*k].len() >= FEWEST_SEEN) else {
        return Vec::new();
    };
    let mut keys = vec![first];
    let mut last = first;
    let mut next = first + 1;
    while next < seen.frames {
        let base = &seen.by_frame[last];
        let mut moved: Vec<f64> = base
            .iter()
            .filter_map(|(path, at)| {
                seen.at(*path, next)
                    .map(|there| (there[0] - at[0]).hypot(there[1] - at[1]))
            })
            .collect();
        if moved.len() < FEWEST_SHARED && next > last + 1 {
            last = next - 1;
            keys.push(last);
            continue;
        }
        moved.sort_by(f64::total_cmp);
        let middle = moved.get(moved.len() / 2).copied().unwrap_or(0.0);
        let left = moved.len() as f64 / base.len().max(1) as f64;
        if middle > KEY_MOTION * seen.width
            || left < 0.7
            || moved.len() < FEWEST_SHARED
            || next - last >= KEY_SPACING
        {
            last = next;
            keys.push(next);
        }
        next += 1;
    }
    if let Some(end) = (0..seen.frames)
        .rev()
        .find(|k| seen.by_frame[*k].len() >= FEWEST_SEEN)
    {
        if keys.last().is_some_and(|last| *last < end) {
            keys.push(end);
        }
    }
    keys
}

/// What moves of the focal length when cameras adjust.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focal {
    Held,
    Shared,
    Own,
}

/// The lens a reconstruction goes by: one length, or a base to scale by frame, where the picture grows.
#[derive(Clone, Copy)]
struct Lens<'a> {
    base: f64,
    scales: Option<&'a [f64]>,
}

impl Lens<'_> {
    fn at(&self, frame: usize) -> f64 {
        match self.scales {
            Some(scales) => self.base * scales[frame],
            None => self.base,
        }
    }

    /// What moves of the focal length when cameras adjust as a reconstruction grows.
    fn adjusting(&self) -> Focal {
        if self.scales.is_some() {
            Focal::Own
        } else {
            Focal::Held
        }
    }

    /// A focal length for `frame`, from how the cameras nearest it have come out against the scales.
    fn near(&self, cameras: &BTreeMap<usize, Camera>, frame: usize) -> Option<f64> {
        let scales = self.scales?;
        let nearest = cameras
            .iter()
            .min_by_key(|(other, _)| other.abs_diff(frame))?;
        Some(nearest.1.focal / scales[*nearest.0] * scales[frame])
    }
}

/// Cameras and points as far as they go.
#[derive(Clone, Default)]
struct Reconstruction {
    cameras: BTreeMap<usize, Camera>,
    points: BTreeMap<u32, Vector>,
    /// Sightings left out, by path and frame.
    outliers: HashSet<(u32, usize)>,
}

impl Reconstruction {
    /// What `frames` see of the points: the cameras, the points, the sightings, each point's path.
    fn sightings(
        &self,
        seen: &Seen,
        frames: &[usize],
    ) -> (Vec<Camera>, Vec<Vector>, Vec<Sighting>, Vec<u32>) {
        let cameras: Vec<Camera> = frames.iter().map(|frame| self.cameras[frame]).collect();
        let mut indexes: HashMap<u32, u32> = HashMap::new();
        let (mut points, mut paths, mut sightings) = (Vec::new(), Vec::new(), Vec::new());
        for (camera, frame) in frames.iter().enumerate() {
            for (path, at) in &seen.by_frame[*frame] {
                let Some(point) = self.points.get(path) else {
                    continue;
                };
                if self.outliers.contains(&(*path, *frame)) {
                    continue;
                }
                let index = *indexes.entry(*path).or_insert_with(|| {
                    points.push(*point);
                    paths.push(*path);
                    (points.len() - 1) as u32
                });
                sightings.push(Sighting {
                    camera: camera as u32,
                    point: index,
                    at: *at,
                });
            }
        }
        (cameras, points, sightings, paths)
    }

    /// Adjust the cameras of `frames`, the first staying where it is, and what they see.
    fn adjusted(&mut self, seen: &Seen, frames: &[usize], focal: Focal, iterations: usize) {
        let (mut cameras, mut points, sightings, paths) = self.sightings(seen, frames);
        if sightings.is_empty() {
            return;
        }
        let mut fixed = vec![false; cameras.len()];
        fixed[0] = true;
        adjust(
            &mut cameras,
            &mut points,
            &sightings,
            &Options {
                free_focal: focal == Focal::Shared,
                own_focals: focal == Focal::Own,
                fixed_points: false,
                fixed,
                huber: seen.noise,
                iterations,
            },
        );
        for (frame, camera) in frames.iter().zip(&cameras) {
            self.cameras.insert(*frame, *camera);
        }
        for (path, point) in paths.iter().zip(&points) {
            self.points.insert(*path, *point);
        }
    }

    /// Leave out sightings past the tolerance, and points left with fewer than two.
    fn rejected(&mut self, seen: &Seen) {
        let limit = seen.tolerance().powi(2);
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for (frame, camera) in &self.cameras {
            for (path, at) in &seen.by_frame[*frame] {
                let Some(point) = self.points.get(path) else {
                    continue;
                };
                if self.outliers.contains(&(*path, *frame)) {
                    continue;
                }
                match adjust::off(camera, *point, *at) {
                    Some(r) if r[0] * r[0] + r[1] * r[1] <= limit => {
                        *counts.entry(*path).or_default() += 1;
                    }
                    _ => {
                        self.outliers.insert((*path, *frame));
                    }
                }
            }
        }
        self.points
            .retain(|path, _| counts.get(path).copied().unwrap_or(0) >= 2);
    }

    /// Place paths that two or more of the cameras see and that have no point yet, where the rays
    /// meet at more than `least_angle` degrees.
    fn triangulated(&mut self, seen: &Seen, least_angle: f64) {
        let mut seeing: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for frame in self.cameras.keys() {
            for (path, _) in &seen.by_frame[*frame] {
                if !self.points.contains_key(path) {
                    seeing.entry(*path).or_default().push(*frame);
                }
            }
        }
        let limit = seen.tolerance().powi(2);
        let found: Vec<(u32, Vector)> = seeing
            .par_iter()
            .filter(|(_, frames)| frames.len() >= 2)
            .filter_map(|(path, frames)| {
                let views: Vec<(View, [f64; 2])> = frames
                    .iter()
                    .map(|frame| {
                        let camera = &self.cameras[frame];
                        let at = seen.at(*path, *frame)?;
                        Some((camera.view, [at[0] / camera.focal, at[1] / camera.focal]))
                    })
                    .collect::<Option<_>>()?;
                let point = views::triangulate(&views)?;
                let (first, last) = (views[0].0.place, views[views.len() - 1].0.place);
                if views::ray_angle(first, last, point).to_degrees() < least_angle {
                    return None;
                }
                for frame in frames {
                    let r = adjust::off(&self.cameras[frame], point, seen.at(*path, *frame)?)?;
                    if r[0] * r[0] + r[1] * r[1] > limit {
                        return None;
                    }
                }
                Some((*path, point))
            })
            .collect();
        self.points.extend(found);
    }

    /// The camera of `frame` from the points it sees, starting at `start`, with the paths of the
    /// sightings that do not agree, or none when too few agree.
    fn resected(
        &self,
        seen: &Seen,
        frame: usize,
        start: Camera,
        free_focal: bool,
    ) -> Option<(Camera, Vec<u32>)> {
        let (mut points, mut sightings, mut paths) = (Vec::new(), Vec::new(), Vec::new());
        for (path, at) in &seen.by_frame[frame] {
            if let Some(point) = self.points.get(path) {
                sightings.push(Sighting {
                    camera: 0,
                    point: points.len() as u32,
                    at: *at,
                });
                points.push(*point);
                paths.push(*path);
            }
        }
        if sightings.len() < 6 {
            return None;
        }
        let limit = seen.tolerance().powi(2);
        let options = |huber: f64| Options {
            free_focal,
            own_focals: false,
            fixed_points: true,
            fixed: vec![false],
            huber,
            iterations: 40,
        };
        let agrees = |camera: &Camera, sighting: &Sighting| {
            adjust::off(camera, points[sighting.point as usize], sighting.at)
                .is_some_and(|r| r[0] * r[0] + r[1] * r[1] <= limit)
        };
        let mut best: Option<(usize, Camera)> = None;
        // Loosely, then tightly; from the guess and, failing that, from the nearest camera.
        let nearest = self
            .cameras
            .iter()
            .min_by_key(|(other, _)| other.abs_diff(frame))
            .map(|(_, camera)| *camera);
        for from in std::iter::once(start).chain(nearest) {
            if best
                .as_ref()
                .is_some_and(|(most, _)| *most as f64 >= 0.8 * sightings.len() as f64)
            {
                break;
            }
            let mut cameras = vec![from];
            let mut held = points.clone();
            adjust(
                &mut cameras,
                &mut held,
                &sightings,
                &options(seen.tolerance() * 4.0),
            );
            adjust(&mut cameras, &mut held, &sightings, &options(seen.noise));
            let agreeing = sightings.iter().filter(|s| agrees(&cameras[0], s)).count();
            if best.as_ref().is_none_or(|(most, _)| agreeing > *most) {
                best = Some((agreeing, cameras[0]));
            }
        }
        let (agreeing, camera) = best?;
        if agreeing < 6 || (agreeing as f64) < 0.5 * sightings.len() as f64 {
            return None;
        }
        let keep: Vec<Sighting> = sightings
            .iter()
            .filter(|s| agrees(&camera, s))
            .copied()
            .collect();
        let mut cameras = vec![camera];
        let mut held = points.clone();
        adjust(&mut cameras, &mut held, &keep, &options(seen.noise));
        let off = sightings
            .iter()
            .filter(|s| !agrees(&cameras[0], s))
            .map(|s| paths[s.point as usize])
            .collect();
        Some((cameras[0], off))
    }

    /// How well it explains all that its frames see: how far each sighting lands from where it was,
    /// no more than the tolerance, which is also what a sighting with no point counts as.
    fn score(&self, seen: &Seen) -> f64 {
        let limit = seen.tolerance();
        let (mut sum, mut count) = (0.0, 0usize);
        for (frame, camera) in &self.cameras {
            for (path, at) in &seen.by_frame[*frame] {
                count += 1;
                sum += self
                    .points
                    .get(path)
                    .and_then(|point| adjust::off(camera, *point, *at))
                    .map_or(limit, |r| r[0].hypot(r[1]).min(limit));
            }
        }
        if count > 0 {
            sum / count as f64
        } else {
            f64::INFINITY
        }
    }

    /// How far each sighting not left out lands from where it was, in pixels.
    fn errors(&self, seen: &Seen) -> Vec<f64> {
        let mut errors = Vec::new();
        for (frame, camera) in &self.cameras {
            for (path, at) in &seen.by_frame[*frame] {
                if self.outliers.contains(&(*path, *frame)) {
                    continue;
                }
                let landed = self
                    .points
                    .get(path)
                    .and_then(|point| adjust::off(camera, *point, *at));
                if let Some(r) = landed {
                    errors.push(r[0].hypot(r[1]));
                }
            }
        }
        errors
    }
}

/// A first guess at the camera of `frame`, from those on either side of it.
fn guess(reconstruction: &Reconstruction, frame: usize) -> Option<Camera> {
    let cameras = &reconstruction.cameras;
    let before = cameras.range(..frame).next_back().map(|(k, c)| (*k, *c));
    let after = cameras.range(frame + 1..).next().map(|(k, c)| (*k, *c));
    // Going on from `near` as it came from `far`, `by` times as far.
    let onward = |near: Camera, far: Camera, by: f64| {
        let by = by.min(2.0);
        let turned = axis_of(&product(&near.view.turn, &transposed(&far.view.turn)));
        Camera {
            view: View {
                turn: product(&rotation(times(turned, by)), &near.view.turn),
                place: add(
                    near.view.place,
                    times(minus(near.view.place, far.view.place), by),
                ),
            },
            focal: near.focal,
        }
    };
    match (before, after) {
        (Some((a, ca)), Some((b, cb))) => {
            let t = (frame - a) as f64 / (b - a) as f64;
            Some(Camera {
                view: View {
                    turn: between(&ca.view.turn, &cb.view.turn, t),
                    place: add(times(ca.view.place, 1.0 - t), times(cb.view.place, t)),
                },
                focal: ca.focal,
            })
        }
        (Some((a, ca)), None) => Some(match cameras.range(..a).next_back() {
            Some((b, cb)) => onward(ca, *cb, (frame - a) as f64 / (a - b) as f64),
            None => ca,
        }),
        (None, Some((a, ca))) => Some(match cameras.range(a + 1..).next() {
            Some((b, cb)) => onward(ca, *cb, (a - frame) as f64 / (b - a) as f64),
            None => ca,
        }),
        (None, None) => None,
    }
}

/// A second camera for a start, and the points the two place.
type Start = (View, BTreeMap<u32, Vector>);

/// Ways to start from frames `a` and `b` at `focal`: from the essential matrix, and from a homography
/// for what is flat.
fn starts(seen: &Seen, a: usize, b: usize, lens: Lens<'_>) -> Vec<Start> {
    let shared = seen.shared(a, b);
    if shared.len() < 12 {
        return Vec::new();
    }
    let (focal, focal_b) = (lens.at(a), lens.at(b));
    let first: Vec<[f64; 2]> = shared
        .iter()
        .map(|s| [s.1[0] / focal, s.1[1] / focal])
        .collect();
    let second: Vec<[f64; 2]> = shared
        .iter()
        .map(|s| [s.2[0] / focal_b, s.2[1] / focal_b])
        .collect();
    let tolerance = 2.0 * seen.noise;
    let mut views_found: Vec<View> = Vec::new();
    let seed = (a * 7919 + b) as u64;
    if let Some((e, agree)) =
        views::essential_robustly(&first, &second, tolerance / focal, 400, seed)
    {
        let (p, q) = agreeing_pairs(&first, &second, &agree);
        if let Some(view) = views::in_front(&views::apart(&e), &p, &q) {
            views_found.push(view);
        }
    }
    let (from, to): (Vec<[f64; 2]>, Vec<[f64; 2]>) = shared.iter().map(|s| (s.1, s.2)).unzip();
    if let Some((h, agree)) = fit_robustly(&from, &to, tolerance) {
        let h = h.0;
        // Calibrated: K⁻¹ H K, with K = diag(f, f, 1) on each side.
        let grew = focal / focal_b;
        let calibrated = [
            [h[0][0] * grew, h[0][1] * grew, h[0][2] / focal_b],
            [h[1][0] * grew, h[1][1] * grew, h[1][2] / focal_b],
            [h[2][0] * focal, h[2][1] * focal, h[2][2]],
        ];
        let (p, q) = agreeing_pairs(&first, &second, &agree);
        for (turn, shift) in views::homography_apart(&calibrated, &p, &q) {
            if length(shift) > 1e-6 {
                views_found.push(View::from_shift(turn, unit(shift)));
            }
        }
    }
    let limit = (seen.tolerance() / focal).powi(2);
    views_found
        .into_iter()
        .filter_map(|view| {
            let mut points = BTreeMap::new();
            for (index, pair) in shared.iter().enumerate() {
                let Some(point) =
                    views::triangulate(&[(View::AT_REST, first[index]), (view, second[index])])
                else {
                    continue;
                };
                let fits = views::off_squared(&View::AT_REST, point, first[index])
                    .is_some_and(|off| off < limit)
                    && views::off_squared(&view, point, second[index])
                        .is_some_and(|off| off < limit)
                    && views::ray_angle([0.0; 3], view.place, point).to_degrees() > 0.3;
                if fits {
                    points.insert(pair.0, point);
                }
            }
            (points.len() >= 10 && points.len() * 3 >= shared.len()).then_some((view, points))
        })
        .collect()
}

fn agreeing_pairs(
    a: &[[f64; 2]],
    b: &[[f64; 2]],
    agree: &[bool],
) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    a.iter()
        .zip(b)
        .zip(agree)
        .filter(|(_, agrees)| **agrees)
        .map(|((p, q), _)| (*p, *q))
        .unzip()
}

/// How many key frames on a start's second frame may be.
const START_REACH: usize = 6;

/// The second key frame after `keys[at]` that gives the most points with the widest angle between
/// their rays, and all the ways to start from there.
fn best_starts(
    seen: &Seen,
    keys: &[usize],
    at: usize,
    lens: Lens<'_>,
) -> Option<(usize, Vec<Start>)> {
    let mut best: Option<(f64, usize, Vec<Start>)> = None;
    for offset in 1..START_REACH.min(keys.len() - at) {
        let found = starts(seen, keys[at], keys[at + offset], lens);
        let score = found
            .iter()
            .map(|(view, points)| {
                let mut angles: Vec<f64> = points
                    .values()
                    .map(|point| views::ray_angle([0.0; 3], view.place, *point).to_degrees())
                    .collect();
                angles.sort_by(f64::total_cmp);
                let middle = angles.get(angles.len() / 2).copied().unwrap_or(0.0);
                points.len() as f64 * middle.min(4.0)
            })
            .fold(0.0, f64::max);
        if !found.is_empty() && best.as_ref().is_none_or(|(most, ..)| score > *most) {
            best = Some((score, offset, found));
        }
    }
    best.map(|(_, offset, found)| (offset, found))
}

/// Grow a reconstruction over `keys` from a start between `keys[at]` and `keys[at + offset]`, toward
/// both ends of the shot. A key frame that cannot be found ends the way it was going.
fn grown(
    seen: &Seen,
    keys: &[usize],
    (at, offset): (usize, usize),
    start: &Start,
    lens: Lens<'_>,
) -> Option<Reconstruction> {
    let (a, b) = (keys[at], keys[at + offset]);
    let mut reconstruction = Reconstruction::default();
    reconstruction.cameras.insert(
        a,
        Camera {
            view: View::AT_REST,
            focal: lens.at(a),
        },
    );
    reconstruction.cameras.insert(
        b,
        Camera {
            view: start.0,
            focal: lens.at(b),
        },
    );
    reconstruction.points = start.1.clone();
    reconstruction.adjusted(seen, &[a, b], lens.adjusting(), 10);
    reconstruction.rejected(seen);
    let add_one = |reconstruction: &mut Reconstruction, frame: usize, added: &mut usize| {
        let Some(mut from) = guess(reconstruction, frame) else {
            return false;
        };
        if let Some(focal) = lens.near(&reconstruction.cameras, frame) {
            from.focal = focal;
        }
        let free = lens.scales.is_some();
        let Some((camera, off)) = reconstruction.resected(seen, frame, from, free) else {
            return false;
        };
        reconstruction.cameras.insert(frame, camera);
        reconstruction
            .outliers
            .extend(off.into_iter().map(|path| (path, frame)));
        reconstruction.triangulated(seen, 1.0);
        *added += 1;
        let frames: Vec<usize> = reconstruction.cameras.keys().copied().collect();
        if added.is_multiple_of(3) || frames.len() <= 6 {
            reconstruction.adjusted(seen, &frames, lens.adjusting(), 10);
            reconstruction.rejected(seen);
        }
        true
    };
    let mut added = 0;
    for key in &keys[at + 1..at + offset] {
        if !add_one(&mut reconstruction, *key, &mut added) {
            break;
        }
    }
    let (mut onward, mut back) = (keys[at + offset + 1..].iter(), keys[..at].iter().rev());
    let (mut onward_open, mut back_open) = (true, true);
    while onward_open || back_open {
        if onward_open {
            onward_open = onward
                .next()
                .is_some_and(|key| add_one(&mut reconstruction, *key, &mut added));
        }
        if back_open {
            back_open = back
                .next()
                .is_some_and(|key| add_one(&mut reconstruction, *key, &mut added));
        }
    }
    let frames: Vec<usize> = reconstruction.cameras.keys().copied().collect();
    reconstruction.adjusted(seen, &frames, lens.adjusting(), 10);
    reconstruction.rejected(seen);
    (reconstruction.cameras.len() >= 2 && reconstruction.points.len() >= 8)
        .then_some(reconstruction)
}

/// The best reconstruction of `keys` at `focal` from `keys[at]`, with its score: less is better, and
/// covering less of the keys counts against it.
fn best_reconstruction(
    seen: &Seen,
    keys: &[usize],
    at: usize,
    lens: Lens<'_>,
) -> Option<(f64, Reconstruction)> {
    let (offset, found) = best_starts(seen, keys, at, lens)?;
    found
        .par_iter()
        .filter_map(|start| {
            let reconstruction = grown(seen, keys, (at, offset), start, lens)?;
            if reconstruction.errors(seen).is_empty() {
                return None;
            }
            let covered = reconstruction.cameras.len() as f64 / keys.len() as f64;
            Some((
                reconstruction.score(seen) / covered.max(0.05),
                reconstruction,
            ))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The focal length of a horizontal angle in degrees.
fn focal_of(width: f64, angle: f64) -> f64 {
    width * 0.5 / (angle.clamp(1.0, 170.0).to_radians() * 0.5).tan()
}

/// Horizontal angles a search looks through, in degrees.
const ANGLES: [f64; 14] = [
    8.0, 12.0, 16.0, 20.0, 25.0, 30.0, 36.0, 43.0, 51.0, 60.0, 70.0, 82.0, 96.0, 112.0,
];

/// What a search for the focal length found.
struct Search {
    focal: f64,
    /// Whether what it fits at changes enough across lengths to say.
    told: bool,
}

/// How much worse the fit must be a good way from the best focal length for the lens to be known.
const TOLD_BY: f64 = 1.15;

/// The focal length `error` is least at: over a range, and closer around the best (golden section).
fn search_focal(width: f64, error: &(dyn Fn(f64) -> f64 + Sync)) -> Search {
    let focals: Vec<f64> = ANGLES.iter().map(|angle| focal_of(width, *angle)).collect();
    let errors: Vec<f64> = focals.par_iter().map(|focal| error(*focal)).collect();
    let index = (0..focals.len())
        .min_by(|a, b| errors[*a].total_cmp(&errors[*b]))
        .unwrap_or(5);
    let (mut low, mut high) = (
        focals[(index + 1).min(focals.len() - 1)].ln(),
        focals[index.saturating_sub(1)].ln(),
    );
    if low > high {
        std::mem::swap(&mut low, &mut high);
    }
    let golden = 0.618_033_988_75;
    let (mut x1, mut x2) = (high - golden * (high - low), low + golden * (high - low));
    let (mut e1, mut e2) = rayon::join(|| error(x1.exp()), || error(x2.exp()));
    for _ in 0..7 {
        if e1 < e2 {
            high = x2;
            x2 = x1;
            e2 = e1;
            x1 = high - golden * (high - low);
            e1 = error(x1.exp());
        } else {
            low = x1;
            x1 = x2;
            e1 = e2;
            x2 = low + golden * (high - low);
            e2 = error(x2.exp());
        }
    }
    let (log, least) = if e1 < e2 { (x1, e1) } else { (x2, e2) };
    let focal = if least <= errors[index] {
        log.exp()
    } else {
        focals[index]
    };
    let least = least.min(errors[index]);
    // Two lengths well to either side of the best: any lens fits a camera that never turns.
    let worse = |by: f64| error(focal * by);
    let (shorter, longer) = rayon::join(|| worse(0.6), || worse(1.6));
    Search {
        focal,
        told: shorter.min(longer) > least * TOLD_BY,
    }
}

/// How much a camera that only turns leaves unexplained between two frames, in pixels: a quarter of
/// what they share lands further than this from where the best rotation puts it.
fn left_between(seen: &Seen, a: usize, b: usize, lens: Lens<'_>) -> Option<f64> {
    let shared = seen.shared(a, b);
    if shared.len() < FEWEST_SHARED {
        return None;
    }
    let Some(_) = lens.scales else {
        return left_at(&shared, lens.at(a), lens.at(b));
    };
    // How much the picture grew was only read off it, and a camera that only turns gets the lens that
    // suits it best before anything is said to be left over.
    let (focal_a, focal_b) = (lens.at(a), lens.at(b));
    let left = |log: f64| left_at(&shared, focal_a, focal_b * log.exp()).unwrap_or(f64::INFINITY);
    let (mut low, mut high) = (-LENS_LEEWAY, LENS_LEEWAY);
    let golden = 0.618_033_988_75;
    let (mut x1, mut x2) = (high - golden * (high - low), low + golden * (high - low));
    let (mut e1, mut e2) = (left(x1), left(x2));
    for _ in 0..8 {
        if e1 < e2 {
            high = x2;
            x2 = x1;
            e2 = e1;
            x1 = high - golden * (high - low);
            e1 = left(x1);
        } else {
            low = x1;
            x1 = x2;
            e1 = e2;
            x2 = low + golden * (high - low);
            e2 = left(x2);
        }
    }
    Some(e1.min(e2).min(left(0.0)))
}

/// [`left_between`] of what two frames `shared`, at a focal length each.
fn left_at(shared: &[(u32, [f64; 2], [f64; 2])], focal_a: f64, focal: f64) -> Option<f64> {
    let ray = |at: [f64; 2], focal: f64| unit([at[0] / focal, at[1] / focal, 1.0]);
    let from: Vec<Vector> = shared.iter().map(|s| ray(s.1, focal_a)).collect();
    let to: Vec<Vector> = shared.iter().map(|s| ray(s.2, focal)).collect();
    let offs = |turn: &Matrix| -> Vec<f64> {
        from.iter()
            .zip(&to)
            .map(|(p, q)| length(minus(*q, apply(turn, *p))))
            .collect()
    };
    let last_quarter = |offs: &[f64]| {
        let mut sorted = offs.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() * 3 / 4]
    };
    let mut turn = views::best_turn(&from, &to)?;
    // Once more, past what is furthest off.
    let off = offs(&turn);
    let limit = last_quarter(&off) * 1.5;
    let (near_from, near_to): (Vec<Vector>, Vec<Vector>) = from
        .iter()
        .zip(&to)
        .zip(&off)
        .filter(|(_, off)| **off <= limit)
        .map(|((p, q), _)| (*p, *q))
        .unzip();
    if let Some(better) = views::best_turn(&near_from, &near_to) {
        turn = better;
    }
    Some(last_quarter(&offs(&turn)) * focal)
}

/// The most a camera that only turns leaves unexplained, between key frames as far apart as still
/// share enough, which is where depth shows most, past what the tracker may have wandered by then:
/// none if no frames are far apart.
fn left_by_turning(seen: &Seen, keys: &[usize], lens: Lens<'_>) -> Option<f64> {
    let step = (keys.len() / 24).max(1);
    keys.iter()
        .enumerate()
        .step_by(step)
        .filter_map(|(index, a)| {
            let b = keys[index + 1..]
                .iter()
                .rev()
                .find(|b| seen.shared(*a, **b).len() >= FEWEST_SHARED)?;
            let left = left_between(seen, *a, *b, lens)?;
            Some(left - WANDER_PER_FRAME * (*b - *a) as f64)
        })
        .max_by(f64::total_cmp)
}

/// The key frame that a start's reach from it shows the most depth.
fn where_depth_shows(seen: &Seen, keys: &[usize], lens: Lens<'_>) -> usize {
    let mut most = (0.0, 0);
    for (index, a) in keys.iter().enumerate() {
        let b = keys[(index + START_REACH - 1).min(keys.len() - 1)];
        let left = (b > *a).then(|| left_between(seen, *a, b, lens)).flatten();
        if let Some(left) = left {
            if left > most.0 {
                most = (left, index);
            }
        }
    }
    most.1
}

/// How far a lens may be from what the picture's growing says, as a logarithm, for a camera that only
/// turns.
const LENS_LEEWAY: f64 = 0.15;
/// How much a lens may have come apart from what the picture's growing says, where a camera also moves
/// along its line, before it is not what happened.
const MOST_FROM_GROWING: f64 = 2.0;
/// How much a camera that only turns can be left unexplained by, in pixels, past the tracker's wander.
const ONLY_TURNING_BELOW: f64 = 0.75;
/// How far a followed point wanders for each frame it is followed, in pixels, as all of them do the
/// same way: panning across a room, 0.0044 over 120 frames and 0.0047 over 480.
const WANDER_PER_FRAME: f64 = 0.006;
/// Less than this share of how far off things are, and a camera has not moved.
const LEAST_BASELINE: f64 = 0.01;
/// How far things must move over a shot to count as moving, in pixels.
const LEAST_MOTION: f64 = 1.0;

/// A reconstruction as it is written out.
#[derive(Clone)]
pub struct Solved {
    pub cameras: BTreeMap<usize, Camera>,
    /// What it placed: the frames it is first and last seen on, where it is first seen, where it is.
    pub points: Vec<((usize, usize), [f64; 2], Vector)>,
    pub lens_known: bool,
    /// How far sightings land from where they were: the mean, and all but the worst twentieth.
    pub error: f64,
    pub worst: f64,
    /// What share of all that was seen on solved frames agrees.
    pub agreeing: f64,
    /// Whether its lens changes over the shot.
    pub zooms: bool,
    /// Whether it is as well explained by a lens that zooms as by a camera moving along its line, which
    /// nothing in the picture tells apart.
    pub zoom_or_push: bool,
}

/// The most sightings may be off on average, in pixels, for a solve to be worth keeping.
const MOST_ERROR: f64 = 2.5;
/// The least share of what was seen that a solve must agree with.
const LEAST_AGREEING: f64 = 0.4;
/// How many key frames a search for the focal length reads.
const SEARCH_KEYS: usize = 10;

/// How far what `keys` share moves over the shot, at most, in pixels.
fn most_motion(seen: &Seen, keys: &[usize]) -> f64 {
    let mut most = 0.0f64;
    for (index, a) in keys.iter().enumerate() {
        for b in &keys[index + 1..] {
            let mut moved: Vec<f64> = seen
                .shared(*a, *b)
                .iter()
                .map(|s| (s.2[0] - s.1[0]).hypot(s.2[1] - s.1[1]))
                .collect();
            if moved.is_empty() {
                continue;
            }
            moved.sort_by(f64::total_cmp);
            most = most.max(moved[moved.len() / 2]);
        }
    }
    most
}

/// How much the picture must grow or shrink over a shot, at the most apart, to be asked whether a lens
/// did it.
const LEAST_ZOOM: f64 = 0.004;
/// As much as a picture growing is worth asking about: past this, what was read of it is not a lens.
const MOST_ZOOM: f64 = 20.0;
/// How much a focal length must change for a lens to be said to zoom.
const LEAST_FOCAL_CHANGE: f64 = 0.003;
/// How much it must change for one that does no better than one lens to be said not to be told from it.
const ZOOM_NOT_TOLD_FROM: f64 = 0.02;
/// How much of one lens's error a lens that zooms may leave, to stand for it.
const ZOOM_BETTER_BY: f64 = 0.8;
/// How much more than one lens's error it may leave and still be as good an account of the shot.
const AS_GOOD_BY: f64 = 1.1;

/// How much bigger the picture is on each frame than on the first, from how far apart what it shows is:
/// no move, no turn, and no roll changes that, but for what a move does by depth, which comes out even.
fn scales_over(seen: &Seen) -> Vec<f64> {
    let least = 0.05 * seen.width;
    let mut scales = vec![1.0; seen.frames];
    for frame in 1..seen.frames {
        let shared = seen.shared(frame - 1, frame);
        let half = shared.len() / 2;
        let mut ratios: Vec<f64> = (0..shared.len())
            .filter_map(|index| {
                let (a, b) = (&shared[index], &shared[(index + half) % shared.len()]);
                let before = (a.1[0] - b.1[0]).hypot(a.1[1] - b.1[1]);
                let after = (a.2[0] - b.2[0]).hypot(a.2[1] - b.2[1]);
                (before >= least).then(|| after / before)
            })
            .collect();
        ratios.sort_by(f64::total_cmp);
        let ratio = if ratios.len() >= FEWEST_SEEN {
            ratios[ratios.len() / 2]
        } else {
            1.0
        };
        scales[frame] = scales[frame - 1] * ratio;
    }
    scales
}

/// How far apart the most and the least of `values` are, as a share of the least.
fn spread(values: impl Iterator<Item = f64>) -> f64 {
    let (least, most) = values.fold((f64::MAX, 0.0f64), |(least, most), value| {
        (least.min(value), most.max(value))
    });
    most / least.max(1e-12) - 1.0
}

/// The shot with a lens that changes: each frame starts from how much the picture grew, and its own
/// focal length then goes wherever fits, against what stands at other depths.
fn with_a_zoom(followed: &Followed, scales: &[f64]) -> Result<Solved, Unsolved> {
    let found = one_lens(followed, None, Some(scales))?;
    let mut solved = written(
        &found.seen,
        &found.reconstruction,
        found.parallax,
        found.lens_known,
    )?;
    let from_growing = solved
        .cameras
        .iter()
        .map(|(frame, camera)| camera.focal / scales[*frame]);
    if spread(from_growing) > MOST_FROM_GROWING {
        return Err(Unsolved::DoesNotFit);
    }
    solved.zooms = spread(solved.cameras.values().map(|camera| camera.focal)) >= LEAST_FOCAL_CHANGE;
    Ok(solved)
}

/// Solve the camera of what was `followed`, at a focal length if it is known, which then holds. Else
/// with one lens; and where the picture grows or shrinks, also with a lens that changes, which stands
/// if one lens explains the shot clearly less well. Where each does as well, nothing tells them apart,
/// and the one lens stands and says so.
pub fn solve(followed: &Followed, known: Option<f64>) -> Result<Solved, Unsolved> {
    let one = one_lens(followed, known, None).and_then(|found| {
        written(
            &found.seen,
            &found.reconstruction,
            found.parallax,
            found.lens_known,
        )
    });
    if known.is_some() || followed.frames < 3 {
        return one;
    }
    let scales = scales_over(&Seen::new(followed));
    let grew = spread(scales.iter().copied());
    if !(LEAST_ZOOM..MOST_ZOOM).contains(&grew) {
        return one;
    }
    let says_so = |solved: Solved| Solved {
        zoom_or_push: true,
        ..solved
    };
    match (one, with_a_zoom(followed, &scales)) {
        (Ok(one), Ok(zoom)) => {
            if !zoom.zooms {
                Ok(one)
            } else if zoom.error < ZOOM_BETTER_BY * one.error {
                Ok(zoom)
            } else if zoom.error <= AS_GOOD_BY * one.error
                && spread(zoom.cameras.values().map(|camera| camera.focal)) >= ZOOM_NOT_TOLD_FROM
            {
                Ok(says_so(one))
            } else {
                Ok(one)
            }
        }
        // With the zoom taken out nothing shows any depth, and yet a move along its line fits.
        (Ok(one), Err(Unsolved::OnlyTurns | Unsolved::Still)) => Ok(says_so(one)),
        (Ok(one), Err(_)) => Ok(one),
        // One lens explains it by turning, and the picture's growing is what wide lenses do at the edges.
        (Err(Unsolved::OnlyTurns), _) => Err(Unsolved::OnlyTurns),
        (Err(_), Ok(zoom)) => Ok(zoom),
        (Err(_), Err(Unsolved::OnlyTurns | Unsolved::Still)) => Err(Unsolved::TurnsAndZooms),
        (Err(error), Err(_)) => Err(error),
    }
}

/// What one lens makes of the shot, as a shot that was not asked about a zoom would come out.
#[cfg(test)]
pub fn solve_one_lens(followed: &Followed) -> Result<Solved, Unsolved> {
    one_lens(followed, None, None).and_then(|found| {
        written(
            &found.seen,
            &found.reconstruction,
            found.parallax,
            found.lens_known,
        )
    })
}

/// A reconstruction, before it is written out.
struct OneLens {
    seen: Seen,
    reconstruction: Reconstruction,
    parallax: f64,
    lens_known: bool,
}

/// The reconstruction of what was `followed`, with one lens, known or searched for, or a base searched
/// for and `scales` of it to start each frame from.
fn one_lens(
    followed: &Followed,
    known: Option<f64>,
    scales: Option<&[f64]>,
) -> Result<OneLens, Unsolved> {
    let mut seen = Seen::new(followed);
    if seen.frames < 3 {
        return Err(Unsolved::TooShort);
    }
    let keys = key_frames(&seen);
    if keys.len() < 2 {
        return Err(Unsolved::NoDetail);
    }
    if most_motion(&seen, &keys) < LEAST_MOTION {
        return Err(Unsolved::Still);
    }
    // What a camera that only turns leaves unexplained, at the lens it would fit best with.
    let lens = |base: f64| Lens { base, scales };
    let left = |focal: f64| left_by_turning(&seen, &keys, lens(focal)).unwrap_or(f64::INFINITY);
    let turning = match known {
        Some(focal) => focal,
        None => search_focal(seen.width, &left).focal,
    };
    let parallax = left_by_turning(&seen, &keys, lens(turning));
    if parallax.is_some_and(|parallax| parallax < ONLY_TURNING_BELOW) {
        return Err(Unsolved::OnlyTurns);
    }
    // The keys the search reads begin where things show most depth.
    let begin =
        where_depth_shows(&seen, &keys, lens(turning)).min(keys.len().saturating_sub(SEARCH_KEYS));
    let some = &keys[begin..(begin + SEARCH_KEYS).min(keys.len())];
    let (focal, lens_known) = match known {
        Some(focal) => (focal, true),
        None => {
            let error = |focal: f64| {
                best_reconstruction(&seen, some, 0, lens(focal))
                    .map_or(f64::INFINITY, |found| found.0)
            };
            let search = search_focal(seen.width, &error);
            if search.told {
                (search.focal, true)
            } else {
                (rest_focal(followed.height), false)
            }
        }
    };
    let (_, mut reconstruction) =
        best_reconstruction(&seen, &keys, begin, lens(focal)).ok_or(Unsolved::DoesNotFit)?;
    let let_go = if scales.is_some() {
        Focal::Own
    } else if lens_known && known.is_none() {
        Focal::Shared
    } else {
        Focal::Held
    };
    let frames: Vec<usize> = reconstruction.cameras.keys().copied().collect();
    reconstruction.adjusted(&seen, &frames, let_go, 40);
    reconstruction.rejected(&seen);
    // How far paths wander, from the median: for 2D Gaussian noise of deviation s the median distance
    // is 1.1774 s. Then what moves by itself falls out.
    let mut errors = reconstruction.errors(&seen);
    if errors.len() > 20 {
        errors.sort_by(f64::total_cmp);
        let deviation = errors[errors.len() / 2] / 1.1774;
        seen.noise = (1.5 * deviation).clamp(0.05, seen.noise);
        reconstruction.outliers.clear();
        reconstruction.rejected(&seen);
    }
    reconstruction.triangulated(&seen, 1.0);
    reconstruction.adjusted(&seen, &frames, let_go, 40);
    reconstruction.rejected(&seen);
    reconstruction.adjusted(&seen, &frames, let_go, 20);
    // The frames between, from the points, each by itself.
    let between: Vec<usize> = (0..seen.frames)
        .filter(|frame| !reconstruction.cameras.contains_key(frame))
        .collect();
    // Between the key frames a lens that changes goes straight, with how much the picture grew taken out.
    let focal_between = |frame: usize| {
        let scales = scales?;
        let log = |(key, camera): (&usize, &Camera)| (*key, (camera.focal / scales[*key]).ln());
        let cameras = &reconstruction.cameras;
        let before = cameras.range(..frame).next_back().map(log);
        let after = cameras.range(frame + 1..).next().map(log);
        let log = match (before, after) {
            (Some((a, la)), Some((b, lb))) => la + (lb - la) * (frame - a) as f64 / (b - a) as f64,
            (Some((_, only)), None) | (None, Some((_, only))) => only,
            (None, None) => return None,
        };
        Some(log.exp() * scales[frame])
    };
    let found: Vec<(usize, Option<(Camera, Vec<u32>)>)> = between
        .par_iter()
        .map(|frame| {
            let from = guess(&reconstruction, *frame).map(|camera| Camera {
                focal: focal_between(*frame).unwrap_or(camera.focal),
                ..camera
            });
            let found = from.and_then(|from| reconstruction.resected(&seen, *frame, from, false));
            (*frame, found)
        })
        .collect();
    for (frame, found) in found {
        if let Some((camera, off)) = found {
            reconstruction.cameras.insert(frame, camera);
            reconstruction
                .outliers
                .extend(off.into_iter().map(|path| (path, frame)));
        }
    }
    reconstruction.rejected(&seen);
    Ok(OneLens {
        seen,
        reconstruction,
        parallax: parallax.unwrap_or(f64::INFINITY),
        lens_known,
    })
}

/// The focal length of the camera a document starts from, for frames of that `height`.
fn rest_focal(height: f64) -> f64 {
    height / 2.0 / (super::REST_FOV.to_radians() / 2.0).tan()
}

fn written(
    seen: &Seen,
    reconstruction: &Reconstruction,
    parallax: f64,
    lens_known: bool,
) -> Result<Solved, Unsolved> {
    let mut errors = reconstruction.errors(seen);
    if errors.is_empty() {
        return Err(Unsolved::DoesNotFit);
    }
    errors.sort_by(f64::total_cmp);
    let error = errors.iter().sum::<f64>() / errors.len() as f64;
    let worst = errors[(errors.len() * 19 / 20).min(errors.len() - 1)];
    let all: usize = reconstruction
        .cameras
        .keys()
        .map(|frame| seen.by_frame[*frame].len())
        .sum();
    let agreeing = errors.len() as f64 / all.max(1) as f64;
    if error > MOST_ERROR || agreeing < LEAST_AGREEING {
        return Err(Unsolved::DoesNotFit);
    }
    // A reconstruction that explains no more than turning does, or a camera that hardly moved for how
    // far off things are, says nothing of depth.
    let places: Vec<Vector> = reconstruction
        .cameras
        .values()
        .map(|camera| camera.view.place)
        .collect();
    let baseline = places
        .iter()
        .flat_map(|a| places.iter().map(move |b| length(minus(*a, *b))))
        .fold(0.0, f64::max);
    let mut distances: Vec<f64> = reconstruction
        .points
        .values()
        .map(|point| length(minus(*point, places[0])))
        .collect();
    distances.sort_by(f64::total_cmp);
    let distance = distances.get(distances.len() / 2).copied().unwrap_or(1.0);
    if baseline / distance.max(1e-12) < LEAST_BASELINE || parallax <= 1.25 * error {
        return Err(Unsolved::OnlyTurns);
    }
    // Where each is first seen, for finding it, and how long it is seen, for how far to trust it.
    let points = reconstruction
        .points
        .iter()
        .filter_map(|(path, point)| {
            let (start, at) = &seen.paths[*path as usize];
            let usable = |step: &usize| {
                let frame = start + step;
                reconstruction.cameras.contains_key(&frame)
                    && !reconstruction.outliers.contains(&(*path, frame))
            };
            let first = (0..at.len()).find(usable)?;
            let last = (0..at.len()).rev().find(usable)?;
            Some(((start + first, start + last), at[first], *point))
        })
        .collect();
    Ok(Solved {
        cameras: reconstruction.cameras.clone(),
        points,
        lens_known,
        error,
        worst,
        agreeing,
        zooms: false,
        zoom_or_push: false,
    })
}
