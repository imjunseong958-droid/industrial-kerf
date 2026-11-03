//! Portable editing patterns, written in the Kerf language. Media declarations
//! are typed slots (`path="slot:slot1"`), never references to the author's files.
//! The looks a pattern uses that Kerf does not bundle travel with it.
//! Instantiation produces an ordinary edit script for the runtime to propose.

use crate::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use kerf_model::look::{self, Look};
use kerf_model::{
    Catalog, Clip, ClipChange, Content, Id, Media, NewClip, Op, Param, ParamType, Project, Rate,
    Sequence, SequenceChange, Time, TrackChange, TrackKind, TrackRoleChange, Transition, Value,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Binding {
    pub media: Id,
    /// Offset from the replacement media's first frame, including handles.
    pub from: Time,
    /// The replacement's sound onsets, in its own time, when it is music the
    /// pattern's markers should land on; empty leaves the markers where
    /// they are.
    pub beats: Vec<Time>,
}

/// A read preset.
#[derive(Clone, Debug)]
pub struct Preset {
    pub pattern: Project,
    /// The looks it carries, each once.
    pub looks: Vec<Look>,
    /// What the pattern reads with: the catalog it was read against and
    /// the looks it carries.
    pub catalog: Catalog,
}

fn diagnostics(errors: impl IntoIterator<Item = kerf_model::Diagnostic>) -> String {
    errors
        .into_iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn valid(project: &Project, catalog: &Catalog) -> Result<(), String> {
    let errors: Vec<_> = kerf_model::validate(project, catalog)
        .into_iter()
        .filter(|d| d.is_error())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(diagnostics(errors))
    }
}

/// Sequence dependencies in children-first order, independent of file order.
fn order(project: &Project, root: &Id) -> Result<Vec<Id>, String> {
    fn visit(
        project: &Project,
        id: &Id,
        seen: &mut BTreeSet<Id>,
        out: &mut Vec<Id>,
    ) -> Result<(), String> {
        if !seen.insert(id.clone()) {
            return Ok(());
        }
        let sequence = project
            .sequence(id.as_str())
            .ok_or_else(|| format!("no sequence `{id}`"))?;
        for clip in sequence.tracks.iter().flat_map(|t| &t.clips) {
            if let Content::Sequence { sequence, .. } = &clip.content {
                visit(project, sequence, seen, out)?;
            }
        }
        out.push(id.clone());
        Ok(())
    }
    let mut out = Vec::new();
    visit(project, root, &mut BTreeSet::new(), &mut out)?;
    Ok(out)
}

/// Export a sequence and its nested sequences. Each cut has a separate media
/// slot; linked picture and sound from the same source share one. Source ranges
/// start at zero, with just enough room for the original transition handles.
pub fn export(
    project: &Project,
    catalog: &Catalog,
    sequence: &Id,
    name: &str,
) -> Result<String, String> {
    valid(project, catalog)?;
    let mut pattern = Project::new(name);
    for id in order(project, sequence)?.into_iter().rev() {
        pattern.sequences.push(
            project
                .sequence(id.as_str())
                .expect("visited sequence")
                .clone(),
        );
    }
    pattern.sequences[0].name = name.to_owned();
    let mut slot_ids = project.id_gen();
    let mut groups = BTreeMap::<(Id, Id), usize>::new();
    let mut ranges: Vec<(Media, Time, Time)> = Vec::new();
    for sequence in &mut pattern.sequences {
        for track in &mut sequence.tracks {
            for clip in &mut track.clips {
                let Some((low, high)) = required_source(clip, &track.transitions, sequence.rate)
                else {
                    continue;
                };
                let Content::Media { media, .. } = &mut clip.content else {
                    continue;
                };
                let source = project.media(media.as_str()).expect("validated media");
                let key = (
                    clip.link.clone().unwrap_or_else(|| clip.id.clone()),
                    media.clone(),
                );
                let next = ranges.len();
                let index = *groups.entry(key).or_insert_with(|| {
                    let mut slot = source.clone();
                    slot.id = slot_ids.next("slot");
                    slot.path = format!("slot:{}", slot.id);
                    slot.start = Time::ZERO;
                    slot.reel = None;
                    slot.video = None;
                    slot.audio = None;
                    ranges.push((slot, low, high));
                    next
                });
                let (slot, from, to) = &mut ranges[index];
                *from = (*from).min(low);
                *to = (*to).max(high);
                match track.kind {
                    TrackKind::Video => slot.video = source.video,
                    TrackKind::Audio => slot.audio = source.audio,
                    TrackKind::Subtitle => unreachable!("validated media track"),
                }
                *media = slot.id.clone();
            }
        }
    }
    let offsets: BTreeMap<_, _> = ranges
        .iter()
        .map(|(m, from, _)| (m.id.clone(), *from))
        .collect();
    for sequence in &mut pattern.sequences {
        for clip in sequence.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if let Content::Media {
                media, source_in, ..
            } = &mut clip.content
            {
                *source_in -= offsets[media];
            }
        }
    }
    pattern.media = ranges
        .into_iter()
        .map(|(mut media, from, to)| {
            media.duration = to - from;
            media
        })
        .collect();
    valid(&pattern, catalog)?;
    let mut document = print_document(&pattern, catalog, PrintOptions::default());
    for look in used_looks(&pattern, catalog).into_values() {
        if Catalog::bundled()
            .look(look.id.as_str(), look.version)
            .is_none()
        {
            document.push('\n');
            document.push_str(&carried(look));
        }
    }
    Ok(document)
}

/// The looks `pattern` uses, each once.
fn used_looks<'a>(pattern: &Project, catalog: &'a Catalog) -> BTreeMap<(Id, u32), &'a Look> {
    let mut looks = BTreeMap::new();
    for track in pattern.sequences.iter().flat_map(|s| &s.tracks) {
        let effects = track
            .clips
            .iter()
            .flat_map(|c| &c.effects)
            .map(|e| (&e.fx, e.version));
        let transitions = track.transitions.iter().map(|t| (&t.fx, t.version));
        for (fx, version) in effects.chain(transitions) {
            if let Some(look) = catalog.look(fx.as_str(), version) {
                looks.insert((fx.clone(), version), look);
            }
        }
    }
    looks
}

/// A look as a preset carries it: its `look` line, and the rest of the
/// file indented under it.
fn carried(look: &Look) -> String {
    let mut lines = look
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .skip_while(|line| line.starts_with('#') || *line == "kerf-look 1");
    let mut out = String::new();
    if let Some(header) = lines.next() {
        out.push_str(header);
        out.push('\n');
    }
    for line in lines {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Take the looks a preset carries out of its text: each a `look` line at
/// the top level and the lines indented under it. Their lines are left
/// blank, so the rest reads with the file's own line numbers.
fn carried_looks(document: &str) -> Result<(String, Vec<Look>), String> {
    let mut rest = Vec::new();
    let mut blocks: Vec<(usize, Vec<&str>)> = Vec::new();
    let mut inside = false;
    for (index, line) in document.lines().enumerate() {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            inside = line.split_whitespace().next() == Some("look");
            if inside {
                blocks.push((index + 1, Vec::new()));
            }
        }
        if let (true, Some((_, lines))) = (inside, blocks.last_mut()) {
            lines.push(line);
            rest.push("");
        } else {
            rest.push(line);
        }
    }
    let looks = blocks
        .into_iter()
        .map(|(first, mut lines)| {
            while lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.pop();
            }
            let body = lines.join("\n");
            // Padded, so errors name the preset's own lines.
            let padded = format!(
                "kerf-look 1\n{}{body}\n",
                "\n".repeat(first.saturating_sub(2))
            );
            let mut look = look::read(&padded, "preset").map_err(|e| e.to_string())?;
            look.text = format!("kerf-look 1\n{body}\n");
            Ok(look)
        })
        .collect::<Result<_, String>>()?;
    Ok((rest.join("\n"), looks))
}

/// The source interval a clip plays plus the handles its transitions read
/// before its first and after its last frame. Handles round up, so the
/// validator's handle check holds for a slot cut to exactly this interval.
fn required_source(clip: &Clip, transitions: &[Transition], rate: Rate) -> Option<(Time, Time)> {
    let (mut low, mut high) = clip.source_range()?;
    let speed = clip.speed();
    for transition in transitions {
        let (before, after) = transition.split_at_cut(rate);
        let head = if transition.to == clip.id {
            speed.source_span_covering(before)
        } else {
            Time::ZERO
        };
        let tail = if transition.from == clip.id {
            speed.source_span_covering(after)
        } else {
            Time::ZERO
        };
        // Reverse clips read their head from later source and their tail from earlier.
        if speed.is_reverse() {
            low -= tail;
            high += head;
        } else {
            low -= head;
            high += tail;
        }
    }
    Some((low, high))
}

/// Read and validate a portable preset. Every media input must be a slot;
/// unbound file paths and hidden, unused sequences are rejected. A look it
/// carries must be one it uses, and the same as one `catalog` already has
/// by that id and version.
pub fn parse(document: &str, catalog: &Catalog) -> Result<Preset, String> {
    let (document, carried) = carried_looks(document)?;
    let mut catalog = catalog.clone();
    let mut looks: Vec<Look> = Vec::new();
    for look in carried {
        if looks
            .iter()
            .any(|other| other.id == look.id && other.version == look.version)
        {
            continue;
        }
        match catalog.look(look.id.as_str(), look.version) {
            Some(known) if known.same(&look) => {}
            Some(known) => {
                return Err(format!(
                    "the preset's `{}@{}` is not the one in {}",
                    look.id, look.version, known.source
                ))
            }
            None => catalog.define_look(look.clone())?,
        }
        looks.push(look);
    }
    let project = parse_document(&document, &catalog)
        .map_err(diagnostics)?
        .project;
    valid(&project, &catalog)?;
    let used = used_looks(&project, &catalog);
    if let Some(look) = looks
        .iter()
        .find(|look| !used.contains_key(&(look.id.clone(), look.version)))
    {
        return Err(format!(
            "the preset carries `{}@{}`, which it does not use",
            look.id, look.version
        ));
    }
    let root = project
        .sequences
        .first()
        .ok_or("the preset has no sequence")?;
    if root.duration() <= Time::ZERO {
        return Err("the preset is empty".into());
    }
    if order(&project, &root.id)?.len() != project.sequences.len() {
        return Err("the preset contains sequences outside its pattern".into());
    }
    if !project.retired.is_empty() {
        return Err("presets cannot contain retired ids".into());
    }
    for media in &project.media {
        if media.path != format!("slot:{}", media.id)
            || media.start != Time::ZERO
            || media.reel.is_some()
        {
            return Err(format!(
                "media `{}` must be a portable slot with zero timecode",
                media.id
            ));
        }
        if !project
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .any(|c| matches!(&c.content, Content::Media { media: id, .. } if *id == media.id))
        {
            return Err(format!("unused slot `{}`", media.id));
        }
    }
    Ok(Preset {
        pattern: project,
        looks,
        catalog,
    })
}

/// How far into `media` a binding may start and still fill `slot`; negative
/// when the media is too short. Errors when it lacks a stream the slot needs.
fn room(slot: &Media, media: &Media) -> Result<Time, String> {
    let missing: Vec<_> = [
        ("video", slot.video.is_some() && media.video.is_none()),
        ("audio", slot.audio.is_some() && media.audio.is_none()),
    ]
    .into_iter()
    .filter_map(|(stream, missing)| missing.then_some(stream))
    .collect();
    if !missing.is_empty() {
        return Err(format!(
            "slot `{}` needs {}, which `{}` lacks",
            slot.id,
            missing.join(" and "),
            media.id
        ));
    }
    Ok(media.duration - slot.duration)
}

/// The latest whole frame of `media` from which it can fill `slot`, or `None`
/// when it lacks a needed stream or enough material.
pub fn latest_start(slot: &Media, media: &Media) -> Option<Time> {
    let room = room(slot, media).ok().filter(|room| !room.is_negative())?;
    Some(media.rate.time_of(media.rate.frames_floor(room)))
}

/// Captions and title generator text that can be overridden by template clip id.
pub fn text_slots(project: &Project) -> BTreeMap<String, String> {
    project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .flat_map(|t| &t.clips)
        .filter_map(|clip| match &clip.content {
            Content::Caption { text } => Some((clip.id.to_string(), text.clone())),
            Content::Generator { fx, params, .. } if fx == "title" => {
                let text = match params.get("text") {
                    Some(Param::Static(Value::Text(text))) => text.clone(),
                    _ => String::new(),
                };
                Some((clip.id.to_string(), text))
            }
            _ => None,
        })
        .collect()
}

/// Replace a target sequence with a bound pattern, leaving other sequences and
/// all existing media intact. Fresh ids avoid both live and retired identifiers.
/// No project state changes here: callers stage the returned script as a proposal.
pub fn instantiate(
    project: &Project,
    catalog: &Catalog,
    document: &str,
    target: &Id,
    bindings: &BTreeMap<String, Binding>,
    texts: &BTreeMap<String, String>,
) -> Result<String, String> {
    let Preset {
        mut pattern,
        catalog: carried,
        ..
    } = parse(document, catalog)?;
    let catalog = &carried;
    let destination = project
        .sequence(target.as_str())
        .ok_or_else(|| format!("no sequence `{target}`"))?;
    if destination.tracks.iter().any(|t| t.locked) {
        return Err("unlock the target sequence's tracks before applying a preset".into());
    }
    if project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .flat_map(|t| &t.clips)
        .any(|c| matches!(&c.content, Content::Sequence { sequence, .. } if sequence == target))
    {
        return Err("cannot replace a sequence used by another sequence".into());
    }
    for slot in bindings.keys() {
        if pattern.media(slot).is_none() {
            return Err(format!("unknown slot `{slot}`"));
        }
    }
    for slot in &pattern.media {
        let binding = bindings
            .get(slot.id.as_str())
            .ok_or_else(|| format!("missing binding for `{}`", slot.id))?;
        let media = project
            .media(binding.media.as_str())
            .ok_or_else(|| format!("no media `{}`", binding.media))?;
        if binding.from.is_negative() || binding.from > room(slot, media)? {
            return Err(format!("slot `{}` needs {} of media from {} (including transition handles), but `{}` is {} long",
                slot.id, slot.duration, binding.from, media.id, media.duration));
        }
    }
    let allowed_texts = text_slots(&pattern);
    for key in texts.keys() {
        if !allowed_texts.contains_key(key) {
            return Err(format!("unknown text slot `{key}`"));
        }
    }
    let root = pattern.sequences[0].id.clone();
    let sequence_order = order(&pattern, &root)?;
    let mut generator = project.id_gen();
    // The clips that play music a binding re-times, and its onsets.
    let mut beats = BTreeMap::<Id, &[Time]>::new();
    let mut ids = BTreeMap::new();
    ids.insert(root.clone(), target.clone());
    for id in pattern.ids() {
        if !ids.contains_key(id) {
            ids.insert(id.clone(), generator.next("p"));
        }
    }
    for sequence in &mut pattern.sequences {
        sequence.id = ids[&sequence.id].clone();
        for marker in &mut sequence.markers {
            marker.id = ids[&marker.id].clone();
        }
        for track in &mut sequence.tracks {
            for transition in &mut track.transitions {
                transition.id = ids[&transition.id].clone();
                transition.from = ids[&transition.from].clone();
                transition.to = ids[&transition.to].clone();
            }
            for clip in &mut track.clips {
                if let Some(text) = texts.get(clip.id.as_str()) {
                    match &mut clip.content {
                        Content::Caption { text: current } => current.clone_from(text),
                        Content::Generator { params, .. } => {
                            params.insert("text".into(), Param::Static(Value::Text(text.clone())));
                        }
                        _ => unreachable!("checked text slot"),
                    }
                }
                clip.id = ids[&clip.id].clone();
                clip.link = clip.link.as_ref().map(|id| ids[id].clone());
                // A look's time that lands on a marker follows its new id.
                for effect in &mut clip.effects {
                    let Some(def) = catalog.fx(effect.fx.as_str()) else {
                        continue;
                    };
                    for (name, param) in &mut effect.params {
                        let time = def.find(name).is_some_and(|d| d.ty == ParamType::Time);
                        if let (true, Param::Static(Value::Text(marker))) = (time, param) {
                            if let Some(id) =
                                Id::new(marker.as_str()).ok().and_then(|id| ids.get(&id))
                            {
                                *marker = id.to_string();
                            }
                        }
                    }
                }
                match &mut clip.content {
                    Content::Media {
                        media, source_in, ..
                    } => {
                        let binding = &bindings[media.as_str()];
                        *media = binding.media.clone();
                        *source_in += binding.from;
                        if !binding.beats.is_empty() {
                            beats.insert(clip.id.clone(), &binding.beats);
                        }
                    }
                    Content::Sequence { sequence, .. } => *sequence = ids[sequence].clone(),
                    _ => {}
                }
            }
        }
    }
    for sequence in &mut pattern.sequences {
        place_on_beats(sequence, &beats);
    }
    let mut ops = Vec::new();
    let clips: Vec<_> = destination
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.id.clone())
        .collect();
    if !clips.is_empty() {
        ops.push(Op::Delete { clips });
    }
    for marker in &destination.markers {
        ops.push(Op::RemoveMarker {
            id: marker.id.clone(),
        });
    }
    for track in &destination.tracks {
        ops.push(Op::RemoveTrack {
            sequence: target.clone(),
            track: track.id.clone(),
        });
    }
    for original in sequence_order {
        let sequence = pattern
            .sequence(ids[&original].as_str())
            .expect("remapped sequence");
        if original == root {
            ops.push(Op::SetSequence {
                sequence: target.clone(),
                change: SequenceChange {
                    name: Some(sequence.name.clone()),
                    rate: Some(sequence.rate),
                    size: Some((sequence.width, sequence.height)),
                    sample_rate: Some(sequence.sample_rate),
                    start: Some((sequence.start, sequence.drop_frame)),
                },
            });
        } else {
            let mut empty = sequence.clone();
            empty.tracks.clear();
            empty.markers.clear();
            ops.push(Op::AddSequence(empty));
        }
        append_sequence(sequence, &mut ops);
    }
    let script = print_ops(project, catalog, &ops);
    apply_script(project, catalog, &script).map_err(diagnostics)?;
    Ok(script)
}

/// Move each of `sequence`'s markers over a clip in `beats` to the onset of
/// its music nearest it, on a whole frame and after the marker before; the
/// looks whose times land on them follow. Markers over other clips, or with
/// no onset left in their clip, stay.
fn place_on_beats(sequence: &mut Sequence, beats: &BTreeMap<Id, &[Time]>) {
    let rate = sequence.rate;
    let clips: Vec<(&Clip, &[Time])> = sequence
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|clip| beats.get(&clip.id).map(|onsets| (clip, *onsets)))
        .collect();
    if clips.is_empty() {
        return;
    }
    let mut order: Vec<usize> = (0..sequence.markers.len()).collect();
    order.sort_by_key(|&index| sequence.markers[index].at);
    let mut moved = Vec::new();
    let mut last: Option<Time> = None;
    for index in order {
        let at = sequence.markers[index].at;
        let Some((clip, onsets)) = clips
            .iter()
            .find(|(clip, _)| clip.start <= at && at < clip.end())
        else {
            continue;
        };
        let nearest = onsets
            .iter()
            .filter(|onset| {
                clip.source_range()
                    .is_some_and(|(low, high)| (low..high).contains(*onset))
            })
            .filter_map(|onset| clip.played_at(*onset))
            .map(|time| rate.round(time))
            .filter(|time| clip.start <= *time && *time < clip.end())
            .filter(|time| last.is_none_or(|last| *time > last))
            .min_by(|a, b| {
                let distance = |time: &Time| (time.seconds() - at.seconds()).abs();
                distance(a).total_cmp(&distance(b))
            });
        let placed = nearest.unwrap_or(at);
        moved.push((index, placed));
        last = Some(last.map_or(placed, |last| last.max(placed)));
    }
    for (index, placed) in moved {
        sequence.markers[index].at = placed;
    }
}

fn append_sequence(sequence: &Sequence, ops: &mut Vec<Op>) {
    // Markers first: a look's time may land on one.
    for marker in &sequence.markers {
        ops.push(Op::AddMarker {
            sequence: sequence.id.clone(),
            id: Some(marker.id.clone()),
            at: marker.at,
            duration: marker.duration,
            name: marker.name.clone(),
            color: marker.color,
            note: marker.note.clone(),
        });
    }
    let mut links = BTreeMap::<Id, Vec<Id>>::new();
    for track in &sequence.tracks {
        ops.push(Op::AddTrack {
            sequence: sequence.id.clone(),
            id: track.id.clone(),
            kind: track.kind,
            index: None,
        });
        for clip in &track.clips {
            ops.push(Op::Overwrite {
                sequence: sequence.id.clone(),
                at: clip.start,
                duration: clip.duration,
                targets: vec![(
                    track.id.clone(),
                    NewClip {
                        id: Some(clip.id.clone()),
                        content: clip.content.clone(),
                        name: clip.name.clone(),
                        props: clip.props.clone(),
                    },
                )],
            });
            if !clip.enabled {
                ops.push(Op::Set {
                    clip: clip.id.clone(),
                    changes: vec![ClipChange::Enabled(false)],
                });
            }
            for effect in &clip.effects {
                ops.push(Op::FxAdd {
                    clip: clip.id.clone(),
                    effect: effect.clone(),
                    index: None,
                });
            }
            if let Some(link) = &clip.link {
                links.entry(link.clone()).or_default().push(clip.id.clone());
            }
        }
        for transition in &track.transitions {
            ops.push(Op::AddTransition {
                id: Some(transition.id.clone()),
                from: transition.from.clone(),
                to: transition.to.clone(),
                fx: transition.fx.clone(),
                version: transition.version,
                duration: transition.duration,
                align: transition.align,
                params: transition.params.clone(),
            });
        }
    }
    for (group, clips) in links {
        ops.push(Op::Link {
            clips,
            group: Some(group),
        });
    }
    // Lock only after all content and link groups have been installed.
    for track in &sequence.tracks {
        ops.push(Op::SetTrack {
            sequence: sequence.id.clone(),
            track: track.id.clone(),
            change: TrackChange {
                name: Some(track.name.clone()),
                enabled: Some(track.enabled),
                locked: Some(track.locked),
                muted: Some(track.muted),
            },
        });
        if track.role.is_some() || !track.sync_locked {
            ops.push(Op::SetTrackRole {
                sequence: sequence.id.clone(),
                track: track.id.clone(),
                change: TrackRoleChange {
                    role: Some(track.role.clone()),
                    sync_locked: Some(track.sync_locked),
                },
            });
        }
    }
}
