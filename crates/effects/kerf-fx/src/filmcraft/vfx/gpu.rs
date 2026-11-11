//! The GPU's side of the effects Kerf's looks are made of (`kernels.wgsl`). The CPU code stays the
//! reference and is unchanged: [`Kernel::eval`] reads the same parameters the same way, and a test
//! compares each kernel with [`effects::apply`](crate::filmcraft::effects::apply).

use super::*;
use crate::filmcraft::effects::{
    boxes_for_gauss, choice, curve_lut, curve_param, f, gaussian_boxes, grade_space, hue_lut,
    is_identity_curve, on, text, Wheels,
};
use crate::filmcraft::gpu::run as run_ops;
use crate::filmcraft::gpufx::FxOp;
use kerf_color::{luma709, rgb_to_ycbcr, Lut, Matrix};
use kerf_draw::blend::Blend;
use kerf_geom::Rect;
use kerf_gpu::{wgpu, Frame, Gpu, Picture};
use std::sync::{Arc, OnceLock};

/// One of these effects with its parameters read.
#[derive(Clone, Debug, PartialEq)]
pub enum Kernel {
    /// It changes nothing as set.
    Nothing,
    WonderGlow {
        threshold: f32,
        saturation: f32,
        tint: Option<[f32; 3]>,
        /// Half of the radius in the picture's pixels.
        radius: f32,
        gain: f32,
        screen: bool,
    },
    Glint {
        threshold: f32,
        /// The angle of each ray, in radians.
        rays: Vec<f32>,
        length: f32,
        tint: [f32; 3],
        colorize: f32,
        gain: [f32; 3],
    },
    RgbSplit {
        offset: [f32; 2],
        radial: Option<([f32; 2], f32, f32)>,
        blend: f32,
    },
    WaveWarp {
        across: [f32; 2],
        height: f32,
        width: f32,
        phase: f32,
    },
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
    Leaks {
        colors: [[f32; 3]; 2],
        intensity: f32,
        scale: f32,
        direction: [f32; 2],
        time: f32,
        /// `hash_u`'s seed for the first octave, and the second, and the third.
        seeds: [u64; 3],
        mode: u32,
    },
    /// Add Grain, whose picture is encoded and decoded as a kept effect's is.
    Grain(Box<Grain>),
    CornerPin {
        /// From a pixel to where it comes from across the picture, 0 to 1, row by row.
        map: [f32; 9],
        size: [f32; 2],
    },
    DropShadow {
        /// How far the shadow lies from what casts it, in pixels.
        offset: [f64; 2],
        color: [f32; 3],
        opacity: f32,
        /// What softens it, when anything does.
        blur: Option<FxOp>,
        only: bool,
    },
    RoundedCrop(kerf_kernel::RoundedCrop<f32>),
    Spacer {
        /// Where the picture is put: its left, its top, and how much smaller it is.
        place: [f64; 3],
        told: kerf_kernel::Spacer<f32>,
    },
    Mosaic {
        /// A block's width and height, in pixels.
        block: [f32; 2],
        /// How many blocks there are across and down.
        blocks: [u32; 2],
        soft: f32,
    },
}

/// What a Lumetri does that [`FxOp::Lumetri`] does not, in the order `lumetri` does it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lumetri {
    input: Option<Arc<Lut>>,
    /// Its plain numbers, and its named look and wheels when nothing has to come before those.
    base: FxOp,
    more: Option<LumetriMore>,
    hsl: Option<HslSecondary>,
    sharpen: Option<FxOp>,
}

/// `lumetri_advanced`'s curves, and a look's LUT and the wheels that come after it.
#[derive(Clone, Debug, PartialEq)]
struct LumetriMore {
    look: Option<(Arc<Lut>, f32)>,
    wheels: Option<Wheels>,
    /// Luma, red, green, blue; hue vs sat, hue vs hue, hue vs luma, luma vs sat; sat vs sat.
    curves: [Option<Vec<f32>>; 9],
}

/// `hsl_secondary` with no Denoise: its key, the blur over it, and what it does inside it.
#[derive(Clone, Debug, PartialEq)]
struct HslSecondary {
    hue: [f32; 2],
    saturation: f32,
    luma: [f32; 2],
    soft: f32,
    blur: Option<FxOp>,
    show_mask: u32,
    /// Temperature, tint, saturation, hue shift.
    correct: [f32; 4],
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

const CURVE_POINTS: usize = 1024;

/// `hash_u`'s constant on its seed.
const HASH_SEED: u64 = 0x27D4_EB2F_1656_67C5;

/// The first, in `Kernel::Lumetri`'s table, of a LUT's rows: three when there are curves.
fn lut_first_row(more: &LumetriMore) -> usize {
    if more.curves.iter().any(Option::is_some) {
        3
    } else {
        0
    }
}

/// A LUT's cube, or `None` for a shaper or for no cube, which a kernel does not read.
fn cube_of(lut: &Lut) -> Option<&kerf_color::Lut3d> {
    lut.cube.as_ref().filter(|_| lut.shaper.is_none())
}

/// The table `lumetri_more` and `lumetri_input` read: curves in rows, a cube below them.
fn lumetri_table(curves: &[Option<Vec<f32>>; 9], first: usize, lut: Option<&Lut>) -> Image {
    let cube = lut.and_then(cube_of);
    let size = cube.map_or(0, |cube| cube.size);
    let w = (size * size).max(if first > 0 { CURVE_POINTS } else { 1 });
    let h = first + size;
    let mut px = vec![0.0; w * h.max(1) * 4];
    for (index, curve) in curves.iter().enumerate() {
        let Some(curve) = curve else {
            continue;
        };
        let (row, channel) = (index / 4, index % 4);
        for (x, value) in curve.iter().enumerate() {
            px[(row * w + x) * 4 + channel] = *value;
        }
    }
    if let Some(cube) = cube {
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let value = cube.at(r, g, b);
                    let at = ((first + b) * w + r + g * size) * 4;
                    px[at..at + 3].copy_from_slice(&value);
                }
            }
        }
    }
    Image { w, h: h.max(1), px }
}

impl Lumetri {
    /// A Lumetri that its plain numbers do not say all of, or `None`: when they do, which the op
    /// draws; in HDR; and for what stays on the CPU, a LUT with a 1D shaper and the HSL Secondary's
    /// Denoise, a median that costs the GPU what it costs the CPU.
    fn eval(e: &EffectInstance, cx: &FxCtx, (w, h): (usize, usize)) -> Option<Self> {
        let mut base = FxOp::eval(e, cx, w, h)?;
        let FxOp::Lumetri {
            plain,
            gpu_capable,
            look,
            wheels,
            ..
        } = &mut base
        else {
            return None;
        };
        if *plain {
            return None;
        }
        if grade_space(e, cx, "hdr_white").is_hdr()
            || grade_space(e, cx, "curves_hdr_range").is_hdr()
        {
            return None;
        }
        *plain = true;
        *gpu_capable = true;
        let (basic_on, creative_on, curves_on) =
            (on(e, "basic_on"), on(e, "creative_on"), on(e, "curves_on"));
        let cube = |id: &str| {
            let lut = crate::filmcraft::luts::resolve(cx.project, text(e, id));
            lut.filter(|lut| cube_of(lut).is_some())
        };
        let input = if basic_on && !text(e, "input_lut").is_empty() {
            Some(cube("input_lut")?)
        } else {
            None
        };
        let look_lut = if creative_on && !text(e, "look_lut").is_empty() {
            Some(cube("look_lut")?)
        } else {
            None
        };
        // A look's LUT comes before the wheels.
        let wheels_after = if look_lut.is_some() {
            wheels.take()
        } else {
            None
        };
        if look_lut.is_some() {
            *look = 0;
        }
        let lut = |id: &str| {
            curve_param(e, id)
                .filter(|curve| curves_on && !is_identity_curve(curve))
                .map(|curve| curve_lut(&curve, CURVE_POINTS))
        };
        let hue = |id: &str| {
            curve_param(e, id)
                .filter(|_| curves_on)
                .and_then(|curve| hue_lut(&curve, CURVE_POINTS))
        };
        let curves = [
            lut("curve_luma"),
            lut("curve_red"),
            lut("curve_green"),
            lut("curve_blue"),
            hue("hue_vs_sat"),
            hue("hue_vs_hue"),
            hue("hue_vs_luma"),
            hue("luma_vs_sat"),
            hue("sat_vs_sat"),
        ];
        let more =
            (look_lut.is_some() || wheels_after.is_some() || curves.iter().any(Option::is_some))
                .then(|| LumetriMore {
                    look: look_lut.map(|lut| (lut, f(e, "look_intensity", cx) / 100.0)),
                    wheels: wheels_after,
                    curves,
                });
        let hsl = if crate::filmcraft::effects::b(e, "hsl_on") {
            let denoise = f(e, "hsl_denoise", cx).clamp(0.0, 100.0) / 100.0;
            if denoise > 0.0 {
                return None;
            }
            let sigma = f(e, "hsl_blur", cx).clamp(0.0, 100.0) / 100.0 * 20.0 * cx.px_scale;
            let radii = || {
                boxes_for_gauss(sigma, 3)
                    .into_iter()
                    .map(|r| r as u32)
                    .collect::<Vec<u32>>()
            };
            Some(HslSecondary {
                hue: [
                    f(e, "hsl_hue", cx) / 360.0,
                    (f(e, "hsl_hue_range", cx) / 360.0).max(1e-3),
                ],
                saturation: f(e, "hsl_sat_min", cx) / 100.0,
                luma: [
                    f(e, "hsl_luma_min", cx) / 100.0,
                    f(e, "hsl_luma_max", cx) / 100.0,
                ],
                soft: (f(e, "hsl_soft", cx) / 100.0 * 0.3).max(0.01),
                blur: (sigma >= 0.3).then(|| FxOp::Gaussian {
                    rx: radii(),
                    ry: radii(),
                    repeat: true,
                }),
                show_mask: choice(e, "hsl_show_mask"),
                correct: [
                    f(e, "hsl_temp", cx) / 100.0,
                    f(e, "hsl_tint", cx) / 100.0,
                    f(e, "hsl_sat", cx) / 100.0,
                    f(e, "hsl_hue_shift", cx) / 360.0,
                ],
            })
        } else {
            None
        };
        let sharpen = if creative_on {
            f(e, "sharpen", cx) / 100.0
        } else {
            0.0
        };
        let sharpen = (sharpen.abs() > 1e-3).then(|| {
            let radius = 1.2 * cx.px_scale.max(0.35);
            let (rx, ry) = gaussian_boxes(w, h, radius, radius);
            FxOp::Unsharp {
                rx,
                ry,
                amount: sharpen.max(-1.0),
                threshold: 0.0,
            }
        });
        Some(Self {
            input,
            base,
            more,
            hsl,
            sharpen,
        })
    }

    fn run(&self, frame: &mut Frame<'static>, picture: Picture) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        let mut picture = picture;
        if let Some(cube) = self.input.as_deref().and_then(cube_of) {
            let none: [Option<Vec<f32>>; 9] = Default::default();
            let table = lumetri_table(&none, 0, self.input.as_deref());
            let table = frame.image(&table);
            let next = stage.pass(
                frame,
                &stage.lumetri_input,
                &picture,
                Some(&table),
                &U {
                    a: [cube.size as u32, 0, 0, 0],
                    p0: [
                        cube.domain_min[0],
                        cube.domain_min[1],
                        cube.domain_min[2],
                        0.0,
                    ],
                    p1: [
                        cube.domain_max[0],
                        cube.domain_max[1],
                        cube.domain_max[2],
                        0.0,
                    ],
                    ..U::default()
                },
            );
            frame.release(table);
            frame.release(picture);
            picture = next;
        }
        picture = run_ops(frame, picture, std::slice::from_ref(&self.base))?;
        if let Some(more) = &self.more {
            let first = lut_first_row(more);
            let lut = more.look.as_ref().map(|(lut, _)| &**lut);
            let table = frame.image(&lumetri_table(&more.curves, first, lut));
            let cube = lut.and_then(cube_of);
            let mut parts = u32::from(cube.is_some()) | u32::from(more.wheels.is_some()) << 1;
            for (index, curve) in more.curves.iter().enumerate() {
                parts |= u32::from(curve.is_some()) << (index + 2);
            }
            let (low, high, size) = cube.map_or(([0.0; 3], [1.0; 3], 0), |cube| {
                (cube.domain_min, cube.domain_max, cube.size as u32)
            });
            let amount = more.look.as_ref().map_or(0.0, |(_, amount)| *amount);
            let [lift, mid, gain] = more.wheels.map_or([[0.0; 3]; 3], |wheels| {
                [wheels.lift, wheels.mid, wheels.gain]
            });
            let next = stage.pass(
                frame,
                &stage.lumetri_more,
                &picture,
                Some(&table),
                &U {
                    a: [parts, size, first as u32, 0],
                    p0: [low[0], low[1], low[2], amount],
                    p1: [high[0], high[1], high[2], 0.0],
                    p2: [lift[0], lift[1], lift[2], 0.0],
                    p3: [mid[0], mid[1], mid[2], 0.0],
                    p4: [gain[0], gain[1], gain[2], 0.0],
                    ..U::default()
                },
            );
            frame.release(table);
            frame.release(picture);
            picture = next;
        }
        if let Some(hsl) = &self.hsl {
            let key = stage.pass(
                frame,
                &stage.hsl_key,
                &picture,
                None,
                &U {
                    p0: [hsl.hue[0], hsl.hue[1], hsl.saturation, hsl.luma[0]],
                    p1: [hsl.luma[1], hsl.soft, 0.0, 0.0],
                    ..U::default()
                },
            );
            let key = match &hsl.blur {
                Some(blur) => run_ops(frame, key, std::slice::from_ref(blur))?,
                None => key,
            };
            let next = stage.pass(
                frame,
                &stage.hsl_apply,
                &picture,
                Some(&key),
                &U {
                    a: [hsl.show_mask, 0, 0, 0],
                    p0: hsl.correct,
                    ..U::default()
                },
            );
            frame.release(key);
            frame.release(picture);
            picture = next;
        }
        if let Some(sharpen) = &self.sharpen {
            picture = run_ops(frame, picture, std::slice::from_ref(sharpen))?;
        }
        Some(picture)
    }
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

/// `hash3`'s constant on its third coordinate.
const HASH_Z: u64 = 0x1656_67B1_9E37_79F9;

impl Kernel {
    /// The effect `e` on a picture of `size`, if it has a kernel.
    pub fn eval(e: &EffectInstance, cx: &FxCtx, size: (usize, usize)) -> Option<Self> {
        Some(match e.effect.as_str() {
            "wonder_glow" => Kernel::WonderGlow {
                threshold: fv(e, "threshold", cx) / 100.0,
                saturation: fv(e, "saturation", cx) / 100.0,
                tint: bv(e, "use_color").then(|| lin(cv(e, "color", cx))),
                radius: fv(e, "radius", cx) * cx.px_scale * 0.5,
                gain: fv(e, "intensity", cx) / 100.0 * 1.5,
                screen: chv(e, "mode") == 1,
            },
            "glint" => {
                let count = [2usize, 4, 6, 8][chv(e, "rays").min(3) as usize];
                let rotation = fv(e, "rotation", cx).to_radians();
                let intensity = fv(e, "intensity", cx) / 100.0 * 3.0;
                let color = lin(cv(e, "color", cx));
                Kernel::Glint {
                    threshold: fv(e, "threshold", cx) / 100.0,
                    rays: (0..count)
                        .map(|i| rotation + std::f32::consts::TAU * i as f32 / count as f32)
                        .collect(),
                    length: fv(e, "length", cx) * cx.px_scale * 0.35,
                    tint: color,
                    colorize: fv(e, "colorize", cx) / 100.0,
                    gain: color.map(|channel| intensity * channel.max(0.2)),
                }
            }
            "rgb_split" => {
                let amount = fv(e, "amount", cx) * cx.px_scale;
                if amount.abs() < 1e-3 {
                    return Some(Kernel::Nothing);
                }
                let angle = fv(e, "angle", cx).to_radians();
                let (w, h) = size;
                let sized = Image {
                    w,
                    h,
                    px: Vec::new(),
                };
                let center = pv(e, "center", cx, &sized);
                let reach = ((w as f64).hypot(h as f64) / 2.0) as f32;
                Kernel::RgbSplit {
                    offset: [angle.cos() * amount, angle.sin() * amount],
                    radial: (chv(e, "mode") == 1).then_some((
                        [center.x as f32, center.y as f32],
                        reach,
                        amount,
                    )),
                    blend: fv(e, "blend", cx) / 100.0,
                }
            }
            "wave_warp" => {
                let direction = f64::from(fv(e, "direction", cx)).to_radians();
                let phase = cx.seconds * f64::from(fv(e, "speed", cx)) * std::f64::consts::TAU;
                Kernel::WaveWarp {
                    across: [direction.sin() as f32, -direction.cos() as f32],
                    height: f64::from(fv(e, "height", cx)) as f32 * cx.px_scale,
                    width: (fv(e, "width", cx) * cx.px_scale).max(1.0),
                    // Only its angle counts, which keeps its precision in a long clip.
                    phase: phase.rem_euclid(std::f64::consts::TAU) as f32,
                }
            }
            "noise" => {
                let amount = fv(e, "amount", cx) / 100.0;
                if amount <= 0.0 {
                    return Some(Kernel::Nothing);
                }
                let frame = crate::filmcraft::effects::noise_frame_seed(cx);
                Kernel::Noise {
                    amount,
                    grain: (fv(e, "grain_size", cx).max(1.0) * cx.px_scale).max(1.0),
                    colored: bv(e, "color"),
                    clip: bv(e, "clip"),
                    hashes: [frame, frame + 7777, frame + 99_991].map(|z| z.wrapping_mul(HASH_Z)),
                }
            }
            "color_key" => {
                let color = crate::filmcraft::effects::color(e, "color", cx);
                let y = rgb_to_ycbcr(color[0], color[1], color[2], Matrix::Bt709);
                Kernel::ColorKey {
                    key: y,
                    tolerance: crate::filmcraft::effects::f(e, "tolerance", cx) / 255.0 * 0.6
                        + 0.01,
                    softness: crate::filmcraft::effects::f(e, "feather", cx) / 50.0 * 0.2 + 0.01,
                }
            }
            "luma_key" => Kernel::LumaKey {
                threshold: crate::filmcraft::effects::f(e, "threshold", cx) / 100.0,
                cutoff: crate::filmcraft::effects::f(e, "cutoff", cx) / 100.0,
            },
            "logo_cutout" => Kernel::LogoCutout {
                background: match chv(e, "background") {
                    0 => [1.0; 3],
                    1 => [0.0; 3],
                    _ => {
                        let c = cv(e, "color", cx);
                        [c[0], c[1], c[2]]
                    }
                },
                threshold: fv(e, "threshold", cx) / 100.0,
                softness: (fv(e, "softness", cx) / 100.0).max(1e-3),
                unmultiply: bv(e, "unmultiply"),
                invert: bv(e, "invert"),
            },
            "ultra_key" => {
                let color = cv(e, "key_color", cx);
                let key = rgb_to_ycbcr(color[0], color[1], color[2], Matrix::Bt709);
                let preset =
                    crate::filmcraft::project::effect::ultra_key_setting(chv(e, "setting"));
                let pf = |id: &str| {
                    preset
                        .and_then(|rows| {
                            rows.iter().find(|(k, _)| *k == id).map(|(_, v)| *v as f32)
                        })
                        .unwrap_or_else(|| fv(e, id, cx))
                };
                let correction = [
                    fv(e, "cc_saturation", cx) / 100.0,
                    fv(e, "cc_hue", cx) / 360.0,
                    fv(e, "cc_luminance", cx) / 100.0,
                ];
                Kernel::UltraKey(Box::new(UltraKey {
                    choke: pf("choke") / 100.0,
                    soften: pf("soften") / 100.0,
                    px_scale: cx.px_scale,
                    key,
                    key_size: (key[1] * key[1] + key[2] * key[2]).max(1e-6),
                    gain: 1.0 + 2.0 * pf("transparency") / 100.0,
                    highlight: pf("highlight") / 100.0,
                    shadow: pf("shadow") / 100.0,
                    tolerance: 0.08 + pf("tolerance") / 100.0 * 0.5,
                    pedestal: (pf("pedestal") / 100.0 * 0.3).min(0.9),
                    contrast: pf("contrast") / 100.0,
                    mid: pf("mid_point") / 100.0,
                    spill: pf("spill") / 100.0,
                    desaturate: pf("desaturate") / 100.0,
                    range: pf("range") / 100.0,
                    spill_luma: pf("spill_luma") / 100.0,
                    correction,
                    output: chv(e, "output"),
                    dominant: if color[1] >= color[0] && color[1] >= color[2] {
                        1
                    } else if color[2] >= color[0] {
                        2
                    } else {
                        0
                    },
                    corrects: (correction[0] - 1.0).abs() > 1e-4
                        || correction[1].abs() > 1e-6
                        || (correction[2] - 1.0).abs() > 1e-4,
                }))
            }
            "lumetri" => Kernel::Lumetri(Box::new(Lumetri::eval(e, cx, size)?)),
            "light_leaks" => {
                let direction = fv(e, "direction", cx).to_radians();
                let seed = fv(e, "seed", cx).max(0.0) as u64;
                Kernel::Leaks {
                    colors: [lin(cv(e, "c1", cx)), lin(cv(e, "c2", cx))],
                    intensity: fv(e, "intensity", cx) / 100.0,
                    scale: (fv(e, "scale", cx) / 100.0).max(0.05) * size.0.max(size.1) as f32 * 0.6,
                    direction: [direction.cos(), direction.sin()],
                    time: (cx.seconds * f64::from(fv(e, "speed", cx))) as f32,
                    seeds: [seed, seed + 1, seed + 2].map(|one| one.wrapping_mul(HASH_SEED)),
                    mode: chv(e, "mode"),
                }
            }
            "corner_pin" => {
                let (w, h) = (size.0 as f64, size.1 as f64);
                let sized = Image {
                    w: size.0,
                    h: size.1,
                    px: Vec::new(),
                };
                let q = [
                    pv(e, "upper_left", cx, &sized),
                    pv(e, "upper_right", cx, &sized),
                    pv(e, "lower_right", cx, &sized),
                    pv(e, "lower_left", cx, &sized),
                ];
                let ident = [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(w, 0.0),
                    Vec2::new(w, h),
                    Vec2::new(0.0, h),
                ];
                // Corners left where they are change nothing, and neither does a map with no way back.
                if q.iter().zip(&ident).all(|(a, b)| (*a - *b).length() < 1e-6) {
                    return Some(Kernel::Nothing);
                }
                let Some(inverse) = distort::h_inverse(&distort::square_to_quad(q)) else {
                    return Some(Kernel::Nothing);
                };
                let map = inverse.map(|value| value as f32);
                // Corners in a line divide by nothing, and what comes of that is the CPU's to draw.
                if map.iter().any(|value| !value.is_finite()) {
                    return None;
                }
                Kernel::CornerPin {
                    map,
                    size: [w as f32, h as f32],
                }
            }
            "drop_shadow" => {
                let c = crate::filmcraft::effects::color(e, "color", cx);
                let direction = f64::from(f(e, "direction", cx)).to_radians();
                let distance = f64::from(f(e, "distance", cx)) * f64::from(cx.px_scale);
                let soft = f(e, "softness", cx) * cx.px_scale * 0.5;
                Kernel::DropShadow {
                    offset: [direction.sin() * distance, -direction.cos() * distance],
                    color: dec([c[0], c[1], c[2]]),
                    opacity: f(e, "opacity", cx) / 100.0,
                    blur: (soft > 0.3).then(|| {
                        let (rx, ry) = gaussian_boxes(size.0, size.1, soft, soft);
                        FxOp::Gaussian {
                            rx,
                            ry,
                            repeat: false,
                        }
                    }),
                    only: crate::filmcraft::effects::b(e, "only"),
                }
            }
            "rounded_crop" => {
                let (w, h) = (size.0 as f32, size.1 as f32);
                let (l, t, r, b) = (
                    fv(e, "left", cx) / 100.0 * w,
                    fv(e, "top", cx) / 100.0 * h,
                    w - fv(e, "right", cx) / 100.0 * w,
                    h - fv(e, "bottom", cx) / 100.0 * h,
                );
                Kernel::RoundedCrop(kerf_kernel::RoundedCrop {
                    edges: [l, t, r, b],
                    radius: (fv(e, "radius", cx) * cx.px_scale)
                        .min(((r - l).min(b - t) / 2.0).max(0.0)),
                    feather: fv(e, "feather", cx) * cx.px_scale,
                    border: fv(e, "border", cx) * cx.px_scale,
                    border_color: lin(cv(e, "border_color", cx)),
                })
            }
            "spacer" => {
                let k = f64::from(cx.px_scale);
                let l = f64::from(fv(e, "left", cx)) * k;
                let (t, r, b) = if bv(e, "uniform") {
                    (l, l, l)
                } else {
                    (
                        f64::from(fv(e, "top", cx)) * k,
                        f64::from(fv(e, "right", cx)) * k,
                        f64::from(fv(e, "bottom", cx)) * k,
                    )
                };
                let (w, h) = (size.0 as f64, size.1 as f64);
                let (bw, bh) = ((w - l - r).max(1.0), (h - t - b).max(1.0));
                let s = (bw / w).min(bh / h);
                let (dw, dh) = (w * s, h * s);
                let (x0, y0) = (l + (bw - dw) / 2.0, t + (bh - dh) / 2.0);
                let radius = (f64::from(fv(e, "radius", cx)) * k).min(dw.min(dh) / 2.0);
                Kernel::Spacer {
                    place: [x0, y0, s],
                    told: kerf_kernel::Spacer {
                        edges: [x0 as f32, y0 as f32, (x0 + dw) as f32, (y0 + dh) as f32],
                        radius: if radius > 0.0 { radius as f32 } else { 0.0 },
                        fill: if bv(e, "fill") { 1.0 } else { 0.0 },
                        color: lin(cv(e, "color", cx)),
                    },
                }
            }
            "mosaic" => {
                let across = fv(e, "horizontal", cx).max(1.0) as usize;
                let down = fv(e, "vertical", cx).max(1.0) as usize;
                let (w, h) = (size.0 as f32, size.1 as f32);
                let bw = (w / across as f32).max(1.0);
                let bh = (h / down as f32).max(1.0);
                let blocks = [(w / bw).ceil() as u32, (h / bh).ceil() as u32];
                Kernel::Mosaic {
                    block: [bw, bh],
                    blocks: blocks.map(|count| count.max(1)),
                    soft: (fv(e, "softness", cx) / 100.0).clamp(0.0, 1.0),
                }
            }
            _ => return None,
        })
    }

    /// `effect` on a picture brought under 1 and put back, as a picture holding values over 1
    /// needs for an effect written for 0 to 1: `src` is the picture, and `effect` runs on it with
    /// its pixels each under 1.
    pub fn held(
        frame: &mut Frame<'static>,
        picture: Picture,
        effect: impl FnOnce(&mut Frame<'static>, Picture) -> Option<Picture>,
    ) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        let unit = stage.pass(frame, &stage.to_unit, &picture, None, &U::default());
        let done = effect(frame, unit)?;
        let out = stage.pass(
            frame,
            &stage.from_unit,
            &done,
            Some(&picture),
            &U::default(),
        );
        frame.release(done);
        frame.release(picture);
        Some(out)
    }

    /// `effect` on a picture brought inside BT.709 and put back: `Image::to_gamut` and its way back.
    pub fn in_gamut(
        frame: &mut Frame<'static>,
        picture: Picture,
        effect: impl FnOnce(&mut Frame<'static>, Picture) -> Option<Picture>,
    ) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        let inside = stage.pass(frame, &stage.to_gamut, &picture, None, &U::default());
        let done = effect(frame, inside)?;
        let out = stage.pass(
            frame,
            &stage.from_gamut,
            &done,
            Some(&picture),
            &U::default(),
        );
        frame.release(done);
        frame.release(picture);
        Some(out)
    }

    /// The kernel on `picture`, or `None` when the kernels did not build.
    pub fn run(&self, frame: &mut Frame<'static>, picture: Picture) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        Some(match self {
            Kernel::Nothing => picture,
            Kernel::WonderGlow {
                threshold,
                saturation,
                tint,
                radius,
                gain,
                screen,
            } => {
                let hi = stage.pass(
                    frame,
                    &stage.highlights,
                    &picture,
                    None,
                    &U::floats([*threshold, 0.15, 0.0, 0.0]),
                );
                let saturates = (saturation - 1.0).abs() >= 1e-4;
                let color = tint.unwrap_or([0.0; 3]);
                let graded = stage.pass(
                    frame,
                    &stage.grade_light,
                    &hi,
                    None,
                    &U {
                        a: [u32::from(saturates), u32::from(tint.is_some()), 0, 0],
                        p0: [*saturation, 1.0, 0.0, 0.0],
                        p1: [color[0], color[1], color[2], 0.0],
                        ..U::default()
                    },
                );
                frame.release(hi);
                let (w, h) = (picture.w, picture.h);
                let mut sum = frame.clear(w, h);
                for (factor, weight) in [(0.25f32, 0.45f32), (1.0, 0.35), (3.0, 0.2)] {
                    let sigma = radius * factor;
                    let (rx, ry) = gaussian_boxes(w as usize, h as usize, sigma, sigma);
                    let blur = FxOp::Gaussian {
                        rx,
                        ry,
                        repeat: false,
                    };
                    let copy = frame.copy(&graded);
                    let blurred = run_ops(frame, copy, std::slice::from_ref(&blur))?;
                    let next = stage.pass(
                        frame,
                        &stage.accumulate,
                        &sum,
                        Some(&blurred),
                        &U::floats([weight, 0.0, 0.0, 0.0]),
                    );
                    frame.release(blurred);
                    frame.release(sum);
                    sum = next;
                }
                frame.release(graded);
                let out = stage.add_light(frame, &picture, &sum, [*gain; 3], *screen);
                frame.release(sum);
                frame.release(picture);
                out
            }
            Kernel::Glint {
                threshold,
                rays,
                length,
                tint,
                colorize,
                gain,
            } => {
                let hi = stage.pass(
                    frame,
                    &stage.highlights,
                    &picture,
                    None,
                    &U::floats([*threshold, 0.08, 0.0, 0.0]),
                );
                let mut sum = frame.clear(picture.w, picture.h);
                for ray in rays {
                    let streaked = stage.streak(frame, &hi, (ray.cos(), ray.sin()), *length);
                    let next = stage.pass(
                        frame,
                        &stage.accumulate,
                        &sum,
                        Some(&streaked),
                        &U::floats([1.0, 0.0, 0.0, 0.0]),
                    );
                    frame.release(streaked);
                    frame.release(sum);
                    sum = next;
                }
                frame.release(hi);
                if *colorize > 0.0 {
                    let next = stage.pass(
                        frame,
                        &stage.grade_light,
                        &sum,
                        None,
                        &U {
                            a: [0, 1, 0, 0],
                            p0: [1.0, *colorize, 0.0, 0.0],
                            p1: [tint[0], tint[1], tint[2], 0.0],
                            ..U::default()
                        },
                    );
                    frame.release(sum);
                    sum = next;
                }
                let out = stage.add_light(frame, &picture, &sum, *gain, false);
                frame.release(sum);
                frame.release(picture);
                out
            }
            Kernel::RgbSplit {
                offset,
                radial,
                blend,
            } => {
                let (center, reach, amount) = radial.unwrap_or(([0.0; 2], 1.0, 0.0));
                let out = stage.pass(
                    frame,
                    &stage.rgb_split,
                    &picture,
                    None,
                    &U {
                        a: [u32::from(radial.is_some()), 0, 0, 0],
                        p0: [offset[0], offset[1], *blend, 0.0],
                        p1: [center[0], center[1], reach, amount],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::WaveWarp {
                across,
                height,
                width,
                phase,
            } => {
                let out = stage.pass(
                    frame,
                    &stage.wave_warp,
                    &picture,
                    None,
                    &U {
                        p0: [across[0], across[1], *height, *width],
                        p1: [*phase, 0.0, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::Noise {
                amount,
                grain,
                colored,
                clip,
                hashes,
            } => {
                let words = |hash: u64| [hash as u32, (hash >> 32) as u32];
                let [z0, z1, z2] = hashes.map(words);
                let out = stage.pass(
                    frame,
                    &stage.noise,
                    &picture,
                    None,
                    &U {
                        a: [u32::from(*colored), u32::from(*clip), z0[0], z0[1]],
                        b: [z1[0], z1[1], z2[0], z2[1]],
                        p0: [*amount, *grain, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::ColorKey {
                key,
                tolerance,
                softness,
            } => {
                let out = stage.pass(
                    frame,
                    &stage.color_key,
                    &picture,
                    None,
                    &U {
                        p0: [key[0], key[1], key[2], *tolerance],
                        p1: [*softness, 0.0, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::LumaKey { threshold, cutoff } => {
                let out = stage.pass(
                    frame,
                    &stage.luma_key,
                    &picture,
                    None,
                    &U::floats([*threshold, *cutoff, 0.0, 0.0]),
                );
                frame.release(picture);
                out
            }
            Kernel::LogoCutout {
                background,
                threshold,
                softness,
                unmultiply,
                invert,
            } => {
                let out = stage.pass(
                    frame,
                    &stage.logo_cutout,
                    &picture,
                    None,
                    &U {
                        a: [u32::from(*unmultiply), u32::from(*invert), 0, 0],
                        p0: [background[0], background[1], background[2], 0.0],
                        p1: [*threshold, *softness, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::UltraKey(key) => {
                let numbers = U {
                    a: [key.output, key.dominant, u32::from(key.corrects), 0],
                    p0: [key.key[0], key.key[1], key.key[2], key.key_size],
                    p1: [key.gain, key.highlight, key.shadow, key.tolerance],
                    p2: [key.pedestal, key.contrast, key.mid, key.spill],
                    p3: [key.desaturate, key.range, key.spill_luma, 0.0],
                    p4: [key.correction[0], key.correction[1], key.correction[2], 0.0],
                    ..U::default()
                };
                let out = if key.choke > 0.0 || key.soften > 0.0 {
                    let mut matte = stage.pass(frame, &stage.ultra_matte, &picture, None, &numbers);
                    if key.choke > 0.0 {
                        let copy = frame.copy(&matte);
                        let sigma = (key.choke * 4.0 * key.px_scale).max(0.5);
                        let blurred = blur_plane(frame, copy, sigma)?;
                        let choked = stage.pass(
                            frame,
                            &stage.ultra_choke,
                            &matte,
                            Some(&blurred),
                            &U::floats([key.choke * 0.5, 0.0, 0.0, 0.0]),
                        );
                        frame.release(blurred);
                        frame.release(matte);
                        matte = choked;
                    }
                    if key.soften > 0.0 {
                        matte = blur_plane(frame, matte, key.soften * 10.0 * key.px_scale)?;
                    }
                    let out =
                        stage.pass(frame, &stage.ultra_keyed, &picture, Some(&matte), &numbers);
                    frame.release(matte);
                    out
                } else {
                    stage.pass(frame, &stage.ultra_key, &picture, None, &numbers)
                };
                frame.release(picture);
                out
            }
            Kernel::RadialZoom {
                kind,
                center,
                zoom,
                arc,
                quality,
            } => {
                let encoded = stage.pass(frame, &stage.encode_kept, &picture, None, &U::default());
                frame.release(picture);
                let blurred = stage.pass(
                    frame,
                    &stage.radial_zoom,
                    &encoded,
                    None,
                    &U {
                        a: [*kind, 0, 0, 0],
                        p0: [center[0], center[1], *zoom, *arc],
                        p1: [*quality, 0.0, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(encoded);
                let out = stage.pass(frame, &stage.decode_kept, &blurred, None, &U::default());
                frame.release(blurred);
                out
            }
            Kernel::Lumetri(lumetri) => return lumetri.run(frame, picture),
            Kernel::Leaks {
                colors,
                intensity,
                scale,
                direction,
                time,
                seeds,
                mode,
            } => {
                let low = |value: u64| value as u32;
                let high = |value: u64| (value >> 32) as u32;
                let out = stage.pass(
                    frame,
                    &stage.light_leaks,
                    &picture,
                    None,
                    &U {
                        a: [*mode, 0, low(seeds[0]), high(seeds[0])],
                        b: [low(seeds[1]), high(seeds[1]), low(seeds[2]), high(seeds[2])],
                        p0: [colors[0][0], colors[0][1], colors[0][2], *intensity],
                        p1: [colors[1][0], colors[1][1], colors[1][2], *scale],
                        p2: [direction[0], direction[1], *time, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::Grain(grain) => {
                let encoded = stage.pass(frame, &stage.encode_kept, &picture, None, &U::default());
                frame.release(picture);
                let grained = stage.pass(
                    frame,
                    &stage.add_grain,
                    &encoded,
                    None,
                    &U {
                        a: [grain.mode, u32::from(grain.mono), 0, 0],
                        b: [grain.seeds[0], grain.seeds[1], grain.seeds[2], 0],
                        p0: [grain.gains[0], grain.gains[1], grain.gains[2], grain.aspect],
                        p1: [grain.sizes[0], grain.sizes[1], grain.sizes[2], grain.frame],
                        p2: [
                            grain.tones[0],
                            grain.tones[1],
                            grain.tones[2],
                            grain.midpoint,
                        ],
                        p3: [grain.saturation, 0.0, 0.0, 0.0],
                        ..U::default()
                    },
                );
                frame.release(encoded);
                let out = stage.pass(frame, &stage.decode_kept, &grained, None, &U::default());
                frame.release(grained);
                out
            }
            Kernel::CornerPin { map, size } => {
                let out = stage.pass(
                    frame,
                    &stage.corner_pin,
                    &picture,
                    None,
                    &U {
                        p0: [map[0], map[1], map[2], map[3]],
                        p1: [map[4], map[5], map[6], map[7]],
                        p2: [map[8], size[0], size[1], 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::DropShadow {
                offset,
                color,
                opacity,
                blur,
                only,
            } => {
                let (w, h) = (picture.w, picture.h);
                let whole = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
                let by = Affine::translate(offset[0], offset[1]);
                let empty = frame.clear(w, h);
                let moved = frame.composite(empty, &picture, &by, whole, 1.0, Blend::Normal);
                let mut cast = stage.pass(
                    frame,
                    &stage.shadow,
                    &moved,
                    None,
                    &U::floats([color[0], color[1], color[2], *opacity]),
                );
                frame.release(moved);
                if let Some(blur) = blur {
                    cast = run_ops(frame, cast, std::slice::from_ref(blur))?;
                }
                if !only {
                    let over = &Affine::IDENTITY;
                    cast = frame.composite(cast, &picture, over, whole, 1.0, Blend::Normal);
                }
                frame.release(picture);
                cast
            }
            Kernel::RoundedCrop(told) => {
                let [r, g, b] = told.border_color;
                let out = stage.pass(
                    frame,
                    &stage.rounded_crop,
                    &picture,
                    None,
                    &U {
                        p0: told.edges,
                        p1: [told.radius, told.feather, told.border, 0.0],
                        p2: [r, g, b, 0.0],
                        ..U::default()
                    },
                );
                frame.release(picture);
                out
            }
            Kernel::Spacer { place, told } => {
                let (w, h) = (picture.w, picture.h);
                let whole = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
                let [x0, y0, s] = *place;
                let to = Affine::translate(x0, y0).then_apply(&Affine::scale(s, s));
                let empty = frame.clear(w, h);
                let placed = frame.composite(empty, &picture, &to, whole, 1.0, Blend::Normal);
                frame.release(picture);
                let [r, g, b] = told.color;
                let out = stage.pass(
                    frame,
                    &stage.spacer,
                    &placed,
                    None,
                    &U {
                        p0: told.edges,
                        p1: [told.radius, told.fill, 0.0, 0.0],
                        p2: [r, g, b, 0.0],
                        ..U::default()
                    },
                );
                frame.release(placed);
                out
            }
            Kernel::Mosaic {
                block,
                blocks,
                soft,
            } => {
                let numbers = U::floats([block[0], block[1], *soft, 0.0]);
                // A picture with a pixel to each block, which every pixel of the block then reads.
                let small = frame.take(blocks[0], blocks[1]);
                frame.dispatch(
                    &stage.mosaic_blocks,
                    &stage.layout,
                    &numbers.bytes(),
                    (&picture.view, &small.view, None),
                    (blocks[0].div_ceil(16), blocks[1].div_ceil(16)),
                );
                let out = stage.pass(frame, &stage.mosaic, &picture, Some(&small), &numbers);
                frame.release(small);
                frame.release(picture);
                out
            }
        })
    }
}

/// `blur_plane` of `vfx`, with its plane in the red of a picture.
fn blur_plane(frame: &mut Frame<'static>, plane: Picture, sigma: f32) -> Option<Picture> {
    if sigma <= 0.3 {
        return Some(plane);
    }
    let (rx, ry) = gaussian_boxes(plane.w as usize, plane.h as usize, sigma, sigma);
    run_ops(
        frame,
        plane,
        &[FxOp::Gaussian {
            rx,
            ry,
            repeat: false,
        }],
    )
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

/// `U` in `kernels.wgsl`.
#[derive(Default)]
struct U {
    a: [u32; 4],
    b: [u32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    p3: [f32; 4],
    p4: [f32; 4],
}

impl U {
    fn floats(p0: [f32; 4]) -> Self {
        Self {
            p0,
            ..Self::default()
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(112);
        for value in self.a.iter().chain(&self.b) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let floats = [&self.p0, &self.p1, &self.p2, &self.p3, &self.p4];
        for value in floats.into_iter().flatten() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

/// The kernels' WGSL: what is written by hand, and on the end of it what `kerf_kernel` writes out.
fn source() -> String {
    format!(
        "{}{}",
        include_str!("kernels.wgsl"),
        kerf_kernel::rounded_wgsl()
    )
}

struct Stage {
    layout: wgpu::BindGroupLayout,
    highlights: wgpu::ComputePipeline,
    grade_light: wgpu::ComputePipeline,
    accumulate: wgpu::ComputePipeline,
    add_light: wgpu::ComputePipeline,
    streak: wgpu::ComputePipeline,
    scale_all: wgpu::ComputePipeline,
    rgb_split: wgpu::ComputePipeline,
    wave_warp: wgpu::ComputePipeline,
    noise: wgpu::ComputePipeline,
    color_key: wgpu::ComputePipeline,
    luma_key: wgpu::ComputePipeline,
    logo_cutout: wgpu::ComputePipeline,
    ultra_key: wgpu::ComputePipeline,
    ultra_matte: wgpu::ComputePipeline,
    ultra_choke: wgpu::ComputePipeline,
    ultra_keyed: wgpu::ComputePipeline,
    lumetri_input: wgpu::ComputePipeline,
    lumetri_more: wgpu::ComputePipeline,
    hsl_key: wgpu::ComputePipeline,
    hsl_apply: wgpu::ComputePipeline,
    light_leaks: wgpu::ComputePipeline,
    add_grain: wgpu::ComputePipeline,
    encode_kept: wgpu::ComputePipeline,
    decode_kept: wgpu::ComputePipeline,
    radial_zoom: wgpu::ComputePipeline,
    to_unit: wgpu::ComputePipeline,
    from_unit: wgpu::ComputePipeline,
    to_gamut: wgpu::ComputePipeline,
    from_gamut: wgpu::ComputePipeline,
    corner_pin: wgpu::ComputePipeline,
    shadow: wgpu::ComputePipeline,
    rounded_crop: wgpu::ComputePipeline,
    spacer: wgpu::ComputePipeline,
    mosaic_blocks: wgpu::ComputePipeline,
    mosaic: wgpu::ComputePipeline,
}

impl Stage {
    fn new(gpu: &Gpu) -> Option<Self> {
        let built = gpu.build(|device| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kerf-vfx"),
                source: wgpu::ShaderSource::Wgsl(source().into()),
            });
            let layout = kerf_gpu::kernel_layout(device);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kerf-vfx"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let kernel = |entry: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            Self {
                highlights: kernel("highlights"),
                grade_light: kernel("grade_light"),
                accumulate: kernel("accumulate"),
                add_light: kernel("add_light"),
                streak: kernel("streak"),
                scale_all: kernel("scale_all"),
                rgb_split: kernel("rgb_split"),
                wave_warp: kernel("wave_warp"),
                noise: kernel("noise"),
                color_key: kernel("color_key"),
                luma_key: kernel("luma_key"),
                logo_cutout: kernel("logo_cutout"),
                ultra_key: kernel("ultra_key"),
                ultra_matte: kernel("ultra_matte"),
                ultra_choke: kernel("ultra_choke"),
                ultra_keyed: kernel("ultra_keyed"),
                lumetri_input: kernel("lumetri_input"),
                lumetri_more: kernel("lumetri_more"),
                hsl_key: kernel("hsl_key"),
                hsl_apply: kernel("hsl_apply"),
                light_leaks: kernel("light_leaks"),
                add_grain: kernel("add_grain"),
                encode_kept: kernel("encode_kept"),
                decode_kept: kernel("decode_kept"),
                radial_zoom: kernel("radial_zoom"),
                to_unit: kernel("to_unit"),
                from_unit: kernel("from_unit"),
                to_gamut: kernel("to_gamut"),
                from_gamut: kernel("from_gamut"),
                corner_pin: kernel("corner_pin"),
                shadow: kernel("shadow"),
                rounded_crop: kernel("rounded_crop"),
                spacer: kernel("spacer"),
                mosaic_blocks: kernel("mosaic_blocks"),
                mosaic: kernel("mosaic"),
                layout: layout.clone(),
            }
        });
        built
            .map_err(|error| eprintln!("kerf-fx: look kernels did not build: {error}"))
            .ok()
    }

    fn pass(
        &self,
        frame: &mut Frame<'static>,
        pipeline: &wgpu::ComputePipeline,
        src: &Picture,
        aux: Option<&Picture>,
        uniforms: &U,
    ) -> Picture {
        let out = frame.take(src.w, src.h);
        frame.dispatch(
            pipeline,
            &self.layout,
            &uniforms.bytes(),
            (&src.view, &out.view, aux.map(|aux| &aux.view)),
            (src.w.div_ceil(16), src.h.div_ceil(16)),
        );
        out
    }

    fn add_light(
        &self,
        frame: &mut Frame<'static>,
        picture: &Picture,
        light: &Picture,
        gain: [f32; 3],
        screen: bool,
    ) -> Picture {
        self.pass(
            frame,
            &self.add_light,
            picture,
            Some(light),
            &U {
                a: [u32::from(screen), 0, 0, 0],
                p0: [gain[0], gain[1], gain[2], 0.0],
                ..U::default()
            },
        )
    }

    /// `lights::streak`: `src` smeared along `direction` by doubling.
    fn streak(
        &self,
        frame: &mut Frame<'static>,
        src: &Picture,
        direction: (f32, f32),
        length: f32,
    ) -> Picture {
        let falloff = (-1.0 / length.max(1.0)).exp();
        let mut current = frame.copy(src);
        let mut n = 1.0f32;
        while n < length * 3.0 {
            let next = self.pass(
                frame,
                &self.streak,
                &current,
                None,
                &U::floats([direction.0 * n, direction.1 * n, falloff.powf(n), 0.0]),
            );
            frame.release(current);
            current = next;
            n *= 2.0;
        }
        let out = self.pass(
            frame,
            &self.scale_all,
            &current,
            None,
            &U::floats([1.0 - falloff, 0.0, 0.0, 0.0]),
        );
        frame.release(current);
        out
    }
}

fn stage(gpu: &'static Gpu) -> Option<&'static Stage> {
    static STAGE: OnceLock<Option<Stage>> = OnceLock::new();
    STAGE.get_or_init(|| Stage::new(gpu)).as_ref()
}

/// Whether `gpu` can run these kernels.
pub fn available(gpu: &'static Gpu) -> bool {
    stage(gpu).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filmcraft::effects::apply;
    use crate::filmcraft::project::find_effect;

    /// A picture with bright spots, every kind of alpha, and hues.
    fn picture(w: usize, h: usize) -> Image {
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let bright = (x / 9 + y / 7) % 5 == 0;
                let a = match y % 11 {
                    0 => 0.0,
                    1 | 2 => 0.6,
                    _ => 1.0,
                };
                let c = if bright {
                    [1.0, 0.95, 0.8]
                } else {
                    [u, v, 1.0 - u]
                };
                px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
            }
        }
        Image { w, h, px }
    }

    fn context(seconds: f64) -> FxCtx<'static> {
        FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 24.0,
            working: kerf_color::WorkingSpace::Rec709,
        }
    }

    fn effect(id: &str, params: &[(&str, ParamValue)]) -> EffectInstance {
        let mut e = find_effect(id).unwrap_or_else(|| panic!("{id}")).instance();
        for (key, value) in params {
            e.params
                .get_mut(*key)
                .unwrap_or_else(|| panic!("{id}.{key}"))
                .value = value.clone();
        }
        e
    }

    fn on_gpu(gpu: &'static Gpu, image: &Image, kernel: &Kernel) -> Vec<f32> {
        let mut frame = gpu.frame();
        let picture = frame.image(image);
        let out = kernel.run(&mut frame, picture).expect("the stage builds");
        frame.read_floats(&out).expect("read back")
    }

    #[test]
    fn kernels_match_the_effects() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let fl = ParamValue::Float;
        let on = ParamValue::Bool(true);
        let choice = ParamValue::Choice;
        let pt = |x: f64, y: f64| ParamValue::Vec2(Vec2::new(x, y));
        // A corner pin's four, clockwise from the upper left, in pixels.
        let corners = |at: [(f64, f64); 4]| {
            let names = ["upper_left", "upper_right", "lower_right", "lower_left"];
            names
                .into_iter()
                .zip(at)
                .map(|(name, (x, y))| (name, pt(x, y)))
                .collect::<Vec<_>>()
        };
        let cases: Vec<(&str, Vec<(&str, ParamValue)>)> = vec![
            ("wonder_glow", vec![]),
            (
                "wonder_glow",
                vec![
                    ("threshold", fl(40.0)),
                    ("radius", fl(30.0)),
                    ("intensity", fl(120.0)),
                ],
            ),
            (
                "wonder_glow",
                vec![("mode", choice(1)), ("saturation", fl(160.0))],
            ),
            (
                "wonder_glow",
                vec![("use_color", on.clone()), ("radius", fl(6.0))],
            ),
            ("glint", vec![]),
            (
                "glint",
                vec![
                    ("rays", choice(3)),
                    ("length", fl(60.0)),
                    ("rotation", fl(30.0)),
                ],
            ),
            ("glint", vec![("rays", choice(0)), ("colorize", fl(70.0))]),
            ("rgb_split", vec![("amount", fl(7.5)), ("angle", fl(30.0))]),
            ("rgb_split", vec![("amount", fl(12.0)), ("mode", choice(1))]),
            ("rgb_split", vec![("amount", fl(9.0)), ("blend", fl(40.0))]),
            ("rgb_split", vec![("amount", fl(0.0))]),
            ("wave_warp", vec![("height", fl(10.0)), ("width", fl(40.0))]),
            (
                "wave_warp",
                vec![
                    ("height", fl(6.0)),
                    ("width", fl(20.0)),
                    ("direction", fl(35.0)),
                    ("speed", fl(2.0)),
                ],
            ),
            ("noise", vec![("amount", fl(30.0))]),
            (
                "noise",
                vec![
                    ("amount", fl(40.0)),
                    ("color", on.clone()),
                    ("grain_size", fl(3.0)),
                ],
            ),
            ("noise", vec![("amount", fl(20.0)), ("clip", on.clone())]),
            ("noise", vec![("amount", fl(0.0))]),
            ("color_key", vec![]),
            (
                "color_key",
                vec![
                    ("color", ParamValue::Color([0.1, 0.8, 0.2, 1.0])),
                    ("tolerance", fl(90.0)),
                    ("feather", fl(20.0)),
                ],
            ),
            ("luma_key", vec![]),
            (
                "luma_key",
                vec![("threshold", fl(50.0)), ("cutoff", fl(47.0))],
            ),
            (
                "luma_key",
                vec![("threshold", fl(60.0)), ("cutoff", fl(20.0))],
            ),
            ("logo_cutout", vec![]),
            (
                "logo_cutout",
                vec![("background", choice(1)), ("threshold", fl(15.0))],
            ),
            (
                "logo_cutout",
                vec![
                    ("background", choice(2)),
                    ("color", ParamValue::Color([0.2, 0.3, 0.9, 1.0])),
                    ("invert", on.clone()),
                ],
            ),
            ("ultra_key", vec![]),
            (
                "ultra_key",
                vec![
                    ("key_color", ParamValue::Color([0.1, 0.2, 0.9, 1.0])),
                    ("setting", choice(3)),
                    ("transparency", fl(70.0)),
                    ("contrast", fl(30.0)),
                    ("spill", fl(80.0)),
                ],
            ),
            (
                "ultra_key",
                vec![
                    ("key_color", ParamValue::Color([0.3, 0.45, 1.0, 1.0])),
                    ("setting", choice(3)),
                    ("shadow", fl(100.0)),
                    ("highlight", fl(60.0)),
                    ("tolerance", fl(90.0)),
                ],
            ),
            ("ultra_key", vec![("output", choice(1))]),
            ("ultra_key", vec![("output", choice(2))]),
            (
                "ultra_key",
                vec![
                    ("cc_saturation", fl(150.0)),
                    ("cc_hue", fl(40.0)),
                    ("cc_luminance", fl(80.0)),
                ],
            ),
            // Its presets set these, so only as they are set by hand.
            (
                "ultra_key",
                vec![("setting", choice(3)), ("choke", fl(20.0))],
            ),
            (
                "ultra_key",
                vec![("setting", choice(3)), ("soften", fl(30.0))],
            ),
            (
                "ultra_key",
                vec![
                    ("setting", choice(3)),
                    ("choke", fl(60.0)),
                    ("soften", fl(10.0)),
                    ("contrast", fl(30.0)),
                    ("output", choice(1)),
                ],
            ),
            ("light_leaks", vec![]),
            (
                "light_leaks",
                vec![
                    ("mode", choice(1)),
                    ("seed", fl(7.0)),
                    ("direction", fl(70.0)),
                ],
            ),
            (
                "light_leaks",
                vec![("mode", choice(2)), ("scale", fl(30.0)), ("speed", fl(3.0))],
            ),
            // Corners left where they are.
            ("corner_pin", vec![]),
            // Seen from the side, as a tracked surface is.
            ("corner_pin", corners([(10.0, 6.0), (80.0, 2.0), (90.0, 50.0), (4.0, 44.0)])),
            // Small in the middle, where most of the picture is left clear.
            ("corner_pin", corners([(30.0, 15.0), (60.0, 18.0), (58.0, 40.0), (28.0, 36.0)])),
            // Past every edge.
            (
                "corner_pin",
                corners([(-20.0, -10.0), (110.0, -5.0), (120.0, 60.0), (-10.0, 70.0)]),
            ),
            // Turned over, left for right.
            ("corner_pin", corners([(90.0, 5.0), (5.0, 5.0), (5.0, 50.0), (90.0, 50.0)])),
            // One corner alone, the others where the catalog starts them.
            ("corner_pin", vec![("lower_right", pt(70.0, 30.0))]),
        ];
        let cases: Vec<_> = cases.into_iter().chain(shaped_cases()).collect();
        let image = picture(96, 54);
        let mut failures = Vec::new();
        for (id, params) in cases {
            let e = effect(id, &params);
            let cx = context(1.3);
            let kernel = Kernel::eval(&e, &cx, (image.w, image.h)).expect("a kernel");
            let mut cpu = image.clone();
            apply(&mut cpu, &e, &cx);
            let gpu_px = on_gpu(gpu, &image, &kernel);
            let worst = cpu
                .px
                .iter()
                .zip(&gpu_px)
                .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
                .fold(0.0f32, f32::max);
            if worst > 2e-3 {
                failures.push(format!("{id} {params:?}: differs by {worst:.2e}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Drop Shadow, Rounded Crop, Spacer, and Mosaic, each way they can be set.
    fn shaped_cases() -> Vec<(&'static str, Vec<(&'static str, ParamValue)>)> {
        let fl = ParamValue::Float;
        let on = ParamValue::Bool(true);
        let off = ParamValue::Bool(false);
        let color = |r: f32, g: f32, b: f32| ParamValue::Color([r, g, b, 1.0]);
        vec![
            ("drop_shadow", vec![]),
            (
                "drop_shadow",
                vec![
                    ("distance", fl(12.0)),
                    ("softness", fl(8.0)),
                    ("opacity", fl(80.0)),
                ],
            ),
            (
                "drop_shadow",
                vec![
                    ("distance", fl(7.5)),
                    ("direction", fl(250.0)),
                    ("color", color(0.9, 0.2, 0.1)),
                    ("only", on.clone()),
                ],
            ),
            // Its roundness of 40 is more than half of this picture, so its ends are round.
            ("rounded_crop", vec![]),
            (
                "rounded_crop",
                vec![
                    ("left", fl(10.0)),
                    ("top", fl(20.0)),
                    ("right", fl(5.0)),
                    ("bottom", fl(8.0)),
                    ("radius", fl(9.0)),
                    ("feather", fl(4.0)),
                ],
            ),
            (
                "rounded_crop",
                vec![
                    ("radius", fl(12.0)),
                    ("border", fl(3.0)),
                    ("border_color", color(0.2, 0.6, 1.0)),
                ],
            ),
            (
                "rounded_crop",
                vec![
                    ("radius", fl(0.0)),
                    ("left", fl(30.0)),
                    ("border", fl(1.5)),
                    ("feather", fl(2.0)),
                ],
            ),
            // Its 40 pixels a side leave this picture a fiftieth of its size, halved on the way.
            ("spacer", vec![]),
            ("spacer", vec![("left", fl(6.0))]),
            (
                "spacer",
                vec![
                    ("left", fl(8.0)),
                    ("radius", fl(6.0)),
                    ("fill", on.clone()),
                    ("color", color(0.1, 0.3, 0.2)),
                ],
            ),
            (
                "spacer",
                vec![
                    ("uniform", off),
                    ("left", fl(20.0)),
                    ("top", fl(2.0)),
                    ("right", fl(4.0)),
                    ("bottom", fl(10.0)),
                    ("radius", fl(3.0)),
                ],
            ),
            ("spacer", vec![("left", fl(0.0)), ("fill", on.clone())]),
            ("mosaic", vec![]),
            (
                "mosaic",
                vec![
                    ("horizontal", fl(7.0)),
                    ("vertical", fl(4.0)),
                    ("softness", fl(60.0)),
                ],
            ),
            // More blocks across than there are pixels.
            (
                "mosaic",
                vec![("horizontal", fl(200.0)), ("vertical", fl(3.0))],
            ),
            // Blocks of a whole number of pixels, and all the way soft.
            (
                "mosaic",
                vec![
                    ("horizontal", fl(16.0)),
                    ("vertical", fl(9.0)),
                    ("softness", fl(100.0)),
                ],
            ),
        ]
    }

    /// What a kernel written once in `kerf_kernel` draws on the CPU, as `f32`, with the passes around it
    /// that the CPU's own code makes; none for a kernel written by hand.
    fn from_one_source(kernel: &Kernel, image: &Image) -> Option<Image> {
        let each = |from: &Image, pixel: &dyn Fn([f32; 4], f32, f32) -> [f32; 4]| {
            let mut out = from.clone();
            for (at, p) in out.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (at % from.w, at / from.w);
                *p = pixel(*p, x as f32 + 0.5, y as f32 + 0.5);
            }
            out
        };
        let (w, h) = (image.w, image.h);
        Some(match kernel {
            Kernel::RoundedCrop(told) => {
                each(image, &|p, x, y| kerf_kernel::rounded_crop(p, x, y, told))
            }
            Kernel::Spacer { place, told } => {
                let [x0, y0, s] = *place;
                let to = Affine::translate(x0, y0).then_apply(&Affine::scale(s, s));
                let placed = image.transformed(w, h, &to);
                each(&placed, &|p, x, y| kerf_kernel::spacer(p, x, y, told))
            }
            Kernel::DropShadow {
                offset,
                color,
                opacity,
                blur,
                only,
            } => {
                let moved = image.transformed(w, h, &Affine::translate(offset[0], offset[1]));
                let mut cast = each(&moved, &|p, _, _| kerf_kernel::shadow(p, *color, *opacity));
                if let Some(blur) = blur {
                    blur.apply(&mut cast);
                }
                if !only {
                    kerf_draw::blend::composite(&mut cast, image, 1.0, Blend::Normal);
                }
                cast
            }
            _ => return None,
        })
    }

    /// The kernels written once in `kerf_kernel` come, as `f32`, to the very values the CPU's code
    /// comes to, which needs no GPU to know: what the GPU runs is that source written out.
    #[test]
    fn kernels_from_one_source_are_the_effects_to_the_bit() {
        let image = picture(96, 54);
        let cx = context(0.0);
        let mut held = 0;
        for (id, params) in shaped_cases() {
            let e = effect(id, &params);
            let kernel = Kernel::eval(&e, &cx, (image.w, image.h)).expect("a kernel");
            let Some(twin) = from_one_source(&kernel, &image) else {
                assert_eq!(id, "mosaic", "{id} is written by hand");
                continue;
            };
            let mut cpu = image.clone();
            apply(&mut cpu, &e, &cx);
            // (equal rather than the same bits: a zero may come out with another sign)
            assert!(twin.px == cpu.px, "{id} {params:?}");
            assert_ne!(cpu.px, image.px, "{id} {params:?} changes nothing");
            held += 1;
        }
        assert_eq!(held, 12);
    }

    /// A Lumetri's curves, LUTs, HSL Secondary, and sharpening, each and together, and with what comes
    /// before them in the effect, where the order shows.
    #[test]
    fn lumetri_past_its_plain_numbers_matches_the_effect() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let fl = ParamValue::Float;
        let on = ParamValue::Bool(true);
        let curve = |points: &[[f32; 2]]| ParamValue::Curve(points.to_vec());
        let bend = curve(&[[0.0, 0.0], [0.3, 0.2], [0.7, 0.85], [1.0, 1.0]]);
        let around = curve(&[[0.0, 0.5], [0.33, 0.85], [0.66, 0.25]]);
        let lut = ParamValue::Text("builtin:look-bleach-bypass".into());
        let log = ParamValue::Text("builtin:slog3-sgamut3cine-to-rec709".into());
        let cases: Vec<(&str, Vec<(&str, ParamValue)>)> = vec![
            ("sharpen", vec![("sharpen", fl(50.0))]),
            ("soften by sharpening", vec![("sharpen", fl(-40.0))]),
            ("luma curve", vec![("curve_luma", bend.clone())]),
            ("red curve", vec![("curve_red", bend.clone())]),
            (
                "green and blue curves",
                vec![("curve_green", bend.clone()), ("curve_blue", bend.clone())],
            ),
            ("hue vs sat", vec![("hue_vs_sat", around.clone())]),
            ("hue vs hue", vec![("hue_vs_hue", around.clone())]),
            ("hue vs luma", vec![("hue_vs_luma", around.clone())]),
            ("luma vs sat", vec![("luma_vs_sat", around.clone())]),
            ("sat vs sat", vec![("sat_vs_sat", around.clone())]),
            ("look LUT", vec![("look_lut", lut.clone())]),
            (
                "half a look LUT, then wheels",
                vec![
                    ("look_lut", lut.clone()),
                    ("look_intensity", fl(50.0)),
                    ("wheel_shadows_l", fl(30.0)),
                ],
            ),
            ("input LUT", vec![("input_lut", log.clone())]),
            (
                "named look and curve",
                vec![
                    ("look", ParamValue::Choice(6)),
                    ("curve_luma", bend.clone()),
                ],
            ),
            ("HSL", vec![("hsl_on", on.clone()), ("hsl_hue", fl(200.0))]),
            (
                "HSL corrected",
                vec![
                    ("hsl_on", on.clone()),
                    ("hsl_hue", fl(120.0)),
                    ("hsl_hue_range", fl(90.0)),
                    ("hsl_sat", fl(150.0)),
                    ("hsl_hue_shift", fl(30.0)),
                    ("hsl_temp", fl(40.0)),
                    ("hsl_tint", fl(-30.0)),
                ],
            ),
            (
                "HSL blurred",
                vec![
                    ("hsl_on", on.clone()),
                    ("hsl_hue_range", fl(120.0)),
                    ("hsl_sat", fl(40.0)),
                    ("hsl_blur", fl(20.0)),
                ],
            ),
            (
                "HSL mask, gray",
                vec![
                    ("hsl_on", on.clone()),
                    ("hsl_show_mask", ParamValue::Choice(1)),
                ],
            ),
            (
                "HSL mask, black",
                vec![
                    ("hsl_on", on.clone()),
                    ("hsl_show_mask", ParamValue::Choice(2)),
                ],
            ),
            (
                "HSL mask, white",
                vec![
                    ("hsl_on", on.clone()),
                    ("hsl_show_mask", ParamValue::Choice(3)),
                ],
            ),
            (
                "everything at once",
                vec![
                    ("exposure", fl(0.4)),
                    ("input_lut", log.clone()),
                    ("look_lut", lut.clone()),
                    ("wheel_shadows_l", fl(20.0)),
                    ("curve_luma", bend.clone()),
                    ("hue_vs_sat", around.clone()),
                    ("hsl_on", on.clone()),
                    ("hsl_sat", fl(60.0)),
                    ("sharpen", fl(30.0)),
                    ("vignette_amount", fl(-2.0)),
                ],
            ),
        ];
        let image = picture(96, 54);
        let mut failures = Vec::new();
        for (name, params) in cases {
            let e = effect("lumetri", &params);
            let cx = context(0.0);
            let kernel = Kernel::eval(&e, &cx, (image.w, image.h))
                .unwrap_or_else(|| panic!("{name} has no kernel"));
            let mut cpu = image.clone();
            apply(&mut cpu, &e, &cx);
            assert_ne!(cpu.px, image.px, "{name} changes nothing");
            let gpu_px = on_gpu(gpu, &image, &kernel);
            let worst = cpu
                .px
                .iter()
                .zip(&gpu_px)
                .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
                .fold(0.0f32, f32::max);
            if worst > 2e-3 {
                failures.push(format!("{name}: differs by {worst:.2e}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert!(kerf_gpu::shared().is_some(), "the device failed");
    }

    #[test]
    fn what_a_lumetri_leaves_to_the_cpu() {
        let cx = context(0.0);
        let text = |value: &str| ParamValue::Text(value.into());
        // Its plain numbers go as an op.
        assert_eq!(Kernel::eval(&effect("lumetri", &[]), &cx, (8, 8)), None);
        // A median a pixel costs the GPU what it costs the CPU.
        let denoised = effect(
            "lumetri",
            &[
                ("hsl_on", ParamValue::Bool(true)),
                ("hsl_denoise", ParamValue::Float(50.0)),
            ],
        );
        assert_eq!(Kernel::eval(&denoised, &cx, (8, 8)), None);
        // A LUT that cannot be read does nothing on the CPU, and comes out of the plain path too.
        let missing = effect("lumetri", &[("look_lut", text("lib:nowhere"))]);
        assert_eq!(Kernel::eval(&missing, &cx, (8, 8)), None);
    }

    #[test]
    fn holding_values_over_1_puts_them_back() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let mut image = picture(96, 54);
        for pixel in image.px.as_chunks_mut::<4>().0.iter_mut().step_by(3) {
            for value in &mut pixel[..3] {
                *value *= 8.0;
            }
        }
        // And a color past BT.709 at every fifth.
        for pixel in image.px.as_chunks_mut::<4>().0.iter_mut().step_by(5) {
            pixel[0] = -0.3 * pixel[3];
        }
        // What the compositor's CPU path does, with Gamma as what runs in between.
        let gamma = FxOp::Gamma { g: 1.4 };
        let mut cpu = image.clone();
        let gains = cpu.to_unit();
        gamma.apply(&mut cpu);
        cpu.from_unit(&gains);
        assert!(cpu.peak() > 1.5, "the test needs values over 1");
        let mut frame = gpu.frame();
        let start = frame.image(&image);
        let out = Kernel::held(&mut frame, start, |frame, unit| {
            run_ops(frame, unit, std::slice::from_ref(&gamma))
        })
        .expect("the stage builds");
        let gpu_px = frame.read_floats(&out).expect("read back");
        let worst = cpu
            .px
            .iter()
            .zip(&gpu_px)
            .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
            .fold(0.0f32, f32::max);
        assert!(worst < 2e-3, "differs by {worst:.2e}");
    }

    /// That the kernels are WGSL, what is written by hand and what is written out together, with no
    /// GPU to say so: kernels that do not build take every frame off the GPU, and a machine with none
    /// would not know.
    #[test]
    fn the_kernels_are_wgsl() {
        let source = source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{error:?}"));
    }

    #[test]
    fn nothing_changes_nothing() {
        let e = effect("noise", &[("amount", ParamValue::Float(0.0))]);
        assert_eq!(
            Kernel::eval(&e, &context(0.0), (8, 8)),
            Some(Kernel::Nothing)
        );
    }

    /// What a corner pin leaves alone on the CPU it leaves alone here, what it moves has a kernel, and
    /// what it cannot say in numbers stays with the CPU.
    #[test]
    fn a_corner_pin_is_nothing_a_kernel_or_the_cpus() {
        let pt = |x: f64, y: f64| ParamValue::Vec2(Vec2::new(x, y));
        let cx = context(0.0);
        let eval = |params: &[(&str, ParamValue)]| {
            let e = effect("corner_pin", params);
            let kernel = Kernel::eval(&e, &cx, (96, 54));
            let mut cpu = picture(96, 54);
            apply(&mut cpu, &e, &cx);
            (kernel, cpu.px != picture(96, 54).px)
        };
        assert_eq!(eval(&[]), (Some(Kernel::Nothing), false));
        // All four on one point: a map with no way back, which the CPU leaves the picture by.
        let on = |x: f64, y: f64| {
            ["upper_left", "upper_right", "lower_right", "lower_left"].map(|name| (name, pt(x, y)))
        };
        assert_eq!(eval(&on(30.0, 20.0)), (Some(Kernel::Nothing), false));
        // In a line: the map divides by nothing, and the CPU draws what comes of that.
        let flat = [
            ("upper_left", pt(10.0, 10.0)),
            ("upper_right", pt(40.0, 10.0)),
            ("lower_right", pt(70.0, 10.0)),
            ("lower_left", pt(90.0, 10.0)),
        ];
        assert_eq!(eval(&flat), (None, true));
        let (kernel, changed) = eval(&[("upper_left", pt(12.0, 9.0))]);
        assert!(matches!(kernel, Some(Kernel::CornerPin { .. })) && changed);
    }

    #[test]
    fn other_effects_have_no_kernel_here() {
        let e = effect("tint", &[]);
        assert_eq!(Kernel::eval(&e, &context(0.0), (8, 8)), None);
    }
}
