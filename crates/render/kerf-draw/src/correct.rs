//! `color.basic`, with the arithmetic of the graph's filters.
//!
//! The graph lowers `color.basic` to FFmpeg's `exposure`, `eq`,
//! `colorbalance`, and `vibrance` filters, with `mix` scaling every
//! correction toward identity. This is the same arithmetic in floating
//! point, on sRGB-encoded straight color: `eq` works on BT.601 studio-range
//! luma and chroma codes, as FFmpeg converts untagged pictures, and the
//! others on RGB.

/// `color.basic`'s parameters with `mix` applied, as the graph lowers
/// them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Correction {
    exposure: f32,
    /// What exposure multiplies by.
    gain: f32,
    contrast: f32,
    brightness: f32,
    saturation: f32,
    gamma: f32,
    /// Color balance shadows for red, green, and blue.
    shadows: [f32; 3],
    highlights: f32,
    vibrance: f32,
}

impl Correction {
    /// The correction `value` gives each of `color.basic`'s parameters.
    pub fn new(value: &dyn Fn(&str) -> f64) -> Self {
        let mix = value("mix");
        let balance = |value: f64| value.clamp(-1.0, 1.0) as f32;
        let (temperature, tint, shadows) = (
            value("temperature") / 1000.0 * mix,
            value("tint") * mix,
            value("shadows") * mix,
        );
        let exposure = (value("exposure") * mix) as f32;
        Self {
            exposure,
            gain: exposure.exp2(),
            contrast: (1.0 + (value("contrast") - 1.0) * mix) as f32,
            brightness: (value("brightness") * mix) as f32,
            saturation: (1.0 + (value("saturation") - 1.0) * mix) as f32,
            gamma: (1.0 + (value("gamma") - 1.0) * mix) as f32,
            shadows: [
                balance(temperature + tint + shadows),
                balance(-tint + shadows),
                balance(-temperature + tint + shadows),
            ],
            highlights: balance(value("highlights") * mix),
            vibrance: (value("vibrance") * mix) as f32,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.exposure == 0.0 && !self.eq() && !self.balance() && self.vibrance == 0.0
    }

    fn eq(&self) -> bool {
        self.contrast != 1.0
            || self.brightness != 0.0
            || self.saturation != 1.0
            || self.gamma != 1.0
    }

    fn balance(&self) -> bool {
        self.shadows.iter().any(|s| *s != 0.0) || self.highlights != 0.0
    }

    /// Correct one straight sRGB-encoded color (0..1).
    pub fn apply(&self, mut rgb: [f32; 3]) -> [f32; 3] {
        if self.exposure != 0.0 {
            rgb = rgb.map(|c| (c * self.gain).clamp(0.0, 1.0));
        }
        if self.eq() {
            rgb = self.equalize(rgb);
        }
        if self.balance() {
            rgb = self.color_balance(rgb);
        }
        if self.vibrance != 0.0 {
            rgb = vibrance(rgb, self.vibrance);
        }
        rgb
    }

    /// FFmpeg's `eq`: contrast, brightness, and gamma on luma codes,
    /// saturation as a contrast on chroma codes.
    fn equalize(&self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let y = 16.0 + 219.0 * (0.299 * r + 0.587 * g + 0.114 * b);
        let cb = 128.0 + 224.0 * (-0.168_736 * r - 0.331_264 * g + 0.5 * b);
        let cr = 128.0 + 224.0 * (0.5 * r - 0.418_688 * g - 0.081_312 * b);
        let y = plane(y, self.contrast, self.brightness, self.gamma);
        let (cb, cr) = (
            plane(cb, self.saturation, 0.0, 1.0),
            plane(cr, self.saturation, 0.0, 1.0),
        );
        let (y, cb, cr) = (
            (y - 16.0) / 219.0,
            (cb - 128.0) / 224.0,
            (cr - 128.0) / 224.0,
        );
        [
            y + 1.402 * cr,
            y - 0.344_136 * cb - 0.714_136 * cr,
            y + 1.772 * cb,
        ]
        .map(|c| c.clamp(0.0, 1.0))
    }

    /// FFmpeg's `colorbalance` with lightness preserved.
    fn color_balance(&self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let l = r.max(g).max(b) + r.min(g).min(b);
        let [rs, gs, bs] = self.shadows;
        let h = self.highlights;
        let (r, g, b) = (
            component(r, l, rs, h),
            component(g, l, gs, h),
            component(b, l, bs, h),
        );
        preserve_lightness([r, g, b], l)
    }
}

/// One plane through `eq`'s code arithmetic: its integer path without
/// gamma, its table with it.
fn plane(code: f32, contrast: f32, brightness: f32, gamma: f32) -> f32 {
    if contrast == 1.0 && brightness == 0.0 && gamma == 1.0 {
        return code;
    }
    let out = if gamma == 1.0 && contrast.abs() < 7.9 {
        let contrast = (contrast * 4096.0).trunc();
        let brightness = (((100.0 * brightness + 100.0).trunc() * 511.0) / 200.0).trunc()
            - 128.0
            - (contrast / 32.0).trunc();
        code * contrast / 4096.0 + brightness
    } else {
        let v = contrast * (code / 255.0 - 0.5) + 0.5 + brightness;
        if v <= 0.0 {
            0.0
        } else {
            (v.powf(1.0 / gamma) * 256.0).min(255.0)
        }
    };
    out.clamp(0.0, 255.0)
}

/// `colorbalance`'s weighting of a channel by lightness `l` (the sum of
/// the largest and smallest channel), midtones left alone.
fn component(v: f32, l: f32, shadows: f32, highlights: f32) -> f32 {
    const A: f32 = 4.0;
    const B: f32 = 0.333;
    const SCALE: f32 = 0.7;
    let shadows = shadows * ((B - l) * A + 0.5).clamp(0.0, 1.0) * SCALE;
    let highlights = highlights * ((l + B - 1.0) * A + 0.5).clamp(0.0, 1.0) * SCALE;
    (v + shadows + highlights).clamp(0.0, 1.0)
}

fn preserve_lightness([r, g, b]: [f32; 3], l: f32) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = l * 0.5;
    let mut hue = if r == g && g == b {
        0.0
    } else if max == r {
        60.0 * ((g - b) / (max - min))
    } else if max == g {
        60.0 * (2.0 + (b - r) / (max - min))
    } else {
        60.0 * (4.0 + (r - g) / (max - min))
    };
    if hue < 0.0 {
        hue += 360.0;
    }
    let saturation = if max == 1.0 || min == 0.0 {
        0.0
    } else {
        (max - min) / (1.0 - (2.0 * l - 1.0).abs())
    };
    let channel = |n: f32| {
        let a = saturation * l.min(1.0 - l);
        let k = (n + hue / 30.0) % 12.0;
        (l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)).clamp(0.0, 1.0)
    };
    [channel(0.0), channel(8.0), channel(4.0)]
}

/// FFmpeg's `vibrance` with its default balance and BT.709 luma.
fn vibrance([r, g, b]: [f32; 3], intensity: f32) -> [f32; 3] {
    let saturation = r.max(g).max(b) - r.min(g).min(b);
    let luma = 0.212_656 * r + 0.715_158 * g + 0.072_186 * b;
    let sign = if intensity > 0.0 { 1.0 } else { -1.0 };
    let scale = 1.0 + intensity * (1.0 + sign * saturation);
    [r, g, b].map(|c| (luma + (c - luma) * scale).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::Catalog;

    fn correction(values: &[(&str, f64)]) -> Correction {
        let def = Catalog::builtin().fx("color.basic").unwrap();
        Correction::new(&|name| {
            values
                .iter()
                .find(|(set, _)| *set == name)
                .map(|(_, value)| *value)
                .or_else(|| def.find(name).and_then(|param| param.default.as_number()))
                .unwrap_or(0.0)
        })
    }

    #[test]
    fn defaults_leave_pictures_alone() {
        assert!(correction(&[]).is_identity());
        assert!(correction(&[("exposure", 1.0), ("mix", 0.0)]).is_identity());
    }

    #[test]
    fn exposure_doubles_a_stop() {
        let out = correction(&[("exposure", 1.0)]).apply([0.25, 0.5, 0.75]);
        assert_eq!(out, [0.5, 1.0, 1.0]);
    }

    #[test]
    fn saturation_zero_leaves_gray() {
        let [r, g, b] = correction(&[("saturation", 0.0)]).apply([0.8, 0.2, 0.1]);
        assert!((r - g).abs() < 0.02 && (g - b).abs() < 0.02, "{r} {g} {b}");
    }

    #[test]
    fn warm_temperature_lifts_red_shadows() {
        let [r, _, b] = correction(&[("temperature", 200.0)]).apply([0.2, 0.2, 0.2]);
        assert!(r > b, "{r} {b}");
    }
}
