//! How well a take followed its guide, clip by clip.
//!
//! A take's depth, read back from it by a model, is a picture of the same frames as its
//! guide's depth pass ([`kerf_compose::Pass::Depth`]). Held together over a whole frame they
//! say little of the subject: most of a frame is ground that is where it was whatever the
//! subject did. So this reads them by the clip. The guide's ids pass
//! ([`kerf_compose::Pass::Ids`]) says which clip each pixel is of, and for those pixels this
//! says how far the take is from the guide.
//!
//! The two are not on one scale. A depth pass goes by the scene's own numbers, and a model
//! stretches each frame to its own nearest and farthest. What both keep is order: which
//! pixel is nearer than which. So each pixel is given its place in its frame's order of near
//! and far, from 0 to 1, in the guide and in the take, and a clip is `off` by how far apart
//! those two places are, over its pixels. A subject the take put elsewhere has, where the
//! guide has it, whatever was behind it.
//!
//! It is read on a small frame, over the pixels the guide has a depth for.

use std::path::Path;

use crate::error::{EngineError, Result};
use kerf_compose::{ComposeOptions, Compositor, Pass};
use kerf_model::{Catalog, Project, Time};

/// The height of the frames read, in pixels, where the sequence is as tall.
const HEIGHT: f64 = 216.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// How a take's depth holds beside its guide.
#[derive(Clone, Debug, PartialEq)]
pub struct Followed {
    /// How many frames were read.
    pub frames: usize,
    /// Over the whole frame, how alike the order of near and far is in the two: the rank
    /// correlation, 1 where it is the same order, as a mean over the frames.
    pub order: f64,
    /// The same as each clip's `off`, over every pixel the guide has a depth for.
    pub off: f64,
    /// The clips the guide has in depth, from the lowest track up.
    pub clips: Vec<ClipFollowed>,
}

/// How a take holds beside its guide over the pixels of one clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipFollowed {
    pub clip: String,
    /// The frames it shows on.
    pub frames: usize,
    /// The share of the frame it has, as a mean over those frames.
    pub shows: f64,
    /// How far its pixels are in the take from where the guide has them in the frame's
    /// order of near and far, as a share of that order: a mean over its pixels and frames.
    pub off: f64,
    /// The same with its direction kept: above 0 where the take has them farther.
    pub farther: f64,
    /// The frame it is furthest off on, counted from 0, and how far.
    pub worst: (usize, f64),
}

#[derive(Clone, Default)]
struct Sum {
    frames: usize,
    pixels: f64,
    off: f64,
    farther: f64,
    worst: (usize, f64),
}

/// A guide being held to a take, frame by frame.
pub struct Following {
    depth: Compositor,
    ids: Compositor,
    colors: Vec<(String, [u8; 3])>,
    scale: f64,
    size: (u32, u32),
    frames: usize,
    order: f64,
    whole: f64,
    clips: Vec<Sum>,
}

impl Following {
    /// The guide `sequence` (the first when `None`) of `project` draws. Media is read from
    /// where the project's paths say, so resolve them first.
    pub fn new(project: &Project, sequence: Option<&str>) -> Result<Self> {
        let target = crate::program::program_sequence(project, sequence)?;
        let drawing = |pass: Pass| -> Result<Compositor> {
            Ok(Compositor::new(
                project,
                Catalog::builtin(),
                &ComposeOptions {
                    sequence: Some(target.id.to_string()),
                    pass,
                    ..ComposeOptions::default()
                },
            )?)
        };
        let (depth, ids) = (drawing(Pass::Depth)?, drawing(Pass::Ids)?);
        let scale = (HEIGHT / f64::from(target.height.max(1))).min(1.0);
        let first = ids.frame(Time::ZERO, scale);
        let colors = ids.ids();
        Ok(Self {
            clips: vec![Sum::default(); colors.len()],
            depth,
            ids,
            colors,
            scale,
            size: (first.width, first.height),
            frames: 0,
            order: 0.0,
            whole: 0.0,
        })
    }

    /// How wide and tall the frames it reads are: a take's are given at this size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The take's depth at sequence time `at`, a byte a pixel, nearer whiter, row by row at
    /// [`Self::size`].
    pub fn frame(&mut self, at: Time, take: &[u8]) {
        let guide = self.depth.frame(at, self.scale);
        let ids = self.ids.frame(at, self.scale);
        let pixels = (guide.width * guide.height) as usize;
        if take.len() != pixels || ids.rgba.len() != guide.rgba.len() {
            return;
        }
        // The pixels the guide has a depth for, and each one's place in the order of each.
        let known: Vec<usize> = (0..pixels)
            .filter(|pixel| guide.rgba[pixel * 4] > 0)
            .collect();
        let number = self.frames;
        self.frames += 1;
        if known.len() < 2 {
            return;
        }
        let of_guide = places(known.iter().map(|pixel| guide.rgba[pixel * 4]));
        let of_take = places(known.iter().map(|pixel| take[*pixel]));
        self.order += correlation(&of_guide, &of_take);
        let mut sums = vec![(0usize, 0.0f64, 0.0f64); self.colors.len()];
        let mut whole = 0.0;
        for (index, pixel) in known.iter().enumerate() {
            // A place nearer 1 is nearer the camera.
            let farther = of_guide[index] - of_take[index];
            whole += farther.abs();
            let color = &ids.rgba[pixel * 4..pixel * 4 + 3];
            let Some(clip) = self.colors.iter().position(|(_, own)| {
                own.iter().zip(color).all(|(a, b)| a.abs_diff(*b) <= NEAR)
            }) else {
                continue;
            };
            sums[clip].0 += 1;
            sums[clip].1 += farther.abs();
            sums[clip].2 += farther;
        }
        self.whole += whole / known.len() as f64;
        for (sum, (count, off, farther)) in self.clips.iter_mut().zip(sums) {
            if count == 0 {
                continue;
            }
            let off = off / count as f64;
            sum.frames += 1;
            sum.pixels += count as f64 / pixels as f64;
            sum.off += off;
            sum.farther += farther / count as f64;
            if off > sum.worst.1 {
                sum.worst = (number, off);
            }
        }
    }

    /// What the frames given came to.
    pub fn done(self) -> Followed {
        let mean = |sum: f64, frames: usize| rounded(sum / frames.max(1) as f64);
        Followed {
            frames: self.frames,
            order: mean(self.order, self.frames),
            off: mean(self.whole, self.frames),
            clips: self
                .colors
                .iter()
                .zip(&self.clips)
                .filter(|(_, sum)| sum.frames > 0)
                .map(|((clip, _), sum)| ClipFollowed {
                    clip: clip.clone(),
                    frames: sum.frames,
                    shows: mean(sum.pixels, sum.frames),
                    off: mean(sum.off, sum.frames),
                    farther: mean(sum.farther, sum.frames),
                    worst: (sum.worst.0, rounded(sum.worst.1)),
                })
                .collect(),
        }
    }
}

/// How the depth read back from a take, the video at `take`, holds beside the guide
/// `sequence` (the first when `None`) of `project` draws: the take's frames being `rate` a
/// second from the sequence's start, as a guide drawn at that rate has them.
pub fn follows(
    project: &Project,
    sequence: Option<&str>,
    take: &Path,
    rate: u32,
) -> Result<Followed> {
    let mut following = Following::new(project, sequence)?;
    let ends = crate::program::program_sequence(project, sequence)?.duration();
    let size = following.size();
    crate::index::small_grey_frames(take, rate.max(1), size, |seconds, grey| {
        if let Some(at) = Time::from_seconds(seconds).filter(|at| *at < ends) {
            following.frame(at, grey);
        }
    })?;
    if following.frames == 0 {
        return Err(EngineError::msg(format!(
            "`{}` has no frame within the sequence",
            take.display()
        )));
    }
    Ok(following.done())
}

fn rounded(share: f64) -> f64 {
    (share * 10000.0).round() / 10000.0
}

/// Each of `values` as its place in their order, from 0 for the least to 1 for the most:
/// those that are the same share the middle of the places they take up.
fn places(values: impl Iterator<Item = u8>) -> Vec<f64> {
    let values: Vec<u8> = values.collect();
    let mut counts = [0usize; 256];
    for value in &values {
        counts[usize::from(*value)] += 1;
    }
    let mut place = [0.0f64; 256];
    let mut under = 0usize;
    for (value, count) in counts.iter().enumerate() {
        place[value] = (under as f64 + *count as f64 / 2.0) / values.len() as f64;
        under += count;
    }
    values.iter().map(|value| place[usize::from(*value)]).collect()
}

/// How alike two orders are: 1 the same, -1 turned about, 0 where one says nothing of the
/// other, or is all one value.
fn correlation(one: &[f64], other: &[f64]) -> f64 {
    let count = one.len() as f64;
    let mean = |of: &[f64]| of.iter().sum::<f64>() / count;
    let (mean_one, mean_other) = (mean(one), mean(other));
    let (mut both, mut of_one, mut of_other) = (0.0, 0.0, 0.0);
    for (a, b) in one.iter().zip(other) {
        both += (a - mean_one) * (b - mean_other);
        of_one += (a - mean_one).powi(2);
        of_other += (b - mean_other).powi(2);
    }
    let spread = (of_one * of_other).sqrt();
    if spread > 0.0 {
        both / spread
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_is_a_share_of_the_order_and_equals_share_theirs() {
        assert_eq!(
            places([10u8, 30, 20, 30].into_iter()),
            [0.125, 0.75, 0.375, 0.75]
        );
        let up = [0.0, 0.25, 0.5, 1.0];
        assert!((correlation(&up, &up) - 1.0).abs() < 1e-12);
        assert!((correlation(&up, &[1.0, 0.75, 0.5, 0.0]) + 1.0).abs() < 1e-12);
        assert_eq!(correlation(&up, &[0.5; 4]), 0.0);
    }
}
