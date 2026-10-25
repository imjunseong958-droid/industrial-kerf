//! Stylize effects (batch 2): cartoon, roughened edges, scatter, strobe, relief, painterly
//! strokes, kaleidoscope, tiling, colour emboss, vignette and thresholds.
use crate::ParamUi;
use crate::Value;
use crate::{Buf, EffectCtx, EffectSpec, num, p, slider};
use effectcraft_raster::Image;
use rayon::prelude::*;

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Stylize", params, render, gpu: false, float: true }
}

// ---- Cartoon ----

// ---- Roughen Edges ----

// ---- Scatter ----

// ---- Strobe Light ----

// ---- Texturize ----

// ---- Brush Strokes ----

// ---- CC Kaleida ----

// ---- CC RepeTile ----

// ---- Motion Tile ----

fn motion_tile(ctx: &EffectCtx, b: Buf) -> Buf {
    let s = b.scale;
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    if lw <= 0.0 || lh <= 0.0 {
        return b;
    }
    let tc = ctx.params.v2("tileCenter");
    let tw = (lw * ctx.params.f("tileWidth") / 100.0).max(0.01);
    let th = (lh * ctx.params.f("tileHeight") / 100.0).max(0.01);
    let mirror = ctx.params.b("mirrorEdges");
    let phase = ctx.params.f("phase") / 360.0;
    let hshift = ctx.params.b("horizontalPhaseShift");
    let (nw, nh, noff) = if ctx.adjustment {
        (b.img.width, b.img.height, b.offset)
    } else {
        let ow = (lw * ctx.params.f("outputWidth") / 100.0).max(1.0 / s);
        let oh = (lh * ctx.params.f("outputHeight") / 100.0).max(1.0 / s);
        ((ow * s).round().max(1.0) as u32, (oh * s).round().max(1.0) as u32, [(ow - lw) * 0.5 * s, (oh - lh) * 0.5 * s])
    };
    let mut out = Image::new(nw, nh);
    let src = &b.img;
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let u = (x as f64 + 0.5 - noff[0]) / s;
            let v = (y as f64 + 0.5 - noff[1]) / s;
            let mut tx = (u - tc[0]) / tw + 0.5;
            let mut ty = (v - tc[1]) / th + 0.5;
            if hshift {
                tx += phase * ty.floor();
            } else {
                ty += phase * tx.floor();
            }
            let (i, j) = (tx.floor(), ty.floor());
            let (mut fx, mut fy) = (tx - i, ty - j);
            if mirror {
                if (i as i64).rem_euclid(2) == 1 {
                    fx = 1.0 - fx;
                }
                if (j as i64).rem_euclid(2) == 1 {
                    fy = 1.0 - fy;
                }
            }
            *px = src.sample_bilinear(fx * lw * s + b.offset[0], fy * lh * s + b.offset[1]);
        }
    });
    Buf { img: out, offset: noff, scale: s }
}

// ---- Color Emboss ----

// ---- CC Vignette ----

// ---- CC Threshold / CC Threshold RGB ----

pub fn specs() -> Vec<EffectSpec> {
    let pt = |x, y| Value::Vec2([x, y]);
    vec![spec(
        "ec.stylize.motiontile",
        "Motion Tile",
        vec![
            p("tileCenter", "Tile Center", pt(0.5, 0.5), ParamUi::Point),
            p("tileWidth", "Tile Width", num(100.0), slider(1.0, 1000.0, 1.0, 200.0, 1)),
            p("tileHeight", "Tile Height", num(100.0), slider(1.0, 1000.0, 1.0, 200.0, 1)),
            p("outputWidth", "Output Width", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
            p("outputHeight", "Output Height", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
            p("mirrorEdges", "Mirror Edges", Value::Bool(false), ParamUi::Checkbox),
            p("phase", "Phase", num(0.0), ParamUi::Angle),
            p("horizontalPhaseShift", "Horizontal Phase Shift", Value::Bool(false), ParamUi::Checkbox),
        ],
        motion_tile,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    fn run_at(id: &str, vals: &[(&str, Value)], img: Image, time: f64) -> Buf {
        let s = crate::find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in vals {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: Default::default() };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn ramp(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, [x as f32 / w as f32, y as f32 / h as f32, 0.3, 1.0]);
            }
        }
        img
    }

    #[test]
    fn motion_tile_output_size() {
        let out = run_at(
            "ec.stylize.motiontile",
            &[("outputWidth", num(200.0)), ("outputHeight", num(150.0)), ("tileCenter", Value::Vec2([5.0, 4.0]))],
            ramp(10, 8),
            0.0,
        );
        assert_eq!((out.img.width, out.img.height), (20, 12));
        assert_eq!(out.offset, [5.0, 2.0]);
        // Default tiles: identity inside the layer rect.
        let same = run_at("ec.stylize.motiontile", &[("tileCenter", Value::Vec2([5.0, 4.0]))], ramp(10, 8), 0.0);
        let img = ramp(10, 8);
        for (a, b) in same.img.data.iter().zip(img.data.iter()) {
            assert!((a[0] - b[0]).abs() < 1e-4);
        }
    }
}
