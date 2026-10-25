//! Single- or multi-channel float planes: the tracker's working images (a crop of the frame in
//! the matching channel, pre-processed), their pyramids and bilinear sampling.
//!
//! Continuous coordinates follow the raster crate: pixel `i` covers `[i, i + 1)` and its centre is
//! at `i + 0.5`. Downsampling by 2 keeps that convention (level-1 coordinate = level-0 / 2).

/// An interleaved float plane.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub ch: usize,
    pub data: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize, ch: usize) -> Plane {
        Plane { w, h, ch, data: vec![0.0; w * h * ch] }
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        (y * self.w + x) * self.ch
    }

    /// Value at integer pixel `(x, y)` with edges repeated.
    #[inline]
    pub fn at(&self, x: isize, y: isize, c: usize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.data[self.idx(x, y) + c]
    }

    /// Bilinear sample at continuous coordinates (edges repeated) into `out[..ch]`.
    #[inline]
    pub fn sample(&self, x: f64, y: f64, out: &mut [f32]) {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = (fx - x0) as f32;
        let ty = (fy - y0) as f32;
        let (x0, y0) = (x0 as isize, y0 as isize);
        let inside = x0 >= 0 && y0 >= 0 && (x0 as usize) + 1 < self.w && (y0 as usize) + 1 < self.h;
        if inside {
            let i00 = self.idx(x0 as usize, y0 as usize);
            let i10 = i00 + self.ch;
            let i01 = i00 + self.w * self.ch;
            let i11 = i01 + self.ch;
            for c in 0..self.ch {
                let top = self.data[i00 + c] + (self.data[i10 + c] - self.data[i00 + c]) * tx;
                let bot = self.data[i01 + c] + (self.data[i11 + c] - self.data[i01 + c]) * tx;
                out[c] = top + (bot - top) * ty;
            }
        } else {
            for c in 0..self.ch {
                let a = self.at(x0, y0, c);
                let b = self.at(x0 + 1, y0, c);
                let cc = self.at(x0, y0 + 1, c);
                let d = self.at(x0 + 1, y0 + 1, c);
                let top = a + (b - a) * tx;
                let bot = cc + (d - cc) * tx;
                out[c] = top + (bot - top) * ty;
            }
        }
    }

    /// Half resolution (2 × 2 box average; odd sizes round up with the edge repeated).
    pub fn downsample(&self) -> Plane {
        let w = self.w.div_ceil(2).max(1);
        let h = self.h.div_ceil(2).max(1);
        let mut out = Plane::new(w, h, self.ch);
        for y in 0..h {
            for x in 0..w {
                for c in 0..self.ch {
                    let (sx, sy) = (2 * x as isize, 2 * y as isize);
                    let v = self.at(sx, sy, c) + self.at(sx + 1, sy, c) + self.at(sx, sy + 1, c) + self.at(sx + 1, sy + 1, c);
                    let i = out.idx(x, y) + c;
                    out.data[i] = v * 0.25;
                }
            }
        }
        out
    }

    /// Central-difference gradients `(d/dx, d/dy)`.
    pub fn gradients(&self) -> (Plane, Plane) {
        let mut gx = Plane::new(self.w, self.h, self.ch);
        let mut gy = Plane::new(self.w, self.h, self.ch);
        for y in 0..self.h {
            for x in 0..self.w {
                for c in 0..self.ch {
                    let (xi, yi) = (x as isize, y as isize);
                    let i = self.idx(x, y) + c;
                    gx.data[i] = 0.5 * (self.at(xi + 1, yi, c) - self.at(xi - 1, yi, c));
                    gy.data[i] = 0.5 * (self.at(xi, yi + 1, c) - self.at(xi, yi - 1, c));
                }
            }
        }
        (gx, gy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_and_pyramid_share_coordinates() {
        let mut p = Plane::new(8, 8, 1);
        for y in 0..8 {
            for x in 0..8 {
                p.data[y * 8 + x] = x as f32 + 10.0 * y as f32;
            }
        }
        let mut o = [0.0];
        p.sample(2.5, 3.5, &mut o);
        assert_eq!(o[0], 32.0);
        p.sample(3.0, 3.5, &mut o);
        assert!((o[0] - 32.5).abs() < 1e-6);
        // A linear ramp keeps its value at matching coordinates on the next level.
        let d = p.downsample();
        d.sample(1.5, 1.5, &mut o);
        let mut o0 = [0.0];
        p.sample(3.0, 3.0, &mut o0);
        assert!((o[0] - o0[0]).abs() < 1e-5, "{} vs {}", o[0], o0[0]);
    }
}
