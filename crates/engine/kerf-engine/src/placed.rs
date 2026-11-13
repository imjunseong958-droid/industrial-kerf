//! Where a take has a clip's subject, beside where its guide has it.
//!
//! [`crate::follows`] holds a take's depth to its guide's and says how far a clip's pixels
//! are from their place in the order of near and far. It does not say where the subject is:
//! a crate drawn a third of the frame to one side reads there as the ground behind it. This
//! reads the take's own picture, with no model. The guide's ids pass
//! ([`kerf_compose::Pass::Ids`]) gives the box around the clip on each frame, as `look` with
//! `shows` does. The subject is found on the frame the guide shows most of it on, inside
//! that box with room around it, and followed from there to both ends
//! ([`kerf_track::follow_object`]), a tracker holding best what it was shown large. That is
//! done twice, in the take and in the guide's own picture, and it is those two boxes that
//! are held together frame by frame: a box followed in a picture is not the box of an ids
//! pass, wider by its soft edge and by what is hidden of it, and what one way of looking
//! adds to both is in neither's distance from the other. On a filmed set the guide's own
//! picture held to its ids box was 0.03 of a frame height off; held to itself it is 0.
//!
//! Distances are in frame heights, across as down, so that one number is one distance.

use std::path::Path;

use crate::error::{EngineError, Result};
use kerf_compose::{ComposeOptions, Compositor, Pass};
use kerf_model::{Catalog, Project, Time};

/// The height of the frames read, in pixels, where the sequence is as tall: more than
/// [`crate::follows`] reads, a subject being a small part of a frame.
const HEIGHT: f64 = 360.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// The most frames read: a shot a model makes, and some more.
pub const MOST_FRAMES: usize = 240;

/// How much of its own size the guide's box is widened by on each side, to look for the
/// subject in: a take need not have it to the pixel, and a cut wants something around it.
const ROOM: f64 = 0.5;

/// Where a take has a clip's subject beside where its guide has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub clip: String,
    /// The frames the guide shows the clip on.
    pub shown: usize,
    /// Those of them the subject was followed on in the take and in the guide's own picture,
    /// which the rest is a mean over.
    pub frames: usize,
    /// The frame it was found on and followed from, counted from 0: the one the guide
    /// shows most of it on.
    pub from: usize,
    /// The first and the last frame it was followed on.
    pub read: (usize, usize),
    /// The share of the frame the guide gives it, as a mean.
    pub shows: f64,
    /// How far the middle of its box in the take is from the middle of its box in the
    /// guide, in frame heights.
    pub off: f64,
    /// The same across the frame, with its direction: above 0 where the take has it to the
    /// right.
    pub across: f64,
    /// The same down the frame: above 0 where the take has it lower.
    pub down: f64,
    /// How large its box is in the take beside the guide's, by its sides summed over the
    /// frames, so that the frames it is large on count for most: 1 the same, above 1 where
    /// the take drew it larger.
    pub size: f64,
    /// The frame it is furthest off on, and how far.
    pub worst: (usize, f64),
}

/// A box in pixels, left, top, right, bottom, the last two past its last pixel.
type Box = [f64; 4];

/// A clip's subject being looked for in a take, frame by frame.
pub struct Placing {
    ids: Compositor,
    picture: Compositor,
    clip: String,
    color: [u8; 3],
    scale: f64,
    size: (u32, u32),
    /// The guide's box on each frame given, and how many pixels of the clip show.
    guide: Vec<Option<(Box, usize)>>,
    /// The guide's own picture on each, and the take's.
    drawn: Vec<Vec<u8>>,
    take: Vec<Vec<u8>>,
}

impl Placing {
    /// The clip `clip` of the guide `sequence` (the first when `None`) of `project`. Media
    /// is read from where the project's paths say, so resolve them first.
    pub fn new(project: &Project, sequence: Option<&str>, clip: &str) -> Result<Self> {
        let target = crate::program::program_sequence(project, sequence)?;
        let drawing = |pass: Pass| {
            Compositor::new(
                project,
                Catalog::builtin(),
                &ComposeOptions {
                    sequence: Some(target.id.to_string()),
                    pass,
                    ..ComposeOptions::default()
                },
            )
        };
        let (ids, picture) = (drawing(Pass::Ids)?, drawing(Pass::Picture)?);
        let Some((_, color)) = ids.ids().into_iter().find(|(id, _)| id == clip) else {
            return Err(EngineError::msg(format!(
                "sequence `{}` draws no clip `{clip}`: `clip` is one its guide shows",
                target.id
            )));
        };
        let scale = (HEIGHT / f64::from(target.height.max(1))).min(1.0);
        let first = ids.frame(Time::ZERO, scale);
        Ok(Self {
            ids,
            picture,
            clip: clip.to_owned(),
            color,
            scale,
            size: (first.width, first.height),
            guide: Vec::new(),
            drawn: Vec::new(),
            take: Vec::new(),
        })
    }

    /// How wide and tall the frames it reads are: a take's are given at this size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The take's picture at sequence time `at`, four bytes a pixel, row by row at
    /// [`Self::size`]. Frames are given in order; those past [`MOST_FRAMES`] are left.
    pub fn frame(&mut self, at: Time, take: &[u8]) {
        let ids = self.ids.frame(at, self.scale);
        if take.len() != ids.rgba.len() || self.take.len() >= MOST_FRAMES {
            return;
        }
        let (width, height) = (ids.width as usize, ids.height as usize);
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        let mut pixels = 0;
        for (index, pixel) in ids.rgba.as_chunks::<4>().0.iter().enumerate() {
            if self
                .color
                .iter()
                .zip(pixel)
                .all(|(own, drawn)| own.abs_diff(*drawn) <= NEAR)
            {
                let (x, y) = (index % width, index / width);
                (left, top) = (left.min(x), top.min(y));
                (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
                pixels += 1;
            }
        }
        self.guide.push((pixels > 0).then_some((
            [left as f64, top as f64, right as f64, bottom as f64],
            pixels,
        )));
        self.drawn.push(self.picture.frame(at, self.scale).rgba);
        self.take.push(take.to_vec());
    }

    /// What the frames given came to: none where the guide shows the clip on none of them,
    /// or the subject cannot be followed on one frame of both pictures.
    pub fn done(self) -> Option<Placed> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let (from, (seed, _)) = self
            .guide
            .iter()
            .enumerate()
            .filter_map(|(frame, shown)| Some((frame, (*shown)?)))
            .max_by_key(|(_, (_, pixels))| *pixels)?;
        let (wide, tall) = (width as f64, height as f64);
        let (across, down) = ((seed[2] - seed[0]) * ROOM, (seed[3] - seed[1]) * ROOM);
        let hints = kerf_track::Hints {
            inside: vec![[
                (seed[0] + seed[2]) / 2.0 / wide,
                (seed[1] + seed[3]) / 2.0 / tall,
            ]],
            outside: Vec::new(),
            bbox: Some([
                ((seed[0] - across) / wide).max(0.0),
                ((seed[1] - down) / tall).max(0.0),
                ((seed[2] + across) / wide).min(1.0),
                ((seed[3] + down) / tall).min(1.0),
            ]),
        };
        // From the frame it is largest on to the last, and from it back to the first, in one
        // picture and then the other.
        let follow = |pictures: &[Vec<u8>]| {
            let mut found: Vec<Option<Box>> = vec![None; pictures.len()];
            let ways: [Vec<usize>; 2] = [
                (from..pictures.len()).collect(),
                (0..=from).rev().collect(),
            ];
            for frames in ways {
                let followed = kerf_track::follow_object(frames.len(), &hints, None, |index| {
                    Some((width, height, pictures[*frames.get(index)?].clone()))
                });
                for (frame, outline) in frames.into_iter().zip(followed.outlines) {
                    let mut around = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                    for [x, y] in &outline {
                        around = [
                            around[0].min(x * wide),
                            around[1].min(y * tall),
                            around[2].max(x * wide),
                            around[3].max(y * tall),
                        ];
                    }
                    if around[2] > around[0] && around[3] > around[1] {
                        found[frame] = Some(around);
                    }
                }
            }
            found
        };
        let (asked, found) = (follow(&self.drawn), follow(&self.take));
        let shown = self.guide.iter().flatten().count();
        let (mut frames, mut read) = (0usize, (usize::MAX, 0usize));
        let (mut off, mut right, mut lower, mut shows) = (0.0, 0.0, 0.0, 0.0);
        let mut size = (0.0, 0.0);
        let mut worst = (from, 0.0);
        for (frame, shown) in self.guide.iter().enumerate() {
            let (Some((_, pixels)), Some(guide), Some(taken)) =
                (shown, &asked[frame], &found[frame])
            else {
                continue;
            };
            let middle = |of: &Box| [(of[0] + of[2]) / 2.0, (of[1] + of[3]) / 2.0];
            let (asked, drawn) = (middle(guide), middle(taken));
            let (x, y) = ((drawn[0] - asked[0]) / tall, (drawn[1] - asked[1]) / tall);
            let far = x.hypot(y);
            let sides = |of: &Box| (of[2] - of[0]) + (of[3] - of[1]);
            frames += 1;
            read = (read.0.min(frame), read.1.max(frame));
            off += far;
            right += x;
            lower += y;
            size.0 += sides(taken);
            size.1 += sides(guide);
            shows += *pixels as f64 / (wide * tall);
            if far > worst.1 {
                worst = (frame, far);
            }
        }
        if frames == 0 {
            return None;
        }
        let mean = |sum: f64| rounded(sum / frames as f64);
        Some(Placed {
            clip: self.clip,
            shown,
            frames,
            from,
            read,
            shows: mean(shows),
            off: mean(off),
            across: mean(right),
            down: mean(lower),
            size: rounded(size.0 / size.1),
            worst: (worst.0, rounded(worst.1)),
        })
    }
}

/// Where the take at `take`, a video, has the subject of `clip` beside where the guide
/// `sequence` (the first when `None`) of `project` has it: the take's frames being `rate` a
/// second from the sequence's start, as a guide drawn at that rate has them.
pub fn placed(
    project: &Project,
    sequence: Option<&str>,
    clip: &str,
    take: &Path,
    rate: u32,
) -> Result<Placed> {
    let mut placing = Placing::new(project, sequence, clip)?;
    let ends = crate::program::program_sequence(project, sequence)?.duration();
    let size = placing.size();
    crate::index::small_color_frames(take, rate.max(1), size, |seconds, rgba| {
        if let Some(at) = Time::from_seconds(seconds).filter(|at| *at < ends) {
            placing.frame(at, rgba);
        }
    })?;
    let shown = placing.guide.iter().flatten().count();
    placing.done().ok_or_else(|| {
        EngineError::msg(if shown == 0 {
            format!(
                "the guide shows `{clip}` on no frame of `{}`",
                take.display()
            )
        } else {
            format!(
                "`{}` has nothing to follow where the guide has `{clip}`",
                take.display()
            )
        })
    })
}

fn rounded(share: f64) -> f64 {
    (share * 10000.0).round() / 10000.0
}
