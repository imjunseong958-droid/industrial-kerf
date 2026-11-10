//! Kerf's effects and transitions.
//!
//! [`transitions`] are the transitions carried from FilmCraft, which Kerf's
//! own renderer draws from both clips across a cut, and [`filmcraft`] the
//! video effects carried from FilmCraft, which it runs on a clip's picture.
//! The rest of this crate is the EffectCraft effects Kerf keeps ([`kept`]),
//! which FilmCraft has nothing like.
//!
//! EffectCraft's effects run on premultiplied floating-point RGBA in layer
//! space. Kerf writes their parameters as numbers, angles, choices,
//! switches, colors, and points, which become an `_x` and a `_y` number
//! across the picture: the catalog lists them from a table this crate
//! generates (`kerf-model/src/raster_effects.tsv`). Kerf's renderer runs
//! them through [`kept`]; [`RasterStack`] runs them on a clip's 8-bit
//! source frames, after its masks cut them, as playback and the graph's
//! bake do.

use kerf_model::{Catalog, Effect, Engine, Rgba, Time, Value};
use kerf_raster_fx::{
    Buf, EffectCtx, EffectEnv, EffectSpec, Image, ParamSpec, ParamUi, Params, Value as FxValue,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

pub mod filmcraft;
mod guide;
pub mod kept;
pub mod picture;
pub mod transitions;

pub use kerf_draw::mask::cuts;

/// The version of what the EffectCraft effects (`kerf-raster-fx`, on
/// `kerf-raster`) draw, in the cache key of every baked and prerendered
/// file. Bump it with a change there that changes a picture, so files made
/// before it are made again.
pub const EFFECTS_VERSION: u32 = 1;

/// Kept effects that read the clip's other frames: they fix what changes
/// between neighbouring frames.
const READS_OTHER_FRAMES: &[&str] = &["ec.distort.rollingshutterrepair"];

/// Whether the raster effect `id` reads the clip's other frames, which only
/// the compositor gives it; on one picture alone it passes that picture
/// through.
pub fn reads_other_frames(id: &str) -> bool {
    spec(id).is_some_and(|spec| spec.category == "Time" || READS_OTHER_FRAMES.contains(&spec.id))
}

/// Layer parameters whose layer an effect only hears (the audio
/// visualizers' Audio Layer): in Kerf they name a sound track.
const SOUND_LAYERS: &[&str] = &["audioLayer"];

/// Effects named after other products: Kerf's id and name for each.
const RENAMED: &[(&str, (&str, &str))] =
    &[("ec.blur.ccradial", ("blur.radialzoom", "Radial Zoom Blur"))];

/// Kerf's id and name for an EffectCraft effect, if Kerf keeps it.
fn carried(spec: &EffectSpec) -> Option<(String, String)> {
    if !spec.params.iter().all(|p| writable(&p.ui)) {
        return None;
    }
    let (id, name) = match RENAMED.iter().find(|(id, _)| *id == spec.id) {
        Some((_, (id, name))) => ((*id).to_owned(), (*name).to_owned()),
        None => {
            let id = spec.id.strip_prefix("ec.")?;
            match spec.name.strip_prefix("CC ") {
                // Cycore's effects drop their brand: `blur.cccross` is
                // `blur.cross`, "CC Cross Blur" is "Cross Blur".
                Some(name) => {
                    let (group, effect) = id.rsplit_once('.')?;
                    let effect = effect.strip_prefix("cc").unwrap_or(effect);
                    (format!("{group}.{effect}"), name.to_owned())
                }
                None => (id.to_owned(), spec.name.to_owned()),
            }
        }
    };
    kept::kept(&id).then_some((id, name))
}

fn writable(ui: &ParamUi) -> bool {
    matches!(
        ui,
        ParamUi::Number
            | ParamUi::Slider { .. }
            | ParamUi::Angle
            | ParamUi::Popup { .. }
            | ParamUi::Checkbox
            | ParamUi::Color
            | ParamUi::Point
            | ParamUi::Text
            | ParamUi::File { .. }
            | ParamUi::Layer
            | ParamUi::Mask
            | ParamUi::Hidden
    )
}

/// The `_x` and `_y` numbers a point parameter is written as.
fn point_names(id: &str) -> [String; 2] {
    [format!("{id}_x"), format!("{id}_y")]
}

/// The EffectCraft effect behind a Kerf raster effect id.
pub fn spec(id: &str) -> Option<&'static EffectSpec> {
    static BY_ID: OnceLock<HashMap<String, &'static EffectSpec>> = OnceLock::new();
    BY_ID
        .get_or_init(|| {
            kerf_raster_fx::registry()
                .iter()
                .filter_map(|spec| carried(spec).map(|(id, ..)| (id, spec)))
                .collect()
        })
        .get(id)
        .copied()
}

/// A popup's choices as Kerf enum values: lowercase words joined by `-`,
/// unique within the popup.
fn choices(options: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(options.len());
    for (index, option) in options.iter().enumerate() {
        let mut slug = String::new();
        for word in option
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
        {
            if !slug.is_empty() {
                slug.push('-');
            }
            slug.push_str(&word.to_ascii_lowercase());
        }
        if slug.is_empty() || out.contains(&slug) {
            slug = format!(
                "{}{}",
                if slug.is_empty() { "option" } else { &slug },
                index + 1
            );
        }
        out.push(slug);
    }
    out
}

fn color(rgba: [f64; 4]) -> Rgba {
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba {
        r: channel(rgba[0]),
        g: channel(rgba[1]),
        b: channel(rgba[2]),
        a: channel(rgba[3]),
    }
}

/// The catalog table for every carried effect, in EffectCraft's order (by
/// category, then name). See `kerf_model::catalog` for the format.
pub fn catalog_table() -> String {
    let mut out = String::from(
        "# The EffectCraft effects Kerf keeps, generated by kerf-fx.\n\
         # Regenerate with `UPDATE_CATALOG=1 cargo test -p kerf-fx`.\n",
    );
    for spec in kerf_raster_fx::registry() {
        let Some((id, name)) = carried(spec) else {
            continue;
        };
        let guidance = guide::GUIDE
            .iter()
            .find(|(guided, _)| *guided == id)
            .map_or("", |(_, guidance)| guidance);
        writeln!(out, "effect\t{id}\t{}\t{name}\t{guidance}", spec.category).unwrap();
        let listed = spec.params.iter().filter(|p| p.ui != ParamUi::Hidden);
        for param in listed {
            assert!(
                !param.id.contains(['\t', ' ', '=', '"', '{', '}']),
                "{}: parameter `{}` is not a script word",
                spec.id,
                param.id
            );
            if let (ParamUi::Point, FxValue::Vec2([x, y])) = (&param.ui, &param.default) {
                // A point is written as two numbers across the picture.
                let [x_name, y_name] = point_names(param.id);
                for other in &spec.params {
                    assert!(
                        other.id != x_name && other.id != y_name,
                        "{}: {}",
                        spec.id,
                        other.id
                    );
                }
                writeln!(
                    out,
                    "\tnumber\t{x_name}\t{x}\t\tpicture widths\t{} (0 left, 1 right)",
                    param.name
                )
                .unwrap();
                writeln!(
                    out,
                    "\tnumber\t{y_name}\t{y}\t\tpicture heights\t{} (0 top, 1 bottom)",
                    param.name
                )
                .unwrap();
                continue;
            }
            let (ty, default, constraint, unit) = match (&param.ui, &param.default) {
                (ParamUi::Popup { options }, FxValue::Enum(index)) => {
                    let choices = choices(options);
                    let default = choices[(*index as usize).min(choices.len() - 1)].clone();
                    ("enum", default, choices.join("|"), "")
                }
                (ParamUi::Checkbox, FxValue::Bool(on)) => {
                    ("bool", on.to_string(), String::new(), "")
                }
                (ParamUi::Color, FxValue::Color(rgba)) => {
                    ("color", color(*rgba).to_string(), String::new(), "")
                }
                (ParamUi::Text, FxValue::Str(text)) => ("text", text.clone(), String::new(), ""),
                // A mask is one of the clip's, by name (EffectCraft counts
                // them from 1, and 0 is none).
                (ParamUi::Mask, FxValue::Scalar(_)) => ("mask", String::new(), String::new(), ""),
                // Another layer is a track of the sequence: a sound track
                // where the effect draws its sound, a picture track else.
                (ParamUi::Layer, FxValue::Layer(None)) if SOUND_LAYERS.contains(&param.id) => {
                    ("sound", String::new(), String::new(), "")
                }
                (ParamUi::Layer, FxValue::Layer(None)) => {
                    ("track", String::new(), String::new(), "")
                }
                (ParamUi::File { filters }, FxValue::Str(path)) => {
                    ("file", path.clone(), filters.join("|"), "")
                }
                (ui, FxValue::Scalar(value)) => match ui {
                    // The valid range, widened to hold the default.
                    ParamUi::Slider { min, max, .. } => (
                        "number",
                        value.to_string(),
                        format!("{}..{}", min.min(*value), max.max(*value)),
                        "",
                    ),
                    ParamUi::Angle => ("number", value.to_string(), String::new(), "degrees"),
                    _ => ("number", value.to_string(), String::new(), ""),
                },
                (ui, default) => panic!("{}: `{}` is {ui:?} with {default:?}", spec.id, param.id),
            };
            assert!(
                !default.contains(['\t', '\n']),
                "{}: `{}` has a default the table cannot hold",
                spec.id,
                param.id
            );
            writeln!(
                out,
                "\t{ty}\t{}\t{default}\t{constraint}\t{unit}\t{}",
                param.id, param.name
            )
            .unwrap();
        }
    }
    out
}

/// A clip's raster effects, ready to run on its source frames.
pub struct RasterStack {
    steps: Vec<Step>,
    /// The clip's masks, which cut each frame before the effects run, as
    /// Kerf's renderer cuts them (`kerf_draw::mask`).
    masks: Vec<kerf_model::Mask>,
    layer_size: [f64; 2],
}

struct Step {
    spec: &'static EffectSpec,
    params: kerf_model::Params,
    seed: u32,
}

impl RasterStack {
    /// The enabled raster effects of `effects` on a picture of `width` ×
    /// `height` pixels. `seed` makes random effects differ between clips
    /// and stay the same between renders.
    pub fn new(effects: &[Effect], width: u32, height: u32, seed: u32) -> Result<Self, String> {
        let steps = effects
            .iter()
            .filter(|effect| effect.enabled)
            .filter(|effect| {
                Catalog::builtin()
                    .fx(effect.fx.as_str())
                    .is_none_or(|fx| fx.engine == Engine::Raster)
            })
            .enumerate()
            .map(|(index, effect)| {
                let spec = spec(effect.fx.as_str())
                    .ok_or_else(|| format!("`{}` is not a raster effect", effect.fx))?;
                Ok(Step {
                    spec,
                    params: effect.params.clone(),
                    seed: seed.wrapping_add(index as u32),
                })
            })
            .collect::<Result<_, String>>()?;
        let layer_size = [f64::from(width), f64::from(height)];
        Ok(Self {
            steps,
            masks: Vec::new(),
            layer_size,
        })
    }

    /// Cut each frame with `masks` first, as a clip's masks cut it before
    /// its effects run. Masks that cut nothing are left out.
    pub fn with_masks(mut self, masks: &[kerf_model::Mask]) -> Self {
        self.masks = masks
            .iter()
            .filter(|mask| kerf_draw::mask::cuts(mask))
            .cloned()
            .collect();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.masks.is_empty()
    }

    /// Run the stack on one frame of straight 8-bit RGBA at clip time `at`.
    /// The frame may be smaller than the picture, as a proxy is; effects
    /// measured in pixels keep their size relative to the picture.
    pub fn apply(&self, rgba: &mut [u8], width: u32, height: u32, at: Time, frame_rate: f64) {
        let scale = f64::from(width) / self.layer_size[0];
        if self.steps.is_empty() {
            kerf_cut(rgba, width, height, &self.masks, self.layer_size, scale, at);
            return;
        }
        // The masks cut the picture to transparency, as Kerf's renderer cuts
        // it, and the effects run on that; the frame is then over black.
        let covered = if self.masks.is_empty() {
            None
        } else {
            let flat = kerf_draw::mask::prepare(
                &self.masks,
                (self.layer_size[0], self.layer_size[1]),
                scale,
                at,
            );
            kerf_draw::mask::coverage(&flat, width as usize, height as usize)
        };
        let mut image = Image::new(width, height);
        for (index, (pixel, bytes)) in image.data.iter_mut().zip(rgba.as_chunks::<4>().0.iter()).enumerate() {
            let cover = covered.as_ref().map_or(1.0, |covered| covered[index]);
            let alpha = f32::from(bytes[3]) / 255.0 * cover;
            let channel = |value: u8| f32::from(value) / 255.0 * alpha;
            *pixel = [
                channel(bytes[0]),
                channel(bytes[1]),
                channel(bytes[2]),
                alpha,
            ];
        }
        let mut buf = Buf {
            img: image,
            offset: [0.0; 2],
            scale,
        };
        let time = at.seconds();
        for (index, step) in self.steps.iter().enumerate() {
            let params = step.values(at, self.layer_size);
            let ctx = EffectCtx {
                params: &params,
                time,
                layer_size: self.layer_size,
                seed: step.seed,
                adjustment: false,
                env: EffectEnv {
                    comp_time: time,
                    frame_rate,
                    effect_index: index,
                    ..EffectEnv::default()
                },
            };
            buf = kerf_raster_fx::apply(step.spec, &ctx, buf);
        }
        write_back(&buf, rgba, width, height, scale);
        if covered.is_some() {
            over_black(rgba);
        }
    }
}

/// Cut opaque straight RGBA by `masks` as Kerf's renderer cuts a picture
/// over black: each pixel's light weighed by its coverage.
fn kerf_cut(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    masks: &[kerf_model::Mask],
    layer_size: [f64; 2],
    scale: f64,
    at: Time,
) {
    let flat = kerf_draw::mask::prepare(masks, (layer_size[0], layer_size[1]), scale, at);
    let Some(coverage) = kerf_draw::mask::coverage(&flat, width as usize, height as usize) else {
        return;
    };
    let decode = kerf_draw::color::decode_table();
    for (pixel, covered) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        for channel in &mut pixel[..3] {
            *channel = kerf_draw::color::encode(decode[usize::from(*channel)] * covered);
        }
        pixel[3] = 255;
    }
}

impl Step {
    /// EffectCraft's parameters at clip time `at`.
    fn values(&self, at: Time, layer_size: [f64; 2]) -> Params {
        let values = self
            .spec
            .params
            .iter()
            .map(|param| {
                (
                    param.id.to_owned(),
                    value_at(param, &self.params, at, layer_size),
                )
            })
            .collect();
        Params { values }
    }
}

/// The value of an EffectCraft parameter at clip time `at`: Kerf's value
/// where the clip sets one, the effect's default otherwise. A point is
/// read from its `_x` and `_y` numbers across a picture of `layer_size`
/// pixels.
pub fn value_at(
    param: &ParamSpec,
    params: &kerf_model::Params,
    at: Time,
    layer_size: [f64; 2],
) -> FxValue {
    match (&param.ui, &param.default) {
        (ParamUi::Point, FxValue::Vec2(default)) => {
            let [x, y] = point_names(param.id).map(|name| {
                params
                    .get(&name)
                    .and_then(|set| set.value_at(at).as_number())
            });
            FxValue::Vec2([
                x.unwrap_or(default[0]) * layer_size[0],
                y.unwrap_or(default[1]) * layer_size[1],
            ])
        }
        // A hidden parameter is set only by Kerf itself, as a renderer puts
        // in the Warp Stabilizer's analysis; the catalog lists none.
        _ => params
            .get(param.id)
            .and_then(|set| convert(set.value_at(at), param))
            .unwrap_or_else(|| kerf_raster_fx::default_value(param, layer_size)),
    }
}

fn convert(value: Value, param: &ParamSpec) -> Option<FxValue> {
    Some(match (value, &param.ui) {
        (Value::Number(number), _) => FxValue::Scalar(number),
        (Value::Bool(on), _) => FxValue::Bool(on),
        (Value::Color(rgba), _) => FxValue::Color(
            [rgba.r, rgba.g, rgba.b, rgba.a].map(|channel| f64::from(channel) / 255.0),
        ),
        (Value::Enum(choice), ParamUi::Popup { options }) => {
            FxValue::Enum(choices(options).iter().position(|c| *c == choice)? as u32)
        }
        (Value::Text(text), ParamUi::Text | ParamUi::File { .. } | ParamUi::Hidden) => {
            FxValue::Str(text)
        }
        _ => return None,
    })
}

/// Copy the buffer back over the frame it came from. Effects may grow the
/// buffer (a glow past the edges) or move its origin; the frame keeps its
/// own rectangle.
/// Straight 8-bit RGBA over black, opaque, mixed in linear light.
fn over_black(rgba: &mut [u8]) {
    let decode = kerf_draw::color::decode_table();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = f32::from(pixel[3]) / 255.0;
        for channel in &mut pixel[..3] {
            *channel = kerf_draw::color::encode(decode[usize::from(*channel)] * alpha);
        }
        pixel[3] = 255;
    }
}

fn write_back(buf: &Buf, rgba: &mut [u8], width: u32, height: u32, scale: f64) {
    let ratio = buf.scale / scale;
    let (source_width, source_height) = (i64::from(buf.img.width), i64::from(buf.img.height));
    for y in 0..height {
        let row = ((f64::from(y) * ratio) + buf.offset[1]).round() as i64;
        for x in 0..width {
            let column = ((f64::from(x) * ratio) + buf.offset[0]).round() as i64;
            let pixel = if (0..source_width).contains(&column) && (0..source_height).contains(&row)
            {
                buf.img.data[(row * source_width + column) as usize]
            } else {
                [0.0; 4]
            };
            let alpha = pixel[3].clamp(0.0, 1.0);
            let channel = |value: f32| {
                let straight = if alpha > 0.0 { value / alpha } else { 0.0 };
                (straight.clamp(0.0, 1.0) * 255.0).round() as u8
            };
            let at = ((y * width + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&[
                channel(pixel[0]),
                channel(pixel[1]),
                channel(pixel[2]),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Id, Param};

    #[test]
    fn raster_catalog_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../foundation/kerf-model/src/raster_effects.tsv"
        );
        let table = catalog_table();
        if std::env::var_os("UPDATE_CATALOG").is_some() {
            std::fs::write(path, &table).unwrap();
            return;
        }
        assert!(
            std::fs::read_to_string(path).unwrap() == table,
            "the raster catalog is out of date; run `UPDATE_CATALOG=1 cargo test -p kerf-fx`"
        );
    }

    #[test]
    fn every_listed_effect_runs_here() {
        let raster: Vec<_> = Catalog::builtin()
            .all()
            .filter(|fx| fx.engine == Engine::Raster)
            .collect();
        assert_eq!(raster.len(), kept::KEPT.len());
        for fx in raster {
            let spec = spec(fx.id.as_str()).unwrap_or_else(|| panic!("{}", fx.id));
            for param in &fx.params {
                let named = |p: &ParamSpec| {
                    p.id == param.name
                        || (p.ui == ParamUi::Point && point_names(p.id).contains(&param.name))
                };
                assert!(spec.params.iter().any(named), "{}: {}", fx.id, param.name);
            }
        }
        assert!(spec("blur.radialzoom").is_some());
        assert!(spec("blur.cross").is_none());
        assert!(spec("transition.linearwipe").is_none());
    }

    #[test]
    fn every_carried_effect_says_when_to_use_it() {
        let carried: Vec<String> = kerf_raster_fx::registry()
            .iter()
            .filter_map(|spec| carried(spec).map(|(id, ..)| id))
            .collect();
        for id in &carried {
            assert!(
                guide::GUIDE.iter().any(|(guided, _)| guided == id),
                "no guidance for {id}"
            );
        }
        for (guided, guidance) in guide::GUIDE {
            assert!(
                carried.iter().any(|id| id == guided),
                "guidance for {guided}, which is not carried"
            );
            assert!(
                guidance.ends_with('.') && !guidance.contains('\t'),
                "{guided}"
            );
        }
    }

    #[test]
    fn points_are_written_across_the_picture() {
        let spec = spec("distort.bulge").unwrap();
        let step = Step {
            spec,
            params: [("center_x".to_owned(), Param::Static(Value::Number(0.25)))]
                .into_iter()
                .collect(),
            seed: 0,
        };
        let values = step.values(Time::ZERO, [200.0, 100.0]);
        // x as set, y at its default of the middle, both in pixels.
        assert_eq!(values.v2("center"), [50.0, 50.0]);
    }

    #[test]
    fn choices_are_script_words() {
        let options = ["Red & Green", "Red & Green", "—", "Alpha"].map(String::from);
        assert_eq!(
            choices(&options),
            ["red-green", "red-green2", "option3", "alpha"]
        );
    }

    fn effect(id: &str, params: &[(&str, Param)]) -> Effect {
        Effect {
            fx: Id::new(id).unwrap(),
            version: 1,
            enabled: true,
            params: params
                .iter()
                .map(|(name, param)| ((*name).to_owned(), param.clone()))
                .collect(),
        }
    }

    #[test]
    fn curves_are_written_as_points() {
        let curve = Param::Static(Value::Text("0,0 0.5,0.75 1,1".into()));
        let stack =
            RasterStack::new(&[effect("color.curves", &[("rgb", curve)])], 1, 1, 0).unwrap();
        let mut frame = [128, 128, 128, 255];
        stack.apply(&mut frame, 1, 1, Time::ZERO, 24.0);
        // The midpoint lifted to three quarters.
        assert!(frame[0].abs_diff(191) <= 2, "{frame:?}");
    }

    #[test]
    fn hue_saturation_desaturates_a_frame() {
        let grey = Param::Static(Value::Number(-100.0));
        let stack = RasterStack::new(
            &[effect("color.huesaturation", &[("saturation", grey)])],
            1,
            1,
            0,
        )
        .unwrap();
        let mut frame = [200, 100, 0, 255];
        stack.apply(&mut frame, 1, 1, Time::ZERO, 24.0);
        assert!(frame[0] == frame[1] && frame[1] == frame[2], "{frame:?}");
    }

    #[test]
    fn parameters_follow_their_keyframes() {
        let spec = spec("color.huesaturation").unwrap();
        let saturation = spec
            .params
            .iter()
            .find(|p| p.id == "saturation")
            .expect("Hue/Saturation has a saturation");
        let keys = Param::Animated(vec![
            kerf_model::Keyframe {
                at: Time::ZERO,
                value: Value::Number(0.0),
                interp: kerf_model::Interp::Linear,
            },
            kerf_model::Keyframe {
                at: Time::from_flicks(705_600_000),
                value: Value::Number(100.0),
                interp: kerf_model::Interp::Linear,
            },
        ]);
        let step = Step {
            spec,
            params: [(saturation.id.to_owned(), keys)].into_iter().collect(),
            seed: 0,
        };
        let half = step.values(Time::from_flicks(352_800_000), [2.0, 1.0]);
        assert_eq!(half.f("saturation"), 50.0);
    }
}
