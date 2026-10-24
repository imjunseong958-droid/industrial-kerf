//! Text effects: Numbers and Timecode.
//!
//! Effects sit below the text engine in the crate layering, so text here is drawn with a small
//! built-in single-stroke font (original work: capitals, digits and common punctuation laid out
//! on a 4 × 6 unit grid; lowercase letters use the capitals) and rasterised as anti-aliased round
//! strokes.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::generate3::{poly_segs, raster_segs};
use crate::util::{Plane, hash1};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, category: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category, params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

// ---------------------------------------------------------------- stroke font

/// Glyph strokes on a grid x ∈ [0, 4], y ∈ [0, 6] (y down, baseline at 6): polylines separated
/// by `;`, points by spaces. A single point is a dot.
fn glyph_src(c: char) -> &'static str {
    match c.to_ascii_uppercase() {
        '0' => "1,0 3,0 4,1 4,5 3,6 1,6 0,5 0,1 1,0; 3.6,0.6 0.4,5.4",
        '1' => "1,1 2,0 2,6; 1,6 3,6",
        '2' => "0,1 1,0 3,0 4,1 4,2 0,6 4,6",
        '3' => "0,0 4,0 2,2.5 3,2.5 4,3.5 4,5 3,6 1,6 0,5",
        '4' => "3,6 3,0 0,4 4,4",
        '5' => "4,0 0,0 0,2.5 3,2.5 4,3.5 4,5 3,6 0,6",
        '6' => "3.5,0 1,0 0,1 0,5 1,6 3,6 4,5 4,3.5 3,2.5 0,2.5",
        '7' => "0,0 4,0 1.5,6",
        '8' => "1,0 3,0 4,1 4,2 3,3 1,3 0,2 0,1 1,0; 1,3 0,4 0,5 1,6 3,6 4,5 4,4 3,3",
        '9' => "4,3.5 1,3.5 0,2.5 0,1 1,0 3,0 4,1 4,5 3,6 0.5,6",
        'A' => "0,6 2,0 4,6; 0.7,4 3.3,4",
        'B' => "0,0 0,6 3,6 4,5 4,4 3,3 0,3; 0,0 3,0 4,1 4,2 3,3",
        'C' => "4,1 3,0 1,0 0,1 0,5 1,6 3,6 4,5",
        'D' => "0,0 0,6 2.5,6 4,4.5 4,1.5 2.5,0 0,0",
        'E' => "4,0 0,0 0,6 4,6; 0,3 3,3",
        'F' => "4,0 0,0 0,6; 0,3 3,3",
        'G' => "4,1 3,0 1,0 0,1 0,5 1,6 3,6 4,5 4,3.5 2.5,3.5",
        'H' => "0,0 0,6; 4,0 4,6; 0,3 4,3",
        'I' => "1,0 3,0; 2,0 2,6; 1,6 3,6",
        'J' => "4,0 4,5 3,6 1,6 0,5",
        'K' => "0,0 0,6; 4,0 0,4; 1.3,3 4,6",
        'L' => "0,0 0,6 4,6",
        'M' => "0,6 0,0 2,3 4,0 4,6",
        'N' => "0,6 0,0 4,6 4,0",
        'O' => "1,0 3,0 4,1 4,5 3,6 1,6 0,5 0,1 1,0",
        'P' => "0,6 0,0 3,0 4,1 4,2 3,3 0,3",
        'Q' => "1,0 3,0 4,1 4,5 3,6 1,6 0,5 0,1 1,0; 2.5,4.5 4,6.3",
        'R' => "0,6 0,0 3,0 4,1 4,2 3,3 0,3; 2,3 4,6",
        'S' => "4,1 3,0 1,0 0,1 0,2 1,3 3,3 4,4 4,5 3,6 1,6 0,5",
        'T' => "0,0 4,0; 2,0 2,6",
        'U' => "0,0 0,5 1,6 3,6 4,5 4,0",
        'V' => "0,0 2,6 4,0",
        'W' => "0,0 1,6 2,2 3,6 4,0",
        'X' => "0,0 4,6; 4,0 0,6",
        'Y' => "0,0 2,3 4,0; 2,3 2,6",
        'Z' => "0,0 4,0 0,6 4,6",
        ':' => "2,2; 2,5",
        ';' => "2,2; 2,5 1.5,6.8",
        '.' => "2,6",
        ',' => "2,5.6 1.5,6.8",
        '-' => "1,3.5 3,3.5",
        '+' => "0.5,3.5 3.5,3.5; 2,2 2,5",
        '/' => "4,0 0,6",
        '(' => "2.5,-0.3 1.5,1.2 1.5,4.8 2.5,6.3",
        ')' => "1.5,-0.3 2.5,1.2 2.5,4.8 1.5,6.3",
        '\'' => "2,0 2,1.5",
        '"' => "1.4,0 1.4,1.5; 2.6,0 2.6,1.5",
        '!' => "2,0 2,4.2; 2,6",
        '?' => "0,1 1,0 3,0 4,1 4,2 2,3.5 2,4.4; 2,6",
        '%' => "0,6 4,0; 0.6,0.6; 3.4,5.4",
        '=' => "0.5,2.5 3.5,2.5; 0.5,4.5 3.5,4.5",
        '#' => "1.3,0.5 1,5.5; 3,0.5 2.7,5.5; 0,2 4,2; 0,4 4,4",
        '&' => "4,6 1,2 1,0.8 1.8,0 2.8,0.8 2.8,1.8 0,4 0,5.2 1,6 2.5,6 4,4",
        '*' => "2,1 2,4; 0.7,1.8 3.3,3.2; 3.3,1.8 0.7,3.2",
        '_' => "0,6.5 4,6.5",
        '<' => "4,1 0,3.5 4,6",
        '>' => "0,1 4,3.5 0,6",
        '@' => "3,4 3,2 1.8,2 1.2,3 1.8,4 3,4 4,3.5 4,1 3,0 1,0 0,1 0,5 1,6 3.5,6",
        '$' => "4,1 3,0.5 1,0.5 0,1.5 1,3 3,3 4,4.5 3,5.5 1,5.5 0,5; 2,-0.3 2,6.3",
        _ => "",
    }
}

/// A glyph's strokes on the 4 × 6 grid (Particle Playground's text particles).
pub(crate) fn glyph_strokes(c: char) -> Vec<Vec<[f64; 2]>> {
    parse_glyph(c)
}

fn parse_glyph(c: char) -> Vec<Vec<[f64; 2]>> {
    glyph_src(c)
        .split(';')
        .map(|s| {
            s.split_whitespace()
                .filter_map(|pt| {
                    let (x, y) = pt.split_once(',')?;
                    Some([x.parse().ok()?, y.parse().ok()?])
                })
                .collect::<Vec<[f64; 2]>>()
        })
        .filter(|v| !v.is_empty())
        .collect()
}

/// Advance width in grid units (glyph ink + 1.6 units of side bearing).
fn advance(c: char, proportional: bool) -> f64 {
    if c == ' ' {
        return 3.0;
    }
    if !proportional {
        return 5.6;
    }
    let g = parse_glyph(c);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in &g {
        for q in s {
            lo = lo.min(q[0]);
            hi = hi.max(q[0]);
        }
    }
    if lo > hi { 3.0 } else { (hi - lo) + 1.6 }
}

/// Left ink edge of a glyph (so proportional glyphs start at their ink).
fn ink_left(c: char, proportional: bool) -> f64 {
    if !proportional {
        return 0.8;
    }
    parse_glyph(c).iter().flatten().map(|q| q[0]).fold(f64::INFINITY, f64::min).min(4.0) - 0.8
}

/// Text metrics: grid unit in px for a font size (cap height ≈ 0.7 em) and stroke radius.
fn unit(size: f64) -> f64 {
    size * 0.7 / 6.0
}
fn weight(size: f64) -> f64 {
    (size * 0.045).max(0.35)
}

/// Width of a line of text in px (tracking in px between characters).
fn line_width(text: &str, size: f64, tracking: f64, proportional: bool) -> f64 {
    let u = unit(size);
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    text.chars().map(|c| advance(c, proportional) * u).sum::<f64>() + tracking * (n as f64 - 1.0) - 1.6 * u
}

/// Polylines (px) of a line of text whose baseline starts at `origin`.
fn layout_line(text: &str, origin: [f64; 2], size: f64, tracking: f64, proportional: bool, out: &mut Vec<Vec<[f64; 2]>>) {
    let u = unit(size);
    let mut x = origin[0];
    for c in text.chars() {
        let left = ink_left(c, proportional);
        for s in parse_glyph(c) {
            out.push(s.iter().map(|q| [x + (q[0] - left - 0.8) * u, origin[1] + (q[1] - 6.0) * u]).collect());
        }
        x += advance(c, proportional) * u + tracking;
    }
}

/// How text paints: fill/stroke display option, colours, and compositing.
/// How text is drawn: Display Options (0 fill only, 1 stroke only, 2 fill over stroke, 3 stroke
/// over fill), colours, stroke width (buffer px), on the layer or on transparency, opacity.
#[derive(Clone, Copy, Debug)]
pub struct TextLook {
    pub display: u32,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_w: f64,
    pub on_original: bool,
    pub opacity: f32,
}

fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

fn tint(c: [f32; 4], a: f32) -> Px {
    let a = (a * c[3]).clamp(0.0, 1.0);
    [c[0] * a, c[1] * a, c[2] * a, a]
}

/// Coverage of text polylines (`r` = glyph stroke radius in px): the fill and, when the look
/// strokes, the stroke ring.
fn text_coverage(w: usize, h: usize, polys: &[Vec<[f64; 2]>], r: f64, look: &TextLook) -> (Plane, Option<Plane>) {
    let segs_at = |rad: f64| {
        let mut s = Vec::new();
        for pl in polys {
            poly_segs(pl, false, rad.max(0.05), 1.0, &mut s);
        }
        s
    };
    let fill = raster_segs(w, h, &segs_at(r), 1.0);
    let sw = look.stroke_w.max(0.0);
    let ring = if look.display == 0 || sw <= 0.0 {
        None
    } else {
        let outer = raster_segs(w, h, &segs_at(r + sw * 0.5), 1.0);
        let inner = if r - sw * 0.5 > 0.05 { raster_segs(w, h, &segs_at(r - sw * 0.5), 1.0) } else { Plane::new(w, h) };
        Some(outer.zip_map(&inner, |a, b| (a - b).max(0.0)))
    };
    (fill, ring)
}

/// One pixel of drawn text: `px` under the fill / stroke coverages `f`, `s` (opacity applied).
fn text_px(px: Px, f: f32, s: f32, look: &TextLook) -> Px {
    let mut out = if look.on_original { px } else { [0.0; 4] };
    match look.display {
        1 => out = over(out, tint(look.stroke, s)),
        2 => {
            out = over(out, tint(look.stroke, s));
            out = over(out, tint(look.fill, f));
        }
        3 => {
            out = over(out, tint(look.fill, f));
            out = over(out, tint(look.stroke, s));
        }
        _ => out = over(out, tint(look.fill, f)),
    }
    out
}

/// Parameter `id`, read from its `group` twirl-down when the effect nests it there.
fn grouped<'a>(pr: &'a crate::Params, group: &str, id: &str) -> Option<&'a Value> {
    pr.get(&format!("{group}/{id}")).or_else(|| pr.get(id))
}

fn look_from(ctx: &EffectCtx, b: &Buf, on_original_id: &str) -> TextLook {
    let pr = ctx.params;
    let fs = |id: &str| grouped(pr, "fillAndStroke", id);
    TextLook {
        display: fs("displayOptions").map(Value::as_enum).unwrap_or(0),
        fill: fs("fillColor").map(Value::as_color).unwrap_or([1.0; 4]),
        stroke: fs("strokeColor").map(Value::as_color).unwrap_or([1.0; 4]),
        stroke_w: fs("strokeWidth").map(Value::as_f64).unwrap_or(0.0) * b.scale,
        on_original: pr.b(on_original_id),
        opacity: 1.0,
    }
}

// ---------------------------------------------------------------- Numbers

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Saturday", "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

/// Proleptic Gregorian date from a day count since 2000-01-01 (a Saturday).
fn date_from_days(days: i64) -> (i64, u32, u32, usize) {
    // Days since 0000-03-01 based civil calendar conversion.
    let z = days + 730_425; // 2000-01-01 relative to 0000-03-01
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, days.rem_euclid(7) as usize)
}

fn timecode(frames: i64, fps: f64) -> String {
    let fr = fps.round().max(1.0) as i64;
    let neg = frames < 0;
    let f = frames.abs();
    let s = format!("{:02}:{:02}:{:02}:{:02}", f / (fr * 3600), (f / (fr * 60)) % 60, (f / fr) % 60, f % fr);
    if neg { format!("-{s}") } else { s }
}

/// Numbers' twirl-down groups (Format, Fill and Stroke).
const NUMBERS_GROUPS: [&str; 2] = ["format/", "fillAndStroke/"];

/// `params` with Numbers' group prefixes stripped (`format/value` → `value`), so the drawing
/// helpers shared with Basic Text read plain ids. Plain keys already present win.
fn ungroup(params: &crate::Params) -> crate::Params {
    let mut out = params.clone();
    for (k, v) in &params.values {
        if let Some(leaf) = NUMBERS_GROUPS.iter().find_map(|g| k.strip_prefix(g)) {
            out.values.entry(leaf.to_string()).or_insert_with(|| v.clone());
        }
    }
    out
}

fn numbers_text(ctx: &EffectCtx) -> String {
    let flat = ungroup(ctx.params);
    let pr = &flat;
    let kind = pr.e("type");
    let dp = pr.f("decimalPlaces").round().clamp(0.0, 10.0) as usize;
    let mut v = pr.f("value");
    if pr.b("randomValues") {
        let frame = (ctx.time * ctx.fps()).floor() as i64 as u32;
        v = hash1(frame, 17, ctx.seed) as f64 * pr.f("value");
    }
    let cur = pr.b("currentTimeDate");
    let t = if cur { ctx.time + v } else { v };
    match kind {
        1 => {
            let s = format!("{:.*}", dp, v.abs());
            let (ip, fp) = s.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((s.clone(), String::new()));
            format!("{}{:0>5}{}", if v < 0.0 { "-" } else { "" }, ip, fp)
        }
        2..=4 => {
            let fps = [30.0, 25.0, 24.0][(kind - 2) as usize];
            timecode((t * fps + 1e-6).floor() as i64, fps)
        }
        5 => {
            let s = t.floor() as i64;
            let a = s.abs();
            format!("{}{:02}:{:02}:{:02}", if s < 0 { "-" } else { "" }, a / 3600, (a / 60) % 60, a % 60)
        }
        6..=8 => {
            let (y, m, d, wd) = date_from_days(v.floor() as i64);
            match kind {
                6 => format!("{:02}/{:02}/{:02}", m, d, y.rem_euclid(100)),
                7 => format!("{} {}, {}", &MONTHS[m as usize - 1][..3], d, y),
                _ => format!("{}, {} {}, {}", DAYS[wd], MONTHS[m as usize - 1], d, y),
            }
        }
        9 => {
            let i = v.round() as i64;
            if i < 0 { format!("-{:X}", -i) } else { format!("{i:X}") }
        }
        _ => format!("{:.*}", dp, if cur { t } else { v }),
    }
}

/// What Numbers or Timecode draws: glyph polylines, stroke radius and look, whether the layer
/// stays under the text, and Timecode's box (`[x0, x1, y0, y1]` in buffer pixels, colour,
/// opacity × colour alpha).
struct TextPlan {
    polys: Vec<Vec<[f64; 2]>>,
    r: f64,
    look: TextLook,
    keep: bool,
    boxed: Option<([f64; 4], [f32; 4], f32)>,
}

fn numbers_plan(ctx: &EffectCtx, b: &Buf) -> TextPlan {
    let flat = ungroup(ctx.params);
    let ctx = &EffectCtx { params: &flat, time: ctx.time, layer_size: ctx.layer_size, seed: ctx.seed, adjustment: ctx.adjustment, env: ctx.env };
    let pr = ctx.params;
    let text = numbers_text(ctx);
    let size = (pr.f("size") * b.scale).max(0.5);
    let tracking = pr.f("tracking") * b.scale;
    let prop = pr.b("proportionalSpacing");
    let pos = b.to_px(pr.v2("position"));
    let wdt = line_width(&text, size, tracking, prop);
    let mut polys = Vec::new();
    layout_line(&text, [pos.0 - wdt * 0.5, pos.1 + unit(size) * 3.0], size, tracking, prop, &mut polys);
    let mut look = look_from(ctx, b, "compositeOnOriginal");
    let keep = look.on_original;
    look.on_original = true;
    TextPlan { polys, r: weight(size), look, keep, boxed: None }
}

/// Timecode's box over pixel (`x`, `y`).
fn box_px(px: Px, x: usize, y: usize, (r, bc, a): ([f64; 4], [f32; 4], f32)) -> Px {
    let [x0, x1, y0, y1] = r;
    let fy = y as f64 + 0.5;
    let cy = ((fy - y0 + 0.5).clamp(0.0, 1.0) * (y1 - fy + 0.5).clamp(0.0, 1.0)) as f32;
    if cy <= 0.0 {
        return px;
    }
    let fx = x as f64 + 0.5;
    let cx = ((fx - x0 + 0.5).clamp(0.0, 1.0) * (x1 - fx + 0.5).clamp(0.0, 1.0)) as f32;
    let k = cx * cy * a;
    if k > 0.0 { over(px, [bc[0] * k, bc[1] * k, bc[2] * k, k]) } else { px }
}

fn draw_plan(mut b: Buf, plan: &TextPlan) -> Buf {
    let w = b.img.width as usize;
    let (fill, ring) = text_coverage(w, b.img.height as usize, &plan.polys, plan.r, &plan.look);
    let op = plan.look.opacity;
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let mut o = if plan.keep { *px } else { [0.0; 4] };
        if let Some(bx) = plan.boxed {
            o = box_px(o, i % w.max(1), i / w.max(1), bx);
        }
        let f = fill.data[i] * op;
        let s = ring.as_ref().map(|r| r.data[i]).unwrap_or(0.0) * op;
        *px = text_px(o, f, s, &plan.look);
    });
    b
}

fn numbers(ctx: &EffectCtx, b: Buf) -> Buf {
    let plan = numbers_plan(ctx, &b);
    draw_plan(b, &plan)
}

/// Numbers and Timecode for the GPU compositor (effectcraft-gpu `fx_text`): the glyph
/// coverage is rasterised here (the fill in x, the stroke ring in y, opacity applied), the
/// kernel composites it.
#[derive(Clone, Debug)]
pub struct TextLayer {
    pub coverage: Image,
    /// Display Options (0 fill only, 1 stroke only, 2 fill over stroke, 3 stroke over fill).
    pub display: u32,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    /// The layer stays under the text (else the text is drawn on transparency).
    pub keep: bool,
    /// Timecode's box: `[x0, x1, y0, y1]` (buffer pixels), colour, opacity × colour alpha.
    pub boxed: Option<([f64; 4], [f32; 4], f32)>,
}

/// [`TextLayer`] of Numbers (`ec.text.numbers`) or Timecode (`ec.text.timecode`) on a buffer
/// of `b`'s geometry.
pub fn text_layer(id: &str, ctx: &EffectCtx, b: &Buf) -> Option<TextLayer> {
    let plan = match id {
        "ec.text.numbers" => numbers_plan(ctx, b),
        "ec.text.timecode" => timecode_plan(ctx, b),
        _ => return None,
    };
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let (fill, ring) = text_coverage(w, h, &plan.polys, plan.r, &plan.look);
    let op = plan.look.opacity;
    let mut coverage = Image::new(b.img.width, b.img.height);
    coverage.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        *px = [fill.data[i] * op, ring.as_ref().map(|r| r.data[i]).unwrap_or(0.0) * op, 0.0, 0.0];
    });
    Some(TextLayer { coverage, display: plan.look.display, fill: plan.look.fill, stroke: plan.look.stroke, keep: plan.keep, boxed: plan.boxed })
}

// ---------------------------------------------------------------- Timecode

/// SMPTE drop-frame label for nominal 30/60 fps rates.
fn drop_frame(frames: i64, nominal: i64) -> String {
    let drop = nominal / 15; // 2 at 30, 4 at 60
    let per_min = nominal * 60 - drop;
    let per_10 = per_min * 10 + drop;
    let f = frames.max(0);
    let d = f / per_10;
    let m = f % per_10;
    let adj = if m > drop { f + drop * 9 * d + drop * ((m - drop) / per_min) } else { f + drop * 9 * d };
    format!("{:02};{:02};{:02};{:02}", adj / (nominal * 3600), (adj / (nominal * 60)) % 60, (adj / nominal) % 60, adj % nominal)
}

fn timecode_plan(ctx: &EffectCtx, b: &Buf) -> TextPlan {
    let pr = ctx.params;
    let src = pr.e("timeSource");
    let (t, fps) = match src {
        1 => (ctx.env.comp_time, ctx.fps()),
        2 => (ctx.time, pr.f("timeUnits").max(1.0)),
        _ => (ctx.time, ctx.fps()),
    };
    let mut frame = (t * fps + 1e-6).floor() as i64;
    if src == 2 {
        frame += pr.f("startingFrame").round() as i64;
    }
    let nominal = fps.round() as i64;
    let text = match pr.e("displayFormat") {
        1 => format!("{frame}"),
        2 => format!("{}+{:02}", frame.div_euclid(16), frame.rem_euclid(16)),
        3 => format!("{}+{:02}", frame.div_euclid(40), frame.rem_euclid(40)),
        _ => {
            if pr.b("dropFrame") && (nominal == 30 || nominal == 60) {
                drop_frame(frame, nominal)
            } else {
                timecode(frame, fps)
            }
        }
    };
    let size = (pr.f("textSize") * b.scale).max(0.5);
    let pos = b.to_px(pr.v2("textPosition"));
    let wdt = line_width(&text, size, 0.0, false);
    let op = (pr.f("opacity") / 100.0) as f32;
    let boxed = pr.b("showBox").then(|| {
        let bc = pr.color("boxColor");
        let pad = size * 0.25;
        ([pos.0 - wdt * 0.5 - pad, pos.0 + wdt * 0.5 + pad, pos.1 - size * 0.5, pos.1 + size * 0.5], bc, op * bc[3])
    });
    let mut polys = Vec::new();
    layout_line(&text, [pos.0 - wdt * 0.5, pos.1 + unit(size) * 3.0], size, 0.0, false, &mut polys);
    let look = TextLook { display: 0, fill: pr.color("textColor"), stroke: [0.0; 4], stroke_w: 0.0, on_original: true, opacity: op };
    TextPlan { polys, r: weight(size), look, keep: pr.b("renderOnOriginal"), boxed }
}

fn timecode_fx(ctx: &EffectCtx, b: Buf) -> Buf {
    let plan = timecode_plan(ctx, &b);
    draw_plan(b, &plan)
}

// ---------------------------------------------------------------- registry

pub fn specs() -> Vec<EffectSpec> {
    let check = |id: &'static str, name: &'static str, v: bool| p(id, name, Value::Bool(v), ParamUi::Checkbox);
    let pt = |x: f64, y: f64| Value::Vec2([x, y]);
    let display = || popup(&["Fill Only", "Stroke Only", "Fill Over Stroke", "Stroke Over Fill"]);
    let size = || slider(0.0, 512.0, 0.0, 200.0, 1);
    vec![
        spec(
            "ec.text.numbers",
            "Numbers",
            "Text",
            vec![
                p(
                    "format/type",
                    "Type",
                    Value::Enum(0),
                    popup(&[
                        "Number",
                        "Number [Leading Zeros]",
                        "Timecode [30]",
                        "Timecode [25]",
                        "Timecode [24]",
                        "Time",
                        "Numerical Date",
                        "Short Date",
                        "Long Date",
                        "Hexadecimal",
                    ]),
                ),
                check("format/randomValues", "Random Values", false),
                p("format/value", "Value/Offset/Random Max", num(0.0), slider(-30000.0, 30000.0, -1000.0, 1000.0, 3)),
                p("format/decimalPlaces", "Decimal Places", num(2.0), slider(0.0, 10.0, 0.0, 10.0, 0)),
                check("format/currentTimeDate", "Current Time/Date", false),
                p("fillAndStroke/position", "Position", pt(0.5, 0.5), ParamUi::Point),
                p("fillAndStroke/displayOptions", "Display Options", Value::Enum(0), display()),
                p("fillAndStroke/fillColor", "Fill Color", col(1.0, 0.0, 0.0), ParamUi::Color),
                p("fillAndStroke/strokeColor", "Stroke Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("fillAndStroke/strokeWidth", "Stroke Width", num(2.0), slider(0.0, 50.0, 0.0, 50.0, 1)),
                p("size", "Size", num(36.0), size()),
                p("tracking", "Tracking", num(0.0), slider(-100.0, 100.0, -20.0, 50.0, 1)),
                check("proportionalSpacing", "Proportional Spacing", true),
                check("compositeOnOriginal", "Composite On Original", true),
            ],
            numbers,
        ),
        spec(
            "ec.text.timecode",
            "Timecode",
            "Text",
            vec![
                p("displayFormat", "Display Format", Value::Enum(0), popup(&["SMPTE HH:MM:SS:FF", "Frames", "Feet + Frames (35mm)", "Feet + Frames (16mm)"])),
                p("timeSource", "Time Source", Value::Enum(0), popup(&["Layer Source", "Composition", "Custom"])),
                p("timeUnits", "Time Units", num(30.0), slider(1.0, 120.0, 1.0, 60.0, 2)),
                check("dropFrame", "Drop Frame", false),
                p("startingFrame", "Starting Frame", num(0.0), slider(-1_000_000.0, 1_000_000.0, 0.0, 10_000.0, 0)),
                p("textPosition", "Text Position", pt(0.5, 0.9), ParamUi::Point),
                p("textSize", "Text Size", num(36.0), size()),
                p("textColor", "Text Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                check("showBox", "Show Box", true),
                p("boxColor", "Box Color", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("opacity", "Opacity", num(100.0), pct()),
                check("renderOnOriginal", "Composite on Original", true),
            ],
            timecode_fx,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, Params};

    fn run_env(id: &str, set: &[(&str, Value)], img: Image, time: f64, env: EffectEnv) -> Buf {
        let s = crate::find(id).unwrap();
        let ls = [img.width as f64, img.height as f64];
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for p in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&p.ui, &p.default) {
                params.values.insert(p.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            assert!(params.values.contains_key(*k), "{id}: unknown {k}");
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time, layer_size: ls, seed: 3, adjustment: false, env };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn run(id: &str, set: &[(&str, Value)], img: Image, time: f64) -> Buf {
        run_env(id, set, img, time, EffectEnv::default())
    }

    #[test]
    fn every_glyph_parses_within_grid() {
        for c in (b'!'..=b'~').map(|c| c as char) {
            for s in parse_glyph(c) {
                for q in s {
                    assert!((-0.5..=4.5).contains(&q[0]) && (-0.5..=7.0).contains(&q[1]), "{c}: {q:?}");
                }
            }
        }
        assert!(!parse_glyph('A').is_empty() && !parse_glyph('a').is_empty() && parse_glyph(' ').is_empty());
    }

    #[test]
    fn numbers_formats() {
        let params = |vals: &[(&str, Value)]| {
            let s = crate::find("ec.text.numbers").unwrap();
            let mut p = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
            for (k, v) in vals {
                p.values.insert(k.to_string(), v.clone());
            }
            p
        };
        let txt = |vals: &[(&str, Value)], t: f64| {
            let pr = params(vals);
            let ctx = EffectCtx { params: &pr, time: t, layer_size: [10.0, 10.0], seed: 1, adjustment: false, env: Default::default() };
            numbers_text(&ctx)
        };
        assert_eq!(txt(&[("value", num(8.0))], 0.0), "8.00");
        assert_eq!(txt(&[("value", num(42.5)), ("type", Value::Enum(1))], 0.0), "00042.50");
        assert_eq!(txt(&[("value", num(3661.5)), ("type", Value::Enum(2))], 0.0), "01:01:01:15");
        assert_eq!(txt(&[("value", num(255.0)), ("type", Value::Enum(9))], 0.0), "FF");
        assert_eq!(txt(&[("value", num(0.0)), ("type", Value::Enum(6))], 0.0), "01/01/00");
        assert_eq!(txt(&[("value", num(60.0)), ("type", Value::Enum(8))], 0.0), "Wednesday, March 1, 2000");
        assert_eq!(txt(&[("value", num(1.0)), ("currentTimeDate", Value::Bool(true)), ("decimalPlaces", num(1.0))], 2.0), "3.0");
        assert_ne!(
            txt(&[("value", num(100.0)), ("randomValues", Value::Bool(true))], 0.0),
            txt(&[("value", num(100.0)), ("randomValues", Value::Bool(true))], 1.0)
        );
    }

    #[test]
    fn numbers_draws_glyph_at_position() {
        let img = Image::new(80, 60);
        let a = run("ec.text.numbers", &[("format/value", num(8.0)), ("format/decimalPlaces", num(0.0))], img.clone(), 0.0);
        assert_eq!(a.img.data, run("ec.text.numbers", &[("format/value", num(8.0)), ("format/decimalPlaces", num(0.0))], img.clone(), 0.0).img.data);
        // Ink only around the centre.
        let mut inside = 0.0;
        let mut outside = 0.0;
        for y in 0..60 {
            for x in 0..80 {
                let v = a.img.get(x, y)[3];
                if (25..55).contains(&x) && (10..50).contains(&y) {
                    inside += v;
                } else {
                    outside += v;
                }
            }
        }
        assert!(inside > 20.0 && outside < 0.5, "{inside} {outside}");
        let stroke = run("ec.text.numbers", &[("format/value", num(8.0)), ("fillAndStroke/displayOptions", Value::Enum(2))], img, 0.0);
        assert!(stroke.img.data.iter().any(|p| p[3] > 0.5 && p[1] > 0.5 * p[3]), "white stroke visible");
    }

    #[test]
    fn timecode_changes_over_time() {
        let img = Image::filled(120, 60, [0.2, 0.2, 0.2, 1.0]);
        let a = run("ec.text.timecode", &[], img.clone(), 0.0);
        let b = run("ec.text.timecode", &[], img.clone(), 1.0);
        assert_eq!(a.img.data, run("ec.text.timecode", &[], img.clone(), 0.0).img.data);
        assert_ne!(a.img.data, b.img.data);
        assert_eq!(timecode(30 * 61 + 5, 30.0), "00:01:01:05");
        assert_eq!(drop_frame(1800, 30), "00;01;00;02");
        assert_eq!(drop_frame(17982, 30), "00;10;00;00");
    }
}
