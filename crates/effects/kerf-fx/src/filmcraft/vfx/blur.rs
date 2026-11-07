//! Blur & Sharpen effects.

use crate::filmcraft::project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::filmcraft::effects::gaussian;

/// One row of pixels of what an iris covers around the pixel it lands on.
#[derive(Clone, Copy, Debug)]
pub(super) struct IrisRow {
    /// How far the row is from the pixel, in rows.
    dy: isize,
    /// Where it starts and ends, from the pixel, in pixels.
    from: f64,
    to: f64,
    /// How much of the row's height the iris covers.
    weight: f64,
}

/// How far an iris reaches to its left and right at height `y`. `n == 0` is a circle, and otherwise
/// it has `n` straight sides with a corner at `rot`.
fn iris_extent(y: f64, radius: f64, n: u32, rot: f32) -> Option<(f64, f64)> {
    if n == 0 {
        let half = (radius * radius - y * y).sqrt();
        return half.is_finite().then_some((-half, half));
    }
    let corner = |k: u32| {
        let angle = f64::from(rot) + std::f64::consts::TAU * f64::from(k) / f64::from(n);
        (angle.cos() * radius, angle.sin() * radius)
    };
    let mut reach: Option<(f64, f64)> = None;
    for k in 0..n {
        let (a, b) = (corner(k), corner(k + 1));
        if (a.1 - y) * (b.1 - y) > 0.0 || a.1 == b.1 {
            continue;
        }
        let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
        reach = Some(reach.map_or((x, x), |(from, to)| (from.min(x), to.max(x))));
    }
    reach
}

/// The rows an iris covers, each measured at eight heights, so that its top and bottom edges are as
/// soft as its sides.
pub(super) fn iris_rows(radius: f64, n: u32, rot: f32) -> Vec<IrisRow> {
    const HEIGHTS: usize = 8;
    let reach = radius.ceil() as isize;
    (-reach..=reach)
        .filter_map(|dy| {
            let (mut from, mut to, mut hits) = (0.0, 0.0, 0usize);
            for height in 0..HEIGHTS {
                let y = dy as f64 + (height as f64 + 0.5) / HEIGHTS as f64 - 0.5;
                if let Some((left, right)) = iris_extent(y, radius, n, rot) {
                    from += left;
                    to += right;
                    hits += 1;
                }
            }
            (hits > 0).then(|| IrisRow {
                dy,
                from: from / hits as f64,
                to: to / hits as f64,
                weight: hits as f64 / HEIGHTS as f64,
            })
        })
        .collect()
}

/// `src` averaged over an iris at each pixel, its edges held. A running total along each row gives
/// a row's worth of pixels for two reads, so the cost goes with the iris's height and not its area.
fn gather_iris(src: &Image, iris: &[IrisRow]) -> Image {
    /// Rows drawn together, sharing the running totals of the rows they read.
    const BAND: usize = 32;
    let (w, h) = (src.w, src.h);
    let mut out = Image::new(w, h);
    let reach = iris
        .iter()
        .map(|row| row.dy.unsigned_abs())
        .max()
        .unwrap_or(0);
    let widest = iris
        .iter()
        .map(|row| row.from.abs().max(row.to.abs()))
        .fold(0.0, f64::max);
    let pad = widest.ceil() as usize + 2;
    let stride = (w + 2 * pad + 1) * 4;
    let area: f64 = iris
        .iter()
        .map(|row| row.weight * (row.to - row.from))
        .sum();
    // Where a span's end falls among the totals, which lie between the pixels: the total before it,
    // and how far to the next.
    let place = |end: f64| {
        let at = end + 0.5 + pad as f64;
        (at.floor() as usize, at - at.floor())
    };
    out.px
        .par_chunks_mut(w * 4 * BAND)
        .enumerate()
        .for_each(|(band, rows)| {
            let first = band * BAND;
            let last = first + rows.len() / (w * 4) - 1;
            let lowest = first.saturating_sub(reach);
            let highest = (last + reach).min(h - 1);
            let mut totals = vec![0f64; stride * (highest - lowest + 1)];
            for (line, y) in totals.chunks_exact_mut(stride).zip(lowest..) {
                let pixels = &src.px[y * w * 4..(y + 1) * w * 4];
                let mut total = [0f64; 4];
                for (j, next) in line[4..].chunks_exact_mut(4).enumerate() {
                    let x = j.saturating_sub(pad).min(w - 1);
                    for k in 0..4 {
                        // One bad pixel would otherwise take the rest of its row with it.
                        let value = pixels[x * 4 + k];
                        if value.is_finite() {
                            total[k] += f64::from(value);
                        }
                        next[k] = total[k];
                    }
                }
            }
            let mut sum = vec![0f64; w * 4];
            for (index, row) in rows.chunks_exact_mut(w * 4).enumerate() {
                let y = (first + index) as isize;
                sum.fill(0.0);
                for span in iris {
                    let read = (y + span.dy).clamp(0, h as isize - 1) as usize - lowest;
                    let line = &totals[read * stride..][..stride];
                    let ((a, fa), (b, fb)) = (place(span.from), place(span.to));
                    let n = w * 4;
                    let (a0, a1) = (&line[a * 4..][..n], &line[(a + 1) * 4..][..n]);
                    let (b0, b1) = (&line[b * 4..][..n], &line[(b + 1) * 4..][..n]);
                    let ends = a0.iter().zip(a1).zip(b0).zip(b1);
                    for (value, (((a0, a1), b0), b1)) in sum.iter_mut().zip(ends) {
                        let start = a0 + (a1 - a0) * fa;
                        let end = b0 + (b1 - b0) * fb;
                        *value += (end - start) * span.weight;
                    }
                }
                for (out, value) in row.iter_mut().zip(&sum) {
                    *out = (value / area) as f32;
                }
            }
        });
    out
}

/// Bokeh Blur: a lens-iris shaped gather blur with highlight boost (bright points bloom into
/// iris-shaped discs). Large radii run on a reduced image (the result is smooth).
pub fn bokeh(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let r = fv(e, "amount", cx) * cx.px_scale;
    if r < 0.5 {
        return;
    }
    let sides = [0u32, 3, 4, 5, 6, 8][chv(e, "shape").min(5) as usize];
    let rot = fv(e, "rotation", cx).to_radians();
    let boost = fv(e, "highlight", cx) / 100.0 * 6.0;
    let th = fv(e, "threshold", cx) / 100.0;
    let (fw, fh) = (img.w, img.h);
    let mut reduced: Option<Image> = None;
    let mut down = 1.0f32;
    while r / down > 12.0 {
        let picture = reduced.as_ref().unwrap_or(img);
        if picture.w < 8 || picture.h < 8 {
            break;
        }
        reduced = Some(picture.downsample2());
        down *= 2.0;
    }
    let rr = r / down;
    let work = reduced.as_mut().unwrap_or(img);
    // boost highlights (energy, not weight: bright points stay bright after averaging)
    if boost > 0.0 {
        work.px.par_chunks_mut(4).for_each(|p| {
            if p[3] <= 1e-6 {
                return;
            }
            let l = luma(enc(Image::unpremul([p[0], p[1], p[2], p[3]])));
            let k = 1.0 + boost * smoothstep(th, 1.0, l);
            for v in &mut p[..3] {
                *v *= k;
            }
        });
    }
    // An iris is as wide as its radius and half a pixel more, as it was when it was gathered ring by
    // ring, with the last ring on the edge and each ring standing for what is around it.
    let rings = rr.ceil().clamp(1.0, 12.0);
    let reach = f64::from(rr * (rings + 0.5) / rings);
    let blurred = gather_iris(work, &iris_rows(reach, sides, rot));
    *img = if down > 1.0 {
        blurred.transformed(
            fw,
            fh,
            &Affine::scale(fw as f64 / blurred.w as f64, fh as f64 / blurred.h as f64),
        )
    } else {
        blurred
    };
}

/// Blur one channel of an image (0..3) with a Gaussian.
fn blur_channel(img: &mut Image, ch: usize, sx: f32, sy: f32, repeat: bool) {
    if sx <= 0.3 && sy <= 0.3 {
        return;
    }
    let mut plane = Image {
        w: img.w,
        h: img.h,
        px: img
            .px
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[ch], 0.0, 0.0, 0.0])
            .collect(),
    };
    gaussian(&mut plane, sx, sy, repeat);
    img.px
        .par_chunks_mut(4)
        .zip(plane.px.par_chunks(4))
        .for_each(|(p, q)| p[ch] = q[0]);
}

/// Channel Blur: independent Gaussian blur per R, G, B and alpha.
pub fn channel_blur(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let dims = chv(e, "dimensions");
    let repeat = bv(e, "repeat_edge");
    for (ch, id) in ["red", "green", "blue", "alpha"].iter().enumerate() {
        let s = fv(e, id, cx) * cx.px_scale * 0.5;
        blur_channel(
            img,
            ch,
            if dims == 2 { 0.0 } else { s },
            if dims == 1 { 0.0 } else { s },
            repeat,
        );
    }
}

/// Mix between a stack of increasingly blurred copies by a per-pixel amount 0..1.
fn variable_blur(img: &mut Image, max_sigma: f32, amount: impl Fn(usize, usize) -> f32 + Sync) {
    if max_sigma <= 0.3 {
        return;
    }
    let levels = [0.0f32, 0.125, 0.25, 0.5, 1.0];
    let stack: Vec<Image> = levels
        .par_iter()
        .map(|&k| {
            let mut c = img.clone();
            if k > 0.0 {
                gaussian(&mut c, max_sigma * k, max_sigma * k, true);
            }
            c
        })
        .collect();
    let w = img.w;
    img.px
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let a = amount(x, y).clamp(0.0, 1.0);
                let i = levels
                    .partition_point(|&l| l <= a)
                    .clamp(1, levels.len() - 1);
                let (l0, l1) = (levels[i - 1], levels[i]);
                let t = ((a - l0) / (l1 - l0)).clamp(0.0, 1.0);
                let (p0, p1) = (stack[i - 1].get(x, y), stack[i].get(x, y));
                for k in 0..4 {
                    row[x * 4 + k] = p0[k] + (p1[k] - p0[k]) * t;
                }
            }
        });
}

/// Compound Blur: blur amount per pixel from a blur layer's luminance (another track, or this
/// layer's own luminance when the Blur Layer is None).
pub fn compound(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let max = fv(e, "max", cx) * cx.px_scale * 0.5;
    let invert = bv(e, "invert");
    let layer = chv(e, "layer") as usize;
    let track = if layer > 0 {
        cx.env
            .and_then(|env| env.track(layer - 1).map(|t| (t, env.layer_to_output())))
    } else {
        None
    };
    let stretch = bv(e, "stretch");
    let own = img.clone();
    let (iw, ih) = (img.w as f64, img.h as f64);
    variable_blur(img, max, |x, y| {
        let l = match &track {
            Some((t, m)) => {
                let p = if stretch {
                    m.apply(Vec2::new(x as f64 + 0.5, y as f64 + 0.5))
                } else {
                    Vec2::new(
                        (x as f64 + 0.5) / iw * t.w as f64,
                        (y as f64 + 0.5) / ih * t.h as f64,
                    )
                };
                let s = t.sample_bilinear_clamped(p.x as f32, p.y as f32);
                kerf_color::linear_to_srgb(luma([s[0], s[1], s[2]]).clamp(0.0, 1.0))
            }
            None => {
                let s = own.get(x, y);
                kerf_color::linear_to_srgb(luma([s[0], s[1], s[2]]).clamp(0.0, 1.0))
            }
        };
        if invert {
            1.0 - l
        } else {
            l
        }
    });
}

/// Focus Blur: sharp inside a radial or linear focus area, blurring smoothly outside it.
pub fn focus(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let amount = fv(e, "amount", cx) * cx.px_scale * 0.5;
    let c = pv(e, "center", cx, img);
    let linear = chv(e, "shape") == 1;
    let (s, co) = (fv(e, "angle", cx) as f64).to_radians().sin_cos();
    let size = fv(e, "size", cx) as f64 * cx.px_scale as f64 * 0.5;
    let feather = (fv(e, "feather", cx) as f64 * cx.px_scale as f64).max(1.0);
    let show = bv(e, "show");
    let dist = |x: usize, y: usize| -> f32 {
        let (dx, dy) = (x as f64 + 0.5 - c.x, y as f64 + 0.5 - c.y);
        let d = if linear {
            (-dx * s + dy * co).abs()
        } else {
            dx.hypot(dy)
        };
        (((d - size) / feather).clamp(0.0, 1.0)) as f32
    };
    variable_blur(img, amount, dist);
    if show {
        let w = img.w;
        img.px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let k = (1.0 - dist(x, y)) * 0.35;
                    let p = &mut row[x * 4..x * 4 + 4];
                    over(p, [k, 0.0, 0.0, k]);
                }
            });
    }
}

/// Reduce Interlace Flicker: a vertical-only softening across fields.
pub fn interlace_flicker(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let s = fv(e, "softness", cx) * cx.px_scale.max(0.25);
    gaussian(img, 0.0, s, true);
}
