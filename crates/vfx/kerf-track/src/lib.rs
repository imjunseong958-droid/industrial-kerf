//! Following what moves in a picture. [`follow`] tracks a point through frames with pyramidal
//! Lucas-Kanade and a forward-backward check, and [`simplified`] keeps only the points that shape
//! its way, for writing it as keyframes. Kerf keeps no tracking data of its own: what a track finds
//! becomes plain keyframes in the document, which merge, read, and edit like any others.

pub mod planar;
pub mod roto;

pub use planar::{follow_plane, simplified_quads, FollowedPlane, Quad};
pub use roto::{follow_object, simplified_outlines, FollowedObject, Hints};

use kerf_raster::klt::{track, GrayPyramid, LkOpts};
use kerf_raster::plane::Plane;

/// How many levels the search looks through: a point can jump far between frames.
const PYRAMID_LEVELS: usize = 4;

/// How alike the patch around a point must stay to the one it started as, or it counts as lost.
const LOST_BELOW: f64 = 0.3;

/// Half the side of the patch compared with where the point started, in pixels.
const PATCH_RADIUS: isize = 7;

/// Where a followed point is in one frame, in fractions of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    /// How alike the patch here is to the one at the start, from -1 to 1.
    pub confidence: f64,
}

/// A followed point's way through the frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followed {
    /// One a frame from the first, up to where it was lost.
    pub points: Vec<Point>,
    /// The frame it could no longer be found in.
    pub lost_at: Option<usize>,
}

fn pyramid(width: usize, height: usize, rgba: &[u8]) -> GrayPyramid {
    let mut plane = Plane::new(width, height, 1);
    for (value, pixel) in plane.data.iter_mut().zip(rgba.chunks_exact(4)) {
        *value = (0.2126 * f32::from(pixel[0])
            + 0.7152 * f32::from(pixel[1])
            + 0.0722 * f32::from(pixel[2]))
            / 255.0;
    }
    GrayPyramid::from_plane(plane, PYRAMID_LEVELS)
}

/// The patch around `center`, in level 0 of `pyramid`.
fn patch(pyramid: &GrayPyramid, center: [f64; 2]) -> Vec<f32> {
    let Some(plane) = pyramid.levels.first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut value = [0.0f32];
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            plane.sample(center[0] + dx as f64, center[1] + dy as f64, &mut value);
            out.push(value[0]);
        }
    }
    out
}

/// Normalized cross-correlation of two patches: 1 alike, 0 unrelated.
fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len()).max(1) as f64;
    let mean = |v: &[f32]| v.iter().map(|x| f64::from(*x)).sum::<f64>() / n;
    let (ma, mb) = (mean(a), mean(b));
    let (mut sum, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x) - ma, f64::from(*y) - mb);
        sum += x * y;
        va += x * x;
        vb += y * y;
    }
    if va <= 1e-12 || vb <= 1e-12 {
        // A flat patch on either side says nothing of the other.
        return if va <= 1e-12 && vb <= 1e-12 { 1.0 } else { 0.0 };
    }
    sum / (va * vb).sqrt()
}

/// Follow the point at `start`, in fractions of the frame, through `count` frames that `frame` gives
/// as `(width, height, rgba)`. It stops at the frame the point is lost in, and at one that `frame`
/// cannot give.
pub fn follow(
    count: usize,
    start: [f64; 2],
    mut frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> Followed {
    let mut followed = Followed::default();
    let Some((width, height, rgba)) = frame(0) else {
        return followed;
    };
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return followed;
    }
    let (w, h) = (width as f64, height as f64);
    let mut at = [start[0] * w, start[1] * h];
    let mut previous = pyramid(width, height, &rgba);
    let template = patch(&previous, at);
    followed.points.push(Point {
        x: start[0],
        y: start[1],
        confidence: 1.0,
    });
    let options = LkOpts::default();
    let mut moved = [0.0f64; 2];
    for index in 1..count {
        let Some((frame_w, frame_h, rgba)) = frame(index) else {
            followed.lost_at = Some(index);
            return followed;
        };
        if (frame_w, frame_h) != (width, height) {
            followed.lost_at = Some(index);
            return followed;
        }
        let next = pyramid(width, height, &rgba);
        // It tends to go on as it was going, which helps across a fast move.
        let found = track(&previous, &next, &[at], Some(&[moved]), &options)
            .first()
            .copied()
            .flatten()
            // A guess that overshot a change of direction: look again from where it was.
            .or_else(|| {
                track(&previous, &next, &[at], None, &options)
                    .first()
                    .copied()
                    .flatten()
            });
        let Some(found) = found else {
            followed.lost_at = Some(index);
            return followed;
        };
        let confidence = correlation(&template, &patch(&next, found));
        if confidence < LOST_BELOW {
            followed.lost_at = Some(index);
            return followed;
        }
        moved = [found[0] - at[0], found[1] - at[1]];
        at = found;
        previous = next;
        followed.points.push(Point {
            x: at[0] / w,
            y: at[1] / h,
            confidence,
        });
    }
    followed
}

/// The indexes of the points that keep the way within `tolerance` of the whole, first and last among
/// them (Ramer-Douglas-Peucker, with time as the measure along it).
pub fn simplified(points: &[Point], tolerance: f64) -> Vec<usize> {
    if points.len() <= 2 {
        return (0..points.len()).collect();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((low, high)) = spans.pop() {
        let mut worst = (0.0, None);
        for index in low + 1..high {
            let t = (index - low) as f64 / (high - low) as f64;
            let dx = points[low].x + (points[high].x - points[low].x) * t - points[index].x;
            let dy = points[low].y + (points[high].y - points[low].y) * t - points[index].y;
            let off = dx.abs().max(dy.abs());
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
    (0..points.len()).filter(|index| keep[*index]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 320;
    const H: usize = 180;

    /// A textured picture with a soft bright blob at `center`, in pixels.
    fn picture(center: [f64; 2]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let (dx, dy) = (x as f64 + 0.5 - center[0], y as f64 + 0.5 - center[1]);
                let blob = (-(dx * dx + dy * dy) / 40.0).exp();
                let ground = 0.1 + 0.05 * ((x / 13 + y / 11) % 2) as f64;
                let value = ((ground + blob * 0.85) * 255.0) as u8;
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        rgba
    }

    #[test]
    fn it_follows_a_moving_point_to_within_a_pixel() {
        let way = |index: usize| {
            let t = index as f64;
            [60.0 + t * 3.1, 90.0 + 30.0 * (t * 0.13).sin()]
        };
        let start = way(0);
        let followed = follow(48, [start[0] / W as f64, start[1] / H as f64], |index| {
            Some((W, H, picture(way(index))))
        });
        assert_eq!(followed.lost_at, None);
        assert_eq!(followed.points.len(), 48);
        for (index, point) in followed.points.iter().enumerate() {
            let [x, y] = way(index);
            let off = (point.x * W as f64 - x).hypot(point.y * H as f64 - y);
            assert!(off < 1.0, "frame {index} is {off} off");
            assert!(point.confidence > 0.5, "{}", point.confidence);
        }
    }

    #[test]
    fn it_keeps_up_with_a_fast_move_and_a_turn() {
        let way = |index: usize| {
            let t = index as f64;
            let there = if index < 12 {
                t * 14.0
            } else {
                168.0 - (t - 12.0) * 14.0
            };
            [70.0 + there, 90.0]
        };
        let followed = follow(24, [way(0)[0] / W as f64, 0.5], |index| {
            Some((W, H, picture(way(index))))
        });
        assert_eq!(followed.lost_at, None);
        let last = followed.points[23];
        assert!((last.x * W as f64 - way(23)[0]).abs() < 2.0, "{last:?}");
    }

    #[test]
    fn it_says_where_it_lost_a_point_that_leaves_or_vanishes() {
        // Leaving the frame.
        let way = |index: usize| [290.0 + index as f64 * 8.0, 90.0];
        let gone = follow(30, [way(0)[0] / W as f64, 0.5], |index| {
            Some((W, H, picture(way(index))))
        });
        assert!(gone.lost_at.is_some());
        assert!(gone.points.len() < 30);
        // Cut to something else.
        let cut = follow(10, [0.5, 0.5], |index| {
            let center = if index < 5 {
                [160.0, 90.0]
            } else {
                [40.0, 30.0]
            };
            Some((W, H, picture(center)))
        });
        assert_eq!(cut.lost_at, Some(5));
    }

    #[test]
    fn frames_that_cannot_be_had_end_it() {
        let none = follow(5, [0.5, 0.5], |_| None);
        assert!(none.points.is_empty());
        let short = follow(5, [0.5, 0.5], |index| {
            (index < 2).then(|| (W, H, picture([160.0, 90.0])))
        });
        assert_eq!(short.lost_at, Some(2));
        assert_eq!(short.points.len(), 2);
    }

    fn point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            confidence: 1.0,
        }
    }

    #[test]
    fn a_straight_way_needs_only_its_ends() {
        let straight: Vec<Point> = (0..100)
            .map(|index| point(index as f64 * 0.001, index as f64 * 0.002))
            .collect();
        assert_eq!(simplified(&straight, 0.0005), [0, 99]);
    }

    #[test]
    fn a_turn_keeps_its_corner_and_nothing_strays_past_the_tolerance() {
        let bent: Vec<Point> = (0..100)
            .map(|index| {
                let t = index as f64;
                point(t.min(50.0) * 0.01, (t - 50.0).max(0.0) * 0.01)
            })
            .collect();
        assert_eq!(simplified(&bent, 0.001), [0, 50, 99]);
        let wavy: Vec<Point> = (0..200)
            .map(|index| point(index as f64 * 0.005, (index as f64 * 0.1).sin() * 0.1))
            .collect();
        let kept = simplified(&wavy, 0.002);
        assert!(kept.len() < 60, "{}", kept.len());
        for pair in kept.windows(2) {
            for index in pair[0]..pair[1] {
                let t = (index - pair[0]) as f64 / (pair[1] - pair[0]) as f64;
                let y = wavy[pair[0]].y + (wavy[pair[1]].y - wavy[pair[0]].y) * t;
                assert!((y - wavy[index].y).abs() <= 0.0021);
            }
        }
    }

    #[test]
    fn short_ways_are_kept_whole() {
        assert_eq!(simplified(&[], 0.1), Vec::<usize>::new());
        assert_eq!(simplified(&[point(0.0, 0.0)], 0.1), [0]);
    }

    /// A white square on a flat red ground: nothing to hold on to but its corners.
    fn square(left: f64) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let inside =
                    (x as f64) >= left && (x as f64) < left + 32.0 && (81..99).contains(&y);
                rgba.extend_from_slice(if inside {
                    &[255, 255, 255, 255]
                } else {
                    &[200, 0, 0, 255]
                });
            }
        }
        rgba
    }

    #[test]
    fn it_holds_on_to_a_corner_of_a_flat_shape() {
        let way = |index: usize| 80.0 + index as f64 * 4.0;
        let followed = follow(25, [way(0) / W as f64, 81.0 / H as f64], |index| {
            Some((W, H, square(way(index))))
        });
        assert_eq!(followed.lost_at, None, "{:?}", followed.points.last());
        let last = followed.points[24];
        assert!((last.x * W as f64 - way(24)).abs() < 1.5, "{last:?}");
        assert!((last.y * H as f64 - 81.0).abs() < 1.5, "{last:?}");
    }

    #[test]
    fn a_flat_area_has_nothing_to_follow() {
        let followed = follow(5, [0.1, 0.1], |_| Some((W, H, square(80.0))));
        assert_eq!(followed.lost_at, Some(1));
    }
}
