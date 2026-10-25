//! Generate effects, batch 3: Fractal (Mandelbrot/Julia), mask-driven Scribble and Stroke,
//! Vegas (animated segments along image contours or masks), Write-on, Paint Bucket (flood
//! fill), Eyedropper Fill, CC Glue Gun, CC Threads, and the audio visualisers Audio Spectrum
//! and Audio Waveform (fed by the host's audio).
//!
//! Also home of the shared anti-aliased segment rasteriser ([`raster_segs`]) used by the text
//! effects.

use crate::Value;
use crate::ParamUi;
use effectcraft_raster::Px;
use rayon::prelude::*;
use std::f64::consts::PI;

use crate::util::{Plane, poly_length, premul, smoothstep};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Generate", params, render, gpu: false, float: true }
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

// ---------------------------------------------------------------- shared rasterisation

/// A round-capped line segment in buffer pixels with radius `r` and intensity `v`.
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub r: f64,
    pub v: f32,
}

/// Rasterise segments into a coverage plane (max-combined). `hardness` (0..1) is the fraction
/// of the radius that is fully opaque; the rest falls off smoothly.
pub(crate) fn raster_segs(w: usize, h: usize, segs: &[Seg], hardness: f64) -> Plane {
    let mut out = Plane::new(w, h);
    if w == 0 || h == 0 || segs.is_empty() {
        return out;
    }
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); h];
    for (i, s) in segs.iter().enumerate() {
        let pad = s.r + 1.0;
        let y0 = (s.a[1].min(s.b[1]) - pad).floor().max(0.0) as usize;
        let y1 = (s.a[1].max(s.b[1]) + pad).ceil().min(h as f64);
        if y1 <= 0.0 || y0 >= h {
            continue;
        }
        for row in buckets.iter_mut().take(y1 as usize).skip(y0) {
            row.push(i as u32);
        }
    }
    let hard = hardness.clamp(0.0, 1.0);
    out.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for &i in &buckets[y] {
            let s = segs[i as usize];
            let pad = s.r + 1.0;
            let x0 = (s.a[0].min(s.b[0]) - pad).floor().max(0.0) as usize;
            let x1 = ((s.a[0].max(s.b[0]) + pad).ceil().max(0.0) as usize).min(w);
            let (dx, dy) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
            let l2 = dx * dx + dy * dy;
            let inner = s.r * hard - 0.5;
            let outer = s.r + 0.5;
            for (x, o) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let px = x as f64 + 0.5;
                let t = if l2 > 1e-12 { (((px - s.a[0]) * dx + (py - s.a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let d = ((px - s.a[0] - dx * t).powi(2) + (py - s.a[1] - dy * t).powi(2)).sqrt();
                if d >= outer {
                    continue;
                }
                let c = if d <= inner { 1.0 } else { 1.0 - smoothstep(inner as f32, outer as f32, d as f32) } * s.v;
                if c > *o {
                    *o = c;
                }
            }
        }
    });
    out
}

/// Segments along a polyline (buffer px).
pub fn poly_segs(pts: &[[f64; 2]], closed: bool, r: f64, v: f32, out: &mut Vec<Seg>) {
    let n = pts.len();
    if n == 1 {
        out.push(Seg { a: pts[0], b: pts[0], r, v });
    }
    if n < 2 {
        return;
    }
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        out.push(Seg { a: pts[i], b: pts[(i + 1) % n], r, v });
    }
}

#[inline]
fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// Mask `idx` (1-based; 0 = none) mapped to buffer pixels.
pub(crate) fn mask_px(ctx: &EffectCtx, b: &Buf, idx: usize) -> Option<(Vec<[f64; 2]>, bool, bool)> {
    let m = ctx.env.masks.get(idx.checked_sub(1)?)?;
    if m.points.is_empty() {
        return None;
    }
    Some((m.points.iter().map(|&q| [q[0] * b.scale + b.offset[0], q[1] * b.scale + b.offset[1]]).collect(), m.closed, m.inverted))
}

// ---------------------------------------------------------------- Fractal

// ---------------------------------------------------------------- Scribble

// ---------------------------------------------------------------- Stroke

// ---------------------------------------------------------------- Vegas

// ---------------------------------------------------------------- Write-on / Glue Gun

// ---------------------------------------------------------------- Paint Bucket

// ---------------------------------------------------------------- Eyedropper Fill

// ---------------------------------------------------------------- CC Threads

// ---------------------------------------------------------------- Audio Spectrum / Waveform

/// In-place iterative radix-2 FFT (`re`, `im` lengths a power of two).
pub(crate) fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    if n < 2 {
        return;
    }
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let (ws, wc) = ang.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, bb) = (start + k, start + k + len / 2);
                let tr = re[bb] * cr - im[bb] * ci;
                let ti = re[bb] * ci + im[bb] * cr;
                re[bb] = re[a] - tr;
                im[bb] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wc - ci * ws;
                ci = cr * ws + ci * wc;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

const AUDIO_RATE: u32 = 48_000;

/// Fetch mono (or one channel) samples of the audio layer around the current time.
fn audio_window(ctx: &EffectCtx, dur_ms: f64, offset_ms: f64, channel: u32) -> Option<Vec<f64>> {
    let id = ctx.params.get("audioLayer")?.as_layer()?;
    let host = ctx.env.host?;
    let dur = (dur_ms / 1000.0).max(0.001);
    let frames = ((dur * AUDIO_RATE as f64) as usize).clamp(16, 1 << 16);
    let start = ctx.env.comp_time + offset_ms / 1000.0 - dur * 0.5;
    let s = host.audio(id, start, frames, AUDIO_RATE)?;
    Some(
        (0..frames)
            .map(|i| {
                let (l, r) = (*s.get(2 * i).unwrap_or(&0.0) as f64, *s.get(2 * i + 1).unwrap_or(&0.0) as f64);
                match channel {
                    1 => l,
                    2 => r,
                    _ => (l + r) * 0.5,
                }
            })
            .collect(),
    )
}

/// The drawing baseline: a polyline in buffer px (mask path or start→end line), sampled at
/// parameter u∈[0,1] → (point, unit normal).
struct Baseline {
    pts: Vec<[f64; 2]>,
    closed: bool,
    len: f64,
    polar: bool,
    center: [f64; 2],
    radius: f64,
}

impl Baseline {
    fn new(ctx: &EffectCtx, b: &Buf) -> Baseline {
        let pr = ctx.params;
        let polar = pr.b("usePolarPath");
        let (pts, closed) = match mask_px(ctx, b, pr.f("path").round() as usize) {
            Some((p, c, _)) if !polar => (p, c),
            _ => {
                let s = b.to_px(pr.v2("startPoint"));
                let e = b.to_px(pr.v2("endPoint"));
                (vec![[s.0, s.1], [e.0, e.1]], false)
            }
        };
        let len = poly_length(&pts, closed);
        let center = pts[0];
        let radius = if pts.len() > 1 { ((pts[1][0] - pts[0][0]).powi(2) + (pts[1][1] - pts[0][1]).powi(2)).sqrt() } else { 1.0 };
        Baseline { pts, closed, len, polar, center, radius }
    }
    fn at(&self, u: f64) -> ([f64; 2], [f64; 2]) {
        if self.polar {
            let a = u * 2.0 * PI - PI * 0.5;
            let (s, c) = a.sin_cos();
            return ([self.center[0] + c * self.radius * 0.5, self.center[1] + s * self.radius * 0.5], [c, s]);
        }
        let (q, d) = crate::util::poly_point_at(&self.pts, self.closed, u * self.len);
        (q, [d[1], -d[0]])
    }
}

/// Draw line/dot marks with core (inside colour) and soft halo (outside colour).
fn draw_marks(b: &mut Buf, segs_core: &[Seg], thickness: f64, softness: f64, inside: [f32; 4], outside: [f32; 4], composite: bool) {
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let core = raster_segs(w, h, segs_core, 1.0);
    let halo_segs: Vec<Seg> = segs_core.iter().map(|s| Seg { r: s.r + thickness * softness.max(0.0) + 0.5, ..*s }).collect();
    let halo = if softness > 0.0 { raster_segs(w, h, &halo_segs, 0.0) } else { core.clone() };
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, hl) = (core.data[i], halo.data[i]);
        let a = c.max(hl);
        let base = if composite { *px } else { [0.0; 4] };
        if a <= 0.0 {
            *px = base;
            return;
        }
        let t = if a > 0.0 { c / a } else { 0.0 };
        let col = crate::util::lerp3([outside[0], outside[1], outside[2]], [inside[0], inside[1], inside[2]], t);
        *px = over(base, premul(col, a));
    });
}

/// Audio Spectrum / Audio Waveform, shared with the GPU compositor (effectcraft-gpu `fx_gen2`):
/// the marks (the audio analysis runs on the CPU) and how [`draw_marks`] colours them.
#[derive(Clone, Debug)]
pub struct MarksPlan {
    pub segs: Vec<Seg>,
    pub thickness: f64,
    pub softness: f64,
    pub inside: [f32; 4],
    pub outside: [f32; 4],
    pub composite: bool,
}

fn marks(ctx: &EffectCtx, segs: Vec<Seg>, thick: f64, soft: f64) -> MarksPlan {
    let pr = ctx.params;
    MarksPlan {
        segs,
        thickness: thick,
        softness: soft * 2.0,
        inside: pr.color("insideColor"),
        outside: pr.color("outsideColor"),
        composite: pr.b("compositeOnOriginal"),
    }
}

fn audio_spectrum(ctx: &EffectCtx, mut b: Buf) -> Buf {
    if let Some(m) = spectrum_plan(ctx, &b) {
        draw_marks(&mut b, &m.segs, m.thickness, m.softness, m.inside, m.outside, m.composite);
    }
    b
}

fn audio_waveform(ctx: &EffectCtx, mut b: Buf) -> Buf {
    if let Some(m) = waveform_plan(ctx, &b) {
        draw_marks(&mut b, &m.segs, m.thickness, m.softness, m.inside, m.outside, m.composite);
    }
    b
}

fn compute_fft_mags(samples: &[f64]) -> (usize, f64, Vec<f64>) {
    let m = samples.len();
    let n = m.next_power_of_two().max(64);
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    let denom = (m.max(2) - 1) as f64;
    for (i, s) in samples.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / denom).cos();
        if let Some(r) = re.get_mut(i) {
            *r = s * w;
        }
    }
    fft(&mut re, &mut im);
    let hz_per_bin = AUDIO_RATE as f64 / n as f64;
    let scale = 4.0 / m.max(1) as f64;
    let num_bins = n / 2 + 1;
    let mut mags = Vec::with_capacity(num_bins);
    for k in 0..num_bins {
        let r = re.get(k).copied().unwrap_or(0.0);
        let im_val = im.get(k).copied().unwrap_or(0.0);
        let mag = (r * r + im_val * im_val).sqrt() * scale;
        mags.push(mag);
    }
    (n, hz_per_bin, mags)
}

fn spectrum_plan(ctx: &EffectCtx, b: &Buf) -> Option<MarksPlan> {
    let pr = ctx.params;
    let samples = audio_window(ctx, pr.f("audioDuration"), pr.f("audioOffset"), 0)?;
    let (_n, hz_per_bin, mags) = compute_fft_mags(&samples);

    let bands = pr.f("frequencyBands").round().clamp(1.0, 4096.0) as usize;
    let f0 = pr.f("startFrequency").max(0.0);
    let f1 = pr.f("endFrequency").max(f0 + 1.0);
    let max_bin = mags.len().saturating_sub(1);

    let get_band_mag = |fc: f64| -> f64 {
        if mags.is_empty() || hz_per_bin <= 0.0 {
            return 0.0;
        }
        let k_float = (fc / hz_per_bin).clamp(0.0, max_bin as f64);
        let k0 = (k_float.floor() as usize).min(max_bin.saturating_sub(1));
        let k1 = (k0 + 1).min(max_bin);
        let frac = (k_float - k0 as f64).clamp(0.0, 1.0);
        let m0 = mags.get(k0).copied().unwrap_or(0.0);
        let m1 = mags.get(k1).copied().unwrap_or(0.0);
        m0 * (1.0 - frac) + m1 * frac
    };

    let max_h = pr.f("maximumHeight") * b.scale;
    let thick = (pr.f("thickness") * b.scale).max(0.1);
    let soft = pr.f("softness") / 100.0;
    let disp = pr.e("displayOptions");
    let side = pr.e("sideOptions");
    let base = Baseline::new(ctx, b);
    let mut tops = Vec::with_capacity(bands);
    let mut segs = Vec::new();
    for i in 0..bands {
        let u = if bands > 1 { i as f64 / (bands - 1) as f64 } else { 0.5 };
        let fc = f0 + (f1 - f0) * u;
        let v = get_band_mag(fc);
        let hgt = if (v * max_h).is_finite() { (v * max_h).clamp(0.0, max_h.max(0.0)) } else { 0.0 };
        let (q, nrm) = base.at(u);
        let (ua, ub) = match side {
            0 => (0.0, hgt),
            1 => (-hgt, 0.0),
            _ => (-hgt, hgt),
        };
        let pa = [q[0] + nrm[0] * ua, q[1] + nrm[1] * ua];
        let pb = [q[0] + nrm[0] * ub, q[1] + nrm[1] * ub];
        match disp {
            0 => segs.push(Seg { a: pa, b: pb, r: thick * 0.5, v: 1.0 }),
            _ => tops.push((pa, pb)),
        }
    }
    if disp == 1 {
        for w2 in tops.windows(2) {
            segs.push(Seg { a: w2[0].1, b: w2[1].1, r: thick * 0.5, v: 1.0 });
            if side == 2 {
                segs.push(Seg { a: w2[0].0, b: w2[1].0, r: thick * 0.5, v: 1.0 });
            }
        }
    } else if disp == 2 {
        for &(pa, pb) in &tops {
            segs.push(Seg { a: pb, b: pb, r: thick * 0.5, v: 1.0 });
            if side == 2 {
                segs.push(Seg { a: pa, b: pa, r: thick * 0.5, v: 1.0 });
            }
        }
    }
    Some(marks(ctx, segs, thick, soft))
}

fn waveform_plan(ctx: &EffectCtx, b: &Buf) -> Option<MarksPlan> {
    let pr = ctx.params;
    let samples = audio_window(ctx, pr.f("audioDuration"), pr.f("audioOffset"), pr.e("waveformOptions"))?;
    let count = pr.f("displayedSamples").round().clamp(2.0, 4096.0) as usize;
    let max_h = pr.f("maximumHeight") * b.scale;
    let thick = (pr.f("thickness") * b.scale).max(0.1);
    let soft = pr.f("softness") / 100.0;
    let disp = pr.e("displayOptions");
    let base = Baseline::new(ctx, b);
    let m = samples.len();
    let mut pts = Vec::with_capacity(count);
    let mut segs = Vec::new();
    for i in 0..count {
        let u = i as f64 / (count - 1) as f64;
        let a = ((u * (m - 1) as f64) as usize).min(m - 1);
        let bnd = (((i + 1) as f64 / count as f64 * (m - 1) as f64) as usize).clamp(a, m - 1);
        // Peak with sign of the window for a stable digital display.
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for s in &samples[a..=bnd] {
            lo = lo.min(*s);
            hi = hi.max(*s);
        }
        let (q, nrm) = base.at(u);
        match disp {
            0 => segs.push(Seg {
                a: [q[0] + nrm[0] * lo * max_h, q[1] + nrm[1] * lo * max_h],
                b: [q[0] + nrm[0] * hi * max_h, q[1] + nrm[1] * hi * max_h],
                r: thick * 0.5,
                v: 1.0,
            }),
            _ => {
                let v = samples[a] * max_h;
                pts.push([q[0] + nrm[0] * v, q[1] + nrm[1] * v]);
            }
        }
    }
    if disp == 1 {
        poly_segs(&pts, false, thick * 0.5, 1.0, &mut segs);
    } else if disp == 2 {
        segs.extend(pts.iter().map(|&q| Seg { a: q, b: q, r: thick * 0.5, v: 1.0 }));
    }
    Some(marks(ctx, segs, thick, soft))
}

// ---------------------------------------------------------------- registry

pub fn specs() -> Vec<EffectSpec> {
    let ang = || ParamUi::Angle;
    let check = |id: &'static str, name: &'static str, v: bool| p(id, name, Value::Bool(v), ParamUi::Checkbox);
    let mask_idx = |id: &'static str, name: &'static str, d: f64| p(id, name, num(d), ParamUi::Mask);
    let display = || popup(&["Digital", "Analog lines", "Analog dots"]);
    vec![
        spec(
            "ec.generate.audiospectrum",
            "Audio Spectrum",
            vec![
                p("audioLayer", "Audio Layer", Value::Layer(None), ParamUi::Layer),
                p("startPoint", "Start Point", pt(0.1, 0.5), ParamUi::Point),
                p("endPoint", "End Point", pt(0.9, 0.5), ParamUi::Point),
                mask_idx("path", "Path", 0.0),
                check("usePolarPath", "Use Polar Path", false),
                p("startFrequency", "Start Frequency", num(20.0), slider(1.0, 20000.0, 1.0, 2000.0, 0)),
                p("endFrequency", "End Frequency", num(2000.0), slider(1.0, 20000.0, 1.0, 10000.0, 0)),
                p("frequencyBands", "Frequency bands", num(64.0), slider(1.0, 4096.0, 1.0, 256.0, 0)),
                p("maximumHeight", "Maximum Height", num(50.0), slider(0.0, 10000.0, 0.0, 500.0, 1)),
                p("audioDuration", "Audio Duration (milliseconds)", num(90.0), slider(1.0, 30000.0, 1.0, 1000.0, 1)),
                p("audioOffset", "Audio Offset (milliseconds)", num(0.0), slider(-30000.0, 30000.0, -1000.0, 1000.0, 1)),
                p("thickness", "Thickness", num(3.0), slider(0.0, 1000.0, 0.0, 50.0, 2)),
                p("softness", "Softness", num(50.0), pct()),
                p("insideColor", "Inside Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("outsideColor", "Outside Color", col(0.2, 0.4, 1.0), ParamUi::Color),
                check("blendOverlappingColors", "Blend Overlapping Colors", true),
                p("hueInterpolation", "Hue Interpolation", num(0.0), ang()),
                check("dynamicHuePhase", "Dynamic Hue Phase", false),
                check("colorSymmetry", "Color Symmetry", false),
                p("displayOptions", "Display Options", Value::Enum(0), display()),
                p("sideOptions", "Side Options", Value::Enum(0), popup(&["Side A", "Side B", "Side A & B"])),
                check("durationAveraging", "Duration Averaging", false),
                check("compositeOnOriginal", "Composite On Original", false),
            ],
            audio_spectrum,
        ),
        spec(
            "ec.generate.audiowaveform",
            "Audio Waveform",
            vec![
                p("audioLayer", "Audio Layer", Value::Layer(None), ParamUi::Layer),
                p("startPoint", "Start Point", pt(0.1, 0.5), ParamUi::Point),
                p("endPoint", "End Point", pt(0.9, 0.5), ParamUi::Point),
                mask_idx("path", "Path", 0.0),
                check("usePolarPath", "Use Polar Path", false),
                p("displayedSamples", "Displayed Samples", num(200.0), slider(2.0, 4096.0, 2.0, 1000.0, 0)),
                p("maximumHeight", "Maximum Height", num(100.0), slider(0.0, 10000.0, 0.0, 500.0, 1)),
                p("audioDuration", "Audio Duration (milliseconds)", num(200.0), slider(1.0, 30000.0, 1.0, 1000.0, 1)),
                p("audioOffset", "Audio Offset (milliseconds)", num(0.0), slider(-30000.0, 30000.0, -1000.0, 1000.0, 1)),
                p("thickness", "Thickness", num(2.0), slider(0.0, 1000.0, 0.0, 50.0, 2)),
                p("softness", "Softness", num(10.0), pct()),
                p("randomSeed", "Random Seed (Analog)", num(1.0), slider(1.0, 10000.0, 1.0, 100.0, 0)),
                p("insideColor", "Inside Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("outsideColor", "Outside Color", col(0.2, 0.4, 1.0), ParamUi::Color),
                p("waveformOptions", "Waveform Options", Value::Enum(0), popup(&["Mono", "Left", "Right"])),
                p("displayOptions", "Display Options", Value::Enum(1), display()),
                check("compositeOnOriginal", "Composite On Original", false),
            ],
            audio_waveform,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, EffectHost, LayerPixels, Params};
    use effectcraft_raster::Image;

    /// Run with point defaults scaled to the layer size.
    pub(crate) fn run_env(id: &str, set: &[(&str, Value)], img: Image, time: f64, env: EffectEnv) -> Buf {
        let s = crate::find(id).unwrap();
        let ls = [img.width as f64, img.height as f64];
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for p in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&p.ui, &p.default) {
                params.values.insert(p.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            assert!(params.values.contains_key(*k), "{id}: unknown {k}");
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time, layer_size: ls, seed: 3, adjustment: false, env };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn run(id: &str, set: &[(&str, Value)], img: Image) -> Buf {
        run_env(id, set, img, 0.5, EffectEnv::default())
    }

    fn alpha_sum(img: &Image, x0: usize, y0: usize, x1: usize, y1: usize) -> f32 {
        let mut s = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                s += img.data[y * img.width as usize + x][3];
            }
        }
        s
    }

    #[test]
    fn audio_spectrum_linear_frequency_and_linear_amplitude() {
        let host = Tone; // Tone produces 440 Hz
        let env = EffectEnv { host: Some(&host), comp_time: 1.0, ..Default::default() };
        let img = Image::new(200, 100);
        let out = run_env(
            "ec.generate.audiospectrum",
            &[
                ("audioLayer", Value::Layer(Some(1))),
                ("startPoint", Value::Vec2([0.0, 50.0])),
                ("endPoint", Value::Vec2([200.0, 50.0])),
                ("startFrequency", num(20.0)),
                ("endFrequency", num(2000.0)),
                ("frequencyBands", num(100.0)),
                ("maximumHeight", num(40.0)),
                ("thickness", num(2.0)),
                ("softness", num(0.0)),
            ],
            img,
            1.0,
            env,
        );
        // Verify 440 Hz tone produces a peak above baseline, while high frequencies have zero bar height
        let peak_zone = alpha_sum(&out.img, 36, 10, 48, 48);
        let high_zone = alpha_sum(&out.img, 120, 10, 150, 48);
        let silent_zone = alpha_sum(&out.img, 170, 10, 195, 48);
        assert!(peak_zone > 5.0, "peak zone has energy: {peak_zone}");
        assert!(peak_zone > high_zone * 2.0, "linear peak is distinctly taller than far frequencies: {peak_zone} vs {high_zone}");
        assert_eq!(silent_zone, 0.0, "silent end has zero height, not boosted by dB scale");
    }

    /// #536: an animated Brush Position draws a stroke that stays (Stroke Length 0), expires
    /// after Stroke Length, and lays its dabs Brush Spacing seconds apart.

    struct Tone;
    impl EffectHost for Tone {
        fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
            None
        }
        fn audio(&self, _: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
            Some(
                (0..frames)
                    .flat_map(|i| {
                        let t = start + i as f64 / rate as f64;
                        let v = (2.0 * PI * 440.0 * t).sin() as f32 * 0.8;
                        [v, v]
                    })
                    .collect(),
            )
        }
    }

    #[test]
    fn audio_effects_without_audio_pass_through() {
        let src = Image::filled(16, 16, [0.3, 0.3, 0.3, 1.0]);
        for id in ["ec.generate.audiospectrum", "ec.generate.audiowaveform"] {
            assert_eq!(run(id, &[], src.clone()).img.data, src.data, "{id}");
        }
    }

    #[test]
    fn audio_effects_draw_with_host() {
        let host = Tone;
        let env = EffectEnv { host: Some(&host), comp_time: 1.0, ..Default::default() };
        let img = Image::new(96, 64);
        for id in ["ec.generate.audiospectrum", "ec.generate.audiowaveform"] {
            let a = run_env(id, &[("audioLayer", Value::Layer(Some(1)))], img.clone(), 1.0, env);
            let b = run_env(id, &[("audioLayer", Value::Layer(Some(1)))], img.clone(), 1.0, env);
            assert_eq!(a.img.data, b.img.data, "{id}");
            assert!(alpha_sum(&a.img, 0, 0, 96, 64) > 10.0, "{id} drew nothing");
        }
    }

    #[test]
    fn fft_finds_tone() {
        let n = 256;
        let mut re: Vec<f64> = (0..n).map(|i| (2.0 * PI * 8.0 * i as f64 / n as f64).cos()).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let mags: Vec<f64> = (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
        let peak = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, 8);
    }
}
