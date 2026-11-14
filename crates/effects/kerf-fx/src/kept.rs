//! The EffectCraft effects Kerf keeps beside FilmCraft's, which FilmCraft
//! has nothing like, run by Kerf's own renderer on its pictures: restoration
//! (grain, dust, rolling shutter, lens, upscaling), keying and matte
//! clean-up, curves and finer color tools, a few blurs and distortions, and
//! the audio visualizers.
//!
//! Kerf's pictures are premultiplied linear light; EffectCraft's effects
//! run on premultiplied sRGB-encoded floats, so a picture is encoded for
//! them and decoded after.

use crate::{spec, value_at};
use kerf_draw::color::{to_encoded, to_linear};
use kerf_draw::effect::EffectHost as Host;
use kerf_model::{Effect, Mask, Param, Time, Value};
use kerf_raster_fx::{
    Buf, EffectCtx, EffectEnv, EffectHost, Image, LayerPixels, ParamUi, Params, Value as FxValue,
};

/// The kept effects, by Kerf id.
pub const KEPT: &[&str] = &[
    "blur.bilateral",
    "blur.radialzoom",
    "color.autocolor",
    "color.autocontrast",
    "color.autolevels",
    "color.curves",
    "color.huesaturation",
    "color.photofilter",
    "color.selectivecolor",
    "color.shadowhighlight",
    "distort.opticscompensation",
    "distort.rollingshutterrepair",
    "distort.upscale",
    "generate.audiospectrum",
    "generate.audiowaveform",
    "key.advancedspill",
    "key.keycleaner",
    "key.unmult",
    "matte.simplechoker",
    "noise.addgrain",
    "noise.dustscratches",
    "noise.removegrain",
];

/// Whether Kerf's renderer runs the raster effect `id`.
pub fn kept(id: &str) -> bool {
    KEPT.contains(&id)
}

/// How a kept effect runs on a clip's picture.
pub struct Run<'a> {
    /// Clip time.
    pub at: Time,
    /// Sequence time, in seconds.
    pub sequence_time: f64,
    pub frame_rate: f64,
    /// The effect's place among the clip's effects.
    pub index: usize,
    /// Makes random effects differ between clips and stay the same between
    /// renders.
    pub seed: u32,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f64,
    /// The clip's masks, which effects read as paths.
    pub masks: &'a [Mask],
    /// The sequence's sound tracks, by id.
    pub sounds: &'a [String],
    /// The clip's other frames and the sequence's sound.
    pub host: Option<&'a dyn Host>,
}

/// Radial Zoom Blur as a kernel, with its parameters read as [`apply`] reads them.
pub fn radial_zoom_kernel(
    effect: &Effect,
    at: Time,
    size: (usize, usize),
    per_source: f64,
) -> Option<crate::filmcraft::vfx::gpu::Kernel> {
    use crate::filmcraft::vfx::gpu::Kernel;
    let spec = spec("blur.radialzoom")?;
    let per_source = per_source.max(1e-6);
    let layer_size = [size.0 as f64 / per_source, size.1 as f64 / per_source];
    let mut values = Params::default();
    for param in &spec.params {
        values.values.insert(
            param.id.to_owned(),
            value_at(param, &effect.params, at, layer_size),
        );
    }
    let amount = values.f("amount");
    if amount == 0.0 {
        return Some(Kernel::Nothing);
    }
    let center = values.v2("center");
    Some(Kernel::RadialZoom {
        kind: values.e("type"),
        center: [
            (center[0] * per_source) as f32,
            (center[1] * per_source) as f32,
        ],
        zoom: ((amount / 100.0).clamp(-1.0, 1.0) * 0.5) as f32,
        arc: (amount.to_radians() * 0.25) as f32,
        quality: values.f("quality").clamp(1.0, 100.0) as f32,
    })
}

/// Optics Compensation as a kernel, with its parameters read as [`apply`] reads them, or `None`
/// for what stays on the CPU: a picture that grows to keep its stretched corners (`resize`).
pub fn optics_kernel(
    effect: &Effect,
    at: Time,
    size: (usize, usize),
    per_source: f64,
) -> Option<crate::filmcraft::vfx::gpu::Kernel> {
    use crate::filmcraft::vfx::gpu::Kernel;
    use std::f64::consts::FRAC_PI_2;
    let spec = spec("distort.opticscompensation")?;
    let per_source = per_source.max(1e-6);
    let layer_size = [size.0 as f64 / per_source, size.1 as f64 / per_source];
    let mut values = Params::default();
    for param in &spec.params {
        values.values.insert(
            param.id.to_owned(),
            value_at(param, &effect.params, at, layer_size),
        );
    }
    let fov = values.f("fieldOfView").clamp(0.0, 179.0).to_radians();
    if fov < 1e-6 {
        return Some(Kernel::Nothing);
    }
    let reverse = values.b("reverseLensDistortion");
    let optimal = values.b("optimalPixels");
    if reverse && !optimal && values.e("resize") > 0 {
        return None;
    }
    let (lw, lh) = (size.0 as f64, size.1 as f64);
    let reach = match values.e("fovOrientation") {
        1 => lh * 0.5,
        2 => lw.hypot(lh) * 0.5,
        _ => lw * 0.5,
    }
    .max(1.0);
    let half = fov * 0.5;
    let focal = reach / half.tan();
    let center = values.v2("viewCenter").map(|v| v * per_source);
    let fit = if optimal {
        let far = [(0.0, 0.0), (lw, 0.0), (0.0, lh), (lw, lh)]
            .iter()
            .map(|p| (p.0 - center[0]).hypot(p.1 - center[1]))
            .fold(1.0, f64::max);
        if reverse {
            let th = (far / reach * half).min(FRAC_PI_2 - 1e-3);
            (focal * th.tan() / far).max(1.0)
        } else {
            ((far / focal).atan() / half * reach / far).min(1.0)
        }
    } else {
        1.0
    };
    Some(Kernel::Optics {
        center: [center[0] as f32, center[1] as f32],
        fit: fit as f32,
        focal: focal as f32,
        reach: reach as f32,
        half: half as f32,
        reverse,
    })
}

/// Add Grain as a kernel, with its parameters read as [`apply`] reads them, or `None` for what stays
/// on the CPU: a Softness, whose blur of each plane costs what it costs there, and a view of its
/// matte or a preview of its region.
pub fn grain_kernel(
    effect: &Effect,
    at: Time,
    size: (usize, usize),
    per_source: f64,
    seed: u32,
) -> Option<crate::filmcraft::vfx::gpu::Kernel> {
    use crate::filmcraft::vfx::gpu::{grain_gains, Grain, Kernel};
    let spec = spec("noise.addgrain")?;
    let per_source = per_source.max(1e-6);
    let layer_size = [size.0 as f64 / per_source, size.1 as f64 / per_source];
    let mut values = Params::default();
    for param in &spec.params {
        values.values.insert(
            param.id.to_owned(),
            value_at(param, &effect.params, at, layer_size),
        );
    }
    let ctx = EffectCtx {
        params: &values,
        time: at.seconds(),
        layer_size,
        seed,
        adjustment: false,
        env: EffectEnv::default(),
    };
    let look = kerf_raster_fx::GrainLook::from_params(&ctx);
    if values.e("viewingMode") != 2 || look.softness * per_source > 0.05 {
        return None;
    }
    if look.intensity <= 0.0 {
        return Some(Kernel::Nothing);
    }
    let size = look.size * per_source as f32;
    let sizes = [0, 1, 2].map(|k| {
        let by = if look.mono {
            1.0
        } else {
            look.channel_size[k].max(0.05)
        };
        (size * by).max(0.05)
    });
    Some(Kernel::Grain(Box::new(Grain {
        mode: look.mode,
        mono: look.mono,
        seeds: [0u32, 1, 2].map(|k| look.seed.wrapping_add(k * 101)),
        gains: grain_gains(&look),
        aspect: look.aspect,
        sizes,
        frame: look.frame,
        tones: look.tones,
        midpoint: look.midpoint,
        saturation: look.saturation,
    })))
}

/// One of the kept color effects as a kernel (Curves, Hue/Saturation, Photo Filter, Selective Color,
/// Unmult), with its parameters read as [`apply`] reads them, or `None` for what stays on the CPU: any
/// other effect, and Curves with a point outside 0 to 1, where a curve is no line past its ends.
pub fn color_kernel(
    effect: &Effect,
    at: Time,
    size: (usize, usize),
    per_source: f64,
) -> Option<crate::filmcraft::vfx::gpu::Kernel> {
    use crate::filmcraft::vfx::gpu::{curves_table, KeptColor, Kernel};
    let id = effect.fx.as_str();
    let spec = spec(id)?;
    let per_source = per_source.max(1e-6);
    let layer_size = [size.0 as f64 / per_source, size.1 as f64 / per_source];
    let mut values = Params::default();
    for param in &spec.params {
        values.values.insert(
            param.id.to_owned(),
            value_at(param, &effect.params, at, layer_size),
        );
    }
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    let kept = match id {
        "color.photofilter" => {
            let filter = values.e("filter");
            let color = match kerf_raster_fx::PHOTO_FILTERS.get(filter as usize) {
                Some((_, rgb)) if filter != kerf_raster_fx::PHOTO_FILTER_CUSTOM => {
                    rgb.map(|value| f32::from(value) / 255.0)
                }
                _ => {
                    let color = values.color("color");
                    [color[0], color[1], color[2]]
                }
            };
            KeptColor::PhotoFilter(kerf_kernel::PhotoFilter {
                color,
                density: values.f("density") as f32 / 100.0,
                keep: flag(values.b("preserveLuminosity")),
            })
        }
        "key.unmult" => {
            let white = values.e("backgroundColor") == 1;
            let level = if white {
                values.f("whiteLevel")
            } else {
                values.f("blackLevel")
            };
            let level = level.clamp(0.0, 100.0) as f32 / 100.0;
            let soft = values.f("softness").clamp(0.0, 100.0) as f32 / 100.0;
            let start = if white { 1.0 - level } else { level };
            KeptColor::Unmult(kerf_kernel::Unmult {
                white: flag(white),
                start,
                width: soft * (1.0 - start),
                decontaminate: flag(values.b("removeColorMatting")),
                clip: flag(values.b("clipHdrResults")),
            })
        }
        "color.curves" => {
            let curves = ["rgb", "red", "green", "blue", "alpha"]
                .map(|id| kerf_raster_fx::Curve::parse(values.s(id)));
            let curved = (0u32..)
                .zip(&curves)
                .map(|(which, curve)| u32::from(curve.is_some()) << which)
                .sum();
            match curved {
                0 => KeptColor::Through,
                _ => KeptColor::Curves {
                    curved,
                    table: curves_table(&curves)?,
                },
            }
        }
        "color.huesaturation" => {
            let ctx = EffectCtx {
                params: &values,
                time: at.seconds(),
                layer_size,
                seed: 0,
                adjustment: false,
                env: EffectEnv::default(),
            };
            let turn = |id: &str| values.f(id) as f32 / 360.0;
            let share = |id: &str| values.f(id) as f32 / 100.0;
            KeptColor::HueSaturation {
                master: [turn("hue"), share("saturation"), share("lightness")],
                colorize: values.b("colorize").then(|| {
                    [
                        turn("colorizeHue"),
                        share("colorizeSaturation"),
                        share("colorizeLightness"),
                    ]
                }),
                ranges: kerf_raster_fx::huesat_ranges(&ctx),
            }
        }
        "color.selectivecolor" => {
            let take: [[f32; 4]; 9] = std::array::from_fn(|range| {
                kerf_raster_fx::SC_IDS[range].map(|leaf| {
                    values.f(&kerf_raster_fx::sc_param_id(range, leaf)) as f32 / 100.0
                })
            });
            if take.iter().flatten().all(|value| *value == 0.0) {
                KeptColor::Through
            } else {
                KeptColor::SelectiveColor {
                    take,
                    relative: values.e("method") == 0,
                }
            }
        }
        // Its other method finds the key by adding up the whole picture, which a kernel cannot.
        "key.advancedspill" if values.e("method") == 1 => {
            let key = values.color("ultraSettings/keyColor");
            let primary = if key[1] >= key[0] && key[1] >= key[2] {
                1
            } else if key[2] >= key[0] && key[2] >= key[1] {
                2
            } else {
                0
            };
            let share = |id: &str| values.f(id) as f32 / 100.0;
            KeptColor::Spill {
                primary,
                amount: share("suppression"),
                range: share("ultraSettings/spillRange"),
                luma: share("ultraSettings/lumaCorrection"),
                key_hue: kerf_color::rgb_to_hsl(key[0], key[1], key[2])[0],
                tolerance: values
                    .get("ultraSettings/tolerance")
                    .map_or(100.0, FxValue::as_f64) as f32
                    / 100.0,
                desaturate: share("ultraSettings/desaturate"),
                neutral: share("ultraSettings/spillColorCorrection"),
            }
        }
        _ => return None,
    };
    Some(Kernel::Kept(Box::new(kept)))
}

/// Run the kept effect `effect` on `picture`, premultiplied linear light.
pub fn apply(picture: &mut kerf_draw::Image, effect: &Effect, run: &Run) {
    let Some(spec) = spec(effect.fx.as_str()).filter(|_| kept(effect.fx.as_str())) else {
        return;
    };
    let per_source = run.per_source.max(1e-6);
    // The layer is the picture in source pixels.
    let layer_size = [picture.w as f64 / per_source, picture.h as f64 / per_source];
    let masks: Vec<kerf_raster_fx::MaskShape> = run
        .masks
        .iter()
        .map(|mask| kerf_raster_fx::MaskShape {
            name: mask.name.to_string(),
            points: kerf_draw::mask::outline(
                &mask.shape.at(run.at),
                layer_size[0],
                layer_size[1],
                1.0,
            )
            .into_iter()
            .map(|[x, y]| [f64::from(x), f64::from(y)])
            .collect(),
            closed: mask.shape.is_closed(),
            inverted: mask.inverted,
        })
        .collect();
    let mut values = Params::default();
    for param in &spec.params {
        let value = match param.ui {
            // A mask by name, from 1; 0 for none.
            ParamUi::Mask => FxValue::Scalar(
                text(effect.params.get(param.id))
                    .and_then(|name| run.masks.iter().position(|mask| mask.name.as_str() == name))
                    .map_or(0.0, |index| (index + 1) as f64),
            ),
            // A sound track by id, from 1; another picture track reads as
            // none, as no kept effect reads one.
            ParamUi::Layer => FxValue::Layer(
                text(effect.params.get(param.id))
                    .and_then(|track| run.sounds.iter().position(|id| *id == track))
                    .map(|index| index as u64 + 1),
            ),
            _ => value_at(param, &effect.params, run.at, layer_size),
        };
        values.values.insert(param.id.to_owned(), value);
    }
    let host = run.host.map(|host| Adapter {
        host,
        sounds: run.sounds,
        per_source,
    });
    let ctx = EffectCtx {
        params: &values,
        time: run.at.seconds(),
        layer_size,
        seed: run.seed,
        adjustment: false,
        env: EffectEnv {
            masks: &masks,
            host: host.as_ref().map(|host| host as &dyn EffectHost),
            comp_time: run.sequence_time,
            frame_rate: run.frame_rate,
            effect_index: run.index,
            ..EffectEnv::default()
        },
    };
    let buf = Buf {
        img: encoded(picture),
        offset: [0.0; 2],
        scale: per_source,
    };
    let buf = kerf_raster_fx::apply(spec, &ctx, buf);
    decode_into(&buf, picture, per_source);
}

fn text(param: Option<&Param>) -> Option<String> {
    match param? {
        Param::Static(Value::Text(text) | Value::Enum(text)) if !text.is_empty() => {
            Some(text.clone())
        }
        _ => None,
    }
}

/// Kerf's picture as EffectCraft's: premultiplied, sRGB-encoded.
fn encoded(picture: &kerf_draw::Image) -> Image {
    let mut image = Image::new(picture.w as u32, picture.h as u32);
    for (pixel, linear) in image.data.iter_mut().zip(picture.px.as_chunks::<4>().0.iter()) {
        let alpha = linear[3];
        *pixel = if alpha <= 1e-6 {
            [0.0; 4]
        } else {
            let channel = |c: f32| to_encoded((c / alpha).max(0.0)) * alpha;
            [
                channel(linear[0]),
                channel(linear[1]),
                channel(linear[2]),
                alpha,
            ]
        };
    }
    image
}

/// EffectCraft's buffer back over the picture it came from. An effect may
/// grow the buffer or move its origin; the picture keeps its rectangle.
fn decode_into(buf: &Buf, picture: &mut kerf_draw::Image, per_source: f64) {
    let ratio = buf.scale / per_source;
    let (bw, bh) = (i64::from(buf.img.width), i64::from(buf.img.height));
    let width = picture.w;
    for (index, out) in picture.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((index % width) as f64, (index / width) as f64);
        let column = (x * ratio + buf.offset[0]).round() as i64;
        let row = (y * ratio + buf.offset[1]).round() as i64;
        let pixel = if (0..bw).contains(&column) && (0..bh).contains(&row) {
            buf.img.data[(row * bw + column) as usize]
        } else {
            [0.0; 4]
        };
        let alpha = pixel[3].clamp(0.0, 1.0);
        let channel = |c: f32| {
            if alpha <= 1e-6 {
                0.0
            } else {
                to_linear((c / alpha).max(0.0)) * alpha
            }
        };
        out.copy_from_slice(&[
            channel(pixel[0]),
            channel(pixel[1]),
            channel(pixel[2]),
            alpha,
        ]);
    }
}

/// Kerf's host as EffectCraft's.
struct Adapter<'a> {
    host: &'a dyn Host,
    sounds: &'a [String],
    per_source: f64,
}

impl EffectHost for Adapter<'_> {
    fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
        None
    }

    fn audio(&self, id: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
        let track = self.sounds.get(usize::try_from(id).ok()?.checked_sub(1)?)?;
        self.host.sound(track, start, frames, rate)
    }

    fn self_at(&self, layer_time: f64, effects: usize) -> Option<Buf> {
        let picture = self
            .host
            .picture_at(Time::from_seconds(layer_time)?, effects)?;
        Some(Buf {
            img: encoded(&picture),
            offset: [0.0; 2],
            scale: self.per_source,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_optics_kernel_matches_the_effect() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let (w, h) = (96usize, 54usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let bright = (x / 9 + y / 7) % 3 == 0;
                let a = if y % 11 == 0 { 0.5 } else { 1.0 };
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let c = if bright { [1.0, 0.9, 0.7] } else { [u, v, 1.0 - u] };
                px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
            }
        }
        let image = kerf_draw::Image { w, h, px };
        let number = |value: f64| kerf_model::Param::Static(Value::Number(value));
        let flag = |on: bool| kerf_model::Param::Static(Value::Bool(on));
        let choice = |name: &str| kerf_model::Param::Static(Value::Enum(name.into()));
        // A lens's curve put in, taken out, measured each way, off its center, and fitted.
        let cases: Vec<Vec<(&str, kerf_model::Param)>> = vec![
            vec![("fieldOfView", number(70.0))],
            vec![("fieldOfView", number(90.0)), ("reverseLensDistortion", flag(true))],
            vec![("fieldOfView", number(120.0)), ("fovOrientation", choice("vertical"))],
            vec![
                ("fieldOfView", number(60.0)),
                ("fovOrientation", choice("diagonal")),
                ("viewCenter_x", number(0.3)),
                ("viewCenter_y", number(0.6)),
            ],
            vec![("fieldOfView", number(100.0)), ("optimalPixels", flag(true))],
            vec![
                ("fieldOfView", number(80.0)),
                ("reverseLensDistortion", flag(true)),
                ("optimalPixels", flag(true)),
            ],
        ];
        for params in cases {
            let mut effect = Effect {
                fx: kerf_model::Id::new("distort.opticscompensation").unwrap(),
                version: 1,
                enabled: true,
                params: Default::default(),
            };
            let said = format!("{:?}", params.iter().map(|(name, _)| *name).collect::<Vec<_>>());
            for (name, value) in params {
                effect.params.insert(name.into(), value);
            }
            let mut cpu = image.clone();
            apply(
                &mut cpu,
                &effect,
                &Run {
                    at: Time::ZERO,
                    sequence_time: 0.0,
                    frame_rate: 24.0,
                    index: 0,
                    seed: 1,
                    per_source: 1.0,
                    masks: &[],
                    sounds: &[],
                    host: None,
                },
            );
            let kernel = optics_kernel(&effect, Time::ZERO, (w, h), 1.0).expect("a kernel");
            let mut frame = gpu.frame();
            let picture = frame.image(&image);
            let out = kernel.run(&mut frame, picture).expect("the stage builds");
            let drawn = frame.read_floats(&out).expect("read back");
            let worst = cpu
                .px
                .iter()
                .zip(&drawn)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 4e-3, "{said}: differs by {worst:.2e}");
            assert_ne!(cpu.px, image.px, "{said} changes nothing");
        }
        // Nothing asked is nothing done, and a picture that grows is the CPU's.
        let mut effect = Effect {
            fx: kerf_model::Id::new("distort.opticscompensation").unwrap(),
            version: 1,
            enabled: true,
            params: Default::default(),
        };
        assert!(matches!(
            optics_kernel(&effect, Time::ZERO, (w, h), 1.0),
            Some(crate::filmcraft::vfx::gpu::Kernel::Nothing)
        ));
        effect.params.insert("fieldOfView".into(), number(60.0));
        effect.params.insert("reverseLensDistortion".into(), flag(true));
        effect.params.insert("resize".into(), choice("max-2x"));
        assert!(optics_kernel(&effect, Time::ZERO, (w, h), 1.0).is_none());
    }

    #[test]
    fn the_radial_zoom_kernel_matches_the_effect() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let (w, h) = (96usize, 54usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let bright = (x / 9 + y / 7) % 3 == 0;
                let a = if y % 11 == 0 { 0.5 } else { 1.0 };
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let c = if bright {
                    [1.0, 0.9, 0.7]
                } else {
                    [u, v, 1.0 - u]
                };
                px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
            }
        }
        let image = kerf_draw::Image { w, h, px };
        let number = |value: f64| kerf_model::Param::Static(Value::Number(value));
        for (kind, amount, center) in [
            ("straight-zoom", 40.0, (0.5, 0.5)),
            ("fading-zoom", -30.0, (0.2, 0.7)),
            ("centered", 60.0, (0.5, 0.2)),
            ("rotate", 25.0, (0.5, 0.5)),
            ("rotate-fading", -40.0, (0.8, 0.4)),
        ] {
            let mut effect = Effect {
                fx: kerf_model::Id::new("blur.radialzoom").unwrap(),
                version: 1,
                enabled: true,
                params: Default::default(),
            };
            effect.params.insert(
                "type".into(),
                kerf_model::Param::Static(Value::Enum(kind.into())),
            );
            effect.params.insert("amount".into(), number(amount));
            effect.params.insert("center_x".into(), number(center.0));
            effect.params.insert("center_y".into(), number(center.1));
            let mut cpu = image.clone();
            apply(
                &mut cpu,
                &effect,
                &Run {
                    at: Time::ZERO,
                    sequence_time: 0.0,
                    frame_rate: 24.0,
                    index: 0,
                    seed: 1,
                    per_source: 1.0,
                    masks: &[],
                    sounds: &[],
                    host: None,
                },
            );
            let kernel = radial_zoom_kernel(&effect, Time::ZERO, (w, h), 1.0).expect("a kernel");
            let mut frame = gpu.frame();
            let picture = frame.image(&image);
            let out = kernel.run(&mut frame, picture).expect("the stage builds");
            let drawn = frame.read_floats(&out).expect("read back");
            let worst = cpu
                .px
                .iter()
                .zip(&drawn)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 4e-3, "{kind}: differs by {worst:.2e}");
            assert_ne!(cpu.px, image.px, "{kind} changes nothing");
        }
    }

    /// Add Grain, its kernel against the effect, in each blending mode and each way it colors.
    #[test]
    fn the_grain_kernel_matches_the_effect() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let (w, h) = (128usize, 72usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let a = if y % 11 == 0 { 0.5 } else { 1.0 };
                let a = if y % 13 == 0 { 0.0 } else { a };
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                px.extend_from_slice(&[u * a, v * a, (1.0 - u) * a, a]);
            }
        }
        let image = kerf_draw::Image { w, h, px };
        let number = |name: &str, value: f64| {
            (
                name.to_owned(),
                kerf_model::Param::Static(Value::Number(value)),
            )
        };
        let choice = |name: &str, value: &str| {
            (
                name.to_owned(),
                kerf_model::Param::Static(Value::Enum(value.into())),
            )
        };
        let flag = |name: &str, value: bool| {
            (
                name.to_owned(),
                kerf_model::Param::Static(Value::Bool(value)),
            )
        };
        let cases = [
            vec![number("tweaking/intensity", 1.0)],
            vec![
                number("tweaking/intensity", 3.0),
                choice("application/blendingMode", "multiply"),
            ],
            vec![
                number("tweaking/intensity", 2.0),
                choice("application/blendingMode", "add"),
                number("tweaking/size", 2.5),
            ],
            vec![
                number("tweaking/intensity", 2.0),
                choice("application/blendingMode", "screen"),
                number("tweaking/aspectRatio", 2.0),
            ],
            vec![
                number("tweaking/intensity", 2.0),
                choice("application/blendingMode", "overlay"),
                flag("color/monochromatic", true),
            ],
            vec![
                number("tweaking/intensity", 4.0),
                number("color/saturation", 0.4),
                number("color/tintAmount", 0.6),
                number("color/redBalance", 1.5),
                number("application/highlights", 0.2),
                number("animation/randomSeed", 9.0),
                number("tweaking/channelSize/greenSize", 2.0),
                number("tweaking/channelIntensities/blueIntensity", 2.0),
            ],
        ];
        let at = Time::from_seconds(0.5).expect("a time");
        for params in cases {
            let effect = Effect {
                fx: kerf_model::Id::new("noise.addgrain").unwrap(),
                version: 1,
                enabled: true,
                params: params.into_iter().collect(),
            };
            let mut cpu = image.clone();
            apply(
                &mut cpu,
                &effect,
                &Run {
                    at,
                    sequence_time: 0.5,
                    frame_rate: 24.0,
                    index: 0,
                    seed: 11,
                    per_source: 1.0,
                    masks: &[],
                    sounds: &[],
                    host: None,
                },
            );
            assert_ne!(cpu.px, image.px, "{:?} changes nothing", effect.params);
            let kernel = grain_kernel(&effect, at, (w, h), 1.0, 11).expect("a kernel");
            let mut frame = gpu.frame();
            let picture = frame.image(&image);
            let out = kernel.run(&mut frame, picture).expect("the stage builds");
            let drawn = frame.read_floats(&out).expect("read back");
            let worst = cpu
                .px
                .iter()
                .zip(&drawn)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 2e-3, "{:?}: differs by {worst:.2e}", effect.params);
        }
        assert!(kerf_gpu::shared().is_some(), "the device failed");
    }

    /// A picture of hues, grays, the corners of the cube, and every kind of alpha.
    fn colors(w: usize, h: usize) -> kerf_draw::Image {
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
                let c = match x % 7 {
                    0 => [v, v, v],
                    1 => [f32::from(y % 2 == 0), f32::from(y % 3 == 0), f32::from(y % 5 == 0)],
                    2 => [u, u, 0.02],
                    _ => kerf_color::hsl_to_rgb(u, 0.2 + 0.8 * v, 0.1 + 0.8 * ((y % 9) as f32 / 8.0))
                        .map(kerf_color::srgb_to_linear),
                };
                let a = match y % 11 {
                    0 => 0.0,
                    1 | 2 => 0.35 + 0.5 * u,
                    _ => 1.0,
                };
                px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
            }
        }
        kerf_draw::Image { w, h, px }
    }

    /// The color effects with kernels, each way they can be set: an id and what a document says of it.
    fn color_cases() -> Vec<(&'static str, Vec<(&'static str, Value)>)> {
        let number = Value::Number;
        let choice = |word: &str| Value::Enum(word.into());
        let text = |text: &str| Value::Text(text.into());
        let color = |r, g, b| Value::Color(kerf_model::Rgba { r, g, b, a: 255 });
        vec![
            ("color.photofilter", vec![]),
            (
                "color.photofilter",
                vec![("filter", choice("cooling-filter-80")), ("density", number(80.0))],
            ),
            (
                "color.photofilter",
                vec![
                    ("filter", choice("custom")),
                    ("color", color(64, 192, 255)),
                    ("density", number(100.0)),
                    ("preserveLuminosity", Value::Bool(false)),
                ],
            ),
            ("key.unmult", vec![]),
            ("key.unmult", vec![("backgroundColor", choice("white"))]),
            (
                "key.unmult",
                vec![("blackLevel", number(20.0)), ("softness", number(40.0))],
            ),
            (
                "key.unmult",
                vec![
                    ("backgroundColor", choice("white")),
                    ("whiteLevel", number(80.0)),
                    ("softness", number(30.0)),
                    ("removeColorMatting", Value::Bool(false)),
                    ("clipHdrResults", Value::Bool(false)),
                ],
            ),
            // No curve but the line it starts as: there and back, and nothing between.
            ("color.curves", vec![]),
            ("color.curves", vec![("rgb", text("0,0 0.5,0.7 1,1"))]),
            (
                "color.curves",
                vec![
                    ("red", text("0,0.1 0.4,0.3 1,0.9")),
                    ("blue", text("0,0 0.6,0.4 1,1")),
                ],
            ),
            // What has no alpha is given some.
            ("color.curves", vec![("alpha", text("0,0.2 1,0.8"))]),
            // The master goes below 0 and above 1, where the green curve is a line past its ends.
            (
                "color.curves",
                vec![
                    ("rgb", text("0.1,0 0.9,1")),
                    ("green", text("0,0.05 0.5,0.4 1,0.95")),
                ],
            ),
            ("color.huesaturation", vec![]),
            (
                "color.huesaturation",
                vec![
                    ("hue", number(40.0)),
                    ("saturation", number(30.0)),
                    ("lightness", number(-10.0)),
                ],
            ),
            ("color.huesaturation", vec![("saturation", number(-100.0))]),
            (
                "color.huesaturation",
                vec![
                    ("redsHue", number(60.0)),
                    ("redsSaturation", number(50.0)),
                    ("bluesLightness", number(-40.0)),
                    ("greensSaturation", number(-60.0)),
                    ("greensRangeStart", number(80.0)),
                    ("greensEndFalloff", number(0.0)),
                ],
            ),
            (
                "color.huesaturation",
                vec![
                    ("colorize", Value::Bool(true)),
                    ("colorizeHue", number(200.0)),
                    ("colorizeSaturation", number(60.0)),
                    ("colorizeLightness", number(20.0)),
                ],
            ),
            // A hue more than a turn back.
            (
                "color.huesaturation",
                vec![
                    ("colorize", Value::Bool(true)),
                    ("colorizeHue", number(-460.0)),
                    ("colorizeLightness", number(-30.0)),
                ],
            ),
            // The spill suppressor told its key: green, blue with what it takes put back as
            // light, and near the key's hue alone with the color taken out.
            ("key.advancedspill", vec![("method", choice("ultra"))]),
            (
                "key.advancedspill",
                vec![
                    ("method", choice("ultra")),
                    ("ultraSettings/keyColor", Value::Color(kerf_model::Rgba::rgb(20, 40, 230))),
                    ("suppression", number(80.0)),
                    ("ultraSettings/spillRange", number(20.0)),
                    ("ultraSettings/lumaCorrection", number(60.0)),
                    ("ultraSettings/spillColorCorrection", number(40.0)),
                ],
            ),
            (
                "key.advancedspill",
                vec![
                    ("method", choice("ultra")),
                    ("ultraSettings/keyColor", Value::Color(kerf_model::Rgba::rgb(230, 30, 20))),
                    ("ultraSettings/tolerance", number(30.0)),
                    ("ultraSettings/desaturate", number(70.0)),
                ],
            ),
            ("color.selectivecolor", vec![]),
            (
                "color.selectivecolor",
                vec![
                    ("details/reds/redsCyan", number(40.0)),
                    ("details/reds/redsBlack", number(-20.0)),
                    ("details/neutrals/neutralsYellow", number(30.0)),
                ],
            ),
            (
                "color.selectivecolor",
                vec![
                    ("method", choice("absolute")),
                    ("details/blues/bluesMagenta", number(50.0)),
                    ("details/whites/whitesBlack", number(30.0)),
                    ("details/blacks/blacksCyan", number(-40.0)),
                    ("details/cyans/cyansYellow", number(25.0)),
                    ("details/yellows/yellowsMagenta", number(-35.0)),
                ],
            ),
        ]
    }

    fn set(id: &str, params: &[(&'static str, Value)]) -> Effect {
        let params: Vec<(&str, Param)> = params
            .iter()
            .map(|(name, value)| (*name, Param::Static(value.clone())))
            .collect();
        effect(id, &params)
    }

    /// Curves, Hue/Saturation, Photo Filter, Selective Color, and Unmult, each kernel against the
    /// effect. The CPU encodes a picture from a table and the kernel by the curve itself, which is
    /// most of what there is between them.
    #[test]
    fn the_color_kernels_match_the_effects() {
        let Some(gpu) = kerf_gpu::shared() else {
            eprintln!("no GPU; skipping");
            return;
        };
        let image = colors(96, 54);
        let mut failures = Vec::new();
        for (id, params) in color_cases() {
            let effect = set(id, &params);
            let mut cpu = image.clone();
            apply(&mut cpu, &effect, &run());
            let kernel =
                color_kernel(&effect, Time::ZERO, (image.w, image.h), 1.0).expect("a kernel");
            let mut frame = gpu.frame();
            let picture = frame.image(&image);
            let out = kernel.run(&mut frame, picture).expect("the stage builds");
            let drawn = frame.read_floats(&out).expect("read back");
            let worst = cpu
                .px
                .iter()
                .zip(&drawn)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            if worst > 2e-3 {
                failures.push(format!("{id} {params:?}: differs by {worst:.2e}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert!(kerf_gpu::shared().is_some(), "the device failed");
    }

    /// Photo Filter and Unmult are written once in `kerf_kernel`; as `f32`, between the CPU's own way
    /// there and back, they come to the very values the effects come to, with no GPU to say so.
    #[test]
    fn color_kernels_from_one_source_are_the_effects_to_the_bit() {
        use crate::filmcraft::vfx::gpu::{KeptColor, Kernel};
        let image = colors(96, 54);
        let number = Value::Number;
        // A hard edge too, which a kernel on a GPU may put a pixel either side of.
        let hard = [
            ("key.unmult", vec![("softness", number(0.0)), ("blackLevel", number(30.0))]),
            (
                "key.unmult",
                vec![
                    ("backgroundColor", Value::Enum("white".into())),
                    ("softness", number(0.0)),
                ],
            ),
        ];
        let mut held = 0;
        for (id, params) in color_cases().into_iter().chain(hard) {
            let effect = set(id, &params);
            let Some(Kernel::Kept(kept)) = color_kernel(&effect, Time::ZERO, (image.w, image.h), 1.0)
            else {
                panic!("{id} has no kernel");
            };
            let mut twin = encoded(&image);
            match &*kept {
                KeptColor::PhotoFilter(told) => {
                    for px in &mut twin.data {
                        *px = kerf_kernel::photo_filter(*px, told);
                    }
                }
                KeptColor::Unmult(told) => {
                    for px in &mut twin.data {
                        *px = kerf_kernel::unmult(*px, told);
                    }
                }
                // Nothing between there and back, and the rest are written by hand.
                KeptColor::Through => {}
                _ => continue,
            }
            let twin = Buf {
                img: twin,
                offset: [0.0; 2],
                scale: 1.0,
            };
            let mut drawn = image.clone();
            decode_into(&twin, &mut drawn, 1.0);
            let mut cpu = image.clone();
            apply(&mut cpu, &effect, &run());
            // (equal rather than the same bits: a zero may come out with another sign)
            assert!(drawn.px == cpu.px, "{id} {params:?}");
            held += 1;
        }
        assert_eq!(held, 3 + 4 + 2 + 2);
    }

    #[test]
    fn curves_past_0_and_1_stay_on_the_cpu() {
        let curve = |points: &str| set("color.curves", &[("rgb", Value::Text(points.into()))]);
        let kernel = |effect: &Effect| color_kernel(effect, Time::ZERO, (8, 8), 1.0);
        assert!(kernel(&curve("0,0 0.5,0.6 1,1")).is_some());
        // A curve is a line past its first and last point only when those are its ends.
        assert!(kernel(&curve("-0.2,0 0.5,0.6 1,1")).is_none());
        assert!(kernel(&curve("0,0 0.5,0.6 1.3,1")).is_none());
        assert!(kernel(&set("blur.bilateral", &[])).is_none());
    }

    #[test]
    fn a_softened_or_previewed_grain_stays_on_the_cpu() {
        let with = |name: &str, value: Value| Effect {
            fx: kerf_model::Id::new("noise.addgrain").unwrap(),
            version: 1,
            enabled: true,
            params: [(name.to_owned(), kerf_model::Param::Static(value))]
                .into_iter()
                .collect(),
        };
        let kernel = |effect: &Effect| grain_kernel(effect, Time::ZERO, (8, 8), 1.0, 1);
        assert!(kernel(&with("tweaking/softness", Value::Number(1.0))).is_none());
        assert!(kernel(&with("viewingMode", Value::Enum("preview".into()))).is_none());
        assert!(kernel(&with("tweaking/intensity", Value::Number(1.0))).is_some());
    }

    use super::*;
    use kerf_model::{Catalog, Engine, Id};

    fn effect(id: &str, params: &[(&str, Param)]) -> Effect {
        Effect {
            fx: Id::new(id).unwrap(),
            version: 1,
            enabled: true,
            params: params
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
        }
    }

    fn run<'a>() -> Run<'a> {
        Run {
            at: Time::ZERO,
            sequence_time: 0.0,
            frame_rate: 24.0,
            index: 0,
            seed: 0,
            per_source: 1.0,
            masks: &[],
            sounds: &[],
            host: None,
        }
    }

    #[test]
    fn every_kept_effect_is_a_raster_effect_that_runs() {
        for id in KEPT {
            let fx = Catalog::builtin().fx(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(fx.engine, Engine::Raster, "{id}");
            let mut picture = kerf_draw::Image::filled(24, 16, [0.2, 0.3, 0.1, 1.0]);
            apply(&mut picture, &effect(id, &[]), &run());
            assert!(picture.px.iter().all(|v| v.is_finite()), "{id}");
        }
    }

    #[test]
    fn a_kept_effect_runs_in_encoded_light() {
        // Hue/Saturation at no change leaves the picture as it was.
        let mut picture = kerf_draw::Image::filled(8, 8, [0.2, 0.05, 0.6, 1.0]);
        let before = picture.px.clone();
        apply(&mut picture, &effect("color.huesaturation", &[]), &run());
        assert!(picture
            .px
            .iter()
            .zip(&before)
            .all(|(a, b)| (a - b).abs() < 1e-3));
        // Desaturated fully, it is grey.
        let grey = effect(
            "color.huesaturation",
            &[("saturation", Param::Static(Value::Number(-100.0)))],
        );
        apply(&mut picture, &grey, &run());
        let p = &picture.px[..4];
        assert!(
            (p[0] - p[1]).abs() < 1e-3 && (p[1] - p[2]).abs() < 1e-3,
            "{p:?}"
        );
    }
}
