//! Stylize, Perspective and generator effects (plus the Legacy and obsolete generators).

use std::collections::VecDeque;

use crate::filmcraft::project::EffectInstance;
use rayon::prelude::*;

use super::*;

/// 1-D squared Euclidean distance transform: lower envelope of parabolas (Felzenszwalb &
/// Huttenlocher, "Distance Transforms of Sampled Functions", 2012).
fn edt_1d(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let isect = |q: usize, p: usize| {
        ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * q as f64 - 2.0 * p as f64)
    };
    for q in 1..n {
        let mut s = isect(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = isect(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *o = (q as f64 - p as f64).powi(2) + f[p];
    }
}

/// Euclidean distance from every pixel to the nearest pixel where `inside` is true.
pub(crate) fn distance_to(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    let big = 1e12f64;
    let mut g: Vec<f64> = inside.iter().map(|&b| if b { 0.0 } else { big }).collect();
    g.par_chunks_mut(w).for_each(|row| {
        let f = row.to_vec();
        edt_1d(&f, row);
    });
    let cols: Vec<Vec<f64>> = (0..w)
        .into_par_iter()
        .map(|x| {
            let f: Vec<f64> = (0..h).map(|y| g[y * w + x]).collect();
            let mut o = vec![0f64; h];
            edt_1d(&f, &mut o);
            o
        })
        .collect();
    let mut out = vec![0f32; w * h];
    for (x, c) in cols.iter().enumerate() {
        for y in 0..h {
            out[y * w + x] = c[y].sqrt() as f32;
        }
    }
    out
}

/// Stroke: a solid outline around the layer's alpha (outside, centred or inside the edge).
pub fn stroke(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let width = fv(e, "width", cx) * cx.px_scale;
    if width <= 0.0 {
        return;
    }
    let col = lin(cv(e, "color", cx));
    let op = fv(e, "opacity", cx) / 100.0;
    let pos = chv(e, "position");
    let only = bv(e, "only");
    // pad by one pixel so the frame border counts as an edge
    let (w, h) = (img.w + 2, img.h + 2);
    let mut inside = vec![false; w * h];
    for y in 0..img.h {
        for x in 0..img.w {
            inside[(y + 1) * w + x + 1] = img.px[(y * img.w + x) * 4 + 3] > 0.5;
        }
    }
    let outside: Vec<bool> = inside.iter().map(|b| !b).collect();
    let d_out = distance_to(&inside, w, h);
    let d_in = distance_to(&outside, w, h);
    let iw = img.w;
    img.px
        .par_chunks_mut(iw * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..iw {
                let i = (y + 1) * w + x + 1;
                // signed distance to the edge, positive outside (pixel centres sit ±0.5 from it)
                let s = if inside[i] {
                    -(d_in[i] - 0.5)
                } else {
                    d_out[i] - 0.5
                };
                let (lo, hi) = match pos {
                    0 => (0.0, width),
                    1 => (-width / 2.0, width / 2.0),
                    _ => (-width, 0.0),
                };
                let k = ((s - lo + 0.5).clamp(0.0, 1.0)).min((hi - s + 0.5).clamp(0.0, 1.0)) * op;
                let p = &mut row[x * 4..x * 4 + 4];
                if only {
                    p.copy_from_slice(&[col[0] * k, col[1] * k, col[2] * k, k]);
                } else if k > 0.0 {
                    over(p, [col[0] * k, col[1] * k, col[2] * k, k]);
                }
            }
        });
}

/// Gradient (26.2): linear / radial / reflected / diamond ramp with midpoint and scatter.
pub fn gradient(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let s = pv(e, "start", cx, img);
    let en = pv(e, "end", cx, img);
    let sc = cv(e, "start_color", cx);
    let ec = cv(e, "end_color", cx);
    let shape = chv(e, "shape");
    let scatter = fv(e, "scatter", cx) / 100.0;
    let mid = (fv(e, "midpoint", cx) / 100.0).clamp(0.01, 0.99);
    let gamma = (0.5f32).ln() / mid.ln();
    let blend = fv(e, "blend", cx) / 100.0;
    let (vx, vy) = ((en.x - s.x) as f32, (en.y - s.y) as f32);
    let l2 = (vx * vx + vy * vy).max(1e-6);
    let l = l2.sqrt();
    generate(img, |x, y, p| {
        let (dx, dy) = (x - s.x as f32, y - s.y as f32);
        let along = (dx * vx + dy * vy) / l2;
        let across = (-dx * vy + dy * vx) / l2;
        let mut t = match shape {
            1 => (dx * dx + dy * dy).sqrt() / l,
            2 => along.abs(),
            3 => along.abs() + across.abs(),
            _ => along,
        };
        if scatter > 0.0 {
            t += (hash01(x as i64, y as i64, 0, 13) - 0.5) * scatter * 0.1;
        }
        let t = t.clamp(0.0, 1.0).powf(gamma);
        let c = dec(lerp3([sc[0], sc[1], sc[2]], [ec[0], ec[1], ec[2]], t));
        let a = sc[3] + (ec[3] - sc[3]) * t;
        let g = [c[0] * a, c[1] * a, c[2] * a, a];
        [0, 1, 2, 3].map(|k| g[k] + (p[k] - g[k]) * blend)
    });
}


/// Checkerboard (obsolete): a checkerboard of Color and transparency.
pub fn checkerboard(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let a = pv(e, "anchor", cx, img);
    let cw = (fv(e, "width", cx) * cx.px_scale).max(1.0);
    let ch = if bv(e, "square") {
        cw
    } else {
        (fv(e, "height", cx) * cx.px_scale).max(1.0)
    };
    let fe = fv(e, "feather", cx) * cx.px_scale;
    let col = lin(cv(e, "color", cx));
    let op = fv(e, "opacity", cx) / 100.0;
    let mode = chv(e, "mode");
    generate(img, |x, y, p| {
        let (u, v) = ((x - a.x as f32) / cw, (y - a.y as f32) / ch);
        let par = ((u.floor() as i64 + v.floor() as i64).rem_euclid(2)) as f32;
        let edge = {
            let du = (u - u.round()).abs() * cw;
            let dv = (v - v.round()).abs() * ch;
            if fe > 0.0 {
                smoothstep(0.0, fe, du.min(dv))
            } else {
                1.0
            }
        };
        let k = par * edge * op;
        match mode {
            0 => [col[0] * k, col[1] * k, col[2] * k, k],
            1 => {
                let mut q = p;
                over(&mut q, [col[0] * k, col[1] * k, col[2] * k, k]);
                q
            }
            m => {
                let base = Image::unpremul(p);
                let bm = [0u32, 0, 1, 3, 2, 4][m.min(5) as usize];
                let o = lerp3(base, blend_simple(bm, base, col), k);
                [o[0] * p[3], o[1] * p[3], o[2] * p[3], p[3]]
            }
        }
    });
}

/// Paint Bucket (obsolete): flood fill from Fill Point within Tolerance.
pub fn paint_bucket(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let pt = pv(e, "point", cx, img);
    let (w, h) = (img.w, img.h);
    let (sx, sy) = (pt.x.floor() as isize, pt.y.floor() as isize);
    if sx < 0 || sy < 0 || sx as usize >= w || sy as usize >= h {
        return;
    }
    let sel = chv(e, "selector");
    let tol = fv(e, "tolerance", cx) / 255.0;
    let col = lin(cv(e, "color", cx));
    let op = fv(e, "opacity", cx) / 100.0;
    let mode = chv(e, "mode");
    let px = |i: usize| -> [f32; 4] {
        let p = &img.px[i * 4..i * 4 + 4];
        let c = enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
        [c[0], c[1], c[2], p[3]]
    };
    let seed = px(sy as usize * w + sx as usize);
    let similar = |q: [f32; 4]| -> bool {
        match sel {
            0 => (0..4).all(|k| (q[k] - seed[k]).abs() <= tol),
            1 => (0..3).all(|k| (q[k] - seed[k]).abs() <= tol),
            2 => q[3] <= tol.max(1e-3),
            _ => (q[3] - seed[3]).abs() <= tol,
        }
    };
    let mut fill = vec![0f32; w * h];
    let mut queue = VecDeque::new();
    let start = sy as usize * w + sx as usize;
    if similar(seed) {
        fill[start] = 1.0;
        queue.push_back(start);
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let mut visit = |j: usize| {
            if fill[j] == 0.0 && similar(px(j)) {
                fill[j] = 1.0;
                queue.push_back(j);
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < w {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - w);
        }
        if y + 1 < h {
            visit(i + w);
        }
    }
    if bv(e, "invert") {
        fill.iter_mut().for_each(|v| *v = 1.0 - *v);
    }
    let fill = blur_plane(&fill, w, h, 0.5);
    img.px
        .par_chunks_mut(4)
        .zip(fill.par_iter())
        .for_each(|(p, &f)| {
            let k = f * op;
            if k <= 0.0 {
                return;
            }
            let s = [col[0] * k, col[1] * k, col[2] * k, k];
            match mode {
                1 => {
                    let mut q = s;
                    over(&mut q, [p[0], p[1], p[2], p[3]]);
                    p.copy_from_slice(&q);
                }
                2..=4 => {
                    let base = Image::unpremul([p[0], p[1], p[2], p[3]]);
                    let bm = [0u32, 0, 1, 3, 2][mode as usize];
                    let o = lerp3(base, blend_simple(bm, base, col), k);
                    for c in 0..3 {
                        p[c] = o[c] * p[3];
                    }
                }
                _ => over(p, s),
            }
        });
}

