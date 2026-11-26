//! Stylize, Perspective and generator effects (plus the Legacy and obsolete generators).

use crate::video::project::EffectInstance;
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
