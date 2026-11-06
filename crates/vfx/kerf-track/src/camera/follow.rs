//! Long paths of points through a shot, for a solve to read: corners of detail followed with
//! Lucas-Kanade and a forward-backward check, a lost one ending its path, and new ones found where the
//! frame has none, as EffectCraft's `track/src/camtrack/features.rs` does. A frame most of them do not
//! get to is a cut, and where the shot ends.

use crate::planar::corners_inside;
use crate::pyramid;
use kerf_raster::klt::{track, GrayPyramid, LkOpts};

/// One point's way through consecutive frames from `start`, in pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub start: usize,
    pub at: Vec<[f64; 2]>,
}

/// What was followed through a shot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followed {
    pub width: f64,
    pub height: f64,
    /// How many frames were read before it ended.
    pub frames: usize,
    pub paths: Vec<Path>,
    /// The frame it could go no further at: a cut, or one that could not be had.
    pub ended_at: Option<usize>,
}

/// A point being followed: its path, where it is, and how far it just went.
type Live = (usize, [f64; 2], [f64; 2]);

/// How many points to keep following.
const WANTED: usize = 500;
/// Fewer than this share of what is wanted, and new ones are looked for.
const LOOK_AGAIN_BELOW: f64 = 0.85;
/// From this many points, a frame that keeps fewer than this share of them is another shot.
const CUT_FROM: usize = 40;
const CUT_BELOW: f64 = 0.25;
/// Shorter than this a path says nothing a solve can use.
const SHORTEST: usize = 3;
/// How far from the frame's edge a new point starts, in pixels.
const MARGIN: f64 = 8.0;

/// Follow what can be followed through the `count` frames `frame` gives as `(width, height, rgba)`.
pub fn follow(
    count: usize,
    mut frame: impl FnMut(usize) -> Option<(usize, usize, Vec<u8>)>,
) -> Followed {
    let mut followed = Followed::default();
    let Some((width, height, rgba)) = frame(0) else {
        followed.ended_at = Some(0);
        return followed;
    };
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        followed.ended_at = Some(0);
        return followed;
    }
    followed.width = width as f64;
    followed.height = height as f64;
    followed.frames = 1;
    let options = LkOpts {
        radius: 6,
        iterations: 30,
        epsilon: 0.005,
        fb_max: 0.5,
        ..LkOpts::default()
    };
    let mut live: Vec<Live> = Vec::new();
    let mut previous = pyramid(width, height, &rgba);
    look_again(&previous, &mut live, &mut followed, 0);
    for index in 1..count {
        let next = match frame(index) {
            Some((w, h, rgba)) if (w, h) == (width, height) => pyramid(width, height, &rgba),
            _ => {
                followed.ended_at = Some(index);
                break;
            }
        };
        let at: Vec<[f64; 2]> = live.iter().map(|point| point.1).collect();
        let moved: Vec<[f64; 2]> = live.iter().map(|point| point.2).collect();
        let found = track(&previous, &next, &at, Some(&moved), &options);
        let kept = found.iter().flatten().count();
        if live.len() >= CUT_FROM && (kept as f64) < live.len() as f64 * CUT_BELOW {
            followed.ended_at = Some(index);
            break;
        }
        let mut survivors = Vec::with_capacity(kept);
        for (point, found) in live.iter().zip(found) {
            if let Some(now) = found {
                followed.paths[point.0].at.push(now);
                let went = [now[0] - point.1[0], now[1] - point.1[1]];
                survivors.push((point.0, now, went));
            }
        }
        live = survivors;
        followed.frames = index + 1;
        look_again(&next, &mut live, &mut followed, index);
        previous = next;
    }
    followed.paths.retain(|path| path.at.len() >= SHORTEST);
    followed
}

/// A grid of where points are, so that new ones keep their distance.
struct Taken {
    cell: f64,
    columns: usize,
    rows: usize,
    taken: Vec<bool>,
}

impl Taken {
    fn new(width: f64, height: f64, cell: f64) -> Self {
        let columns = (width / cell).ceil() as usize + 1;
        let rows = (height / cell).ceil() as usize + 1;
        Self {
            cell,
            columns,
            rows,
            taken: vec![false; columns * rows],
        }
    }

    fn cell_of(&self, at: [f64; 2]) -> (usize, usize) {
        ((at[0] / self.cell) as usize, (at[1] / self.cell) as usize)
    }

    fn is_free(&self, at: [f64; 2]) -> bool {
        let (x, y) = self.cell_of(at);
        !self
            .taken
            .get(y * self.columns + x)
            .copied()
            .unwrap_or(true)
    }

    fn take(&mut self, at: [f64; 2]) {
        let (cx, cy) = self.cell_of(at);
        for y in cy.saturating_sub(1)..=(cy + 1).min(self.rows - 1) {
            for x in cx.saturating_sub(1)..=(cx + 1).min(self.columns - 1) {
                self.taken[y * self.columns + x] = true;
            }
        }
    }
}

/// New points away from those still followed, when too few are left.
fn look_again(pictures: &GrayPyramid, live: &mut Vec<Live>, followed: &mut Followed, frame: usize) {
    if (live.len() as f64) >= WANTED as f64 * LOOK_AGAIN_BELOW {
        return;
    }
    let Some(plane) = pictures.levels.first() else {
        return;
    };
    let (w, h) = (followed.width, followed.height);
    let apart = (w.max(h) / (WANTED as f64).sqrt() * 0.6).clamp(5.0, 40.0);
    let mut taken = Taken::new(w, h, apart);
    for point in live.iter() {
        taken.take(point.1);
    }
    let whole = [
        [MARGIN, MARGIN],
        [w - MARGIN, MARGIN],
        [w - MARGIN, h - MARGIN],
        [MARGIN, h - MARGIN],
    ];
    for at in corners_inside(plane, &whole, WANTED) {
        if live.len() >= WANTED {
            break;
        }
        if !taken.is_free(at) {
            continue;
        }
        taken.take(at);
        followed.paths.push(Path {
            start: frame,
            at: vec![at],
        });
        live.push((followed.paths.len() - 1, at, [0.0, 0.0]));
    }
}
