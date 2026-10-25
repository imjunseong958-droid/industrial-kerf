//! Coverage-mask operations used for text and shape appearance: strokes (outer / centre / inner)
//! from an exact Euclidean distance transform, Gaussian-like blur for shadows, offsets.

use crate::raster::Mask;

/// Where a stroke sits relative to the shape's edge (Premiere's stroke types).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StrokeKind {
    #[default]
    Outer,
    Center,
    Inner,
}

const INF: f32 = 1e20;

/// 1-D squared distance transform (Felzenszwalb & Huttenlocher) of `f` into `d`.
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let fq = f[q] + (q * q) as f32;
        let mut s;
        loop {
            let p = v[k];
            s = (fq - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        if s <= z[k] {
            // k == 0: q dominates everything so far
            v[0] = q;
            z[0] = -INF;
            z[1] = INF;
            continue;
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = INF;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        d[q] = (dq * dq + f[p]).min(INF);
    }
}

/// Euclidean distance (in pixels, between pixel centres) from every pixel to the nearest pixel
/// where `seed` is true.
fn edt(w: usize, h: usize, seed: impl Fn(usize) -> bool) -> Vec<f32> {
    let mut g = vec![INF; w * h];
    for (i, v) in g.iter_mut().enumerate() {
        if seed(i) {
            *v = 0.0;
        }
    }
    let n = w.max(h);
    let mut f = vec![0.0f32; n];
    let mut d = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v[..h], &mut z[..h + 1]);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..y * w + w]);
        edt_1d(&f[..w], &mut d[..w], &mut v[..w], &mut z[..w + 1]);
        for x in 0..w {
            g[y * w + x] = d[x].sqrt();
        }
    }
    g
}

/// Approximate signed distance to the shape edge: negative inside, positive outside, with
/// sub-pixel refinement from the coverage values.
pub fn signed_distance(m: &Mask) -> Vec<f32> {
    let (w, h) = (m.w, m.h);
    let outside = edt(w, h, |i| m.a[i] >= 0.5);
    let inside = edt(w, h, |i| m.a[i] < 0.5);
    (0..w * h)
        .map(|i| {
            let c = m.a[i];
            if c > 0.0 && c < 1.0 {
                0.5 - c
            } else if c >= 0.5 {
                -(inside[i] - 0.5).max(0.0)
            } else {
                (outside[i] - 0.5).max(0.0)
            }
        })
        .collect()
}

/// Stroke coverage of width `width` pixels around the shape in `m`.
pub fn stroke(m: &Mask, width: f32, kind: StrokeKind) -> Mask {
    let sd = signed_distance(m);
    let a = sd
        .iter()
        .zip(&m.a)
        .map(|(&d, &c)| {
            let v = match kind {
                StrokeKind::Outer => (width + 0.5 - d).clamp(0.0, 1.0).max(c),
                StrokeKind::Center => (width / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0),
                StrokeKind::Inner => (width + 0.5 + d).clamp(0.0, 1.0).min(c),
            };
            if kind == StrokeKind::Outer { v } else { v.min(1.0) }
        })
        .collect();
    Mask { w: m.w, h: m.h, a }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Path, fill};

    #[test]
    fn distance_transform_is_euclidean() {
        let d = edt(9, 9, |i| i == 4 * 9 + 4);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert!((d[4 * 9 + 7] - 3.0).abs() < 1e-5);
        assert!((d[0] - (32.0f32).sqrt()).abs() < 1e-4);
        let none = edt(3, 3, |_| false);
        assert!(none.iter().all(|&v| v > 1e5));
    }

    #[test]
    fn strokes_sit_outside_centre_and_inside() {
        let m = fill(&Path::rect(10.0, 10.0, 30.0, 30.0), 40, 40, 0.0, 0.0);
        let outer = stroke(&m, 4.0, StrokeKind::Outer);
        assert!(outer.get(7, 20) > 0.99, "outer covers 3px outside");
        assert!(outer.get(4, 20) < 0.01);
        assert!(outer.get(20, 20) > 0.99, "outer includes the fill area");
        let inner = stroke(&m, 4.0, StrokeKind::Inner);
        assert!(inner.get(12, 20) > 0.99 && inner.get(7, 20) < 0.01 && inner.get(20, 20) < 0.01);
        let centre = stroke(&m, 4.0, StrokeKind::Center);
        assert!(centre.get(9, 20) > 0.99 && centre.get(11, 20) > 0.99 && centre.get(20, 20) < 0.01 && centre.get(5, 20) < 0.01);
    }
}
