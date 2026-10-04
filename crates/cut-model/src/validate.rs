//! Project invariants.
//!
//! Operations keep these invariants; `validate` checks a whole project, such
//! as one just read from text or converted from another format.

use crate::catalog::{Catalog, Domain, FxKind};
use crate::diag::Diagnostic;
use crate::id::Id;
use crate::model::{Clip, Content, Params, Project, Sequence, Track, TrackKind};
use crate::time::Time;
use std::collections::{HashMap, HashSet};

pub fn validate(project: &Project, catalog: &Catalog) -> Vec<Diagnostic> {
    let mut validator = Validator {
        project,
        catalog,
        out: Vec::new(),
    };
    validator.ids();
    validator.media();
    for sequence in &project.sequences {
        validator.sequence(sequence);
    }
    validator.nesting();
    validator.links();
    validator.out
}

/// Errors only.
pub fn errors(project: &Project, catalog: &Catalog) -> Vec<Diagnostic> {
    validate(project, catalog)
        .into_iter()
        .filter(Diagnostic::is_error)
        .collect()
}

struct Validator<'a> {
    project: &'a Project,
    catalog: &'a Catalog,
    out: Vec<Diagnostic>,
}

impl Validator<'_> {
    fn error(&mut self, code: &'static str, entity: &Id, message: String) {
        self.out
            .push(Diagnostic::error(code, message).entity(entity));
    }

    fn warning(&mut self, code: &'static str, entity: &Id, message: String) {
        self.out
            .push(Diagnostic::warning(code, message).entity(entity));
    }

    fn ids(&mut self) {
        let project = self.project;
        let mut seen = HashMap::<&str, &str>::new();
        let mut entities: Vec<(&Id, &str)> = Vec::new();
        entities.extend(project.media.iter().map(|m| (&m.id, "media")));
        for sequence in &project.sequences {
            entities.push((&sequence.id, "sequence"));
            entities.extend(sequence.markers.iter().map(|m| (&m.id, "marker")));
            for track in &sequence.tracks {
                entities.extend(track.clips.iter().map(|c| (&c.id, "clip")));
                entities.extend(track.transitions.iter().map(|t| (&t.id, "transition")));
            }
        }
        for (id, kind) in entities {
            if let Some(previous) = seen.insert(id.as_str(), kind) {
                self.error(
                    "id.duplicate",
                    id,
                    format!("`{id}` names both a {previous} and a {kind}"),
                );
            }
        }
        let links: HashSet<&Id> = project
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .filter_map(|c| c.link.as_ref())
            .collect();
        for link in links {
            if let Some(kind) = seen.get(link.as_str()) {
                self.error(
                    "id.duplicate",
                    link,
                    format!("link group `{link}` reuses the id of a {kind}"),
                );
            }
        }
    }

    fn media(&mut self) {
        for media in &self.project.media {
            if media.path.is_empty() {
                self.error(
                    "media.path",
                    &media.id,
                    format!("media `{}` has no path", media.id),
                );
            }
            if !media.duration.is_positive() {
                self.error(
                    "media.duration",
                    &media.id,
                    format!("media `{}` needs a positive duration", media.id),
                );
            }
            if media.start.is_negative() || !media.rate.is_aligned(media.start) {
                self.error(
                    "media.timecode",
                    &media.id,
                    format!("media `{}` start timecode is not on a frame", media.id),
                );
            }
            if media.drop_frame && !media.rate.supports_drop_frame() {
                self.error(
                    "media.timecode",
                    &media.id,
                    format!("media `{}` uses drop-frame at {}", media.id, media.rate),
                );
            }
            if media.video.is_none() && media.audio.is_none() {
                self.error(
                    "media.streams",
                    &media.id,
                    format!("media `{}` has neither video nor audio", media.id),
                );
            }
            if media.video.is_some_and(|v| v.width == 0 || v.height == 0)
                || media
                    .audio
                    .is_some_and(|a| a.channels == 0 || a.sample_rate == 0)
            {
                self.error(
                    "media.streams",
                    &media.id,
                    format!("media `{}` has an empty stream", media.id),
                );
            }
        }
    }

    fn sequence(&mut self, sequence: &Sequence) {
        let id = &sequence.id;
        if sequence.width == 0 || sequence.height == 0 || sequence.sample_rate == 0 {
            self.error(
                "sequence.format",
                id,
                format!("sequence `{id}` needs a frame size and sample rate"),
            );
        } else if sequence.width % 2 == 1 || sequence.height % 2 == 1 {
            self.warning(
                "sequence.format",
                id,
                format!("sequence `{id}` has an odd frame size that most codecs reject"),
            );
        }
        if sequence.start.is_negative() || !sequence.rate.is_aligned(sequence.start) {
            self.error(
                "sequence.timecode",
                id,
                format!("sequence `{id}` start timecode is not on a frame"),
            );
        }
        if sequence.drop_frame && !sequence.rate.supports_drop_frame() {
            self.error(
                "sequence.timecode",
                id,
                format!("sequence `{id}` uses drop-frame at {}", sequence.rate),
            );
        }
        let mut track_ids = HashSet::new();
        for track in &sequence.tracks {
            if !track_ids.insert(track.id.as_str()) {
                self.error(
                    "track.duplicate",
                    &track.id,
                    format!("sequence `{id}` has two tracks named `{}`", track.id),
                );
            }
            self.track(sequence, track);
        }
        for marker in &sequence.markers {
            if marker.at.is_negative()
                || marker.duration.is_negative()
                || !sequence.rate.is_aligned(marker.at)
                || !sequence.rate.is_aligned(marker.duration)
            {
                self.error(
                    "marker.time",
                    &marker.id,
                    format!("marker `{}` must start and last whole frames", marker.id),
                );
            }
        }
    }

    fn track(&mut self, sequence: &Sequence, track: &Track) {
        let rate = sequence.rate;
        let mut previous: Option<&Clip> = None;
        for clip in &track.clips {
            if clip.start.is_negative() || !clip.duration.is_positive() {
                self.error(
                    "clip.time",
                    &clip.id,
                    format!(
                        "clip `{}` needs a non-negative start and a positive duration",
                        clip.id
                    ),
                );
            }
            if !rate.is_aligned(clip.start) || !rate.is_aligned(clip.duration) {
                self.error(
                    "clip.misaligned",
                    &clip.id,
                    format!("clip `{}` does not start and end on {rate} frames", clip.id),
                );
            }
            if let Some(previous) = previous {
                if clip.start < previous.end() {
                    self.error(
                        "clip.overlap",
                        &clip.id,
                        format!(
                            "clip `{}` overlaps `{}` on track {}",
                            clip.id, previous.id, track.id
                        ),
                    );
                }
            }
            previous = Some(clip);
            self.clip(sequence, track, clip);
        }
        self.transitions(sequence, track);
    }

    fn clip(&mut self, sequence: &Sequence, track: &Track, clip: &Clip) {
        let id = &clip.id;
        let domain = track.kind.domain();
        match (&clip.content, domain) {
            (Content::Caption { text }, None) => {
                if text.trim().is_empty() {
                    self.warning("clip.caption", id, format!("caption `{id}` is empty"));
                }
            }
            (Content::Caption { .. }, Some(_)) | (_, None) => self.error(
                "clip.domain",
                id,
                format!(
                    "clip `{id}`: subtitle tracks hold captions only, and captions belong on subtitle tracks"
                ),
            ),
            (Content::Media { media, .. }, Some(domain)) => match self.project.media(media.as_str()) {
                None => self.error("ref.missing", id, format!("clip `{id}` uses unknown media `{media}`")),
                Some(found) => {
                    if !found.supports(domain) {
                        self.error(
                            "clip.domain",
                            id,
                            format!(
                                "clip `{id}` is on {} track {} but media `{media}` has no {} stream",
                                domain.name(),
                                track.id,
                                domain.name()
                            ),
                        );
                    }
                    self.source_bounds(clip, found.duration);
                }
            },
            (Content::Sequence { sequence: nested, .. }, Some(_)) => {
                if nested == &sequence.id {
                    self.error("sequence.cycle", id, format!("clip `{id}` nests its own sequence"));
                } else if let Some(found) = self.project.sequence(nested.as_str()) {
                    self.source_bounds(clip, found.duration());
                } else {
                    self.error(
                        "ref.missing",
                        id,
                        format!("clip `{id}` uses unknown sequence `{nested}`"),
                    );
                }
            }
            (Content::Generator { fx, params }, Some(domain)) => {
                self.fx_params(id, fx, FxKind::Generator, domain, params);
            }
        }
        match domain {
            Some(domain) => {
                for (name, param) in &clip.props {
                    match self.catalog.prop(domain, name) {
                        None => self.error(
                            "param.unknown",
                            id,
                            format!("clip `{id}` has no {} property `{name}`", domain.name()),
                        ),
                        Some(def) => {
                            if let Err(message) = def.check(param) {
                                self.error("param.invalid", id, format!("clip `{id}`: {message}"));
                            }
                        }
                    }
                }
                for effect in &clip.effects {
                    self.fx_params(id, &effect.fx, FxKind::Effect, domain, &effect.params);
                }
            }
            None if !clip.props.is_empty() || !clip.effects.is_empty() => self.error(
                "clip.domain",
                id,
                format!("caption `{id}` cannot carry properties or effects"),
            ),
            None => {}
        }
    }

    fn source_bounds(&mut self, clip: &Clip, available: Time) {
        let Some((source_in, source_out)) = clip.source_range() else {
            return;
        };
        if source_in.is_negative() || source_out > available {
            self.error(
                "clip.source_range",
                &clip.id,
                format!(
                    "clip `{}` uses source {source_in}–{source_out} but its source is {available} long",
                    clip.id
                ),
            );
        }
    }

    fn fx_params(&mut self, owner: &Id, fx: &Id, kind: FxKind, domain: Domain, params: &Params) {
        let Some(def) = self.catalog.fx(fx.as_str()) else {
            self.error(
                "fx.unknown",
                owner,
                format!("`{owner}` uses unknown {} `{fx}`", kind.name()),
            );
            return;
        };
        if def.kind != kind || def.domain != domain {
            self.error(
                "fx.kind",
                owner,
                format!(
                    "`{owner}` needs a {} {}, but `{fx}` is a {} {}",
                    domain.name(),
                    kind.name(),
                    def.domain.name(),
                    def.kind.name()
                ),
            );
            return;
        }
        for (name, param) in params {
            match def.find(name) {
                None => self.error(
                    "param.unknown",
                    owner,
                    format!("`{fx}` has no parameter `{name}`"),
                ),
                Some(param_def) => {
                    if let Err(message) = param_def.check(param) {
                        self.error("param.invalid", owner, format!("`{owner}` {fx}: {message}"));
                    }
                }
            }
        }
    }

    fn transitions(&mut self, sequence: &Sequence, track: &Track) {
        let mut cuts = HashSet::new();
        for transition in &track.transitions {
            let id = &transition.id;
            let from = track.clip_index(transition.from.as_str());
            let to = track.clip_index(transition.to.as_str());
            let (Some(from), Some(to)) = (from, to) else {
                self.error(
                    "transition.ref",
                    id,
                    format!(
                        "transition `{id}` joins `{}` and `{}`, which are not both on track {}",
                        transition.from, transition.to, track.id
                    ),
                );
                continue;
            };
            let (from, to) = (&track.clips[from], &track.clips[to]);
            if from.end() != to.start {
                self.error(
                    "transition.not_adjacent",
                    id,
                    format!(
                        "transition `{id}` needs `{}` to end where `{}` starts",
                        from.id, to.id
                    ),
                );
                continue;
            }
            if !cuts.insert((&from.id, &to.id)) {
                self.error(
                    "transition.duplicate",
                    id,
                    format!(
                        "the cut between `{}` and `{}` has two transitions",
                        from.id, to.id
                    ),
                );
            }
            let Some(domain) = track.kind.domain() else {
                self.error(
                    "transition.domain",
                    id,
                    "subtitle tracks have no transitions".into(),
                );
                continue;
            };
            self.fx_params(
                id,
                &transition.fx,
                FxKind::Transition,
                domain,
                &transition.params,
            );
            if !transition.duration.is_positive() || !sequence.rate.is_aligned(transition.duration)
            {
                self.error(
                    "transition.time",
                    id,
                    format!("transition `{id}` must last a positive number of frames"),
                );
                continue;
            }
            let (before, after) = transition.split_at_cut(sequence.rate);
            if before > from.duration || after > to.duration {
                self.error(
                    "transition.too_long",
                    id,
                    format!("transition `{id}` is longer than the clips it joins"),
                );
            }
            if self.head_handle(to).is_some_and(|handle| handle < before)
                || self.tail_handle(from).is_some_and(|handle| handle < after)
            {
                self.error(
                    "transition.handles",
                    id,
                    format!(
                        "transition `{id}` needs {before} of media before `{}` and {after} after `{}`",
                        to.id, from.id
                    ),
                );
            }
        }
    }

    fn source_length(&self, clip: &Clip) -> Option<Time> {
        match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()).map(|m| m.duration),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map(Sequence::duration),
            Content::Generator { .. } | Content::Caption { .. } => None,
        }
    }

    /// Unused source available before the clip's first frame, in timeline
    /// time. `None` means unlimited.
    fn head_handle(&self, clip: &Clip) -> Option<Time> {
        let (source_in, source_out) = clip.source_range()?;
        let speed = clip.speed();
        let handle = if speed.is_reverse() {
            self.source_length(clip)? - source_out
        } else {
            source_in
        };
        Some(speed.timeline_span(handle))
    }

    fn tail_handle(&self, clip: &Clip) -> Option<Time> {
        let (source_in, source_out) = clip.source_range()?;
        let speed = clip.speed();
        let handle = if speed.is_reverse() {
            source_in
        } else {
            self.source_length(clip)? - source_out
        };
        Some(speed.timeline_span(handle))
    }

    fn nesting(&mut self) {
        let project = self.project;
        let children: HashMap<&str, Vec<&str>> = project
            .sequences
            .iter()
            .map(|sequence| {
                let nested = sequence
                    .tracks
                    .iter()
                    .flat_map(|t| &t.clips)
                    .filter_map(|c| match &c.content {
                        Content::Sequence { sequence, .. } => Some(sequence.as_str()),
                        _ => None,
                    })
                    .collect();
                (sequence.id.as_str(), nested)
            })
            .collect();
        fn visits_cycle<'a>(
            node: &'a str,
            children: &HashMap<&'a str, Vec<&'a str>>,
            path: &mut Vec<&'a str>,
        ) -> bool {
            if path.contains(&node) {
                return true;
            }
            path.push(node);
            let found = children
                .get(node)
                .is_some_and(|next| next.iter().any(|child| visits_cycle(child, children, path)));
            path.pop();
            found
        }
        for sequence in &project.sequences {
            if visits_cycle(sequence.id.as_str(), &children, &mut Vec::new()) {
                self.error(
                    "sequence.cycle",
                    &sequence.id,
                    format!("sequence `{}` contains itself through nesting", sequence.id),
                );
            }
        }
    }

    fn links(&mut self) {
        let mut groups = HashMap::<&Id, Vec<&Id>>::new();
        for sequence in &self.project.sequences {
            for clip in sequence.tracks.iter().flat_map(|t| &t.clips) {
                if let Some(link) = &clip.link {
                    groups.entry(link).or_default().push(&sequence.id);
                }
            }
        }
        for (link, sequences) in groups {
            if sequences.len() < 2 {
                self.warning(
                    "link.single",
                    link,
                    format!("link group `{link}` has one clip"),
                );
            }
            if sequences.windows(2).any(|pair| pair[0] != pair[1]) {
                self.error(
                    "link.sequences",
                    link,
                    format!("link group `{link}` spans sequences"),
                );
            }
        }
    }
}

/// The track kind a content type may be placed on.
pub fn accepts(kind: TrackKind, content: &Content) -> bool {
    matches!(
        (kind, content),
        (TrackKind::Subtitle, Content::Caption { .. })
            | (
                TrackKind::Video | TrackKind::Audio,
                Content::Media { .. } | Content::Sequence { .. } | Content::Generator { .. }
            )
    )
}
