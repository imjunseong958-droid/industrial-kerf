//! Building the timeline view ([`TimelineView`]) editor snapshots carry, and
//! the catalog of effects in the same terms: parameter definitions as
//! [`ViewParamDef`]s, and values as JSON.

use crate::{
    TimelineView, ViewClip, ViewEffect, ViewFxDef, ViewMarker, ViewMask, ViewOutline,
    ViewOutlineKey, ViewParamDef, ViewSequence, ViewSource, ViewTake, ViewTrack, ViewTrackKind,
    ViewTransition,
};
use kerf_model::{
    Catalog, Clip, Content, Domain, FxKind, Mask, MaskMode, MaskShape, Media, Param, ParamDef,
    ParamType, Params, Project, Rate, Sequence, Time, Track, TrackKind, Value,
};
use serde_json::{json, Map, Value as Json};
use std::collections::BTreeMap;

/// The view as the JSON a snapshot carries.
pub(crate) fn to_json(view: &TimelineView) -> Json {
    serde_json::to_value(view).unwrap_or_default()
}

/// [`timeline_view`] with each source's takes and the cast and styles it shows.
pub(crate) fn timeline_with_takes(project: &Project, directory: &std::path::Path) -> TimelineView {
    let mut view = timeline_view(project);
    for (source, media) in view.sources.iter_mut().zip(&project.media) {
        source.takes = kerf_engine::recipe::takes::listed(directory, media)
            .into_iter()
            .map(|(number, path, chosen)| ViewTake {
                number,
                path,
                chosen,
            })
            .collect();
        let (cast, styles) = kerf_engine::recipe::cast::named_by(media);
        source.shows = [cast, styles].concat();
    }
    view
}

/// The most points a signal's curve is sent as: a long clip is read every few frames.
const SIGNAL_POINTS: usize = 512;

/// The timeline view of `project`, without takes.
pub(crate) fn timeline_view(project: &Project) -> TimelineView {
    let mut view = TimelineView {
        name: project.name.clone(),
        sources: project.media.iter().map(source_view).collect(),
        sequences: project.sequences.iter().map(sequence_view).collect(),
        properties: clip_properties(),
        mask_properties: Catalog::builtin()
            .mask_props()
            .iter()
            .map(param_def_view)
            .collect(),
        mask_modes: MaskMode::ALL.map(|mode| mode.name().to_owned()).into(),
        generators: Catalog::builtin()
            .all()
            .filter(|def| def.kind == FxKind::Generator)
            .map(|def| {
                let params = def.params.iter().map(param_def_view).collect();
                (def.id.to_string(), params)
            })
            .collect(),
        fps: 30.0,
        ..TimelineView::default()
    };
    let Some(sequence) = project.sequences.first() else {
        return view;
    };
    let rate = sequence.rate;
    view.sequence = Some(sequence.id.as_str().to_owned());
    view.fps = rate.fps();
    view.rate = Some(rate.to_string());
    view.drop_frame = Some(sequence.drop_frame);
    view.sample_rate = Some(sequence.sample_rate);
    view.width = Some(sequence.width);
    view.height = Some(sequence.height);
    view.start = Some(sequence.start.seconds());
    view.duration = sequence.duration().seconds();
    view.tracks = sequence
        .tracks
        .iter()
        .map(|track| track_view(project, sequence, track))
        .collect();
    view.markers = sequence
        .markers
        .iter()
        .map(|marker| ViewMarker {
            id: marker.id.as_str().to_owned(),
            time: marker.at.seconds(),
            frame: frames(rate, marker.at),
            duration: marker.duration.seconds(),
            label: marker.name.clone(),
            color: marker.color.map(|color| color.name().to_owned()),
            note: marker.note.clone(),
        })
        .collect();
    view
}

fn source_view(media: &Media) -> ViewSource {
    ViewSource {
        id: media.id.as_str().to_owned(),
        path: media.path.clone(),
        duration: media.duration.seconds(),
        rate: media.rate.to_string(),
        fps: media.rate.fps(),
        has_audio: media.audio.is_some(),
        width: media.video.map(|video| video.width),
        height: media.video.map(|video| video.height),
        channels: media.audio.map(|audio| audio.channels),
        sample_rate: media.audio.map(|audio| audio.sample_rate),
        recipe: media.recipe.clone(),
        steps: media.steps.len(),
        takes: Vec::new(),
        shows: Vec::new(),
    }
}

fn sequence_view(sequence: &Sequence) -> ViewSequence {
    ViewSequence {
        id: sequence.id.as_str().to_owned(),
        name: sequence.name.clone(),
        width: sequence.width,
        height: sequence.height,
        rate: sequence.rate.to_string(),
        fps: sequence.rate.fps(),
        drop_frame: sequence.drop_frame,
        duration: sequence.duration().seconds(),
        color: sequence.color.id().to_owned(),
    }
}

/// The properties a clip has by its track's type, `video` and `audio`, as a timeline's
/// `properties` carries them.
pub fn clip_properties() -> BTreeMap<String, Vec<ViewParamDef>> {
    [Domain::Video, Domain::Audio]
        .into_iter()
        .map(|domain| {
            let defs = Catalog::builtin().props(domain).iter();
            (domain.name().to_owned(), defs.map(param_def_view).collect())
        })
        .collect()
}

fn frames(rate: Rate, time: Time) -> i64 {
    rate.frames_round(time)
}

fn track_view(project: &Project, sequence: &Sequence, track: &Track) -> ViewTrack {
    let rate = sequence.rate;
    ViewTrack {
        id: track.id.as_str().to_owned(),
        kind: match track.kind {
            TrackKind::Video => ViewTrackKind::Video,
            TrackKind::Audio => ViewTrackKind::Audio,
            TrackKind::Subtitle => ViewTrackKind::Subtitle,
        },
        enabled: track.enabled,
        locked: track.locked,
        muted: track.muted,
        role: track.role_or_default().map(str::to_owned),
        role_assigned: track.role.is_some(),
        sync_locked: track.sync_locked,
        bus: track.bus.as_ref().map(|bus| bus.as_str().to_owned()),
        clips: track
            .clips
            .iter()
            .map(|clip| clip_view(project, rate, clip))
            .collect(),
        transitions: track
            .transitions
            .iter()
            .map(|transition| {
                let cut = track
                    .clip(transition.from.as_str())
                    .map_or(Time::ZERO, Clip::end);
                let (before, after) = transition.split_at_cut(rate);
                ViewTransition {
                    id: transition.id.as_str().to_owned(),
                    from: transition.from.as_str().to_owned(),
                    to: transition.to.as_str().to_owned(),
                    fx: transition.fx.as_str().to_owned(),
                    version: transition.version,
                    align: transition.align.name().to_owned(),
                    start: (cut - before).seconds(),
                    end: (cut + after).seconds(),
                    duration: transition.duration.seconds(),
                }
            })
            .collect(),
        name: track.name.clone(),
    }
}

fn clip_view(project: &Project, rate: Rate, clip: &Clip) -> ViewClip {
    let mut view = ViewClip {
        id: clip.id.as_str().to_owned(),
        timeline_start: clip.start.seconds(),
        duration: clip.duration.seconds(),
        start_frame: frames(rate, clip.start),
        duration_frames: frames(rate, clip.duration),
        enabled: clip.enabled,
        name: clip.name.clone(),
        link: clip.link.as_ref().map(|link| link.as_str().to_owned()),
        parent: clip
            .parent
            .as_ref()
            .map(|parent| parent.as_str().to_owned()),
        ..ViewClip::default()
    };
    if let Some((source_in, source_out)) = clip.source_range() {
        let speed = clip.speed();
        view.source_in = Some(source_in.seconds());
        view.source_out = Some(source_out.seconds());
        view.speed = Some(speed.average(clip.duration));
        view.speed_ratio = Some(speed.to_string());
        view.reverse = speed.is_reverse();
    }
    let mut params = None;
    match &clip.content {
        Content::Media { media, .. } => {
            view.kind = "media".into();
            view.source_id = Some(media.as_str().to_owned());
            view.source_rate = project
                .media(media.as_str())
                .map(|media| media.rate.to_string());
        }
        Content::Sequence { sequence, .. } => {
            view.kind = "sequence".into();
            view.sequence_id = Some(sequence.as_str().to_owned());
            // By the name it goes by, or its id with none.
            view.name = view.name.take().or_else(|| {
                let nested = project
                    .sequences
                    .iter()
                    .find(|nested| nested.id == *sequence);
                let name = nested
                    .map(|nested| nested.name.trim())
                    .filter(|name| !name.is_empty());
                Some(name.unwrap_or(sequence.as_str()).to_owned())
            });
        }
        Content::Generator {
            fx,
            version,
            params: generator,
        } => {
            view.kind = "generator".into();
            view.fx = Some(fx.as_str().to_owned());
            view.version = Some(*version);
            view.name = view.name.take().or_else(|| match generator.get("text") {
                Some(Param::Static(Value::Text(text))) => Some(text.clone()),
                _ => Some(fx.to_string()),
            });
            // A signal draws nothing, so what there is to show of it is its number: what
            // it comes to along the clip, for an editor to draw as a curve.
            if fx.as_str() == kerf_model::SIGNAL {
                view.signal = signal(generator, rate, clip);
            }
            params = Some(generator);
        }
        Content::Caption { text } => {
            view.kind = "caption".into();
            view.text = Some(text.clone());
            view.name = view.name.take().or_else(|| Some(text.clone()));
        }
    }
    view.props = clip
        .props
        .iter()
        .chain(params.into_iter().flatten())
        .map(|(name, param)| (name.clone(), param_json(param, rate)))
        .collect();
    view.effects = clip
        .effects
        .iter()
        .map(|effect| ViewEffect {
            fx: effect.fx.as_str().to_owned(),
            version: effect.version,
            enabled: effect.enabled,
            doc: Catalog::builtin()
                .fx(effect.fx.as_str())
                .map(|def| def.doc.clone()),
            params: params_json(&effect.params, rate),
        })
        .collect();
    view.masks = clip
        .masks
        .iter()
        .map(|mask| mask_view(mask, rate))
        .collect();
    view
}

/// A signal's number on frames along the clip, at most [`SIGNAL_POINTS`] and its last frame.
fn signal(params: &Params, rate: Rate, clip: &Clip) -> Vec<f64> {
    let length = frames(rate, clip.duration).max(1);
    let step = (length as usize).div_ceil(SIGNAL_POINTS).max(1);
    let read = |frame: i64| match params
        .get(kerf_model::SIGNAL_VALUE)
        .map(|value| value.value_at(rate.time_of(frame)))
    {
        Some(Value::Number(number)) if number.is_finite() => {
            (number.clamp(0.0, 1.0) * 1e4).round() / 1e4
        }
        _ => 0.0,
    };
    let mut values: Vec<f64> = (0..length).step_by(step).map(read).collect();
    // Its last frame too, so that the curve ends where the clip does.
    if (length - 1) % step as i64 != 0 {
        values.push(read(length - 1));
    }
    values
}

fn mask_view(mask: &Mask, rate: Rate) -> ViewMask {
    let (outline, keys) = match &mask.shape {
        // Where it starts, as a still outline reads, and its keyframes.
        MaskShape::Moving(keys) => (
            keys.first()
                .map_or_else(ViewOutline::default, |first| outline_view(&first.shape)),
            Some(
                keys.iter()
                    .map(|key| ViewOutlineKey {
                        at: key.at.seconds(),
                        frame: frames(rate, key.at),
                        interp: key.interp.name().into_owned(),
                        outline: outline_view(&key.shape),
                    })
                    .collect(),
            ),
        ),
        still => (outline_view(still), None),
    };
    ViewMask {
        name: mask.name.as_str().to_owned(),
        mode: mask.mode.name().to_owned(),
        inverted: mask.inverted,
        params: params_json(&mask.params, rate),
        outline,
        keys,
    }
}

fn outline_view(shape: &MaskShape) -> ViewOutline {
    match shape {
        MaskShape::Rect(rect) => ViewOutline {
            rect: Some(*rect),
            ..ViewOutline::default()
        },
        MaskShape::Ellipse(ellipse) => ViewOutline {
            ellipse: Some(*ellipse),
            ..ViewOutline::default()
        },
        MaskShape::Polygon {
            points,
            closed,
            soft,
        } => ViewOutline {
            points: Some(points.clone()),
            closed: Some(*closed),
            soft: soft.clone(),
            ..ViewOutline::default()
        },
        MaskShape::Moving(_) => ViewOutline::default(),
    }
}

fn params_json(params: &Params, rate: Rate) -> Map<String, Json> {
    params
        .iter()
        .map(|(name, param)| (name.clone(), param_json(param, rate)))
        .collect()
}

/// The whole catalog: picture and sound effects, transitions, and
/// generators (titles and other pictures a clip makes on its own, `kind`
/// `generator`), each with its display name, when to use it, what renders
/// it, its parameter definitions and its `domain` (`video` or `audio`).
pub fn fx_catalog_all() -> Vec<ViewFxDef> {
    Catalog::builtin().all().map(fx_def_view).collect()
}

fn fx_def_view(fx: &kerf_model::FxDef) -> ViewFxDef {
    let name = fx
        .doc
        .split_once(": ")
        .map_or_else(|| fx.doc.trim_end_matches('.'), |(name, _)| name);
    ViewFxDef {
        id: fx.id.as_str().to_owned(),
        kind: fx.kind.name().to_owned(),
        category: fx.category.clone(),
        name: name.to_owned(),
        doc: fx.doc.clone(),
        engine: fx.engine.name().to_owned(),
        params: fx.params.iter().map(param_def_view).collect(),
        domain: fx.domain.name().to_owned(),
    }
}

fn param_def_view(def: &ParamDef) -> ViewParamDef {
    let (options, extensions) = match &def.ty {
        ParamType::Enum(options) => (Some(options.clone()), None),
        ParamType::File(extensions) => (None, Some(extensions.clone())),
        _ => (None, None),
    };
    ViewParamDef {
        name: def.name.clone(),
        kind: def.ty.name().to_owned(),
        default: value_json(&def.default),
        animatable: def.animatable,
        doc: def.doc.clone(),
        min: def.min,
        max: def.max,
        unit: def.unit.clone(),
        options,
        extensions,
    }
}

fn param_json(param: &Param, rate: Rate) -> Json {
    match param {
        Param::Static(value) => value_json(value),
        // The editor has numbers and keyframes: an expression shows as what it gives at the
        // clip's first frame, and setting it there replaces it.
        Param::Expr(_) => value_json(&param.value_at(Time::ZERO)),
        Param::Animated(keys) => keys
            .iter()
            .map(|key| {
                json!({
                    "at": key.at.seconds(),
                    "frame": frames(rate, key.at),
                    "value": value_json(&key.value),
                    "interp": key.interp.name(),
                })
            })
            .collect(),
    }
}

fn value_json(value: &Value) -> Json {
    match value {
        Value::Number(n) => json!(n),
        Value::Bool(b) => json!(b),
        Value::Text(text) | Value::Enum(text) => json!(text),
        Value::Color(color) => json!(color.to_string()),
    }
}
