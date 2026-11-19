//! Kerf's compositor: a sequence drawn frame by frame from Kerf's model, in
//! linear light, with the drawing carried from FilmCraft (`kerf-draw`), the
//! transitions and effects carried from FilmCraft and the EffectCraft
//! effects Kerf keeps (`kerf-fx`), and media decoded by libav. `draw` says
//! how each part of a sequence is drawn.

mod alone;
mod disk;
mod domains;
mod draw;
mod footage;
mod frames;
pub mod ramps;
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

/// What of a sequence is drawn: its picture, or a pass of it. It is the document's to name, in a
/// `render` step under a media's line, so the model holds it ([`kerf_model::Pass`]).
pub use kerf_model::Pass;

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
    /// A folder to keep frames in between sessions and renders, by what they depend on:
    /// [`default_frame_folder`] for the user's own.
    pub frame_folder: Option<std::path::PathBuf>,
    /// What is drawn: the picture, unless it says otherwise.
    pub pass: Pass,
    /// A clip of the sequence to draw alone as a matte, in place of any pass: white where a
    /// pixel is that clip's, as [`Pass::Ids`] has it, and black everywhere else, so what
    /// stands before the clip is cut out of it.
    pub matte: Option<String>,
}

/// A stamp of the code that draws frames: what was drawn under one stamp is what would be drawn
/// again, given the same [`Compositor::frame_key`]. Anything kept by frame keys outside this crate
/// is kept under it.
pub const DRAW_STAMP: &str = env!("KERF_DRAW_STAMP");

pub use alone::{draws_effect, effects_on_picture, Alone};
pub use draw::{Form, InDepth, World, ID_COLORS};
pub use disk::{default_frame_folder, frame_cache_stats, FrameCacheStats};

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

/// What the file of a `gen=mesh` clip holds besides its triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshFile {
    pub triangles: usize,
    /// How wide, tall, and long it is at rest, for a height of 1.
    pub size: [f64; 3],
    /// Its animations: what the file calls each, and how many seconds it lasts.
    pub animations: Vec<(String, f64)>,
    /// Which of them the clip plays.
    pub playing: Option<usize>,
    pub cameras: Vec<MeshCamera>,
}

/// A camera in a mesh's file.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshCamera {
    pub name: String,
    /// How much it sees down its picture, in degrees: none for one with no perspective.
    pub fov: Option<f64>,
    /// How wide its picture is for its height, where the file says.
    pub aspect: Option<f64>,
}

/// Where a camera is, in the words of a `gen=camera@1` clip: `x` in frame widths, `y` and `z`
/// in frame heights, and the rest in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPlace {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub pan: f64,
    pub tilt: f64,
    pub roll: f64,
    pub fov: Option<f64>,
}

/// Where a camera is in the space of a mesh's file, which is glTF's: x to the right, y up, and
/// z toward the viewer, in the file's own units. One the file does not hold: a set rebuilt
/// from a frame was seen from its origin, along its z backward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileView {
    pub eye: [f64; 3],
    /// The way its picture's right is, and the way it looks: each of length 1.
    pub right: [f64; 3],
    pub forward: [f64; 3],
}

impl FileView {
    /// The camera at the file's origin, looking along its z backward with its y up.
    pub const ORIGIN: Self = Self {
        eye: [0.0; 3],
        right: [1.0, 0.0, 0.0],
        forward: [0.0, 0.0, -1.0],
    };
}

/// Where a clip of footage has its media's picture on the program's frame at a moment
/// ([`Compositor::lands`]): the numbers the frame is drawn from, with nothing drawn and no
/// file read.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    /// The frame of its media it shows then, counted from 0.
    pub frame: i64,
    /// What its crop leaves of the picture: left, top, right, bottom, each a share of it.
    pub kept: [f64; 4],
    /// How opaque it is then, a transition's fade apart.
    pub opacity: f64,
    /// Whether masks cut its picture.
    pub masked: bool,
    /// The effects done to its picture, by their ids, in order: a look as the effects it
    /// stands for. What a look sets of the clip's own place is in where it lands, and what
    /// an effect does to the picture is not.
    pub effects: Vec<String>,
    /// Where a place in the picture is on the frame, both in shares: (x, y) goes to
    /// (a x + c y + e, b x + d y + f), the six in that order.
    to_frame: [f64; 6],
}

impl Landing {
    /// Where on the frame a place in the media's picture is, each across and down from 0
    /// to 1. Past 0 or 1 it is off the frame.
    pub fn on_frame(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.to_frame;
        [a * x + c * y + e, b * x + d * y + f]
    }
}

impl Compositor {
    pub fn new(
        project: &Project,
        catalog: &Catalog,
        options: &ComposeOptions,
    ) -> Result<Self, ComposeError> {
        Ok(Self {
            drawing: draw::Drawing::new(project, catalog, options)?,
        })
    }

    /// The color each clip that draws has in [`Pass::Ids`], as the bytes of a frame: the program's
    /// clips from its lowest track up, then those of the sequences it nests. Along an edge two
    /// clips share a pixel, which is then between their colors. The first [`ID_COLORS`] each
    /// have a color of their own, and after them the colors come round.
    pub fn ids(&self) -> Vec<(String, [u8; 3])> {
        self.drawing.ids()
    }

    /// What the file of the program's `gen=mesh` clip `clip` holds.
    pub fn mesh_file(&self, clip: &str) -> Result<MeshFile, String> {
        self.drawing.mesh_file(clip)
    }

    /// Where camera `camera` of the file of the program's mesh clip `clip` is at sequence time
    /// `at`, were it a camera of the sequence: the mesh being where its clip puts it then, and
    /// its file's animation as far in as the clip has it. `None` outside the clip.
    pub fn mesh_camera(&self, clip: &str, camera: usize, at: Time) -> Option<CameraPlace> {
        self.drawing.mesh_camera(clip, camera, at)
    }

    /// Where a camera at `view` in the space of the file of the program's mesh clip `clip` is
    /// at sequence time `at`, were it a camera of the sequence, the mesh being where its clip
    /// puts it then: as [`Self::mesh_camera`], for a camera the file does not hold. Its `fov`
    /// is none, a lens being no part of where a camera is.
    pub fn mesh_view(&self, clip: &str, view: &FileView, at: Time) -> Option<CameraPlace> {
        self.drawing.mesh_view(clip, view, at)
    }

    /// What the program has in depth at sequence time `at`, in the world its frame is drawn
    /// from: each clip in depth that draws as its corners and its surface, where the document
    /// puts it and its parents and its file's animation have it then. Nothing is drawn for
    /// it. It is in frame heights, so it is the same whatever size the sequence is, but for
    /// footage fitted to the frame, which is fitted to whole pixels of it.
    pub fn in_depth(&self, at: Time) -> World {
        self.drawing.in_depth(at)
    }

    /// Where the program's clip `clip` has its media's picture on the frame at sequence time
    /// `at`: the placing a frame is drawn by, its crop, fit, scale, turn, and move, its motion
    /// path, and its parents', as [`Self::frame`] puts it down. It is refused, with why,
    /// for a clip that does not show then, one that plays no footage, and one in depth, which
    /// a camera puts on the frame.
    pub fn lands(&self, clip: &str, at: Time) -> Result<Landing, String> {
        self.drawing.lands(clip, at)
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
    /// little-endian RGBA, for a master that keeps more than 8 bits. Kept only on disk.
    pub fn frame16(&self, at: Time, scale: f64) -> Frame {
        self.drawing.frame16(at, scale)
    }

    /// A number for everything the frame at sequence time `at` depends on: two frames with the same
    /// number are the same picture, whichever document or compositor they come from. `None` when
    /// the frame depends on more than the document says, such as a LUT file or another track.
    pub fn frame_key(&self, at: Time, scale: f64) -> Option<u64> {
        self.drawing.frame_key(at, scale)
    }

    /// Wait for the frames asked to be kept on disk to be there.
    pub fn flush_frames(&self) {
        self.drawing.flush_frames();
    }

    /// Whether the frame at sequence time `at` is kept, and would not be drawn.
    pub fn is_kept(&self, at: Time, scale: f64) -> bool {
        self.frame_key(at, scale).is_some_and(frames::has)
    }

    /// The least share of the canvas that goes to the GPU when nothing says it must: smaller frames
    /// are the CPU's, which at [`Accel::Auto`] can cost a scene in depth every core it has.
    pub fn smallest_on_gpu(&self) -> f64 {
        self.drawing.smallest_on_gpu()
    }

    /// Whether the GPU would draw the frame at sequence time `at`, at `scale` of the canvas: every
    /// part of it has a kernel, and there is a GPU to run them. A frame that is one clip untouched
    /// is the decoded picture itself and draws on neither.
    pub fn draws_on_gpu(&self, at: Time, scale: f64) -> bool {
        self.drawing.draws_on_gpu(at, scale)
    }

    /// Why the CPU draws the frame at sequence time `at`, one line a cause: none where the GPU does.
    pub fn why_on_cpu(&self, at: Time, scale: f64) -> Vec<String> {
        self.drawing.why_on_cpu(at, scale)
    }

    /// The picture at sequence time `at` as the GPU draws it, or `None` where [`Self::frame`] would give
    /// it to the CPU, even partway through, which a test cannot tell from a match by the picture.
    pub fn frame_on_gpu(&self, at: Time, scale: f64) -> Option<Frame> {
        self.drawing.frame_on_gpu(at, scale)
    }

    /// Media that could not be read, each with why. Their clips draw
    /// nothing.
    pub fn problems(&self) -> Vec<String> {
        self.drawing.problems()
    }
}
