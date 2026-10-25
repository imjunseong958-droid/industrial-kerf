//! Color Correction, batch 2: Curves, Levels (Individual Controls), automatic corrections
//! (Auto Levels / Contrast / Color, Equalize), Shadow/Highlight, Selective Color, Color Balance
//! (HLS), Color Link, Broadcast Colors and the CC toners/offsets/kernel.

use effectcraft_color::luminance;
use crate::Value;
use crate::ParamUi;
use effectcraft_raster::Image;
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, lerp3, premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Color Correction", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(-100.0, 100.0, -100.0, 100.0, 1)
}

/// Map straight colour + alpha of every pixel (row-parallel).
fn map_ca(img: &mut Image, f: impl Fn([f32; 3], f32) -> ([f32; 3], f32) + Sync) {
    img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 && px[0] == 0.0 && px[1] == 0.0 && px[2] == 0.0 {
            // Fully transparent: only alpha transforms can change it.
            let (_, na) = f([0.0; 3], 0.0);
            if na > 0.0 {
                *px = [0.0, 0.0, 0.0, na.clamp(0.0, 1.0)];
            }
            return;
        }
        let (nc, na) = f(c, a);
        *px = premul(nc, na.clamp(0.0, 1.0));
    });
}

// ---------------------------------------------------------------------------------------------
// Curves

/// A tone curve through control points, interpolated with a monotone cubic (Fritsch–Carlson
/// 1980) Hermite spline and extended linearly outside the first/last point.
#[derive(Clone, Debug)]
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>,
    lut: Vec<f32>,
}

const LUT_N: usize = 1024;

impl Curve {
    /// Parse "x,y x,y …" (separators: whitespace or ';'). Returns None for an identity curve or
    /// fewer than two valid points.
    pub fn parse(s: &str) -> Option<Curve> {
        let mut pts: Vec<(f32, f32)> = s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|t| !t.is_empty())
            .filter_map(|t| {
                let mut it = t.split(',');
                let x = it.next()?.trim().parse::<f32>().ok()?;
                let y = it.next()?.trim().parse::<f32>().ok()?;
                (x.is_finite() && y.is_finite()).then_some((x, y))
            })
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if pts.len() < 2 || pts.iter().all(|(x, y)| (x - y).abs() < 1e-6) {
            return None;
        }
        let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
        let n = xs.len();
        let d: Vec<f32> = (0..n - 1).map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k])).collect();
        let mut ms = vec![0.0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            ms[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) * 0.5 };
        }
        for k in 0..n - 1 {
            if d[k].abs() < 1e-12 {
                ms[k] = 0.0;
                ms[k + 1] = 0.0;
            } else {
                let a = ms[k] / d[k];
                let b = ms[k + 1] / d[k];
                let s = a * a + b * b;
                if s > 9.0 {
                    let t = 3.0 / s.sqrt();
                    ms[k] = t * a * d[k];
                    ms[k + 1] = t * b * d[k];
                }
            }
        }
        let mut c = Curve { xs, ys, ms, lut: Vec::new() };
        c.lut = (0..=LUT_N).map(|i| c.eval_exact(i as f32 / LUT_N as f32)).collect();
        Some(c)
    }

    fn eval_exact(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.ms[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1] + (x - self.xs[n - 1]) * self.ms[n - 1];
        }
        let k = self.xs.partition_point(|&v| v <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[k] + h10 * h * self.ms[k] + h01 * self.ys[k + 1] + h11 * h * self.ms[k + 1]
    }

    pub fn eval(&self, x: f32) -> f32 {
        if !(0.0..=1.0).contains(&x) {
            return self.eval_exact(x);
        }
        let f = x * LUT_N as f32;
        let i = (f as usize).min(LUT_N - 1);
        let t = f - i as f32;
        self.lut[i] + (self.lut[i + 1] - self.lut[i]) * t
    }
}

fn ev(c: &Option<Curve>, v: f32) -> f32 {
    c.as_ref().map(|c| c.eval(v)).unwrap_or(v)
}

/// Curves. Each channel's curve is a hidden string parameter (`rgb`, `red`, `green`, `blue`,
/// `alpha`) holding control points as `"x,y x,y …"` in 0..1 (whitespace or `;` separated,
/// sorted on load); the default `"0,0 1,1"` is the identity. Colour channels go through the
/// master RGB curve first, then their own curve.
fn curves(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let [m, r, g, bl, a] = ["rgb", "red", "green", "blue", "alpha"].map(|id| Curve::parse(ctx.params.s(id)));
    if m.is_none() && r.is_none() && g.is_none() && bl.is_none() && a.is_none() {
        return b;
    }
    let chans = [r, g, bl];
    map_ca(&mut b.img, |c, al| {
        let o = [0, 1, 2].map(|i| ev(&chans[i], ev(&m, c[i])));
        (o, ev(&a, al))
    });
    b
}

// ---------------------------------------------------------------------------------------------
// Levels (Individual Controls)

/// Levels (Individual Controls) parameter ids: each channel's controls sit in its own
/// twirl-down group (RGB, Red, Green, Blue, Alpha).
const LV_IDS: [[&str; 5]; 5] = [
    ["rgb/rgbInBlack", "rgb/rgbInWhite", "rgb/rgbGamma", "rgb/rgbOutBlack", "rgb/rgbOutWhite"],
    ["red/redInBlack", "red/redInWhite", "red/redGamma", "red/redOutBlack", "red/redOutWhite"],
    ["green/greenInBlack", "green/greenInWhite", "green/greenGamma", "green/greenOutBlack", "green/greenOutWhite"],
    ["blue/blueInBlack", "blue/blueInWhite", "blue/blueGamma", "blue/blueOutBlack", "blue/blueOutWhite"],
    ["alpha/alphaInBlack", "alpha/alphaInWhite", "alpha/alphaGamma", "alpha/alphaOutBlack", "alpha/alphaOutWhite"],
];
const LV_NAMES: [[&str; 5]; 5] = [
    ["Input Black", "Input White", "Gamma", "Output Black", "Output White"],
    ["Red Input Black", "Red Input White", "Red Gamma", "Red Output Black", "Red Output White"],
    ["Green Input Black", "Green Input White", "Green Gamma", "Green Output Black", "Green Output White"],
    ["Blue Input Black", "Blue Input White", "Blue Gamma", "Blue Output Black", "Blue Output White"],
    ["Alpha Input Black", "Alpha Input White", "Alpha Gamma", "Alpha Output Black", "Alpha Output White"],
];

// ---------------------------------------------------------------------------------------------
// Histograms and automatic corrections

const BINS: usize = 1024;

/// Alpha-weighted histograms of straight R, G, B and luminance.
fn histograms(img: &Image) -> [Vec<f64>; 4] {
    img.data
        .par_chunks(4096)
        .map(|chunk| {
            let mut h: [Vec<f64>; 4] = std::array::from_fn(|_| vec![0.0; BINS]);
            for &px in chunk {
                let (c, a) = unpremul(px);
                if a <= 1e-4 {
                    continue;
                }
                let bin = |v: f32| ((v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize).min(BINS - 1);
                for k in 0..3 {
                    h[k][bin(c[k])] += a as f64;
                }
                h[3][bin(luminance(c[0], c[1], c[2]))] += a as f64;
            }
            h
        })
        .reduce(
            || std::array::from_fn(|_| vec![0.0; BINS]),
            |mut a, b| {
                for k in 0..4 {
                    for i in 0..BINS {
                        a[k][i] += b[k][i];
                    }
                }
                a
            },
        )
}

/// (low, high) values clipping `black`/`white` fractions of the histogram.
fn clip_points(h: &[f64], black: f64, white: f64) -> (f32, f32) {
    let total: f64 = h.iter().sum();
    if total <= 0.0 {
        return (0.0, 1.0);
    }
    let mut acc = 0.0;
    let mut lo = 0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc > total * black {
            lo = i;
            break;
        }
    }
    acc = 0.0;
    let mut hi = BINS - 1;
    for (i, v) in h.iter().enumerate().rev() {
        acc += v;
        if acc > total * white {
            hi = i;
            break;
        }
    }
    let (lo, hi) = (lo as f32 / (BINS - 1) as f32, hi as f32 / (BINS - 1) as f32);
    if hi - lo < 1e-3 { (0.0, 1.0) } else { (lo, hi) }
}

/// Alpha-weighted mean (0..1) of a histogram.
fn hist_mean(h: &[f64]) -> f64 {
    let (mut s, mut w) = (0.0, 0.0);
    for (i, v) in h.iter().enumerate() {
        s += v * i as f64;
        w += v;
    }
    if w > 0.0 { s / w / (BINS - 1) as f64 } else { 0.5 }
}

/// The histograms the automatic corrections analyse: the current frame's, or with Temporal
/// Smoothing (seconds) the sum over the frames within that range on either side. Scene Detect
/// stops at a frame whose brightness jumps (a cut).
fn smoothed_histograms(ctx: &EffectCtx, img: &Image) -> [Vec<f64>; 4] {
    let mut h = histograms(img);
    let secs = ctx.params.f("temporalSmoothing");
    let Some(host) = ctx.env.host.filter(|_| secs > 0.0) else { return h };
    let fps = ctx.fps();
    let n = (secs * fps).round().clamp(0.0, 240.0) as i64;
    let scene = ctx.params.b("sceneDetect");
    let m0 = hist_mean(&h[3]);
    for dir in [-1i64, 1] {
        for k in 1..=n {
            let Some(nb) = host.self_at(ctx.time + (dir * k) as f64 / fps, ctx.env.effect_index) else { break };
            let hn = histograms(&nb.img);
            if scene && (hist_mean(&hn[3]) - m0).abs() > 0.15 {
                break;
            }
            for c in 0..4 {
                for i in 0..BINS {
                    h[c][i] += hn[c][i];
                }
            }
        }
    }
    h
}

fn stretch(v: f32, lo: f32, hi: f32) -> f32 {
    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// 0 = Auto Levels (per channel), 1 = Auto Contrast (shared), 2 = Auto Color (per channel + neutral mids).
fn auto_correct(ctx: &EffectCtx, mut b: Buf, kind: u32) -> Buf {
    let blend = (ctx.params.f("blend") / 100.0).clamp(0.0, 1.0) as f32;
    let (ranges, gam) = auto_correct_settings(ctx, &b.img, kind);
    map_ca(&mut b.img, |c, a| {
        let o = [0, 1, 2].map(|k| stretch(c[k], ranges[k].0, ranges[k].1).powf(gam[k]));
        (lerp3(o, c, blend), a)
    });
    b
}

/// What Auto Levels (`kind` 0), Auto Contrast (1) and Auto Color (2) measure on `img`: each
/// channel's input range (low, high) and gamma. (The GPU effects apply them.)
pub fn auto_correct_settings(ctx: &EffectCtx, img: &Image, kind: u32) -> ([(f32, f32); 3], [f32; 3]) {
    let black = ctx.params.f("blackClip").clamp(0.0, 49.0) / 100.0;
    let white = ctx.params.f("whiteClip").clamp(0.0, 49.0) / 100.0;
    let h = smoothed_histograms(ctx, img);
    let ranges: [(f32, f32); 3] = if kind == 1 {
        let mut combined = vec![0.0; BINS];
        for k in 0..3 {
            for i in 0..BINS {
                combined[i] += h[k][i];
            }
        }
        let r = clip_points(&combined, black, white);
        [r; 3]
    } else {
        [0, 1, 2].map(|k| clip_points(&h[k], black, white))
    };
    // Auto Color: per-channel gamma that moves each channel's mean to the common mean.
    let mut gam = [1.0f32; 3];
    if kind == 2 && ctx.params.b("snapNeutralMidtones") {
        let mut sums = [0.0f64; 4];
        for px in &img.data {
            let (c, a) = unpremul(*px);
            if a > 1e-4 {
                for k in 0..3 {
                    sums[k] += (stretch(c[k], ranges[k].0, ranges[k].1) * a) as f64;
                }
                sums[3] += a as f64;
            }
        }
        if sums[3] > 0.0 {
            let means = [0, 1, 2].map(|k| (sums[k] / sums[3]) as f32);
            let target = (means[0] + means[1] + means[2]) / 3.0;
            for k in 0..3 {
                if means[k] > 1e-3 && means[k] < 0.999 && target > 1e-3 && target < 0.999 {
                    gam[k] = (target.ln() / means[k].ln()).clamp(0.2, 5.0);
                }
            }
        }
    }
    (ranges, gam)
}

fn auto_levels(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 0)
}
fn auto_contrast(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 1)
}
fn auto_color(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 2)
}

// ---------------------------------------------------------------------------------------------
// Shadow/Highlight

/// Shadow/Highlight's shadow and highlight amounts for `img` (Auto Amounts measures them).
pub fn shadow_highlight_amounts(ctx: &EffectCtx, img: &Image) -> (f32, f32) {
    let (mut s_amt, mut h_amt) = (ctx.params.f("shadowAmount") as f32 / 100.0, ctx.params.f("highlightAmount") as f32 / 100.0);
    if ctx.params.b("autoAmounts") {
        // Temporal Smoothing / Scene Detect average the analysis over neighbouring frames.
        let mean = if ctx.params.f("temporalSmoothing") > 0.0 { hist_mean(&smoothed_histograms(ctx, img)[3]) } else { hist_mean(&histograms(img)[3]) } as f32;
        s_amt = ((0.6 - mean) * 1.5).clamp(0.0, 1.0) * 0.8 + 0.1;
        h_amt = ((mean - 0.6) * 1.5).clamp(0.0, 1.0) * 0.8;
    }
    (s_amt, h_amt)
}

/// The luminance values clipping `black` / `white` fractions of `img`'s alpha-weighted
/// luminance histogram (Shadow/Highlight's Black / White Clip).
pub fn luma_clip_points(img: &Image, black: f64, white: f64) -> (f32, f32) {
    clip_points(&histograms(img)[3], black, white)
}

fn shadow_highlight(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let (s_amt, h_amt) = shadow_highlight_amounts(ctx, &b.img);
    let luma = Plane::luma(&b.img);
    let (bc, wc) = (ctx.params.f("moreOptions/blackClip").clamp(0.0, 49.0) / 100.0, ctx.params.f("moreOptions/whiteClip").clamp(0.0, 49.0) / 100.0);
    let clip0 = (bc > 0.0 || wc > 0.0).then(|| luma_clip_points(&b.img, bc, wc));
    let s_tw = (ctx.params.f("moreOptions/shadowTonalWidth") as f32 / 100.0).max(0.01);
    let h_tw = (ctx.params.f("moreOptions/highlightTonalWidth") as f32 / 100.0).max(0.01);
    let s_r = ctx.params.f("moreOptions/shadowRadius").max(0.0) * b.scale / 2.0;
    let h_r = ctx.params.f("moreOptions/highlightRadius").max(0.0) * b.scale / 2.0;
    let cc = ctx.params.f("moreOptions/colorCorrection") as f32 / 100.0;
    let mc = ctx.params.f("moreOptions/midtoneContrast") as f32 / 100.0;
    let blend = (ctx.params.f("blend") / 100.0).clamp(0.0, 1.0) as f32;
    let base_s = gauss_plane(&luma, s_r, s_r);
    let base_h = if (h_r - s_r).abs() < 1e-9 { base_s.clone() } else { gauss_plane(&luma, h_r, h_r) };
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        let ws = (1.0 - base_s.data[i] / s_tw).clamp(0.0, 1.0).powi(2);
        let wh = ((base_h.data[i] - (1.0 - h_tw)) / h_tw).clamp(0.0, 1.0).powi(2);
        let gain = (1.0 + s_amt * ws * 2.0) * (1.0 - h_amt * wh * 0.5);
        let l = luminance(c[0], c[1], c[2]);
        let nl = l * gain;
        let sat = gain.max(1e-3).powf(cc);
        let mut o = c.map(|v| nl + (v - l) * sat);
        if mc != 0.0 {
            let shift = (0.5 + (nl - 0.5) * (1.0 + mc)) - nl;
            o = o.map(|v| v + shift);
        }
        let o = lerp3(o.map(|v| v.max(0.0)), c, blend);
        *px = premul(o, a);
    });
    // Black / White Clip: the adjusted image's extremes (ignoring those fractions of pixels)
    // are mapped back onto the original's, so lifting shadows or recovering highlights doesn't
    // flatten the ends of the tonal range.
    if let Some((lo0, hi0)) = clip0 {
        let (lo1, hi1) = luma_clip_points(&b.img, bc, wc);
        if (lo0, hi0) != (lo1, hi1) && hi1 - lo1 > 1e-3 {
            let k = (hi0 - lo0) / (hi1 - lo1);
            map_ca(&mut b.img, |c, a| (c.map(|v| (lo0 + (v - lo1) * k).max(0.0)), a));
        }
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Selective Color

const SC_IDS: [[&str; 4]; 9] = [
    ["redsCyan", "redsMagenta", "redsYellow", "redsBlack"],
    ["yellowsCyan", "yellowsMagenta", "yellowsYellow", "yellowsBlack"],
    ["greensCyan", "greensMagenta", "greensYellow", "greensBlack"],
    ["cyansCyan", "cyansMagenta", "cyansYellow", "cyansBlack"],
    ["bluesCyan", "bluesMagenta", "bluesYellow", "bluesBlack"],
    ["magentasCyan", "magentasMagenta", "magentasYellow", "magentasBlack"],
    ["whitesCyan", "whitesMagenta", "whitesYellow", "whitesBlack"],
    ["neutralsCyan", "neutralsMagenta", "neutralsYellow", "neutralsBlack"],
    ["blacksCyan", "blacksMagenta", "blacksYellow", "blacksBlack"],
];
/// Selective Color's Colors popup (the range whose controls Effect Controls shows) and the
/// Details twirl-down group of each range.
pub const SC_COLORS: [&str; 9] = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas", "Whites", "Neutrals", "Blacks"];
pub const SC_GROUPS: [&str; 9] = ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"];

/// Spec id of range `r`'s control `leaf` (`details/reds/redsCyan`).
fn sc_param_id(r: usize, leaf: &str) -> String {
    format!("details/{}/{leaf}", SC_GROUPS[r])
}

const SC_NAMES: [[&str; 4]; 9] = [
    ["Reds Cyan", "Reds Magenta", "Reds Yellow", "Reds Black"],
    ["Yellows Cyan", "Yellows Magenta", "Yellows Yellow", "Yellows Black"],
    ["Greens Cyan", "Greens Magenta", "Greens Yellow", "Greens Black"],
    ["Cyans Cyan", "Cyans Magenta", "Cyans Yellow", "Cyans Black"],
    ["Blues Cyan", "Blues Magenta", "Blues Yellow", "Blues Black"],
    ["Magentas Cyan", "Magentas Magenta", "Magentas Yellow", "Magentas Black"],
    ["Whites Cyan", "Whites Magenta", "Whites Yellow", "Whites Black"],
    ["Neutrals Cyan", "Neutrals Magenta", "Neutrals Yellow", "Neutrals Black"],
    ["Blacks Cyan", "Blacks Magenta", "Blacks Yellow", "Blacks Black"],
];

/// How much a colour belongs to each Selective Color range.
fn sc_weights(c: [f32; 3]) -> [f32; 9] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[b].total_cmp(&c[a]));
    let (mx, md, mn) = (c[idx[0]], c[idx[1]], c[idx[2]]);
    let mut w = [0.0f32; 9];
    // Primaries: weight = max − mid when that primary is the largest component.
    w[[0, 2, 4][idx[0]]] = mx - md;
    // Secondaries: weight = mid − min when the complementary primary is the smallest.
    w[[3, 5, 1][idx[2]]] = md - mn;
    w[6] = ((mn - 0.5) * 2.0).max(0.0);
    w[8] = ((0.5 - mx) * 2.0).max(0.0);
    w[7] = (1.0 - ((mx - 0.5).abs() + (mn - 0.5).abs())).clamp(0.0, 1.0);
    w
}

fn selective_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let adj: [[f32; 4]; 9] = std::array::from_fn(|r| SC_IDS[r].map(|id| ctx.params.f(&sc_param_id(r, id)) as f32 / 100.0));
    if adj.iter().all(|r| r.iter().all(|v| *v == 0.0)) {
        return b;
    }
    let relative = ctx.params.e("method") == 0;
    map_ca(&mut b.img, |c, a| {
        let w = sc_weights(c);
        let mut o = c;
        for (r, wr) in w.iter().enumerate() {
            if *wr <= 0.0 {
                continue;
            }
            let k = adj[r][3];
            for i in 0..3 {
                let ink = 1.0 - c[i];
                let delta = adj[r][i] + k;
                let delta = if relative { delta * ink.max(0.0) } else { delta };
                o[i] -= wr * delta;
            }
        }
        (o.map(|v| v.max(0.0)), a)
    });
    b
}

// ---------------------------------------------------------------------------------------------
// Color Balance (HLS), Color Link, Broadcast Colors

// ---------------------------------------------------------------------------------------------
// CC Toner, CC Color Offset, CC Kernel

const K_IDS: [&str; 9] = ["k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9"];
const K_NAMES: [&str; 9] = ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"];

pub fn specs() -> Vec<EffectSpec> {
    let mut lv_params = Vec::new();
    for (ids, names) in LV_IDS.iter().zip(LV_NAMES.iter()) {
        lv_params.push(p(ids[0], names[0], num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[1], names[1], num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[2], names[2], num(1.0), slider(0.1, 10.0, 0.1, 3.0, 2)));
        lv_params.push(p(ids[3], names[3], num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[4], names[4], num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
    }
    lv_params.push(p("clipToOutputBlack", "Clip To Output Black", Value::Enum(2), popup(&crate::color_fx::LEVELS_CLIP)));
    lv_params.push(p("clipToOutputWhite", "Clip To Output White", Value::Enum(2), popup(&crate::color_fx::LEVELS_CLIP)));
    let mut sc_params = vec![p("method", "Method", Value::Enum(0), popup(&["Relative", "Absolute"])), p("colors", "Colors", Value::Enum(0), popup(&SC_COLORS))];
    // Details: every range's Cyan / Magenta / Yellow / Black in its own twirl-down.
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    for (r, (ids, names)) in SC_IDS.iter().zip(SC_NAMES.iter()).enumerate() {
        for k in 0..4 {
            sc_params.push(p(leak(sc_param_id(r, ids[k])), names[k], num(0.0), pct()));
        }
    }
    let clip_params = |extra: bool| {
        let mut v = vec![
            p("temporalSmoothing", "Temporal Smoothing (seconds)", num(0.0), slider(0.0, 4.0, 0.0, 4.0, 2)),
            p("sceneDetect", "Scene Detect", Value::Bool(false), ParamUi::Checkbox),
            p("blackClip", "Black Clip", num(0.1), slider(0.0, 49.0, 0.0, 10.0, 2)),
            p("whiteClip", "White Clip", num(0.1), slider(0.0, 49.0, 0.0, 10.0, 2)),
            p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
        ];
        if extra {
            v.insert(4, p("snapNeutralMidtones", "Snap Neutral Midtones", Value::Bool(true), ParamUi::Checkbox));
        }
        v
    };
    let mut k_params: Vec<crate::ParamSpec> =
        K_IDS.iter().zip(K_NAMES.iter()).map(|(id, n)| p(id, n, num(if *id == "k5" { 1.0 } else { 0.0 }), slider(-100.0, 100.0, -10.0, 10.0, 2))).collect();
    k_params.push(p("scale", "Scale", num(1.0), slider(-100.0, 100.0, 0.0, 4.0, 3)));
    k_params.push(p("offset", "Offset", num(0.0), slider(-1.0, 1.0, -1.0, 1.0, 3)));
    k_params.push(p("absolute", "Absolute Value", Value::Bool(false), ParamUi::Checkbox));
    // Each curve is its points, "x,y x,y …" from 0 to 1.
    let curve = |id: &'static str, name: &'static str| p(id, name, Value::Str("0,0 1,1".into()), ParamUi::Text);
    vec![
        spec(
            "ec.color.curves",
            "Curves",
            vec![
                // The channel the graph edits (the Effect Controls graph's Channel popup).
                p("channel", "Channel", Value::Enum(0), ParamUi::Hidden),
                curve("rgb", "RGB"),
                curve("red", "Red"),
                curve("green", "Green"),
                curve("blue", "Blue"),
                curve("alpha", "Alpha"),
            ],
            curves,
        ),
        spec("ec.color.autolevels", "Auto Levels", clip_params(false), auto_levels),
        spec("ec.color.autocontrast", "Auto Contrast", clip_params(false), auto_contrast),
        spec("ec.color.autocolor", "Auto Color", clip_params(true), auto_color),
        spec(
            "ec.color.shadowhighlight",
            "Shadow/Highlight",
            vec![
                p("autoAmounts", "Auto Amounts", Value::Bool(true), ParamUi::Checkbox),
                p("shadowAmount", "Shadow Amount", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("highlightAmount", "Highlight Amount", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("temporalSmoothing", "Temporal Smoothing (seconds)", num(0.0), slider(0.0, 4.0, 0.0, 4.0, 2)),
                p("sceneDetect", "Scene Detect", Value::Bool(false), ParamUi::Checkbox),
                p("moreOptions/shadowTonalWidth", "Shadow Tonal Width", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("moreOptions/shadowRadius", "Shadow Radius", num(30.0), slider(0.0, 2500.0, 0.0, 100.0, 0)),
                p("moreOptions/highlightTonalWidth", "Highlight Tonal Width", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("moreOptions/highlightRadius", "Highlight Radius", num(30.0), slider(0.0, 2500.0, 0.0, 100.0, 0)),
                p("moreOptions/colorCorrection", "Color Correction", num(20.0), slider(-100.0, 100.0, -100.0, 100.0, 0)),
                p("moreOptions/midtoneContrast", "Midtone Contrast", num(0.0), pct()),
                p("moreOptions/blackClip", "Black Clip", num(0.01), slider(0.0, 49.0, 0.0, 10.0, 2)),
                p("moreOptions/whiteClip", "White Clip", num(0.01), slider(0.0, 49.0, 0.0, 10.0, 2)),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            shadow_highlight,
        ),
        spec("ec.color.selectivecolor", "Selective Color", sc_params, selective_color),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, find};

    fn run(id: &str, img: &Image, over: &[(&str, Value)]) -> Image {
        let s = find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in over {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx =
            EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: Default::default() };
        (s.render)(&ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 }).img
    }

    fn ramp() -> Image {
        let mut img = Image::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                let v = x as f32 / 15.0;
                let a = if y == 0 { 0.5 } else { 1.0 };
                img.set(x, y, [v * a, (1.0 - v) * a, 0.3 * a, a]);
            }
        }
        img
    }

    fn close(a: &Image, b: &Image, eps: f32) -> bool {
        a.data.iter().zip(&b.data).all(|(p, q)| (0..4).all(|i| (p[i] - q[i]).abs() <= eps))
    }

    #[test]
    fn curve_parsing() {
        assert!(Curve::parse("0,0 1,1").is_none());
        assert!(Curve::parse("garbage").is_none());
        let c = Curve::parse("1,1; 0,0 0.5,0.7").unwrap();
        assert_eq!(c.xs, vec![0.0, 0.5, 1.0]);
        assert!((c.eval(0.5) - 0.7).abs() < 1e-3);
        // Monotone: never decreases.
        let mut last = -1.0;
        for i in 0..=100 {
            let v = c.eval(i as f32 / 100.0);
            assert!(v >= last - 1e-6);
            last = v;
        }
    }

    #[test]
    fn curves_identity_is_identity() {
        let img = ramp();
        assert_eq!(run("ec.color.curves", &img, &[]), img);
        let explicit = run("ec.color.curves", &img, &[("rgb", Value::Str("0,0 0.25,0.25 1,1".into()))]);
        assert!(close(&explicit, &img, 1e-4));
    }

    #[test]
    fn curves_midpoint_brightens() {
        let img = Image::filled(4, 4, [0.5, 0.5, 0.5, 1.0]);
        let out = run("ec.color.curves", &img, &[("rgb", Value::Str("0,0 0.5,0.75 1,1".into()))]);
        assert!((out.data[0][0] - 0.75).abs() < 1e-3);
        let red = run("ec.color.curves", &img, &[("red", Value::Str("0,1 1,1".into()))]);
        assert!((red.data[0][0] - 1.0).abs() < 1e-4 && (red.data[0][1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn auto_levels_stretches_low_contrast() {
        let mut img = Image::new(32, 4);
        for y in 0..4 {
            for x in 0..32 {
                let v = 0.4 + 0.2 * x as f32 / 31.0;
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = run("ec.color.autolevels", &img, &[("blackClip", num(0.0)), ("whiteClip", num(0.0))]);
        let lo = out.data.iter().map(|p| p[0]).fold(1.0, f32::min);
        let hi = out.data.iter().map(|p| p[0]).fold(0.0, f32::max);
        assert!(lo < 0.02 && hi > 0.98, "{lo} {hi}");
        let ac = run("ec.color.autocontrast", &img, &[("blackClip", num(0.0)), ("whiteClip", num(0.0))]);
        assert!(ac.data.iter().map(|p| p[0]).fold(0.0, f32::max) > 0.98);
    }

    #[test]
    fn auto_color_neutralises_cast() {
        let mut img = Image::new(16, 4);
        for y in 0..4 {
            for x in 0..16 {
                let v = x as f32 / 15.0;
                img.set(x, y, [v * 0.6 + 0.1, v * 0.9, v * 0.5, 1.0]);
            }
        }
        let out = run("ec.color.autocolor", &img, &[]);
        let p = out.get(8, 1);
        assert!((p[0] - p[1]).abs() < 0.08 && (p[1] - p[2]).abs() < 0.08, "{p:?}");
    }

    #[test]
    fn selective_color_zero_is_identity_and_reds_respond() {
        let img = ramp();
        assert_eq!(run("ec.color.selectivecolor", &img, &[]), img);
        let red = Image::filled(2, 2, [1.0, 0.1, 0.1, 1.0]);
        let out = run("ec.color.selectivecolor", &red, &[("details/reds/redsCyan", num(100.0)), ("method", Value::Enum(1))]);
        assert!(out.data[0][0] < 0.3, "{:?}", out.data[0]);
        let blue = Image::filled(2, 2, [0.1, 0.1, 1.0, 1.0]);
        assert_eq!(run("ec.color.selectivecolor", &blue, &[("details/reds/redsCyan", num(100.0))]), blue);
    }

    #[test]
    fn shadow_highlight_lifts_shadows() {
        let img = Image::filled(8, 8, [0.1, 0.1, 0.1, 1.0]);
        let out = run("ec.color.shadowhighlight", &img, &[("autoAmounts", Value::Bool(false))]);
        assert!(out.data[0][0] > 0.15);
    }

    /// A host whose other layers and other frames are a fixed image.
    struct ImgHost(Image);
    impl crate::EffectHost for ImgHost {
        fn layer(&self, _: u64, _: bool) -> Option<crate::LayerPixels> {
            Some(crate::LayerPixels { buf: Buf { img: self.0.clone(), offset: [0.0; 2], scale: 1.0 }, size: [2.0, 2.0] })
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
        fn self_at(&self, _: f64, _: usize) -> Option<Buf> {
            Some(Buf { img: self.0.clone(), offset: [0.0; 2], scale: 1.0 })
        }
    }

    fn run_host(id: &str, img: &Image, over: &[(&str, Value)], host: &ImgHost) -> Image {
        let env = crate::EffectEnv { host: Some(host), frame_rate: 10.0, ..Default::default() };
        crate::run_fx(id, over, img.clone(), 1.0, env).img
    }

    #[test]
    fn auto_levels_temporal_smoothing_reads_neighbour_frames() {
        let mut img = Image::new(4, 1);
        for x in 0..4 {
            let v = 0.3 + 0.1 * x as f32;
            img.set(x, 0, [v, v, v, 1.0]);
        }
        let wide = ImgHost(ramp());
        let p0 = [("blackClip", num(0.0)), ("whiteClip", num(0.0))];
        let alone = run_host("ec.color.autolevels", &img, &p0, &wide);
        let smooth = run_host("ec.color.autolevels", &img, &[p0[0].clone(), p0[1].clone(), ("temporalSmoothing", num(0.2))], &wide);
        // Alone the frame is stretched to full range; with full-range neighbours much less.
        assert!(alone.data[0][0] < 0.01);
        assert!(smooth.data[0][0] > 0.1, "{:?}", smooth.data[0]);
    }

    #[test]
    fn shadow_highlight_clip_keeps_the_tonal_range() {
        let img = ramp();
        let lift = [("autoAmounts", Value::Bool(false)), ("shadowAmount", num(80.0))];
        let unclipped = run(
            "ec.color.shadowhighlight",
            &img,
            &[lift[0].clone(), lift[1].clone(), ("moreOptions/blackClip", num(0.0)), ("moreOptions/whiteClip", num(0.0))],
        );
        let clipped = run("ec.color.shadowhighlight", &img, &[lift[0].clone(), lift[1].clone(), ("moreOptions/blackClip", num(1.0))]);
        let min = |im: &Image| im.data.iter().filter(|p| p[3] > 0.99).map(|p| luminance(p[0], p[1], p[2])).fold(f32::MAX, f32::min);
        assert!(min(&clipped) < min(&unclipped));
    }
}
