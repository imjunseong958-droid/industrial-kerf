//! Distort: CC Flo Motion, Liquify and Rolling Shutter Repair.
//!
//! * **CC Flo Motion** — two knots that pull the image toward themselves (positive amount) or
//!   push it away (negative), with a falloff exponent, optional tiling and supersampled
//!   antialiasing.
//! * **Liquify** — brush strokes build a displacement mesh (a grid of layer-space offsets). The
//!   strokes are the effect's data (the hidden `distortionMesh` text, one stroke per line, so it
//!   animates with hold keyframes and is written by the `liquify.stroke` command); replaying
//!   them is deterministic and cached. Tools: warp, turbulence, twirl clockwise / counter-
//!   clockwise, pucker, bloat, shift pixels, reflection, clone, reconstruction and freeze / thaw.
//!   Distortion Percentage scales the mesh and Distortion Mesh Offset moves it.
//! * **Rolling Shutter Repair** — every scanline of a CMOS frame is exposed a little later than
//!   the previous one, so moving content shears. Motion is measured between the neighbouring
//!   frames with pyramidal Lucas–Kanade (`klt`) on a grid of points; each pixel is
//!   then resampled from where it was at the frame's mid-exposure time:
//!   `out(p) = in(p + v(p)·rate·(s(p) − ½))`, with `s` the scan position (0…1 along the scan
//!   direction) and `v` the per-frame velocity (Warp: one smooth velocity per scanline band;
//!   Pixel Motion: a dense interpolated field).

use crate::ParamUi;
use crate::Value;
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};
use effectcraft_raster::Image;
use rayon::prelude::*;

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

// ---------------------------------------------------------------- CC Flo Motion

// ---------------------------------------------------------------- Liquify

// ---------------------------------------------------------------- Rolling Shutter Repair

/// Per-pixel velocity field (layer pixels per frame) on a coarse grid, from tracked points.
struct Flow {
    nx: usize,
    ny: usize,
    cell: f64,
    v: Vec<[f64; 2]>,
}

impl Flow {
    fn at(&self, x: f64, y: f64) -> [f64; 2] {
        let fx = (x / self.cell - 0.5).clamp(0.0, (self.nx - 1) as f64);
        let fy = (y / self.cell - 0.5).clamp(0.0, (self.ny - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.nx - 1), (y0 + 1).min(self.ny - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let g = |i: usize, j: usize| self.v[j * self.nx + i];
        let l = |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        l(l(g(x0, y0), g(x1, y0), tx), l(g(x0, y1), g(x1, y1), tx), ty)
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Measure motion around the current frame (`cur`) from `prev` to `next` (all in the same
/// buffer grid). `per_row`: Warp method (one velocity per scanline band).
/// `detail` (Pixel Motion Detail, 0..1; 0.2 = the standard grid) scales the number of motion
/// vectors the Pixel Motion method measures: more vectors follow finer local motion.
fn measure(cur: &Image, prev: Option<&Image>, next: Option<&Image>, scale: f64, detailed: bool, per_row: bool, detail: f64) -> Option<Flow> {
    use crate::klt::{GrayPyramid, LkOpts, analysis_factor, track};
    let factor = analysis_factor(cur.width, cur.height, if detailed { 640 } else { 320 });
    let pc = GrayPyramid::from_image(cur, factor, 4);
    let (w, h) = (pc.width(), pc.height());
    let base = if detailed { 24.0 } else { 12.0 };
    let n = if per_row { base as usize } else { (base * (0.5 + detail.clamp(0.0, 1.0) * 2.5)).round().clamp(4.0, 96.0) as usize };
    let cell_w = w as f64 / n as f64;
    let rows = ((n as f64) * h as f64 / w.max(1) as f64).round().max(2.0) as usize;
    let cell_h = h as f64 / rows as f64;
    let pts: Vec<[f64; 2]> = (0..rows).flat_map(|j| (0..n).map(move |i| [(i as f64 + 0.5) * cell_w, (j as f64 + 0.5) * cell_h])).collect();
    let o = LkOpts { radius: 4, fb_max: 1.0, ..Default::default() };
    let go = |other: Option<&Image>| other.map(|img| track(&pc, &GrayPyramid::from_image(img, factor, 4), &pts, None, &o));
    let (tn, tp) = (go(next), go(prev));
    // Velocity in plane pixels per frame.
    let vel: Vec<Option<[f64; 2]>> = (0..pts.len())
        .map(|i| {
            let p = pts[i];
            let fw = tn.as_ref().and_then(|t| t[i]).map(|q| [q[0] - p[0], q[1] - p[1]]);
            let bw = tp.as_ref().and_then(|t| t[i]).map(|q| [p[0] - q[0], p[1] - q[1]]);
            match (fw, bw) {
                (Some(a), Some(b)) => Some([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]),
                (a, b) => a.or(b),
            }
        })
        .collect();
    let valid: Vec<[f64; 2]> = vel.iter().flatten().copied().collect();
    if valid.is_empty() {
        return None;
    }
    let mut vx: Vec<f64> = valid.iter().map(|v| v[0]).collect();
    let mut vy: Vec<f64> = valid.iter().map(|v| v[1]).collect();
    let global = [median(&mut vx), median(&mut vy)];
    let k = factor as f64 / scale.max(1e-9);
    let mut v = vec![[0.0; 2]; n * rows];
    for j in 0..rows {
        // Row (band) median for Warp; per point (outliers replaced) for Pixel Motion.
        let band: Vec<[f64; 2]> = (0..n).filter_map(|i| vel[j * n + i]).collect();
        let row_med = if band.is_empty() {
            global
        } else {
            let mut a: Vec<f64> = band.iter().map(|v| v[0]).collect();
            let mut b: Vec<f64> = band.iter().map(|v| v[1]).collect();
            [median(&mut a), median(&mut b)]
        };
        for i in 0..n {
            let s = if per_row {
                row_med
            } else {
                match vel[j * n + i] {
                    Some(q) if (q[0] - row_med[0]).abs() + (q[1] - row_med[1]).abs() < 3.0 + 0.5 * (row_med[0].abs() + row_med[1].abs()) => q,
                    _ => row_med,
                }
            };
            v[j * n + i] = [s[0] * k, s[1] * k];
        }
    }
    // Cell size in layer pixels (grid is uniform in plane pixels; assume square plane pixels).
    Some(Flow { nx: n, ny: rows, cell: cell_w * k, v })
}

fn same_grid(nb: Buf, b: &Buf) -> Image {
    if nb.img.width == b.img.width && nb.img.height == b.img.height && nb.offset == b.offset && nb.scale == b.scale {
        return nb.img;
    }
    let k = nb.scale / b.scale.max(1e-9);
    crate::util::gen_image(b.img.width, b.img.height, |x, y| {
        let lx = (x as f64 + 0.5 - b.offset[0]) * k + nb.offset[0];
        let ly = (y as f64 + 0.5 - b.offset[1]) * k + nb.offset[1];
        nb.img.sample_bilinear(lx, ly)
    })
}

fn rolling_shutter(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let rate = ctx.params.f("rollingShutterRate") / 100.0;
    if rate == 0.0 {
        return b;
    }
    let Some(host) = ctx.env.host else { return b };
    let fps = ctx.fps();
    let prev = host.self_at(ctx.time - 1.0 / fps, ctx.env.effect_index).map(|nb| same_grid(nb, &b));
    let next = host.self_at(ctx.time + 1.0 / fps, ctx.env.effect_index).map(|nb| same_grid(nb, &b));
    if prev.is_none() && next.is_none() {
        return b;
    }
    let per_row = ctx.params.e("advanced/method") == 0;
    let Some(flow) = measure(
        &b.img,
        prev.as_ref(),
        next.as_ref(),
        1.0,
        ctx.params.b("advanced/detailedAnalysis"),
        per_row,
        ctx.params.f("advanced/pixelMotionDetail") / 100.0,
    ) else {
        return b;
    };
    let dir = ctx.params.e("scanDirection");
    let src = b.img.clone();
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let s = match dir {
                1 => 1.0 - fy / h,
                2 => fx / w,
                3 => 1.0 - fx / w,
                _ => fy / h,
            };
            let tau = rate * (s - 0.5);
            let v = flow.at(fx, fy);
            *px = src.sample_bilinear_clamped(fx + v[0] * tau, fy + v[1] * tau);
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    vec![spec(
        "ec.distort.rollingshutterrepair",
        "Rolling Shutter Repair",
        vec![
            p("rollingShutterRate", "Rolling Shutter Rate", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            p("scanDirection", "Scan Direction", Value::Enum(0), popup(&["Top → Bottom", "Bottom → Top", "Left → Right", "Right → Left"])),
            p("advanced/method", "Method", Value::Enum(0), popup(&["Warp", "Pixel Motion"])),
            p("advanced/detailedAnalysis", "Detailed Analysis", Value::Bool(false), ParamUi::Checkbox),
            p("advanced/pixelMotionDetail", "Pixel Motion Detail", num(20.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
        ],
        rolling_shutter,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, EffectHost, LayerPixels, run_fx};

    /// A textured frame panning right at 3 px per frame, captured with a rolling shutter: row
    /// `y` is exposed `(y / h − ½)` frames late (rate 100 %).
    fn texture(x: f64, y: f64) -> f32 {
        let a = ((x * 0.31).sin() * (y * 0.23).cos() + (x * 0.07 + y * 0.11).sin()) as f32;
        0.5 + 0.25 * a
    }
    fn rs_frame(f: f64, shutter: bool) -> Image {
        let (w, h) = (96u32, 64u32);
        crate::util::gen_image(w, h, |x, y| {
            let tau = if shutter { (y as f64 + 0.5) / h as f64 - 0.5 } else { 0.0 };
            let shift = 3.0 * (f + tau);
            let v = texture(x as f64 + 0.5 - shift, y as f64 + 0.5);
            [v, v, v, 1.0]
        })
    }
    struct Pan;
    impl EffectHost for Pan {
        fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
            None
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
        fn self_at(&self, t: f64, _: usize) -> Option<Buf> {
            Some(Buf { img: rs_frame(t * 24.0, true), offset: [0.0; 2], scale: 1.0 })
        }
    }

    #[test]
    fn rolling_shutter_repair_unskews_a_pan() {
        let host = Pan;
        let env = EffectEnv { host: Some(&host), frame_rate: 24.0, ..Default::default() };
        let t = 5.0 / 24.0;
        let skewed = rs_frame(5.0, true);
        let truth = rs_frame(5.0, false);
        let err = |a: &Image| {
            let mut s = 0.0;
            let mut n = 0.0;
            for y in 4..60 {
                for x in 12..84 {
                    s += (a.get(x, y)[0] - truth.get(x, y)[0]).abs();
                    n += 1.0;
                }
            }
            s / n
        };
        for method in [0u32, 1] {
            let out = run_fx(
                "ec.distort.rollingshutterrepair",
                &[("rollingShutterRate", num(100.0)), ("advanced/method", Value::Enum(method))],
                skewed.clone(),
                t,
                env,
            );
            assert!(err(&out.img) < err(&skewed) * 0.4, "method {method}: {} vs {}", err(&out.img), err(&skewed));
            let again = run_fx(
                "ec.distort.rollingshutterrepair",
                &[("rollingShutterRate", num(100.0)), ("advanced/method", Value::Enum(method))],
                skewed.clone(),
                t,
                env,
            );
            assert_eq!(out.img.data, again.img.data);
        }
        // Pixel Motion Detail changes how many vectors Pixel Motion measures (and still repairs).
        let detail = |d: f64| {
            run_fx(
                "ec.distort.rollingshutterrepair",
                &[("rollingShutterRate", num(100.0)), ("advanced/method", Value::Enum(1)), ("advanced/pixelMotionDetail", num(d))],
                skewed.clone(),
                t,
                env,
            )
            .img
        };
        let (fine, coarse) = (detail(100.0), detail(0.0));
        assert_ne!(fine.data, coarse.data);
        assert!(err(&fine) < err(&skewed) * 0.5 && err(&coarse) < err(&skewed) * 0.5);
        let none = run_fx("ec.distort.rollingshutterrepair", &[], skewed.clone(), t, EffectEnv::default());
        assert_eq!(none.img.data, skewed.data);
    }
}
