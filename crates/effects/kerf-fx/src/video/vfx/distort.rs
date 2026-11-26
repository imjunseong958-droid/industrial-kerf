//! Distort and Transform effects (geometry).

use crate::video::project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::video::effects::warp_opt;

/// 3×3 projective matrix, row-major, mapping (x, y, 1).
pub(crate) type H3 = [f64; 9];

/// Heckbert's square-to-quad: maps the unit square's (0,0),(1,0),(1,1),(0,1) to `q`'s points.
pub(crate) fn square_to_quad(q: [Vec2; 4]) -> H3 {
    let [p0, p1, p2, p3] = q;
    let sx = p0.x - p1.x + p2.x - p3.x;
    let sy = p0.y - p1.y + p2.y - p3.y;
    if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
        return [
            p1.x - p0.x,
            p2.x - p1.x,
            p0.x,
            p1.y - p0.y,
            p2.y - p1.y,
            p0.y,
            0.0,
            0.0,
            1.0,
        ];
    }
    let (dx1, dx2, dy1, dy2) = (p1.x - p2.x, p3.x - p2.x, p1.y - p2.y, p3.y - p2.y);
    let det = dx1 * dy2 - dx2 * dy1;
    let g = (sx * dy2 - dx2 * sy) / det;
    let h = (dx1 * sy - sx * dy1) / det;
    [
        p1.x - p0.x + g * p1.x,
        p3.x - p0.x + h * p3.x,
        p0.x,
        p1.y - p0.y + g * p1.y,
        p3.y - p0.y + h * p3.y,
        p0.y,
        g,
        h,
        1.0,
    ]
}

pub(crate) fn h_inverse(m: &H3) -> Option<H3> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let co = [
        e * i - f * h,
        -(d * i - f * g),
        d * h - e * g,
        -(b * i - c * h),
        a * i - c * g,
        -(a * h - b * g),
        b * f - c * e,
        -(a * f - c * d),
        a * e - b * d,
    ];
    let det = a * co[0] + b * co[1] + c * co[2];
    if det.abs() < 1e-18 {
        return None;
    }
    Some([
        co[0] / det,
        co[3] / det,
        co[6] / det,
        co[1] / det,
        co[4] / det,
        co[7] / det,
        co[2] / det,
        co[5] / det,
        co[8] / det,
    ])
}

#[inline]
pub(crate) fn h_apply(m: &H3, x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[6] * x + m[7] * y + m[8];
    if w.abs() < 1e-12 {
        return None;
    }
    Some((
        (m[0] * x + m[1] * y + m[2]) / w,
        (m[3] * x + m[4] * y + m[5]) / w,
    ))
}

/// Corner Pin: the layer's corners moved to four points (a projective warp).
pub fn corner_pin(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let (w, h) = (img.w as f64, img.h as f64);
    let q = [
        pv(e, "upper_left", cx, img),
        pv(e, "upper_right", cx, img),
        pv(e, "lower_right", cx, img),
        pv(e, "lower_left", cx, img),
    ];
    let ident = [
        Vec2::new(0.0, 0.0),
        Vec2::new(w, 0.0),
        Vec2::new(w, h),
        Vec2::new(0.0, h),
    ];
    if q.iter().zip(&ident).all(|(a, b)| (*a - *b).length() < 1e-6) {
        return;
    }
    let Some(inv) = h_inverse(&square_to_quad(q)) else {
        return;
    };
    warp_opt(img, |x, y| {
        let (u, v) = h_apply(&inv, x, y)?;
        (u >= -1e-3 && v >= -1e-3 && u <= 1.0 + 1e-3 && v <= 1.0 + 1e-3).then_some((u * w, v * h))
    });
}

/// Mosaic (and Mosaic (Legacy)): solid blocks; Softness blends towards a smooth interpolation of
/// the block colours.
pub fn mosaic(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let bx = fv(e, "horizontal", cx).max(1.0) as usize;
    let by = fv(e, "vertical", cx).max(1.0) as usize;
    let soft = (fv(e, "softness", cx) / 100.0).clamp(0.0, 1.0);
    let (w, h) = (img.w, img.h);
    let bw = (w as f32 / bx as f32).max(1.0);
    let bh = (h as f32 / by as f32).max(1.0);
    let (nx, ny) = (
        (w as f32 / bw).ceil() as usize,
        (h as f32 / bh).ceil() as usize,
    );
    let mut small = Image::new(nx.max(1), ny.max(1));
    for j in 0..small.h {
        for i in 0..small.w {
            let p = img.sample_bilinear_clamped((i as f32 + 0.5) * bw, (j as f32 + 0.5) * bh);
            let k = (j * small.w + i) * 4;
            small.px[k..k + 4].copy_from_slice(&p);
        }
    }
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let (u, v) = ((x as f32 + 0.5) / bw, (y as f32 + 0.5) / bh);
                // tile index as in the original Mosaic (pixel x belongs to tile floor(x / bw))
                let (ti, tj) = ((x as f32 / bw) as usize, (y as f32 / bh) as usize);
                let sharp = small.get(ti.min(small.w - 1), tj.min(small.h - 1));
                let p = if soft > 0.0 {
                    let sm = small.sample_bilinear_clamped(u, v);
                    [0, 1, 2, 3].map(|k| sharp[k] + (sm[k] - sharp[k]) * soft)
                } else {
                    sharp
                };
                row[x * 4..x * 4 + 4].copy_from_slice(&p);
            }
        });
}
