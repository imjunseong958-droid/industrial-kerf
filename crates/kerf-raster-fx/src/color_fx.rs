//! Color Correction effects.

use crate::ParamUi;
use crate::Value;
use kerf_color::{hsl_to_rgb, luma709, rgb_to_hsl};

use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Color Correction", params, render, gpu: false, float: true }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Hue/Saturation's colour ranges (Channel Control entries after Master): id prefix, display
/// name and centre hue in degrees.
pub const HUESAT_RANGES: [(&str, &str, f64); 6] = [
    ("reds", "Reds", 0.0),
    ("yellows", "Yellows", 60.0),
    ("greens", "Greens", 120.0),
    ("cyans", "Cyans", 180.0),
    ("blues", "Blues", 240.0),
    ("magentas", "Magentas", 300.0),
];

/// Hue/Saturation's Channel Control popup.
pub const HUESAT_CHANNELS: [&str; 7] = ["Master", "Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];

/// One colour range of Hue/Saturation: the hues it covers fully (`start..end`, degrees, may
/// wrap), the fall-off widths either side, and its hue / saturation / lightness adjustments.
#[derive(Clone, Copy, Debug)]
struct HueRange {
    start: f32,
    end: f32,
    fall_start: f32,
    fall_end: f32,
    hue: f32,
    sat: f32,
    light: f32,
}

impl HueRange {
    /// Membership of hue `h` (degrees): 1 inside the range, ramping to 0 across the fall-offs.
    fn weight(&self, h: f32) -> f32 {
        let width = (self.end - self.start).rem_euclid(360.0);
        let d = (h - self.start).rem_euclid(360.0);
        if d <= width {
            return 1.0;
        }
        let after = d - width;
        let before = 360.0 - d;
        let wa = if self.fall_end > 0.0 { 1.0 - after / self.fall_end } else { 0.0 };
        let wb = if self.fall_start > 0.0 { 1.0 - before / self.fall_start } else { 0.0 };
        wa.max(wb).clamp(0.0, 1.0)
    }
}

/// The colour ranges with non-zero adjustments.
fn huesat_ranges(ctx: &EffectCtx) -> Vec<HueRange> {
    let f = |id: String, d: f64| ctx.params.get(&id).map(Value::as_f64).unwrap_or(d) as f32;
    HUESAT_RANGES
        .iter()
        .map(|(r, _, c)| HueRange {
            start: f(format!("{r}RangeStart"), c - 15.0),
            end: f(format!("{r}RangeEnd"), c + 15.0),
            fall_start: f(format!("{r}StartFalloff"), 30.0).max(0.0),
            fall_end: f(format!("{r}EndFalloff"), 30.0).max(0.0),
            hue: f(format!("{r}Hue"), 0.0) / 360.0,
            sat: f(format!("{r}Saturation"), 0.0) / 100.0,
            light: f(format!("{r}Lightness"), 0.0) / 100.0,
        })
        .filter(|r| r.hue != 0.0 || r.sat != 0.0 || r.light != 0.0)
        .collect()
}

fn adjust_sat(s: f32, sat: f32) -> f32 {
    if sat >= 0.0 { s + (1.0 - s) * sat * s.min(1.0) } else { s * (1.0 + sat) }
}

fn adjust_light(l: f32, light: f32) -> f32 {
    if light >= 0.0 { l + (1.0 - l) * light } else { l * (1.0 + light) }
}

/// Hue/Saturation. Master adjusts every pixel; each colour range (Reds … Magentas, chosen in
/// Channel Control) adjusts pixels whose original hue lies in its range, fading across the
/// fall-offs (greys, having no hue, belong to no range). Colorize replaces hue and
/// saturation.
fn hue_saturation(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hue = ctx.params.f("hue") as f32 / 360.0;
    let sat = ctx.params.f("saturation") as f32 / 100.0;
    let light = ctx.params.f("lightness") as f32 / 100.0;
    let colorize = ctx.params.b("colorize");
    let ch = ctx.params.f("colorizeHue") as f32 / 360.0;
    let cs = ctx.params.f("colorizeSaturation") as f32 / 100.0;
    let cl = ctx.params.f("colorizeLightness") as f32 / 100.0;
    let ranges = huesat_ranges(ctx);
    b.img.map_straight(|c| {
        let [h, s, l] = rgb_to_hsl(c[0], c[1], c[2]);
        let (h, s, l) = if colorize {
            (ch, cs, (l + cl * if cl > 0.0 { 1.0 - l } else { l }).clamp(0.0, 1.0))
        } else {
            let mut s2 = adjust_sat(s, sat);
            let mut l2 = adjust_light(l, light);
            let mut h2 = h + hue;
            for r in &ranges {
                let w = r.weight(h * 360.0) * (s * 20.0).min(1.0);
                if w > 0.0 {
                    h2 += r.hue * w;
                    s2 = adjust_sat(s2.clamp(0.0, 1.0), r.sat * w);
                    l2 = adjust_light(l2, r.light * w);
                }
            }
            (h2.rem_euclid(1.0), s2.clamp(0.0, 1.0), l2)
        };
        let [r, g, bl] = hsl_to_rgb(h, s, l);
        [r, g, bl]
    });
    b
}

/// Options of Levels' Clip To Output Black / White.
pub const LEVELS_CLIP: [&str; 3] = ["Off", "On", "Off for 32 bpc Color"];

/// Photo Filter's Filter menu: the conventional camera filters (Wratten-style warming /
/// cooling conversion and light-balancing filters) and plain colours, as 8-bit sRGB values
/// chosen for EffectCraft; the last entry, Custom, uses the Color parameter.
pub const PHOTO_FILTERS: [(&str, [u8; 3]); 21] = [
    ("Warming Filter (85)", [240, 140, 30]),
    ("Warming Filter (LBA)", [245, 155, 25]),
    ("Warming Filter (81)", [240, 180, 40]),
    ("Cooling Filter (80)", [20, 110, 250]),
    ("Cooling Filter (LBB)", [20, 95, 245]),
    ("Cooling Filter (82)", [30, 180, 250]),
    ("Red", [230, 30, 30]),
    ("Orange", [240, 130, 30]),
    ("Yellow", [245, 225, 35]),
    ("Green", [30, 200, 30]),
    ("Cyan", [30, 200, 230]),
    ("Blue", [30, 55, 230]),
    ("Violet", [150, 30, 230]),
    ("Magenta", [225, 30, 225]),
    ("Sepia", [170, 120, 55]),
    ("Deep Red", [250, 0, 0]),
    ("Deep Blue", [0, 40, 200]),
    ("Deep Emerald", [0, 135, 0]),
    ("Deep Yellow", [255, 210, 0]),
    ("Underwater", [0, 190, 175]),
    ("Custom", [0, 0, 0]),
];
/// Index of Photo Filter's Custom entry.
pub const PHOTO_FILTER_CUSTOM: u32 = PHOTO_FILTERS.len() as u32 - 1;

fn photo_filter(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let f = ctx.params.e("filter");
    let c = match PHOTO_FILTERS.get(f as usize) {
        Some((_, rgb)) if f != PHOTO_FILTER_CUSTOM => rgb.map(|v| v as f32 / 255.0),
        _ => {
            let c = ctx.params.color("color");
            [c[0], c[1], c[2]]
        }
    };
    let d = ctx.params.f("density") as f32 / 100.0;
    let keep = ctx.params.b("preserveLuminosity");
    b.img.map_straight(|x| {
        let f = [x[0] * c[0], x[1] * c[1], x[2] * c[2]];
        let mut o = mix(x, f, d);
        if keep {
            let l0 = luma709(x[0], x[1], x[2]);
            let l1 = luma709(o[0], o[1], o[2]).max(1e-6);
            o = o.map(|v| v * l0 / l1);
        }
        o
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pct = || slider(-100.0, 100.0, -100.0, 100.0, 1);
    let mut gpg = vec![p("blackStretch", "Black Stretch", num(0.0), slider(0.0, 4.0, 0.0, 4.0, 2))];
    for (n, label) in [("red", "Red"), ("green", "Green"), ("blue", "Blue")] {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        gpg.push(p(leak(format!("{n}Gamma")), leak(format!("{label} Gamma")), num(1.0), slider(0.1, 10.0, 0.1, 4.0, 2)));
        gpg.push(p(leak(format!("{n}Pedestal")), leak(format!("{label} Pedestal")), num(0.0), slider(-2.0, 2.0, -1.0, 1.0, 2)));
        gpg.push(p(leak(format!("{n}Gain")), leak(format!("{label} Gain")), num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)));
    }
    vec![
        spec(
            "ec.color.huesaturation",
            "Hue/Saturation",
            {
                let mut v = vec![
                    p("channelControl", "Channel Control", Value::Enum(0), popup(&HUESAT_CHANNELS)),
                    p("hue", "Master Hue", num(0.0), ParamUi::Angle),
                    p("saturation", "Master Saturation", num(0.0), pct()),
                    p("lightness", "Master Lightness", num(0.0), pct()),
                ];
                let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
                for (r, name, c) in HUESAT_RANGES {
                    let single = name.trim_end_matches('s');
                    v.push(p(leak(format!("{r}Hue")), leak(format!("{single} Hue")), num(0.0), ParamUi::Angle));
                    v.push(p(leak(format!("{r}Saturation")), leak(format!("{single} Saturation")), num(0.0), pct()));
                    v.push(p(leak(format!("{r}Lightness")), leak(format!("{single} Lightness")), num(0.0), pct()));
                    v.push(p(leak(format!("{r}RangeStart")), leak(format!("{single} Range Start")), num(c - 15.0), slider(-360.0, 720.0, -180.0, 360.0, 0)));
                    v.push(p(leak(format!("{r}RangeEnd")), leak(format!("{single} Range End")), num(c + 15.0), slider(-360.0, 720.0, -180.0, 360.0, 0)));
                    v.push(p(leak(format!("{r}StartFalloff")), leak(format!("{single} Start Falloff")), num(30.0), slider(0.0, 180.0, 0.0, 90.0, 0)));
                    v.push(p(leak(format!("{r}EndFalloff")), leak(format!("{single} End Falloff")), num(30.0), slider(0.0, 180.0, 0.0, 90.0, 0)));
                }
                v.extend([
                    p("colorize", "Colorize", Value::Bool(false), ParamUi::Checkbox),
                    p("colorizeHue", "Colorize Hue", num(0.0), ParamUi::Angle),
                    p("colorizeSaturation", "Colorize Saturation", num(25.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                    p("colorizeLightness", "Colorize Lightness", num(0.0), pct()),
                ]);
                v
            },
            hue_saturation,
        ),
        spec(
            "ec.color.photofilter",
            "Photo Filter",
            vec![
                p("filter", "Filter", Value::Enum(0), popup(&PHOTO_FILTERS.map(|f| f.0))),
                p("color", "Color", col(0.93, 0.54, 0.09), ParamUi::Color),
                p("density", "Density", num(25.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("preserveLuminosity", "Preserve Luminosity", Value::Bool(true), ParamUi::Checkbox),
            ],
            photo_filter,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};
    use kerf_raster::Image;

    fn px(id: &str, vals: &[(&str, Value)], c: [f32; 4]) -> [f32; 4] {
        run_fx(id, vals, Image::filled(2, 2, c), 0.0, EffectEnv::default()).img.get(0, 0)
    }

    #[test]
    fn photo_filter_presets_and_custom() {
        let g = [0.5, 0.5, 0.5, 1.0];
        let cool = px("ec.color.photofilter", &[("filter", Value::Enum(3)), ("preserveLuminosity", Value::Bool(false))], g);
        assert!(cool[2] > cool[0], "a cooling filter is blue: {cool:?}");
        let custom = |c| px("ec.color.photofilter", &[("filter", Value::Enum(PHOTO_FILTER_CUSTOM)), ("color", c)], g);
        assert!(custom(col(0.0, 1.0, 0.0))[1] > custom(col(1.0, 0.0, 0.0))[1]);
        // The Color swatch is ignored by the presets.
        let warm = |c| px("ec.color.photofilter", &[("color", c)], g);
        assert_eq!(warm(col(0.0, 1.0, 0.0)), warm(col(1.0, 0.0, 0.0)));
    }
}
