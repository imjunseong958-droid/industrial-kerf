//! The GPU's side of the transitions (`kernels.wgsl`). The CPU code stays the reference:
//! [`Kernel::of`] reads the same parameters the same way, with the CPU's own functions for what a
//! frame works out once (a wipe's sweep, the cards of a scene), and a test holds each kernel to
//! [`apply_scaled`](super::apply_scaled) value by value.

use super::motion::{card_scene, whip_layers, Layer};
use super::wipe::{angle_dir, field_range, star_dist, threshold, WipeStyle};
use super::{
    bell, disc_offsets, ease_in, ease_in_out, ease_out, hash, mid_mix, smoothstep, Fx, Tx,
};
use crate::filmcraft::gpu::run as run_ops;
use crate::filmcraft::gpufx::FxOp;
use kerf_draw::Image;
use kerf_gpu::{wgpu, Frame, Gpu, Picture};
use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::OnceLock;

/// Why a transition stays on the CPU, by its FilmCraft id. The test in `gpu_tests.rs` says which they
/// are by the catalog's names.
const ON_THE_CPU: &[(&str, &str)] = &[(
    "morph_cut",
    "dense optical flow between the two frames, which needs both read back",
)];

/// As many of the numbers as `v` in `kernels.wgsl` has room for.
const NUMBERS: usize = 800;

/// The most samples a pixel takes of a card scene's cards, past which the uniforms would not hold them.
const MOST_CARDS: usize = 36;

/// Which of the kernels in `kernels.wgsl` a pass runs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Entry {
    Wipes,
    Scene,
    Mixes,
    Blurs,
    Moves,
    Grunge,
}

/// The numbers one pass reads.
#[derive(Clone, Debug, PartialEq)]
struct Pass {
    entry: Entry,
    kind: u32,
    count: u32,
    layers: u32,
    seed: u32,
    shutter: f32,
    numbers: Vec<f32>,
}

impl Pass {
    fn new(entry: Entry, kind: u32, numbers: Vec<f32>) -> Self {
        Self {
            entry,
            kind,
            count: 1,
            layers: 0,
            seed: 0,
            shutter: 0.0,
            numbers,
        }
    }

    fn with_count(mut self, count: usize) -> Self {
        self.count = count as u32;
        self
    }

    fn with_seed(mut self, seed: u32) -> Self {
        self.seed = seed;
        self
    }

    fn with_time(mut self, (shutter, samples): (f32, usize)) -> Self {
        self.shutter = shutter;
        self.count = samples as u32;
        self
    }

    fn bytes(&self, (w, h): (u32, u32), p: f32) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + NUMBERS * 4);
        for value in [self.kind, self.count, self.layers, self.seed] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in [w as f32, h as f32, p, self.shutter] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let mut written = 0;
        for value in self.numbers.iter().take(NUMBERS) {
            out.extend_from_slice(&value.to_le_bytes());
            written += 1;
        }
        out.resize(out.len() + (NUMBERS - written) * 4, 0);
        out
    }
}

/// One transition at one moment, its numbers worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    size: (u32, u32),
    p: f32,
    how: How,
}

#[derive(Clone, Debug, PartialEq)]
enum How {
    /// At its first moment it is the outgoing clip, and at its last the incoming one.
    Outgoing,
    Incoming,
    /// One pass reading the two clips.
    One(Pass),
    /// Each clip blurred, then mixed.
    BlurDissolve {
        sigma_a: f32,
        sigma_b: f32,
        k: f32,
    },
    /// The clips mixed, and its own blur added back.
    Glow {
        k: f32,
        sigma: f32,
        gain: f32,
    },
    /// The clips mixed, and the zoom blur of what is bright in that added back.
    Ray {
        k: f32,
        rays: Pass,
    },
    /// Pushed along, then smeared along that way.
    Whip {
        pushed: Pass,
        smear: Option<Pass>,
    },
    /// Each quarter of the outgoing clip moved off over the incoming one.
    CenterSplit {
        off: (f64, f64),
    },
}

fn truth(value: bool) -> f32 {
    f32::from(u8::from(value))
}

/// A wipe's numbers, as `kernels.wgsl` reads them: its style, the sweep's threshold, the field's own.
fn wipe(
    t: &Tx,
    st: WipeStyle,
    kind: u32,
    field: &(impl Fn(f32, f32) -> f32 + Sync),
    own: &[f32],
) -> How {
    let mut numbers = vec![st.feather, st.border, st.aa, threshold(t, &st, field)];
    numbers.extend_from_slice(&st.border_color);
    numbers.extend_from_slice(own);
    How::One(Pass::new(Entry::Wipes, kind, numbers).with_seed(t.seed()))
}

/// Cards as `kernels.wgsl` reads them: from 4, four rows for each. `None` when the moments do not
/// all have the same number of them.
fn card_numbers(t: &Tx, focal: f32, moments: &[Vec<Layer>]) -> Option<(Vec<f32>, usize)> {
    let layers = moments.first()?.len();
    if moments.iter().any(|one| one.len() != layers) || layers * moments.len() > MOST_CARDS {
        return None;
    }
    let eye = super::cards::eye(t.wf, t.hf, focal);
    let mut numbers = vec![eye[0], eye[1], eye[2], 0.0];
    for layer in moments.iter().flatten() {
        let is_b = |image: &Image| std::ptr::eq(image, t.b);
        let mut flags = 0u32;
        if is_b(layer.front) {
            flags |= 1;
        }
        if let Some(back) = layer.back {
            flags |= 2;
            if is_b(back) {
                flags |= 4;
            }
        }
        if layer.mirror_y {
            flags |= 8;
        }
        if layer.shade {
            flags |= 16;
        }
        let card = &layer.card;
        numbers.extend_from_slice(&card.origin);
        numbers.push(layer.opacity);
        numbers.extend_from_slice(&card.ex);
        numbers.push(flags as f32);
        numbers.extend_from_slice(&card.ey);
        numbers.push(0.0);
        numbers.extend_from_slice(&layer.clip);
    }
    Some((numbers, layers))
}

/// The moments `motion::scene` draws a scene at.
fn moments((shutter, samples): (f32, usize), p: f32) -> Vec<f32> {
    if samples <= 1 || shutter <= 0.0 {
        vec![p]
    } else {
        (0..samples)
            .map(|i| (p + (i as f32 / (samples - 1) as f32 - 0.5) * shutter).clamp(0.0, 1.0))
            .collect()
    }
}

/// A color's numbers, then a kernel's own.
fn with_color(color: [f32; 4], own: &[f32]) -> Vec<f32> {
    let mut numbers = color.to_vec();
    numbers.extend_from_slice(own);
    numbers
}

impl Kernel {
    /// The transition `e` at progress `p` between pictures of `size`, or `None` when it stays on
    /// the CPU. `scale` is what [`apply_scaled`](super::apply_scaled) takes.
    pub fn of(e: &Fx, size: (usize, usize), p: f32, scale: f32) -> Option<Self> {
        let (w, h) = size;
        if w == 0 || h == 0 || ON_THE_CPU.iter().any(|(id, _)| *id == e.id) {
            return None;
        }
        let p = if p.is_finite() {
            p.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let how = if p <= 0.0 {
            How::Outgoing
        } else if p >= 1.0 {
            How::Incoming
        } else {
            // What the CPU's functions read of a picture is its size.
            let (a, b) = (
                Image {
                    w,
                    h,
                    px: Vec::new(),
                },
                Image {
                    w,
                    h,
                    px: Vec::new(),
                },
            );
            let t = Tx {
                e,
                a: &a,
                b: &b,
                p,
                w,
                h,
                wf: w as f32,
                hf: h as f32,
                scale,
            };
            how(e.id.as_str(), &t)?
        };
        Some(Self {
            size: (w as u32, h as u32),
            p,
            how,
        })
    }

    /// The two pictures mixed, or `None` when the kernels did not build.
    pub fn run(&self, frame: &mut Frame<'static>, a: &Picture, b: &Picture) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        Some(match &self.how {
            How::Outgoing => frame.copy(a),
            How::Incoming => frame.copy(b),
            How::One(pass) => stage.pass(frame, pass, self.p, a, Some(b)),
            How::BlurDissolve {
                sigma_a,
                sigma_b,
                k,
            } => {
                let blurred_a = blurred(frame, a, *sigma_a)?;
                let blurred_b = blurred(frame, b, *sigma_b)?;
                let out = stage.mix(frame, self.p, &blurred_a, &blurred_b, *k);
                frame.release(blurred_a);
                frame.release(blurred_b);
                out
            }
            How::Glow { k, sigma, gain } => {
                let mixed = stage.mix(frame, self.p, a, b, *k);
                let light = blurred(frame, &mixed, *sigma)?;
                let add = Pass::new(Entry::Blurs, 6, vec![*gain]);
                let out = stage.pass(frame, &add, self.p, &mixed, Some(&light));
                frame.release(mixed);
                frame.release(light);
                out
            }
            How::Ray { k, rays } => {
                let mixed = stage.mix(frame, self.p, a, b, *k);
                let bright = Pass::new(Entry::Mixes, 16, Vec::new());
                let bright = stage.pass(frame, &bright, self.p, &mixed, None);
                let out = stage.pass(frame, rays, self.p, &mixed, Some(&bright));
                frame.release(mixed);
                frame.release(bright);
                out
            }
            How::Whip { pushed, smear } => {
                let moved = stage.pass(frame, pushed, self.p, a, Some(b));
                match smear {
                    Some(smear) => {
                        let out = stage.pass(frame, smear, self.p, &moved, None);
                        frame.release(moved);
                        out
                    }
                    None => moved,
                }
            }
            How::CenterSplit { off } => {
                let mut out = frame.copy(b);
                let whole = kerf_draw::Rect::new(0.0, 0.0, f64::from(a.w), f64::from(a.h));
                let quarters = [
                    (-off.0, -off.1, 0.0, 0.0),
                    (off.0, -off.1, 0.5, 0.0),
                    (-off.0, off.1, 0.0, 0.5),
                    (off.0, off.1, 0.5, 0.5),
                ];
                for (dx, dy, qx, qy) in quarters {
                    let quarter = Pass::new(Entry::Mixes, 15, vec![qx, qy]);
                    let quarter = stage.pass(frame, &quarter, self.p, a, None);
                    out = frame.composite(
                        out,
                        &quarter,
                        &kerf_draw::Affine::translate(dx, dy),
                        whole,
                        1.0,
                        kerf_draw::Blend::Normal,
                    );
                    frame.release(quarter);
                }
                out
            }
        })
    }
}

/// A blurred copy, as `blurred` makes it on the CPU.
fn blurred(frame: &mut Frame<'static>, picture: &Picture, sigma: f32) -> Option<Picture> {
    let copy = frame.copy(picture);
    if sigma <= 0.3 {
        return Some(copy);
    }
    let (across, down) =
        kerf_draw::blur::gaussian_radii(picture.w as usize, picture.h as usize, sigma, sigma);
    let radii = |radii: Vec<usize>| radii.into_iter().map(|r| r as u32).collect();
    run_ops(
        frame,
        copy,
        &[FxOp::Gaussian {
            rx: radii(across),
            ry: radii(down),
            repeat: true,
        }],
    )
}

/// The numbers of the transition `id`, as its arm of `apply` works them out.
fn how(id: &str, t: &Tx) -> Option<How> {
    if let Some(how) = how_wipe(id, t) {
        return Some(how);
    }
    if let Some(found) = card_scene(id, t) {
        let at = moments(found.blur, t.p);
        let built: Vec<Vec<Layer>> = at.iter().map(|&q| (found.build)(q)).collect();
        let (numbers, layers) = card_numbers(t, found.focal, &built)?;
        let mut pass = Pass::new(Entry::Scene, 0, numbers).with_count(at.len());
        pass.layers = layers as u32;
        return Some(How::One(pass));
    }
    how_mix(id, t)
        .or_else(|| how_blur(id, t))
        .or_else(|| how_move(id, t))
        .or_else(|| how_grunge(id, t))
}

fn how_wipe(id: &str, t: &Tx) -> Option<How> {
    let st = WipeStyle::of(t);
    let (cx, cy) = t.center();
    let (w, h) = (t.wf, t.hf);
    let asp = w / h.max(1.0);
    Some(match id {
        "soft_wipe" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            wipe(t, st, 0, &|x, y| x * dx + y * dy, &[dx, dy])
        }
        "clock_wipe" => {
            use std::f32::consts::TAU;
            let start = t.num("start").to_radians();
            let scale = (w + h) * 0.5;
            let field = move |x: f32, y: f32| {
                let a = (x - cx).atan2(-(y - cy)) - start;
                a.rem_euclid(TAU) / TAU * scale
            };
            wipe(t, st, 1, &field, &[cx, cy, start, scale])
        }
        "radial_wipe" => {
            use std::f32::consts::TAU;
            let (ox, oy, a0) = match t.choice("corner") {
                1 => (w, 0.0, FRAC_PI_2),
                2 => (w, h, PI),
                3 => (0.0, h, -FRAC_PI_2),
                _ => (0.0, 0.0, 0.0),
            };
            let scale = (w + h) * 0.5;
            let field = move |x: f32, y: f32| {
                let a = (y - oy).atan2(x - ox) - a0;
                let a = a.rem_euclid(TAU);
                let a = if a > PI * 1.5 { 0.0 } else { a.min(FRAC_PI_2) };
                a / FRAC_PI_2 * scale
            };
            wipe(t, st, 2, &field, &[ox, oy, a0, scale])
        }
        "star_wipe" => {
            let points = t.num("points").round().clamp(3.0, 24.0) as u32;
            let inner = t.frac("inner");
            let (s, c) = t.num("rotation").to_radians().sin_cos();
            let field = move |x: f32, y: f32| {
                let (rx, ry) = (x - cx, y - cy);
                star_dist(rx * c + ry * s, -rx * s + ry * c, points, inner)
            };
            wipe(t, st, 3, &field, &[cx, cy, s, c, points as f32, inner])
        }
        "panel_wipe" => {
            let (dx, dy) = t.dir();
            let count = t.num("panels").round().clamp(1.0, 64.0);
            let along_len = t.travel();
            let across_len = if dx != 0.0 { h } else { w };
            let field = move |x: f32, y: f32| {
                let along = x * dx + y * dy;
                let across = if dx != 0.0 { y } else { x };
                let idx = (across / across_len * count)
                    .floor()
                    .clamp(0.0, count - 1.0);
                along + idx * along_len * 0.6 / count
            };
            wipe(t, st, 4, &field, &[dx, dy, count, along_len, across_len])
        }
        "plateau_wipe" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            let count = t.num("plateaus").round().clamp(1.0, 64.0);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| -x * dy + y * dx);
            let span = (hi - lo).max(1.0);
            let step = (w.max(h)) * 0.35 / count.max(1.0);
            let field = move |x: f32, y: f32| {
                let along = x * dx + y * dy;
                let across = ((-x * dy + y * dx) - lo) / span;
                let idx = (across * count).floor().clamp(0.0, count - 1.0);
                along + (idx - (count - 1.0) / 2.0).abs() * step
            };
            wipe(t, st, 5, &field, &[dx, dy, count, lo, span, step])
        }
        "iris_round" => {
            let field = move |x: f32, y: f32| kerf_kernel::iris_round(x, y, cx, cy);
            wipe(t, st, 6, &field, &[cx, cy])
        }
        "iris_box" => {
            let field = move |x: f32, y: f32| (x - cx).abs().max((y - cy).abs() * asp);
            wipe(t, st, 7, &field, &[cx, cy, asp])
        }
        "iris_diamond" => {
            let field = move |x: f32, y: f32| (x - cx).abs() + (y - cy).abs() * asp;
            wipe(t, st, 8, &field, &[cx, cy, asp])
        }
        "iris_cross" => {
            let field = move |x: f32, y: f32| {
                let (u, v) = ((x - cx).abs(), (y - cy).abs() * asp);
                let k = 0.3;
                (u / k).max(v).min(u.max(v / k))
            };
            wipe(t, st, 9, &field, &[cx, cy, asp])
        }
        "barn_doors" => {
            let vertical = t.choice("orientation") == 1;
            let field = move |x: f32, y: f32| {
                if vertical {
                    (x - cx).abs()
                } else {
                    (y - cy).abs() * asp
                }
            };
            wipe(t, st, 10, &field, &[cx, cy, asp, truth(vertical)])
        }
        "inset" => {
            let (ox, oy) = match t.choice("corner") {
                1 => (w, 0.0),
                2 => (w, h),
                3 => (0.0, h),
                _ => (0.0, 0.0),
            };
            let field = move |x: f32, y: f32| (x - ox).abs().max((y - oy).abs() * asp);
            wipe(t, st, 11, &field, &[ox, oy, asp])
        }
        "grunge" => {
            let scale = t.px("scale").max(4.0);
            let s = t.seed();
            let st = WipeStyle {
                feather: 2.0 * t.scale,
                border: t.px("edge_width"),
                border_color: t.color("edge_color"),
                aa: 1.0,
            };
            let field = move |x: f32, y: f32| {
                let n = super::fbm(x / scale, y / scale, s) * 0.8
                    + super::vnoise(x / (scale * 0.15), y / (scale * 0.15), s + 9) * 0.2;
                n * scale * 3.0
            };
            wipe(t, st, 12, &field, &[scale])
        }
        _ => return None,
    })
}

fn how_mix(id: &str, t: &Tx) -> Option<How> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    let k = mid_mix(p);
    let env = bell(p);
    let mixes = |kind: u32, numbers: Vec<f32>| How::One(Pass::new(Entry::Mixes, kind, numbers));
    Some(match id {
        "additive_dissolve" => {
            let (ka, kb) = ((2.0 * (1.0 - p)).min(1.0), (2.0 * p).min(1.0));
            mixes(1, vec![ka, kb])
        }
        "film_dissolve" => mixes(2, Vec::new()),
        "burn_alpha" => {
            let soft = t.frac("softness").clamp(0.01, 1.0) * 0.3;
            let scale = (220.0 * t.unit()).max(1.0);
            let thr = -soft + p * (1.0 + 2.0 * soft);
            let fade = (p * 10.0).min((1.0 - p) * 10.0).min(1.0);
            How::One(
                Pass::new(
                    Entry::Mixes,
                    3,
                    with_color(t.color("burn_color"), &[soft, scale, thr, fade]),
                )
                .with_seed(t.seed()),
            )
        }
        "luma_fade" => {
            let soft = t.frac("softness").clamp(0.005, 1.0) * 0.5;
            let thr = -soft + p * (1.0 + 2.0 * soft);
            mixes(
                4,
                vec![
                    soft,
                    truth(t.choice("source") == 1),
                    truth(t.flag("invert")),
                    thr,
                ],
            )
        }
        "mosaic_transition" => {
            let max = t.px("block_size").max(1.0);
            let bs = 1.0 + (max - 1.0) * bell(p);
            mixes(5, vec![bs, k, truth(t.flag("dissolve"))])
        }
        "burn_chroma" => {
            let amt = t.frac("amount");
            let hue = p * PI * 2.0;
            mixes(
                6,
                vec![
                    k,
                    amt * env * 1.8,
                    1.0 + 0.5 * hue.cos(),
                    1.0 + 0.5 * (hue + 2.1).cos(),
                    1.0 + 0.5 * (hue + 4.2).cos(),
                ],
            )
        }
        "chroma_leak" | "light_leak" => {
            let amt = t.frac("amount");
            let s = t.seed();
            let leak = id == "light_leak";
            let user = t.color("color");
            let cols: [[f32; 4]; 3] = if leak {
                [
                    user,
                    [user[0], user[1] * 0.8, user[2] * 0.5, 1.0],
                    [1.0, 0.9, 0.7, 1.0],
                ]
            } else {
                [
                    [1.0, 0.15, 0.6, 1.0],
                    [0.1, 0.8, 1.0, 1.0],
                    [1.0, 0.85, 0.1, 1.0],
                ]
            };
            let mut numbers = vec![k, amt, env, 0.0];
            for i in 0..3 {
                let r0 = hash(i, 1, s);
                let r1 = hash(i, 2, s);
                let side = if leak { 0.0 } else { r0 };
                numbers.extend_from_slice(&[
                    w * (side + (r1 - 0.5) * 0.4 + (p - 0.5) * (0.8 + r0)),
                    h * (0.2 + 0.6 * hash(i, 3, s) + (p - 0.5) * (r1 - 0.5)),
                    w.max(h) * (0.25 + 0.25 * hash(i, 4, s)),
                    0.0,
                ]);
                numbers.extend_from_slice(&cols[i as usize]);
            }
            mixes(7, numbers)
        }
        "flare" => {
            let (dx, dy) = angle_dir(t.num("angle") + 90.0);
            let diag = (w * w + h * h).sqrt();
            let (fx, fy) = (
                w / 2.0 + dx * diag * (p - 0.5) * 0.9,
                h / 2.0 + dy * diag * (p - 0.5) * 0.9,
            );
            mixes(
                8,
                with_color(
                    t.color("color"),
                    &[k, t.frac("amount"), env, dx, dy, diag, fx, fy, h.max(1.0)],
                ),
            )
        }
        "flash" => {
            let inten = t.frac("intensity").clamp(0.0, 1.0);
            let cut = smoothstep(0.45, 0.55, p);
            let f = inten * (-((p - 0.5) / 0.12).powi(2)).exp() * smoothstep(0.0, 0.2, env);
            mixes(9, with_color(t.color("color"), &[cut, f]))
        }
        "phosphor" => {
            let decay = t.frac("decay").clamp(0.0, 1.0);
            let kk = smoothstep(0.15, 0.85, p);
            let persist = (1.0 - p).powf(0.5 + 3.0 * decay) * smoothstep(0.0, 0.15, p);
            mixes(10, with_color(t.color("color"), &[kk, persist]))
        }
        "solarize" => mixes(11, vec![k, t.frac("amount").clamp(0.0, 1.0) * env]),
        "stripe" => {
            let count = t.num("stripes").round().clamp(1.0, 64.0);
            let (dx, dy) = angle_dir(t.num("angle") + 90.0);
            let (px_, py_) = (-dy, dx);
            let ends = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
            let rng = |f: &dyn Fn(f32, f32) -> f32| {
                let v = ends.map(|(a, b)| f(a, b));
                (
                    v.iter().copied().fold(f32::INFINITY, f32::min),
                    v.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                )
            };
            let (alo, ahi) = rng(&|x, y| x * dx + y * dy);
            let (clo, chi) = rng(&|x, y| x * px_ + y * py_);
            let span = (ahi - alo).max(1.0);
            mixes(
                12,
                with_color(
                    t.color("color"),
                    &[count, dx, dy, px_, py_, alo, clo, chi, span, span * 0.08],
                ),
            )
        }
        "light_sweep" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            let bw = t.px("width").max(2.0);
            let ends = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
                .map(|(cx, cy): (f32, f32)| cx * dx + cy * dy);
            let lo = ends.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = ends.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let e = lo - bw + p * (hi - lo + 2.0 * bw);
            mixes(
                13,
                with_color(t.color("color"), &[dx, dy, bw, t.frac("amount"), e]),
            )
        }
        "neon_wipe" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            let glow = t.px("glow").max(1.0);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
            let m = glow * 0.5 + 2.0;
            let thr = lo - m + p * (hi - lo + 2.0 * m);
            let fade = (p * 10.0).min((1.0 - p) * 10.0).min(1.0);
            mixes(
                14,
                with_color(
                    t.color("color"),
                    &[
                        dx,
                        dy,
                        glow,
                        t.frac("amount"),
                        thr,
                        fade,
                        WipeStyle::of(t).aa,
                    ],
                ),
            )
        }
        "center_split" => {
            let pd = f64::from(p);
            How::CenterSplit {
                off: (
                    f64::from(t.w as u32) * 0.5 * pd,
                    f64::from(t.h as u32) * 0.5 * pd,
                ),
            }
        }
        "blur_dissolve" => {
            let sigma = t.px("blur") * 0.5;
            How::BlurDissolve {
                sigma_a: sigma * smoothstep(0.0, 0.6, p),
                sigma_b: sigma * (1.0 - smoothstep(0.4, 1.0, p)),
                k: smoothstep(0.25, 0.75, p),
            }
        }
        "glow" => How::Glow {
            k,
            sigma: t.px("radius").max(1.0) * 0.5,
            gain: t.frac("intensity") * env * 2.0,
        },
        "ray" => {
            let (cx, cy) = t.center();
            let g = t.frac("amount") * env * 3.0;
            let len = t.frac("length").clamp(0.0, 1.0);
            How::Ray {
                k,
                rays: Pass::new(Entry::Blurs, 4, vec![cx, cy, len * 0.9, g]),
            }
        }
        "whip" => {
            let (dx, dy) = t.dir();
            let travel = t.travel();
            let layers = whip_layers(t);
            let (numbers, count) =
                card_numbers(t, super::cards::focal_for(t.wf, t.hf, 0.0), &[layers])?;
            let mut pushed = Pass::new(Entry::Scene, 0, numbers);
            pushed.layers = count as u32;
            let len = t.frac("blur") * bell(t.p) * travel * 0.18;
            let smear = (len >= 0.75).then(|| {
                Pass::new(Entry::Blurs, 5, vec![dx * len, dy * len])
                    .with_count(super::taps(len * 2.0))
            });
            How::Whip { pushed, smear }
        }
        _ => return None,
    })
}

fn how_blur(id: &str, t: &Tx) -> Option<How> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    let k = mid_mix(p);
    let env = bell(p);
    let blurs = |kind: u32, numbers: Vec<f32>, count: usize| {
        How::One(Pass::new(Entry::Blurs, kind, numbers).with_count(count))
    };
    Some(match id {
        "cross_zoom" | "zoom_blur" => {
            let (cx, cy) = t.center();
            let st = t.frac("strength");
            let zooms = id == "cross_zoom";
            let za = if zooms {
                1.0 + 3.0 * st * ease_in((p / 0.5).min(1.0))
            } else {
                1.0 + 0.1 * st * ease_in(p)
            };
            let zb = if zooms {
                1.0 + 3.0 * st * (1.0 - ease_out(((p - 0.5) / 0.5).clamp(0.0, 1.0)))
            } else {
                1.0 + 0.1 * st * (1.0 - ease_out(p))
            };
            let blur = st * env * 0.6;
            blurs(
                0,
                vec![cx, cy, za, zb, blur, k],
                super::taps(blur * w.max(h) * 0.5),
            )
        }
        "directional_blur_transition" => {
            let (dx, dy) = angle_dir(t.num("angle"));
            let len = t.px("blur") * env;
            blurs(1, vec![dx, dy, len, len * 0.5, k], super::taps(len * 2.0))
        }
        "lens_blur" => {
            let r = t.px("radius");
            let ra = r * smoothstep(0.0, 0.55, p);
            let rb = r * (1.0 - smoothstep(0.45, 1.0, p));
            let n = ((r * 0.8) as usize).clamp(8, 48);
            let (oa, ob) = (disc_offsets(ra, n), disc_offsets(rb, n));
            let mut numbers = vec![k, oa.len() as f32, ob.len() as f32, 0.0];
            for (x, y) in oa.iter().chain(&ob) {
                numbers.push(*x);
                numbers.push(*y);
            }
            blurs(2, numbers, 1)
        }
        "radial_blur" => {
            let (cx, cy) = t.center();
            let max = t.num("angle").to_radians();
            let ang = max * env;
            let (ra, rb) = (max * 0.25 * ease_in(p), -max * 0.25 * (1.0 - ease_out(p)));
            blurs(
                3,
                vec![cx, cy, ra, rb, ang, k],
                super::taps(ang * w.max(h) * 0.5),
            )
        }
        _ => return None,
    })
}

fn how_move(id: &str, t: &Tx) -> Option<How> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    let (dx, dy) = t.dir();
    let horiz = dx != 0.0;
    let travel = t.travel();
    let moves = |kind: u32, numbers: Vec<f32>| Pass::new(Entry::Moves, kind, numbers);
    Some(How::One(match id {
        "film_roll" => {
            let gap = t.px("gap");
            moves(
                0,
                with_color(
                    t.color("gap_color"),
                    &[dx, dy, travel + gap, travel, gap, truth(horiz)],
                ),
            )
            .with_time(super::shutter(t, 0.1))
        }
        "stretch" => {
            let e = ease_in_out(p);
            let (len, axis_x) = if horiz { (w, true) } else { (h, false) };
            let fwd = if horiz { dx > 0.0 } else { dy > 0.0 };
            let split = if fwd { len * e } else { len * (1.0 - e) };
            moves(1, vec![len, split, truth(axis_x), truth(fwd)])
        }
        "block_motion" => {
            let cols = t.num("blocks").round().clamp(2.0, 32.0);
            let rows = (cols * h / w).round().clamp(1.0, 32.0);
            let (lanes, cells) = if horiz { (rows, cols) } else { (cols, rows) };
            let (sh, n) = super::shutter(t, 0.08);
            let sgn = if horiz { dx } else { dy };
            moves(2, vec![lanes, cells, sgn, truth(horiz)]).with_time((sh, n.min(4)))
        }
        "frame" => {
            let fw = t.px("frame_width");
            let smin = t.frac("scale").clamp(0.05, 1.0);
            let shrink = smoothstep(0.0, 0.3, p) * (1.0 - smoothstep(0.7, 1.0, p));
            let s = 1.0 - (1.0 - smin) * shrink;
            let move_t = ease_in_out(((p - 0.25) / 0.5).clamp(0.0, 1.0));
            let border = fw * shrink;
            let step = travel * 0.5 * (1.0 + s) + border * 2.0 + 2.0;
            moves(
                3,
                with_color(
                    t.color("frame_color"),
                    &[s, border, dx, dy, move_t * step, (move_t - 1.0) * step],
                ),
            )
        }
        "louver" => moves(
            4,
            vec![
                t.num("slats").round().clamp(1.0, 64.0),
                truth(t.choice("orientation") == 0),
            ],
        ),
        "mirror_transition" => {
            let refl = travel * 0.3;
            let e = -refl + ease_in_out(p) * (travel + 2.0 * refl);
            moves(5, vec![truth(horiz), truth(dx + dy < 0.0), refl, e])
        }
        "page_peel" => {
            let r = t.px("radius").max(1.0);
            let (ox, oy) = match t.choice("corner") {
                0 => (0.0, 0.0),
                1 => (w, 0.0),
                3 => (0.0, h),
                _ => (w, h),
            };
            let (mut nx, mut ny) = (w / 2.0 - ox, h / 2.0 - oy);
            let nl = (nx * nx + ny * ny).sqrt().max(1e-3);
            nx /= nl;
            ny /= nl;
            let smax = (w - 2.0 * ox).abs() * nx.abs() + (h - 2.0 * oy).abs() * ny.abs();
            let f = -1.0 + p * (smax + r * 1.2 + 2.0);
            moves(6, vec![r, truth(t.flag("shadow")), ox, oy, nx, ny, f])
        }
        "slice" => {
            let count = t.num("slices").round().clamp(1.0, 64.0);
            let ang = t.num("angle").to_radians();
            let (ax, ay) = (ang.cos(), ang.sin());
            let (px_, py_) = (-ay, ax);
            let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (cx, cy) in corners {
                let v = cx * px_ + cy * py_;
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let span = (hi - lo).max(1.0);
            let dist = w.abs() * ax.abs() + h * ay.abs() + span * 0.2;
            moves(7, vec![count, ax, ay, px_, py_, lo, span, dist])
                .with_time(super::shutter(t, 0.08))
        }
        "wave" => {
            let band = travel * 0.25;
            let e = -band + p * (travel + 2.0 * band);
            moves(
                8,
                vec![
                    t.px("amplitude"),
                    t.px("wavelength").max(4.0),
                    truth(dx + dy < 0.0),
                    truth(horiz),
                    band,
                    e,
                    bell(p),
                ],
            )
        }
        "stretch_wipe" => {
            let s_len = t.px("stretch").max(1.0);
            let (lo, hi) = field_range(w, h, &|x: f32, y: f32| x * dx + y * dy);
            let e = lo - s_len + p * (hi - lo + 2.0 * s_len);
            moves(9, vec![dx, dy, s_len, e])
        }
        "earthquake" => {
            let shake = t.px("shake");
            moves(10, vec![shake, 0.04 * shake / w.max(1.0) * 10.0])
                .with_seed(t.seed())
                .with_time(super::shutter(t, 0.03))
        }
        _ => return None,
    }))
}

fn how_grunge(id: &str, t: &Tx) -> Option<How> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    let env = bell(p);
    let k = mid_mix(p);
    let s = t.seed();
    let u = t.unit().max(0.01);
    let grunge = |kind: u32, own: &[f32]| {
        let mut numbers = vec![env, k, u, 0.0];
        numbers.extend_from_slice(own);
        Pass::new(Entry::Grunge, kind, numbers).with_seed(s)
    };
    Some(How::One(match id {
        "chaos" => {
            let amt = t.frac("amount");
            grunge(0, &[amt, (p * 30.0).floor(), amt * env * w * 0.15])
        }
        "flicker" => {
            let nf = t.num("flickers").round().clamp(1.0, 40.0);
            let j = (p * nf * 2.0).floor() as i32;
            let show_b = hash(j, 3, s) < smoothstep(0.1, 0.9, p);
            let gain = 1.0 + (hash(j, 5, s) - 0.5) * 1.2 * env;
            grunge(1, &[truth(show_b), gain])
        }
        "glass" => grunge(2, &[t.px("cell_size").max(4.0), t.px("refraction")]),
        "glitch" => grunge(3, &[t.frac("amount"), (p * 24.0).floor()]),
        "kaleidoscope" => {
            let segments = t.num("segments").round().clamp(2.0, 32.0);
            grunge(
                4,
                &[std::f32::consts::TAU / segments, 1.0 + t.frac("zoom") * env],
            )
        }
        "liquid_distortion" => grunge(5, &[t.px("amount"), t.px("scale").max(4.0)]),
        "tv_power" => {
            let line = t.px("line").max(1.0);
            let (is_b, q) = if p < 0.5 {
                (false, p * 2.0)
            } else {
                (true, (1.0 - p) * 2.0)
            };
            let sy = 1.0 - smoothstep(0.0, 0.65, q);
            let sx = 1.0 - smoothstep(0.6, 1.0, q);
            let hy = (h / 2.0 * sy).max(line / 2.0 * sx);
            let hx = w / 2.0 * sx;
            let glow = t.color("glow_color");
            grunge(
                6,
                &[
                    glow[0],
                    glow[1],
                    glow[2],
                    glow[3],
                    truth(is_b),
                    sy,
                    sx,
                    hy,
                    hx,
                    smoothstep(0.2, 0.7, q),
                    line,
                ],
            )
        }
        "vhs_damage" => grunge(
            7,
            &[t.frac("amount"), (p * 30.0).floor(), h * (1.2 - 1.4 * p)],
        ),
        _ => return None,
    }))
}

/// The kernels, built once.
struct Stage {
    layout: wgpu::BindGroupLayout,
    wipes: wgpu::ComputePipeline,
    scene: wgpu::ComputePipeline,
    mixes: wgpu::ComputePipeline,
    blurs: wgpu::ComputePipeline,
    moves: wgpu::ComputePipeline,
    grunge: wgpu::ComputePipeline,
}

impl Stage {
    fn new(gpu: &Gpu) -> Option<Self> {
        let built = gpu.build(|device| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kerf-transitions"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{}{}",
                        include_str!("kernels.wgsl"),
                        kerf_kernel::wipe_wgsl()
                    )
                    .into(),
                ),
            });
            let layout = kerf_gpu::kernel_layout(device);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kerf-transitions"),
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
                wipes: kernel("wipes"),
                scene: kernel("scene"),
                mixes: kernel("mixes"),
                blurs: kernel("blurs"),
                moves: kernel("moves"),
                grunge: kernel("grunge"),
                layout,
            }
        });
        match built {
            Ok(stage) => Some(stage),
            Err(error) => {
                eprintln!("kerf-fx: the transitions' kernels did not build: {error}");
                None
            }
        }
    }

    fn pass(
        &self,
        frame: &mut Frame<'static>,
        pass: &Pass,
        p: f32,
        a: &Picture,
        b: Option<&Picture>,
    ) -> Picture {
        let (w, h) = (a.w, a.h);
        let out = frame.take(w, h);
        let pipeline = match pass.entry {
            Entry::Wipes => &self.wipes,
            Entry::Scene => &self.scene,
            Entry::Mixes => &self.mixes,
            Entry::Blurs => &self.blurs,
            Entry::Moves => &self.moves,
            Entry::Grunge => &self.grunge,
        };
        frame.dispatch(
            pipeline,
            &self.layout,
            &pass.bytes((w, h), p),
            (&a.view, &out.view, b.map(|picture| &picture.view)),
            (w.div_ceil(16), h.div_ceil(16)),
        );
        out
    }

    fn mix(&self, frame: &mut Frame<'static>, p: f32, a: &Picture, b: &Picture, k: f32) -> Picture {
        self.pass(frame, &Pass::new(Entry::Mixes, 0, vec![k]), p, a, Some(b))
    }
}

fn stage(gpu: &'static Gpu) -> Option<&'static Stage> {
    static STAGE: OnceLock<Option<Stage>> = OnceLock::new();
    STAGE.get_or_init(|| Stage::new(gpu)).as_ref()
}

/// Whether `gpu` can run these kernels, building them if that has not been done.
pub fn available(gpu: &'static Gpu) -> bool {
    stage(gpu).is_some()
}
