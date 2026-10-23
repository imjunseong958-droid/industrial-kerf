//! One-time import of `project.cut.json` (v1 or v2) into the kernel model.
//!
//! v2 stored float seconds, modeled transitions as overlapping clips, and
//! played a video clip's embedded sound from the picture clip. The import
//! snaps times to the project frame grid, turns each overlap into a cut whose
//! overlapping material becomes handles, and moves embedded sound onto linked
//! audio clips. Anything without an equivalent is reported, never silently
//! dropped.

mod legacy;

use kerf_model::{
    validate, Align, AudioInfo, Catalog, Clip, Content, Diagnostic, Domain, Effect, FxKind, Id,
    Interp, Keyframe, Marker, MarkerColor, Media, Param, ParamType, Params, Project, Rate, Ratio,
    Sequence, Time, Track, TrackKind, Transition, Value, VideoInfo,
};
use legacy::TrackType;
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The imported project and what the import changed or left behind.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub project: Project,
    pub warnings: Vec<String>,
}

/// Import a v1 or v2 project file's JSON text.
pub fn import_json(text: &str) -> Result<Imported, Vec<String>> {
    let source = legacy::parse(text).map_err(|error| vec![error])?;
    let rate = Rate::from_fps(source.fps).map_err(|e| vec![format!("project frame rate: {e}")])?;
    let mut importer = Importer {
        source: &source,
        rate,
        catalog: Catalog::builtin(),
        names: HashSet::new(),
        media: HashMap::new(),
        warnings: Vec::new(),
        snapped: 0,
    };
    let project = importer.project();
    let mut warnings = importer.warnings;
    if importer.snapped > 0 {
        warnings.push(format!(
            "{} times were snapped to the {rate} fps frame grid",
            importer.snapped
        ));
    }
    let (errors, notes): (Vec<Diagnostic>, Vec<Diagnostic>) =
        validate(&project, Catalog::builtin())
            .into_iter()
            .partition(Diagnostic::is_error);
    warnings.extend(notes.into_iter().map(|d| d.to_string()));
    if !errors.is_empty() {
        return Err(errors.into_iter().map(|d| d.to_string()).collect());
    }
    Ok(Imported { project, warnings })
}

struct Importer<'a> {
    source: &'a legacy::Project,
    rate: Rate,
    catalog: &'a Catalog,
    names: HashSet<String>,
    media: HashMap<String, Id>,
    warnings: Vec<String>,
    snapped: usize,
}

/// A clip being imported with the transitions v2 attached to it.
struct Pending {
    clip: Clip,
    transitions: Vec<legacy::Transition>,
    volume: Option<f64>,
}

impl Importer<'_> {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// A valid, project-unique id derived from `text`.
    fn name(&mut self, text: &str, prefix: &str) -> Id {
        let mut base: String = text
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .take(Id::MAX_LEN - 4)
            .collect();
        if !base.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            base = format!("{prefix}_{base}");
            base.truncate(Id::MAX_LEN - 4);
        }
        let mut candidate = base.clone();
        let mut n = 2;
        while !self.names.insert(candidate.clone()) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        Id::new(candidate).expect("sanitized ids are valid")
    }

    /// The first free `<prefix><n>`.
    fn next(&mut self, prefix: &str) -> Id {
        let mut n = 1;
        loop {
            let candidate = format!("{prefix}{n}");
            if self.names.insert(candidate.clone()) {
                return Id::new(candidate).expect("generated ids are valid");
            }
            n += 1;
        }
    }

    fn time(&mut self, seconds: f64) -> Time {
        let exact = Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO);
        let snapped = self.rate.round(exact);
        if (snapped - exact).abs().seconds() > 1e-6 {
            self.snapped += 1;
        }
        snapped
    }

    fn project(&mut self) -> Project {
        let source = self.source;
        let (width, height) = self.canvas();
        let mut sequence = Sequence {
            id: self.next("s"),
            name: source.name.clone().unwrap_or_else(|| "Main".into()),
            rate: self.rate,
            drop_frame: false,
            width,
            height,
            sample_rate: 48_000,
            start: Time::ZERO,
            tracks: Vec::new(),
            markers: Vec::new(),
        };
        let media = self.import_media(width, height);
        let mut track_names = HashSet::new();
        // v2 played a picture clip's embedded audio only when no audio
        // track had clips.
        let explicit_audio = source
            .tracks
            .iter()
            .any(|t| t.kind == TrackType::Audio && !t.clips.is_empty());
        let mut embedded = Vec::new();
        for track in &source.tracks {
            let (imported, volumes) =
                self.import_track(track, &media, &mut track_names, &mut sequence.markers);
            if !explicit_audio && imported.kind == TrackKind::Video {
                embedded.push((sequence.tracks.len(), volumes));
            }
            sequence.tracks.push(imported);
        }
        for (index, volumes) in embedded {
            if let Some(audio) = self.embedded_audio(
                &mut sequence.tracks[index],
                &media,
                &volumes,
                &mut track_names,
            ) {
                sequence.tracks.push(audio);
            }
        }
        self.overlays(&mut sequence, &mut track_names);
        for marker in &source.markers {
            let marker = self.marker(marker);
            sequence.markers.push(marker);
        }
        sequence.markers.sort_by_key(|m| m.at);
        if !source.transitions.is_empty() || !source.effects.is_empty() {
            self.warn(
                "project-level transitions and effects have no equivalent and were not imported",
            );
        }
        if source.metadata.as_ref().is_some_and(|m| !m.is_empty()) {
            self.warn("project metadata was not imported");
        }
        Project {
            name: source.name.clone().unwrap_or_default(),
            media,
            sequences: vec![sequence],
            retired: Default::default(),
        }
    }

    fn canvas(&self) -> (u32, u32) {
        let from_project = self.source.width.zip(self.source.height);
        let from_media = self
            .source
            .sources
            .iter()
            .find_map(|s| s.width.zip(s.height));
        let (width, height) = from_project.or(from_media).unwrap_or((1920.0, 1080.0));
        (
            width.round().max(2.0) as u32,
            height.round().max(2.0) as u32,
        )
    }

    fn import_media(&mut self, width: u32, height: u32) -> Vec<Media> {
        let source = self.source;
        let mut used_on = HashMap::<&str, (bool, bool, f64)>::new();
        for track in &source.tracks {
            for clip in &track.clips {
                let Some(id) = &clip.source_id else { continue };
                let entry = used_on.entry(id.as_str()).or_default();
                match track.kind {
                    TrackType::Audio => entry.1 = true,
                    _ => entry.0 = true,
                }
                if let (Some(input), Some(duration)) = (clip.source_in, clip.timeline_duration()) {
                    let out = clip
                        .source_out
                        .unwrap_or(input + duration * clip.speed.unwrap_or(1.0).abs());
                    entry.2 = entry.2.max(out);
                }
            }
        }
        let mut assumed_audio = false;
        let mut media = Vec::new();
        for item in &source.sources {
            let id = self.name(&item.id, "m");
            self.media.insert(item.id.clone(), id.clone());
            let (on_video, on_audio, furthest) =
                used_on.get(item.id.as_str()).copied().unwrap_or_default();
            let video = match item.width.zip(item.height) {
                Some((w, h)) => Some(VideoInfo {
                    width: w.round() as u32,
                    height: h.round() as u32,
                    alpha: false,
                }),
                None if on_video || !on_audio => {
                    self.warn(format!(
                        "media `{id}` has no recorded frame size; assumed {width}x{height}"
                    ));
                    Some(VideoInfo {
                        width,
                        height,
                        alpha: false,
                    })
                }
                None => None,
            };
            // The v2 renderer played sound only from sources recording
            // `hasAudio: true`; an unrecorded source is silent.
            if item.has_audio.is_none() && !on_audio && on_video {
                self.warn(format!(
                    "media `{id}` does not record whether it has sound; it was imported as silent"
                ));
            }
            let audio = (item.has_audio == Some(true) || on_audio).then(|| {
                assumed_audio = true;
                AudioInfo {
                    channels: 2,
                    sample_rate: 48_000,
                }
            });
            let duration = match item.duration {
                Some(seconds) if seconds > 0.0 => self
                    .rate
                    .round(Time::from_seconds(seconds).unwrap_or(Time::ZERO))
                    .max(self.rate.frame_duration()),
                _ => {
                    self.warn(format!("media `{id}` has no recorded duration; assumed it ends at its furthest use"));
                    let end = Time::from_seconds(furthest).unwrap_or(Time::ZERO);
                    self.rate.time_of(self.rate.frames_floor(end) + 1)
                }
            };
            if item.subs.is_some() {
                self.warn(format!(
                    "sidecar subtitles of media `{id}` were not imported"
                ));
            }
            media.push(Media {
                id,
                path: item.path.clone(),
                rate: self.rate,
                drop_frame: false,
                start: Time::ZERO,
                duration,
                video,
                audio,
                reel: None,
            });
        }
        if assumed_audio {
            self.warn("audio formats are assumed to be 2ch@48000 until the media is probed again");
        }
        media
    }

    fn import_track(
        &mut self,
        track: &legacy::Track,
        media: &[Media],
        track_names: &mut HashSet<String>,
        markers: &mut Vec<Marker>,
    ) -> (Track, HashMap<Id, f64>) {
        let kind = match track.kind {
            TrackType::Audio => TrackKind::Audio,
            _ => TrackKind::Video,
        };
        let id = unique_track(&track.id, kind, track_names);
        let mut imported = Track::new(id.clone(), kind);
        imported.name = track.name.clone();
        imported.enabled = track.enabled.unwrap_or(true);
        imported.muted = track.muted.unwrap_or(false);
        imported.locked = track.locked.unwrap_or(false);
        let mut pending: Vec<Pending> = track
            .clips
            .iter()
            .filter_map(|clip| self.clip(track, clip, kind, media))
            .collect();
        pending.sort_by_key(|p| p.clip.start);
        let track_transitions: Vec<legacy::Transition> =
            track.transitions.iter().flatten().cloned().collect();
        let domain = kind.domain().expect("video or audio");
        let mut transitions = Vec::new();
        let mut index = 1;
        while index < pending.len() {
            let (head, tail) = pending.split_at_mut(index);
            let (a, b) = (&mut head[index - 1], &mut tail[0]);
            let overlap = a.clip.end() - b.clip.start;
            if overlap.is_positive() {
                let requested = b
                    .transitions
                    .first()
                    .or(a.transitions.first())
                    .or(track_transitions.first())
                    .cloned();
                if b.clip.start <= a.clip.start {
                    self.warn(format!(
                        "clip `{}` started with `{}` on track {id}; it was dropped",
                        a.clip.id, b.clip.id
                    ));
                    pending.remove(index - 1);
                    continue;
                }
                match requested {
                    Some(requested) => {
                        let align = match requested.alignment {
                            Some(legacy::Alignment::In) => Align::Start,
                            Some(legacy::Alignment::Out) => Align::End,
                            Some(legacy::Alignment::Center) | None => Align::Center,
                        };
                        let before = match align {
                            Align::Center => self.rate.time_of(self.rate.frames_floor(overlap) / 2),
                            Align::Start => Time::ZERO,
                            Align::End => overlap,
                        };
                        let cut = b.clip.start + before;
                        a.clip.move_tail(cut - a.clip.end());
                        b.clip.move_head(cut - b.clip.start);
                        let (from, to) = (a.clip.id.clone(), b.clip.id.clone());
                        let transition = self.transition(&requested, domain, align, overlap);
                        transitions.push(Transition {
                            from,
                            to,
                            ..transition
                        });
                    }
                    None => {
                        self.warn(format!(
                            "clip `{}` overlapped `{}` without a transition; it was shortened",
                            a.clip.id, b.clip.id
                        ));
                        a.clip.move_tail(b.clip.start - a.clip.end());
                    }
                }
            } else if !a.transitions.is_empty() || !b.transitions.is_empty() {
                self.warn(format!(
                    "the transition between `{}` and `{}` was dropped: v2 transitions need overlapping clips",
                    a.clip.id, b.clip.id
                ));
            }
            index += 1;
        }
        let volumes = pending
            .iter()
            .filter_map(|p| p.volume.map(|v| (p.clip.id.clone(), v)))
            .collect();
        imported.clips = pending.into_iter().map(|p| p.clip).collect();
        imported.transitions = transitions;
        if track.effects.as_ref().is_some_and(|e| !e.is_empty()) {
            self.warn(format!(
                "effects on track {id} have no equivalent and were not imported"
            ));
        }
        for marker in track.markers.iter().flatten() {
            let marker = self.marker(marker);
            markers.push(marker);
        }
        (imported, volumes)
    }

    fn clip(
        &mut self,
        track: &legacy::Track,
        clip: &legacy::Clip,
        kind: TrackKind,
        media: &[Media],
    ) -> Option<Pending> {
        let id = self.name(&clip.id, "c");
        let Some(duration_seconds) = clip.timeline_duration().filter(|d| *d > 0.0) else {
            self.warn(format!("clip `{id}` has no duration and was not imported"));
            return None;
        };
        let content = self.content(track, clip, &id, kind, media)?;
        let start = self.time(clip.timeline_start);
        let duration = self.time(duration_seconds).max(self.rate.frame_duration());
        let mut imported = Clip::new(id.clone(), content, start, duration);
        match kind {
            TrackKind::Video => self.visual_props(&id, clip, &mut imported.props),
            _ => {
                if let Some(volume) = clip.volume {
                    imported
                        .props
                        .insert("gain".into(), Param::Static(Value::Number(gain_db(volume))));
                }
            }
        }
        let domain = kind.domain().expect("video or audio");
        for effect in clip.effects.iter().flatten() {
            if let Some(effect) = self.effect(&id, effect, domain) {
                imported.effects.push(effect);
            }
        }
        if clip.gaps.as_ref().is_some_and(|g| !g.is_empty()) {
            self.warn(format!("gaps inside clip `{id}` were not imported"));
        }
        Some(Pending {
            clip: imported,
            transitions: clip.transitions.iter().flatten().cloned().collect(),
            volume: clip.volume,
        })
    }

    /// What a v2 clip shows, or `None` when it shows nothing importable.
    fn content(
        &mut self,
        track: &legacy::Track,
        clip: &legacy::Clip,
        id: &Id,
        kind: TrackKind,
        media: &[Media],
    ) -> Option<Content> {
        let text = clip.text.clone().or_else(|| match &clip.title {
            Some(Json::String(text)) => Some(text.clone()),
            _ => None,
        });
        Some(match (track.kind, &clip.source_id, text) {
            (TrackType::Adjustment, ..) => generator("adjustment", Params::new()),
            (TrackType::Title, _, Some(text)) | (_, None, Some(text)) => generator(
                "title",
                Params::from([("text".to_owned(), Param::Static(Value::Text(text)))]),
            ),
            (_, Some(source), _) => {
                let Some(media_id) = self.media.get(source).cloned() else {
                    self.warn(format!(
                        "clip `{id}` uses unknown source `{source}` and was not imported"
                    ));
                    return None;
                };
                let speed = clip.speed.unwrap_or(1.0);
                let ratio = Ratio::parse(
                    format!("{speed:.4}")
                        .trim_end_matches('0')
                        .trim_end_matches('.'),
                )
                .or_else(|_| Ratio::new(speed.round().clamp(-100.0, 100.0) as i32, 1));
                let ratio = match ratio {
                    Ok(ratio) => {
                        if (ratio.value() - speed).abs() > 1e-6 {
                            self.warn(format!("clip `{id}` speed {speed} became {ratio}"));
                        }
                        ratio
                    }
                    Err(_) => {
                        self.warn(format!(
                            "clip `{id}` speed {speed} is unsupported; it plays at 1"
                        ));
                        Ratio::ONE
                    }
                };
                let domain = kind.domain().expect("video or audio");
                if !media.iter().any(|m| m.id == media_id && m.supports(domain)) {
                    self.warn(format!(
                        "clip `{id}` needs {} its media lacks; it was not imported",
                        domain.name()
                    ));
                    return None;
                }
                Content::Media {
                    media: media_id,
                    source_in: self.time(clip.source_in.unwrap_or(0.0)),
                    speed: ratio,
                }
            }
            (_, None, None) => {
                self.warn(format!(
                    "clip `{id}` has neither media nor text and was not imported"
                ));
                return None;
            }
        })
    }

    /// The transition a v2 overlap asked for. `from` and `to` are filled in
    /// by the caller.
    fn transition(
        &mut self,
        requested: &legacy::Transition,
        domain: Domain,
        align: Align,
        duration: Time,
    ) -> Transition {
        let unset = Id::new("_").expect("valid id");
        let id = match &requested.id {
            Some(id) => self.name(id, "t"),
            None => self.next("t"),
        };
        let kind = requested.kind.as_str();
        let def = self
            .catalog
            .fx(kind)
            .filter(|def| def.kind == FxKind::Transition && def.domain == domain);
        let (fx, params) = match def {
            Some(def) => {
                let mut params = Params::new();
                for (name, raw) in requested.params.iter().flatten() {
                    match def.find(name).and_then(|param| json_value(param, raw)) {
                        Some(value) => {
                            params.insert(name.clone(), Param::Static(value));
                        }
                        None => self.warn(format!(
                            "`{kind}` parameter `{name}` on transition `{id}` was not imported"
                        )),
                    }
                }
                (def.id.clone(), params)
            }
            None => {
                if !matches!(kind, "fade" | "crossfade" | "dissolve") {
                    self.warn(format!("transition `{kind}` became a dissolve"));
                }
                let fx = if domain == Domain::Video {
                    "dissolve"
                } else {
                    "crossfade"
                };
                (Id::new(fx).expect("catalog id"), Params::new())
            }
        };
        Transition {
            id,
            from: unset.clone(),
            to: unset,
            fx,
            duration,
            align,
            params,
        }
    }

    fn visual_props(&mut self, id: &Id, clip: &legacy::Clip, props: &mut Params) {
        let mut set = |this: &mut Self, name: &str, value: f64| {
            let def = this
                .catalog
                .prop(Domain::Video, name)
                .expect("video property");
            let clamped = value.clamp(def.min.unwrap_or(f64::MIN), def.max.unwrap_or(f64::MAX));
            if clamped != value {
                this.warn(format!(
                    "clip `{id}` {name} {value} was clamped to {clamped}"
                ));
            }
            if Some(clamped) != def.default.as_number() {
                props.insert(name.to_owned(), Param::Static(Value::Number(clamped)));
            }
        };
        if let Some(opacity) = clip.opacity {
            set(self, "opacity", opacity);
        }
        if let Some(crop) = &clip.crop {
            let edge = |value: f64| (value * 1e6).round() / 1e6;
            set(self, "crop_l", edge(crop.x));
            set(self, "crop_t", edge(crop.y));
            set(self, "crop_r", edge(1.0 - crop.x - crop.width));
            set(self, "crop_b", edge(1.0 - crop.y - crop.height));
        }
        if let Some(transform) = &clip.transform {
            let position = |axis: &str| match &transform.position {
                Some(Json::Object(map)) => map.get(axis).and_then(Json::as_f64),
                _ => None,
            };
            for (name, value) in [
                ("x", transform.x.or_else(|| position("x"))),
                ("y", transform.y.or_else(|| position("y"))),
                ("scale", transform.scale),
                ("scale_x", transform.scale_x),
                ("scale_y", transform.scale_y),
                ("rotation", transform.rotation),
                ("anchor_x", transform.anchor_x),
                ("anchor_y", transform.anchor_y),
            ] {
                if let Some(value) = value {
                    set(self, name, value);
                }
            }
            if transform.z.is_some()
                || transform.rotation_x.is_some()
                || transform.rotation_y.is_some()
                || position("z").is_some()
            {
                self.warn(format!("clip `{id}` 3D transform values were not imported"));
            }
        }
        if let Some(blend) = clip.blend_mode.as_ref().or(clip.blend.as_ref()) {
            let def = self
                .catalog
                .prop(Domain::Video, "blend")
                .expect("blend property");
            let value = Value::Enum(blend.to_lowercase());
            if def.check_value(&value).is_ok() {
                if value != def.default {
                    props.insert("blend".into(), Param::Static(value));
                }
            } else {
                self.warn(format!("clip `{id}` blend mode `{blend}` is unsupported"));
            }
        }
        let mut animated: BTreeMap<&'static str, Vec<Keyframe>> = BTreeMap::new();
        for key in clip.keyframes.iter().flatten() {
            let Some(name) = keyframe_property(&key.property) else {
                self.warn(format!(
                    "clip `{id}` keyframes on `{}` were not imported",
                    key.property
                ));
                continue;
            };
            let Some(value) = key.value.as_f64() else {
                self.warn(format!(
                    "clip `{id}` has a non-numeric `{}` keyframe",
                    key.property
                ));
                continue;
            };
            let interp = match key.easing.as_deref().map(str::to_lowercase).as_deref() {
                None | Some("linear") => Interp::Linear,
                Some("hold" | "step") => Interp::Hold,
                Some("easein" | "ease-in" | "ease_in") => Interp::EaseIn,
                Some("easeout" | "ease-out" | "ease_out") => Interp::EaseOut,
                Some("ease" | "easeinout" | "ease-in-out" | "ease_in_out") => Interp::Ease,
                Some(other) => {
                    self.warn(format!("clip `{id}` easing `{other}` became linear"));
                    Interp::Linear
                }
            };
            let at = self.time(key.time);
            let keys = animated.entry(name).or_default();
            keys.retain(|k| k.at != at);
            keys.push(Keyframe {
                at,
                value: Value::Number(value),
                interp,
            });
        }
        for (name, mut keys) in animated {
            keys.sort_by_key(|k| k.at);
            props.insert(name.to_owned(), Param::Animated(keys));
        }
    }

    fn effect(&mut self, id: &Id, effect: &legacy::Effect, domain: Domain) -> Option<Effect> {
        let enabled = effect.enabled.unwrap_or(true);
        if effect.kind.eq_ignore_ascii_case("color") {
            if domain != Domain::Video {
                self.warn(format!("color correction on audio clip `{id}` was dropped"));
                return None;
            }
            let def = self.catalog.fx("color.basic").expect("color.basic");
            let mut params = Params::new();
            for (name, raw) in effect.params.iter().flatten() {
                match (def.find(name), raw.as_f64()) {
                    (Some(param), Some(value)) => {
                        if param.default.as_number() != Some(value) {
                            params.insert(name.clone(), Param::Static(Value::Number(value)));
                        }
                    }
                    _ => self.warn(format!(
                        "color parameter `{name}` on clip `{id}` was not imported"
                    )),
                }
            }
            return Some(Effect {
                fx: def.id.clone(),
                enabled,
                params,
            });
        }
        let def = self
            .catalog
            .fx(&effect.kind)
            .filter(|d| d.kind == FxKind::Effect && d.domain == domain);
        let Some(def) = def else {
            self.warn(format!(
                "effect `{}` on clip `{id}` has no equivalent and was not imported",
                effect.kind
            ));
            return None;
        };
        let mut params = Params::new();
        for (name, raw) in effect.params.iter().flatten() {
            match def.find(name).and_then(|param| json_value(param, raw)) {
                Some(value) => {
                    params.insert(name.clone(), Param::Static(value));
                }
                None => self.warn(format!(
                    "`{}` parameter `{name}` on clip `{id}` was not imported",
                    effect.kind
                )),
            }
        }
        Some(Effect {
            fx: def.id.clone(),
            enabled,
            params,
        })
    }

    /// v2 played a picture clip's own sound when no audio track had clips.
    /// Give that sound linked clips on a new audio track.
    fn embedded_audio(
        &mut self,
        video: &mut Track,
        media: &[Media],
        volumes: &HashMap<Id, f64>,
        track_names: &mut HashSet<String>,
    ) -> Option<Track> {
        let has_sound = |clip: &Clip| match &clip.content {
            Content::Media { media: id, .. } => {
                media.iter().any(|m| m.id == *id && m.audio.is_some())
            }
            _ => false,
        };
        if !video.clips.iter().any(has_sound) {
            return None;
        }
        let mut audio = Track::new(
            unique_track("A1", TrackKind::Audio, track_names),
            TrackKind::Audio,
        );
        audio.muted = video.muted;
        audio.enabled = video.enabled;
        let mut partners = HashMap::new();
        for clip in video.clips.iter_mut().filter(|c| has_sound(c)) {
            let link = self.next("L");
            let mut sound = Clip::new(
                self.name(&format!("{}_a", clip.id), "c"),
                clip.content.clone(),
                clip.start,
                clip.duration,
            );
            if let Some(volume) = volumes.get(&clip.id) {
                sound.props.insert(
                    "gain".into(),
                    Param::Static(Value::Number(gain_db(*volume))),
                );
            }
            sound.link = Some(link.clone());
            clip.link = Some(link);
            partners.insert(clip.id.clone(), sound.id.clone());
            audio.clips.push(sound);
        }
        for transition in &video.transitions {
            if let (Some(from), Some(to)) =
                (partners.get(&transition.from), partners.get(&transition.to))
            {
                audio.transitions.push(Transition {
                    id: self.next("t"),
                    from: from.clone(),
                    to: to.clone(),
                    fx: Id::new("crossfade").expect("catalog id"),
                    duration: transition.duration,
                    align: transition.align,
                    params: Params::new(),
                });
            }
        }
        Some(audio)
    }

    fn overlays(&mut self, sequence: &mut Sequence, track_names: &mut HashSet<String>) {
        for overlay in &self.source.overlays {
            let Some(cues) = &overlay.cues else {
                match &overlay.path {
                    Some(path) => self.warn(format!(
                        "subtitle file `{path}` was not imported; bring it in as a caption track"
                    )),
                    None => self.warn("an overlay without an equivalent was not imported"),
                }
                continue;
            };
            let mut track = Track::new(
                unique_track("S1", TrackKind::Subtitle, track_names),
                TrackKind::Subtitle,
            );
            let window_start = overlay.start.unwrap_or(0.0);
            let window_end = overlay.duration.map_or(f64::INFINITY, |d| window_start + d);
            for cue in cues {
                let (start, end) = (cue.start.max(window_start), cue.end.min(window_end));
                let (start, end) = (self.time(start), self.time(end));
                if end <= start || cue.text.trim().is_empty() {
                    continue;
                }
                if let Some(last) = track.clips.last_mut() {
                    if last.end() > start {
                        let trim = start - last.end();
                        if (last.duration + trim).is_positive() {
                            last.move_tail(trim);
                            self.warn(format!(
                                "caption `{}` overlapped the next cue and was shortened",
                                last.id
                            ));
                        } else {
                            self.warn(format!(
                                "caption `{}` was hidden by the next cue and dropped",
                                last.id
                            ));
                            track.clips.pop();
                        }
                    }
                }
                let id = match &cue.id {
                    Some(id) => self.name(id, "cap"),
                    None => self.next("cap"),
                };
                track.clips.push(Clip::new(
                    id,
                    Content::Caption {
                        text: cue.text.clone(),
                    },
                    start,
                    end - start,
                ));
            }
            if overlay.style.is_some() {
                let name = overlay.id.as_deref().unwrap_or("captions");
                self.warn(format!("caption style of `{name}` was not imported"));
            }
            sequence.tracks.push(track);
        }
    }

    fn marker(&mut self, marker: &legacy::Marker) -> Marker {
        let color = marker
            .color
            .as_deref()
            .and_then(|c| MarkerColor::parse(&c.to_lowercase()));
        if marker.color.is_some() && color.is_none() {
            self.warn(format!(
                "marker `{}` color is not in the marker palette",
                marker.id
            ));
        }
        Marker {
            id: self.name(&marker.id, "mk"),
            at: self.time(marker.time),
            duration: self.time(marker.duration.unwrap_or(0.0)),
            name: marker.label.clone(),
            color,
            note: None,
        }
    }
}

fn generator(fx: &str, params: Params) -> Content {
    Content::Generator {
        fx: Id::new(fx).expect("catalog id"),
        params,
    }
}

fn unique_track(text: &str, kind: TrackKind, taken: &mut HashSet<String>) -> Id {
    let prefix = match kind {
        TrackKind::Video => "V",
        TrackKind::Audio => "A",
        TrackKind::Subtitle => "S",
    };
    let mut candidate = if Id::is_valid(text) {
        text.to_owned()
    } else {
        format!("{prefix}1")
    };
    let mut n = 1;
    while !taken.insert(candidate.clone()) {
        n += 1;
        candidate = format!("{prefix}{n}");
    }
    Id::new(candidate).expect("valid track id")
}

fn gain_db(volume: f64) -> f64 {
    if volume <= 0.0 {
        -96.0
    } else {
        let db = 20.0 * volume.log10();
        (db * 100.0).round().clamp(-9600.0, 2400.0) / 100.0
    }
}

/// A v2 parameter value read as `def` expects.
fn json_value(def: &kerf_model::ParamDef, raw: &Json) -> Option<Value> {
    match (&def.ty, raw) {
        (ParamType::Number, raw) => raw.as_f64().map(Value::Number),
        (ParamType::Bool, Json::Bool(b)) => Some(Value::Bool(*b)),
        (ParamType::Text, Json::String(s)) => Some(Value::Text(s.clone())),
        (ParamType::Enum(_), Json::String(s)) => Some(Value::Enum(s.clone())),
        (ParamType::Color, Json::String(s)) => kerf_model::Rgba::parse(s).map(Value::Color),
        _ => None,
    }
}

fn keyframe_property(property: &str) -> Option<&'static str> {
    let normalized: String = property
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    Some(match normalized.as_str() {
        "opacity" => "opacity",
        "scale" | "transform.scale" | "transform.scaleuniform" => "scale",
        "scalex" | "transform.scalex" => "scale_x",
        "scaley" | "transform.scaley" => "scale_y",
        "x" | "position.x" | "transform.x" | "transform.position.x" => "x",
        "y" | "position.y" | "transform.y" | "transform.position.y" => "y",
        "rotation" | "transform.rotation" => "rotation",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
