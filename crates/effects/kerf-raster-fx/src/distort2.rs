//! Distort effects, batch 2: Turbulent Displace, Displacement Map (self as map), Optics
//! Compensation, Magnify, Mesh Warp, Bezier Warp, CC Bend It, CC Lens, CC Page Turn, CC Tiler,
//! CC Griddler.
//!
//! Mesh Warp and Bezier Warp share a grid rasteriser: a (nx+1)×(ny+1) grid of destination points
//! and matching source points is split into triangles, bucketed per output row and filled
//! row-parallel with inverse barycentric sampling.
//!
//! Mesh Warp's distortion mesh is stored in the hidden string parameter `mesh`: whitespace (or
//! `;`) separated `dx,dy` pairs, row-major over the (rows+1)×(columns+1) vertices, giving each
//! vertex's offset in layer pixels. An empty string (or one with the wrong count) is the
//! undistorted mesh.

use crate::ParamUi;
use crate::Value;
use std::f64::consts::FRAC_PI_2;

use crate::util::{layer_rect, remap};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

// ---------------------------------------------------------------- Turbulent Displace

// ---------------------------------------------------------------- Displacement Map

// ---------------------------------------------------------------- Optics Compensation

fn optics_compensation(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let fov = ctx.params.f("fieldOfView").clamp(0.0, 179.0).to_radians();
    if fov < 1e-6 {
        return b;
    }
    let reverse = ctx.params.b("reverseLensDistortion");
    let (lx, ly, lw, lh) = layer_rect(ctx, &b);
    let r_ref = match ctx.params.e("fovOrientation") {
        1 => lh * 0.5,
        2 => (lw * lw + lh * lh).sqrt() * 0.5,
        _ => lw * 0.5,
    }
    .max(1.0);
    let half = fov * 0.5;
    let f = r_ref / half.tan();
    // Resize (with Reverse Lens Distortion): grow the layer so the stretched corners stay
    // visible, up to 2× / 4× the layer size (Unlimited is capped at 8×).
    // Optimal Pixels (invalidates Resize): scale the result so the whole corrected picture
    // fits the layer (Reverse) or fills it with no empty corners (forward).
    let optimal = ctx.params.b("optimalPixels");
    let resize = if optimal { 0 } else { ctx.params.e("resize") };
    if reverse && resize > 0 && !ctx.adjustment {
        let c = b.to_px(ctx.params.v2("viewCenter"));
        let rc =
            [(lx, ly), (lx + lw, ly), (lx, ly + lh), (lx + lw, ly + lh)].iter().map(|p| ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()).fold(0.0, f64::max);
        let th = rc / r_ref * half;
        let cap = lw.max(lh) * [0.0, 0.5, 1.5, 3.5][resize.min(3) as usize];
        let grow = if th < FRAC_PI_2 - 1e-3 { (f * th.tan() - rc).max(0.0) } else { cap };
        b.pad(grow.min(cap).min(4096.0).ceil() as u32);
    }
    let c = b.to_px(ctx.params.v2("viewCenter"));
    let fit = if optimal {
        let rc =
            [(lx, ly), (lx + lw, ly), (lx, ly + lh), (lx + lw, ly + lh)].iter().map(|p| ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()).fold(1.0, f64::max);
        if reverse {
            let th = (rc / r_ref * half).min(FRAC_PI_2 - 1e-3);
            (f * th.tan() / rc).max(1.0)
        } else {
            ((rc / f).atan() / half * r_ref / rc).min(1.0)
        }
    } else {
        1.0
    };
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let r = (dx * dx + dy * dy).sqrt() * fit;
        if r < 1e-9 {
            return Some((x, y));
        }
        let rs = if reverse {
            (r / f).atan() / half * r_ref
        } else {
            let th = r / r_ref * half;
            if th >= FRAC_PI_2 - 1e-4 {
                return None;
            }
            f * th.tan()
        };
        let k = rs / r * fit;
        Some((c.0 + dx * k, c.1 + dy * k))
    });
    b
}

// ---------------------------------------------------------------- Magnify

// ---------------------------------------------------------------- grid rasteriser

// ---------------------------------------------------------------- Bezier Warp

// ---------------------------------------------------------------- CC Bend It

// ---------------------------------------------------------------- CC Lens

// ---------------------------------------------------------------- CC Page Turn

// ---------------------------------------------------------------- CC Tiler / CC Griddler

pub fn specs() -> Vec<EffectSpec> {
    vec![spec(
        "ec.distort.opticscompensation",
        "Optics Compensation",
        vec![
            p("fieldOfView", "Field Of View (FOV)", num(0.0), slider(0.0, 180.0, 0.0, 180.0, 1)),
            p("reverseLensDistortion", "Reverse Lens Distortion", Value::Bool(false), ParamUi::Checkbox),
            p("fovOrientation", "FOV Orientation", Value::Enum(0), popup(&["Horizontal", "Vertical", "Diagonal"])),
            p("viewCenter", "View Center", pt(0.5, 0.5), ParamUi::Point),
            p("optimalPixels", "Optimal Pixels (Invalidates Resize)", Value::Bool(false), ParamUi::Checkbox),
            p("resize", "Resize", Value::Enum(0), popup(&["Off", "Max 2X", "Max 4X", "Unlimited"])),
        ],
        optics_compensation,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;
    use kerf_raster::Image;

    fn test_img(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let a = 1.0;
                img.set(x, y, [x as f32 / w as f32 * a, y as f32 / h as f32 * a, ((x * 7 + y * 3) % 11) as f32 / 11.0 * a, a]);
            }
        }
        img
    }

    fn run(id: &str, set: &[(&str, Value)], img: Image) -> Buf {
        let s = crate::find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        let ls = [img.width as f64, img.height as f64];
        for p in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&p.ui, &p.default) {
                params.values.insert(p.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: ls, seed: 3, adjustment: false, env: Default::default() };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn max_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|i| (p[i] - q[i]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
    }

    #[test]
    fn optics_fov_zero_is_identity_and_nonzero_warps() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.opticscompensation", &[], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.opticscompensation", &[("fieldOfView", num(90.0))], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn optics_resize_grows_reversed_layers() {
        let img = test_img(32, 24);
        let set = |r: u32| vec![("fieldOfView", num(120.0)), ("reverseLensDistortion", Value::Bool(true)), ("resize", Value::Enum(r))];
        let off = run("ec.distort.opticscompensation", &set(0), img.clone());
        assert_eq!(off.img.width, 32);
        let max2 = run("ec.distort.opticscompensation", &set(1), img.clone());
        assert!(max2.img.width > 32 && max2.img.width <= 32 + 2 * 16 + 2, "{}", max2.img.width);
        let max4 = run("ec.distort.opticscompensation", &set(2), img);
        assert!(max4.img.width >= max2.img.width);
    }

    #[test]
    fn optics_optimal_pixels_fills_or_fits_the_layer() {
        let img = Image::filled(32, 24, [0.5, 0.5, 0.5, 1.0]);
        // Forward: without Optimal Pixels the corners sample outside the layer.
        let plain = run("ec.distort.opticscompensation", &[("fieldOfView", num(120.0))], img.clone());
        assert!(plain.img.get(0, 0)[3] < 0.5);
        let opt = run("ec.distort.opticscompensation", &[("fieldOfView", num(120.0)), ("optimalPixels", Value::Bool(true))], img.clone());
        assert!(opt.img.get(0, 0)[3] > 0.9 && opt.img.get(31, 23)[3] > 0.9);
        // Reverse: everything fits; Resize is ignored.
        let rev = [("fieldOfView", num(120.0)), ("reverseLensDistortion", Value::Bool(true)), ("optimalPixels", Value::Bool(true)), ("resize", Value::Enum(2))];
        let r = run("ec.distort.opticscompensation", &rev, img.clone());
        assert_eq!(r.img.width, 32);
        // The edges of the source now sit inside the layer: empty space around them.
        assert!(r.img.get(0, 0)[3] < 0.5 && r.img.get(16, 12)[3] > 0.9);
    }
}
