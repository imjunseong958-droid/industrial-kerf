//! A clip's Kerf effect as the FilmCraft effect it carries, at a time drawn: its parameters
//! read back from the way [`super::catalog`] writes them, and its LUT files read.

use super::catalog::{axes, carried, written, Written};
use super::project::{
    EffectDef, EffectInstance, Param as FxParam, ParamDef, ParamValue, Project, ProjectLut,
};
use crate::filmcraft::tick::Tick;
use kerf_geom::Vec2;
use kerf_model::{Effect, Param, Rgba, Time, Value};
use std::collections::BTreeMap;

/// FilmCraft ticks per Kerf flick.
const TICKS_PER_FLICK: i64 =
    crate::filmcraft::tick::TICKS_PER_SECOND / kerf_model::time::FLICKS_PER_SECOND;

/// A Kerf time as FilmCraft's.
pub fn tick(time: Time) -> Tick {
    Tick(time.flicks().saturating_mul(TICKS_PER_FLICK))
}

fn time(tick: Tick) -> Time {
    Time::from_flicks(tick.0.div_euclid(TICKS_PER_FLICK))
}

/// The picture an effect runs on.
#[derive(Clone, Copy, Debug)]
pub struct Picture {
    /// Its size, in its own pixels.
    pub size: (usize, usize),
    /// How many of its pixels a source pixel is (FilmCraft's `px_scale`).
    pub per_source: f32,
}

impl Picture {
    /// A point across the picture (0 to 1) in source pixels, as FilmCraft writes points.
    fn source_point(self, x: f64, y: f64) -> Vec2 {
        let per_source = f64::from(self.per_source.max(1e-6));
        Vec2::new(
            x * self.size.0 as f64 / per_source,
            y * self.size.1 as f64 / per_source,
        )
    }
}

/// The FilmCraft effect a clip's `effect` carries, with its parameters at clip time `at` on
/// `picture`; `track` is the place (from 1, as FilmCraft's `Video 1`) of a picture track by id.
pub fn instance(
    effect: &Effect,
    at: Time,
    picture: Picture,
    track: &dyn Fn(&str) -> Option<usize>,
) -> Option<EffectInstance> {
    let def = carried(effect.fx.as_str())?;
    let mut params = BTreeMap::new();
    for param in &def.params {
        let value = read(def, param, effect, at, picture, track)
            .unwrap_or_else(|| FxParam::new(param.default.clone()));
        params.insert(param.id.to_owned(), value);
    }
    Some(EffectInstance {
        effect: def.id.to_owned(),
        enabled: true,
        params,
    })
}

/// What a FilmCraft effect knows beyond its parameters as it runs.
pub struct Run<'a> {
    /// The sequence's working color space, which Lumetri grades in.
    pub working: kerf_color::WorkingSpace,
    /// Clip time.
    pub at: Time,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f32,
    /// The sequence's timecode now, for burn-ins.
    pub timecode: &'a str,
    pub clip_name: &'a str,
    /// The LUT files the program's effects name.
    pub library: &'a Project,
    /// The clip's other frames and the sequence's other tracks; none on an adjustment layer.
    pub env: Option<&'a dyn super::vfx::FxEnv>,
    pub frame_rate: f64,
}

/// Run a FilmCraft effect on `picture`, premultiplied linear light.
pub fn apply(picture: &mut kerf_draw::Image, instance: &EffectInstance, run: &Run) {
    let cx = super::effects::FxCtx {
        t: tick(run.at),
        px_scale: run.per_source,
        seconds: run.at.seconds(),
        timecode: run.timecode,
        clip_name: run.clip_name,
        project: Some(run.library),
        env: run.env,
        frame_rate: run.frame_rate,
        working: run.working,
    };
    super::effects::apply(picture, instance, &cx);
}

/// An effect as the GPU runs it.
#[derive(Clone, Debug, PartialEq)]
pub enum GpuEffect {
    Op(super::gpufx::FxOp),
    Kernel(super::vfx::gpu::Kernel),
}

impl GpuEffect {
    /// The effect on `picture`, or `None` when its kernels did not build.
    pub fn run(
        &self,
        frame: &mut kerf_gpu::Frame<'static>,
        picture: kerf_gpu::Picture,
    ) -> Option<kerf_gpu::Picture> {
        match self {
            Self::Op(op) => super::gpu::run(frame, picture, std::slice::from_ref(op)),
            Self::Kernel(kernel) => kernel.run(frame, picture),
        }
    }
}

/// `instance` on a picture of `size` as the GPU runs it, when it can run it as `apply` would.
pub fn gpu_effect(instance: &EffectInstance, run: &Run, size: (usize, usize)) -> Option<GpuEffect> {
    let cx = super::effects::FxCtx {
        t: tick(run.at),
        px_scale: run.per_source,
        seconds: run.at.seconds(),
        timecode: run.timecode,
        clip_name: run.clip_name,
        project: Some(run.library),
        env: None,
        frame_rate: run.frame_rate,
        working: run.working,
    };
    if let Some(kernel) = super::vfx::gpu::Kernel::eval(instance, &cx, size) {
        return Some(GpuEffect::Kernel(kernel));
    }
    super::gpufx::FxOp::eval(instance, &cx, size.0, size.1)
        .filter(super::gpufx::FxOp::gpu_ok)
        .map(GpuEffect::Op)
}

/// Run a clip's carried FilmCraft `effects` on an 8-bit RGBA `picture` of `size` alone, as a
/// preview of them does: without the clip's other frames or the sequence's other tracks, at clip
/// time `at`, with `per_source` of its pixels to a source pixel.
pub fn apply_rgba(
    picture: &mut [u8],
    size: (usize, usize),
    effects: &[Effect],
    at: Time,
    per_source: f32,
    frame_rate: f64,
) -> Result<(), String> {
    let library = library(effects)?;
    let mut image = kerf_draw::Image::from_rgba8(picture, size.0, size.1, size.0 * 4, true);
    let on = Picture { size, per_source };
    for effect in effects {
        let Some(instance) = instance(effect, at, on, &|_| None) else {
            continue;
        };
        let run = Run {
            working: kerf_color::WorkingSpace::Rec709,
            at,
            per_source,
            timecode: "00:00:00:00",
            clip_name: "",
            library: &library,
            env: None,
            frame_rate,
        };
        apply(&mut image, &instance, &run);
    }
    picture.copy_from_slice(&image.to_rgba8_over_black());
    Ok(())
}

/// A parameter as FilmCraft's effect reads it; `None` keeps FilmCraft's default.
fn read(
    def: &EffectDef,
    param: &ParamDef,
    effect: &Effect,
    at: Time,
    picture: Picture,
    track: &dyn Fn(&str) -> Option<usize>,
) -> Option<FxParam> {
    let get = |name: &str| effect.params.get(name).cloned();
    let one = |name: &str, convert: fn(&Value, &ParamValue) -> ParamValue| {
        let set = get(name)?;
        let default = param.default.clone();
        Some(over_time([set], at, move |[value]| {
            convert(value, &default)
        }))
    };
    match written(def.id, param) {
        Written::Number => one(param.id, |value, default| match value {
            Value::Number(n) => ParamValue::Float(*n),
            _ => default.clone(),
        }),
        Written::Bool => one(param.id, |value, default| match value {
            Value::Bool(b) => ParamValue::Bool(*b),
            _ => default.clone(),
        }),
        Written::Color => one(param.id, |value, default| match value {
            Value::Color(c) => ParamValue::Color(straight(*c)),
            _ => default.clone(),
        }),
        Written::Text => one(param.id, |value, default| match value {
            Value::Text(t) => ParamValue::Text(t.clone()),
            _ => default.clone(),
        }),
        Written::Choice(words) => {
            let word = text(&get(param.id)?.value_at(at))?;
            let index = words.iter().position(|w| *w == word)?;
            Some(FxParam::new(ParamValue::Choice(index as u32)))
        }
        Written::Track => {
            let id = text(&get(param.id)?.value_at(at))?;
            let place = if id.is_empty() {
                0
            } else {
                track(&id).unwrap_or(0)
            };
            Some(FxParam::new(ParamValue::Choice(place as u32)))
        }
        Written::Point { offset } => {
            let [x, y] = axes(param.id);
            let (sx, sy) = (get(&x), get(&y));
            if sx.is_none() && sy.is_none() && !offset {
                // Where the catalog starts it.
                let (fx, fy) = start(def, param);
                return Some(FxParam::new(ParamValue::Vec2(picture.source_point(fx, fy))));
            }
            let default = param.default.as_vec2().unwrap_or_default();
            let (dx, dy) = if offset {
                (
                    if default.x.is_nan() { 0.0 } else { default.x },
                    if default.y.is_nan() { 0.0 } else { default.y },
                )
            } else {
                start(def, param)
            };
            let sx = sx.unwrap_or(Param::Static(Value::Number(dx)));
            let sy = sy.unwrap_or(Param::Static(Value::Number(dy)));
            Some(over_time([sx, sy], at, move |[x, y]| {
                let (x, y) = (x.as_number().unwrap_or(dx), y.as_number().unwrap_or(dy));
                ParamValue::Vec2(if offset {
                    Vec2::new(x, y)
                } else {
                    picture.source_point(x, y)
                })
            }))
        }
        Written::Wheel => {
            let [x, y] = axes(param.id);
            let (sx, sy) = (get(&x), get(&y));
            if sx.is_none() && sy.is_none() {
                return None;
            }
            let zero = || Param::Static(Value::Number(0.0));
            Some(over_time(
                [sx.unwrap_or_else(zero), sy.unwrap_or_else(zero)],
                at,
                |[x, y]| {
                    ParamValue::Vec2(Vec2::new(
                        x.as_number().unwrap_or(0.0),
                        y.as_number().unwrap_or(0.0),
                    ))
                },
            ))
        }
        Written::Curve => {
            let text = text(&get(param.id)?.value_at(at))?;
            Some(FxParam::new(ParamValue::Curve(curve(&text)?)))
        }
        Written::Lut => {
            let path = get(param.id)
                .and_then(|p| text(&p.value_at(at)))
                .filter(|p| !p.trim().is_empty());
            let camera = (param.id == "input_lut")
                .then(|| get("camera_lut").and_then(|p| text(&p.value_at(at))))
                .flatten()
                .filter(|c| c != "none");
            let spec = match (path, camera) {
                (Some(path), _) => format!("lib:{path}"),
                (None, Some(camera)) => format!("builtin:{camera}"),
                (None, None) => return None,
            };
            Some(FxParam::new(ParamValue::Text(spec)))
        }
    }
}

/// A parameter from Kerf parameters `set`, as `convert` reads their values: its value at `at`,
/// and, when any of them animate, at any time.
fn over_time<const N: usize>(
    set: [Param; N],
    at: Time,
    convert: impl Fn([&Value; N]) -> ParamValue + Send + Sync + 'static,
) -> FxParam {
    let values = |time: Time| set.each_ref().map(|p| p.value_at(time));
    let now = {
        let v = values(at);
        convert(v.each_ref())
    };
    if set.iter().all(|p| matches!(p, Param::Static(_))) {
        return FxParam::new(now);
    }
    FxParam::over_time(now, move |t| {
        let v = set.each_ref().map(|p| p.value_at(time(t)));
        convert(v.each_ref())
    })
}

/// Where the catalog starts a point, across the picture.
fn start(def: &EffectDef, param: &ParamDef) -> (f64, f64) {
    let v = param
        .default
        .as_vec2()
        .unwrap_or(Vec2::new(f64::NAN, f64::NAN));
    let (ax, ay) = super::project::effect::auto_point(def.id, param.id)
        .unwrap_or((0.5, if param.id == "end" { 1.0 } else { 0.5 }));
    (
        if v.x.is_nan() { ax } else { 0.0 },
        if v.y.is_nan() { ay } else { 0.0 },
    )
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(t) | Value::Enum(t) => Some(t.clone()),
        _ => None,
    }
}

/// A Kerf color as FilmCraft's: sRGB-encoded, straight.
fn straight(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a].map(|v| f32::from(v) / 255.0)
}

/// A curve's `x,y` points, sorted by `x`; `None` when one does not read.
pub(crate) fn curve(text: &str) -> Option<Vec<[f32; 2]>> {
    let mut points = Vec::new();
    for point in text.split_whitespace() {
        let (x, y) = point.split_once(',')?;
        let (x, y): (f32, f32) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        points.push([x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]);
    }
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    Some(points)
}

/// The LUT files a clip's effects name, as Lumetri reads them (`lib:<path>`). A file that
/// cannot be read is an error naming it.
pub fn library<'a>(effects: impl IntoIterator<Item = &'a Effect>) -> Result<Project, String> {
    let mut library = Project::new("kerf");
    for effect in effects {
        let Some(def) = carried(effect.fx.as_str()) else {
            continue;
        };
        for param in def
            .params
            .iter()
            .filter(|p| written(def.id, p) == Written::Lut)
        {
            let Some(Param::Static(Value::Text(path))) = effect.params.get(param.id) else {
                continue;
            };
            if path.trim().is_empty() || library.luts.iter().any(|l| l.id == *path) {
                continue;
            }
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read LUT `{path}`: {error}"))?;
            let format = std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("cube")
                .to_ascii_lowercase();
            library.luts.push(ProjectLut {
                id: path.clone(),
                name: path.clone(),
                source_path: Some(path.clone()),
                format,
                text: crate::filmcraft::luts::shared_text(text),
            });
        }
    }
    Ok(library)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filmcraft::effects::{apply, FxCtx};
    use kerf_draw::Image;
    use kerf_model::{Catalog, Id, Interp, Keyframe};

    fn effect(id: &str, params: &[(&str, Param)]) -> Effect {
        Effect {
            fx: Id::new(id).unwrap(),
            version: 2,
            enabled: true,
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        }
    }

    fn number(n: f64) -> Param {
        Param::Static(Value::Number(n))
    }

    const PICTURE: Picture = Picture {
        size: (64, 36),
        per_source: 0.5,
    };

    #[test]
    fn parameters_read_back_as_the_catalog_writes_them() {
        let none = |_: &str| None;
        // Defaults are FilmCraft's.
        let lumetri = instance(&effect("color.lumetri", &[]), Time::ZERO, PICTURE, &none).unwrap();
        assert_eq!(
            lumetri,
            crate::filmcraft::project::find_effect("lumetri")
                .unwrap()
                .instance()
        );
        // A point across the picture is in source pixels: 64 × 36 at half the source.
        let twirl = effect(
            "distort.twirl",
            &[("center_x", number(0.25)), ("angle", number(90.0))],
        );
        let twirl = instance(&twirl, Time::ZERO, PICTURE, &none).unwrap();
        assert_eq!(twirl.vec2_at("center", Tick(0)), Vec2::new(32.0, 36.0));
        assert_eq!(twirl.f64_at("angle", Tick(0)), 90.0);
        // An offset is in pixels; a choice is its word's place.
        let moved = effect(
            "transform.move",
            &[
                ("from_x", number(-40.0)),
                ("easing", Param::Static(Value::Enum("linear".into()))),
            ],
        );
        let moved = instance(&moved, Time::ZERO, PICTURE, &none).unwrap();
        assert_eq!(moved.vec2_at("from", Tick(0)), Vec2::new(-40.0, 0.0));
        assert_eq!(moved.param("easing").unwrap().value, ParamValue::Choice(0));
        // A track by id, and a curve.
        let matte = effect(
            "key.trackmatte",
            &[("matte", Param::Static(Value::Text("v2".into())))],
        );
        let place = |id: &str| (id == "v2").then_some(2);
        let matte = instance(&matte, Time::ZERO, PICTURE, &place).unwrap();
        assert_eq!(matte.param("matte").unwrap().value, ParamValue::Choice(2));
        let graded = effect(
            "color.lumetri",
            &[(
                "curve_luma",
                Param::Static(Value::Text("1,1 0,0.1 0.5,0.6".into())),
            )],
        );
        let graded = instance(&graded, Time::ZERO, PICTURE, &none).unwrap();
        assert_eq!(
            graded.param("curve_luma").unwrap().value,
            ParamValue::Curve(vec![[0.0, 0.1], [0.5, 0.6], [1.0, 1.0]])
        );
    }

    #[test]
    fn animated_parameters_follow_kerf_keyframes() {
        let second = Time::from_seconds(1.0).unwrap();
        let keys = Param::Animated(vec![
            Keyframe {
                at: Time::ZERO,
                value: Value::Number(0.0),
                interp: Interp::Linear,
            },
            Keyframe {
                at: second,
                value: Value::Number(1.0),
                interp: Interp::Linear,
            },
        ]);
        let half = Time::from_seconds(0.5).unwrap();
        let swirl = effect("distort.twirl", &[("center_x", keys)]);
        let swirl = instance(&swirl, half, PICTURE, &|_| None).unwrap();
        // Now, and around now as Transform's shutter reads it.
        assert_eq!(swirl.vec2_at("center", tick(half)).x, 64.0);
        assert_eq!(swirl.vec2_at("center", tick(second)).x, 128.0);
        assert!(swirl.is_animated());
        assert_eq!(time(tick(half)), half);
    }

    #[test]
    fn every_carried_effect_runs_from_its_catalog_defaults() {
        let catalog = Catalog::builtin();
        for (id, _) in crate::filmcraft::catalog::definitions() {
            let def = catalog
                .fx(id)
                .unwrap_or_else(|| panic!("the catalog lists {id}"));
            // Every parameter set to its catalog default reads as FilmCraft's default.
            let params: Vec<(&str, Param)> = def
                .params
                .iter()
                .map(|p| (p.name.as_str(), Param::Static(p.default.clone())))
                .collect();
            let set = instance(
                &effect(id, &params),
                Time::ZERO,
                Picture {
                    size: (24, 16),
                    per_source: 1.0,
                },
                &|_| None,
            )
            .unwrap();
            let bare = instance(
                &effect(id, &[]),
                Time::ZERO,
                Picture {
                    size: (24, 16),
                    per_source: 1.0,
                },
                &|_| None,
            )
            .unwrap();
            for (name, param) in &bare.params {
                let read = &set.params[name].value;
                let close = match (read, &param.value) {
                    (ParamValue::Color(a), ParamValue::Color(b)) => a
                        .iter()
                        .zip(b)
                        .all(|(a, b)| (a - b).abs() <= 0.5 / 255.0 + 1e-6),
                    (a, b) => a == b,
                };
                assert!(close, "{id}: {name} reads {read:?}, not {:?}", param.value);
            }
            let mut picture = Image::filled(24, 16, [0.2, 0.4, 0.1, 1.0]);
            let cx = FxCtx {
                t: Tick(0),
                px_scale: 1.0,
                seconds: 0.0,
                timecode: "00:00:00:00",
                clip_name: "clip",
                project: None,
                env: None,
                frame_rate: 24.0,
                working: kerf_color::WorkingSpace::Rec709,
            };
            apply(&mut picture, &set, &cx);
            assert!(picture.px.iter().all(|v| v.is_finite()), "{id}");
        }
    }

    #[test]
    fn a_lut_file_is_read_into_the_library() {
        let dir = std::env::temp_dir().join(format!("kerf-lut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("swap.cube");
        let cube =
            kerf_color::Lut::from_cube(kerf_color::Lut3d::from_fn(3, |c| [c[2], c[1], c[0]]))
                .to_cube();
        std::fs::write(&path, cube).unwrap();
        let path = path.to_str().unwrap().to_owned();
        let graded = effect(
            "color.lumetri",
            &[("input_lut", Param::Static(Value::Text(path.clone())))],
        );
        let luts = library([&graded]).unwrap();
        assert_eq!(luts.luts.len(), 1);
        let read = instance(&graded, Time::ZERO, PICTURE, &|_| None).unwrap();
        let spec = format!("lib:{path}");
        assert_eq!(
            read.param("input_lut").unwrap().value,
            ParamValue::Text(spec.clone())
        );
        assert!(crate::filmcraft::luts::resolve(Some(&luts), &spec).is_some());
        // A camera conversion when no file is named.
        let camera = effect(
            "color.lumetri",
            &[(
                "camera_lut",
                Param::Static(Value::Enum("slog3-sgamut3cine-to-rec709".into())),
            )],
        );
        let read = instance(&camera, Time::ZERO, PICTURE, &|_| None).unwrap();
        assert_eq!(
            read.param("input_lut").unwrap().value,
            ParamValue::Text("builtin:slog3-sgamut3cine-to-rec709".into())
        );
        let missing = effect(
            "color.lumetri",
            &[(
                "look_lut",
                Param::Static(Value::Text(dir.join("no.cube").to_str().unwrap().into())),
            )],
        );
        assert!(library([&missing]).unwrap_err().contains("no.cube"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_lut_file_read_again_is_parsed_once() {
        let dir = std::env::temp_dir().join(format!("kerf-lut-again-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("again.cube");
        let cube =
            kerf_color::Lut::from_cube(kerf_color::Lut3d::from_fn(3, |c| [c[1], c[0], c[2]]))
                .to_cube();
        std::fs::write(&path, cube).unwrap();
        let path = path.to_str().unwrap().to_owned();
        let graded = effect(
            "color.lumetri",
            &[("input_lut", Param::Static(Value::Text(path.clone())))],
        );
        let spec = format!("lib:{path}");
        // each compositor reads its library again
        let (first, second) = (library([&graded]).unwrap(), library([&graded]).unwrap());
        assert!(std::sync::Arc::ptr_eq(
            &first.luts[0].text,
            &second.luts[0].text
        ));
        let parsed = crate::filmcraft::luts::resolve(Some(&first), &spec).unwrap();
        drop(first);
        let again = crate::filmcraft::luts::resolve(Some(&second), &spec).unwrap();
        assert!(std::sync::Arc::ptr_eq(&parsed, &again));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
