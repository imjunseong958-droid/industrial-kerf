//! One-time import of `project.cut.json` (v1 or v2) into the kernel model.
//!
//! v2 stored float seconds, modeled transitions as overlapping clips, and
//! played a video clip's embedded sound from the picture clip. The import
//! snaps times to the project frame grid, turns each overlap into a cut whose
//! overlapping material becomes handles, and moves embedded sound onto linked
//! audio clips. Anything without an equivalent is reported, never silently
//! dropped.
//!
//! `cutd` also uses this import, with the current model as a hint, to bring
//! edits made through the temporary v2 export (see `export`) back into the
//! model. Whatever v2 cannot express, and whatever the edit left alone, is
//! taken from the hint exactly.

mod export;

pub use export::{export, export_json, exported_span, kernel_track};

use cut_domain as v2;
use cut_model::{
    validate, Align, AudioInfo, Catalog, Clip, Content, Diagnostic, Domain, Effect, FxKind, Id,
    Interp, Keyframe, Marker, MarkerColor, Media, Param, ParamType, Params, Project, Rate, Ratio,
    Sequence, Time, Track, TrackKind, Transition, Value, VideoInfo,
};
use export::{exported_transitions, hidden_clips, Forms, Lane};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The imported project and everything that changed or was left behind.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub project: Project,
    pub warnings: Vec<String>,
    /// The warnings that describe data left behind rather than adjusted.
    pub dropped: Vec<String>,
}

/// Import a v1 or v2 project file's JSON text.
pub fn import_json(text: &str) -> Result<Imported, Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| vec![e.to_string()])?;
    let project = match v2::parse_versioned(&value).map_err(|e| vec![e.to_string()])? {
        v2::VersionedProject::V1(legacy) => {
            v2::migrate_v1_to_v2(&legacy).map_err(|e| vec![e.to_string()])?
        }
        v2::VersionedProject::V2(project) => project,
    };
    import_v2(&project)
}

pub fn import_v2(source: &v2::Project) -> Result<Imported, Vec<String>> {
    import_v2_with(source, None)
}

/// Import `source`, taking anything it cannot express or did not change
/// from `hint`, the model it was exported from. Ids the hint retired stay
/// retired, and ids the edit removed are retired.
pub fn import_v2_with(
    source: &v2::Project,
    hint: Option<&Project>,
) -> Result<Imported, Vec<String>> {
    let hint = hint.and_then(Hint::new);
    let rate = match &hint {
        Some(hint) if hint.forms.fps == source.fps => hint.forms.rate,
        _ => Rate::from_fps(source.fps).map_err(|e| vec![format!("project frame rate: {e}")])?,
    };
    // Retired ids and the ids of clips and transitions v2 cannot show stay
    // out of reach of ids generated during the import.
    let names = hint
        .as_ref()
        .map(|hint| {
            let sequence = hint.project.sequences.first();
            let tracks = sequence.into_iter().flat_map(|s| &s.tracks);
            let hidden = tracks.flat_map(|track| {
                let exported: HashSet<&Id> = exported_transitions(track).map(|t| &t.id).collect();
                let clips = hidden_clips(track);
                let transitions = track
                    .transitions
                    .iter()
                    .filter(move |t| !exported.contains(&t.id))
                    .map(|t| &t.id);
                clips.map(|c| &c.id).chain(transitions)
            });
            hint.project
                .retired
                .iter()
                .chain(hidden)
                .map(|id| id.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut importer = Importer {
        source,
        rate,
        catalog: Catalog::builtin(),
        hint,
        names,
        media: HashMap::new(),
        warnings: Vec::new(),
        dropped: Vec::new(),
        snapped: 0,
    };
    let project = importer.project();
    let dropped = importer.dropped;
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
    Ok(Imported {
        project,
        warnings,
        dropped,
    })
}

/// The model an edited v2 project was exported from, indexed by id.
struct Hint<'a> {
    project: &'a Project,
    forms: Forms,
    /// Clips in their v2 form (extended by transition handles).
    clips: HashMap<&'a str, Clip>,
    transitions: HashMap<&'a str, &'a Transition>,
    media: HashMap<&'a str, &'a Media>,
    markers: HashMap<&'a str, &'a Marker>,
    tracks: HashMap<&'a str, (usize, &'a Track)>,
}

impl<'a> Hint<'a> {
    fn new(project: &'a Project) -> Option<Self> {
        let forms = Forms::of(project)?;
        let sequence = project.sequences.first()?;
        let mut hint = Self {
            project,
            forms,
            clips: HashMap::new(),
            transitions: HashMap::new(),
            media: project.media.iter().map(|m| (m.id.as_str(), m)).collect(),
            markers: sequence
                .markers
                .iter()
                .map(|m| (m.id.as_str(), m))
                .collect(),
            tracks: HashMap::new(),
        };
        for (index, track) in sequence.tracks.iter().enumerate() {
            hint.tracks.insert(track.id.as_str(), (index, track));
            let forms = export::v2_forms(sequence, track);
            for (clip, form) in track.clips.iter().zip(forms) {
                hint.clips.insert(clip.id.as_str(), form);
            }
            for transition in &track.transitions {
                hint.transitions.insert(transition.id.as_str(), transition);
            }
        }
        Some(hint)
    }

    /// Whether the v2 clip's `keys` hold what the hint exported for `id`.
    fn same(&self, id: &str, clip: &Json, keys: &[&str]) -> bool {
        self.forms
            .clips
            .get(id)
            .is_some_and(|form| keys.iter().all(|key| form.get(key) == clip.get(key)))
    }
}

/// v2 clip fields describing what a clip shows.
const CONTENT_KEYS: [&str; 5] = ["sourceId", "in", "speed", "text", "title"];
/// v2 clip fields describing picture properties.
const VISUAL_KEYS: [&str; 6] = [
    "opacity",
    "crop",
    "transform",
    "blend",
    "blendMode",
    "keyframes",
];
/// Kernel properties that v2 clip fields describe.
const V2_PROPS: [&str; 14] = [
    "opacity", "crop_l", "crop_t", "crop_r", "crop_b", "x", "y", "scale", "scale_x", "scale_y",
    "rotation", "anchor_x", "anchor_y", "blend",
];

struct Importer<'a> {
    source: &'a v2::Project,
    rate: Rate,
    catalog: &'a Catalog,
    hint: Option<Hint<'a>>,
    names: HashSet<String>,
    media: HashMap<String, Id>,
    warnings: Vec<String>,
    dropped: Vec<String>,
    snapped: usize,
}

/// A clip being imported with the transitions v2 attached to it.
struct Pending {
    clip: Clip,
    transitions: Vec<v2::Transition>,
    volume: Option<f64>,
}

impl Importer<'_> {
    /// Report an adjustment: the data arrived, changed.
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// Report data that has no equivalent and was left behind.
    fn lose(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.dropped.push(message.clone());
        self.warnings.push(message);
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
        let mut sequence = match self.hint.as_ref().and_then(|h| h.project.sequences.first()) {
            Some(hinted) => {
                self.names.insert(hinted.id.as_str().to_owned());
                Sequence {
                    rate: self.rate,
                    drop_frame: hinted.drop_frame && self.rate == hinted.rate,
                    width,
                    height,
                    tracks: Vec::new(),
                    markers: Vec::new(),
                    ..hinted.clone()
                }
            }
            None => Sequence {
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
            },
        };
        if !self.rate.is_aligned(sequence.start) {
            sequence.start = self.rate.round(sequence.start);
        }
        let media = self.import_media(width, height);
        let mut track_names = HashSet::new();
        // The model is explicit about sound; only plain v2 projects play a
        // picture clip's embedded audio.
        let explicit_audio = self.hint.is_some()
            || source
                .tracks
                .iter()
                .any(|t| t.kind == v2::TrackType::Audio && !t.clips.is_empty());
        let mut embedded = Vec::new();
        for track in &source.tracks {
            // A companion `<track>/titles` or `<track>/adjust` track merges
            // back into the kernel track it was exported from.
            let base = Lane::companion_of(track.id.as_str())
                .filter(|base| {
                    self.hint
                        .as_ref()
                        .is_some_and(|h| h.tracks.contains_key(base))
                        || sequence.tracks.iter().any(|t| t.id == *base)
                })
                .map(str::to_owned);
            let main = base
                .as_ref()
                .and_then(|base| sequence.tracks.iter().position(|t| t.id == base.as_str()));
            let (imported, volumes) = self.import_track(
                track,
                &media,
                &mut track_names,
                &mut sequence.markers,
                base.as_deref().filter(|_| main.is_none()),
            );
            if let Some(index) = main {
                let into = &mut sequence.tracks[index];
                into.clips.extend(imported.clips);
                into.sort();
                into.transitions.extend(imported.transitions);
                continue;
            }
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
        self.restore_hidden(&mut sequence);
        for marker in &source.markers {
            let marker = self.marker(marker);
            sequence.markers.push(marker);
        }
        sequence.markers.sort_by_key(|m| m.at);
        if !source.transitions.is_empty() || !source.effects.is_empty() {
            self.lose(
                "project-level transitions and effects have no equivalent and were not imported",
            );
        }
        if source.metadata.as_ref().is_some_and(|m| !m.is_empty()) {
            self.lose("project metadata was not imported");
        }
        let mut project = Project {
            name: source.name.clone().unwrap_or_default(),
            media,
            sequences: vec![sequence],
            retired: Default::default(),
        };
        if let Some(hint) = &self.hint {
            // Keep the hint's track order; new tracks follow it.
            project.sequences[0].tracks.sort_by_key(|track| {
                hint.tracks
                    .get(track.id.as_str())
                    .map_or(usize::MAX, |t| t.0)
            });
            project
                .sequences
                .extend(hint.project.sequences.iter().skip(1).cloned());
            project.retired = hint.project.retired.clone();
            project.retire_missing(hint.project.ids());
        }
        project
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
                    v2::TrackType::Audio => entry.1 = true,
                    _ => entry.0 = true,
                }
                if let (Some(input), Some(duration)) = (clip.source_in(), clip.timeline_duration())
                {
                    let out = clip
                        .source_out()
                        .unwrap_or(input + duration * clip.speed.unwrap_or(1.0).abs());
                    entry.2 = entry.2.max(out);
                }
            }
        }
        let mut assumed_audio = false;
        let mut media = Vec::new();
        for item in &source.sources {
            let id = self.name(item.id.as_str(), "m");
            self.media.insert(item.id.as_str().to_owned(), id.clone());
            if let Some(hinted) = self.hint.as_ref().and_then(|hint| {
                let unchanged =
                    serde_json::to_value(item).ok().as_ref() == hint.forms.media.get(id.as_str());
                hint.media.get(id.as_str()).filter(|_| unchanged)
            }) {
                media.push((*hinted).clone());
                continue;
            }
            let (on_video, on_audio, furthest) =
                used_on.get(item.id.as_str()).copied().unwrap_or_default();
            let video = match item.width.zip(item.height) {
                Some((w, h)) => Some(VideoInfo {
                    width: w.round() as u32,
                    height: h.round() as u32,
                }),
                None if on_video || !on_audio => {
                    self.warn(format!(
                        "media `{id}` has no recorded frame size; assumed {width}x{height}"
                    ));
                    Some(VideoInfo { width, height })
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
                self.lose(format!(
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

    /// Put back the hint's clips and transitions that v2 cannot show, and
    /// keep the hint's transition order.
    fn restore_hidden(&mut self, sequence: &mut Sequence) {
        let Some(hint) = &self.hint else { return };
        let Some(hinted) = hint.project.sequences.first() else {
            return;
        };
        for track in &hinted.tracks {
            let Some(into) = sequence.tracks.iter_mut().find(|t| t.id == track.id) else {
                continue;
            };
            for clip in hidden_clips(track) {
                if into.clip(clip.id.as_str()).is_none() {
                    into.insert_sorted(clip.clone());
                }
            }
            let exported: HashSet<&Id> = exported_transitions(track).map(|t| &t.id).collect();
            for transition in &track.transitions {
                if exported.contains(&transition.id)
                    || into.transition(transition.id.as_str()).is_some()
                {
                    continue;
                }
                let from = into.clip_index(transition.from.as_str());
                let to = into.clip_index(transition.to.as_str());
                if let (Some(a), Some(b)) = (from, to) {
                    if b == a + 1 && into.clips[a].end() == into.clips[b].start {
                        into.transitions.push(transition.clone());
                    }
                }
            }
            let order = |id: &Id| {
                track
                    .transitions
                    .iter()
                    .position(|t| t.id == *id)
                    .unwrap_or(usize::MAX)
            };
            into.transitions.sort_by_key(|t| order(&t.id));
        }
    }

    fn import_track(
        &mut self,
        track: &v2::Track,
        media: &[Media],
        track_names: &mut HashSet<String>,
        markers: &mut Vec<Marker>,
        named: Option<&str>,
    ) -> (Track, HashMap<Id, f64>) {
        let kind = match track.kind {
            v2::TrackType::Audio => TrackKind::Audio,
            _ => TrackKind::Video,
        };
        let id = match named.and_then(|name| Id::new(name).ok()) {
            Some(id) if track_names.insert(id.as_str().to_owned()) => id,
            _ => unique_track(track.id.as_str(), kind, track_names),
        };
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
        let track_transitions: Vec<v2::Transition> =
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
                    self.lose(format!(
                        "clip `{}` started with `{}` on track {id}; it was dropped",
                        a.clip.id, b.clip.id
                    ));
                    pending.remove(index - 1);
                    continue;
                }
                match requested {
                    Some(requested) => {
                        let align = match requested.alignment {
                            Some(v2::TransitionAlignment::In) => Align::Start,
                            Some(v2::TransitionAlignment::Out) => Align::End,
                            Some(v2::TransitionAlignment::Center) | None => Align::Center,
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
                self.lose(format!(
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
            self.lose(format!(
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
        track: &v2::Track,
        clip: &v2::Clip,
        kind: TrackKind,
        media: &[Media],
    ) -> Option<Pending> {
        let id = self.name(clip.id.as_str(), "c");
        let Some(duration_seconds) = clip.timeline_duration().filter(|d| *d > 0.0) else {
            self.lose(format!("clip `{id}` has no duration and was not imported"));
            return None;
        };
        let transitions: Vec<v2::Transition> = clip.transitions.iter().flatten().cloned().collect();
        let mut value = serde_json::to_value(clip).unwrap_or(Json::Null);
        if let Some(object) = value.as_object_mut() {
            object.remove("transitions");
        }
        let hinted = self
            .hint
            .as_ref()
            .and_then(|hint| hint.clips.get(id.as_str()).cloned());
        let same = |this: &Self, keys: &[&str]| {
            hinted.is_some()
                && this
                    .hint
                    .as_ref()
                    .is_some_and(|hint| hint.same(id.as_str(), &value, keys))
        };
        if let Some(form) = &hinted {
            let unchanged = self
                .hint
                .as_ref()
                .and_then(|hint| hint.forms.clips.get(id.as_str()))
                == Some(&value);
            if unchanged {
                return Some(Pending {
                    clip: form.clone(),
                    transitions,
                    volume: None,
                });
            }
        }
        let content = match &hinted {
            Some(form) if same(self, &CONTENT_KEYS) => form.content.clone(),
            _ => self.content(track, clip, &id, kind, media)?,
        };
        let start = self.time(clip.timeline_start);
        let duration = self.time(duration_seconds).max(self.rate.frame_duration());
        let mut imported = Clip::new(id.clone(), content, start, duration);
        if let Some(form) = &hinted {
            imported.name.clone_from(&form.name);
            imported.enabled = form.enabled;
            imported.link.clone_from(&form.link);
        }
        let v2_props: &[&str] = match kind {
            TrackKind::Video => &V2_PROPS,
            _ => &["gain"],
        };
        let props_keys: &[&str] = match kind {
            TrackKind::Video => &VISUAL_KEYS,
            _ => &["volume"],
        };
        match &hinted {
            Some(form) if same(self, props_keys) => imported.props = form.props.clone(),
            _ => {
                if kind == TrackKind::Video {
                    self.visual_props(&id, clip, &mut imported.props);
                }
                if kind == TrackKind::Audio {
                    if let Some(volume) = clip.volume {
                        imported
                            .props
                            .insert("gain".into(), Param::Static(Value::Number(gain_db(volume))));
                    }
                }
                if let Some(form) = &hinted {
                    for (name, param) in &form.props {
                        if !v2_props.contains(&name.as_str()) {
                            imported.props.insert(name.clone(), param.clone());
                        }
                    }
                }
            }
        }
        match &hinted {
            Some(form) if same(self, &["effects"]) => imported.effects = form.effects.clone(),
            _ => {
                let domain = kind.domain().expect("video or audio");
                for effect in clip.effects.iter().flatten() {
                    if let Some(effect) = self.effect(&id, effect, domain) {
                        imported.effects.push(effect);
                    }
                }
            }
        }
        if clip.gaps.as_ref().is_some_and(|g| !g.is_empty()) {
            self.lose(format!("gaps inside clip `{id}` were not imported"));
        }
        Some(Pending {
            clip: imported,
            transitions,
            volume: clip.volume,
        })
    }

    /// What a v2 clip shows, or `None` when it shows nothing importable.
    fn content(
        &mut self,
        track: &v2::Track,
        clip: &v2::Clip,
        id: &Id,
        kind: TrackKind,
        media: &[Media],
    ) -> Option<Content> {
        let text = clip.text.clone().or_else(|| match &clip.title {
            Some(v2::JsonValue::String(text)) => Some(text.clone()),
            _ => None,
        });
        Some(match (track.kind, &clip.source_id, text) {
            (v2::TrackType::Adjustment, ..) => generator("adjustment", Params::new()),
            (v2::TrackType::Title, _, Some(text)) | (_, None, Some(text)) => generator(
                "title",
                Params::from([("text".to_owned(), Param::Static(Value::Text(text)))]),
            ),
            (_, Some(source), _) => {
                let Some(media_id) = self.media.get(source.as_str()).cloned() else {
                    self.lose(format!(
                        "clip `{id}` uses unknown source `{}` and was not imported",
                        source.as_str()
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
                    self.lose(format!(
                        "clip `{id}` needs {} its media lacks; it was not imported",
                        domain.name()
                    ));
                    return None;
                }
                Content::Media {
                    media: media_id,
                    source_in: self.time(clip.source_in().unwrap_or(0.0)),
                    speed: ratio,
                }
            }
            (_, None, None) => {
                self.lose(format!(
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
        requested: &v2::Transition,
        domain: Domain,
        align: Align,
        duration: Time,
    ) -> Transition {
        let unset = Id::new("_").expect("valid id");
        if let (Some(hint), Some(id)) = (&self.hint, &requested.id) {
            let unchanged = serde_json::to_value(requested).ok().as_ref()
                == hint.forms.transitions.get(id.as_str());
            if let Some(hinted) = hint.transitions.get(id.as_str()).filter(|_| unchanged) {
                let hinted = (*hinted).clone();
                self.names.insert(id.clone());
                return Transition {
                    duration,
                    from: unset.clone(),
                    to: unset,
                    ..hinted
                };
            }
        }
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
                        None => self.lose(format!(
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

    fn visual_props(&mut self, id: &Id, clip: &v2::Clip, props: &mut Params) {
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
                Some(v2::JsonValue::Object(map)) => map.get(axis).and_then(number),
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
                self.lose(format!("clip `{id}` 3D transform values were not imported"));
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
                self.lose(format!("clip `{id}` blend mode `{blend}` is unsupported"));
            }
        }
        let mut animated: BTreeMap<&'static str, Vec<Keyframe>> = BTreeMap::new();
        for key in clip.keyframes.iter().flatten() {
            let Some(name) = keyframe_property(&key.property) else {
                self.lose(format!(
                    "clip `{id}` keyframes on `{}` were not imported",
                    key.property
                ));
                continue;
            };
            let Some(value) = number(&key.value) else {
                self.lose(format!(
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

    fn effect(&mut self, id: &Id, effect: &v2::Effect, domain: Domain) -> Option<Effect> {
        match effect {
            v2::Effect::Color(color) => {
                let p = &color.params;
                let def = self.catalog.fx("color.basic").expect("color.basic");
                let mut params = Params::new();
                for (name, value) in [
                    ("exposure", p.exposure),
                    ("contrast", p.contrast),
                    ("brightness", p.brightness),
                    ("saturation", p.saturation),
                    ("temperature", p.temperature),
                    ("tint", p.tint),
                    ("gamma", p.gamma),
                    ("highlights", p.highlights),
                    ("shadows", p.shadows),
                    ("vibrance", p.vibrance),
                    ("mix", p.mix),
                ] {
                    if def
                        .find(name)
                        .is_some_and(|d| d.default.as_number() != Some(value))
                    {
                        params.insert(name.to_owned(), Param::Static(Value::Number(value)));
                    }
                }
                if domain != Domain::Video {
                    self.lose(format!("color correction on audio clip `{id}` was dropped"));
                    return None;
                }
                Some(Effect {
                    fx: def.id.clone(),
                    enabled: color.enabled.unwrap_or(true),
                    params,
                })
            }
            v2::Effect::Generic(generic) => {
                let def = self
                    .catalog
                    .fx(&generic.kind)
                    .filter(|d| d.kind == FxKind::Effect && d.domain == domain);
                let Some(def) = def else {
                    self.lose(format!(
                        "effect `{}` on clip `{id}` has no equivalent and was not imported",
                        generic.kind
                    ));
                    return None;
                };
                let mut params = Params::new();
                for (name, raw) in generic.params.iter().flatten() {
                    let value = def.find(name).and_then(|param| json_value(param, raw));
                    match value {
                        Some(value) => {
                            params.insert(name.clone(), Param::Static(value));
                        }
                        None => self.lose(format!(
                            "`{}` parameter `{name}` on clip `{id}` was not imported",
                            generic.kind
                        )),
                    }
                }
                Some(Effect {
                    fx: def.id.clone(),
                    enabled: generic.enabled.unwrap_or(true),
                    params,
                })
            }
        }
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
            match overlay {
                v2::Overlay::Captions(captions) => {
                    let hinted = self.hint.as_ref().and_then(|hint| {
                        hint.tracks
                            .get(captions.id.as_str())
                            .filter(|(_, t)| t.kind == TrackKind::Subtitle)
                            .map(|(_, t)| *t)
                    });
                    let mut track = match hinted {
                        Some(hinted) if track_names.insert(hinted.id.as_str().to_owned()) => {
                            Track {
                                clips: Vec::new(),
                                transitions: Vec::new(),
                                ..hinted.clone()
                            }
                        }
                        // An overlay added through the export keeps its id.
                        _ if self.hint.is_some()
                            && Id::is_valid(&captions.id)
                            && !self.names.contains(&captions.id)
                            && track_names.insert(captions.id.clone()) =>
                        {
                            Track::new(
                                Id::new(captions.id.clone()).expect("valid id"),
                                TrackKind::Subtitle,
                            )
                        }
                        _ => Track::new(
                            unique_track("S1", TrackKind::Subtitle, track_names),
                            TrackKind::Subtitle,
                        ),
                    };
                    let window_start = captions.start.unwrap_or(0.0);
                    let window_end = captions
                        .duration
                        .map_or(f64::INFINITY, |d| window_start + d);
                    for cue in &captions.cues {
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
                                    self.lose(format!(
                                        "caption `{}` was hidden by the next cue and dropped",
                                        last.id
                                    ));
                                    track.clips.pop();
                                }
                            }
                        }
                        let named = serde_json::to_value(cue)
                            .ok()
                            .and_then(|cue| cue.get("id")?.as_str().map(str::to_owned));
                        let id = match &named {
                            Some(id) => self.name(id, "cap"),
                            None => self.next("cap"),
                        };
                        let mut caption = Clip::new(
                            id,
                            Content::Caption {
                                text: cue.text.clone(),
                            },
                            start,
                            end - start,
                        );
                        if let Some(hinted) = hinted.and_then(|t| t.clip(caption.id.as_str())) {
                            caption = Clip {
                                start: caption.start,
                                duration: caption.duration,
                                content: caption.content,
                                ..hinted.clone()
                            };
                        }
                        track.clips.push(caption);
                    }
                    if captions.style.is_some() {
                        self.lose(format!(
                            "caption style of `{}` was not imported",
                            captions.id
                        ));
                    }
                    sequence.tracks.push(track);
                }
                v2::Overlay::Subtitles(subtitles) => self.lose(format!(
                    "subtitle file `{}` was not imported; bring it in as a caption track",
                    subtitles.path
                )),
                v2::Overlay::Generic(_) => {
                    self.lose("an overlay without an equivalent was not imported")
                }
            }
        }
    }

    fn marker(&mut self, marker: &v2::Marker) -> Marker {
        if let Some(hint) = &self.hint {
            let unchanged = serde_json::to_value(marker).ok().as_ref()
                == hint.forms.markers.get(marker.id.as_str());
            if let Some(hinted) = hint.markers.get(marker.id.as_str()).filter(|_| unchanged) {
                let hinted = (*hinted).clone();
                self.names.insert(hinted.id.as_str().to_owned());
                return hinted;
            }
        }
        let color = marker
            .color
            .as_deref()
            .and_then(|c| MarkerColor::parse(&c.to_lowercase()));
        if marker.color.is_some() && color.is_none() {
            self.lose(format!(
                "marker `{}` color is not in the marker palette",
                marker.id
            ));
        }
        let id = self.name(&marker.id, "mk");
        let note = self
            .hint
            .as_ref()
            .and_then(|hint| hint.markers.get(id.as_str()))
            .and_then(|hinted| hinted.note.clone());
        Marker {
            id,
            at: self.time(marker.time),
            duration: self.time(marker.duration.unwrap_or(0.0)),
            name: marker.label.clone(),
            color,
            note,
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
fn json_value(def: &cut_model::ParamDef, raw: &v2::JsonValue) -> Option<Value> {
    match (&def.ty, raw) {
        (ParamType::Number, raw) => number(raw).map(Value::Number),
        (ParamType::Bool, v2::JsonValue::Bool(b)) => Some(Value::Bool(*b)),
        (ParamType::Text, v2::JsonValue::String(s)) => Some(Value::Text(s.clone())),
        (ParamType::Enum(_), v2::JsonValue::String(s)) => Some(Value::Enum(s.clone())),
        (ParamType::Color, v2::JsonValue::String(s)) => cut_model::Rgba::parse(s).map(Value::Color),
        _ => None,
    }
}

fn number(value: &v2::JsonValue) -> Option<f64> {
    match value {
        v2::JsonValue::Number(n) => n.as_f64(),
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
