//! Where the camera went: a solve of a shot, from points followed through it, into what
//! `gen=camera@1` says, so that a clip with `three_d=true` under it stays on what the picture shows.
//!
//! Kerf keeps no solve of its own. What this finds is plain keyframes in the document and a list of
//! places in the world to pin clips to. The world is the document's: x runs right and y down in frame
//! widths and heights from the middle, and z runs away from the viewer in frame heights. The solve's
//! freedom is spent so that its first frame is the 2D picture, as a camera at rest shows it: straight
//! on, standing as far back as its lens makes the plane z = 0 fill the frame, with z = 0 at the middle
//! depth of what it sees. So a clip placed where a thing shows on that frame, at z = 0, is there and
//! about as far away.
//!
//! EffectCraft's camera tracker, in Kerf's terms ([`reconstruct`]). Lens distortion is left out, having
//! no place in a document. A lens that changes during a shot is solved where what is in the picture
//! lies at enough depths to tell it from a camera moving along its line, which grows near things more
//! than far ones where a zoom grows all alike. Where nothing can tell them apart it says so.

mod adjust;
mod follow;
mod math;
mod reconstruct;
#[cfg(test)]
mod scene;
#[cfg(test)]
mod tests;
mod views;

use math::{transposed, Matrix};
use std::collections::BTreeMap;

use follow::Followed;

/// The field of view a camera at rest stands back for, in degrees: `gen=camera@1`'s default. A camera
/// is that far behind z = 0 before its own `z`.
pub const REST_FOV: f64 = 40.0;

/// How far a clip or a camera can be from the middle of the frame, in `x` and `y`, by the catalog.
const MOST_FROM_MIDDLE: f64 = 10.0;
/// How far one can be in `z`, by the catalog.
const MOST_AWAY: f64 = 1000.0;

/// Why a shot's camera could not be solved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsolved {
    /// Too few frames to say anything.
    TooShort,
    /// Too little in the picture to hold on to.
    NoDetail,
    /// Nothing in the picture moves.
    Still,
    /// The camera only turns, and without moving nothing tells how far away anything is.
    OnlyTurns,
    /// The camera only turns, and its lens zooms.
    TurnsAndZooms,
    /// No one camera explains what the picture does.
    DoesNotFit,
}

impl std::fmt::Display for Unsolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooShort => "the range is too short to solve: it takes at least three frames",
            Self::NoDetail => {
                "the picture has too little detail to follow, so the camera cannot be solved"
            }
            Self::Still => "nothing in the picture moves, so there is no camera move to solve",
            Self::OnlyTurns => {
                "the camera only turns, as on a tripod: without it moving nothing shows how far away \
                 anything is, so it cannot be solved. A pan is pinned to with `corners` instead"
            }
            Self::TurnsAndZooms => {
                "the camera only turns and its lens zooms: without it moving nothing shows how far \
                 away anything is, so it cannot be solved"
            }
            Self::DoesNotFit => {
                "no one camera explains what the picture does: more of it may be moving by itself \
                 than standing still, or the lens may bend lines"
            }
        })
    }
}

/// The camera on one frame, in `gen=camera@1`'s terms: `x` in frame widths, `y` and `z` in frame
/// heights, the rest in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolvedCamera {
    /// Which of the frames it is.
    pub frame: usize,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub pan: f64,
    pub tilt: f64,
    pub roll: f64,
    /// Down the frame, in degrees.
    pub fov: f64,
}

/// A place in the world that something in the picture stands at: a clip's `x`, `y`, and `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolvedPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The frame it is first seen on, and where, in fractions of the frame.
    pub frame: usize,
    pub seen_at: [f64; 2],
    /// The last frame it is seen on. The longer it is seen, the surer where it is.
    pub last_frame: usize,
}

/// A shot's camera.
#[derive(Clone, Debug, PartialEq)]
pub struct SolvedShot {
    /// One for each frame it could be found on, in order.
    pub cameras: Vec<SolvedCamera>,
    /// Down the frame, in degrees, on its first frame.
    pub fov: f64,
    /// Whether the lens changes over the shot, so that each camera's `fov` is its own.
    pub zooms: bool,
    /// Whether the picture cannot tell a zoom from a camera moving along its line, each explaining it
    /// as well: this says what one lens makes of it, a move.
    pub zoom_or_push: bool,
    /// Whether the picture says what the lens is. A camera that never turns does not, and any lens
    /// fits: `fov` is then the default.
    pub lens_known: bool,
    pub points: Vec<SolvedPoint>,
    /// How far what was followed lands from where the solve puts it, in pixels of the frames read: the
    /// mean, and all but the worst twentieth.
    pub error: f64,
    pub worst: f64,
    /// What share of all that was seen agrees with it.
    pub agreeing: f64,
    pub width: f64,
    pub height: f64,
    /// The frame it could go no further at: a cut, one that could not be had, or one that shows too
    /// little of what came before.
    pub ended_at: Option<usize>,
}

/// Solve the camera of the `count` frames `frame` gives as `(width, height, rgba)`, with `fov`, down
/// the frame in degrees, if the lens is known.
pub fn solve(
    count: usize,
    fov: Option<f64>,
    frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> Result<SolvedShot, Unsolved> {
    solve_followed(&follow::follow(count, frame), fov)
}

/// Solve the camera of what was followed.
fn solve_followed(followed: &Followed, fov: Option<f64>) -> Result<SolvedShot, Unsolved> {
    let known = fov.map(|fov| focal_of_fov(followed.height, fov));
    let solved = reconstruct::solve(followed, known)?;
    written(followed, &solved).ok_or(Unsolved::DoesNotFit)
}

fn focal_of_fov(height: f64, fov: f64) -> f64 {
    height / 2.0 / (fov.clamp(1.0, 170.0).to_radians() / 2.0).tan()
}

fn fov_of_focal(height: f64, focal: f64) -> f64 {
    (2.0 * (height / 2.0 / focal).atan()).to_degrees()
}

/// A camera's pan, tilt, and roll in degrees, as the compositor turns one: `turn`'s rows are its
/// right, its down, and its forward, in the world.
fn angles_of(turn: &Matrix) -> [f64; 3] {
    let forward = turn[2];
    [
        forward[0].atan2(forward[2]).to_degrees(),
        (-forward[1]).clamp(-1.0, 1.0).asin().to_degrees(),
        turn[0][1].atan2(turn[1][1]).to_degrees(),
    ]
}

/// `angle` by whole turns to wherever is nearest `near`.
fn nearest_turn(angle: f64, near: f64) -> f64 {
    angle + ((near - angle) / 360.0).round() * 360.0
}

/// The reconstruction in the document's world: its first camera at rest, and the middle depth of what
/// that sees at z = 0.
fn written(followed: &Followed, solved: &reconstruct::Solved) -> Option<SolvedShot> {
    let (width, height) = (followed.width, followed.height);
    let first = *solved.cameras.values().next()?;
    let focal = first.focal;
    let mut depths: Vec<f64> = solved
        .points
        .iter()
        .map(|(_, _, point)| first.view.seen(*point)[2])
        .filter(|depth| *depth > 0.0)
        .collect();
    depths.sort_by(f64::total_cmp);
    let depth = depths
        .get(depths.len() / 2)
        .copied()
        .filter(|d| *d > 1e-12)?;
    // Pixels of the world for each of the reconstruction's. A long way may not fit what `x` and `y`
    // can say at that, and then what it sees is nearer than z = 0.
    let (mut across, mut down) = (0.0f64, 0.0f64);
    for camera in solved.cameras.values() {
        let seen = first.view.seen(camera.view.place);
        across = across.max(seen[0].abs());
        down = down.max(seen[1].abs());
    }
    let scale = (focal / depth)
        .min(MOST_FROM_MIDDLE * width / across.max(1e-12))
        .min(MOST_FROM_MIDDLE * height / down.max(1e-12));
    let to_world = |point: math::Vector| {
        let seen = first.view.seen(point);
        [seen[0] * scale, seen[1] * scale, seen[2] * scale - focal]
    };
    let rest = focal_of_fov(height, REST_FOV);
    let turned = transposed(&first.view.turn);
    let mut cameras: Vec<SolvedCamera> = Vec::with_capacity(solved.cameras.len());
    let mut last = [0.0, 0.0];
    for (frame, camera) in &solved.cameras {
        let eye = to_world(camera.view.place);
        let [pan, tilt, roll] = angles_of(&math::product(&camera.view.turn, &turned));
        let (pan, roll) = (nearest_turn(pan, last[0]), nearest_turn(roll, last[1]));
        last = [pan, roll];
        cameras.push(SolvedCamera {
            frame: *frame,
            x: eye[0] / width,
            y: eye[1] / height,
            z: (eye[2] + rest) / height,
            pan,
            tilt,
            roll,
            fov: fov_of_focal(height, camera.focal),
        });
    }
    let cameras_end = cameras.last().map(|camera| camera.frame + 1);
    let points = solved
        .points
        .iter()
        .map(|((frame, last_frame), at, point)| {
            let world = to_world(*point);
            SolvedPoint {
                x: world[0] / width,
                y: world[1] / height,
                z: world[2] / height,
                frame: *frame,
                seen_at: [at[0] / width + 0.5, at[1] / height + 0.5],
                last_frame: *last_frame,
            }
        })
        .filter(|point: &SolvedPoint| {
            point.x.abs() <= MOST_FROM_MIDDLE
                && point.y.abs() <= MOST_FROM_MIDDLE
                && point.z.abs() <= MOST_AWAY
        })
        .collect();
    Some(SolvedShot {
        cameras,
        fov: fov_of_focal(height, focal),
        zooms: solved.zooms,
        zoom_or_push: solved.zoom_or_push,
        lens_known: solved.lens_known,
        points,
        error: solved.error,
        worst: solved.worst,
        agreeing: solved.agreeing,
        width,
        height,
        // Where following it stopped, or sooner, where the solve could go no further.
        ended_at: cameras_end
            .filter(|end| *end < followed.frames)
            .or(followed.ended_at),
    })
}

impl SolvedShot {
    /// Where `point` shows through `camera`, in pixels of the frames read: none behind it.
    pub fn shows_at(&self, camera: &SolvedCamera, point: &SolvedPoint) -> Option<[f64; 2]> {
        let (w, h) = (self.width, self.height);
        let turn = turn_of(camera);
        let eye = [
            camera.x * w,
            camera.y * h,
            camera.z * h - focal_of_fov(h, REST_FOV),
        ];
        let seen = math::apply(
            &turn,
            math::minus([point.x * w, point.y * h, point.z * h], eye),
        );
        if seen[2] <= 1e-6 {
            return None;
        }
        let focal = focal_of_fov(h, camera.fov);
        Some([
            focal * seen[0] / seen[2] + w / 2.0,
            focal * seen[1] / seen[2] + h / 2.0,
        ])
    }

    /// The indexes of the cameras that keep the way within `tolerance` pixels of the whole, first and
    /// last among them, as far as what the solve found shows: the rest are written between them.
    pub fn simplified(&self, tolerance: f64) -> Vec<usize> {
        let count = self.cameras.len();
        if count <= 2 {
            return (0..count).collect();
        }
        // What it found, a fair spread of it, is what must not stray.
        let step = (self.points.len() / 48).max(1);
        let watched: Vec<&SolvedPoint> = self.points.iter().step_by(step).collect();
        let mut keep = vec![false; count];
        keep[0] = true;
        keep[count - 1] = true;
        let mut spans = vec![(0, count - 1)];
        while let Some((low, high)) = spans.pop() {
            let (a, b) = (&self.cameras[low], &self.cameras[high]);
            let mut worst = (0.0, None);
            for index in low + 1..high {
                let true_camera = &self.cameras[index];
                let t = (true_camera.frame - a.frame) as f64 / (b.frame - a.frame) as f64;
                let mix = |p: f64, q: f64| p + (q - p) * t;
                let written = SolvedCamera {
                    frame: true_camera.frame,
                    x: mix(a.x, b.x),
                    y: mix(a.y, b.y),
                    z: mix(a.z, b.z),
                    pan: mix(a.pan, b.pan),
                    tilt: mix(a.tilt, b.tilt),
                    roll: mix(a.roll, b.roll),
                    fov: mix(a.fov, b.fov),
                };
                let off = watched
                    .iter()
                    .filter_map(|point| {
                        let there = self.shows_at(true_camera, point)?;
                        let here = self.shows_at(&written, point)?;
                        Some((there[0] - here[0]).hypot(there[1] - here[1]))
                    })
                    .fold(0.0, f64::max);
                if off > worst.0 {
                    worst = (off, Some(index));
                }
            }
            if let (off, Some(index)) = worst {
                if off > tolerance {
                    keep[index] = true;
                    spans.push((low, index));
                    spans.push((index, high));
                }
            }
        }
        (0..count).filter(|index| keep[*index]).collect()
    }

    /// Up to `most` of the points it found, the longest seen of them in each part of the frame they
    /// show in first, for choosing one to pin something to.
    pub fn spread_points(&self, most: usize) -> Vec<SolvedPoint> {
        let columns = (most as f64 * 16.0 / 9.0).sqrt().ceil().max(1.0) as usize;
        let rows = most.div_ceil(columns).max(1);
        let mut cells: BTreeMap<(usize, usize), SolvedPoint> = BTreeMap::new();
        for point in &self.points {
            let cell = (
                ((point.seen_at[1].clamp(0.0, 0.999)) * rows as f64) as usize,
                ((point.seen_at[0].clamp(0.0, 0.999)) * columns as f64) as usize,
            );
            let long = |point: &SolvedPoint| point.last_frame - point.frame;
            if cells.get(&cell).is_none_or(|kept| long(point) > long(kept)) {
                cells.insert(cell, *point);
            }
        }
        cells.into_values().take(most).collect()
    }
}

/// How `gen=camera@1` turns: about y by `pan`, about x by `tilt`, and about its own line by `roll`.
/// The rows are its right, its down, and its forward.
fn turn_of(camera: &SolvedCamera) -> Matrix {
    let (sp, cp) = camera.tilt.to_radians().sin_cos();
    let (sy, cy) = camera.pan.to_radians().sin_cos();
    let (sr, cr) = camera.roll.to_radians().sin_cos();
    let level_right = [cy, 0.0, -sy];
    let level_down = [sy * sp, cp, cy * sp];
    let forward = [sy * cp, -sp, cy * cp];
    let right = math::add(math::times(level_right, cr), math::times(level_down, sr));
    let down = math::add(math::times(level_down, cr), math::times(level_right, -sr));
    [right, down, forward]
}
