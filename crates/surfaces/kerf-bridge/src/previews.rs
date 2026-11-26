//! `effect_previews`: how effects, or generators, would look, as small
//! RGBA pictures for the browser's thumbnails, its hover scrub, and the
//! colour chip on a proposal card.
//!
//! An effect named by id is shown added after the clip's own effects, at
//! its defaults. One given as `{fx, params}` is shown with those
//! parameters; when the clip already has that effect, in its place, so
//! the clip's look before and after a proposal can be compared. A
//! generator is drawn alone on black. Effects run on one decoded picture
//! of a clip of footage, in the order of the stack and by the code a render
//! runs; on a clip with no file of its own (a title, a nested sequence) and
//! for generators, Kerf's compositor draws the picture, as playback does.
//! Like a frame of playback's, each of those begins on the bridge's drawers
//! (`Shared::drawers`): once the bridge is let go a batch draws no more.

use crate::event::{EffectPreview, Event};
use crate::wake::Outbox;
use crate::{emit, Shared};
use kerf_compose::{ComposeOptions, Compositor};
use kerf_model::{
    Catalog, Clip, Content, Effect, FxDef, FxKind, Id, Param, ParamType, Params, Project, Rgba,
    Sequence, Time, Track, TrackKind, Value,
};
use kerf_playback::Drawers;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use std::sync::atomic::Ordering;

/// One entry of `effects`: an id, or an id with parameters and the key
/// its preview echoes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PreviewEffect {
    /// The effect added after the clip's own, at its defaults; a generator drawn alone.
    Id(String),
    /// The effect with these static parameters (numbers kept in range, unknown names
    /// ignored), in place of the clip's own effect of that id when it has one, and the
    /// key its preview echoes.
    Params {
        fx: String,
        #[serde(default)]
        params: Map<String, Json>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Request {
    /// The clip; `""` for generators alone.
    #[serde(default)]
    pub clip: String,
    /// Sequence time, in seconds.
    #[serde(default)]
    pub at: f64,
    pub effects: Vec<PreviewEffect>,
    /// Pixels wide: at most 640, even.
    pub width: u32,
    /// Echoed on every preview of an entry that has no key of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// A picture effect on the clip's picture.
    Effect,
}

fn plan(def: &FxDef) -> Option<Plan> {
    if def.domain != kerf_model::Domain::Video {
        return None;
    }
    match def.kind {
        // An adjustment layer alone draws nothing.
        FxKind::Generator if def.id == "adjustment" => None,
        FxKind::Generator => Some(Plan::Generator),
        // Those with code of their own: an outline goes only on a shape, and a look is other
        // effects by the time it is drawn, so neither has a picture of its own to show.
        FxKind::Effect if kerf_compose::draws_effect(def.id.as_str()) => Some(Plan::Effect),
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
) -> Event {
    use base64::Engine as _;
    Event::EffectPreview(EffectPreview {
        clip: request.clip.clone(),
        fx: entry.fx().to_owned(),
        width,
        height,
        rgba: base64::engine::general_purpose::STANDARD.encode(rgba),
        key: entry.key().or(request.key.as_deref()).map(str::to_owned),
    })
}

/// The clip `id` and the sequence it is on.
fn find_clip<'a>(project: &'a Project, id: &str) -> Option<(&'a Sequence, &'a Clip)> {
    let at = project.locate(id)?;
    Some((&project.sequences[at.sequence], project.clip_at(at)))
}

/// Draw `sequence` of `project` at `at`, `width` pixels wide (even). Nothing once `drawers`
/// are closed.
fn compose(
    drawers: &Drawers,
    project: &Project,
    sequence: &Sequence,
    at: Time,
    width: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    // Asked first: with the drawers closed there is no frame to build a compositor for.
    let _drawing = drawers.begin()?;
    let compositor = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(sequence.id.to_string()),
            captions: false,
            keep_frames: true,
            frame_folder: kerf_compose::default_frame_folder(),
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
    drawers: &Drawers,
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
    compose(drawers, &project, &lone, at, width)
}

/// Generator `def` with `given` parameters alone on black, on a canvas
/// shaped like `like`, at its first frame.
fn compose_generator(
    drawers: &Drawers,
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
        buses: Vec::new(),
        id: Id::new("preview").ok()?,
        name: String::new(),
        rate,
        drop_frame: false,
        width: width_px,
        height: height_px,
        sample_rate: 48_000,
        color: Default::default(),
        start: Time::ZERO,
        tracks: vec![track],
        markers: Vec::new(),
    };
    let project = Project {
        sequences: vec![sequence.clone()],
        ..Project::default()
    };
    compose(drawers, &project, &sequence, Time::ZERO, width)
}

/// The clip's picture at `at`, decoded from its media, `width` pixels wide.
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    /// Seconds into the clip.
    clip_time: Time,
    /// The source's own size, which effect parameters scale by.
    source: (u32, u32),
}

fn decode(project: &Project, clip: &Clip, at: f64, width: u32) -> Result<Option<Picture>, String> {
    let Content::Media { media, .. } = &clip.content else {
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
    let source = clip.source_at(at).unwrap_or(Time::ZERO);
    let width = width.clamp(16, MAX_WIDTH) & !1;
    let height = ((u64::from(width) * u64::from(video.height) / u64::from(video.width.max(1)))
        as u32)
        .max(2)
        & !1;
    #[expect(
        clippy::disallowed_methods,
        reason = "short-lived and waited on: FFmpeg decodes one frame"
    )]
    let output = kerf_engine::process::ffmpeg::hidden_command(
        kerf_engine::process::ffmpeg::resolve_ffmpeg_bin(),
    )
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
        clip_time: at - clip.start,
        source: (video.width, video.height),
    }))
}

/// `picture` through the clip's effects with `effect` previewed among them.
fn on_picture(
    sequence: &Sequence,
    clip: &Clip,
    picture: &Picture,
    effect: Effect,
    replace: bool,
) -> Option<Vec<u8>> {
    let enabled: Vec<Effect> = clip
        .effects
        .iter()
        .filter(|effect| effect.enabled)
        .cloned()
        .collect();
    let stack = with_effect(&enabled, effect, replace);
    let per_source = f64::from(picture.width) / f64::from(picture.source.0.max(1));
    // A clip fits the canvas, so a source pixel is this many of the canvas's.
    let fit = (f64::from(sequence.width) / f64::from(picture.source.0.max(1)))
        .min(f64::from(sequence.height) / f64::from(picture.source.1.max(1)));
    let alone = kerf_compose::Alone {
        size: (picture.width as usize, picture.height as usize),
        at: picture.clip_time,
        per_source,
        per_canvas: per_source / fit.max(1e-6),
        rate: sequence.rate,
        duration: clip.duration,
        seed: kerf_core::raster::seed(clip.id.as_str()),
        masks: &clip.masks,
    };
    let mut frame = picture.rgba.clone();
    kerf_compose::effects_on_picture(&mut frame, &stack, &alone).ok()?;
    Some(frame)
}

/// Emit a preview of each entry of `request`. Stops when a newer request
/// than `generation` arrives, and when the bridge closes. A clip that is not
/// in the project is an error only when an effect needs its picture.
pub(crate) fn run(
    shared: &Shared,
    generation: u64,
    request: &Request,
    tx: &Outbox,
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
        if shared.closed.load(Ordering::SeqCst)
            || shared.effect_previews.load(Ordering::SeqCst) != generation
        {
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
                compose_generator(&shared.drawers, def, entry.params(), like, width)
            }
            Plan::Effect => {
                let (sequence, clip) =
                    found.ok_or_else(|| format!("no clip `{}`", request.clip))?;
                if picture.is_none() {
                    picture = Some(decode(&project, clip, request.at, width)?);
                }
                let effect = previewed(def, entry);
                match picture.as_ref().and_then(Option::as_ref) {
                    Some(picture) => on_picture(sequence, clip, picture, effect, replace)
                        .map(|rgba| (picture.width, picture.height, rgba)),
                    // A clip with no file of its own is drawn by the compositor.
                    None => {
                        let enabled: Vec<Effect> =
                            clip.effects.iter().filter(|e| e.enabled).cloned().collect();
                        let effects = with_effect(&enabled, effect, replace);
                        let at = Time::from_seconds(request.at)
                            .unwrap_or(Time::ZERO)
                            .clamp(clip.start, clip.end() - sequence.rate.frame_duration());
                        compose_clip(
                            &shared.drawers,
                            &project,
                            sequence,
                            clip,
                            effects,
                            at,
                            width,
                        )
                    }
                }
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
    fn a_newer_request_stops_an_older_one() {
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        *shared.model.lock().unwrap() = Some(kerf_model::Project::default());
        shared.effect_previews.store(2, Ordering::SeqCst);
        let (tx, rx) = crate::wake::outbox(8);
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

    #[test]
    fn a_closed_bridge_draws_no_preview() {
        let thumbnail = Request {
            clip: String::new(),
            at: 0.0,
            effects: vec![PreviewEffect::Id("solid".into())],
            width: 64,
            key: None,
        };
        let batch = |shared: &Shared| {
            *shared.model.lock().unwrap() = Some(kerf_model::Project::default());
            let (tx, rx) = crate::wake::outbox(8);
            run(shared, 0, &thumbnail, &tx).unwrap();
            rx.try_recv().is_ok()
        };
        // Open, it draws the thumbnail asked for.
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        assert!(batch(&shared));
        // Its drawers closed, as the bridge closes them when it is let go, the compositor
        // draws nothing for it.
        shared.drawers.close();
        assert!(!batch(&shared));
        // And a batch stops at a closed bridge, whatever would have drawn its pictures.
        let shared = crate::tests_support::shared(&std::env::temp_dir());
        shared.closed.store(true, Ordering::SeqCst);
        assert!(!batch(&shared));
    }
}
