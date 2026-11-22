//! [`Lumetri`]: what a Lumetri does past its plain numbers (its input LUT, curves, look LUT and the
//! wheels after it, HSL Secondary, and sharpening), read from the effect and drawn in passes around
//! [`FxOp::Lumetri`].

use super::stage::{stage, U};
use super::*;
use crate::video::effects::{
    boxes_for_gauss, choice, curve_lut, curve_param, f, gaussian_boxes, grade_space, hue_lut,
    is_identity_curve, on, text, Wheels,
};
use crate::video::gpu::run as run_ops;
use kerf_color::Lut;
use kerf_gpu::{Frame, Picture};
use std::sync::Arc;

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

const CURVE_POINTS: usize = 1024;

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
    pub(super) fn eval(e: &EffectInstance, cx: &FxCtx, (w, h): (usize, usize)) -> Option<Self> {
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
            let lut = crate::video::luts::resolve(cx.project, text(e, id));
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
        let hsl = if crate::video::effects::b(e, "hsl_on") {
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

    pub(super) fn run(&self, frame: &mut Frame<'static>, picture: Picture) -> Option<Picture> {
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
