//! The working picture: premultiplied linear-light RGBA in floating point,
//! row by row. Carried from FilmCraft's renderer (`image.rs`).

use crate::color;
use kerf_geom::{Affine, Rect};
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    /// Four floats a pixel, `w * h * 4` in all.
    pub px: Vec<f32>,
}

impl Image {
    /// A transparent picture.
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![0.0; w * h * 4],
        }
    }

    /// A picture of one premultiplied color.
    pub fn filled(w: usize, h: usize, color: [f32; 4]) -> Self {
        let mut px = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            px.extend_from_slice(&color);
        }
        Self { w, h, px }
    }

    /// Straight sRGB-encoded RGBA bytes, `stride` bytes a row, in linear
    /// light. Opaque unless `alpha` says the picture's transparency counts.
    pub fn from_rgba8(rgba: &[u8], w: usize, h: usize, stride: usize, alpha: bool) -> Self {
        let decode = color::decode_table();
        let mut image = Self::new(w, h);
        image
            .px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                let line = &rgba[y * stride..y * stride + w * 4];
                for (pixel, bytes) in row.chunks_exact_mut(4).zip(line.chunks_exact(4)) {
                    let a = if alpha {
                        f32::from(bytes[3]) / 255.0
                    } else {
                        1.0
                    };
                    pixel[0] = decode[usize::from(bytes[0])] * a;
                    pixel[1] = decode[usize::from(bytes[1])] * a;
                    pixel[2] = decode[usize::from(bytes[2])] * a;
                    pixel[3] = a;
                }
            });
        image
    }

    /// The picture over black as straight sRGB-encoded RGBA bytes, opaque.
    pub fn to_rgba8_over_black(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        out.par_chunks_mut(self.w * 4)
            .zip(self.px.par_chunks(self.w * 4))
            .for_each(|(out, row)| {
                for (bytes, pixel) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    bytes[0] = color::encode(pixel[0]);
                    bytes[1] = color::encode(pixel[1]);
                    bytes[2] = color::encode(pixel[2]);
                    bytes[3] = 255;
                }
            });
        out
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [f32; 4] {
        let i = (y * self.w + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    /// The pixel at `x`, `y`, the nearest edge pixel outside the picture.
    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> [f32; 4] {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.get(x, y)
    }

    /// The pixel at `x`, `y`, transparent outside the picture.
    #[inline]
    pub fn get_or_clear(&self, x: isize, y: isize) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.w as isize || y >= self.h as isize {
            [0.0; 4]
        } else {
            self.get(x as usize, y as usize)
        }
    }

    /// Bilinear sample at continuous pixel coordinates (pixel centers at
    /// +0.5), transparent outside.
    #[inline]
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.get_or_clear(x0, y0);
        let b = self.get_or_clear(x0 + 1, y0);
        let c = self.get_or_clear(x0, y0 + 1);
        let d = self.get_or_clear(x0 + 1, y0 + 1);
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bottom - top) * ty;
        }
        out
    }

    /// Bilinear sample, the nearest edge pixel outside the picture.
    #[inline]
    pub fn sample_bilinear_clamped(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = (x - 0.5).clamp(0.0, self.w as f32 - 1.0);
        let fy = (y - 0.5).clamp(0.0, self.h as f32 - 1.0);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (a, b, c, d) = (
            self.get(x0, y0),
            self.get(x1, y0),
            self.get(x0, y1),
            self.get(x1, y1),
        );
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            out[k] = top + (bottom - top) * ty;
        }
        out
    }

    /// This picture drawn into a transparent `w` × `h` one through `m`,
    /// which maps its pixels to the new picture's: bilinear, from a picture
    /// halved first while `m` shrinks it to under half.
    pub fn transformed(&self, w: usize, h: usize, m: &Affine) -> Image {
        self.transformed_within(w, h, m, Rect::new(0.0, 0.0, self.w as f64, self.h as f64))
    }

    /// [`Image::transformed`] of only `rect` of this picture (a crop, in
    /// its pixels). Colors are sampled inside it, its edge pixels carrying
    /// on past it, and each new pixel is covered by as much of it as falls
    /// in the pixel, so its edges are sharp and antialiased wherever they
    /// land, without a dark fringe.
    pub fn transformed_within(&self, w: usize, h: usize, m: &Affine, rect: Rect) -> Image {
        let mut out = Image::new(w, h);
        let Some(placement) = self.placement(w, h, m, rect) else {
            return out;
        };
        out.px
            .par_chunks_mut(w * 4)
            .enumerate()
            .skip(placement.at.rows.start)
            .take(placement.at.rows.len())
            .for_each(|(y, row)| {
                for x in placement.at.columns.clone() {
                    if let Some(sample) = placement.sample(x, y) {
                        row[x * 4..x * 4 + 4].copy_from_slice(&sample);
                    }
                }
            });
        out
    }

    /// How this picture lands in a `w` × `h` one through `m`, cropped to
    /// `rect`, to sample it there; none where it lands nowhere.
    pub(crate) fn placement(
        &self,
        w: usize,
        h: usize,
        m: &Affine,
        rect: Rect,
    ) -> Option<Placement<'_>> {
        m.inverse()?;
        if rect.w <= 0.0 || rect.h <= 0.0 || self.w == 0 || self.h == 0 {
            return None;
        }
        if Landing::halves((self.w, self.h), m) {
            // Shrinking to under half: from a picture halved first.
            let half = self.downsample2();
            let m = m.then_apply(&Affine::scale(2.0, 2.0));
            let rect = Rect::new(rect.x / 2.0, rect.y / 2.0, rect.w / 2.0, rect.h / 2.0);
            let placement = half.placement(w, h, &m, rect)?.owned();
            return Some(placement);
        }
        Some(Placement {
            image: std::borrow::Cow::Borrowed(self),
            at: Landing::new((self.w, self.h), (w, h), m, rect)?,
        })
    }

    /// The picture at half its size, each pixel the mean of four.
    pub fn downsample2(&self) -> Image {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut out = Image::new(w, h);
        out.px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let (x2, y2) = (2 * x as isize, 2 * y as isize);
                    let a = self.get_clamped(x2, y2);
                    let b = self.get_clamped(x2 + 1, y2);
                    let c = self.get_clamped(x2, y2 + 1);
                    let d = self.get_clamped(x2 + 1, y2 + 1);
                    for k in 0..4 {
                        row[x * 4 + k] = (a[k] + b[k] + c[k] + d[k]) * 0.25;
                    }
                }
            });
        out
    }

    /// `self × (1 − t) + other × t`, a cross-fade of premultiplied pictures
    /// of one size.
    pub fn lerp(mut self, other: &Image, t: f32) -> Image {
        if (other.w, other.h) != (self.w, self.h) {
            return self;
        }
        for (a, b) in self.px.iter_mut().zip(&other.px) {
            *a += (*b - *a) * t;
        }
        self
    }

    /// Fade the whole picture by `alpha`.
    pub fn scale_alpha(&mut self, alpha: f32) {
        if (alpha - 1.0).abs() < 1e-6 {
            return;
        }
        self.px.par_iter_mut().for_each(|value| *value *= alpha);
    }

    /// Keep `coverage` of each pixel (one value a pixel).
    pub fn mul_coverage(&mut self, coverage: &[f32]) {
        self.px
            .par_chunks_mut(4)
            .zip(coverage.par_iter())
            .for_each(|(pixel, &covered)| {
                for value in pixel {
                    *value *= covered;
                }
            });
    }

    /// A premultiplied pixel's straight color.
    #[inline]
    pub fn unpremul(pixel: [f32; 4]) -> [f32; 3] {
        if pixel[3] <= 1e-6 {
            [0.0; 3]
        } else {
            [
                pixel[0] / pixel[3],
                pixel[1] / pixel[3],
                pixel[2] / pixel[3],
            ]
        }
    }

    /// Change each pixel's straight color, in linear light, leaving its
    /// alpha; `f` also gets the pixel's column and row.
    pub fn map_rgb(&mut self, f: impl Fn([f32; 3], usize, usize) -> [f32; 3] + Sync) {
        let w = self.w;
        if w == 0 {
            return;
        }
        self.px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                    let alpha = pixel[3];
                    if alpha <= 1e-6 {
                        continue;
                    }
                    let [r, g, b] = f([pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha], x, y);
                    pixel[0] = r * alpha;
                    pixel[1] = g * alpha;
                    pixel[2] = b * alpha;
                }
            });
    }

    /// Change each pixel's straight sRGB-encoded color (0..1), for
    /// corrections written for encoded values.
    pub fn map_encoded(&mut self, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
        self.map_rgb(|rgb, _, _| f(rgb.map(color::to_encoded)).map(color::to_linear));
    }
}

/// Where a picture lands in another: the numbers that place it, whichever
/// processor then samples it.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    /// The other picture's pixels to this one's.
    pub inverse: Affine,
    /// The part of this picture that shows, in its pixels.
    pub rect: Rect,
    /// Distances across and down this picture, in its pixels, over these,
    /// are distances in the other's.
    pub across: f64,
    pub down: f64,
    /// Colors come from the pixels inside the crop.
    pub lo: [f64; 2],
    pub hi: [f64; 2],
    /// The rows and columns of the other picture it can land on.
    pub rows: std::ops::Range<usize>,
    pub columns: std::ops::Range<usize>,
}

impl Landing {
    /// `rect` of a picture of `size` placed through `m`, its pixels to those
    /// of a picture of `canvas`. `None` when nothing can land.
    pub fn new(
        size: (usize, usize),
        canvas: (usize, usize),
        m: &Affine,
        rect: Rect,
    ) -> Option<Self> {
        let inverse = m.inverse()?;
        if rect.w <= 0.0 || rect.h <= 0.0 || size.0 == 0 || size.1 == 0 {
            return None;
        }
        let (w, h) = canvas;
        let bounds = m.bounds(&rect);
        let y0 = (bounds.y.floor().max(0.0) as usize).min(h);
        let y1 = (bounds.bottom().ceil().max(0.0) as usize).min(h);
        let x0 = (bounds.x.floor().max(0.0) as usize).min(w);
        let x1 = (bounds.right().ceil().max(0.0) as usize).min(w);
        Some(Self {
            inverse,
            rect,
            across: inverse.a.hypot(inverse.c).max(1e-12),
            down: inverse.b.hypot(inverse.d).max(1e-12),
            lo: [rect.x.floor().max(0.0), rect.y.floor().max(0.0)],
            hi: [
                rect.right().ceil().min(size.0 as f64),
                rect.bottom().ceil().min(size.1 as f64),
            ],
            rows: y0..y1.max(y0),
            columns: x0..x1.max(x0),
        })
    }

    /// Whether a picture of `size` placed through `m` shrinks to under half,
    /// and so is sampled from itself at half its size.
    pub fn halves(size: (usize, usize), m: &Affine) -> bool {
        let sx = (m.a * m.a + m.b * m.b).sqrt();
        let sy = (m.c * m.c + m.d * m.d).sqrt();
        let minify = 1.0 / sx.min(sy).max(1e-6);
        minify >= 2.0 && size.0 >= 4 && size.1 >= 4
    }
}

/// A picture placed in another, sampled at the other's pixels.
pub(crate) struct Placement<'a> {
    image: std::borrow::Cow<'a, Image>,
    pub at: Landing,
}

impl Placement<'_> {
    fn owned(self) -> Placement<'static> {
        Placement {
            image: std::borrow::Cow::Owned(self.image.into_owned()),
            at: self.at,
        }
    }

    /// The premultiplied color at pixel `x`, `y` of the other picture, if
    /// any of this one covers it.
    #[inline]
    pub fn sample(&self, x: usize, y: usize) -> Option<[f32; 4]> {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let at = &self.at;
        let m = &at.inverse;
        let (u, v) = (m.a * px + m.c * py + m.e, m.b * px + m.d * py + m.f);
        let rect = &at.rect;
        let edge = |inside: f64, scale: f64| (inside / scale + 0.5).clamp(0.0, 1.0);
        let covered = edge(u - rect.x, at.across)
            * edge(rect.right() - u, at.across)
            * edge(v - rect.y, at.down)
            * edge(rect.bottom() - v, at.down);
        if covered <= 0.0 {
            return None;
        }
        let [lo_x, lo_y] = at.lo;
        let [hi_x, hi_y] = at.hi;
        let sample = self.image.sample_bilinear_clamped(
            u.clamp(lo_x + 0.5, (hi_x - 0.5).max(lo_x + 0.5)) as f32,
            v.clamp(lo_y + 0.5, (hi_y - 0.5).max(lo_y + 0.5)) as f32,
        );
        let covered = covered as f32;
        Some(sample.map(|value| value * covered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_come_back_through_linear_light() {
        let rgba: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
        let image = Image::from_rgba8(&rgba, 16, 16, 64, false);
        assert_eq!(image.to_rgba8_over_black(), rgba);
    }

    #[test]
    fn transparency_counts_only_when_asked() {
        let rgba = [255, 255, 255, 0];
        assert_eq!(Image::from_rgba8(&rgba, 1, 1, 4, false).get(0, 0), [1.0; 4]);
        assert_eq!(Image::from_rgba8(&rgba, 1, 1, 4, true).get(0, 0), [0.0; 4]);
    }

    #[test]
    fn crops_are_sharp_and_keep_their_color() {
        let image = Image::filled(8, 8, [1.0, 0.0, 0.0, 1.0]);
        // The right half, drawn twice as large: no fringe at its edge.
        let crop = Rect::new(4.0, 0.0, 4.0, 8.0);
        let placed = image.transformed_within(16, 16, &Affine::scale(2.0, 2.0), crop);
        assert_eq!(placed.get(7, 4), [0.0; 4]);
        assert_eq!(placed.get(8, 4), [1.0, 0.0, 0.0, 1.0]);
        // A crop edge halfway across a pixel covers half of it.
        let half = image.transformed_within(8, 8, &Affine::IDENTITY, Rect::new(2.5, 0.0, 5.5, 8.0));
        assert_eq!(half.get(2, 2), [0.5, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn identity_transforms_keep_the_picture() {
        let rgba: Vec<u8> = (0..64u8).flat_map(|v| [v * 4, v, 255 - v, 255]).collect();
        let image = Image::from_rgba8(&rgba, 8, 8, 32, false);
        assert_eq!(image.transformed(8, 8, &Affine::IDENTITY), image);
        let moved = image.transformed(8, 8, &Affine::translate(2.0, 0.0));
        assert_eq!(moved.get(5, 3), image.get(3, 3));
        assert_eq!(moved.get(1, 3), [0.0; 4]);
    }
}
