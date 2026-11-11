//! Lights & Glows (and the glow-like Legacy Alpha Glow, the procedural Lens Flare).

use crate::filmcraft::project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::filmcraft::effects::gaussian;

fn blurred(src: &Image, sigma: f32) -> Image {
    let mut g = src.clone();
    gaussian(&mut g, sigma, sigma, false);
    g
}

/// Add `light` (premultiplied linear) to `img`: Add or Screen.
fn add_image(img: &mut Image, light: &Image, gain: [f32; 3], screen: bool) {
    img.px
        .par_chunks_mut(4)
        .zip(light.px.par_chunks(4))
        .for_each(|(p, l)| {
            let add = [l[0] * gain[0], l[1] * gain[1], l[2] * gain[2]];
            if screen {
                let a = [add[0].min(1.0), add[1].min(1.0), add[2].min(1.0)];
                for k in 0..3 {
                    p[k] = p[k] + a[k] - p[k].min(1.0) * a[k];
                }
                p[3] = p[3] + (1.0 - p[3]) * a[0].max(a[1]).max(a[2]);
            } else {
                add_light(p, add);
            }
        });
}

fn saturate_image(img: &mut Image, sat: f32) {
    if (sat - 1.0).abs() < 1e-4 {
        return;
    }
    img.px.par_chunks_mut(4).for_each(|p| {
        let l = luma([p[0], p[1], p[2]]);
        for v in &mut p[..3] {
            *v = (l + (*v - l) * sat).max(0.0);
        }
    });
}

/// Replace the hue of a light buffer with `c` (keeping its luminance), by `k`.
fn tint(img: &mut Image, c: [f32; 3], k: f32) {
    if k <= 0.0 {
        return;
    }
    img.px.par_chunks_mut(4).for_each(|p| {
        let l = luma([p[0], p[1], p[2]]);
        for i in 0..3 {
            p[i] += (l * c[i] * 1.5 - p[i]) * k;
        }
    });
}

/// Sum of `a^k · src(p − k·d)` for k = 0…len (an exponentially decaying streak), by doubling.
fn streak(src: &Image, dir: (f32, f32), len: f32) -> Image {
    let a = (-1.0 / len.max(1.0)).exp();
    let mut s = src.clone();
    let mut n = 1.0f32;
    while n < len * 3.0 {
        let prev = s.clone();
        let k = a.powf(n);
        let (dx, dy) = (dir.0 * n, dir.1 * n);
        let w = s.w;
        s.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let q = prev.sample_bilinear(x as f32 + 0.5 - dx, y as f32 + 0.5 - dy);
                for c in 0..4 {
                    row[x * 4 + c] += q[c] * k;
                }
            }
        });
        n *= 2.0;
    }
    let norm = 1.0 - a;
    s.px.par_iter_mut().for_each(|v| *v *= norm);
    s
}

/// Glint: star streaks from the brightest points.
pub fn glint(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let hi = highlights(img, fv(e, "threshold", cx) / 100.0, 0.08);
    let rays = [2usize, 4, 6, 8][chv(e, "rays").min(3) as usize];
    let len = fv(e, "length", cx) * cx.px_scale;
    let rot = fv(e, "rotation", cx).to_radians();
    let inten = fv(e, "intensity", cx) / 100.0 * 3.0;
    let col = lin(cv(e, "color", cx));
    let colorize = fv(e, "colorize", cx) / 100.0;
    let parts: Vec<Image> = (0..rays)
        .into_par_iter()
        .map(|i| {
            let a = rot + std::f32::consts::TAU * i as f32 / rays as f32;
            streak(&hi, (a.cos(), a.sin()), len * 0.35)
        })
        .collect();
    let mut acc = Image::new(img.w, img.h);
    for p in &parts {
        acc.px
            .par_iter_mut()
            .zip(p.px.par_iter())
            .for_each(|(a, b)| *a += b);
    }
    tint(&mut acc, col, colorize);
    add_image(
        img,
        &acc,
        [
            inten * col[0].max(0.2),
            inten * col[1].max(0.2),
            inten * col[2].max(0.2),
        ],
        false,
    );
}

/// Light Leaks: drifting warm coloured light washes (procedural, seeded, animated by Speed).
pub fn light_leaks(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let c1 = lin(cv(e, "c1", cx));
    let c2 = lin(cv(e, "c2", cx));
    let inten = fv(e, "intensity", cx) / 100.0;
    let scale = (fv(e, "scale", cx) / 100.0).max(0.05) * img.w.max(img.h) as f32 * 0.6;
    let dir = fv(e, "direction", cx).to_radians();
    let t = (cx.seconds * fv(e, "speed", cx) as f64) as f32;
    let seed = fv(e, "seed", cx).max(0.0) as u64;
    let mode = chv(e, "mode");
    let (dx, dy) = (dir.cos(), dir.sin());
    let (w, h) = (img.w as f32, img.h as f32);
    generate(img, |x, y, p| {
        let (u, v) = (x / scale, y / scale);
        // stronger towards the edge the leak enters from
        let along = ((x / w - 0.5) * dx + (y / h - 0.5) * dy) * 2.0;
        let bias = smoothstep(-0.2, 1.0, -along);
        let n = fbm(u + dx * t * 0.3, v + dy * t * 0.3, t * 0.15, 3.0, seed) * 0.5 + 0.5;
        let m = fbm(u * 0.7 + 5.2, v * 0.7, t * 0.1 + 9.0, 2.0, seed) * 0.5 + 0.5;
        let k = smoothstep(0.35, 0.85, n) * bias * inten;
        let light = lerp3(c1, c2, m).map(|c| c * k);
        let base = Image::unpremul(p);
        let o = match mode {
            1 => [base[0] + light[0], base[1] + light[1], base[2] + light[2]],
            2 => blend_simple(4, base, lerp3(base, light.map(|q| q + 0.5), k.min(1.0))),
            _ => blend_simple(2, base, light),
        };
        let a = p[3] + (1.0 - p[3]) * k.min(1.0);
        [o[0] * a, o[1] * a, o[2] * a, a]
    });
}

/// RGB Split: red and blue sampled from opposite offsets (linear or radial from a centre).
pub fn rgb_split(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let amt = fv(e, "amount", cx) * cx.px_scale;
    if amt.abs() < 1e-3 {
        return;
    }
    let radial = chv(e, "mode") == 1;
    let ang = fv(e, "angle", cx).to_radians();
    let c = pv(e, "center", cx, img);
    let blend = fv(e, "blend", cx) / 100.0;
    let maxd = ((img.w as f64).hypot(img.h as f64) / 2.0) as f32;
    let src = img.clone();
    let w = img.w;
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let (ox, oy) = if radial {
                    let (dx, dy) = (px - c.x as f32, py - c.y as f32);
                    (dx / maxd * amt, dy / maxd * amt)
                } else {
                    (ang.cos() * amt, ang.sin() * amt)
                };
                let r = src.sample_bilinear(px - ox, py - oy);
                let g = src.get(x, y);
                let b = src.sample_bilinear(px + ox, py + oy);
                let o = [r[0], g[1], b[2], r[3].max(g[3]).max(b[3])];
                for k in 0..4 {
                    row[x * 4 + k] = o[k] + (g[k] - o[k]) * blend;
                }
            }
        });
}

/// Wonder Glow: a rich multi-scale glow (three radii) with saturation and colour controls.
pub fn wonder_glow(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let mut hi = highlights(img, fv(e, "threshold", cx) / 100.0, 0.15);
    saturate_image(&mut hi, fv(e, "saturation", cx) / 100.0);
    if bv(e, "use_color") {
        tint(&mut hi, lin(cv(e, "color", cx)), 1.0);
    }
    let r = fv(e, "radius", cx) * cx.px_scale * 0.5;
    let inten = fv(e, "intensity", cx) / 100.0;
    let parts: Vec<(Image, f32)> = [(0.25f32, 0.45f32), (1.0, 0.35), (3.0, 0.2)]
        .par_iter()
        .map(|&(k, wgt)| (blurred(&hi, r * k), wgt))
        .collect();
    let mut acc = Image::new(img.w, img.h);
    for (p, wgt) in &parts {
        acc.px
            .par_iter_mut()
            .zip(p.px.par_iter())
            .for_each(|(a, b)| *a += b * wgt);
    }
    add_image(img, &acc, [inten * 1.5; 3], chv(e, "mode") == 1);
}

