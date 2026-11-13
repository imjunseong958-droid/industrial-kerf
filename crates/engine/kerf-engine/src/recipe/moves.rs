//! Where a clip goes: its place on each frame, as a recipe hands it to a model.
//!
//! A guide shows a model where things are, as a picture it follows. Some models take the
//! way a thing goes as numbers of their own: a point for each frame, which the picture is
//! then made to follow. The document has those numbers already. A clip is where the
//! sequence draws it, and that is read from the frame, as `look` with `shows` reads it: the
//! box around the pixels the ids pass gives the clip ([`kerf_compose::Pass::Ids`]), so what
//! hides it and what a camera does to it are in it.
//!
//! A `generate` line names the clip, `moves=crate`, and the request holds where it is on
//! each frame of its sequence, from the sequence's first:
//!
//! ```json
//! "moves": [{ "name": "crate", "rate": 16.0, "frame": [640, 360], "places": [[0.48, 0.56, 0.04, 0.06], null] }]
//! ```
//!
//! A place is the middle of the clip's box across and down the sequence's frame, and how
//! wide and tall the box is, each from 0 to 1; it is `null` on a frame the clip does not
//! show on. `frame` is the sequence's size: a guide drawn at another shape fills its size
//! with the sides cut off evenly, so a place across the sequence is not the same share
//! across the guide. A model's program brings them to the frames and the pixels the model
//! makes; Kerf does not know how a model reads them.

use kerf_compose::{ComposeOptions, Compositor, Pass};
use kerf_model::{Catalog, Project};

/// The height of the frames a place is read on, in pixels, where the sequence is as tall.
const HEIGHT: f64 = 216.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// Where a clip goes, as a model is handed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Moves {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its places are at, which is its sequence's rate.
    pub rate: f64,
    /// How wide and tall its sequence is, in pixels.
    pub frame: (u32, u32),
    /// Where it is on each frame of its sequence, from the first: the middle of its box
    /// across and down, and the box's width and height, in shares of the frame.
    pub places: Vec<Option<[f64; 4]>>,
}

impl Moves {
    /// The sequence the clip `id` of `project` is in, by its id. What is wrong is said as a
    /// `generate` line's `moves=` is read. It draws nothing, so a plan can ask it.
    pub fn sequence_of(project: &Project, id: &str) -> std::result::Result<String, String> {
        let at = project
            .locate(id)
            .ok_or_else(|| format!("`moves={id}`: {}", project.no_clip(id)))?;
        Ok(project.sequences[at.sequence].id.to_string())
    }

    /// Where the clip `id` of `project` goes. Media is read from where the project's paths
    /// say, so resolve them first.
    pub fn read(project: &Project, id: &str) -> std::result::Result<Self, String> {
        let sequence = Self::sequence_of(project, id)?;
        let ids = Compositor::new(
            project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some(sequence.clone()),
                pass: Pass::Ids,
                ..ComposeOptions::default()
            },
        )
        .map_err(|error| format!("`moves={id}`: {error}"))?;
        let Some((_, color)) = ids.ids().into_iter().find(|(drawn, _)| drawn == id) else {
            return Err(format!(
                "`moves={id}` names a clip that draws nothing, so it is nowhere on the frame: a camera, a light, a null, and a signal are not followed"
            ));
        };
        let rate = ids.rate();
        let frames = rate.frames_round(ids.duration()).max(0);
        let scale = (HEIGHT / f64::from(ids.size().1.max(1))).min(1.0);
        let places = (0..frames)
            .map(|frame| {
                let drawn = ids.frame(rate.time_of(frame), scale);
                let (width, height) = (drawn.width as usize, drawn.height as usize);
                let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
                for (index, pixel) in drawn.rgba.as_chunks::<4>().0.iter().enumerate() {
                    if color.iter().zip(pixel).all(|(own, is)| own.abs_diff(*is) <= NEAR) {
                        let (x, y) = (index % width, index / width);
                        (left, top) = (left.min(x), top.min(y));
                        (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
                    }
                }
                (right > left).then(|| {
                    // To four places, which is finer than a small frame says.
                    let share = |of: usize, whole: usize| {
                        (of as f64 / whole.max(1) as f64 * 1e4).round() / 1e4
                    };
                    [
                        share(left + right, width * 2),
                        share(top + bottom, height * 2),
                        share(right - left, width),
                        share(bottom - top, height),
                    ]
                })
            })
            .collect();
        Ok(Self {
            name: id.to_owned(),
            rate: rate.fps(),
            frame: ids.size(),
            places,
        })
    }

    /// Whether it is nowhere on any frame, or in one place on all of them: such a clip says
    /// nothing of where anything goes.
    pub fn still(&self) -> bool {
        let mut shown = self.places.iter().flatten();
        match shown.next() {
            None => true,
            Some(first) => shown.all(|place| place == first),
        }
    }

    /// As a request holds it.
    pub(crate) fn asked(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "rate": self.rate,
            "frame": [self.frame.0, self.frame.1],
            "places": self.places,
        })
    }
}
