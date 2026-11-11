//! Distort and Transform effects (geometry).

use crate::filmcraft::project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::filmcraft::effects::warp_opt;

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

/// Magnify (and Magnify (Legacy)): a circular or square loupe.
pub fn magnify(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let c = pv(e, "center", cx, img);
    let m = (fv(e, "magnification", cx) / 100.0).max(0.01) as f64;
    let r = fv(e, "size", cx) * cx.px_scale;
    let fe = fv(e, "feather", cx) * cx.px_scale;
    let op = fv(e, "opacity", cx) / 100.0;
    let square = chv(e, "shape") == 1;
    let mode = chv(e, "mode");
    let src = img.clone();
    let w = img.w;
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
                let d = if square {
                    dx.abs().max(dy.abs())
                } else {
                    dx.hypot(dy)
                } as f32;
                let k = if fe > 0.0 {
                    ((r - d) / fe).clamp(0.0, 1.0)
                } else {
                    (r - d + 0.5).clamp(0.0, 1.0)
                } * op;
                if k <= 0.0 {
                    continue;
                }
                let s = src.sample_bilinear((c.x + dx / m) as f32, (c.y + dy / m) as f32);
                let p = &mut row[x * 4..x * 4 + 4];
                let base = Image::unpremul([p[0], p[1], p[2], p[3]]);
                let sc = Image::unpremul(s);
                let mixed = blend_simple(mode, base, sc);
                let a = s[3];
                let out = [mixed[0] * a, mixed[1] * a, mixed[2] * a, a];
                for i in 0..4 {
                    p[i] += (out[i] - p[i]) * k;
                }
            }
        });
}

/// Transform: affine place, with optional shutter-angle motion blur when the override is on.
pub fn transform_fx(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let fps = cx.frame_rate.max(1.0);
    let angle = fv(e, "shutter_angle", cx);
    let n = if bv(e, "shutter_override") && angle > 0.5 {
        7
    } else {
        1
    };
    let span = (angle as f64 / 360.0) / fps;
    motion_blurred(img, n, |im, k| {
        let t = cx.t + Tick::from_seconds_f64(k * span);
        let cx2 = FxCtx {
            t,
            px_scale: cx.px_scale,
            seconds: cx.seconds + k * span,
            timecode: cx.timecode,
            clip_name: cx.clip_name,
            project: cx.project,
            env: cx.env,
            frame_rate: cx.frame_rate,
            working: cx.working,
        };
        let anchor = crate::filmcraft::effects::point(e, "anchor", &cx2, im);
        let pos = crate::filmcraft::effects::point(e, "position", &cx2, im);
        let sh = crate::filmcraft::effects::f(e, "scale_height", &cx2) as f64 / 100.0;
        let sw = if crate::filmcraft::effects::b(e, "uniform_scale") {
            sh
        } else {
            crate::filmcraft::effects::f(e, "scale_width", &cx2) as f64 / 100.0
        };
        let rot = crate::filmcraft::effects::f(e, "rotation", &cx2) as f64;
        let skew = (crate::filmcraft::effects::f(e, "skew", &cx2) as f64)
            .to_radians()
            .tan();
        let skew_axis = crate::filmcraft::effects::f(e, "skew_axis", &cx2) as f64;
        let op = crate::filmcraft::effects::f(e, "opacity", &cx2) / 100.0;
        let sk = Affine::rotate_deg(skew_axis)
            .then_apply(&Affine {
                a: 1.0,
                b: 0.0,
                c: skew,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            })
            .then_apply(&Affine::rotate_deg(-skew_axis));
        let m = Affine::translate(pos.x, pos.y)
            .then_apply(&Affine::rotate_deg(rot))
            .then_apply(&sk)
            .then_apply(&Affine::scale(sw, sh))
            .then_apply(&Affine::translate(-anchor.x, -anchor.y));
        affine_warp(im, &m);
        if op < 1.0 - 1e-5 {
            im.px.par_chunks_mut(4).for_each(|p| {
                for v in p.iter_mut() {
                    *v *= op;
                }
            });
        }
    });
}

/// Accumulate `n` renders of `f(img, k)` for k in −0.5..0.5 (simple motion blur).
fn motion_blurred(img: &mut Image, n: usize, f: impl Fn(&mut Image, f64) + Sync) {
    if n <= 1 {
        f(img, 0.0);
        return;
    }
    let src = img.clone();
    let parts: Vec<Image> = (0..n)
        .into_par_iter()
        .map(|i| {
            let mut c = src.clone();
            f(&mut c, i as f64 / (n - 1) as f64 - 0.5);
            c
        })
        .collect();
    let inv = 1.0 / n as f32;
    img.px
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, v)| *v = parts.iter().map(|p| p.px[i]).sum::<f32>() * inv);
}

/// Spacer: insets the layer inside margins (scaled to fit), with rounded corners and an optional
/// background fill.
pub fn spacer(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let k = cx.px_scale as f64;
    let l = fv(e, "left", cx) as f64 * k;
    let (t, r, b) = if bv(e, "uniform") {
        (l, l, l)
    } else {
        (
            fv(e, "top", cx) as f64 * k,
            fv(e, "right", cx) as f64 * k,
            fv(e, "bottom", cx) as f64 * k,
        )
    };
    let (w, h) = (img.w as f64, img.h as f64);
    let (bw, bh) = ((w - l - r).max(1.0), (h - t - b).max(1.0));
    let s = (bw / w).min(bh / h);
    let (dw, dh) = (w * s, h * s);
    let (x0, y0) = (l + (bw - dw) / 2.0, t + (bh - dh) / 2.0);
    let m = Affine::translate(x0, y0).then_apply(&Affine::scale(s, s));
    let mut out = img.transformed(img.w, img.h, &m);
    let rad = (fv(e, "radius", cx) as f64 * k).min(dw.min(dh) / 2.0);
    if rad > 0.0 {
        round_rect_alpha(&mut out, x0, y0, x0 + dw, y0 + dh, rad as f32, 0.0);
    }
    if bv(e, "fill") {
        let c = lin(cv(e, "color", cx));
        out.px.par_chunks_mut(4).for_each(|p| {
            let k = 1.0 - p[3];
            p[0] += c[0] * k;
            p[1] += c[1] * k;
            p[2] += c[2] * k;
            p[3] = 1.0;
        });
    }
    *img = out;
}

/// Signed distance from (px, py) to a rounded rectangle (negative inside).
#[inline]
pub(crate) fn round_rect_sdf(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (qx, qy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - r
}

/// Multiply alpha by the coverage of a rounded rectangle (feathered inward by `feather`).
fn round_rect_alpha(img: &mut Image, x0: f64, y0: f64, x1: f64, y1: f64, r: f32, feather: f32) {
    let w = img.w;
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let d = round_rect_sdf(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    x0 as f32,
                    y0 as f32,
                    x1 as f32,
                    y1 as f32,
                    r,
                );
                let a = if feather > 0.0 {
                    (-d / feather).clamp(0.0, 1.0)
                } else {
                    (0.5 - d).clamp(0.0, 1.0)
                };
                if a < 1.0 {
                    for v in &mut row[x * 4..x * 4 + 4] {
                        *v *= a;
                    }
                }
            }
        });
}

/// Rounded Crop: crop edges with rounded corners, feather and an optional border.
pub fn rounded_crop(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let (w, h) = (img.w as f32, img.h as f32);
    let (l, t, r, b) = (
        fv(e, "left", cx) / 100.0 * w,
        fv(e, "top", cx) / 100.0 * h,
        w - fv(e, "right", cx) / 100.0 * w,
        h - fv(e, "bottom", cx) / 100.0 * h,
    );
    let rad = (fv(e, "radius", cx) * cx.px_scale).min(((r - l).min(b - t) / 2.0).max(0.0));
    let fe = fv(e, "feather", cx) * cx.px_scale;
    let border = fv(e, "border", cx) * cx.px_scale;
    let bc = lin(cv(e, "border_color", cx));
    let wi = img.w;
    img.px
        .par_chunks_mut(wi * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..wi {
                let d = round_rect_sdf(x as f32 + 0.5, y as f32 + 0.5, l, t, r, b, rad);
                let p = &mut row[x * 4..x * 4 + 4];
                if border > 0.0 {
                    // coverage of the band −border < d < 0 just inside the edge
                    let k = (0.5 - d).clamp(0.0, 1.0) - (0.5 - d - border).clamp(0.0, 1.0);
                    if k > 0.0 {
                        over(p, [bc[0] * k, bc[1] * k, bc[2] * k, k]);
                    }
                }
                let a = if fe > 0.0 {
                    (-d / fe).clamp(0.0, 1.0)
                } else {
                    (0.5 - d).clamp(0.0, 1.0)
                };
                if a < 1.0 {
                    for v in p.iter_mut() {
                        *v *= a;
                    }
                }
            }
        });
}

/// Auto Align: moves the layer's visible content (alpha bounding box) to the chosen frame edge
/// or centre.
pub fn auto_align(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let (hz, vt) = (chv(e, "horizontal"), chv(e, "vertical"));
    if hz == 0 && vt == 0 {
        return;
    }
    let w = img.w;
    let bounds = img
        .px
        .par_chunks(w * 4)
        .enumerate()
        .filter_map(|(y, row)| {
            let xs: Vec<usize> = (0..w).filter(|&x| row[x * 4 + 3] > 0.01).collect();
            Some((xs.first().copied()?, xs.last().copied()?, y))
        })
        .fold(
            || (usize::MAX, 0usize, usize::MAX, 0usize),
            |a, (x0, x1, y)| (a.0.min(x0), a.1.max(x1), a.2.min(y), a.3.max(y)),
        )
        .reduce(
            || (usize::MAX, 0usize, usize::MAX, 0usize),
            |a, b| (a.0.min(b.0), a.1.max(b.1), a.2.min(b.2), a.3.max(b.3)),
        );
    if bounds.0 == usize::MAX {
        return;
    }
    let (x0, x1, y0, y1) = (
        bounds.0 as f64,
        bounds.1 as f64 + 1.0,
        bounds.2 as f64,
        bounds.3 as f64 + 1.0,
    );
    let (mx, my) = (
        (fv(e, "margin_x", cx) * cx.px_scale) as f64,
        (fv(e, "margin_y", cx) * cx.px_scale) as f64,
    );
    let (fw, fh) = (img.w as f64, img.h as f64);
    let dx = match hz {
        1 => mx - x0,
        2 => (fw - (x1 - x0)) / 2.0 - x0,
        3 => fw - mx - x1,
        _ => 0.0,
    };
    let dy = match vt {
        1 => my - y0,
        2 => (fh - (y1 - y0)) / 2.0 - y0,
        3 => fh - my - y1,
        _ => 0.0,
    };
    affine_warp(img, &Affine::translate(dx.round(), dy.round()));
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
