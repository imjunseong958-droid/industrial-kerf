//! Distort effects: inverse-mapped warps (each output pixel samples the source).
use crate::ParamUi;
use crate::Value;
use effectcraft_raster::Image;
use rayon::prelude::*;

use crate::util::layer_rect;
use crate::{Buf, EffectCtx, EffectSpec, num, p, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

/// Resample through `f(x, y) -> source point` (pixel coordinates, centres at +0.5).
fn remap(src: &Image, repeat: bool, f: impl Fn(f64, f64) -> (f64, f64) + Sync) -> Image {
    let mut out = Image::new(src.width, src.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (sx, sy) = f(x as f64 + 0.5, y as f64 + 0.5);
            *px = if repeat { src.sample_bilinear_clamped(sx, sy) } else { src.sample_bilinear(sx, sy) };
        }
    });
    out
}

fn bulge(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let rx = (ctx.params.f("hradius") * b.scale).max(1.0);
    let ry = (ctx.params.f("vradius") * b.scale).max(1.0);
    let c = b.to_px(ctx.params.v2("center"));
    let h = ctx.params.f("height");
    // Taper Radius: higher values make the sides of the bulge shallower (a more peaked profile).
    let taper = 1.0 + ctx.params.f("taperRadius").max(0.0) / 100.0 * 3.0;
    let pin = ctx.params.b("pinAllEdges");
    let (lx, ly, lw, lh) = layer_rect(ctx, &b);
    let edge = (lw.min(lh) * 0.1).max(1.0);
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = ((x - c.0) / rx, (y - c.1) / ry);
        let d2 = dx * dx + dy * dy;
        if d2 >= 1.0 {
            return (x, y);
        }
        let mut amt = h * (1.0 - d2).powf(taper) * 0.5;
        if pin {
            let ex = ((x - lx - 0.5).min(lx + lw - 0.5 - x) / edge).clamp(0.0, 1.0);
            let ey = ((y - ly - 0.5).min(ly + lh - 0.5 - y) / edge).clamp(0.0, 1.0);
            amt *= ex.min(ey);
        }
        let k = 1.0 - amt;
        (c.0 + dx * rx * k, c.1 + dy * ry * k)
    });
    b
}

impl Buf {}

pub fn specs() -> Vec<EffectSpec> {
    let pt = |x, y| Value::Vec2([x, y]);
    vec![spec(
        "ec.distort.bulge",
        "Bulge",
        vec![
            p("hradius", "Horizontal Radius", num(50.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
            p("vradius", "Vertical Radius", num(50.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
            p("center", "Bulge Center", pt(0.5, 0.5), ParamUi::Point),
            p("height", "Bulge Height", num(1.0), slider(-4.0, 4.0, -4.0, 4.0, 2)),
            p("taperRadius", "Taper Radius", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            p("pinAllEdges", "Pin All Edges", Value::Bool(false), ParamUi::Checkbox),
        ],
        bulge,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    fn img(w: u32, h: u32) -> Image {
        crate::util::gen_image(w, h, |x, y| [x as f32 / w as f32, y as f32 / h as f32, ((x * 7 + y * 3) % 11) as f32 / 11.0, 1.0])
    }

    /// Run with Point defaults scaled to the layer (like `instantiate`).
    fn run(id: &str, set: &[(&str, Value)], im: Image) -> Buf {
        let s = crate::find(id).unwrap();
        let ls = [im.width as f64, im.height as f64];
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), crate::default_value(p, ls))).collect() };
        for (k, v) in set {
            assert!(params.values.contains_key(*k), "{id}: unknown param {k}");
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time: 0.0, layer_size: ls, seed: 3, adjustment: false, env: Default::default() };
        crate::apply(s, &ctx, Buf { img: im, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn differs(a: Px4, b: Px4) -> bool {
        (0..4).any(|i| (a[i] - b[i]).abs() > 1e-4)
    }
    type Px4 = [f32; 4];

    #[test]
    fn bulge_taper_and_pin_all_edges() {
        let im = img(40, 40);
        let big = [("hradius", num(40.0)), ("vradius", num(40.0)), ("height", num(2.0))];
        let free = run("ec.distort.bulge", &big, im.clone());
        let mut pinned_set = big.to_vec();
        pinned_set.push(("pinAllEdges", Value::Bool(true)));
        let pinned = run("ec.distort.bulge", &pinned_set, im.clone());
        assert!(differs(free.img.get(20, 0), im.get(20, 0)));
        assert!(!differs(pinned.img.get(20, 0), im.get(20, 0)));
        assert!(!differs(pinned.img.get(0, 20), im.get(0, 20)));
        let mut taper_set = big.to_vec();
        taper_set.push(("taperRadius", num(100.0)));
        let tapered = run("ec.distort.bulge", &taper_set, im.clone());
        assert!(differs(tapered.img.get(30, 20), free.img.get(30, 20)));
    }
}
