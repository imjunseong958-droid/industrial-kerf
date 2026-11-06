//! The GPU's side of the effects Kerf's looks are made of (`kernels.wgsl`). The CPU code stays the
//! reference and is unchanged: [`Kernel::eval`] reads the same parameters the same way, and a test
//! compares each kernel with [`effects::apply`](crate::filmcraft::effects::apply).

use super::*;
use crate::filmcraft::effects::gaussian_boxes;
use crate::filmcraft::gpu::run as run_ops;
use crate::filmcraft::gpufx::FxOp;
use kerf_color::{rgb_to_ycbcr, Matrix};
use kerf_gpu::{wgpu, Frame, Gpu, Picture};
use std::sync::OnceLock;

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
}

/// Ultra Key's numbers as `color::ultra_key` works them out.
#[derive(Clone, Debug, PartialEq)]
pub struct UltraKey {
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
            "noise" | "noise_legacy" => {
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
                // Cleaning up the matte blurs it, which the CPU does.
                if pf("choke") > 0.0 || pf("soften") > 0.0 {
                    return None;
                }
                let correction = [
                    fv(e, "cc_saturation", cx) / 100.0,
                    fv(e, "cc_hue", cx) / 360.0,
                    fv(e, "cc_luminance", cx) / 100.0,
                ];
                Kernel::UltraKey(Box::new(UltraKey {
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
                let out = stage.pass(
                    frame,
                    &stage.ultra_key,
                    &picture,
                    None,
                    &U {
                        a: [key.output, key.dominant, u32::from(key.corrects), 0],
                        p0: [key.key[0], key.key[1], key.key[2], key.key_size],
                        p1: [key.gain, key.highlight, key.shadow, key.tolerance],
                        p2: [key.pedestal, key.contrast, key.mid, key.spill],
                        p3: [key.desaturate, key.range, key.spill_luma, 0.0],
                        p4: [key.correction[0], key.correction[1], key.correction[2], 0.0],
                        ..U::default()
                    },
                );
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
        })
    }
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
    encode_kept: wgpu::ComputePipeline,
    decode_kept: wgpu::ComputePipeline,
    radial_zoom: wgpu::ComputePipeline,
    to_unit: wgpu::ComputePipeline,
    from_unit: wgpu::ComputePipeline,
}

impl Stage {
    fn new(gpu: &Gpu) -> Option<Self> {
        let built = gpu.build(|device| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kerf-vfx"),
                source: wgpu::ShaderSource::Wgsl(include_str!("kernels.wgsl").into()),
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
                encode_kept: kernel("encode_kept"),
                decode_kept: kernel("decode_kept"),
                radial_zoom: kernel("radial_zoom"),
                to_unit: kernel("to_unit"),
                from_unit: kernel("from_unit"),
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
        ];
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

    #[test]
    fn a_matte_cleaned_up_stays_on_the_cpu() {
        let e = effect(
            "ultra_key",
            &[
                ("setting", ParamValue::Choice(3)),
                ("soften", ParamValue::Float(20.0)),
            ],
        );
        assert_eq!(Kernel::eval(&e, &context(0.0), (8, 8)), None);
    }

    #[test]
    fn holding_values_over_1_puts_them_back() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let mut image = picture(96, 54);
        for pixel in image.px.chunks_exact_mut(4).step_by(3) {
            for value in &mut pixel[..3] {
                *value *= 8.0;
            }
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

    #[test]
    fn nothing_changes_nothing() {
        let e = effect("noise", &[("amount", ParamValue::Float(0.0))]);
        assert_eq!(
            Kernel::eval(&e, &context(0.0), (8, 8)),
            Some(Kernel::Nothing)
        );
    }

    #[test]
    fn other_effects_have_no_kernel_here() {
        let e = effect("tint", &[]);
        assert_eq!(Kernel::eval(&e, &context(0.0), (8, 8)), None);
    }
}
