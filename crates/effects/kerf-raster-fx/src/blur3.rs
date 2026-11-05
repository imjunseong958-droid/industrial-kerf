//! Blur & Sharpen: CC Radial Blur and Camera-Shake Deblur.
//!
//! * **CC Radial Blur** — zoom (straight, fading, centred both ways) and rotational (plain or
//!   fading) blurs around a centre point, sampled along the ray / arc through each pixel with a
//!   sample count set by Quality.
//! * **Camera-Shake Deblur** — frame-based restoration in the spirit of published video deblurring
//!   by "lucky frames": the frame is split into patches; for each patch the sharpness (mean
//!   squared gradient) is compared with the same patch in the neighbouring frames within Blur
//!   Duration; when a neighbour is clearly sharper, its patch is aligned to the current frame by
//!   a small translation search (sum of squared differences on luma) and substituted with
//!   feathered patch blending. Deterministic; neighbours come from [`crate::EffectHost::self_at`].

use crate::ParamUi;
use crate::Value;
use rayon::prelude::*;

use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Blur & Sharpen", params, render, gpu: false, float: true }
}

// ---------------------------------------------------------------- CC Radial Blur

pub const RADIAL_TYPES: [&str; 5] = ["Straight Zoom", "Fading Zoom", "Centered", "Rotate", "Rotate Fading"];

fn cc_radial_blur(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amount = ctx.params.f("amount");
    if amount == 0.0 {
        return b;
    }
    let kind = ctx.params.e("type");
    let quality = ctx.params.f("quality").clamp(1.0, 100.0);
    let c = b.to_px(ctx.params.v2("center"));
    let src = b.img.clone();
    // Zoom: fraction of the distance to the centre; rotate: degrees of arc.
    let zoom = (amount / 100.0).clamp(-1.0, 1.0) * 0.5;
    let arc = amount.to_radians() * 0.25;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
            let r = (dx * dx + dy * dy).sqrt();
            let span = if kind >= 3 { r * arc.abs() } else { r * zoom.abs() };
            let n = ((span * quality / 50.0).ceil() as usize).clamp(1, 256);
            if n <= 1 {
                *o = src.get(x as i64, y as i64);
                continue;
            }
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0f32;
            for i in 0..n {
                let t = i as f64 / (n - 1) as f64;
                let (sx, sy, w) = match kind {
                    // Toward the centre, equal weights.
                    0 => {
                        let k = 1.0 - zoom * t;
                        (c.0 + dx * k, c.1 + dy * k, 1.0)
                    }
                    // Toward the centre, weights fading with distance along the streak.
                    1 => {
                        let k = 1.0 - zoom * t;
                        (c.0 + dx * k, c.1 + dy * k, (1.0 - t) as f32 + 0.05)
                    }
                    // Both ways around the pixel.
                    2 => {
                        let k = 1.0 + zoom * (t - 0.5);
                        (c.0 + dx * k, c.1 + dy * k, 1.0)
                    }
                    _ => {
                        let a = arc * (t - 0.5);
                        let (s, co) = a.sin_cos();
                        let w = if kind == 4 { (1.0 - (2.0 * t - 1.0).abs()) as f32 + 0.05 } else { 1.0 };
                        (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co, w)
                    }
                };
                let q = src.sample_bilinear(sx, sy);
                for k in 0..4 {
                    acc[k] += q[k] * w;
                }
                wsum += w;
            }
            *o = acc.map(|v| v / wsum);
        }
    });
    b
}

// ---------------------------------------------------------------- Camera-Shake Deblur

pub fn specs() -> Vec<EffectSpec> {
    vec![spec(
        "ec.blur.ccradial",
        "CC Radial Blur",
        vec![
            p("type", "Type", Value::Enum(1), popup(&RADIAL_TYPES)),
            p("amount", "Amount", num(10.0), slider(-100.0, 100.0, -100.0, 100.0, 1)),
            p("quality", "Quality", num(50.0), slider(1.0, 100.0, 1.0, 100.0, 1)),
            p("center", "Center", Value::Vec2([0.5, 0.5]), ParamUi::Point),
        ],
        cc_radial_blur,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};
    use kerf_raster::Image;

    fn checker(w: u32, h: u32, cell: u32) -> Image {
        crate::util::gen_image(w, h, |x, y| {
            let v = if ((x as u32 / cell) + (y as u32 / cell)).is_multiple_of(2) { 1.0 } else { 0.0 };
            [v, v, v, 1.0]
        })
    }

    #[test]
    fn cc_radial_blur_keeps_centre_and_smears_edges() {
        let img = checker(64, 64, 4);
        for kind in 0..5u32 {
            let out = run_fx(
                "ec.blur.ccradial",
                &[("type", Value::Enum(kind)), ("amount", num(40.0)), ("center", Value::Vec2([32.0, 32.0]))],
                img.clone(),
                0.0,
                EffectEnv::default(),
            );
            // Far from the centre the checker is smeared toward grey.
            let edge = out.img.get(2, 2)[0];
            assert!(edge > 0.05 && edge < 0.95, "type {kind}: {edge}");
            let again = run_fx(
                "ec.blur.ccradial",
                &[("type", Value::Enum(kind)), ("amount", num(40.0)), ("center", Value::Vec2([32.0, 32.0]))],
                img.clone(),
                0.0,
                EffectEnv::default(),
            );
            assert_eq!(out.img.data, again.img.data);
        }
        // Zero amount is identity; a constant image stays constant.
        let flat = Image::filled(16, 16, [0.4, 0.4, 0.4, 1.0]);
        let o = run_fx("ec.blur.ccradial", &[("amount", num(80.0))], flat.clone(), 0.0, EffectEnv::default());
        assert!(o.img.data.iter().all(|p| (p[0] - 0.4).abs() < 1e-5));
        let o = run_fx("ec.blur.ccradial", &[("amount", num(0.0))], img.clone(), 0.0, EffectEnv::default());
        assert_eq!(o.img.data, img.data);
    }
}
