//! Keying effects: colour, luma, difference, range and screen keyers plus spill suppression and
//! matte clean-up. Written from public behaviour descriptions and the standard colour-difference /
//! blue-screen matting literature (Vlahos; Smith & Blinn, "Blue Screen Matting").
use crate::ParamUi;
use crate::Value;
use kerf_raster::gaussian_blur;
use rayon::prelude::*;

use crate::util::{gauss_plane, premul, unpremul, Plane};
use crate::{col, num, p, popup, slider, Buf, EffectCtx, EffectSpec};

fn spec(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    params: Vec<crate::ParamSpec>,
    render: crate::RenderFn,
) -> EffectSpec {
    EffectSpec {
        id,
        name,
        category,
        params,
        render,
        gpu: false,
        float: true,
    }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

/// Index of the largest component and the other two.
fn primary(c: [f32; 4]) -> (usize, usize, usize) {
    if c[1] >= c[0] && c[1] >= c[2] {
        (1, 0, 2)
    } else if c[2] >= c[0] && c[2] >= c[1] {
        (2, 0, 1)
    } else {
        (0, 1, 2)
    }
}

// ---- Linear Color Key ----

// ---- Color Key (obsolete) ----

// ---- Luma Key ----

// ---- Color Difference Key ----

// ---- Color Range ----

// ---- Extract ----

// ---- Screen Key (Keylight-style) ----

// ---- Spill suppression ----

const LUMA_W: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn advanced_spill(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ultra = ctx.params.e("method") == 1;
    let pi = if ultra {
        primary(ctx.params.color("ultraSettings/keyColor")).0
    } else {
        // Dominant key primary: the channel with the largest total excess over the other two.
        let ex = b
            .img
            .data
            .par_iter()
            .map(|px| {
                let (c, a) = unpremul(*px);
                [
                    (c[0] - c[1].max(c[2])).max(0.0) * a,
                    (c[1] - c[0].max(c[2])).max(0.0) * a,
                    (c[2] - c[0].max(c[1])).max(0.0) * a,
                ]
            })
            .reduce(|| [0.0; 3], |x, y| [x[0] + y[0], x[1] + y[1], x[2] + y[2]]);
        if ex[1] >= ex[0] && ex[1] >= ex[2] {
            1
        } else if ex[2] >= ex[0] {
            2
        } else {
            0
        }
    };
    let (o1, o2) = match pi {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let amt = ctx.params.f("suppression") as f32 / 100.0;
    let range = ctx.params.f("ultraSettings/spillRange") as f32 / 100.0;
    let luma = ctx.params.f("ultraSettings/lumaCorrection") as f32 / 100.0;
    // Ultra: Tolerance limits suppression to hues near the key colour; Desaturate takes the
    // colour out of suppressed pixels; Spill Color Correction puts the removed spill back as
    // neutral light (so suppressed areas keep their brightness without the cast).
    let key = ctx.params.color("ultraSettings/keyColor");
    let key_hue = kerf_color::rgb_to_hsl(key[0], key[1], key[2])[0];
    let tol = if ultra {
        ctx.params
            .get("ultraSettings/tolerance")
            .map(Value::as_f64)
            .unwrap_or(100.0) as f32
            / 100.0
    } else {
        1.0
    };
    let desat = if ultra {
        ctx.params.f("ultraSettings/desaturate") as f32 / 100.0
    } else {
        0.0
    };
    let neutral = if ultra {
        ctx.params.f("ultraSettings/spillColorCorrection") as f32 / 100.0
    } else {
        0.0
    };
    b.img.map_straight(|mut c| {
        let w = if tol < 1.0 {
            let h = kerf_color::rgb_to_hsl(c[0], c[1], c[2])[0];
            let d = (h - key_hue).abs();
            let d = d.min(1.0 - d);
            (1.0 - d / (tol * 0.5).max(1e-4)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mx = c[o1].max(c[o2]);
        let avg = 0.5 * (c[o1] + c[o2]);
        let limit = mx + (avg - mx) * range;
        let spill = (c[pi] - limit).max(0.0) * amt * w;
        c[pi] -= spill;
        let back = spill * LUMA_W[pi] * luma + spill * neutral / 3.0;
        let mut c = c.map(|v| v + back);
        if desat > 0.0 && spill > 0.0 {
            let l = kerf_color::luma709(c[0], c[1], c[2]);
            let k = (desat * (spill * 4.0).min(1.0)).min(1.0);
            c = c.map(|v| v + (l - v) * k);
        }
        c
    });
    b
}

// ---- Key Cleaner (approximation) ----

fn key_cleaner(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("additionalEdgeRadius") * b.scale).max(0.0);
    let contrast = ctx.params.f("alphaContrast") as f32 / 100.0;
    let strength = ctx.params.f("strength") as f32 / 100.0;
    let mut a = Plane::alpha(&b.img);
    if ctx.params.b("reduceChatter") {
        a = gauss_plane(&a, 1.0, 1.0).map(|v| ((v - 0.5) * 1.5 + 0.5).clamp(0.0, 1.0));
    }
    let est = if r > 0.0 && strength > 0.0 {
        Some(gaussian_blur(&b.img, r * 0.5, r * 0.5, true))
    } else {
        None
    };
    b.img
        .data
        .par_iter_mut()
        .zip(a.data.par_iter())
        .enumerate()
        .for_each(|(i, (px, &na))| {
            let (mut c, a0) = unpremul(*px);
            if let Some(e) = &est
                && a0 > 1e-4
                && a0 < 0.999
            {
                let (ec, ea) = unpremul(e.data[i]);
                if ea > 1e-4 {
                    let t = strength * (1.0 - a0);
                    c = [0, 1, 2].map(|k| c[k] + (ec[k] - c[k]) * t);
                }
            }
            let na = ((na - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            let na = if a0 <= 1e-6 { 0.0 } else { na };
            *px = premul(c, na);
        });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec(
            "ec.key.advancedspill",
            "Advanced Spill Suppressor",
            "Keying",
            vec![
                p(
                    "method",
                    "Method",
                    Value::Enum(0),
                    popup(&["Standard", "Ultra"]),
                ),
                p(
                    "suppression",
                    "Suppression",
                    num(100.0),
                    slider(0.0, 200.0, 0.0, 100.0, 0),
                ),
                p(
                    "ultraSettings/keyColor",
                    "Key Color",
                    col(0.0, 1.0, 0.0),
                    ParamUi::Color,
                ),
                p("ultraSettings/tolerance", "Tolerance", num(50.0), pct()),
                p("ultraSettings/desaturate", "Desaturate", num(0.0), pct()),
                p("ultraSettings/spillRange", "Spill Range", num(50.0), pct()),
                p(
                    "ultraSettings/spillColorCorrection",
                    "Spill Color Correction",
                    num(0.0),
                    pct(),
                ),
                p(
                    "ultraSettings/lumaCorrection",
                    "Luma Correction",
                    num(0.0),
                    pct(),
                ),
            ],
            advanced_spill,
        ),
        spec(
            "ec.key.keycleaner",
            "Key Cleaner",
            "Keying",
            vec![
                p(
                    "additionalEdgeRadius",
                    "Additional Edge Radius",
                    num(3.0),
                    slider(0.0, 100.0, 0.0, 20.0, 1),
                ),
                p(
                    "reduceChatter",
                    "Reduce Chatter",
                    Value::Bool(false),
                    ParamUi::Checkbox,
                ),
                p(
                    "alphaContrast",
                    "Alpha Contrast",
                    num(100.0),
                    slider(100.0, 500.0, 100.0, 300.0, 0),
                ),
                p("strength", "Strength", num(100.0), pct()),
            ],
            key_cleaner,
        ),
    ]
}
