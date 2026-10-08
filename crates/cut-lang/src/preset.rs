//! Portable editing patterns, written in the Cut language. Media declarations
//! are typed slots (`path="slot:slot1"`), never references to the author's files.
//! Instantiation produces an ordinary edit script for the runtime to propose.

use crate::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use cut_model::{
    Catalog, ClipChange, Content, Id, Media, NewClip, Op, Param, Project, Sequence, SequenceChange,
    Time, TrackChange, TrackKind, Value,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Binding {
    pub media: Id,
    /// Offset from the replacement media's first frame, including handles.
    pub from: Time,
}

fn diagnostics(errors: impl IntoIterator<Item = cut_model::Diagnostic>) -> String {
    errors
        .into_iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn valid(project: &Project, catalog: &Catalog) -> Result<(), String> {
    let errors: Vec<_> = cut_model::validate(project, catalog)
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
                let Some((mut low, mut high)) = clip.source_range() else {
                    continue;
                };
                let speed = clip.speed();
                let Content::Media { media, .. } = &mut clip.content else {
                    continue;
                };
                for transition in &track.transitions {
                    let (before, after) = transition.split_at_cut(sequence.rate);
                    if transition.to == clip.id {
                        if speed.is_reverse() {
                            high += speed.source_span(before);
                        } else {
                            low -= speed.source_span(before);
                        }
                    }
                    if transition.from == clip.id {
                        if speed.is_reverse() {
                            low -= speed.source_span(after);
                        } else {
                            high += speed.source_span(after);
                        }
                    }
                }
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
    Ok(print_document(&pattern, catalog, PrintOptions::default()))
}

/// Read and validate a portable preset. Every media input must be a slot;
/// unbound file paths and hidden, unused sequences are rejected.
pub fn parse(document: &str, catalog: &Catalog) -> Result<Project, String> {
    let project = parse_document(document, catalog)
        .map_err(diagnostics)?
        .project;
    valid(&project, catalog)?;
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
    Ok(project)
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
            Content::Generator { fx, params } if fx == "title" => {
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
    let mut pattern = parse(document, catalog)?;
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
        if (slot.video.is_some() && media.video.is_none())
            || (slot.audio.is_some() && media.audio.is_none())
        {
            return Err(format!(
                "slot `{}` needs {}{}",
                slot.id,
                if slot.video.is_some() { "video" } else { "" },
                if slot.audio.is_some() { " audio" } else { "" }
            ));
        }
        if binding.from.is_negative()
            || binding.from > media.duration
            || slot.duration > media.duration - binding.from
        {
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
                match &mut clip.content {
                    Content::Media {
                        media, source_in, ..
                    } => {
                        let binding = &bindings[media.as_str()];
                        *media = binding.media.clone();
                        *source_in += binding.from;
                    }
                    Content::Sequence { sequence, .. } => *sequence = ids[sequence].clone(),
                    _ => {}
                }
            }
        }
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

fn append_sequence(sequence: &Sequence, ops: &mut Vec<Op>) {
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
    }
}
