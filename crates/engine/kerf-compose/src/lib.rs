//! Kerf's compositor: a sequence drawn frame by frame from Kerf's model, in
//! linear light, with the drawing carried from FilmCraft (`kerf-draw`), the
//! transitions and effects carried from FilmCraft and the EffectCraft
//! effects Kerf keeps (`kerf-fx`), and media decoded by libav. `draw` says
//! how each part of a sequence is drawn.

mod domains;
mod draw;
mod footage;
mod frames;
mod sound;

use kerf_model::{Catalog, Project, Rate, Time};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    #[error("{0}")]
    Invalid(String),
    #[error("{entity}: {what} cannot be drawn yet")]
    Unsupported { entity: String, what: String },
}

impl ComposeError {
    /// The clip, transition, or sequence the error is about.
    pub fn entity(&self) -> Option<&str> {
        match self {
            Self::Unsupported { entity, .. } => Some(entity),
            Self::Invalid(_) => None,
        }
    }
}

/// Where frames are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accel {
    /// On the GPU when it can draw the frame and the frame is large enough to gain from it.
    #[default]
    Auto,
    /// On the CPU, the reference.
    Never,
    /// On the GPU whenever it can draw the frame, whatever its size.
    Always,
}

#[derive(Clone, Debug, Default)]
pub struct ComposeOptions {
    /// The sequence to draw; the first one when absent.
    pub sequence: Option<String>,
    /// Draw captions into the picture, as a render burns them in. A viewer
    /// that shows captions itself leaves them out.
    pub captions: bool,
    pub accel: Accel,
    /// Keep frames in memory by what they depend on, to give again to any compositor asked for the
    /// same picture. Worth it where frames are asked for again, as playback and previews do, and
    /// not for a render, which only passes through them.
    pub keep_frames: bool,
}

/// One frame of straight RGBA, row by row: 8 bits a component, or 16 from
/// [`Compositor::frame16`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A sequence ready to draw.
pub struct Compositor {
    drawing: draw::Drawing,
}

impl Compositor {
    pub fn new(
        project: &Project,
        catalog: &Catalog,
        options: &ComposeOptions,
    ) -> Result<Self, ComposeError> {
        Ok(Self {
            drawing: draw::Drawing::new(
                project,
                catalog,
                options.sequence.as_deref(),
                options.captions,
                options.accel,
                options.keep_frames,
            )?,
        })
    }

    /// The canvas, made even as a render makes it.
    pub fn size(&self) -> (u32, u32) {
        self.drawing.size()
    }

    pub fn rate(&self) -> Rate {
        self.drawing.rate()
    }

    pub fn duration(&self) -> Time {
        self.drawing.duration()
    }

    /// The picture at sequence time `at` over black, at `scale` of the
    /// canvas (1 is full size).
    pub fn frame(&self, at: Time, scale: f64) -> Frame {
        self.drawing.frame(at, scale)
    }

    /// The picture at sequence time `at` over black, at `scale` of the canvas, as 16-bit
    /// little-endian RGBA, for a master that keeps more than 8 bits. Not kept.
    pub fn frame16(&self, at: Time, scale: f64) -> Frame {
        self.drawing.frame16(at, scale)
    }

    /// A number for everything the frame at sequence time `at` depends on: two frames with the same
    /// number are the same picture, whichever document or compositor they come from. `None` when
    /// the frame depends on more than the document says, such as a LUT file or another track.
    pub fn frame_key(&self, at: Time, scale: f64) -> Option<u64> {
        self.drawing.frame_key(at, scale)
    }

    /// Whether the frame at sequence time `at` is kept, and would not be drawn.
    pub fn is_kept(&self, at: Time, scale: f64) -> bool {
        self.frame_key(at, scale).is_some_and(frames::has)
    }

    /// Whether the GPU would draw the frame at sequence time `at`, at `scale` of the canvas: every
    /// part of it has a kernel, and there is a GPU to run them. A frame that is one clip untouched
    /// is the decoded picture itself and draws on neither.
    pub fn draws_on_gpu(&self, at: Time, scale: f64) -> bool {
        self.drawing.draws_on_gpu(at, scale)
    }

    /// Media that could not be read, each with why. Their clips draw
    /// nothing.
    pub fn problems(&self) -> Vec<String> {
        self.drawing.problems()
    }
}
