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
