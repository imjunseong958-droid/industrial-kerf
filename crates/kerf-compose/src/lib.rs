//! Kerf's compositor.
//!
//! A sequence is drawn frame by frame through EffectCraft's renderer
//! instead of an FFmpeg filter graph. [`Compositor::new`] turns the
//! sequence into an EffectCraft project held in memory (`scene` says how
//! each part maps), and [`Compositor::frame`] renders one frame of it on
//! the CPU, reading media through libav. Effects run as part of the frame,
//! raster effects included, so nothing is baked first.

mod color;
mod footage;
mod scene;

use effectcraft_render::{Image, LayerCache, RenderOpts, Renderer};
use footage::LibavFootage;
use kerf_model::{Catalog, Project, Rate, Time};

/// Processed layers kept between frames.
const LAYER_CACHE: usize = 256 << 20;

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

#[derive(Clone, Debug, Default)]
pub struct ComposeOptions {
    /// The sequence to draw; the first one when absent.
    pub sequence: Option<String>,
    /// Draw captions into the picture, as a render burns them in. A viewer
    /// that shows captions itself leaves them out.
    pub captions: bool,
}

/// One frame of straight RGBA, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A sequence ready to draw.
pub struct Compositor {
    scene: scene::Scene,
    footage: LibavFootage,
    cache: LayerCache,
}

impl Compositor {
    pub fn new(
        project: &Project,
        catalog: &Catalog,
        options: &ComposeOptions,
    ) -> Result<Self, ComposeError> {
        let mut scene = scene::build(
            project,
            catalog,
            options.sequence.as_deref(),
            options.captions,
        )?;
        let clips = std::mem::take(&mut scene.clips);
        Ok(Self {
            scene,
            footage: LibavFootage::new(clips),
            cache: LayerCache::new(LAYER_CACHE),
        })
    }

    /// The canvas, made even as a render makes it.
    pub fn size(&self) -> (u32, u32) {
        (self.scene.width, self.scene.height)
    }

    pub fn rate(&self) -> Rate {
        self.scene.rate
    }

    pub fn duration(&self) -> Time {
        self.scene.duration
    }

    /// The picture at sequence time `at` over black, at `scale` of the
    /// canvas (1 is full size).
    pub fn frame(&self, at: Time, scale: f64) -> Frame {
        let image = self.image(at, scale);
        let rgba = image
            .data
            .iter()
            .flat_map(|pixel| {
                let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(byte);
                [r, g, b, 255]
            })
            .collect();
        Frame {
            width: image.width,
            height: image.height,
            rgba,
        }
    }

    /// Media that could not be read, each with why. Their clips draw
    /// nothing.
    pub fn problems(&self) -> Vec<String> {
        self.footage
            .problems()
            .into_iter()
            .map(|(_, problem)| problem)
            .collect()
    }

    /// The premultiplied picture at `at`, transparent where nothing is.
    fn image(&self, at: Time, scale: f64) -> Image {
        let opts = RenderOpts {
            scale: scale.clamp(1.0 / 16.0, 1.0),
            motion_blur: false,
            ..RenderOpts::default()
        };
        let mut renderer = Renderer::new(&self.scene.project, &self.footage, opts);
        renderer.cache = Some(&self.cache);
        renderer.comp_frame(self.scene.comp, scene::ticks(at))
    }
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
