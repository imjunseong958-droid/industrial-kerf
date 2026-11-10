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

    /// Straight 16-bit little-endian RGBA, `stride` bytes a row, with each pixel's code values
    /// (0..1) taken into linear light by `convert`. Opaque unless `alpha` says the picture's
    /// transparency counts.
    pub fn from_rgba16(
        rgba: &[u8],
        (w, h): (usize, usize),
        stride: usize,
        alpha: bool,
        convert: impl Fn([f32; 3]) -> [f32; 3] + Sync,
    ) -> Self {
        let mut image = Self::new(w, h);
        image
            .px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                let line = &rgba[y * stride..y * stride + w * 8];
                for (pixel, bytes) in row.as_chunks_mut::<4>().0.iter_mut().zip(line.as_chunks::<8>().0.iter()) {
                    let code = |at: usize| {
                        f32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]])) / 65535.0
                    };
                    let a = if alpha { code(6) } else { 1.0 };
                    let [r, g, b] = convert([code(0), code(2), code(4)]);
                    pixel.copy_from_slice(&[r * a, g * a, b * a, a]);
                }
            });
        image
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
                for (pixel, bytes) in row.as_chunks_mut::<4>().0.iter_mut().zip(line.as_chunks::<4>().0.iter()) {
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
                for (bytes, pixel) in out.as_chunks_mut::<4>().0.iter_mut().zip(row.as_chunks::<4>().0.iter()) {
                    bytes[0] = color::encode(pixel[0]);
                    bytes[1] = color::encode(pixel[1]);
                    bytes[2] = color::encode(pixel[2]);
                    bytes[3] = 255;
                }
            });
        out
    }

    /// The brightest value any pixel holds.
    pub fn peak(&self) -> f32 {
        self.px
            .par_chunks(4)
            .map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
            .reduce(|| 0.0, f32::max)
    }

    /// The lowest value any pixel holds, as a straight color: below 0 for a color BT.709 cannot hold.
    pub fn floor(&self) -> f32 {
        self.px
            .par_chunks(4)
            .map(|pixel| {
                let lowest = pixel[0].min(pixel[1]).min(pixel[2]);
                if pixel[3] > 0.0 {
                    lowest / pixel[3]
                } else {
                    lowest.min(0.0)
                }
            })
            .reduce(|| 0.0, f32::min)
    }

    /// Bring each pixel under 1 and inside BT.709, hue and all, and say what to put it back by.
    pub fn to_unit(&mut self) -> Vec<[f32; 2]> {
        self.px
            .par_chunks_mut(4)
            .map(|pixel| {
                let held = to_gamut(&mut pixel[..3]);
                let gain = pixel[0].max(pixel[1]).max(pixel[2]).max(1.0);
                for value in &mut pixel[..3] {
                    *value /= gain;
                }
                [gain, held]
            })
            .collect()
    }

    /// Put each pixel back by what [`Image::to_unit`] said.
    pub fn from_unit(&mut self, held: &[[f32; 2]]) {
        self.px
            .par_chunks_mut(4)
            .zip(held)
            .for_each(|(pixel, [gain, chroma])| {
                for value in &mut pixel[..3] {
                    *value *= gain;
                }
                from_gamut(&mut pixel[..3], *chroma);
            });
    }

    /// Bring each pixel inside BT.709, leaving how bright it is alone, and say what to put it back by.
    pub fn to_gamut(&mut self) -> Vec<f32> {
        self.px
            .par_chunks_mut(4)
            .map(|pixel| to_gamut(&mut pixel[..3]))
            .collect()
    }

    /// Put each pixel back by what [`Image::to_gamut`] said.
    pub fn from_gamut(&mut self, held: &[f32]) {
        self.px
            .par_chunks_mut(4)
            .zip(held)
            .for_each(|(pixel, chroma)| from_gamut(&mut pixel[..3], *chroma));
    }

    /// Take `lift` from every straight color and divide what is left by `by`: an even way to bring a
    /// whole picture between 0 and 1 that a filter over its pixels can run on and be undone from.
    pub fn lift_rgb(&mut self, lift: f32, by: f32) {
        if lift == 0.0 {
            return self.scale_rgb(1.0 / by);
        }
        self.px.par_chunks_mut(4).for_each(|pixel| {
            let under = lift * pixel[3];
            for value in &mut pixel[..3] {
                *value = (*value - under) / by;
            }
        });
    }

    /// Undo [`Image::lift_rgb`].
    pub fn unlift_rgb(&mut self, lift: f32, by: f32) {
        if lift == 0.0 {
            return self.scale_rgb(by);
        }
        self.px.par_chunks_mut(4).for_each(|pixel| {
            let under = lift * pixel[3];
            for value in &mut pixel[..3] {
                *value = *value * by + under;
            }
        });
    }

    /// Multiply the color of every pixel.
    pub fn scale_rgb(&mut self, by: f32) {
        self.px.par_chunks_mut(4).for_each(|pixel| {
            for value in &mut pixel[..3] {
                *value *= by;
            }
        });
    }

    /// This picture over black as 16-bit little-endian RGBA with `signal` making each pixel's code
    /// values, 0 to 1, for an HDR master.
    pub fn to_signal16_over_black(&self, signal: impl Fn([f32; 3]) -> [f32; 3] + Sync) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 8];
        out.par_chunks_mut(self.w * 8)
            .zip(self.px.par_chunks(self.w * 4))
            .for_each(|(out, row)| {
                for (bytes, pixel) in out.as_chunks_mut::<8>().0.iter_mut().zip(row.as_chunks::<4>().0.iter()) {
                    let clean = |v: f32| if v.is_nan() { 0.0 } else { v };
                    let codes = signal([clean(pixel[0]), clean(pixel[1]), clean(pixel[2])]);
                    for (channel, value) in codes.into_iter().enumerate() {
                        let code = (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                        bytes[channel * 2..channel * 2 + 2].copy_from_slice(&code.to_le_bytes());
                    }
                    bytes[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
                }
            });
        out
    }

    /// This picture over black as 16-bit little-endian sRGB-encoded RGBA, for a master that keeps
    /// more than 8 bits.
    pub fn to_rgba16_over_black(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 8];
        out.par_chunks_mut(self.w * 8)
            .zip(self.px.par_chunks(self.w * 4))
            .for_each(|(out, row)| {
                for (bytes, pixel) in out.as_chunks_mut::<8>().0.iter_mut().zip(row.as_chunks::<4>().0.iter()) {
                    for channel in 0..3 {
                        let value = if pixel[channel].is_nan() {
                            0.0
                        } else {
                            pixel[channel].clamp(0.0, 1.0)
                        };
                        let code = (color::linear_to_srgb(value) * 65535.0 + 0.5) as u16;
                        bytes[channel * 2..channel * 2 + 2].copy_from_slice(&code.to_le_bytes());
                    }
                    bytes[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
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
                for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
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

fn brightness(rgb: &[f32]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Pull a color BT.709 cannot hold, with a channel below 0, straight toward the gray of its own
/// brightness until it can, and say how much of its color is left: 1 for a color it can hold, where
/// nothing moves, and 0 for one with no light in it to keep, which comes to black.
#[inline]
fn to_gamut(rgb: &mut [f32]) -> f32 {
    let lowest = rgb[0].min(rgb[1]).min(rgb[2]);
    if lowest >= 0.0 {
        return 1.0;
    }
    let gray = brightness(rgb);
    if gray <= 0.0 {
        rgb.fill(0.0);
        return 0.0;
    }
    let left = gray / (gray - lowest);
    for value in rgb.iter_mut() {
        *value = gray + (*value - gray) * left;
    }
    left
}

/// Put back what [`to_gamut`] took, around the brightness the color has now.
#[inline]
fn from_gamut(rgb: &mut [f32], left: f32) {
    if left >= 1.0 || left <= 0.0 {
        return;
    }
    let gray = brightness(rgb);
    for value in rgb.iter_mut() {
        *value = gray + (*value - gray) / left;
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

    /// One pixel, opaque.
    fn pixel(rgb: [f32; 3]) -> Image {
        let mut image = Image::new(1, 1);
        image.px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 1.0]);
        image
    }

    fn near(found: &[f32], wanted: [f32; 3]) {
        for (found, wanted) in found.iter().zip(wanted) {
            assert!((found - wanted).abs() < 1e-5, "{found} of {wanted}");
        }
    }

    #[test]
    fn a_color_past_bt709_goes_inside_and_comes_back() {
        // BT.2020's green, in BT.709's own terms.
        let wide = [-0.5876, 1.1329, -0.1006];
        let mut image = pixel(wide);
        assert!((image.floor() + 0.5876).abs() < 1e-6);
        let held = image.to_unit();
        // Inside, nothing below 0 or above 1, and its brightness is its own.
        assert!(image.px[..3]
            .iter()
            .all(|value| (0.0..=1.0).contains(value)));
        assert!(image.px[0].min(image.px[2]) < 1e-5, "{:?}", image.px);
        image.from_unit(&held);
        near(&image.px[..3], wide);
        // Without a gain, by the brightness alone.
        let mut image = pixel(wide);
        let brightness = |rgb: &[f32]| 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        let before = brightness(&image.px[..3]);
        let held = image.to_gamut();
        assert!((brightness(&image.px[..3]) - before).abs() < 1e-5);
        assert!(image.px[..3].iter().all(|value| *value > -1e-6));
        image.from_gamut(&held);
        near(&image.px[..3], wide);
    }

    #[test]
    fn colors_it_can_hold_move_as_they_did() {
        // Above 1 and inside BT.709: only the gain, as it was, to the bit.
        let bright = [4.0f32, 2.0, 0.5];
        let mut image = pixel(bright);
        assert_eq!(image.floor(), 0.0);
        let held = image.to_unit();
        assert_eq!(held, [[4.0, 1.0]]);
        assert_eq!(&image.px[..3], &[1.0, 0.5, 0.125]);
        image.from_unit(&held);
        assert_eq!(&image.px[..3], &bright);
        assert_eq!(image.to_gamut(), [1.0]);
        assert_eq!(&image.px[..3], &bright);
    }

    #[test]
    fn a_color_with_no_light_in_it_goes_to_black() {
        let mut image = pixel([-0.2, 0.01, -0.3]);
        let held = image.to_gamut();
        assert_eq!(held, [0.0]);
        assert_eq!(&image.px[..3], &[0.0, 0.0, 0.0]);
        image.from_gamut(&held);
        assert_eq!(&image.px[..3], &[0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_whole_picture_lifted_comes_back_and_goes_by_its_alpha() {
        let mut image = Image::new(2, 1);
        image
            .px
            .copy_from_slice(&[-0.5, 2.0, 0.25, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(image.floor(), -0.5);
        let original = image.px.clone();
        image.lift_rgb(-0.5, 5.0);
        assert!(image.px[0].abs() < 1e-6);
        // What is not there has nothing added to it.
        assert_eq!(&image.px[4..8], &[0.0; 4]);
        image.unlift_rgb(-0.5, 5.0);
        for (found, wanted) in image.px.iter().zip(&original) {
            assert!((found - wanted).abs() < 1e-5, "{found} of {wanted}");
        }
        // Nothing to lift is a plain scale, as it was.
        let mut plain = pixel([2.0, 1.0, 0.5]);
        plain.lift_rgb(0.0, 4.0);
        assert_eq!(&plain.px[..3], &[2.0 * 0.25, 1.0 * 0.25, 0.5 * 0.25]);
    }

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
