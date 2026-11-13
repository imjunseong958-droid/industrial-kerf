//! Where a take put a clip's subject, beside where the guide has it.
//!
//! [`crate::follows`] holds a take's depth to its guide and says, clip by clip, how far the
//! take's order of near and far is from the guide's. What it cannot say is where the subject
//! went: a subject drawn a third of the frame to the left is read as the thing that was
//! behind it, and not as a thing that moved. This is the other half. The guide's ids pass
//! ([`kerf_compose::Pass::Ids`]) says where the clip is on each frame, and the subject is
//! followed through the take itself from the middle of that box on the first frame it shows
//! ([`kerf_track::follow_object`]). What the two boxes' middles differ by, frame by frame,
//! is how far the take took the subject, in shares of the frame.
//!
//! It reads the picture of a take and not its depth, so it needs no model: the take is the
//! file the recipe made. What it does need is a subject a tracker can hold, and it says
//! where it lost one (`lost`).
//!
//! It is read on a small frame, as what shows is.

use std::path::Path;

use crate::error::{EngineError, Result};
use kerf_compose::{ComposeOptions, Compositor, Pass};
use kerf_model::{Catalog, Project, Time};

/// The height of the frames read, in pixels, where the sequence is as tall.
const HEIGHT: f64 = 216.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// The most frames read at once: a take of a shot, and then some.
pub const MOST_FRAMES: usize = 240;

/// How far a take took a clip's subject from where its guide has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Went {
    pub clip: String,
    /// The frames both have it on: from the first the guide shows it on to the last read, or
    /// to where the take lost it.
    pub frames: usize,
    /// The share of the frame the guide gives it, as a mean over those frames.
    pub shows: f64,
    /// How far the middle of its box in the take is from the middle of its box in the guide,
    /// the frame being one wide and one tall: a mean over the frames.
    pub off: f64,
    /// The same across the frame, with its direction: above 0 where the take has it to the
    /// right of where the guide does.
    pub across: f64,
    /// The same down the frame: above 0 where the take has it lower.
    pub down: f64,
    /// How wide its box is in the take beside the guide's: 1 where they are the same, above
    /// where the take drew it larger, as a nearer thing is.
    pub wide: f64,
    /// The frame it is furthest off on, counted from the first read, and how far.
    pub worst: (usize, f64),
    /// The frame the tracker could no longer find it in, where it did.
    pub lost: Option<usize>,
}

/// A clip's subject being followed through a take, frame by frame.
pub struct Going {
    ids: Compositor,
    clip: String,
    color: [u8; 3],
    scale: f64,
    size: (u32, u32),
    /// Where the guide has it on each frame given, in shares of the frame, with the share of
    /// the frame it has.
    guide: Vec<Option<([f64; 4], f64)>>,
    /// The take's frames, as they were given, to follow once they are all in.
    take: Vec<Vec<u8>>,
}

impl Going {
    /// The clip `clip` of the guide `sequence` (the first when `None`) of `project` draws.
    /// Media is read from where the project's paths say, so resolve them first.
    pub fn new(project: &Project, sequence: Option<&str>, clip: &str) -> Result<Self> {
        let target = crate::program::program_sequence(project, sequence)?;
        let ids = Compositor::new(
            project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some(target.id.to_string()),
                pass: Pass::Ids,
                ..ComposeOptions::default()
            },
        )?;
        let color = ids
            .ids()
            .into_iter()
            .find(|(id, _)| id == clip)
            .map(|(_, color)| color)
            .ok_or_else(|| {
                EngineError::msg(format!(
                    "`{clip}` draws nothing in sequence `{}`, so the guide does not say where it is",
                    target.id
                ))
            })?;
        let scale = (HEIGHT / f64::from(target.height.max(1))).min(1.0);
        let first = ids.frame(Time::ZERO, scale);
        Ok(Self {
            ids,
            clip: clip.to_owned(),
            color,
            scale,
            size: (first.width, first.height),
            guide: Vec::new(),
            take: Vec::new(),
        })
    }

    /// How wide and tall the frames it reads are: a take's are given at this size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The take's picture at sequence time `at`, four bytes a pixel, row by row at
    /// [`Self::size`]. Frames past [`MOST_FRAMES`] are not read.
    pub fn frame(&mut self, at: Time, take: &[u8]) {
        if self.take.len() >= MOST_FRAMES {
            return;
        }
        let ids = self.ids.frame(at, self.scale);
        if take.len() != ids.rgba.len() {
            return;
        }
        self.guide.push(box_of(
            &ids.rgba,
            (ids.width as usize, ids.height as usize),
            self.color,
        ));
        self.take.push(take.to_vec());
    }

    /// What the frames given came to: nothing where the guide has the clip on none of them.
    pub fn done(self) -> Option<Went> {
        let start = self.guide.iter().position(Option::is_some)?;
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let (first, _) = self.guide[start]?;
        // The box with room around it: a box the subject fills to its edge leaves the cut
        // nothing around it to tell it from, and the take need not have it to the pixel.
        let hints = kerf_track::Hints {
            inside: vec![middle(first)],
            outside: Vec::new(),
            bbox: Some(widened(first, (width, height))),
        };
        let take = &self.take[start..];
        let followed = kerf_track::follow_object(take.len(), &hints, None, |index| {
            take.get(index)
                .map(|frame| (width, height, frame.clone()))
        });
        let mut sums = (0usize, 0.0, 0.0, 0.0, 0.0, 0.0);
        let mut worst = (0usize, 0.0);
        for (index, outline) in followed.outlines.iter().enumerate() {
            let Some((guide, shows)) = self.guide[start + index] else {
                continue;
            };
            if outline.is_empty() {
                continue;
            }
            let found = around(outline);
            let (at, wanted) = (middle(found), middle(guide));
            let (across, down) = (at[0] - wanted[0], at[1] - wanted[1]);
            let off = across.hypot(down);
            sums.0 += 1;
            sums.1 += off;
            sums.2 += across;
            sums.3 += down;
            sums.4 += shows;
            let wide = guide[2] - guide[0];
            sums.5 += if wide > 0.0 {
                (found[2] - found[0]) / wide
            } else {
                1.0
            };
            if off > worst.1 {
                worst = (index, off);
            }
        }
        if sums.0 == 0 {
            return None;
        }
        let mean = |sum: f64| rounded(sum / sums.0 as f64);
        Some(Went {
            clip: self.clip,
            frames: sums.0,
            shows: mean(sums.4),
            off: mean(sums.1),
            across: mean(sums.2),
            down: mean(sums.3),
            wide: mean(sums.5),
            worst: (worst.0, rounded(worst.1)),
            lost: followed.lost_at,
        })
    }
}

/// How far the take at `take` took the subject of `clip` from where the guide `sequence` (the
/// first when `None`) of `project` has it: the take's frames being `rate` a second from the
/// sequence's start, as a guide drawn at that rate has them.
pub fn went(
    project: &Project,
    sequence: Option<&str>,
    clip: &str,
    take: &Path,
    rate: u32,
) -> Result<Went> {
    let mut going = Going::new(project, sequence, clip)?;
    let ends = crate::program::program_sequence(project, sequence)?.duration();
    let size = going.size();
    crate::index::small_color_frames(take, rate.max(1), size, |seconds, rgba| {
        if let Some(at) = Time::from_seconds(seconds).filter(|at| *at < ends) {
            going.frame(at, rgba);
        }
    })?;
    going.done().ok_or_else(|| {
        EngineError::msg(format!(
            "`{clip}` is on no frame of `{}` the guide draws",
            take.display()
        ))
    })
}

fn rounded(share: f64) -> f64 {
    (share * 10000.0).round() / 10000.0
}

/// How far around a box the tracker is let look, in pixels of the frame read.
const ROOM: f64 = 6.0;

/// A box with [`ROOM`] around it, kept inside the frame.
fn widened([left, top, right, bottom]: [f64; 4], (width, height): (usize, usize)) -> [f64; 4] {
    let (across, down) = (ROOM / width.max(1) as f64, ROOM / height.max(1) as f64);
    [
        (left - across).max(0.0),
        (top - down).max(0.0),
        (right + across).min(1.0),
        (bottom + down).min(1.0),
    ]
}

/// The middle of a box: left, top, right, bottom.
fn middle(box_: [f64; 4]) -> [f64; 2] {
    [(box_[0] + box_[2]) / 2.0, (box_[1] + box_[3]) / 2.0]
}

/// The box around an outline, in the shares of the frame its corners are in.
fn around(outline: &[[f64; 2]]) -> [f64; 4] {
    let mut box_ = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for corner in outline {
        box_[0] = box_[0].min(corner[0]);
        box_[1] = box_[1].min(corner[1]);
        box_[2] = box_[2].max(corner[0]);
        box_[3] = box_[3].max(corner[1]);
    }
    box_
}

/// The box around the pixels of `rgba` that are `color`, in shares of the frame, with the
/// share of the frame they are: nothing where none is.
fn box_of(rgba: &[u8], (width, height): (usize, usize), color: [u8; 3]) -> Option<([f64; 4], f64)> {
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0usize, 0usize);
    let mut pixels = 0usize;
    for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if !color
            .iter()
            .zip(pixel)
            .all(|(a, b)| a.abs_diff(*b) <= NEAR)
        {
            continue;
        }
        let (x, y) = (index % width, index / width);
        (left, top) = (left.min(x), top.min(y));
        (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
        pixels += 1;
    }
    (pixels > 0).then(|| {
        (
            [
                left as f64 / width as f64,
                top as f64 / height as f64,
                right as f64 / width as f64,
                bottom as f64 / height as f64,
            ],
            pixels as f64 / (width * height).max(1) as f64,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boxs_middle_and_the_box_around_an_outline() {
        assert_eq!(middle([0.2, 0.4, 0.6, 0.8]), [0.4, 0.6000000000000001]);
        assert_eq!(
            around(&[[0.2, 0.5], [0.4, 0.1], [0.3, 0.9]]),
            [0.2, 0.1, 0.4, 0.9]
        );
    }

    #[test]
    fn a_box_is_widened_by_room_and_no_further_than_the_frame() {
        assert_eq!(
            widened([0.5, 0.5, 0.6, 0.6], (300, 200)),
            [0.48, 0.47, 0.62, 0.63]
        );
        assert_eq!(widened([0.0, 0.0, 1.0, 1.0], (300, 200)), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_colors_box_is_of_its_pixels_alone() {
        // Two pixels of one color in the middle row of a 4x3 frame.
        let mut rgba = vec![0u8; 4 * 3 * 4];
        for pixel in [5usize, 6] {
            rgba[pixel * 4..pixel * 4 + 3].copy_from_slice(&[10, 200, 30]);
        }
        let (box_, shows) = box_of(&rgba, (4, 3), [10, 200, 30]).expect("a box");
        assert_eq!(box_, [0.25, 1.0 / 3.0, 0.75, 2.0 / 3.0]);
        assert!((shows - 2.0 / 12.0).abs() < 1e-12);
        assert_eq!(box_of(&rgba, (4, 3), [9, 9, 9]), None);
    }
}
