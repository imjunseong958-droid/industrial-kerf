//! The GPU's side of the effects Kerf's looks are made of, and of those an editor needs exact
//! (`kernels.wgsl`, and what `kerf_kernel` writes out on the end of it). The CPU code stays the
//! reference and is unchanged: [`Kernel::eval`] reads the same parameters the same way, and a test
//! compares each kernel with [`effects::apply`](crate::video::effects::apply).

mod eval;
mod lumetri;
mod run;
mod stage;
#[cfg(test)]
mod tests;

pub use lumetri::Lumetri;

use super::*;
use crate::video::gpufx::FxOp;
use kerf_color::luma709;
use kerf_gpu::Gpu;

/// One of these effects with its parameters read.
#[derive(Clone, Debug, PartialEq)]
pub enum Kernel {
    /// It changes nothing as set.
    Nothing,
    Noise {
        amount: f32,
        grain: f32,
        colored: bool,
        clip: bool,
        /// The frame's three hashes, each by `hash3`'s constant.
        hashes: [u64; 3],
    },
    ColorKey {
        /// The key's y, cb, cr.
        key: [f32; 3],
        tolerance: f32,
        softness: f32,
    },
    LumaKey {
        threshold: f32,
        cutoff: f32,
    },
    LogoCutout {
        background: [f32; 3],
        threshold: f32,
        softness: f32,
        unmultiply: bool,
        invert: bool,
    },
    UltraKey(Box<UltraKey>),
    RadialZoom {
        kind: u32,
        center: [f32; 2],
        zoom: f32,
        arc: f32,
        quality: f32,
    },
    /// A Lumetri using what its plain numbers do not say.
    Lumetri(Box<Lumetri>),
    /// Add Grain, whose picture is encoded and decoded as a kept effect's is.
    Grain(Box<Grain>),
    Mosaic {
        /// A block's width and height, in pixels.
        block: [f32; 2],
        /// How many blocks there are across and down.
        blocks: [u32; 2],
        soft: f32,
    },
    /// A color effect kept from EffectCraft, on a pixel as `kept::encoded` makes one, which goes back
    /// as `kept::decode_into` takes one, all in a pass.
    Kept(Box<KeptColor>),
    Cineon {
        /// From log to linear (0), from linear to log (1), or from log to log.
        way: u32,
        told: kerf_kernel::Cineon<f32>,
    },
    /// The Key Cleaner, kept from EffectCraft: a key's soft edge given the colors around it, and
    /// its matte more contrast.
    KeyCleaner {
        /// The box passes that find the colors around a pixel, each way, where it looks for any.
        around: Option<Vec<u32>>,
        strength: f32,
        contrast: f32,
    },
    /// The Simple Choker, kept from EffectCraft, drawing its matte in.
    Choke {
        /// The whole pixels its matte comes in by, and how far toward one more.
        radius: u32,
        between: f32,
        /// Whether it comes in at all: a view of the matte alone does not.
        choked: bool,
        matte: bool,
    },
    /// Optics Compensation, kept from EffectCraft: a lens's curve put in or taken out.
    Optics {
        center: [f32; 2],
        /// How much the result is scaled to fit or fill the picture.
        fit: f32,
        focal: f32,
        /// The distance from the center its field of view is measured to, and half that field.
        reach: f32,
        half: f32,
        reverse: bool,
    },
    /// A 360° frame turned, or seen through another field of view (`immersive`).
    Sphere {
        turn: [[f32; 3]; 3],
        /// How much of the sphere the frame holds, across and down.
        share: [f32; 2],
        layout: u32,
    },
    /// A flat picture put on a 360° frame.
    PlaneToSphere {
        turn: [[f32; 3]; 3],
        /// The tangent of half the picture's width on the sphere.
        half: f32,
        feather: f32,
        layout: u32,
    },
    Lens {
        center: [f32; 2],
        /// Half of the picture's diagonal, which its curvature is measured by.
        reach: f32,
        curvature: f32,
    },
}

/// What [`Kernel::Kept`] does between the way there and the way back.
#[derive(Clone, Debug, PartialEq)]
pub enum KeptColor {
    /// Nothing, which an effect that changes nothing as set still goes there and back for.
    Through,
    PhotoFilter(kerf_kernel::PhotoFilter<f32>),
    Unmult(kerf_kernel::Unmult<f32>),
    Curves {
        /// Which of the master, red, green, blue, and alpha have a curve, a bit to each.
        curved: u32,
        /// [`curves_table`].
        table: Vec<f32>,
    },
    HueSaturation {
        /// The master's hue, as a share of a turn, and its saturation and lightness.
        master: [f32; 3],
        /// The hue, saturation, and lightness it colorizes by, when it does.
        colorize: Option<[f32; 3]>,
        ranges: Vec<kerf_raster_fx::HueRange>,
    },
    SelectiveColor {
        /// The cyan, magenta, yellow, and black taken from each of its nine ranges.
        take: [[f32; 4]; 9],
        relative: bool,
    },
    /// The Advanced Spill Suppressor told its key (`keying::advanced_spill`, its Ultra method).
    Spill {
        /// Which of red, green, and blue the key is most of.
        primary: u32,
        amount: f32,
        range: f32,
        luma: f32,
        /// The key's hue, and how far from it, of a turn's half, spill is still taken.
        key_hue: f32,
        tolerance: f32,
        desaturate: f32,
        neutral: f32,
    },
}

/// How many steps a curve of Curves is kept at between 0 and 1, as `Curve::eval` reads it.
const KEPT_CURVE_STEPS: usize = 1024;

/// The table `curves` reads: a row with a step of the master, red, green, and blue curves to a pixel,
/// a row of alpha's, and two rows with each curve's first and last point and the slope it goes on by
/// past it. `None` for a curve with a point outside 0 to 1, where it is not a line past its ends.
pub fn curves_table(curves: &[Option<kerf_raster_fx::Curve>; 5]) -> Option<Vec<f32>> {
    let w = KEPT_CURVE_STEPS + 1;
    let mut px = vec![0.0; w * 4 * 4];
    for (which, curve) in curves.iter().enumerate() {
        let Some(curve) = curve else {
            continue;
        };
        let [first, last] = curve.ends();
        if first[0] < 0.0 || last[0] > 1.0 || curve.table().len() != w {
            return None;
        }
        let (row, channel) = (which / 4, which % 4);
        for (step, value) in curve.table().iter().enumerate() {
            px[(row * w + step) * 4 + channel] = *value;
        }
        for (row, end) in [(2, first), (3, last)] {
            let at = (row * w + which) * 4;
            px[at..at + 3].copy_from_slice(&end);
        }
    }
    Some(px)
}

/// `add_grain`'s numbers, from [`kerf_raster_fx::GrainLook`].
#[derive(Clone, Debug, PartialEq)]
pub struct Grain {
    pub mode: u32,
    pub mono: bool,
    pub seeds: [u32; 3],
    /// What each channel's grain is multiplied by.
    pub gains: [f32; 3],
    pub aspect: f32,
    /// Each channel's grain size, in pixels.
    pub sizes: [f32; 3],
    pub frame: f32,
    pub tones: [f32; 3],
    pub midpoint: f32,
    pub saturation: f32,
}

/// The tint and balance of Add Grain's Color, as `GrainLook::color` applies them, with its intensity.
pub fn grain_gains(look: &kerf_raster_fx::GrainLook) -> [f32; 3] {
    let amp = look.intensity * 0.25;
    let tint = look.tint;
    let tinted = |k: usize| {
        if look.tint_amount > 0.0 {
            let l = luma709(tint[0], tint[1], tint[2]).max(1e-3);
            (1.0 - look.tint_amount) + look.tint_amount * tint[k] / l
        } else {
            1.0
        }
    };
    [0, 1, 2].map(|k| tinted(k) * look.balance[k] * amp * look.channel_intensity[k])
}

/// Ultra Key's numbers as `color::ultra_key` works them out.
#[derive(Clone, Debug, PartialEq)]
pub struct UltraKey {
    /// How far its matte is cleaned up, which blurs it, and the size of a pixel for that.
    choke: f32,
    soften: f32,
    px_scale: f32,
    key: [f32; 3],
    key_size: f32,
    gain: f32,
    highlight: f32,
    shadow: f32,
    tolerance: f32,
    pedestal: f32,
    contrast: f32,
    mid: f32,
    spill: f32,
    desaturate: f32,
    range: f32,
    spill_luma: f32,
    correction: [f32; 3],
    output: u32,
    dominant: u32,
    corrects: bool,
}

/// Whether `gpu` can run these kernels.
pub fn available(gpu: &'static Gpu) -> bool {
    stage::stage(gpu).is_some()
}
