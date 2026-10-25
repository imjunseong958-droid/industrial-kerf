//! Kerf's compositor.
//!
//! A sequence is drawn frame by frame by one of two renderers
//! ([`Renderer`]): Kerf's own (`draw`), which draws Kerf's model directly
//! in linear light with the drawing carried from FilmCraft, or
//! EffectCraft's, described below, which it is replacing. By default a
//! program is drawn by Kerf's, and by EffectCraft's only when it uses what
//! Kerf's does not draw yet: raster effects and transitions.
//!
//! Through EffectCraft, [`Compositor::new`] turns the
//! sequence into an EffectCraft project held in memory (`scene` says how
//! each part maps), and [`Compositor::frame`] renders one frame of it,
//! reading media through libav. Effects run as part of the frame, raster
//! effects included, so nothing is baked first.
//!
//! With [`Device::Auto`] and a GPU present, each frame is drawn where it is
//! drawn faster: EffectCraft times both and keeps the quicker for the
//! sequence and size. The GPU opens in the background ([`warm_gpu`]): the
//! first time a program uses it, macOS can take most of a minute to build
//! its shaders, and frames are drawn on the CPU meanwhile. Light frames, a few layers of
//! footage, stay on the CPU, which only touches what the layers cover;
//! heavy ones, blurs and glows over the frame, go to the GPU. The CPU
//! renderer stays the reference and draws whatever the GPU declines,
//! every frame after its device is lost included.

mod color;
mod draw;
mod footage;
mod scene;
mod sound;

use effectcraft_gpu::Gpu;
use effectcraft_render::{
    Accelerator, Backend, Image, LayerCache, RenderOpts, Renderer as EffectRenderer,
};
use footage::LibavFootage;
use kerf_model::{Catalog, Project, Rate, Time};
use std::sync::{Once, OnceLock};

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
    /// Where frames are drawn.
    pub device: Device,
    /// What draws them.
    pub renderer: Renderer,
}

/// What draws a compositor's frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// Kerf's own where it draws the program, else EffectCraft's.
    #[default]
    Auto,
    /// EffectCraft's renderer, on the CPU or the GPU.
    EffectCraft,
    /// Kerf's own, on the CPU: the picture in linear light, with FilmCraft's
    /// drawing. It does not draw raster effects and transitions yet.
    Kerf,
}

impl Renderer {
    /// The renderer `KERF_RENDERER` names (`kerf`, `effectcraft`, or
    /// `auto`), or this one.
    pub fn from_env(self) -> Self {
        match std::env::var("KERF_RENDERER").as_deref() {
            Ok("kerf") => Self::Kerf,
            Ok("effectcraft") => Self::EffectCraft,
            Ok("auto") => Self::Auto,
            _ => self,
        }
    }
}

/// Where frames are drawn. Without a GPU, they all are on the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Cpu,
    /// On whichever of the CPU and the GPU draws the sequence faster, once
    /// the GPU is open; on the CPU until then.
    Auto,
    /// On the GPU, the CPU drawing only what it declines. Waits for the GPU
    /// to open.
    Gpu,
}

/// The GPU frames are drawn on, opened once for the process: `None`
/// without a usable adapter.
static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Open the GPU in the background, once, so frames that can use it find it
/// open.
pub fn warm_gpu() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("kerf-gpu".into())
            .spawn(|| {
                GPU.get_or_init(Gpu::headless);
            });
    });
}

/// The GPU, opened here if need be.
fn shared_gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(Gpu::headless).as_ref()
}

/// The GPU if it is open, without waiting for it.
fn opened_gpu() -> Option<&'static Gpu> {
    GPU.get().and_then(Option::as_ref)
}

/// The GPU adapter frames are drawn on (`"Apple M2 (Metal)"`), once it is
/// open; `None` while it opens, and on a machine without one.
pub fn gpu_adapter() -> Option<String> {
    opened_gpu().map(Accelerator::name)
}

/// Open the GPU now, waiting for it, and name its adapter (benchmarks).
pub fn open_gpu() -> Option<String> {
    shared_gpu().map(Accelerator::name)
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
    inner: Inner,
}

enum Inner {
    EffectCraft(EffectCraft),
    Kerf(draw::Drawing),
}

/// A sequence drawn by EffectCraft's renderer.
struct EffectCraft {
    scene: scene::Scene,
    footage: LibavFootage,
    cache: LayerCache,
    device: Device,
}

impl Compositor {
    pub fn new(
        project: &Project,
        catalog: &Catalog,
        options: &ComposeOptions,
    ) -> Result<Self, ComposeError> {
        let kerf = || {
            draw::Drawing::new(
                project,
                catalog,
                options.sequence.as_deref(),
                options.captions,
            )
        };
        let inner = match options.renderer {
            Renderer::Kerf => Inner::Kerf(kerf()?),
            Renderer::EffectCraft => {
                Inner::EffectCraft(EffectCraft::new(project, catalog, options)?)
            }
            Renderer::Auto => match kerf() {
                Ok(drawing) => Inner::Kerf(drawing),
                Err(ComposeError::Unsupported { .. }) => {
                    Inner::EffectCraft(EffectCraft::new(project, catalog, options)?)
                }
                Err(error) => return Err(error),
            },
        };
        Ok(Self { inner })
    }

    /// The renderer `options` draw `project` with, Kerf's or EffectCraft's,
    /// found without making either ready to draw.
    pub fn resolve(
        project: &Project,
        catalog: &Catalog,
        options: &ComposeOptions,
    ) -> Result<Renderer, ComposeError> {
        let kerf = || {
            draw::Drawing::new(
                project,
                catalog,
                options.sequence.as_deref(),
                options.captions,
            )
        };
        match options.renderer {
            Renderer::Auto => match kerf() {
                Ok(_) => Ok(Renderer::Kerf),
                Err(ComposeError::Unsupported { .. }) => Ok(Renderer::EffectCraft),
                Err(error) => Err(error),
            },
            renderer => Ok(renderer),
        }
    }

    /// What draws this compositor's frames: Kerf's renderer or
    /// EffectCraft's, never [`Renderer::Auto`].
    pub fn renderer(&self) -> Renderer {
        match self.inner {
            Inner::EffectCraft(_) => Renderer::EffectCraft,
            Inner::Kerf(_) => Renderer::Kerf,
        }
    }

    /// The GPU adapter this compositor draws on now, if any.
    pub fn gpu(&self) -> Option<String> {
        match &self.inner {
            Inner::EffectCraft(compositor) => compositor.device().map(Accelerator::name),
            Inner::Kerf(_) => None,
        }
    }

    /// The canvas, made even as a render makes it.
    pub fn size(&self) -> (u32, u32) {
        match &self.inner {
            Inner::EffectCraft(compositor) => (compositor.scene.width, compositor.scene.height),
            Inner::Kerf(drawing) => drawing.size(),
        }
    }

    pub fn rate(&self) -> Rate {
        match &self.inner {
            Inner::EffectCraft(compositor) => compositor.scene.rate,
            Inner::Kerf(drawing) => drawing.rate(),
        }
    }

    pub fn duration(&self) -> Time {
        match &self.inner {
            Inner::EffectCraft(compositor) => compositor.scene.duration,
            Inner::Kerf(drawing) => drawing.duration(),
        }
    }

    /// The picture at sequence time `at` over black, at `scale` of the
    /// canvas (1 is full size).
    pub fn frame(&self, at: Time, scale: f64) -> Frame {
        match &self.inner {
            Inner::EffectCraft(compositor) => compositor.frame(at, scale),
            Inner::Kerf(drawing) => drawing.frame(at, scale),
        }
    }

    /// Media that could not be read, each with why. Their clips draw
    /// nothing.
    pub fn problems(&self) -> Vec<String> {
        match &self.inner {
            Inner::EffectCraft(compositor) => compositor
                .footage
                .problems()
                .into_iter()
                .map(|(_, problem)| problem)
                .collect(),
            Inner::Kerf(drawing) => drawing.problems(),
        }
    }
}

impl EffectCraft {
    fn new(
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
            device: options.device,
        })
        .inspect(|compositor| match compositor.device {
            Device::Cpu => {}
            Device::Auto => warm_gpu(),
            Device::Gpu => {
                shared_gpu();
            }
        })
    }

    /// The GPU this compositor draws on now, if any.
    fn device(&self) -> Option<&'static Gpu> {
        match self.device {
            Device::Cpu => None,
            Device::Auto => opened_gpu(),
            Device::Gpu => shared_gpu(),
        }
        .filter(|gpu| gpu.available())
    }

    fn frame(&self, at: Time, scale: f64) -> Frame {
        if let Some(frame) = self.plain_frame(at, scale) {
            return frame;
        }
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

    /// A frame that is one clip's media alone, at full size: its picture as
    /// decoded, which drawing it would give again.
    fn plain_frame(&self, at: Time, scale: f64) -> Option<Frame> {
        if scale < 1.0 {
            return None;
        }
        let comp = self.scene.project.comp(self.scene.comp)?;
        let t = scene::ticks(at);
        // Layers of tracks effects read are out of sight.
        let mut showing = comp
            .layers
            .iter()
            .filter(|layer| layer.switches.video && layer.is_active_at(t));
        let layer = showing.next()?;
        if showing.next().is_some() {
            return None;
        }
        let footage = self.scene.plain.get(&layer.id)?;
        let rgba = self
            .footage
            .rgba(*footage, effectcraft_time::Tick(t.0 - layer.start_time.0))?;
        Some(Frame {
            width: self.scene.width,
            height: self.scene.height,
            rgba,
        })
    }

    /// The premultiplied picture at `at`, transparent where nothing is.
    fn image(&self, at: Time, scale: f64) -> Image {
        let gpu = self.device();
        let opts = RenderOpts {
            scale: scale.clamp(1.0 / 16.0, 1.0),
            motion_blur: false,
            backend: match (self.device, gpu) {
                (Device::Auto, Some(_)) => Backend::Auto,
                (Device::Gpu, Some(_)) => Backend::Gpu,
                _ => Backend::Cpu,
            },
            ..RenderOpts::default()
        };
        let mut renderer = EffectRenderer::new(&self.scene.project, &self.footage, opts);
        renderer.cache = Some(&self.cache);
        renderer.accel = gpu.map(|gpu| gpu as &dyn Accelerator);
        renderer.comp_frame(self.scene.comp, scene::ticks(at))
    }
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
