//! The GPU's side of [`FxOp`]: each op as compute passes in `fx.wgsl`, with the arithmetic of
//! [`FxOp::apply`], the reference. Carried from FilmCraft's GPU compositor (`fx.rs`).
//!
//! Box blurs run per pixel for radii up to [`BOX_PER_PIXEL_MAX`] and as a running sum per row or
//! column above, so a huge radius costs the same per pixel as a small one.

use super::gpufx::FxOp;
use kerf_gpu::{wgpu, Frame, Gpu, Picture};
use std::sync::OnceLock;

/// Box radii up to this many pixels are summed per output pixel; larger ones as a running sum,
/// which measured faster from here up (FilmCraft kept this at 32, which made a blur no faster than
/// the CPU's).
pub const BOX_PER_PIXEL_MAX: u32 = 3;

const OP_BRIGHTNESS_CONTRAST: u32 = 1;
const OP_PROC_AMP: u32 = 2;
const OP_TINT: u32 = 3;
const OP_BLACK_WHITE: u32 = 4;
const OP_COLOR_BALANCE: u32 = 5;
const OP_LEAVE_COLOR: u32 = 6;
const OP_CHANGE_TO_COLOR: u32 = 7;
const OP_COLOR_PASS: u32 = 8;
const OP_GAMMA: u32 = 9;
const OP_LEVELS: u32 = 10;
const OP_EXTRACT: u32 = 11;
const OP_INVERT: u32 = 12;
const OP_INVERT_ALPHA: u32 = 13;
const OP_POSTERIZE: u32 = 14;
const OP_ASC_CDL: u32 = 15;
const OP_CHANNEL_MIX: u32 = 16;
const OP_COLOR_REPLACE: u32 = 17;
const OP_ALPHA_ADJUST: u32 = 18;
const OP_VIGNETTE: u32 = 19;
const OP_BOX: u32 = 20;
const OP_DIRECTIONAL: u32 = 22;
const OP_UNSHARP: u32 = 23;
const OP_CROP: u32 = 24;
const OP_RESAMPLE: u32 = 25;
const OP_HFLIP: u32 = 26;
const OP_VFLIP: u32 = 27;
const OP_MIRROR: u32 = 28;
const OP_OFFSET: u32 = 29;
const OP_VIDEO_LIMITER: u32 = 30;
const OP_LUMETRI: u32 = 31;

/// One pass before texture routing: entry point, op code, integer and float parameters.
struct Step {
    run: bool,
    code: u32,
    i1: [u32; 4],
    p: [f32; 24],
    /// Unsharp combine: reads the blurred image and the original.
    combine: bool,
    /// A blur pass of Unsharp Mask (the original stays untouched until the combine).
    unsharp_blur: bool,
}

fn step(code: u32, i1: [u32; 4], p: &[f32]) -> Step {
    let mut q = [0.0; 24];
    for (d, s) in q.iter_mut().zip(p) {
        *d = *s;
    }
    Step {
        run: false,
        code,
        i1,
        p: q,
        combine: false,
        unsharp_blur: false,
    }
}

/// Box passes along x (vertical = false) or y for the radii (radius 0 is a no-op on the CPU).
fn boxes(radii: &[u32], vertical: bool, repeat: bool, out: &mut Vec<Step>) {
    for &r in radii.iter().filter(|r| **r > 0) {
        let mut s = step(OP_BOX, [r, repeat as u32, vertical as u32, 0], &[]);
        s.run = r > BOX_PER_PIXEL_MAX;
        out.push(s);
    }
}

/// The passes of an op (empty: no-op). Unsharp's blur passes come first, then the combine.
fn steps(op: &FxOp) -> Vec<Step> {
    let mut out = Vec::new();
    match op {
        FxOp::BrightnessContrast { br, co } => {
            out.push(step(OP_BRIGHTNESS_CONTRAST, [0; 4], &[*br, *co]))
        }
        FxOp::ProcAmp { br, co, hue, sat } => {
            out.push(step(OP_PROC_AMP, [0; 4], &[*br, *co, *hue, *sat]))
        }
        FxOp::Tint {
            black,
            white,
            amount,
        } => {
            out.push(step(
                OP_TINT,
                [0; 4],
                &[
                    black[0], black[1], black[2], *amount, white[0], white[1], white[2],
                ],
            ));
        }
        FxOp::BlackWhite => out.push(step(OP_BLACK_WHITE, [0; 4], &[])),
        FxOp::ColorBalance {
            sh,
            md,
            hi,
            preserve,
        } => out.push(step(
            OP_COLOR_BALANCE,
            [*preserve as u32, 0, 0, 0],
            &[
                sh[0], sh[1], sh[2], 0.0, md[0], md[1], md[2], 0.0, hi[0], hi[1], hi[2],
            ],
        )),
        FxOp::LeaveColor {
            amount,
            key_hue,
            tol,
            soft,
        } => out.push(step(
            OP_LEAVE_COLOR,
            [0; 4],
            &[*amount, *key_hue, *tol, *soft],
        )),
        FxOp::ChangeToColor {
            from_hue,
            to_hue,
            tol,
            soft,
        } => out.push(step(
            OP_CHANGE_TO_COLOR,
            [0; 4],
            &[*from_hue, *to_hue, *tol, *soft],
        )),
        FxOp::ColorPass { key, sim, reverse } => out.push(step(
            OP_COLOR_PASS,
            [*reverse as u32, 0, 0, 0],
            &[key[0], key[1], key[2], *sim],
        )),
        FxOp::Gamma { g } => out.push(step(OP_GAMMA, [0; 4], &[*g])),
        FxOp::Levels { ib, iw, ob, ow, g } => {
            out.push(step(OP_LEVELS, [0; 4], &[*ib, *iw, *ob, *ow, *g]))
        }
        FxOp::Extract {
            lo,
            hi,
            soft,
            invert,
        } => out.push(step(
            OP_EXTRACT,
            [*invert as u32, 0, 0, 0],
            &[*lo, *hi, *soft],
        )),
        FxOp::Invert { channel: 4, blend } => out.push(step(OP_INVERT_ALPHA, [0; 4], &[*blend])),
        FxOp::Invert { channel, blend } => {
            out.push(step(OP_INVERT, [*channel, 0, 0, 0], &[*blend]))
        }
        FxOp::Posterize { n } => out.push(step(OP_POSTERIZE, [0; 4], &[*n])),
        FxOp::Gaussian { rx, ry, repeat } => {
            boxes(rx, false, *repeat, &mut out);
            boxes(ry, true, *repeat, &mut out);
        }
        FxOp::DirectionalBlur { dx, dy, steps } => {
            if *steps >= 2 {
                out.push(step(OP_DIRECTIONAL, [*steps, 0, 0, 0], &[*dx, *dy]));
            }
        }
        FxOp::Unsharp {
            rx,
            ry,
            amount,
            threshold,
        } => {
            boxes(rx, false, true, &mut out);
            boxes(ry, true, true, &mut out);
            for s in &mut out {
                s.unsharp_blur = true;
            }
            let mut s = step(OP_UNSHARP, [0; 4], &[*amount, *threshold]);
            s.combine = true;
            out.push(s);
        }
        FxOp::Crop {
            x0,
            x1,
            y0,
            y1,
            feather,
        } => out.push(step(OP_CROP, [0; 4], &[*x0, *x1, *y0, *y1, *feather])),
        FxOp::Resample(r) => {
            let op = if (r.opacity - 1.0).abs() < 1e-6 {
                1.0
            } else {
                r.opacity
            };
            match &r.inv {
                Some(m) => out.push(step(
                    OP_RESAMPLE,
                    r.rect,
                    &[
                        m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32, op,
                        1.0,
                    ],
                )),
                None => out.push(step(OP_RESAMPLE, [0; 4], &[0.0; 8])),
            }
        }
        FxOp::HFlip => out.push(step(OP_HFLIP, [0; 4], &[])),
        FxOp::VFlip => out.push(step(OP_VFLIP, [0; 4], &[])),
        FxOp::Mirror { cx, cy, nx, ny } => out.push(step(OP_MIRROR, [0; 4], &[*cx, *cy, *nx, *ny])),
        FxOp::Offset { dx, dy, blend } => out.push(step(OP_OFFSET, [0; 4], &[*dx, *dy, *blend])),
        FxOp::AscCdl {
            slope: s,
            offset: o,
            power: p,
            sat,
        } => {
            out.push(step(
                OP_ASC_CDL,
                [0; 4],
                &[
                    s[0], s[1], s[2], *sat, o[0], o[1], o[2], 0.0, p[0], p[1], p[2],
                ],
            ));
        }
        FxOp::ChannelMix { m } => {
            let p: Vec<f32> = m.iter().flatten().copied().collect();
            out.push(step(OP_CHANNEL_MIX, [0; 4], &p));
        }
        FxOp::ColorReplace {
            sim,
            solid,
            target: t,
            replace: r,
            replace_hsl: h,
        } => out.push(step(
            OP_COLOR_REPLACE,
            [*solid as u32, 0, 0, 0],
            &[
                t[0], t[1], t[2], *sim, r[0], r[1], r[2], 0.0, h[0], h[1], h[2],
            ],
        )),
        FxOp::AlphaAdjust {
            opacity,
            ignore,
            invert,
            mask_only,
        } => {
            out.push(step(
                OP_ALPHA_ADJUST,
                [*ignore as u32, *invert as u32, *mask_only as u32, 0],
                &[*opacity],
            ));
        }
        FxOp::Vignette {
            amount,
            midpoint,
            roundness,
            feather,
            target,
        } => {
            if amount.abs() >= 1e-5 {
                out.push(step(
                    OP_VIGNETTE,
                    [0; 4],
                    &[
                        *amount, *midpoint, *roundness, *feather, target[0], target[1], target[2],
                    ],
                ));
            }
        }
        FxOp::VideoLimiter {
            max,
            comp,
            axis,
            warn,
            warning_color,
        } => {
            out.push(step(
                OP_VIDEO_LIMITER,
                [*axis, *warn as u32, 0, 0],
                &[
                    *max,
                    *comp,
                    warning_color[0],
                    warning_color[1],
                    warning_color[2],
                ],
            ));
        }
        FxOp::Lumetri {
            gains,
            exposure,
            contrast,
            hl,
            sh,
            wh,
            bl,
            sat,
            creative_on,
            faded,
            vib,
            st,
            ht,
            vignette_on,
            va,
            vmid,
            vround,
            vfeather,
            aspect,
            ..
        } => {
            let ge = [
                gains[0] * exposure,
                gains[1] * exposure,
                gains[2] * exposure,
            ];
            let b0 = -bl * 0.15;
            let w0 = 1.0 - wh * 0.15;
            let sh_k = sh * 0.35;
            let hl_k = hl * 0.35;
            let vround_aspect = super::gpufx::lumetri_vignette_aspect(*aspect, *vround);
            let st_k = [
                (st[0] - 0.5) * 0.3,
                (st[1] - 0.5) * 0.3,
                (st[2] - 0.5) * 0.3,
            ];
            let ht_k = [
                (ht[0] - 0.5) * 0.3,
                (ht[1] - 0.5) * 0.3,
                (ht[2] - 0.5) * 0.3,
            ];

            let p = [
                ge[0],
                ge[1],
                ge[2],
                b0,
                w0,
                sh_k,
                hl_k,
                *contrast,
                *faded,
                *sat,
                *vib,
                *va,
                *vmid,
                *vfeather,
                vround_aspect,
                0.0,
                st_k[0],
                st_k[1],
                st_k[2],
                0.0,
                ht_k[0],
                ht_k[1],
                ht_k[2],
                0.0,
            ];
            out.push(step(
                OP_LUMETRI,
                [*creative_on as u32, *vignette_on as u32, 0, 0],
                &p,
            ));
        }
    }
    out
}

/// The effect kernels on a device.
struct Stage {
    px: wgpu::ComputePipeline,
    run: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl Stage {
    fn new(gpu: &Gpu) -> Option<Self> {
        let built = gpu.build(|device| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kerf-fx"),
                source: wgpu::ShaderSource::Wgsl(include_str!("fx.wgsl").into()),
            });
            let layout = kerf_gpu::kernel_layout(device);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kerf-fx"),
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
            let (px, run) = (kernel("fx_px"), kernel("fx_run"));
            Self { px, run, layout }
        });
        built
            .map_err(|error| eprintln!("kerf-fx: effect kernels did not build: {error}"))
            .ok()
    }
}

fn stage(gpu: &'static Gpu) -> Option<&'static Stage> {
    static STAGE: OnceLock<Option<Stage>> = OnceLock::new();
    STAGE.get_or_init(|| Stage::new(gpu)).as_ref()
}

/// Whether `gpu` can run effects.
pub fn available(gpu: &'static Gpu) -> bool {
    stage(gpu).is_some() && super::vfx::gpu::available(gpu)
}

/// `ops` run in order on `picture`, or `None` when the kernels did not build.
pub fn run(frame: &mut Frame<'static>, picture: Picture, ops: &[FxOp]) -> Option<Picture> {
    let stage = stage(frame.gpu())?;
    let (w, h) = (picture.w, picture.h);
    let mut current = picture;
    // Unsharp Mask: the picture its blur started from, kept until the combine.
    let mut original: Option<Picture> = None;
    for step in ops.iter().flat_map(steps) {
        // (an Unsharp without blur passes, radius 0, combines the original with itself)
        if (step.unsharp_blur || step.combine) && original.is_none() {
            original = Some(current.clone());
        }
        let out = frame.take(w, h);
        let mut bytes = Vec::with_capacity(128);
        for value in [step.code, w, h, 0].iter().chain(&step.i1) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in &step.p {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let groups = if step.run {
            let lines = if step.i1[2] != 0 { w } else { h };
            (lines.div_ceil(64), 1)
        } else {
            (w.div_ceil(16), h.div_ceil(16))
        };
        let aux = original
            .as_ref()
            .filter(|_| step.combine)
            .map(|picture| &picture.view);
        frame.dispatch(
            if step.run { &stage.run } else { &stage.px },
            &stage.layout,
            &bytes,
            (&current.view, &out.view, aux),
            groups,
        );
        let previous = std::mem::replace(&mut current, out);
        // The kept original stays out of the frame's free pictures.
        if original
            .as_ref()
            .is_none_or(|kept| kept.texture != previous.texture)
        {
            frame.release(previous);
        }
        if step.combine {
            if let Some(kept) = original.take() {
                frame.release(kept);
            }
        }
    }
    Some(current)
}

#[cfg(test)]
#[path = "gpu_tests.rs"]
mod tests;
