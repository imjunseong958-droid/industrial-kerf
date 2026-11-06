//! What mattes are measured and smoothed with: distance transforms (Felzenszwalb & Huttenlocher
//! 2012) and a Gaussian from repeated box means.

use rayon::prelude::*;

const BIG: f32 = 1e12;

/// 1-D squared distance transform of `f` (lower envelope of parabolas).
fn dt1(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let ff = |q: usize| f[q] as f64 + (q * q) as f64;
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = (ff(q) - ff(v[k])) / (2.0 * (q as f64 - v[k] as f64));
        while s <= z[k] {
            if k == 0 {
                break;
            }
            k -= 1;
            s = (ff(q) - ff(v[k])) / (2.0 * (q as f64 - v[k] as f64));
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, dq) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dd = q as f64 - p as f64;
        *dq = (dd * dd + f[p] as f64) as f32;
    }
}

/// Euclidean distance from every pixel to the nearest pixel where `seed` is true (BIG.sqrt()
/// when there is none).
pub fn distance(seed: &[bool], w: usize, h: usize) -> Vec<f32> {
    if w == 0 || h == 0 {
        return vec![];
    }
    let mut g: Vec<f32> = seed.iter().map(|s| if *s { 0.0 } else { BIG }).collect();
    // Columns.
    let cols: Vec<Vec<f32>> = (0..w)
        .into_par_iter()
        .map(|x| {
            let f: Vec<f32> = (0..h).map(|y| g[y * w + x]).collect();
            let mut d = vec![0.0; h];
            let (mut v, mut z) = (vec![0usize; h], vec![0.0f64; h + 1]);
            dt1(&f, &mut d, &mut v, &mut z);
            d
        })
        .collect();
    for (x, c) in cols.iter().enumerate() {
        for y in 0..h {
            g[y * w + x] = c[y];
        }
    }
    // Rows.
    g.par_chunks_mut(w).for_each(|row| {
        let f = row.to_vec();
        let (mut v, mut z) = (vec![0usize; w], vec![0.0f64; w + 1]);
        dt1(&f, row, &mut v, &mut z);
    });
    g.par_iter_mut().for_each(|v| *v = v.sqrt());
    g
}

/// Signed distance to a binary matte's edge: positive inside (distance to the nearest
/// background pixel centre, minus ½), negative outside.
pub fn signed_distance(m: &[u8], w: usize, h: usize) -> Vec<f32> {
    let fg: Vec<bool> = m.iter().map(|v| *v != 0).collect();
    let bg: Vec<bool> = m.iter().map(|v| *v == 0).collect();
    let din = distance(&bg, w, h);
    let dout = distance(&fg, w, h);
    m.iter().enumerate().map(|(i, v)| if *v != 0 { din[i] - 0.5 } else { 0.5 - dout[i] }).collect()
}

/// Mean over the (2r+1)² window (clamped at the edges: averages only the pixels inside).
pub fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if w == 0 || h == 0 {
        return vec![];
    }
    // Horizontal window sums per row.
    let mut tmp = vec![0.0f64; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let s = &src[y * w..y * w + w];
        let mut acc = 0.0f64;
        for x in 0..r.min(w) {
            acc += s[x] as f64;
        }
        for (x, o) in row.iter_mut().enumerate() {
            if x + r < w {
                acc += s[x + r] as f64;
            }
            if x > r {
                acc -= s[x - r - 1] as f64;
            }
            *o = acc;
        }
    });
    // Vertical prefix sums (row by row, cache friendly), then window differences.
    let mut pre = vec![0.0f64; w * (h + 1)];
    for y in 0..h {
        let (done, rest) = pre.split_at_mut((y + 1) * w);
        let prev = &done[y * w..];
        let cur = &mut rest[..w];
        let t = &tmp[y * w..y * w + w];
        for x in 0..w {
            cur[x] = prev[x] + t[x];
        }
    }
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let a = y.saturating_sub(r);
        let b = (y + r + 1).min(h);
        let cy = (b - a) as f64;
        for (x, o) in row.iter_mut().enumerate() {
            let cx = ((x + r + 1).min(w) - x.saturating_sub(r)) as f64;
            *o = ((pre[b * w + x] - pre[a * w + x]) / (cx * cy)) as f32;
        }
    });
    out
}

/// Approximate Gaussian blur (three box passes) with standard deviation `sigma` pixels.
pub fn gauss(src: &[f32], w: usize, h: usize, sigma: f64) -> Vec<f32> {
    if sigma < 0.3 {
        return src.to_vec();
    }
    // Box radius whose three passes match the variance: 3·((2r+1)² − 1)/12 = σ².
    let r = (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize;
    let a = box_mean(src, w, h, r);
    let b = box_mean(&a, w, h, r);
    box_mean(&b, w, h, r)
}

/// Sample a float plane bilinearly at continuous coordinates (pixel centres at +0.5), clamped.
pub fn sample(p: &[f32], w: usize, h: usize, x: f64, y: f64) -> f32 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let fx = (x - 0.5).clamp(0.0, (w - 1) as f64);
    let fy = (y - 0.5).clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
    let g = |x: usize, y: usize| p[y * w + x];
    let top = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
    let bot = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
    top + (bot - top) * ty
}
