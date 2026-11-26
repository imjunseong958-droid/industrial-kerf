//! [`Kernel::eval`]: an effect's parameters read into its kernel, the same way the CPU's code reads
//! them.

use super::*;
use crate::video::effects::{f, gaussian_boxes};
use kerf_color::{rgb_to_ycbcr, Matrix};

/// `hash3`'s constant on its third coordinate.
const HASH_Z: u64 = 0x1656_67B1_9E37_79F9;

impl Kernel {
    /// The effect `e` on a picture of `size`, if it has a kernel.
    pub fn eval(e: &EffectInstance, cx: &FxCtx, size: (usize, usize)) -> Option<Self> {
        Some(match e.effect.as_str() {
            "noise" => {
                let amount = fv(e, "amount", cx) / 100.0;
                if amount <= 0.0 {
                    return Some(Kernel::Nothing);
                }
                let frame = crate::video::effects::noise_frame_seed(cx);
                Kernel::Noise {
                    amount,
                    grain: (fv(e, "grain_size", cx).max(1.0) * cx.px_scale).max(1.0),
                    colored: bv(e, "color"),
                    clip: bv(e, "clip"),
                    hashes: [frame, frame + 7777, frame + 99_991].map(|z| z.wrapping_mul(HASH_Z)),
                }
            }
            "color_key" => {
                let color = crate::video::effects::color(e, "color", cx);
                let y = rgb_to_ycbcr(color[0], color[1], color[2], Matrix::Bt709);
                Kernel::ColorKey {
                    key: y,
                    tolerance: crate::video::effects::f(e, "tolerance", cx) / 255.0 * 0.6 + 0.01,
                    softness: crate::video::effects::f(e, "feather", cx) / 50.0 * 0.2 + 0.01,
                }
            }
            "luma_key" => Kernel::LumaKey {
                threshold: crate::video::effects::f(e, "threshold", cx) / 100.0,
                cutoff: crate::video::effects::f(e, "cutoff", cx) / 100.0,
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
                let preset = crate::video::project::effect::ultra_key_setting(chv(e, "setting"));
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
            "drop_shadow" => {
                let c = crate::video::effects::color(e, "color", cx);
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
                    only: crate::video::effects::b(e, "only"),
                }
            }
            "vr_projection" | "vr_rotate_sphere" => match immersive::turn_of(e, cx) {
                Some((turn, share)) => Kernel::Sphere {
                    turn,
                    share,
                    layout: chv(e, "frame_layout"),
                },
                None => Kernel::Nothing,
            },
            "vr_plane_to_sphere" => {
                let (turn, half) = immersive::plane_of(e, cx);
                Kernel::PlaneToSphere {
                    turn,
                    half,
                    feather: fv(e, "feather", cx) / 100.0,
                    layout: chv(e, "frame_layout"),
                }
            }
            "lens_distortion" => {
                let (w, h) = (size.0 as f64, size.1 as f64);
                let off = |id: &str, across: f64| {
                    across / 2.0 + f64::from(f(e, id, cx)) / 100.0 * across / 2.0
                };
                Kernel::Lens {
                    center: [
                        off("h_decentering", w) as f32,
                        off("v_decentering", h) as f32,
                    ],
                    reach: (w / 2.0).hypot(h / 2.0) as f32,
                    curvature: (f64::from(f(e, "curvature", cx)) / 100.0) as f32,
                }
            }
            "cineon_converter" => {
                let black = fv(e, "black10", cx);
                let white = fv(e, "white10", cx).max(black + 1.0);
                Kernel::Cineon {
                    way: chv(e, "conversion"),
                    told: kerf_kernel::Cineon {
                        black,
                        white,
                        black_inside: fv(e, "black_internal", cx) / 255.0,
                        white_inside: fv(e, "white_internal", cx) / 255.0,
                        gamma: fv(e, "gamma", cx).max(0.1),
                        rolloff: fv(e, "rolloff", cx) / 255.0,
                        offset: kerf_kernel::cineon_offset(black, white),
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
}
