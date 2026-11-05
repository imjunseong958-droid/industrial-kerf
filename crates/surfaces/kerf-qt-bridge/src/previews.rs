//! `effect_previews`: how effects, or generators, would look, as small
//! RGBA pictures for the browser's thumbnails, its hover scrub, and the
//! colour chip on a proposal card.
//!
//! An effect named by id is shown added after the clip's own effects, at
//! its defaults. One given as `{fx, params}` is shown with those
//! parameters; when the clip already has that effect, in its place, so
//! the clip's look before and after a proposal can be compared. A
//! generator is drawn alone on black. Raster and FilmCraft effects run on
//! one decoded picture of the clip; graph effects and generators are drawn
//! by Kerf's compositor, which playback uses too.

use crate::{emit, Shared};
use kerf_compose::{ComposeOptions, Compositor};
use kerf_model::{
    Catalog, Clip, Content, Effect, Engine, FxDef, FxKind, Id, Param, ParamType, Params, Project,
    Rgba, Sequence, Time, Track, TrackKind, Value,
};
use serde::Deserialize;
use serde_json::{json, Map, Value as Json};
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;

/// One entry of `effects`: an id, or an id with parameters and the key
/// its preview echoes.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum PreviewEffect {
    Id(String),
    Params {
        fx: String,
        #[serde(default)]
        params: Map<String, Json>,
        #[serde(default)]
        key: Option<String>,
    },
}

impl PreviewEffect {
    fn fx(&self) -> &str {
        match self {
            Self::Id(fx) | Self::Params { fx, .. } => fx,
        }
    }

    fn params(&self) -> Option<&Map<String, Json>> {
        match self {
            Self::Id(_) => None,
            Self::Params { params, .. } => Some(params),
        }
    }

    fn key(&self) -> Option<&str> {
        match self {
            Self::Id(_) => None,
            Self::Params { key, .. } => key.as_deref(),
        }
    }
}

/// `effect_previews {clip, at, effects, width, key?}`.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Request {
    #[serde(default)]
    pub clip: String,
    #[serde(default)]
    pub at: f64,
    pub effects: Vec<PreviewEffect>,
    pub width: u32,
    /// Echoed on every preview of an entry that has no key of its own.
    #[serde(default)]
    pub key: Option<String>,
}

/// The longest edge a preview may have, in pixels.
const MAX_WIDTH: u32 = 640;

/// A generator previewed with no text of its own shows this, so a title
/// thumbnail is not an empty black frame.
const SAMPLE_TITLE: &str = "Title";

/// `params` for the parameters `def` has, as the model holds them: static
/// values of the parameter's type, numbers kept inside its range. Unknown
/// names and values of the wrong type are left out (so they keep their
/// defaults), as are keyframes: a preview is one picture.
pub(crate) fn params(def: &FxDef, params: &Map<String, Json>) -> Params {
    let mut out = Params::new();
    for (name, value) in params {
        let Some(param) = def.params.iter().find(|param| &param.name == name) else {
            continue;
        };
        let value = match (&param.ty, value) {
            (ParamType::Number | ParamType::Time, Json::Number(number)) => {
                let Some(mut number) = number.as_f64() else {
                    continue;
                };
                if let Some(min) = param.min {
                    number = number.max(min);
                }
                if let Some(max) = param.max {
                    number = number.min(max);
                }
                Value::Number(number)
            }
            (ParamType::Bool, Json::Bool(flag)) => Value::Bool(*flag),
            (ParamType::Enum(_), Json::String(text)) => Value::Enum(text.clone()),
            (ParamType::Color, Json::String(text)) => match Rgba::parse(text) {
                Some(color) => Value::Color(color),
                None => continue,
            },
            (
                ParamType::Text
                | ParamType::File(_)
                | ParamType::Track
                | ParamType::Sound
                | ParamType::Mask
                | ParamType::Time,
                Json::String(text),
            ) => Value::Text(text.clone()),
            _ => continue,
        };
        out.insert(name.clone(), Param::Static(value));
    }
    out
}

/// What a request previews of each entry.
#[derive(Debug, PartialEq)]
enum Plan {
    /// A generator alone on black.
    Generator,
    /// A picture effect on the clip's decoded picture.
    Raster,
    /// A picture effect the compositor draws.
    Composited,
}

fn plan(def: &FxDef) -> Option<Plan> {
    if def.domain != kerf_model::Domain::Video {
        return None;
    }
    match (def.kind, def.engine) {
        // An adjustment layer alone draws nothing.
        (FxKind::Generator, _) if def.id == "adjustment" => None,
        (FxKind::Generator, _) => Some(Plan::Generator),
        (FxKind::Effect, Engine::Raster | Engine::Kerf) => Some(Plan::Raster),
        (FxKind::Effect, Engine::Graph) => Some(Plan::Composited),
        _ => None,
    }
}

/// The effect a preview adds: `entry`'s parameters on the effect's
/// defaults.
fn previewed(def: &FxDef, entry: &PreviewEffect) -> Effect {
    Effect {
        fx: def.id.clone(),
        version: def.version,
        enabled: true,
        params: entry
            .params()
            .map(|given| params(def, given))
            .unwrap_or_default(),
    }
}

/// `stack` with `effect` previewed in it: in place of the clip's own one
/// when `replace` (its parameters over the clip's), else after them.
fn with_effect(stack: &[Effect], effect: Effect, replace: bool) -> Vec<Effect> {
    let mut stack = stack.to_vec();
    match stack.iter_mut().find(|own| replace && own.fx == effect.fx) {
        Some(own) => {
            own.enabled = true;
            own.params.extend(effect.params);
        }
        None => stack.push(effect),
    }
    stack
}

/// One preview event.
fn preview_event(
    request: &Request,
    entry: &PreviewEffect,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Json {
    use base64::Engine as _;
    let mut event = json!({
        "type": "effect_preview",
        "clip": request.clip,
        "fx": entry.fx(),
        "width": width,
        "height": height,
        "rgba": base64::engine::general_purpose::STANDARD.encode(rgba),
    });
    if let Some(key) = entry.key().or(request.key.as_deref()) {
        event["key"] = json!(key);
    }
    event
}

/// The clip `id` and the sequence it is on.
fn find_clip<'a>(project: &'a Project, id: &str) -> Option<(&'a Sequence, &'a Clip)> {
    let at = project.locate(id)?;
    Some((&project.sequences[at.sequence], project.clip_at(at)))
}

/// Draw `sequence` of `project` at `at`, `width` pixels wide (even).
fn compose(
    project: &Project,
    sequence: &Sequence,
    at: Time,
    width: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(sequence.id.to_string()),
            captions: false,
            keep_frames: true,
            ..Default::default()
        },
    )
    .ok()?;
    let (canvas, _) = compositor.size();
    let frame = compositor.frame(at, f64::from(width) / f64::from(canvas.max(1)));
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    // The compositor draws at the nearest of its own scales.
    let height =
        ((u64::from(width) * u64::from(frame.height) / u64::from(frame.width)) as u32).max(2) & !1;
    let rgba = resample(&frame.rgba, (frame.width, frame.height), (width, height))?;
    Some((width, height, rgba))
}

/// `rgba`, `from` pixels in size, averaged over each pixel's area into
/// `to`, or sampled up from the nearest pixel.
pub(crate) fn resample(rgba: &[u8], from: (u32, u32), to: (u32, u32)) -> Option<Vec<u8>> {
    let (fw, fh) = (from.0 as usize, from.1 as usize);
    let (tw, th) = (to.0 as usize, to.1 as usize);
    if rgba.len() < fw * fh * 4 || fw == 0 || fh == 0 || tw == 0 || th == 0 {
        return None;
    }
    if (fw, fh) == (tw, th) {
        return Some(rgba[..fw * fh * 4].to_vec());
    }
    let mut out = vec![0u8; tw * th * 4];
    for y in 0..th {
        let (y0, y1) = (
            y * fh / th,
            ((y + 1) * fh).div_ceil(th).max(y * fh / th + 1),
        );
        for x in 0..tw {
            let (x0, x1) = (
                x * fw / tw,
                ((x + 1) * fw).div_ceil(tw).max(x * fw / tw + 1),
            );
            let mut sum = [0u32; 4];
            for row in y0..y1.min(fh) {
                for column in x0..x1.min(fw) {
                    let at = (row * fw + column) * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[at + channel]);
                    }
                }
            }
            let count = ((y1.min(fh) - y0) * (x1.min(fw) - x0)).max(1) as u32;
            let at = (y * tw + x) * 4;
            for (channel, total) in sum.iter().enumerate() {
                out[at + channel] = ((total + count / 2) / count) as u8;
            }
        }
    }
    Some(out)
}

/// `clip` alone on `sequence` of `project`, with `effects` in place of its
/// own, drawn by the compositor at `at`.
fn compose_clip(
    project: &Project,
    sequence: &Sequence,
    clip: &Clip,
    effects: Vec<Effect>,
    at: Time,
    width: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let mut alone = clip.clone();
    alone.effects = effects;
    let at_sequence = project.sequences.iter().position(|s| s.id == sequence.id)?;
    let mut project = project.clone();
    let lone = &mut project.sequences[at_sequence];
    let mut track = Track::new(Id::new("preview").ok()?, TrackKind::Video);
    track.clips.push(alone);
    lone.tracks = vec![track];
    lone.markers.clear();
    let lone = lone.clone();
    compose(&project, &lone, at, width)
}

/// Generator `def` with `given` parameters alone on black, on a canvas
/// shaped like `like`, at its first frame.
fn compose_generator(
    def: &FxDef,
    given: Option<&Map<String, Json>>,
    like: Option<&Sequence>,
    width: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let mut chosen = given.map(|given| params(def, given)).unwrap_or_default();
    if def.id == "title" && !chosen.contains_key("text") {
        chosen.insert(
            "text".into(),
            Param::Static(Value::Text(SAMPLE_TITLE.into())),
        );
    }
    let rate = like.map_or(kerf_model::Rate::FPS_24, |sequence| sequence.rate);
    let duration = Time::from_seconds(1.0)?;
    let clip = Clip::new(
        Id::new("preview-generator").ok()?,
        Content::Generator {
            fx: def.id.clone(),
            version: def.version,
            params: chosen,
        },
        Time::ZERO,
        duration,
    );
    let mut track = Track::new(Id::new("V1").ok()?, TrackKind::Video);
    track.clips.push(clip);
    let (width_px, height_px) =
        like.map_or((1920, 1080), |sequence| (sequence.width, sequence.height));
    let sequence = Sequence {
        id: Id::new("preview").ok()?,
        name: String::new(),
        rate,
        drop_frame: false,
        width: width_px,
        height: height_px,
        sample_rate: 48_000,
        start: Time::ZERO,
        tracks: vec![track],
        markers: Vec::new(),
    };
    let project = Project {
        sequences: vec![sequence.clone()],
        ..Project::default()
    };
    compose(&project, &sequence, Time::ZERO, width)
}

/// The clip's picture at `at`, decoded from its media, `width` pixels wide.
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    /// Seconds into the clip.
    clip_time: Time,
    /// The source's own size and rate, which effect parameters scale by.
    source: (u32, u32),
    fps: f64,
}

fn decode(project: &Project, clip: &Clip, at: f64, width: u32) -> Result<Option<Picture>, String> {
    let Content::Media {
        media,
        source_in,
        speed,
    } = &clip.content
    else {
        return Ok(None);
    };
    let Some(media) = project.media(media.as_str()) else {
        return Ok(None);
    };
    let Some(video) = media.video else {
        return Ok(None);
    };
    let at = Time::from_seconds(at)
        .unwrap_or(Time::ZERO)
        .clamp(clip.start, clip.end() - media.rate.frame_duration());
    let clip_time = at - clip.start;
    let source = if speed.is_reverse() {
        *source_in + speed.source_span(clip.duration - clip_time)
    } else {
        *source_in + speed.source_span(clip_time)
    };
    let width = width.clamp(16, MAX_WIDTH) & !1;
    let height = ((u64::from(width) * u64::from(video.height) / u64::from(video.width.max(1)))
        as u32)
        .max(2)
        & !1;
    let output = kerf_engine::ffmpeg::hidden_command(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(kerf_engine::program::secs(source))
        .arg("-i")
        .arg(&media.path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={width}:{height},format=rgba"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .map_err(|err| err.to_string())?;
    let size = width as usize * height as usize * 4;
    if !output.status.success() || output.stdout.len() < size {
        return Err(format!("no picture of `{}` at that time", clip.id));
    }
    let mut rgba = output.stdout;
    rgba.truncate(size);
    Ok(Some(Picture {
        width,
        height,
        rgba,
        clip_time,
        source: (video.width, video.height),
        fps: media.rate.fps(),
    }))
}

/// `picture` through the clip's raster and FilmCraft effects with `effect`
/// previewed among them.
fn raster(clip: &Clip, picture: &Picture, effect: Effect, replace: bool) -> Option<Vec<u8>> {
    let engine_of = |effect: &Effect| {
        Catalog::builtin()
            .fx(effect.fx.as_str())
            .map(|fx| fx.engine)
    };
    let enabled: Vec<Effect> = clip
        .effects
        .iter()
        .filter(|effect| effect.enabled)
        .cloned()
        .collect();
    let stack = with_effect(&enabled, effect, replace);
    let of = |engine: Engine| -> Vec<Effect> {
        stack
            .iter()
            .filter(|effect| engine_of(effect) == Some(engine))
            .cloned()
            .collect()
    };
    let (raster, filmcraft) = (of(Engine::Raster), of(Engine::Kerf));
    let seed = kerf_core::raster::seed(clip.id.as_str());
    let stack =
        kerf_fx::RasterStack::new(&raster, picture.source.0, picture.source.1, seed).ok()?;
    let mut frame = picture.rgba.clone();
    stack.apply(
        &mut frame,
        picture.width,
        picture.height,
        picture.clip_time,
        picture.fps,
    );
    let per_source = picture.width as f32 / picture.source.0.max(1) as f32;
    kerf_fx::filmcraft::instance::apply_rgba(
        &mut frame,
        (picture.width as usize, picture.height as usize),
        &filmcraft,
        picture.clip_time,
        per_source,
        picture.fps,
    )
    .ok()?;
    Some(frame)
}

/// Emit a preview of each entry of `request`. Stops when a newer request
/// than `generation` arrives. A clip that is not in the project is an
/// error only when an effect needs its picture.
pub(crate) fn run(
    shared: &Shared,
    generation: u64,
    request: &Request,
    tx: &SyncSender<Json>,
) -> Result<(), String> {
    let project = shared
        .model
        .lock()
        .map_err(|_| "the project is unavailable".to_owned())?
        .clone()
        .ok_or_else(|| "no project is open".to_owned())?;
    let found = find_clip(&project, &request.clip);
    let width = request.width.clamp(16, MAX_WIDTH) & !1;
    let mut picture: Option<Option<Picture>> = None;
    for entry in &request.effects {
        if shared.previews.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        let Some(def) = Catalog::builtin().fx(entry.fx()) else {
            continue;
        };
        let Some(plan) = plan(def) else {
            continue;
        };
        let replace = entry.params().is_some();
        let drawn = match plan {
            Plan::Generator => {
                let like = found
                    .map(|(sequence, _)| sequence)
                    .or(project.sequences.first());
                compose_generator(def, entry.params(), like, width)
            }
            Plan::Composited => {
                let (sequence, clip) =
                    found.ok_or_else(|| format!("no clip `{}`", request.clip))?;
                let enabled: Vec<Effect> =
                    clip.effects.iter().filter(|e| e.enabled).cloned().collect();
                let effects = with_effect(&enabled, previewed(def, entry), replace);
                let at = Time::from_seconds(request.at)
                    .unwrap_or(Time::ZERO)
                    .clamp(clip.start, clip.end() - sequence.rate.frame_duration());
                compose_clip(&project, sequence, clip, effects, at, width)
            }
            Plan::Raster => {
                let (_, clip) = found.ok_or_else(|| format!("no clip `{}`", request.clip))?;
                if picture.is_none() {
                    picture = Some(decode(&project, clip, request.at, width)?);
                }
                picture
                    .as_ref()
                    .and_then(Option::as_ref)
                    .and_then(|picture| {
                        raster(clip, picture, previewed(def, entry), replace)
                            .map(|rgba| (picture.width, picture.height, rgba))
                    })
            }
        };
        if let Some((width, height, rgba)) = drawn {
            emit(tx, preview_event(request, entry, width, height, &rgba));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_are_ids_or_ids_with_parameters() {
        let crate::Command::EffectPreviews(request) = serde_json::from_value(json!({
            "type": "effect_previews", "clip": "c1", "at": 9.0, "width": 88, "key": "hover",
            "effects": ["light.edgeglow", {"fx": "color.basic", "params": {"saturation": 0.8}, "key": "after"}],
        }))
        .unwrap() else {
            panic!("expected effect_previews");
        };
        assert_eq!(request.key.as_deref(), Some("hover"));
        assert_eq!(
            request.effects[0],
            PreviewEffect::Id("light.edgeglow".into())
        );
        let PreviewEffect::Params { fx, params, key } = &request.effects[1] else {
            panic!("expected parameters");
        };
        assert_eq!(
            (fx.as_str(), key.as_deref()),
            ("color.basic", Some("after"))
        );
        assert_eq!(params["saturation"], 0.8);
        // The shape the browser has always sent still reads.
        assert!(serde_json::from_value::<crate::Command>(json!({
            "type": "effect_previews", "clip": "c1", "at": 0.0, "effects": ["a"], "width": 64,
        }))
        .is_ok());
    }

    #[test]
    fn parameters_take_the_types_and_ranges_the_catalog_gives() {
        let title = Catalog::builtin().fx("title").unwrap();
        let given = json!({
            "text": "Atlas", "size": 4.0, "color": "#ff0000", "align": 3, "unknown": 1,
        });
        let chosen = params(title, given.as_object().unwrap());
        assert_eq!(
            chosen.get("text"),
            Some(&Param::Static(Value::Text("Atlas".into())))
        );
        assert_eq!(
            chosen.get("size"),
            Some(&Param::Static(Value::Number(1.0))),
            "kept inside 0.005..1"
        );
        assert!(matches!(
            chosen.get("color"),
            Some(Param::Static(Value::Color(_)))
        ));
        assert!(!chosen.contains_key("align"), "a number is not a choice");
        assert!(!chosen.contains_key("unknown"));
    }

    #[test]
    fn an_effect_with_parameters_takes_the_place_of_the_clips_own() {
        let def = Catalog::builtin().fx("blur.camera").unwrap();
        let own = Effect {
            fx: def.id.clone(),
            version: def.version,
            enabled: true,
            params: Params::from([("percent".to_owned(), Param::Static(Value::Number(5.0)))]),
        };
        let entry = PreviewEffect::Params {
            fx: "blur.camera".into(),
            params: json!({"percent": 40}).as_object().unwrap().clone(),
            key: None,
        };
        let replaced = with_effect(std::slice::from_ref(&own), previewed(def, &entry), true);
        assert_eq!(replaced.len(), 1);
        assert_eq!(
            replaced[0].params.get("percent"),
            Some(&Param::Static(Value::Number(40.0)))
        );
        let added = with_effect(
            std::slice::from_ref(&own),
            previewed(def, &PreviewEffect::Id("blur.camera".into())),
            false,
        );
        assert_eq!(added.len(), 2, "an id is shown added");
    }

    #[test]
    fn what_each_kind_of_entry_previews() {
        let catalog = Catalog::builtin();
        assert_eq!(plan(catalog.fx("title").unwrap()), Some(Plan::Generator));
        assert_eq!(plan(catalog.fx("solid").unwrap()), Some(Plan::Generator));
        assert_eq!(plan(catalog.fx("adjustment").unwrap()), None);
        assert_eq!(
            plan(catalog.fx("blur.gaussian").unwrap()),
            Some(Plan::Composited)
        );
        assert_eq!(plan(catalog.fx("blur.camera").unwrap()), Some(Plan::Raster));
        let audio = catalog
            .all()
            .find(|fx| fx.domain == kerf_model::Domain::Audio && fx.kind == FxKind::Effect)
            .unwrap();
        assert_eq!(plan(audio), None, "sound has no picture");
    }

    #[test]
    fn a_generator_draws_alone_on_black_with_the_key_echoed() {
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        *shared.model.lock().unwrap() = Some(
            kerf_runtime::parse_document(
                "kerf 1\nproject name=\"Bridge\"\n\
                 sequence s1 rate=24 size=1920x1080 audio=48000\n",
            )
            .unwrap(),
        );
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let request = Request {
            clip: String::new(),
            at: 0.0,
            effects: vec![
                PreviewEffect::Params {
                    fx: "solid".into(),
                    params: json!({"color": "#ff0000"}).as_object().unwrap().clone(),
                    key: Some("red".into()),
                },
                PreviewEffect::Id("title".into()),
                PreviewEffect::Id("adjustment".into()),
                PreviewEffect::Id("no.such.effect".into()),
            ],
            width: 96,
            key: Some("browser".into()),
        };
        run(&shared, 0, &request, &tx).unwrap();
        let events: Vec<Json> = rx.try_iter().collect();
        assert_eq!(events.len(), 2, "solid and title; nothing else draws alone");
        let decode = |event: &Json| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(event["rgba"].as_str().unwrap())
                .unwrap()
        };
        let solid = &events[0];
        assert_eq!(
            (solid["fx"].as_str(), solid["key"].as_str()),
            (Some("solid"), Some("red"))
        );
        assert_eq!(
            (&solid["width"], &solid["height"]),
            (&json!(96), &json!(54))
        );
        let pixels = decode(solid);
        assert_eq!(pixels.len(), 96 * 54 * 4);
        assert!(
            pixels[0] > 200 && pixels[1] < 40 && pixels[2] < 40,
            "{:?}",
            &pixels[..4]
        );
        let title = &events[1];
        assert_eq!(title["key"], "browser");
        let pixels = decode(title);
        assert!(
            pixels.chunks(4).any(|pixel| pixel[0] > 128),
            "the sample title shows"
        );
        assert!(
            pixels[..4].iter().take(3).all(|&channel| channel < 16),
            "on black"
        );

        // An effect needs the clip's picture.
        let missing = Request {
            effects: vec![PreviewEffect::Id("blur.gaussian".into())],
            ..request
        };
        assert!(run(&shared, 0, &missing, &tx).is_err());
    }

    /// A project of one clip of a colourful test picture, its media made
    /// with FFmpeg; `None` without FFmpeg.
    fn colour_bars(directory: &std::path::Path) -> Option<Project> {
        let file = directory.join("bars.mp4");
        let made = std::process::Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=24:duration=2")
            .args(["-pix_fmt", "yuv420p", "-y"])
            .arg(&file)
            .status()
            .ok()?;
        if !made.success() {
            return None;
        }
        let mut project = kerf_runtime::parse_document(&format!(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"{}\" rate=24 dur=48f video=320x180\n\
             sequence s1 rate=24 size=320x180 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=48f\n",
            file.display()
        ))
        .ok()?;
        kerf_runtime::resolve_media_paths(&mut project, directory);
        Some(project)
    }

    fn saturation(rgba: &[u8]) -> f64 {
        rgba.chunks(4)
            .map(|pixel| {
                let (max, min) = (pixel[..3].iter().max(), pixel[..3].iter().min());
                f64::from(max.unwrap() - min.unwrap())
            })
            .sum::<f64>()
            / (rgba.len() / 4) as f64
    }

    #[test]
    fn a_clips_look_with_parameters_is_drawn_from_its_picture() {
        let directory = tempfile::tempdir().unwrap();
        let Some(project) = colour_bars(directory.path()) else {
            eprintln!("skipped: FFmpeg could not make the test picture");
            return;
        };
        let shared = crate::tests_support::shared(directory.path());
        *shared.model.lock().unwrap() = Some(project);
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let grade = |saturation: f64, key: &str| PreviewEffect::Params {
            fx: "color.basic".into(),
            params: json!({"saturation": saturation})
                .as_object()
                .unwrap()
                .clone(),
            key: Some(key.into()),
        };
        let request = Request {
            clip: "c1".into(),
            at: 1.0,
            effects: vec![
                grade(1.0, "before"),
                grade(0.0, "after"),
                PreviewEffect::Params {
                    fx: "channel.invert".into(),
                    params: Map::new(),
                    key: Some("raster".into()),
                },
            ],
            width: 64,
            key: None,
        };
        run(&shared, 0, &request, &tx).unwrap();
        let events: Vec<Json> = rx.try_iter().collect();
        let keys: Vec<&str> = events.iter().filter_map(|e| e["key"].as_str()).collect();
        assert_eq!(keys, ["before", "after", "raster"]);
        let pixels = |event: &Json| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(event["rgba"].as_str().unwrap())
                .unwrap()
        };
        for event in &events {
            assert_eq!(
                (&event["width"], &event["height"]),
                (&json!(64), &json!(36))
            );
            assert_eq!(pixels(event).len(), 64 * 36 * 4);
        }
        let (before, after) = (
            saturation(&pixels(&events[0])),
            saturation(&pixels(&events[1])),
        );
        assert!(before > 40.0, "the test picture is colourful: {before}");
        assert!(after < 4.0, "saturation 0 is grey: {after}");
    }

    #[test]
    fn resampling_averages_each_pixels_area() {
        // Two by two: red, green / blue, white.
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        assert_eq!(
            resample(&rgba, (2, 2), (1, 1)).unwrap(),
            [128, 128, 128, 255]
        );
        assert_eq!(resample(&rgba, (2, 2), (2, 2)).unwrap(), rgba);
        let up = resample(&rgba, (2, 2), (4, 4)).unwrap();
        assert_eq!(&up[..4], &rgba[..4]);
        assert!(resample(&rgba, (3, 3), (1, 1)).is_none(), "too few bytes");
    }

    #[test]
    fn a_newer_request_stops_an_older_one() {
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        *shared.model.lock().unwrap() = Some(kerf_model::Project::default());
        shared.previews.store(2, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let request = Request {
            clip: String::new(),
            at: 0.0,
            effects: vec![PreviewEffect::Id("solid".into())],
            width: 64,
            key: None,
        };
        run(&shared, 1, &request, &tx).unwrap();
        assert!(rx.try_recv().is_err());
    }
}
