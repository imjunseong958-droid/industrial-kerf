//! How a camera moves over a shot, from how the whole of the picture moves between its frames.
//!
//! This is not where the camera went, which [`crate::camera`] solves, at more cost, and only for a
//! camera that moves through a still scene. It reads what any shot shows: blocks of a small grey copy
//! are matched from one frame to the next, and the one move, turn, and zoom that most of them agree on
//! is taken as the camera's, so that a subject crossing the frame is left out. It cannot tell a pan
//! from a move sideways, nor a zoom from a push in.

use kerf_raster::flow::block_flow;
use kerf_raster::Image;

/// Tiles of the picture that are matched from frame to frame, in pixels of the copy.
const TILE: u32 = 16;
/// How far a tile is looked for at the coarsest level of the matching.
const SEARCH: i32 = 8;
/// A tile flatter than this, as a variance of 0 to 1 values, says nothing of where it went.
const FLAT: f32 = 2e-4;
/// How far a tile may be from the agreed move, in pixels of the copy, and still agree.
const AGREES: f64 = 1.0;

/// Below this over a second, in shares of the frame or in degrees, nothing is said of it.
const NOTICED: f64 = 0.01;
const NOTICED_ROLL: f64 = 1.0;
/// Less than this share of what has detail agreeing, and no one move explains the picture.
const FEW_AGREE: f64 = 0.4;
/// Above this much back and forth from step to step, as a share of the frame's width, it is handheld.
const SHAKES: f64 = 0.0015;

/// A small grey frame, from 0 to 1.
pub struct Small {
    width: u32,
    height: u32,
    image: Image,
}

impl Small {
    /// `width` by `height` of bytes.
    pub fn of(width: u32, height: u32, grey: &[u8]) -> Self {
        Self {
            width,
            height,
            image: Image {
                width,
                height,
                data: grey
                    .iter()
                    .map(|value| {
                        let value = f32::from(*value) / 255.0;
                        [value, value, value, 1.0]
                    })
                    .collect(),
            },
        }
    }

    fn variance_of(&self, column: usize, row: usize) -> f32 {
        let tile = TILE as usize;
        let (w, h) = (self.width as usize, self.height as usize);
        let (mut sum, mut square, mut count) = (0.0f32, 0.0f32, 0.0f32);
        for y in row * tile..((row + 1) * tile).min(h) {
            for x in column * tile..((column + 1) * tile).min(w) {
                let value = self.image.data[y * w + x][0];
                sum += value;
                square += value * value;
                count += 1.0;
            }
        }
        let mean = sum / count.max(1.0);
        square / count.max(1.0) - mean * mean
    }
}

/// How the picture moves from one frame to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// How far its middle goes, to the right and down, as shares of the frame's width and height.
    pub across: f64,
    pub down: f64,
    /// How much bigger it gets, and how far it turns clockwise as seen, in radians.
    pub zoom: f64,
    pub turn: f64,
    /// The share of the tiles with detail that this explains.
    pub agreeing: f64,
}

/// The one move that most of how the picture moved from `from` to `to` agrees on, or none if it has too
/// little detail to tell.
pub fn step_between(from: &Small, to: &Small) -> Option<Step> {
    let flow = block_flow(&from.image, &to.image, TILE, SEARCH);
    let (cx, cy) = (f64::from(from.width) / 2.0, f64::from(from.height) / 2.0);
    let tile = f64::from(TILE);
    let mut tiles: Vec<([f64; 2], [f64; 2])> = Vec::with_capacity(flow.cols * flow.rows);
    for row in 0..flow.rows {
        for column in 0..flow.cols {
            if from.variance_of(column, row) < FLAT {
                continue;
            }
            let moved = flow.v[row * flow.cols + column];
            tiles.push((
                [
                    (column as f64 + 0.5) * tile - cx,
                    (row as f64 + 0.5) * tile - cy,
                ],
                [f64::from(moved[0]), f64::from(moved[1])],
            ));
        }
    }
    // A quarter of the picture's tiles at the least, or a few specks would speak for all of it.
    if tiles.len() * 4 < flow.cols * flow.rows {
        return None;
    }
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let mut model = [
        median(tiles.iter().map(|(_, moved)| moved[0]).collect()),
        median(tiles.iter().map(|(_, moved)| moved[1]).collect()),
        0.0,
        0.0,
    ];
    let mut agreeing = 0;
    for _ in 0..4 {
        let kept: Vec<&([f64; 2], [f64; 2])> = tiles
            .iter()
            .filter(|(at, moved)| {
                let wanted = [
                    model[0] + model[2] * at[0] - model[3] * at[1],
                    model[1] + model[2] * at[1] + model[3] * at[0],
                ];
                (moved[0] - wanted[0]).hypot(moved[1] - wanted[1]) <= AGREES
            })
            .collect();
        agreeing = kept.len();
        if kept.len() < 4 {
            break;
        }
        let count = kept.len() as f64;
        let mean = |pick: &dyn Fn(&([f64; 2], [f64; 2])) -> f64| {
            kept.iter().map(|tile| pick(tile)).sum::<f64>() / count
        };
        let (mx, my) = (mean(&|t| t.0[0]), mean(&|t| t.0[1]));
        let (mu, mv) = (mean(&|t| t.1[0]), mean(&|t| t.1[1]));
        let (mut grow, mut twist, mut spread) = (0.0, 0.0, 0.0);
        for (at, moved) in &kept {
            let (x, y) = (at[0] - mx, at[1] - my);
            let (u, v) = (moved[0] - mu, moved[1] - mv);
            grow += x * u + y * v;
            twist += x * v - y * u;
            spread += x * x + y * y;
        }
        let (zoom, turn) = if spread > 0.0 {
            (grow / spread, twist / spread)
        } else {
            (0.0, 0.0)
        };
        model = [
            mu - zoom * mx + turn * my,
            mv - zoom * my - turn * mx,
            zoom,
            turn,
        ];
    }
    Some(Step {
        across: model[0] / f64::from(from.width),
        down: model[1] / f64::from(from.height),
        zoom: model[2],
        turn: model[3],
        agreeing: agreeing as f64 / tiles.len() as f64,
    })
}

/// What a camera did over a shot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Motion {
    /// In words, the strongest first, such as `pan right` or `handheld`; `still`, or `unknown` when too
    /// little of the shot could be read.
    pub camera: Vec<&'static str>,
    /// Over a second: to the right and down as shares of the frame, in as a share of the picture, and
    /// clockwise in degrees.
    pub pan: f64,
    pub tilt: f64,
    pub zoom: f64,
    pub roll: f64,
    /// How much it goes back and forth from step to step, as a share of the frame's width.
    pub shake: f64,
    /// The share of what has detail that follows this.
    pub agreeing: f64,
}

/// What the camera did over `steps` of a shot, each `seconds` long; a step of `None` could not be read.
pub fn motion_over(steps: &[Option<Step>], seconds: f64) -> Motion {
    let read: Vec<Step> = steps.iter().flatten().copied().collect();
    // Half of it read, or it says little of the shot; and none of it is no shot.
    if read.is_empty() || read.len() * 2 < steps.len() {
        return Motion {
            camera: vec!["unknown"],
            ..Motion::default()
        };
    }
    let count = read.len() as f64;
    let mean = |pick: &dyn Fn(&Step) -> f64| read.iter().map(pick).sum::<f64>() / count;
    let agreeing = mean(&|step| step.agreeing);
    // The picture goes the other way from the camera.
    let (pan, tilt) = (
        -mean(&|step| step.across) / seconds,
        -mean(&|step| step.down) / seconds,
    );
    let zoom = mean(&|step| step.zoom) / seconds;
    let roll = -mean(&|step| step.turn).to_degrees() / seconds;
    // What is left of each step past what the steps around it do.
    let reach = 2;
    let shake = if read.len() > reach * 2 {
        let off: Vec<f64> = (reach..read.len() - reach)
            .map(|at| {
                let around = &read[at - reach..=at + reach];
                let middle = |pick: &dyn Fn(&Step) -> f64| {
                    around.iter().map(pick).sum::<f64>() / around.len() as f64
                };
                (read[at].across - middle(&|step| step.across))
                    .hypot(read[at].down - middle(&|step| step.down))
            })
            .collect();
        (off.iter().map(|off| off * off).sum::<f64>() / off.len() as f64).sqrt()
    } else {
        0.0
    };
    let mut said: Vec<(f64, &'static str)> = Vec::new();
    if agreeing < FEW_AGREE {
        said.push((1.0, "unclear"));
    } else {
        if pan.abs() >= NOTICED {
            said.push((pan.abs(), if pan > 0.0 { "pan right" } else { "pan left" }));
        }
        if tilt.abs() >= NOTICED {
            let word = if tilt > 0.0 { "tilt down" } else { "tilt up" };
            said.push((tilt.abs(), word));
        }
        if zoom.abs() >= NOTICED {
            said.push((zoom.abs(), if zoom > 0.0 { "zoom in" } else { "zoom out" }));
        }
        if roll.abs() >= NOTICED_ROLL {
            let word = if roll > 0.0 {
                "roll right"
            } else {
                "roll left"
            };
            said.push((roll.abs() / 100.0, word));
        }
        said.sort_by(|a, b| b.0.total_cmp(&a.0));
        if shake >= SHAKES {
            said.push((0.0, "handheld"));
        }
    }
    let camera = if said.is_empty() {
        vec!["still"]
    } else {
        said.into_iter().map(|(_, word)| word).collect()
    };
    Motion {
        camera,
        pan,
        tilt,
        zoom,
        roll,
        shake,
        agreeing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 320;
    const H: u32 = 180;

    /// Smooth detail that goes on for as far as it is looked at.
    fn detail(x: f64, y: f64) -> f64 {
        let wave = |x: f64, y: f64| (x * 0.31 + y * 0.17).sin() + (x * 0.11 - y * 0.29).sin();
        (0.5 + 0.12 * wave(x, y) + 0.12 * wave(x * 0.37 + 11.0, y * 0.41 - 7.0)).clamp(0.0, 1.0)
    }

    /// A frame of what `at` says is at each pixel.
    fn frame(at: impl Fn(f64, f64) -> f64) -> Small {
        let mut grey = Vec::with_capacity((W * H) as usize);
        for y in 0..H {
            for x in 0..W {
                grey.push((at(f64::from(x) + 0.5, f64::from(y) + 0.5) * 255.0) as u8);
            }
        }
        Small::of(W, H, &grey)
    }

    /// The motion of 16 steps a quarter of a second each, a camera at `place` at each step seeing the
    /// world through what `seen` says.
    fn shot(seen: impl Fn(usize, f64, f64) -> f64) -> Motion {
        let frames: Vec<Small> = (0..17).map(|at| frame(|x, y| seen(at, x, y))).collect();
        let steps: Vec<Option<Step>> = frames
            .windows(2)
            .map(|pair| step_between(&pair[0], &pair[1]))
            .collect();
        motion_over(&steps, 0.25)
    }

    #[test]
    fn a_camera_that_does_not_move_is_still() {
        let motion = shot(|_, x, y| detail(x, y));
        assert_eq!(motion.camera, ["still"], "{motion:?}");
        assert!(
            motion.pan.abs() < 0.001 && motion.agreeing > 0.9,
            "{motion:?}"
        );
    }

    #[test]
    fn a_pan_is_read_with_how_fast_it_goes() {
        // A camera going right by 8 pixels a step: 32 a second, a tenth of the frame's width.
        let motion = shot(|at, x, y| detail(x + 8.0 * at as f64, y));
        assert_eq!(motion.camera, ["pan right"], "{motion:?}");
        assert!((motion.pan - 0.1).abs() < 0.005, "{motion:?}");
        assert!(motion.tilt.abs() < 0.005, "{motion:?}");
        let left = shot(|at, x, y| detail(x - 8.0 * at as f64, y));
        assert_eq!(left.camera, ["pan left"], "{left:?}");
    }

    #[test]
    fn a_tilt_is_read_as_a_tilt() {
        // Going down by 6 pixels a step: 24 a second, of 180.
        let motion = shot(|at, x, y| detail(x, y + 6.0 * at as f64));
        assert_eq!(motion.camera, ["tilt down"], "{motion:?}");
        assert!((motion.tilt - 24.0 / 180.0).abs() < 0.01, "{motion:?}");
    }

    #[test]
    fn a_zoom_is_read_as_a_zoom() {
        // Getting 1% bigger a step, about the middle.
        let motion = shot(|at, x, y| {
            let by = 1.01f64.powi(at as i32);
            detail(160.0 + (x - 160.0) / by, 90.0 + (y - 90.0) / by)
        });
        assert_eq!(motion.camera, ["zoom in"], "{motion:?}");
        assert!((motion.zoom - 0.04).abs() < 0.006, "{motion:?}");
    }

    #[test]
    fn a_turn_is_read_as_a_roll() {
        // The picture turning a degree a step about the middle.
        let motion = shot(|at, x, y| {
            let (s, c) = (at as f64).to_radians().sin_cos();
            let (dx, dy) = (x - 160.0, y - 90.0);
            detail(160.0 + dx * c + dy * s, 90.0 - dx * s + dy * c)
        });
        assert!(motion.camera.contains(&"roll left"), "{motion:?}");
        assert!((motion.roll + 4.0).abs() < 0.8, "{motion:?}");
    }

    #[test]
    fn a_subject_crossing_the_frame_is_not_the_camera() {
        let motion = shot(|at, x, y| {
            let (cx, cy) = (40.0 + 12.0 * at as f64, 90.0);
            if (x - cx).hypot(y - cy) < 40.0 {
                detail(x - 12.0 * at as f64 + 500.0, y + 300.0)
            } else {
                detail(x, y)
            }
        });
        assert_eq!(motion.camera, ["still"], "{motion:?}");
    }

    #[test]
    fn a_hand_held_camera_is_said_to_be() {
        let jitter = |at: usize| ((at as f64 * 12.9898).sin() * 43_758.545).fract() * 3.0;
        let motion = shot(|at, x, y| detail(x + jitter(at), y + jitter(at + 40)));
        assert!(motion.camera.contains(&"handheld"), "{motion:?}");
    }

    #[test]
    fn a_picture_with_nothing_in_it_says_nothing() {
        let motion = shot(|_, _, _| 0.5);
        assert_eq!(motion.camera, ["unknown"], "{motion:?}");
        assert_eq!(motion_over(&[], 0.25).camera, ["unknown"]);
    }
}
