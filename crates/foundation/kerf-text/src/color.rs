//! Colour glyphs: what a font draws in colour rather than as one outline, which is how emoji
//! come. `COLR` layers (v0 solid layers; v1 gradients, transforms and composites) are filled as
//! outlines with the font's `CPAL` palette, and `CBDT` / `sbix` PNG bitmaps (Noto Color Emoji,
//! Apple Color Emoji) are resampled, bilinear, from a halved copy where they are drawn smaller than
//! half. Either is drawn under any affine transform.
//!
//! Colours come as fonts give them: sRGB-encoded, and premultiplied here. A caller that composites
//! in linear light un-premultiplies, decodes and premultiplies again. Where a glyph asks for the
//! text's own colour (palette index 0xFFFF, or a monochrome bitmap), it takes `foreground`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use skrifa::bitmap::{BitmapData, BitmapStrikes, Origin};
use skrifa::color::{
    Brush, ColorGlyphCollection, ColorPainter, ColorPalettes, ColorStop, CompositeMode, Extend,
    Transform,
};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::types::BoundingBox;
use skrifa::GlyphId;

use crate::fonts::{self, FaceId};
use crate::layout::Glyph;
use crate::raster::{apply, compose, fill_into, xform_scale, Mask, Path, Xform};

/// A colour picture: RGBA, premultiplied, sRGB-encoded, 0 to 1, row by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorImage {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

impl ColorImage {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }
    /// Draw `src`, its top-left at (`ox`, `oy`), over this (source over).
    pub fn over(&mut self, src: &ColorImage, ox: i32, oy: i32) {
        for y in 0..src.h {
            let ty = oy + y as i32;
            if ty < 0 || ty >= self.h as i32 {
                continue;
            }
            let x0 = (-ox).max(0) as usize;
            let x1 = src.w.min((self.w as i32 - ox).max(0) as usize);
            if x0 >= x1 {
                continue;
            }
            let d0 = ty as usize * self.w + (ox + x0 as i32) as usize;
            for (d, s) in self.px[d0..d0 + (x1 - x0)]
                .iter_mut()
                .zip(&src.px[y * src.w + x0..y * src.w + x1])
            {
                let keep = 1.0 - s[3];
                for k in 0..4 {
                    d[k] = s[k] + d[k] * keep;
                }
            }
        }
    }
    /// How much of each pixel is covered: its alpha, as a mask.
    pub fn alpha(&self) -> Mask {
        Mask {
            w: self.w,
            h: self.h,
            a: self.px.iter().map(|p| p[3]).collect(),
        }
    }
}

/// How a glyph is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Outline,
    Colr,
    Bitmap,
}

fn kind(face: FaceId, gid: u32) -> Kind {
    static C: OnceLock<Mutex<HashMap<(FaceId, u32), Kind>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    if let Some(k) = c
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(face, gid))
    {
        return *k;
    }
    let f = fonts::face(face);
    let k = match f.font().filter(|_| f.info.color) {
        None => Kind::Outline,
        Some(font)
            if ColorGlyphCollection::new(&font)
                .get(GlyphId::new(gid))
                .is_some() =>
        {
            Kind::Colr
        }
        Some(font)
            if BitmapStrikes::new(&font)
                .iter()
                .any(|strike| strike.get(GlyphId::new(gid)).is_some()) =>
        {
            Kind::Bitmap
        }
        Some(_) => Kind::Outline,
    };
    let mut m = c.lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 50_000 {
        m.clear();
    }
    m.insert((face, gid), k);
    k
}

/// Whether glyph `gid` of `face` is drawn in colour.
pub fn is_color(face: FaceId, gid: u32) -> bool {
    kind(face, gid) != Kind::Outline
}

/// The most pixels one colour glyph is painted over, so a huge size cannot exhaust memory.
const MAX_PIXELS: usize = 4 << 20;

/// Paint colour glyph `g` under `m` (glyph-local pixels, origin on its baseline, y down → device)
/// over the part of its bounds inside `clip` (`[x0, y0, x1, y1]`, device pixels). Returns the
/// picture and the device pixel of its top-left; None for a glyph not drawn in colour, or one
/// with nothing inside `clip`.
pub fn paint(
    g: &Glyph,
    m: &Xform,
    clip: [i32; 4],
    foreground: [f32; 4],
) -> Option<(ColorImage, i32, i32)> {
    let f = fonts::face(g.face);
    let font = f.font()?;
    let k = g.size / f.units_per_em();
    // font units (y up) → device
    let to_device = compose(m, &[k, 0.0, 0.0, -k, 0.0, 0.0]);
    match kind(g.face, g.id) {
        Kind::Outline => None,
        Kind::Colr => {
            let glyph = ColorGlyphCollection::new(&font).get(GlyphId::new(g.id))?;
            let location = fonts::location_of(g.instance);
            let at = location
                .as_deref()
                .map_or(LocationRef::default(), LocationRef::from);
            let mut bounds = Bounds {
                face: g.face,
                instance: g.instance,
                transforms: vec![to_device],
                area: None,
                boxes: None,
            };
            glyph.paint(at, &mut bounds).ok()?;
            let (x, y, w, h) = region(bounds.area.or(bounds.boxes)?, clip)?;
            let palette: Vec<[f32; 4]> = ColorPalettes::new(&font)
                .get(0)
                .map(|p| {
                    p.colors()
                        .iter()
                        .map(|c| [c.red, c.green, c.blue, c.alpha].map(|v| f32::from(v) / 255.0))
                        .collect()
                })
                .unwrap_or_default();
            let start = compose(&[1.0, 0.0, 0.0, 1.0, -x as f32, -y as f32], &to_device);
            let mut painter = Painter {
                face: g.face,
                instance: g.instance,
                palette,
                foreground,
                w,
                h,
                transforms: vec![start],
                clips: Vec::new(),
                layers: vec![vec![[0.0; 4]; w * h]],
            };
            glyph.paint(at, &mut painter).ok()?;
            let px = painter.layers.into_iter().next()?;
            Some((ColorImage { w, h, px }, x, y))
        }
        Kind::Bitmap => bitmap(g, &font, m, clip, foreground),
    }
}

/// `area` (device `[x0, y0, x1, y1]`) in whole pixels, within `clip`: `(x, y, w, h)`.
fn region(area: [f32; 4], clip: [i32; 4]) -> Option<(i32, i32, usize, usize)> {
    if !area.iter().all(|v| v.is_finite()) {
        return None;
    }
    let x0 = (area[0].floor() as i32 - 1).max(clip[0]);
    let y0 = (area[1].floor() as i32 - 1).max(clip[1]);
    let x1 = (area[2].ceil() as i32 + 1).min(clip[2]);
    let y1 = (area[3].ceil() as i32 + 1).min(clip[3]);
    let (w, h) = (
        usize::try_from(x1 - x0).ok()?,
        usize::try_from(y1 - y0).ok()?,
    );
    (w > 0 && h > 0 && w.checked_mul(h)? <= MAX_PIXELS).then_some((x0, y0, w, h))
}

fn xform(t: &Transform) -> Xform {
    [t.xx, t.yx, t.xy, t.yy, t.dx, t.dy]
}

fn invert(m: &Xform) -> Option<Xform> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

fn union(a: Option<[f32; 4]>, b: [f32; 4]) -> [f32; 4] {
    a.map_or(b, |a| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

fn path_area(p: &Path) -> Option<[f32; 4]> {
    p.bounds().map(|(x0, y0, x1, y1)| [x0, y0, x1, y1])
}

fn box_path(b: &BoundingBox<f32>, t: &Xform) -> Path {
    Path::rect(b.x_min, b.y_min, b.x_max, b.y_max).transformed(t)
}

/// A first pass over a COLR glyph: where its layers' outlines (or, with none, its clip boxes) fall.
struct Bounds {
    face: FaceId,
    instance: u32,
    transforms: Vec<Xform>,
    area: Option<[f32; 4]>,
    boxes: Option<[f32; 4]>,
}

impl Bounds {
    fn current(&self) -> Xform {
        self.transforms
            .last()
            .copied()
            .unwrap_or(crate::raster::IDENTITY)
    }
}

impl ColorPainter for Bounds {
    fn push_transform(&mut self, t: Transform) {
        let next = compose(&self.current(), &xform(&t));
        self.transforms.push(next);
    }
    fn pop_transform(&mut self) {
        self.transforms.pop();
    }
    fn push_clip_glyph(&mut self, glyph: GlyphId) {
        let p = crate::render::outline_path(
            self.face,
            glyph.to_u32(),
            self.instance,
            &self.current(),
            1.0,
        );
        if let Some(b) = path_area(&p) {
            self.area = Some(union(self.area, b));
        }
    }
    fn push_clip_box(&mut self, clip: BoundingBox<f32>) {
        if let Some(b) = path_area(&box_path(&clip, &self.current())) {
            self.boxes = Some(union(self.boxes, b));
        }
    }
    fn pop_clip(&mut self) {}
    fn fill(&mut self, _brush: Brush<'_>) {}
    fn push_layer(&mut self, _mode: CompositeMode) {}
}

/// Paints a COLR glyph into its region: a stack of transforms (font units → region pixels), of
/// clips (coverage masks, each the intersection of those under it) and of layers.
struct Painter {
    face: FaceId,
    instance: u32,
    /// Straight sRGB colours of palette 0.
    palette: Vec<[f32; 4]>,
    foreground: [f32; 4],
    w: usize,
    h: usize,
    transforms: Vec<Xform>,
    clips: Vec<Mask>,
    layers: Vec<Vec<[f32; 4]>>,
}

impl Painter {
    fn current(&self) -> Xform {
        self.transforms
            .last()
            .copied()
            .unwrap_or(crate::raster::IDENTITY)
    }
    fn push_clip_path(&mut self, p: &Path) {
        let mut mask = Mask::new(self.w, self.h);
        fill_into(p, &mut mask, 0.0, 0.0);
        if let Some(under) = self.clips.last() {
            for (a, b) in mask.a.iter_mut().zip(&under.a) {
                *a *= b;
            }
        }
        self.clips.push(mask);
    }
    /// A palette entry with `alpha` applied, premultiplied.
    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let c = if index == 0xFFFF {
            self.foreground
        } else {
            self.palette
                .get(usize::from(index))
                .copied()
                .unwrap_or([0.0; 4])
        };
        let a = (c[3] * alpha).clamp(0.0, 1.0);
        [c[0] * a, c[1] * a, c[2] * a, a]
    }
    fn stops(&self, stops: &[ColorStop]) -> Vec<(f32, [f32; 4])> {
        stops
            .iter()
            .map(|s| (s.offset, self.color(s.palette_index, s.alpha)))
            .collect()
    }
}

/// The colour a colour line has at `t`.
fn along(stops: &[(f32, [f32; 4])], t: f32, extend: Extend) -> [f32; 4] {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return [0.0; 4];
    };
    let t = match extend {
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let u = t.rem_euclid(2.0);
            if u > 1.0 {
                2.0 - u
            } else {
                u
            }
        }
        _ => t,
    };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = stops
        .partition_point(|s| s.0 <= t)
        .clamp(1, stops.len() - 1);
    let (a, b) = (stops[i - 1], stops[i]);
    let span = b.0 - a.0;
    let f = if span > 1e-9 { (t - a.0) / span } else { 1.0 };
    [0, 1, 2, 3].map(|k| a.1[k] + (b.1[k] - a.1[k]) * f)
}

/// Where along a two-point conical gradient `p` falls: the greatest `t` whose circle (centre
/// `c0 + t (c1 - c0)`, radius `r0 + t (r1 - r0)`, not negative) passes through `p`.
fn conical(p: (f32, f32), c0: (f32, f32), r0: f32, c1: (f32, f32), r1: f32) -> Option<f32> {
    let (cdx, cdy, dr) = (c1.0 - c0.0, c1.1 - c0.1, r1 - r0);
    let (pdx, pdy) = (p.0 - c0.0, p.1 - c0.1);
    let a = cdx * cdx + cdy * cdy - dr * dr;
    let b = pdx * cdx + pdy * cdy + r0 * dr;
    let c = pdx * pdx + pdy * pdy - r0 * r0;
    let ok = |t: f32| r0 + t * dr >= 0.0;
    if a.abs() < 1e-6 {
        if b.abs() < 1e-9 {
            return None;
        }
        let t = c / (2.0 * b);
        return ok(t).then_some(t);
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let (t1, t2) = ((b + root) / a, (b - root) / a);
    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
    if ok(hi) {
        Some(hi)
    } else if ok(lo) {
        Some(lo)
    } else {
        None
    }
}

impl ColorPainter for Painter {
    fn push_transform(&mut self, t: Transform) {
        let next = compose(&self.current(), &xform(&t));
        self.transforms.push(next);
    }
    fn pop_transform(&mut self) {
        self.transforms.pop();
    }
    fn push_clip_glyph(&mut self, glyph: GlyphId) {
        let p = crate::render::outline_path(
            self.face,
            glyph.to_u32(),
            self.instance,
            &self.current(),
            1.0,
        );
        self.push_clip_path(&p);
    }
    fn push_clip_box(&mut self, clip: BoundingBox<f32>) {
        let p = box_path(&clip, &self.current());
        self.push_clip_path(&p);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn fill(&mut self, brush: Brush<'_>) {
        let w = self.w;
        let inverse = invert(&self.current());
        // The colour at a pixel centre, by where it falls in the brush's space (font units).
        let shade: Box<dyn Fn(f32, f32) -> [f32; 4]> = match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                let c = self.color(palette_index, alpha);
                Box::new(move |_, _| c)
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => {
                let (stops, Some(inv)) = (self.stops(color_stops), inverse) else {
                    return;
                };
                let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
                let len = dx * dx + dy * dy;
                Box::new(move |x, y| {
                    let (fx, fy) = apply(&inv, x, y);
                    let t = if len > 1e-9 {
                        ((fx - p0.x) * dx + (fy - p0.y) * dy) / len
                    } else {
                        0.0
                    };
                    along(&stops, t, extend)
                })
            }
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let (stops, Some(inv)) = (self.stops(color_stops), inverse) else {
                    return;
                };
                Box::new(move |x, y| {
                    let p = apply(&inv, x, y);
                    conical(p, (c0.x, c0.y), r0, (c1.x, c1.y), r1)
                        .map_or([0.0; 4], |t| along(&stops, t, extend))
                })
            }
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend,
            } => {
                let (stops, Some(inv)) = (self.stops(color_stops), inverse) else {
                    return;
                };
                let span = end_angle - start_angle;
                Box::new(move |x, y| {
                    let (fx, fy) = apply(&inv, x, y);
                    // Clockwise, as skrifa gives the angles, from the x axis of the design grid.
                    let angle = (-(fy - c0.y).atan2(fx - c0.x))
                        .to_degrees()
                        .rem_euclid(360.0);
                    let t = if span.abs() > 1e-6 {
                        (angle - start_angle) / span
                    } else {
                        0.0
                    };
                    along(&stops, t, extend)
                })
            }
        };
        let clip = self.clips.last();
        let Some(layer) = self.layers.last_mut() else {
            return;
        };
        for (i, px) in layer.iter_mut().enumerate() {
            let cover = clip.map_or(1.0, |c| c.a[i]);
            if cover <= 0.0 {
                continue;
            }
            let c = shade((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let keep = 1.0 - c[3] * cover;
            for k in 0..4 {
                px[k] = c[k] * cover + px[k] * keep;
            }
        }
    }
    fn push_layer(&mut self, _mode: CompositeMode) {
        self.layers.push(vec![[0.0; 4]; self.w * self.h]);
    }
    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if self.layers.len() < 2 {
            return;
        }
        let Some(src) = self.layers.pop() else { return };
        let Some(dst) = self.layers.last_mut() else {
            return;
        };
        for (d, s) in dst.iter_mut().zip(&src) {
            *d = composite(*s, *d, mode);
        }
    }
}

/// `s` composited onto `d` (both premultiplied) by a COLR composite mode.
fn composite(s: [f32; 4], d: [f32; 4], mode: CompositeMode) -> [f32; 4] {
    let (sa, da) = (s[3], d[3]);
    let porter = |fs: f32, fd: f32| [0, 1, 2, 3].map(|k| s[k] * fs + d[k] * fd);
    let blend = |f: fn(f32, f32) -> f32| {
        let mut out = [0.0; 4];
        for k in 0..3 {
            let (cs, cd) = (
                if sa > 0.0 { s[k] / sa } else { 0.0 },
                if da > 0.0 { d[k] / da } else { 0.0 },
            );
            out[k] = s[k] * (1.0 - da) + d[k] * (1.0 - sa) + sa * da * f(cs, cd);
        }
        out[3] = sa + da - sa * da;
        out
    };
    match mode {
        CompositeMode::Clear => [0.0; 4],
        CompositeMode::Src => s,
        CompositeMode::Dest => d,
        CompositeMode::DestOver => porter(1.0 - da, 1.0),
        CompositeMode::SrcIn => porter(da, 0.0),
        CompositeMode::DestIn => porter(0.0, sa),
        CompositeMode::SrcOut => porter(1.0 - da, 0.0),
        CompositeMode::DestOut => porter(0.0, 1.0 - sa),
        CompositeMode::SrcAtop => porter(da, 1.0 - sa),
        CompositeMode::DestAtop => porter(1.0 - da, sa),
        CompositeMode::Xor => porter(1.0 - da, 1.0 - sa),
        CompositeMode::Plus => [0, 1, 2, 3].map(|k| (s[k] + d[k]).min(1.0)),
        CompositeMode::Multiply => blend(|s, d| s * d),
        CompositeMode::Screen => blend(|s, d| s + d - s * d),
        CompositeMode::Overlay => blend(|s, d| {
            if d <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }),
        CompositeMode::Darken => blend(f32::min),
        CompositeMode::Lighten => blend(f32::max),
        CompositeMode::ColorDodge => blend(|s, d| {
            if d <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (d / (1.0 - s)).min(1.0)
            }
        }),
        CompositeMode::ColorBurn => blend(|s, d| {
            if d >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - d) / s).min(1.0)
            }
        }),
        CompositeMode::HardLight => blend(|s, d| {
            if s <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }),
        CompositeMode::SoftLight => blend(|s, d| {
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                let g = if d <= 0.25 {
                    ((16.0 * d - 12.0) * d + 4.0) * d
                } else {
                    d.sqrt()
                };
                d + (2.0 * s - 1.0) * (g - d)
            }
        }),
        CompositeMode::Difference => blend(|s, d| (s - d).abs()),
        CompositeMode::Exclusion => blend(|s, d| s + d - 2.0 * s * d),
        // The hue, saturation, colour and luminosity modes, and any unknown one, draw over.
        _ => porter(1.0, 1.0 - sa),
    }
}

/// A decoded bitmap and copies of it halved, each half the size of the one before.
struct Pyramid {
    levels: Vec<ColorImage>,
}

/// The decoded bitmaps of glyphs, by face, glyph and strike.
fn decoded(
    face: FaceId,
    gid: u32,
    ppem: u32,
    decode: impl FnOnce() -> Option<ColorImage>,
) -> Option<Arc<Pyramid>> {
    type Cache = Mutex<(usize, HashMap<(FaceId, u32, u32), Option<Arc<Pyramid>>>)>;
    static C: OnceLock<Cache> = OnceLock::new();
    const MAX_BYTES: usize = 64 << 20;
    let c = C.get_or_init(Default::default);
    if let Some(v) = c
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .1
        .get(&(face, gid, ppem))
    {
        return v.clone();
    }
    let v = decode().map(|image| {
        let mut levels = vec![image];
        while let Some(last) = levels.last().filter(|l| l.w > 1 || l.h > 1) {
            let half = halve(last);
            levels.push(half);
        }
        Arc::new(Pyramid { levels })
    });
    let bytes = v
        .as_ref()
        .map_or(0, |p| p.levels.iter().map(|l| l.px.len() * 16).sum());
    let mut m = c.lock().unwrap_or_else(|e| e.into_inner());
    if m.0 + bytes > MAX_BYTES || m.1.len() > 4096 {
        *m = (0, HashMap::new());
    }
    m.0 += bytes;
    m.1.insert((face, gid, ppem), v.clone());
    v
}

fn halve(src: &ColorImage) -> ColorImage {
    let (w, h) = (src.w.div_ceil(2).max(1), src.h.div_ceil(2).max(1));
    let mut out = ColorImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((2 * x + dx).min(src.w - 1), (2 * y + dy).min(src.h - 1));
                let p = src.px[sy * src.w + sx];
                for k in 0..4 {
                    sum[k] += p[k] * 0.25;
                }
            }
            out.px[y * w + x] = sum;
        }
    }
    out
}

/// A PNG as a premultiplied picture; None for one that does not decode, or is over 4096 pixels on
/// a side.
fn decode_png(data: &[u8]) -> Option<ColorImage> {
    let mut decoder =
        png::Decoder::new_with_limits(std::io::Cursor::new(data), png::Limits { bytes: 64 << 20 });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (w, h) = (reader.info().width as usize, reader.info().height as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let channels = frame.color_type.samples();
    let stride = frame.line_size;
    let mut out = ColorImage::new(w, h);
    for y in 0..h {
        let row = buf.get(y * stride..y * stride + w * channels)?;
        for x in 0..w {
            let p = &row[x * channels..x * channels + channels];
            let v = |i: usize| f32::from(p[i]) / 255.0;
            let (r, g, b, a) = match channels {
                1 => (v(0), v(0), v(0), 1.0),
                2 => (v(0), v(0), v(0), v(1)),
                3 => (v(0), v(1), v(2), 1.0),
                _ => (v(0), v(1), v(2), v(3)),
            };
            out.px[y * w + x] = [r * a, g * a, b * a, a];
        }
    }
    Some(out)
}

/// A bitmap glyph resampled under `m`.
fn bitmap(
    g: &Glyph,
    font: &skrifa::FontRef,
    m: &Xform,
    clip: [i32; 4],
    foreground: [f32; 4],
) -> Option<(ColorImage, i32, i32)> {
    let strikes = BitmapStrikes::new(font);
    let ppem = (g.size * xform_scale(m)).max(1.0);
    let glyph = strikes.glyph_for_size(Size::new(ppem), GlyphId::new(g.id))?;
    let (bw, bh) = (glyph.width as usize, glyph.height as usize);
    if bw == 0 || bh == 0 || glyph.ppem_x <= 0.0 || glyph.ppem_y <= 0.0 {
        return None;
    }
    let pyramid = decoded(g.face, g.id, glyph.ppem_y.to_bits(), || match &glyph.data {
        BitmapData::Png(data) => decode_png(data),
        BitmapData::Bgra(data) => {
            let mut out = ColorImage::new(bw, bh);
            for (i, p) in data.as_chunks::<4>().0.iter().take(bw * bh).enumerate() {
                out.px[i] = [p[2], p[1], p[0], p[3]].map(|v| f32::from(v) / 255.0);
            }
            Some(out)
        }
        BitmapData::Mask(mask) => {
            let mut alpha = vec![0u8; bw * bh];
            mask.decode_to_slice(glyph.width, glyph.height, &mut alpha)
                .ok()?;
            let mut out = ColorImage::new(bw, bh);
            for (px, a) in out.px.iter_mut().zip(alpha) {
                let a = f32::from(a) / 255.0;
                *px = [0.0, 1.0, 2.0, 3.0].map(|_| a);
            }
            Some(out)
        }
    })?;
    let base = pyramid.levels.first()?;
    let mono = matches!(glyph.data, BitmapData::Mask(_));
    // Bitmap pixels → glyph-local pixels (y down from the baseline).
    let (sx, sy) = (g.size / glyph.ppem_x, g.size / glyph.ppem_y);
    let upem = fonts::face(g.face).units_per_em();
    let (x0, y0) = match glyph.placement_origin {
        Origin::TopLeft => (
            glyph.bearing_x * g.size / upem + glyph.inner_bearing_x * sx,
            -(glyph.bearing_y * g.size / upem + glyph.inner_bearing_y * sy),
        ),
        Origin::BottomLeft => {
            let x = glyph.bearing_x * g.size / upem + glyph.inner_bearing_x * sx;
            let bottom = -(glyph.bearing_y * g.size / upem + glyph.inner_bearing_y * sy);
            (x, bottom - base.h as f32 * sy)
        }
    };
    let to_device = compose(m, &[sx, 0.0, 0.0, sy, x0, y0]);
    let corners = [
        (0.0, 0.0),
        (base.w as f32, 0.0),
        (0.0, base.h as f32),
        (base.w as f32, base.h as f32),
    ]
    .map(|(x, y)| apply(&to_device, x, y));
    let area = corners
        .iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, p| {
            [a[0].min(p.0), a[1].min(p.1), a[2].max(p.0), a[3].max(p.1)]
        });
    let (x, y, w, h) = region(area, clip)?;
    let inverse = invert(&to_device)?;
    // The level whose pixels are at least half a device pixel across.
    let mut scale = xform_scale(&to_device);
    let mut level = 0;
    while scale < 0.5 && level + 1 < pyramid.levels.len() {
        scale *= 2.0;
        level += 1;
    }
    let src = &pyramid.levels[level];
    let shrink = (base.w as f32 / src.w as f32, base.h as f32 / src.h as f32);
    // Inside, the nearest pixels blend; at the bitmap's edge, the pixel takes what of it is inside.
    let at = |ix: i64, iy: i64| -> [f32; 4] {
        src.px[iy.clamp(0, src.h as i64 - 1) as usize * src.w
            + ix.clamp(0, src.w as i64 - 1) as usize]
    };
    let device_per_pixel = xform_scale(&to_device);
    let mut out = ColorImage::new(w, h);
    for py in 0..h {
        for px in 0..w {
            let (u, v) = apply(
                &inverse,
                (x + px as i32) as f32 + 0.5,
                (y + py as i32) as f32 + 0.5,
            );
            let inside =
                u.min(base.w as f32 - u).min(v).min(base.h as f32 - v) * device_per_pixel + 0.5;
            if inside <= 0.0 {
                continue;
            }
            let (u, v) = (u / shrink.0 - 0.5, v / shrink.1 - 0.5);
            let (fu, fv) = (u.floor(), v.floor());
            let (tu, tv) = (u - fu, v - fv);
            let (iu, iv) = (fu as i64, fv as i64);
            let (a, b, c, d) = (
                at(iu, iv),
                at(iu + 1, iv),
                at(iu, iv + 1),
                at(iu + 1, iv + 1),
            );
            let mut p = [0.0; 4];
            let edge = inside.min(1.0);
            for k in 0..4 {
                p[k] = ((a[k] * (1.0 - tu) + b[k] * tu) * (1.0 - tv)
                    + (c[k] * (1.0 - tu) + d[k] * tu) * tv)
                    * edge;
            }
            if mono {
                let a = p[3] * foreground[3];
                p = [foreground[0] * a, foreground[1] * a, foreground[2] * a, a];
            }
            out.px[py * w + px] = p;
        }
    }
    Some((out, x, y))
}
