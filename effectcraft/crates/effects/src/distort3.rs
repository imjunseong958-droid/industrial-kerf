//! Distort effects, batch 3: CC Bender, CC Blobbylize, CC Power Pin, CC Ripple Pulse, CC Slant,
//! CC Smear, CC Split, CC Split 2, Smear and Reshape (mask driven), Twirl (Legacy), Warp (the
//! fifteen warp styles) and Detail-preserving Upscale.
//!
//! All warps are inverse mapped: every output pixel finds the source point that lands on it.
//! Pixel-sized parameters are multiplied by the buffer scale (downsampled previews).
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};
use crate::Value;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

// ---------------------------------------------------------------- CC Bender

// ---------------------------------------------------------------- shading helpers

// ---------------------------------------------------------------- CC Blobbylize

// ---------------------------------------------------------------- CC Power Pin

// ---------------------------------------------------------------- CC Ripple Pulse

// ---------------------------------------------------------------- CC Slant

// ---------------------------------------------------------------- CC Smear

// ---------------------------------------------------------------- CC Split / CC Split 2

// ---------------------------------------------------------------- Smear (mask based)

// ---------------------------------------------------------------- Reshape

// ---------------------------------------------------------------- Twirl (Legacy)

// ---------------------------------------------------------------- Warp

// ---------------------------------------------------------------- Detail-preserving Upscale

/// Catmull-Rom weight.
fn cubic_w(t: f64) -> f64 {
    let t = t.abs();
    if t < 1.0 {
        1.5 * t * t * t - 2.5 * t * t + 1.0
    } else if t < 2.0 {
        -0.5 * t * t * t + 2.5 * t * t - 4.0 * t + 2.0
    } else {
        0.0
    }
}

fn bicubic(img: &Image, x: f64, y: f64) -> Px {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let wx = [cubic_w(1.0 + tx), cubic_w(tx), cubic_w(1.0 - tx), cubic_w(2.0 - tx)];
    let wy = [cubic_w(1.0 + ty), cubic_w(ty), cubic_w(1.0 - ty), cubic_w(2.0 - ty)];
    let mut o = [0.0f64; 4];
    for (j, wyj) in wy.iter().enumerate() {
        for (i, wxi) in wx.iter().enumerate() {
            let p = img.get_clamped(x0 as i64 - 1 + i as i64, y0 as i64 - 1 + j as i64);
            let w = wxi * wyj;
            for c in 0..4 {
                o[c] += p[c] as f64 * w;
            }
        }
    }
    let a = o[3].clamp(0.0, 1.0) as f32;
    [(o[0] as f32).clamp(0.0, a.max(o[0] as f32)), (o[1] as f32).clamp(0.0, a.max(o[1] as f32)), (o[2] as f32).clamp(0.0, a.max(o[2] as f32)), a]
}

fn upscale(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = (ctx.params.f("scale") / 100.0).clamp(1.0, 10.0);
    if (k - 1.0).abs() < 1e-9 {
        return b;
    }
    let detail = (ctx.params.f("detail") / 100.0) as f32;
    let noise = ctx.params.f("reduceNoise") / 100.0;
    let src = if noise > 0.0 { effectcraft_raster::gaussian_blur(&b.img, noise * 1.5, noise * 1.5, true) } else { b.img.clone() };
    let lc = [ctx.layer_size[0] * 0.5, ctx.layer_size[1] * 0.5];
    // Alpha: Bilinear (index 0) resamples the alpha channel bilinearly, Bicubic like the colour.
    let bilinear_alpha = ctx.params.e("alpha") == 0;
    let up = |img: &Image, x: f64, y: f64| -> Px {
        let p = bicubic(img, x, y);
        if !bilinear_alpha {
            return p;
        }
        let a = img.sample_bilinear_clamped(x, y)[3];
        let k = if p[3] > 1e-6 { a / p[3] } else { 0.0 };
        [p[0] * k, p[1] * k, p[2] * k, a]
    };
    let mut out = if ctx.adjustment {
        // Scale in place about the layer centre.
        let c = b.to_px(lc);
        let mut o = Image::new(src.width, src.height);
        o.rows_mut().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let sx = c.0 + (x as f64 + 0.5 - c.0) / k;
                let sy = c.1 + (y as f64 + 0.5 - c.1) / k;
                *px = if sx < 0.0 || sy < 0.0 || sx > src.width as f64 || sy > src.height as f64 { [0.0; 4] } else { up(&src, sx, sy) };
            }
        });
        o
    } else {
        let w = ((src.width as f64 * k).round() as u32).clamp(1, 16384);
        let h = ((src.height as f64 * k).round() as u32).clamp(1, 16384);
        let (kx, ky) = (w as f64 / src.width as f64, h as f64 / src.height as f64);
        let mut o = Image::new(w, h);
        o.rows_mut().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                *px = up(&src, (x as f64 + 0.5) / kx, (y as f64 + 0.5) / ky);
            }
        });
        let c_sc = [lc[0] * b.scale, lc[1] * b.scale];
        b.offset = [b.offset[0] * kx + c_sc[0] * (kx - 1.0), b.offset[1] * ky + c_sc[1] * (ky - 1.0)];
        o
    };
    if detail > 0.0 {
        let sigma = 0.6 * k;
        let blur = effectcraft_raster::gaussian_blur(&out, sigma, sigma, true);
        out.data.par_iter_mut().zip(blur.data.par_iter()).for_each(|(p, q)| {
            let a = p[3];
            for c in 0..3 {
                p[c] = (p[c] + (p[c] - q[c]) * detail * 1.5).clamp(0.0, a.max(p[c]));
            }
        });
    }
    b.img = out;
    b
}

// ---------------------------------------------------------------- registry

pub fn specs() -> Vec<EffectSpec> {
    let pct = || slider(0.0, 100.0, 0.0, 100.0, 1);
    vec![spec(
        "ec.distort.upscale",
        "Detail-preserving Upscale",
        vec![
            p("scale", "Scale", num(100.0), slider(100.0, 1000.0, 100.0, 400.0, 1)),
            p("reduceNoise", "Reduce Noise", num(0.0), pct()),
            p("detail", "Detail", num(20.0), pct()),
            p("alpha", "Alpha", Value::Enum(1), popup(&["Bilinear", "Bicubic"])),
        ],
        upscale,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, MaskShape, Params};
    use crate::ParamUi;

    fn img(w: u32, h: u32) -> Image {
        crate::util::gen_image(w, h, |x, y| [x as f32 / w as f32, y as f32 / h as f32, ((x * 7 + y * 3) % 11) as f32 / 11.0, 1.0])
    }

    /// Run with Point defaults scaled to the layer (like `instantiate`).
    fn run(id: &str, set: &[(&str, Value)], im: Image, masks: &[MaskShape]) -> Buf {
        let s = crate::find(id).unwrap();
        let ls = [im.width as f64, im.height as f64];
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for ps in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&ps.ui, &ps.default) {
                params.values.insert(ps.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            assert!(params.values.contains_key(*k), "{id}: {k}");
            params.values.insert(k.to_string(), v.clone());
        }
        let env = EffectEnv { masks, ..Default::default() };
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: ls, seed: 1, adjustment: false, env };
        crate::apply(s, &ctx, Buf { img: im, offset: [0.0, 0.0], scale: 1.0 })
    }

    /// #659: CC Smear and CC Tiler on a text layer reach past the text's box to the layer bounds
    /// (comp-sized, as in After Effects), not cut off where the glyphs end.

    #[test]
    fn upscale_grows_about_centre() {
        let im = img(20, 10);
        let o = run("ec.distort.upscale", &[("scale", num(200.0)), ("detail", num(0.0))], im.clone(), &[]);
        assert_eq!((o.img.width, o.img.height), (40, 20));
        // Grown about the layer centre: layer point (0,0) now sits at buffer pixel (10,5), so the
        // centre (10,5) lands in the middle of the 40×20 buffer.
        assert_eq!(o.offset, [10.0, 5.0]);
        let c = o.img.sample_bilinear(20.0, 10.0);
        assert!((c[0] - im.sample_bilinear(10.0, 5.0)[0]).abs() < 0.03);
    }
}
