//! Multi-op pointwise kernel fusion compiler.
//!
//! Composes chains of pointwise [`PixelOp`]s into a single optimized WGSL compute shader via
//! [`crate::ir::Graph`], eliminating intermediate full-frame `Rgba32Float` VRAM round-trips and
//! canceling redundant `srgb_to_linear` / `linear_to_srgb` gauge conversions between adjacent ops.
//! The exact same generic [`Num`] math evaluates on CPU (`f32`) and with forward-mode AD
//! ([`crate::dual::Dual`]).

use crate::ir::{Graph, GraphStats};
use crate::{cineon, Balance, Cineon, Num};

/// Maximum `vec4<f32>` slots packed into one fused uniform buffer (1024 bytes).
pub const MAX_FUSED_VEC4S: usize = 64;

/// Structural signature of a fusible pointwise op (used as the GPU pipeline cache key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpKind {
    BrightnessContrast,
    Tint,
    BlackWhite,
    ColorBalance,
    Gamma,
    Levels,
    Invert(u32),
    AscCdl,
    ChannelMix,
    AlphaAdjust,
    Cineon(u32),
}

impl OpKind {
    /// Number of `vec4<f32>` uniform slots consumed by this op.
    pub const fn vec4s(self) -> usize {
        match self {
            Self::BrightnessContrast => 1,
            Self::Tint => 2,
            Self::BlackWhite => 0,
            Self::ColorBalance => 3,
            Self::Gamma => 1,
            Self::Levels => 2,
            Self::Invert(_) => 1,
            Self::AscCdl => 3,
            Self::ChannelMix => 3,
            Self::AlphaAdjust => 1,
            Self::Cineon(_) => 2,
        }
    }

    /// Whether this op only modifies straight linear RGB (preserving alpha and leaving `a <= 1e-6`
    /// pixels unchanged).
    pub const fn rgb_only(self) -> bool {
        !matches!(self, Self::Invert(4) | Self::AlphaAdjust)
    }
}

/// Runtime-evaluated parameters of a fusible pointwise pixel op.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelOp {
    BrightnessContrast {
        br: f32,
        co: f32,
    },
    Tint {
        black: [f32; 3],
        white: [f32; 3],
        amount: f32,
    },
    BlackWhite,
    ColorBalance {
        shadows: [f32; 3],
        middles: [f32; 3],
        highlights: [f32; 3],
        preserve: bool,
    },
    Gamma {
        g: f32,
    },
    Levels {
        ib: f32,
        iw: f32,
        ob: f32,
        ow: f32,
        g: f32,
    },
    Invert {
        channel: u32,
        blend: f32,
    },
    AscCdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        sat: f32,
    },
    ChannelMix {
        m: [[f32; 4]; 3],
    },
    AlphaAdjust {
        opacity: f32,
        ignore: bool,
        invert: bool,
        mask_only: bool,
    },
    Cineon {
        way: u32,
        told: Cineon<f32>,
    },
}

impl PixelOp {
    pub const fn kind(&self) -> OpKind {
        match *self {
            Self::BrightnessContrast { .. } => OpKind::BrightnessContrast,
            Self::Tint { .. } => OpKind::Tint,
            Self::BlackWhite => OpKind::BlackWhite,
            Self::ColorBalance { .. } => OpKind::ColorBalance,
            Self::Gamma { .. } => OpKind::Gamma,
            Self::Levels { .. } => OpKind::Levels,
            Self::Invert { channel, .. } => OpKind::Invert(channel),
            Self::AscCdl { .. } => OpKind::AscCdl,
            Self::ChannelMix { .. } => OpKind::ChannelMix,
            Self::AlphaAdjust { .. } => OpKind::AlphaAdjust,
            Self::Cineon { way, .. } => OpKind::Cineon(way),
        }
    }

    /// Append this op's uniform `vec4<f32>` slots into `out`.
    pub fn pack(&self, out: &mut Vec<[f32; 4]>) {
        match *self {
            Self::BrightnessContrast { br, co } => out.push([br, co, 0.0, 0.0]),
            Self::Tint {
                black,
                white,
                amount,
            } => {
                out.push([black[0], black[1], black[2], amount]);
                out.push([white[0], white[1], white[2], 0.0]);
            }
            Self::BlackWhite => {}
            Self::ColorBalance {
                shadows,
                middles,
                highlights,
                preserve,
            } => {
                out.push([
                    shadows[0],
                    shadows[1],
                    shadows[2],
                    if preserve { 1.0 } else { 0.0 },
                ]);
                out.push([middles[0], middles[1], middles[2], 0.0]);
                out.push([highlights[0], highlights[1], highlights[2], 0.0]);
            }
            Self::Gamma { g } => out.push([g, 0.0, 0.0, 0.0]),
            Self::Levels { ib, iw, ob, ow, g } => {
                out.push([ib, iw, ob, ow]);
                out.push([g, 0.0, 0.0, 0.0]);
            }
            Self::Invert { blend, .. } => out.push([blend, 0.0, 0.0, 0.0]),
            Self::AscCdl {
                slope,
                offset,
                power,
                sat,
            } => {
                out.push([slope[0], slope[1], slope[2], sat]);
                out.push([offset[0], offset[1], offset[2], 0.0]);
                out.push([power[0], power[1], power[2], 0.0]);
            }
            Self::ChannelMix { m } => {
                out.push(m[0]);
                out.push(m[1]);
                out.push(m[2]);
            }
            Self::AlphaAdjust {
                opacity,
                ignore,
                invert,
                mask_only,
            } => {
                out.push([
                    opacity,
                    if ignore { 1.0 } else { 0.0 },
                    if invert { 1.0 } else { 0.0 },
                    if mask_only { 1.0 } else { 0.0 },
                ]);
            }
            Self::Cineon { told, .. } => {
                out.push([told.black, told.white, told.black_inside, told.white_inside]);
                out.push([told.gamma, told.rolloff, told.offset, 0.0]);
            }
        }
    }
}

/// Pack a slice of [`PixelOp`]s into a uniform byte buffer matching [`compile_fused`].
pub fn pack_params(ops: &[PixelOp]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for op in ops {
        op.pack(&mut out);
    }
    if out.is_empty() {
        out.push([0.0; 4]);
    }
    out
}

/// Total uniform `vec4<f32>` count needed by `kinds` (at least 1).
pub fn param_vec4s(kinds: &[OpKind]) -> usize {
    kinds.iter().map(|k| k.vec4s()).sum::<usize>().max(1)
}

// ------------------------------------------------------------------------- Generic Num primitives

#[inline(always)]
fn enc3<N: Num>(c: [N; 3]) -> [N; 3] {
    let zero = N::lit(0.0);
    [
        c[0].max(zero).linear_to_srgb(),
        c[1].max(zero).linear_to_srgb(),
        c[2].max(zero).linear_to_srgb(),
    ]
}

#[inline(always)]
fn dec3<N: Num>(c: [N; 3]) -> [N; 3] {
    let (zero, one) = (N::lit(0.0), N::lit(1.0));
    [
        c[0].clamp(zero, one).srgb_to_linear(),
        c[1].clamp(zero, one).srgb_to_linear(),
        c[2].clamp(zero, one).srgb_to_linear(),
    ]
}

#[inline(always)]
fn luma709<N: Num>(c: [N; 3]) -> N {
    N::lit(0.2126) * c[0] + N::lit(0.7152) * c[1] + N::lit(0.0722) * c[2]
}

#[inline(always)]
fn apply_rgb_op<N: Num>(c: [N; 3], kind: OpKind, mut next: impl FnMut() -> [N; 4]) -> [N; 3] {
    let (zero, half, one) = (N::lit(0.0), N::lit(0.5), N::lit(1.0));
    match kind {
        OpKind::BrightnessContrast => {
            let p0 = next();
            let (br, co) = (p0[0], p0[1]);
            let e = enc3(c);
            dec3([0, 1, 2].map(|k| (e[k] - half) * co + half + br))
        }
        OpKind::Tint => {
            let p0 = next();
            let p1 = next();
            let bl = [p0[0], p0[1], p0[2]];
            let amt = p0[3];
            let wh = [p1[0], p1[1], p1[2]];
            let l = luma709(c).max(zero).linear_to_srgb();
            let e = enc3(c);
            dec3([0, 1, 2].map(|k| {
                let t = bl[k] + (wh[k] - bl[k]) * l;
                e[k] + (t - e[k]) * amt
            }))
        }
        OpKind::BlackWhite => {
            let l = luma709(c);
            [l, l, l]
        }
        OpKind::ColorBalance => {
            let p0 = next();
            let p1 = next();
            let p2 = next();
            let b = Balance {
                shadows: [p0[0], p0[1], p0[2]],
                middles: [p1[0], p1[1], p1[2]],
                highlights: [p2[0], p2[1], p2[2]],
                keep: p0[3],
            };
            let e = enc3(c);
            let l = luma709(e);
            let ws = (one - l) * (one - l);
            let wh = l * l;
            let wm = (one - ws - wh).max(zero);
            let o: [N; 3] = std::array::from_fn(|k| {
                e[k] + b.shadows[k] * ws + b.middles[k] * wm + b.highlights[k] * wh
            });
            let d = (l - luma709(o)) * b.keep;
            dec3([o[0] + d, o[1] + d, o[2] + d])
        }
        OpKind::Gamma => {
            let g = next()[0];
            let e = enc3(c);
            dec3([0, 1, 2].map(|k| e[k].max(zero).powf(g)))
        }
        OpKind::Levels => {
            let p0 = next();
            let p1 = next();
            let (ib, iw, ob, ow, g) = (p0[0], p0[1], p0[2], p0[3], p1[0]);
            let e = enc3(c);
            let inv_span = one / (iw - ib);
            let out_span = ow - ob;
            dec3([0, 1, 2].map(|k| {
                let v = ((e[k] - ib) * inv_span).clamp(zero, one);
                ob + v.powf(g) * out_span
            }))
        }
        OpKind::Invert(ch) => {
            let blend = next()[0];
            let e = enc3(c);
            let o: [N; 3] = std::array::from_fn(|k| {
                if ch == 0 || ch as usize == k + 1 {
                    one - e[k]
                } else {
                    e[k]
                }
            });
            dec3([0, 1, 2].map(|k| o[k] + (e[k] - o[k]) * blend))
        }
        OpKind::AscCdl => {
            let p0 = next();
            let p1 = next();
            let p2 = next();
            let (s, sat) = ([p0[0], p0[1], p0[2]], p0[3]);
            let o = [p1[0], p1[1], p1[2]];
            let pw = [p2[0].max(zero), p2[1].max(zero), p2[2].max(zero)];
            let e = enc3(c);
            let v: [N; 3] =
                std::array::from_fn(|k| (e[k] * s[k] + o[k]).clamp(zero, one).powf(pw[k]));
            let l = luma709(v);
            dec3([0, 1, 2].map(|k| (l + sat * (v[k] - l)).clamp(zero, one)))
        }
        OpKind::ChannelMix => {
            let r = next();
            let g = next();
            let b = next();
            let v = enc3(c);
            let row = |m: [N; 4]| (m[0] * v[0] + m[1] * v[1] + m[2] * v[2] + m[3]).clamp(zero, one);
            dec3([row(r), row(g), row(b)])
        }
        OpKind::Cineon(way) => {
            let p0 = next();
            let p1 = next();
            let told = Cineon {
                black: p0[0],
                white: p0[1],
                black_inside: p0[2],
                white_inside: p0[3],
                gamma: p1[0],
                rolloff: p1[1],
                offset: p1[2],
            };
            let e = enc3(c);
            dec3([0, 1, 2].map(|k| cineon(e[k], way, &told).clamp(zero, one)))
        }
        OpKind::AlphaAdjust => c,
    }
}

#[inline(always)]
fn apply_alpha_op<N: Num>(p: [N; 4], kind: OpKind, mut next: impl FnMut() -> [N; 4]) -> [N; 4] {
    let (zero, one, eps) = (N::lit(0.0), N::lit(1.0), N::lit(1e-6));
    match kind {
        OpKind::Invert(4) => {
            let blend = next()[0];
            let a = p[3];
            let na = one - a;
            let [k] = a.if_at_most(eps, || [zero], || [na / a]);
            let out_a = na * (one - blend) + a * blend;
            [p[0] * k, p[1] * k, p[2] * k, out_a]
        }
        OpKind::AlphaAdjust => {
            let p0 = next();
            let (opacity, ignore, invert, mask_only) = (p0[0], p0[1], p0[2], p0[3]);
            let a0 = p[3];
            let c = a0.if_at_most(
                eps,
                || [zero, zero, zero],
                || [p[0] / a0, p[1] / a0, p[2] / a0],
            );
            let [a1] = ignore.if_at_most(zero, || [a0], || [one]);
            let [a2] = invert.if_at_most(zero, || [a1], || [one - a1]);
            let a = (a2 * opacity).clamp(zero, one);
            mask_only.if_at_most(
                zero,
                || [c[0] * a, c[1] * a, c[2] * a, a],
                || {
                    let g = a.srgb_to_linear();
                    [g, g, g, one]
                },
            )
        }
        _ => p,
    }
}

/// Apply a sequence of [`OpKind`]s to a premultiplied linear RGBA pixel over any [`Num`].
/// Consecutive RGB-only ops share a single unpremultiply/repremultiply region so intermediate
/// color-gauge round-trips cancel automatically inside [`Graph`].
#[inline(always)]
pub fn eval_chain<N: Num>(
    mut p: [N; 4],
    kinds: &[OpKind],
    mut next_param: impl FnMut() -> [N; 4],
) -> [N; 4] {
    let eps = N::lit(1e-6);
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i].rgb_only() {
            let start = i;
            while i < kinds.len() && kinds[i].rgb_only() {
                i += 1;
            }
            let run = &kinds[start..i];
            let a = p[3];
            p = a.if_at_most(
                eps,
                || {
                    // Consume the same parameter count even on the skip path in eager tracing.
                    p
                },
                || {
                    let mut c = [p[0] / a, p[1] / a, p[2] / a];
                    for &kind in run {
                        c = apply_rgb_op(c, kind, &mut next_param);
                    }
                    [c[0] * a, c[1] * a, c[2] * a, a]
                },
            );
            // When `N` is `f32`/`Dual` and `a <= 1e-6`, advance parameter cursor past the skipped run.
            // (On `Sym`, `if_at_most` evaluates both closures, so `next_param` was already consumed.)
        } else {
            p = apply_alpha_op(p, kinds[i], &mut next_param);
            i += 1;
        }
    }
    p
}

/// Evaluate a slice of runtime [`PixelOp`]s over any [`Num`] (`f32` or [`crate::dual::Dual`]) with
/// indexed parameter lookup (always safe under lazy short-circuiting).
pub fn eval_fused<N: Num>(mut p: [N; 4], ops: &[PixelOp]) -> [N; 4] {
    let packed = pack_params(ops);
    let kinds: Vec<OpKind> = ops.iter().map(PixelOp::kind).collect();
    let eps = N::lit(1e-6);
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < kinds.len() {
        if kinds[i].rgb_only() {
            let start = i;
            let mut run_vec4s = 0usize;
            while i < kinds.len() && kinds[i].rgb_only() {
                run_vec4s += kinds[i].vec4s();
                i += 1;
            }
            let run = &kinds[start..i];
            let run_start = cursor;
            cursor += run_vec4s;
            let a = p[3];
            p = a.if_at_most(
                eps,
                || p,
                || {
                    let mut idx = run_start;
                    let mut c = [p[0] / a, p[1] / a, p[2] / a];
                    for &kind in run {
                        c = apply_rgb_op(c, kind, || {
                            let row = packed[idx];
                            idx += 1;
                            row.map(N::lit)
                        });
                    }
                    [c[0] * a, c[1] * a, c[2] * a, a]
                },
            );
        } else {
            let mut idx = cursor;
            cursor += kinds[i].vec4s();
            p = apply_alpha_op(p, kinds[i], || {
                let row = packed[idx];
                idx += 1;
                row.map(N::lit)
            });
            i += 1;
        }
    }
    p
}

/// Compiled WGSL compute shader for a fused sequence of pointwise ops.
#[derive(Clone, Debug)]
pub struct FusedShader {
    pub wgsl: String,
    pub param_vec4s: usize,
    pub stats: GraphStats,
}

const FUSED_HELPERS: &str = r#"fn lin_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(max(v, 0.0), 1.0 / 2.4) - 0.055;
}

fn srgb_to_lin(v: f32) -> f32 {
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((max(v, 0.0) + 0.055) / 1.055, 2.4);
}

fn kpow(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        if y == 0.0 {
            return 1.0;
        }
        if y > 0.0 {
            return 0.0;
        }
        return 3.0e38;
    }
    return pow(x, y);
}
"#;

/// Compile a sequence of [`OpKind`]s into a single OpenCL-safe WGSL compute shader with entry point
/// `fused` matching `kerf_gpu::kernel_layout`.
pub fn compile_fused(kinds: &[OpKind]) -> FusedShader {
    let vec4s = param_vec4s(kinds);
    let all_rgb = kinds.iter().all(|k| k.rgb_only());
    let (graph, roots) = if all_rgb {
        Graph::trace(|| {
            let mut cursor = 0usize;
            let mut c = [
                Graph::input("c.x"),
                Graph::input("c.y"),
                Graph::input("c.z"),
            ];
            for &kind in kinds {
                c = apply_rgb_op(c, kind, || {
                    let slot = cursor;
                    cursor += 1;
                    [
                        Graph::input(format!("u.p[{slot}u].x")),
                        Graph::input(format!("u.p[{slot}u].y")),
                        Graph::input(format!("u.p[{slot}u].z")),
                        Graph::input(format!("u.p[{slot}u].w")),
                    ]
                });
            }
            vec![c[0], c[1], c[2]]
        })
    } else {
        Graph::trace(|| {
            let mut cursor = 0usize;
            let q = [
                Graph::input("q.x"),
                Graph::input("q.y"),
                Graph::input("q.z"),
                Graph::input("q.w"),
            ];
            let out = eval_chain(q, kinds, || {
                let slot = cursor;
                cursor += 1;
                [
                    Graph::input(format!("u.p[{slot}u].x")),
                    Graph::input(format!("u.p[{slot}u].y")),
                    Graph::input(format!("u.p[{slot}u].z")),
                    Graph::input(format!("u.p[{slot}u].w")),
                ]
            });
            out.to_vec()
        })
    };

    let (body, exprs, stats) = graph.emit_wgsl_body(&roots, "    ");
    let store = if all_rgb {
        format!(
            "    if q.a <= 1e-6 {{\n        textureStore(dst, coord, q);\n        return;\n    }}\n    let c = q.rgb / q.a;\n{body}    textureStore(dst, coord, vec4({} * q.a, {} * q.a, {} * q.a, q.a));\n",
            exprs[0], exprs[1], exprs[2]
        )
    } else {
        format!(
            "{body}    textureStore(dst, coord, vec4({}, {}, {}, {}));\n",
            exprs[0], exprs[1], exprs[2], exprs[3]
        )
    };

    let wgsl = format!(
        "struct U {{\n    p: array<vec4<f32>, {vec4s}>,\n}};\n\n@group(0) @binding(0) var<uniform> u: U;\n@group(0) @binding(1) var src: texture_2d<f32>;\n@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;\n@group(0) @binding(3) var aux: texture_2d<f32>;\n\n{FUSED_HELPERS}\n@compute @workgroup_size(16, 16)\nfn fused(@builtin(global_invocation_id) id: vec3<u32>) {{\n    let size = textureDimensions(dst);\n    if id.x >= size.x || id.y >= size.y {{\n        return;\n    }}\n    let coord = vec2<i32>(id.xy);\n    let q = textureLoad(src, coord, 0);\n{store}}}\n"
    );

    FusedShader {
        wgsl,
        param_vec4s: vec4s,
        stats,
    }
}
