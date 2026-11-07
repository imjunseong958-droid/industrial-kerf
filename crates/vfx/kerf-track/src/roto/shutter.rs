//! What a shutter does to an outline.
//!
//! A subject under an open shutter is smeared along the way it goes: a pixel is covered for the share of
//! the time the subject was over it, and light adds up, so across an edge what is seen is a straight ramp
//! as long as how far that edge went while the shutter was open. What is covered more than half of that
//! time is the subject at the middle of it, less what is narrower than half the blur, which is a sliver.
//!
//! A cut does not stop there. It goes by color in code values, in which a little of a bright subject's
//! light is a lot, so it takes in nearly all of the blur: on what is plainly apart from what is behind it,
//! out to 44% of the blur's length from the middle, where the end is at 50%. So what is half covered is the
//! cut drawn in along the way it went, by as far at the front as at the rear. That holds around a turn as along a straight edge, where
//! the measure across an edge (`soft.rs`) goes only so far, and where a cut is drawn in, a point of its
//! outline does not go to one on the other's: so this is done to a picture of the cut, not to its corners.
//! The measure is used where it is sound, to find how far the cut is from the middle and how long the
//! shutter was open, which is a property of the camera, and the motion says the rest.

use super::soft::{self, Across};
use rayon::prelude::*;

/// What was found of a shot's shutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shutter {
    /// How much of a frame it is open for.
    pub open: f64,
    /// How far what is half covered lies out from the cut, for each pixel an edge goes in a frame. Below
    /// 0 where the cut takes in more than that, as it does of a subject brighter than what is behind it.
    pub middle: f64,
}

/// How wide a mask's soft edge is, to draw best the straight ramp of a blur one long: its fall-off is an
/// S rather than a line, and steeper in the middle.
const FEATHER_OF_RAMP: f64 = 1.33;

/// A turn, a change of size, and a way, taking one frame's corners to the next's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Step {
    from: [f64; 2],
    to: [f64; 2],
    c: f64,
    s: f64,
}

impl Step {
    fn between(corners: &[[f64; 2]], next: &[[f64; 2]]) -> Step {
        let n = corners.len().max(1) as f64;
        let middle = |of: &[[f64; 2]]| {
            let (mut x, mut y) = (0.0, 0.0);
            for at in of {
                x += at[0];
                y += at[1];
            }
            [x / n, y / n]
        };
        let (from, to) = (middle(corners), middle(next));
        let (mut dot, mut cross, mut norm) = (0.0, 0.0, 0.0);
        for (p, q) in corners.iter().zip(next) {
            let (px, py) = (p[0] - from[0], p[1] - from[1]);
            let (qx, qy) = (q[0] - to[0], q[1] - to[1]);
            dot += px * qx + py * qy;
            cross += px * qy - py * qx;
            norm += px * px + py * py;
        }
        Step {
            from,
            to,
            c: dot / norm.max(1e-9),
            s: cross / norm.max(1e-9),
        }
    }

    /// How far a point at `p` goes.
    fn at(&self, p: [f64; 2]) -> [f64; 2] {
        let (x, y) = (p[0] - self.from[0], p[1] - self.from[1]);
        [
            self.to[0] + self.c * x - self.s * y - p[0],
            self.to[1] + self.s * x + self.c * y - p[1],
        ]
    }
}

/// What a frame is going, by the frames either side of it where it has them: what is seen of a frame is at
/// the middle of its exposure, as it is of the frame before and the frame after.
#[derive(Clone, Copy, Debug)]
pub(super) struct Going {
    before: Option<Step>,
    after: Option<Step>,
    /// Whether the way it came and the way it goes on disagree, so that the frames around it say nothing
    /// of how it went. At either end of a shot, the two steps nearest it.
    turned: bool,
}

impl Going {
    pub(super) fn at(&self, p: [f64; 2]) -> [f64; 2] {
        match (self.before, self.after) {
            (Some(a), Some(b)) => {
                let (a, b) = (a.at(p), b.at(p));
                [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
            }
            (Some(only), None) | (None, Some(only)) => only.at(p),
            (None, None) => [0.0, 0.0],
        }
    }
}

pub(super) fn going(frames: &[Vec<[f64; 2]>]) -> Vec<Going> {
    let steps: Vec<Step> = frames
        .windows(2)
        .map(|pair| Step::between(&pair[0], &pair[1]))
        .collect();
    let disagree = |a: usize, b: usize| {
        let (Some(a), Some(b)) = (steps.get(a), steps.get(b)) else {
            return false;
        };
        let (from, to) = (a.at(a.to), b.at(a.to));
        (from[0] - to[0]).hypot(from[1] - to[1])
            > TURNED_AROUND * from[0].hypot(from[1]).max(to[0].hypot(to[1]))
    };
    let last = steps.len();
    (0..frames.len())
        .map(|f| Going {
            before: f.checked_sub(1).and_then(|b| steps.get(b)).copied(),
            after: steps.get(f).copied(),
            turned: match f {
                0 => disagree(0, 1),
                f if f == last => disagree(last.saturating_sub(2), last.saturating_sub(1)),
                f => disagree(f - 1, f),
            },
        })
        .collect()
}

/// How far off what was measured the model may be to be believed, in squares, for how far off one width
/// for all of it is. A subject out of focus and moving reads as wide whichever way its edge faces, and comes
/// to 5 or more on anything measured; a shutter, whose longest edges read short, to 2.4 at most.
const MOST_LEFT: f64 = 3.0;

/// What a shot's corners and the measures across their edges say of its shutter, or `None` where they do
/// not say a blur at least `shortest` long, or say something other than a shutter.
pub(super) fn fit(
    frames: &[Vec<[f64; 2]>],
    measures: &[Vec<Across>],
    shortest: f64,
) -> Option<Shutter> {
    if frames.len() < 2 {
        return None;
    }
    let goes = going(frames);
    let mut seen = Vec::new();
    let mut lengths = Vec::new();
    for ((corners, go), measured) in frames.iter().zip(&goes).zip(measures) {
        if go.turned {
            continue;
        }
        for ((at, out), edge) in corners.iter().zip(soft::normals(corners)).zip(measured) {
            let m = go.at(*at);
            let along = (m[0] * out[0] + m[1] * out[1]).abs();
            let wide = (edge.wide * edge.wide - soft::HARD * soft::HARD)
                .max(0.0)
                .sqrt()
                / FEATHER_OF_RAMP;
            seen.push(Seen {
                along,
                wide,
                out: edge.out,
                weight: edge.weight,
                room: edge.room,
            });
            lengths.push(m[0].hypot(m[1]));
        }
    }
    lengths.sort_by(f64::total_cmp);
    let typical = lengths.get(lengths.len() / 2).copied().unwrap_or(0.0);
    let open = slope(seen.iter(), |one| one.wide)?.clamp(0.0, 1.0);
    // How long the blur is where an edge faces the way it goes, which on a slow subject is no more than
    // the noise that a slope through so little can make a long shutter of.
    let facing = || seen.iter().filter(|one| one.along >= FACING * typical);
    let weights: f64 = facing().map(|one| one.weight).sum();
    let longest = facing().map(|one| one.weight * one.wide).sum::<f64>() / weights.max(1e-9);
    // And no blur is longer than what went across it in a frame: a soft edge on a subject that barely
    // moves is some other softness.
    if longest < shortest || open * typical < shortest {
        return None;
    }
    // An edge as wide whichever way it faces is not a shutter's, however fast it goes.
    let weights: f64 = seen.iter().map(|one| one.weight).sum();
    let mean = seen.iter().map(|one| one.weight * one.wide).sum::<f64>() / weights.max(1e-9);
    let same_width: f64 = seen
        .iter()
        .map(|one| one.weight * (one.wide - mean).powi(2))
        .sum();
    let left: f64 = seen
        .iter()
        .map(|one| one.weight * (one.wide - open * one.along).powi(2))
        .sum();
    if left > MOST_LEFT * same_width {
        return None;
    }
    // Only where the edge was read past its blur is its middle where it reads: short of the plain side, it
    // reads as nearer the cut than it is.
    let sound = seen
        .iter()
        .filter(|one| one.room >= ROOM * open * one.along);
    let middle = slope(sound, |one| one.out)?.clamp(-open / 2.0, open / 2.0);
    if middle.abs() < LEAST_MIDDLE {
        return None;
    }
    Some(Shutter { open, middle })
}

/// What an edge says, for [`fit`].
#[derive(Clone, Copy, Debug)]
struct Seen {
    along: f64,
    wide: f64,
    out: f64,
    weight: f64,
    room: f64,
}

/// How much of its way an edge must go across itself to be said to face it.
const FACING: f64 = 0.7;

/// How far an edge must be read to either side, as a share of how wide its blur is.
const ROOM: f64 = 1.2;

/// What the cut leaves undone below which there is nothing here to add to the measure, for each pixel an
/// edge goes in a frame.
const LEAST_MIDDLE: f64 = 0.05;

/// How much of `what` goes with how far each edge goes in a frame.
fn slope<'a>(seen: impl Iterator<Item = &'a Seen>, what: impl Fn(&Seen) -> f64) -> Option<f64> {
    let (mut sum, mut norm) = (0.0, 0.0);
    for one in seen {
        sum += one.weight * one.along * what(one);
        norm += one.weight * one.along * one.along;
    }
    (norm > 0.0).then(|| sum / norm)
}

/// The way a frame's edges say it went, for each pixel `middle` says an edge goes, where that is far from
/// what the frames around it say, as it is at a bounce: the frames either side of it average to little.
fn seen_going(
    corners: &[[f64; 2]],
    measured: &[Across],
    middle: f64,
    go: &Going,
) -> Option<[f64; 2]> {
    if middle == 0.0 || !go.turned {
        return None;
    }
    let normals = soft::normals(corners);
    let mut best = (f64::INFINITY, [0.0, 0.0]);
    for degrees in (0..180).step_by(5) {
        let (y, x) = f64::from(degrees).to_radians().sin_cos();
        let (mut sum, mut norm) = (0.0, 0.0);
        for (out, edge) in normals.iter().zip(measured) {
            let facing = (x * out[0] + y * out[1]).abs();
            sum += edge.weight * facing * edge.out / middle;
            norm += edge.weight * facing * facing;
        }
        let length = sum / f64::max(norm, 1e-9);
        let off: f64 = normals
            .iter()
            .zip(measured)
            .map(|(out, edge)| {
                let facing = (x * out[0] + y * out[1]).abs();
                let gap = edge.out / middle - length * facing;
                edge.weight * gap * gap
            })
            .sum();
        if off < best.0 {
            best = (off, [x * length, y * length]);
        }
    }
    // It went no further than it did in the frames either side.
    let at = corners.first().copied().unwrap_or_default();
    let longest = [go.before, go.after]
        .iter()
        .flatten()
        .map(|step| {
            let m = step.at(at);
            m[0].hypot(m[1])
        })
        .fold(0.0, f64::max);
    let [x, y] = best.1;
    let by = (longest / x.hypot(y).max(1e-9)).min(1.0);
    Some([x * by, y * by])
}

/// How far apart, as a share of the longer, the ways a frame came and goes on must be for it to have turned
/// around, and the frames around it to say nothing of it.
const TURNED_AROUND: f64 = 0.5;

/// The picture of an outline, as a matte `w` by `h`.
fn filled(corners: &[[f64; 2]], w: usize, h: usize) -> Vec<u8> {
    let mut matte = vec![0u8; w * h];
    let n = corners.len();
    let mut crossings = Vec::new();
    for y in 0..h {
        let line = y as f64 + 0.5;
        crossings.clear();
        for i in 0..n {
            let (a, b) = (corners[i], corners[(i + 1) % n]);
            if (a[1] <= line) != (b[1] <= line) {
                crossings.push(a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            let from = (pair[0] - 0.5).ceil().max(0.0) as usize;
            let to = ((pair[1] - 0.5).floor() as i64 + 1).clamp(0, w as i64) as usize;
            if from < to {
                matte[y * w + from..y * w + to].fill(1);
            }
        }
    }
    matte
}

impl Shutter {
    /// How a frame goes, given what its corners measured.
    pub(super) fn frame(&self, corners: &[[f64; 2]], measured: &[Across], go: Going) -> Frame {
        Frame {
            go,
            seen: seen_going(corners, measured, self.middle, &go),
        }
    }

    /// What is more than half covered, given the outline of the cut and how the frame goes.
    pub(super) fn half_covered(
        &self,
        corners: &[[f64; 2]],
        frame: &Frame,
        w: usize,
        h: usize,
    ) -> Vec<u8> {
        let matte = filled(corners, w, h);
        let on = |x: f64, y: f64| {
            x >= 0.0
                && y >= 0.0
                && (x as usize) < w
                && (y as usize) < h
                && matte[y as usize * w + x as usize] != 0
        };
        let mut out = vec![0u8; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let m = frame.at(p);
                let mut looks = (-4..=4).map(|k| {
                    let t = self.middle * f64::from(k) / 4.0;
                    on(p[0] + m[0] * t, p[1] + m[1] * t)
                });
                let inside = if self.middle < 0.0 {
                    looks.all(|one| one)
                } else {
                    looks.any(|one| one)
                };
                *value = u8::from(inside);
            }
        });
        out
    }

    /// How wide the edge is at each of `corners`.
    pub(super) fn widths(&self, corners: &[[f64; 2]], frame: &Frame) -> Vec<f64> {
        corners
            .iter()
            .zip(soft::normals(corners))
            .map(|(at, out)| {
                let m = frame.at(*at);
                (m[0] * out[0] + m[1] * out[1]).abs() * self.open * FEATHER_OF_RAMP
            })
            .collect()
    }
}

/// How a frame goes: as the frames around it say, or by its own edges where those disagree.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    go: Going,
    seen: Option<[f64; 2]>,
}

impl Frame {
    fn at(&self, p: [f64; 2]) -> [f64; 2] {
        self.seen.unwrap_or_else(|| self.go.at(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ellipse's corners, going around clockwise as seen.
    fn ellipse(center: [f64; 2]) -> Vec<[f64; 2]> {
        (0..40)
            .map(|at| {
                let turn = std::f64::consts::TAU * f64::from(at) / 40.0;
                [center[0] + 80.0 * turn.cos(), center[1] + 40.0 * turn.sin()]
            })
            .collect()
    }

    /// Frames of an ellipse going `speed` a frame, and what a shutter open for `open` of a frame would have
    /// measured of them, if what is half covered lies `middle` out from the cut.
    fn shot(speed: [f64; 2], open: f64, middle: f64) -> (Vec<Vec<[f64; 2]>>, Vec<Vec<Across>>) {
        let frames: Vec<Vec<[f64; 2]>> = (0..12)
            .map(|at| {
                ellipse([
                    300.0 + speed[0] * f64::from(at),
                    200.0 + speed[1] * f64::from(at),
                ])
            })
            .collect();
        let measures = frames
            .iter()
            .map(|corners| {
                soft::normals(corners)
                    .iter()
                    .map(|out| {
                        let along = (speed[0] * out[0] + speed[1] * out[1]).abs();
                        let ramp = open * along * FEATHER_OF_RAMP;
                        Across {
                            wide: (ramp * ramp + soft::HARD * soft::HARD).sqrt(),
                            out: middle * along,
                            weight: 1.0,
                            room: 200.0,
                        }
                    })
                    .collect()
            })
            .collect();
        (frames, measures)
    }

    #[test]
    fn a_shutter_is_found_from_the_edges_it_widens() {
        for (speed, open, middle) in [
            ([64.0, 0.0], 0.5, -0.22),
            ([48.0, 40.0], 1.0, -0.4),
            ([-30.0, 20.0], 0.75, -0.1),
            ([64.0, 0.0], 0.5, 0.2),
        ] {
            let (frames, measures) = shot(speed, open, middle);
            let found = fit(&frames, &measures, 16.0).expect("a shutter");
            assert!((found.open - open).abs() < 0.01, "{found:?}");
            assert!((found.middle - middle).abs() < 0.01, "{found:?}");
        }
    }

    #[test]
    fn what_is_not_a_long_shutter_is_left_to_the_measure() {
        // A held lens on a still subject, and one going too slowly to matter.
        let (frames, measures) = shot([0.0, 0.0], 0.5, -0.22);
        assert_eq!(fit(&frames, &measures, 16.0), None);
        let (frames, measures) = shot([8.0, 0.0], 0.5, -0.22);
        assert_eq!(fit(&frames, &measures, 16.0), None);
        // Fast and sharp.
        let (frames, measures) = shot([64.0, 0.0], 0.0, 0.0);
        assert_eq!(fit(&frames, &measures, 16.0), None);
        // Fast, and as wide whichever way its edge faces: out of focus.
        let (frames, mut measures) = shot([64.0, 0.0], 0.5, -0.22);
        for edge in measures.iter_mut().flatten() {
            edge.wide = 32.0;
        }
        assert_eq!(fit(&frames, &measures, 16.0), None);
        // Barely moving, and soft on one side: a mask's edge, or a lens's.
        let (frames, mut measures) = shot([1.0, 0.0], 1.0, -0.4);
        for (at, edge) in measures.iter_mut().flatten().enumerate() {
            edge.wide = if at % 40 < 20 { 40.0 } else { 3.0 };
        }
        assert_eq!(fit(&frames, &measures, 16.0), None);
        // A blur, with nothing to take from the cut.
        let (frames, measures) = shot([64.0, 0.0], 0.5, 0.0);
        assert_eq!(fit(&frames, &measures, 16.0), None);
    }

    #[test]
    fn edges_too_narrow_to_read_past_their_blur_are_not_gone_by() {
        // Where an edge is read short of its plain side, its middle reads as nearer the cut than it is.
        let (frames, mut measures) = shot([64.0, 0.0], 0.5, -0.22);
        for edge in measures.iter_mut().flatten() {
            if edge.out.abs() > 8.0 {
                edge.room = 200.0;
            } else {
                edge.room = 4.0;
                edge.out = 0.0;
            }
        }
        let found = fit(&frames, &measures, 16.0).expect("a shutter");
        assert!((found.middle + 0.22).abs() < 0.01, "{found:?}");
    }

    #[test]
    fn a_step_says_how_a_turn_and_a_change_of_size_move_a_point() {
        // Turned a quarter around its middle, twice the size, and 10 to the right.
        let from = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        let to = [[10.0, 2.0], [8.0, 0.0], [10.0, -2.0], [12.0, 0.0]];
        let step = Step::between(&from, &to);
        let [x, y] = step.at([1.0, 0.0]);
        assert!((x - 9.0).abs() < 1e-9 && (y - 2.0).abs() < 1e-9, "{x} {y}");
    }

    #[test]
    fn a_frame_that_turned_around_is_told_from_one_going_steadily() {
        let at = |x: f64| ellipse([x, 200.0]);
        let steady: Vec<_> = (0..5).map(|f| at(100.0 + 40.0 * f64::from(f))).collect();
        assert!(going(&steady).iter().all(|go| !go.turned));
        let back: Vec<_> = [100.0, 140.0, 180.0, 140.0, 100.0].map(at).to_vec();
        let turned: Vec<bool> = going(&back).iter().map(|go| go.turned).collect();
        assert_eq!(turned, [false, false, true, false, false]);
        // At the very end of a shot, the two steps nearest it say so.
        let late: Vec<_> = [100.0, 140.0, 180.0, 140.0].map(at).to_vec();
        assert!(going(&late).last().is_some_and(|go| go.turned));
        // It goes by what its edges say then, no further than it went.
        let shutter = Shutter {
            open: 0.5,
            middle: -0.25,
        };
        let corners = &back[2];
        let measured: Vec<Across> = soft::normals(corners)
            .iter()
            .map(|out| Across {
                out: -0.25 * (20.0 * out[0]).abs(),
                weight: 1.0,
                ..Across::default()
            })
            .collect();
        let frame = shutter.frame(corners, &measured, going(&back)[2]);
        let seen = frame.at(corners[0]);
        assert!(
            (seen[0].abs() - 20.0).abs() < 2.0 && seen[1].abs() < 2.0,
            "{seen:?}"
        );
    }

    /// How wide and tall what is 1 in a matte is.
    fn extent(matte: &[u8], w: usize) -> [usize; 2] {
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        for (i, value) in matte.iter().enumerate() {
            if *value != 0 {
                let (x, y) = (i % w, i / w);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        [x1 + 1 - x0, y1 + 1 - y0]
    }

    #[test]
    fn a_cut_is_drawn_in_or_out_along_the_way_it_went_and_not_across() {
        let (w, h) = (200, 100);
        let square = |x: f64| vec![[x, 20.0], [x + 80.0, 20.0], [x + 80.0, 80.0], [x, 80.0]];
        let frames = vec![square(20.0), square(60.0), square(100.0)];
        let go = going(&frames)[1];
        let measured = vec![Across::default(); 4];
        // Going 40 a frame, and a quarter of that either way.
        for (middle, wide) in [(-0.25, 60), (0.25, 100)] {
            let shutter = Shutter { open: 0.5, middle };
            let frame = shutter.frame(&frames[1], &measured, go);
            let half = shutter.half_covered(&frames[1], &frame, w, h);
            assert_eq!(extent(&half, w), [wide, 60], "{middle}");
        }
    }

    #[test]
    fn a_soft_edge_comes_nearest_a_blur_at_this_width() {
        // A mask's S against a shutter's straight ramp 32 long.
        let off = |feather: f64| -> f64 {
            (-400..=400)
                .map(|at| {
                    let x = f64::from(at) / 10.0;
                    let ramp = (x / 32.0 + 0.5).clamp(0.0, 1.0);
                    (ramp - soft::fall_off(x, 32.0 * feather)).powi(2)
                })
                .sum()
        };
        assert!(off(FEATHER_OF_RAMP) < off(FEATHER_OF_RAMP - 0.05));
        assert!(off(FEATHER_OF_RAMP) < off(FEATHER_OF_RAMP + 0.05));
    }

    #[test]
    fn an_edge_is_as_wide_as_it_went_while_the_shutter_was_open() {
        let (frames, measures) = shot([64.0, 0.0], 0.5, -0.22);
        let shutter = fit(&frames, &measures, 16.0).expect("a shutter");
        let goes = going(&frames);
        let frame = shutter.frame(&frames[5], &measures[5], goes[5]);
        let wide = shutter.widths(&frames[5], &frame);
        // At its right and left it goes straight across: 32 long, drawn by a mask's S.
        assert!((wide[0] - 32.0 * FEATHER_OF_RAMP).abs() < 2.0, "{wide:?}");
        assert!((wide[20] - 32.0 * FEATHER_OF_RAMP).abs() < 2.0, "{wide:?}");
        // At its top and bottom, along itself.
        assert!(wide[10] < 1.0 && wide[30] < 1.0, "{wide:?}");
    }
}
