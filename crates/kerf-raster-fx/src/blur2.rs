//! Blur & Sharpen, batch 2: edge-preserving blurs (Bilateral, Smart), per-channel and map-driven
//! blurs (Channel, Compound, CC Vector), bokeh (Camera Lens Blur), radial streaks and small
//! utilities (Reduce Interlace Flicker, CC Cross Blur).

use crate::ParamUi;
use crate::Value;
use crate::{Buf, EffectCtx, EffectSpec, num, p, slider};
use kerf_color::luma709;
use kerf_raster::{Image, Px};
use rayon::prelude::*;

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Blur & Sharpen", params, render, gpu: false, float: true }
}

#[inline]
fn plum(p: Px) -> f32 {
    luma709(p[0], p[1], p[2])
}

// ---------------------------------------------------------------------------------------------
// Bilateral / Smart Blur

/// Edge-preserving neighbourhood average. `hard` = box spatial kernel + hard luminance range test
/// (Smart Blur); otherwise Gaussian spatial × Gaussian range weights (bilateral filter, Tomasi &
/// Manduchi 1998). Large radii are sampled on a stride to stay fast.
fn edge_preserving(src: &Image, r: f64, range: f32, hard: bool) -> Image {
    let ri = r.ceil() as i64;
    let stride = ((r / 6.0).ceil() as i64).max(1);
    let sig_s2 = 2.0 * (r / 2.0).max(0.5).powi(2);
    let sig_r2 = 2.0 * range.max(1e-3).powi(2);
    let mut out = Image::new(src.width, src.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let c = src.data[y * src.width as usize + x];
            let cl = plum(c);
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            let mut dy = -ri;
            while dy <= ri {
                let mut dx = -ri;
                while dx <= ri {
                    let d2 = (dx * dx + dy * dy) as f64;
                    if d2 <= r * r + 0.5 {
                        let q = src.get_clamped(x as i64 + dx, y as i64 + dy);
                        let w = if hard {
                            if (plum(q) - cl).abs() <= range { 1.0 } else { 0.0 }
                        } else {
                            let dc = (q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2) + (q[2] - c[2]).powi(2) + (q[3] - c[3]).powi(2);
                            ((-d2 / sig_s2) as f32 - dc / sig_r2).exp()
                        };
                        if w > 0.0 {
                            for k in 0..4 {
                                acc[k] += q[k] * w;
                            }
                            wsum += w;
                        }
                    }
                    dx += stride;
                }
                dy += stride;
            }
            *o = if wsum > 0.0 { acc.map(|v| v / wsum) } else { c };
        }
    });
    out
}

fn bilateral(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = ctx.params.f("radius") * b.scale;
    if r < 0.5 {
        return b;
    }
    let range = ctx.params.f("threshold") as f32 / 100.0;
    b.img = edge_preserving(&b.img, r, range, false);
    if !ctx.params.b("colorize") {
        b.img.map_straight(|c| {
            let l = luma709(c[0], c[1], c[2]);
            [l, l, l]
        });
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Channel Blur

// ---------------------------------------------------------------------------------------------
// Compound Blur

// ---------------------------------------------------------------------------------------------
// Camera Lens Blur

// ---------------------------------------------------------------------------------------------
// CC Radial Fast Blur

// ---------------------------------------------------------------------------------------------
// CC Vector Blur

// ---------------------------------------------------------------------------------------------
// Reduce Interlace Flicker / CC Cross Blur

pub fn specs() -> Vec<EffectSpec> {
    vec![spec(
        "ec.blur.bilateral",
        "Bilateral Blur",
        vec![
            p("radius", "Radius", num(5.0), slider(0.0, 200.0, 0.0, 50.0, 1)),
            p("threshold", "Threshold", num(10.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            p("colorize", "Colorize", Value::Bool(true), ParamUi::Checkbox),
        ],
        bilateral,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, find};
    use kerf_raster::gaussian_blur;

    fn run(id: &str, img: &Image, over: &[(&str, Value)]) -> Buf {
        let s = find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in over {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx =
            EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: Default::default() };
        (s.render)(&ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 })
    }

    fn edge_img() -> Image {
        let mut img = Image::new(32, 16);
        for y in 0..16 {
            for x in 0..32 {
                let v = if x < 16 { 0.1 } else { 0.9 };
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        img
    }

    #[test]
    fn bilateral_keeps_edge_sharper_than_gaussian() {
        let img = edge_img();
        let bl = run("ec.blur.bilateral", &img, &[("radius", num(4.0))]).img;
        let g = gaussian_blur(&img, 2.0, 2.0, true);
        let jump_b = bl.get(16, 8)[0] - bl.get(15, 8)[0];
        let jump_g = g.get(16, 8)[0] - g.get(15, 8)[0];
        assert!(jump_b > 0.7, "{jump_b}");
        assert!(jump_b > jump_g + 0.3);
    }
}
