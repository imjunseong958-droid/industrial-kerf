//! Portable editing patterns, written in the Kerf language. A media
//! declaration is a typed slot (`path="slot:slot1"`), never a reference to the
//! author's files, or a media the preset makes from the steps under its line
//! (`path="made:man.mp4"`), whose steps name slots for the footage they read
//! (`guide=slot:slot1`). The sequences those steps draw, and the looks a
//! pattern uses that Kerf does not bundle, travel with it. Instantiation
//! produces an ordinary edit script for the runtime to propose.

use crate::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use kerf_model::look::{self, Look};
use kerf_model::making::{Named, Step};
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

/// Sequences, and what they nest, in children-first order from each of `roots`
/// in turn, each once.
fn order_all(project: &Project, roots: &[Id]) -> Result<Vec<Id>, String> {
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
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for root in roots {
        visit(project, root, &mut seen, &mut out)?;
    }
    Ok(out)
}

/// What a media's path is in a preset where the project it is applied to
/// supplies the media: `slot:<id>`.
pub const SLOT: &str = "slot:";
/// What a media's path starts with in a preset that makes the media from the
/// steps under its line: `made:<id>`, and its file's extension.
pub const MADE: &str = "made:";
/// The folder of the project a preset is applied to that a media it makes is
/// written in, as `shots/<id>.mp4`, its takes beside it in `shots/<id>.takes/`.
pub const MADE_FOLDER: &str = "shots";

/// Whether `media` of a preset is one it makes from its steps, not a slot.
pub fn is_made(media: &Media) -> bool {
    media.path.starts_with(MADE)
}

/// The slots of a preset's pattern: the media the project it is applied to
/// supplies, each bound to one of its own.
pub fn slots(pattern: &Project) -> impl Iterator<Item = &Media> {
    pattern.media.iter().filter(|media| !is_made(media))
}

/// The media a preset's pattern makes from the steps under their lines. They
/// take no binding: applying the preset adds each to the project.
pub fn made(pattern: &Project) -> impl Iterator<Item = &Media> {
    pattern.media.iter().filter(|media| is_made(media))
}

/// The slot a media the preset makes is of, frame for frame: where its steps
/// read a slot's file (`guide=slot:slot1`), every slot they read is that one,
/// and no step draws a sequence. A matte or a plate of footage is of it; a
/// shot from a guide is not.
pub fn footage_of(media: &Media) -> Option<Id> {
    if !is_made(media) {
        return None;
    }
    let mut slot = None;
    for step in &media.steps {
        for (named, name) in step.names() {
            match (named, name.strip_prefix(SLOT)) {
                (Named::Sequence, _) => return None,
                (Named::File, Some(read)) => match slot {
                    None => slot = Some(read),
                    Some(one) if one == read => {}
                    Some(_) => return None,
                },
                _ => {}
            }
        }
    }
    slot.and_then(|id| Id::new(id).ok())
}

/// The extension of the file at `path`, with its dot (`.mp4`), or nothing
/// where it has none a made media's path can keep.
fn extension(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 && plain_extension(&name[dot + 1..]) => &name[dot..],
        _ => "",
    }
}

fn plain_extension(extension: &str) -> bool {
    !extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

/// What of a project travels in a preset of `root`: its sequences, the media
/// its steps make, and the footage those steps read.
struct Travelling {
    /// The root first, then each sequence it nests or a step names.
    sequences: Vec<Id>,
    /// The media with steps that a clip plays or a step reads.
    made: Vec<Id>,
    /// The footage a step reads, by its path.
    footage: BTreeMap<String, Id>,
}

/// What travels with `root`: what it nests; each media with steps a clip of
/// what travels plays; the sequences those steps draw and the ones that hold
/// the clips they name; and so on, until nothing more does. A media a step
/// reads by its path travels as made where it has steps, and is footage to
/// bind where it has none and names no recipe file.
fn travelling(project: &Project, root: &Id) -> Result<Travelling, String> {
    let mut travel = Travelling {
        sequences: Vec::new(),
        made: Vec::new(),
        footage: BTreeMap::new(),
    };
    let mut sequences = vec![root.clone()];
    let mut made: Vec<Id> = Vec::new();
    let home = |clip: &str| {
        project
            .sequences
            .iter()
            .find(|s| s.tracks.iter().flat_map(|t| &t.clips).any(|c| c.id == clip))
            .map(|s| s.id.clone())
    };
    loop {
        if let Some(id) = sequences.pop() {
            if travel.sequences.contains(&id) {
                continue;
            }
            let sequence = project
                .sequence(id.as_str())
                .ok_or_else(|| format!("no sequence `{id}`"))?;
            travel.sequences.push(id);
            for clip in sequence.tracks.iter().flat_map(|t| &t.clips) {
                match &clip.content {
                    Content::Sequence { sequence, .. } => sequences.push(sequence.clone()),
                    Content::Media { media, .. } => {
                        let source = project.media(media.as_str()).expect("validated media");
                        if let Some(recipe) = &source.recipe {
                            return Err(format!(
                                "the steps of media `{media}` are in the recipe file `{recipe}`, which a preset does not carry: carry them into the document first (`kerf carry {media}`, or an agent's `edit` with `carry`)"
                            ));
                        }
                        if !source.steps.is_empty() {
                            made.push(media.clone());
                        }
                    }
                    _ => {}
                }
            }
            continue;
        }
        let Some(id) = made.pop() else {
            break;
        };
        if travel.made.contains(&id) {
            continue;
        }
        let media = project.media(id.as_str()).expect("validated media");
        travel.made.push(id.clone());
        for step in &media.steps {
            for (named, name) in step.names() {
                match named {
                    Named::Sequence => sequences.push(
                        project
                            .sequence(name)
                            .ok_or_else(|| {
                                format!("media `{id}`'s steps draw sequence `{name}`, which the project has not")
                            })?
                            .id
                            .clone(),
                    ),
                    Named::Clip => sequences.push(home(name).ok_or_else(|| {
                        format!("media `{id}`'s steps name clip `{name}`, which no sequence of the project has")
                    })?),
                    Named::File => {
                        let Some(read) = project.media.iter().find(|m| m.path == name) else {
                            continue;
                        };
                        if !read.steps.is_empty() {
                            made.push(read.id.clone());
                        } else if read.recipe.is_none() {
                            travel.footage.insert(name.to_owned(), read.id.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(travel)
}

/// Export a sequence, the sequences it nests, and those the steps of a media
/// it plays draw. Each cut of footage has a separate media slot; linked
/// picture and sound from the same source share one, and footage the steps of
/// a media read is one slot for every cut of it. Source ranges start at zero,
/// with just enough room for the original transition handles. A media with
/// steps is one the preset makes (`made:<id>`), its steps naming the slots
/// for the footage they read.
pub fn export(
    project: &Project,
    catalog: &Catalog,
    sequence: &Id,
    name: &str,
) -> Result<String, String> {
    valid(project, catalog)?;
    let travel = travelling(project, sequence)?;
    let mut pattern = Project::new(name);
    for id in &travel.sequences {
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
    // Each slot and the source it plays, where a clip plays it.
    let mut ranges: Vec<(Media, Option<(Time, Time)>)> = Vec::new();
    let new_slot = |source: &Media, id: Id| {
        let mut slot = source.clone();
        slot.path = format!("{SLOT}{id}");
        slot.id = id;
        slot.start = Time::ZERO;
        slot.reel = None;
        slot.recipe = None;
        // What it was recorded beside is the project's, and the pattern has no such media.
        slot.sync = None;
        slot.steps.clear();
        slot.video = None;
        slot.audio = None;
        slot
    };
    // Footage a making reads is one slot: the making is of that one file.
    let mut footage_slots = BTreeMap::<Id, Id>::new();
    for footage in travel.footage.values() {
        let source = project.media(footage.as_str()).expect("validated media");
        let mut slot = new_slot(source, slot_ids.next("slot"));
        // A model reads its picture.
        slot.video = source.video;
        footage_slots.insert(footage.clone(), slot.id.clone());
        groups.insert((footage.clone(), footage.clone()), ranges.len());
        ranges.push((slot, None));
    }
    let made_paths: BTreeMap<String, String> = travel
        .made
        .iter()
        .map(|id| {
            let media = project.media(id.as_str()).expect("validated media");
            (
                media.path.clone(),
                format!("{MADE}{id}{}", extension(&media.path)),
            )
        })
        .collect();
    let mut made: Vec<Media> = travel
        .made
        .iter()
        .map(|id| {
            let mut media = project.media(id.as_str()).expect("validated media").clone();
            media.path.clone_from(&made_paths[&media.path]);
            media.start = Time::ZERO;
            media.reel = None;
            media.sync = None;
            for step in &mut media.steps {
                step.rename(|named, name| match named {
                    Named::File => travel
                        .footage
                        .get(name)
                        .map(|footage| format!("{SLOT}{}", footage_slots[footage]))
                        .or_else(|| made_paths.get(name).cloned()),
                    Named::Sequence | Named::Clip => None,
                });
            }
            media
        })
        .collect();
    // The slot each made media is of, frame for frame, by the slot's index.
    let made_of: BTreeMap<Id, usize> = made
        .iter()
        .filter_map(|media| {
            let slot = footage_of(media)?;
            let index = ranges.iter().position(|(m, _)| m.id == slot)?;
            Some((media.id.clone(), index))
        })
        .collect();
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
                let index = if travel.made.contains(media) {
                    // A matte is over its own frames of the footage: the footage's slot
                    // holds what its clips play too.
                    match made_of.get(media) {
                        Some(index) => *index,
                        None => continue,
                    }
                } else {
                    let source = project.media(media.as_str()).expect("validated media");
                    let key = match footage_slots.contains_key(media) {
                        true => (media.clone(), media.clone()),
                        false => (
                            clip.link.clone().unwrap_or_else(|| clip.id.clone()),
                            media.clone(),
                        ),
                    };
                    let next = ranges.len();
                    let index = *groups.entry(key).or_insert_with(|| {
                        ranges.push((new_slot(source, slot_ids.next("slot")), None));
                        next
                    });
                    let slot = &mut ranges[index].0;
                    match track.kind {
                        TrackKind::Video => slot.video = source.video,
                        TrackKind::Audio => slot.audio = source.audio,
                        TrackKind::Subtitle => unreachable!("validated media track"),
                    }
                    *media = slot.id.clone();
                    index
                };
                let range = &mut ranges[index].1;
                *range = Some(match *range {
                    Some((from, to)) => (from.min(low), to.max(high)),
                    None => (low, high),
                });
            }
        }
    }
    // A slot no clip plays is the whole of the footage a making reads.
    let ranges: Vec<(Media, Time, Time)> = ranges
        .into_iter()
        .map(|(media, range)| {
            let (from, to) = range.unwrap_or((Time::ZERO, media.duration));
            (media, from, to)
        })
        .collect();
    let mut offsets: BTreeMap<_, _> = ranges
        .iter()
        .map(|(m, from, _)| (m.id.clone(), *from))
        .collect();
    for media in &mut made {
        if let Some(index) = made_of.get(&media.id) {
            let (_, from, to) = &ranges[*index];
            offsets.insert(media.id.clone(), *from);
            media.duration = *to - *from;
        }
    }
    for sequence in &mut pattern.sequences {
        for clip in sequence.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if let Content::Media {
                media, source_in, ..
            } = &mut clip.content
            {
                if let Some(offset) = offsets.get(media) {
                    *source_in -= *offset;
                }
            }
        }
    }
    pattern.media = ranges
        .into_iter()
        .map(|(mut media, from, to)| {
            media.duration = to - from;
            media
        })
        .chain(made)
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
            speed.played_covering(-before, Time::ZERO)
        } else {
            Time::ZERO
        };
        let tail = if transition.from == clip.id {
            speed.played_covering(clip.duration, clip.duration + after)
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
    if !project.retired.is_empty() {
        return Err("presets cannot contain retired ids".into());
    }
    let clips: Vec<&Clip> = project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .flat_map(|t| &t.clips)
        .collect();
    // The files the preset's makings read, as their steps name them.
    let read: BTreeSet<&str> = project
        .media
        .iter()
        .flat_map(|m| &m.steps)
        .flat_map(Step::names)
        .filter_map(|(named, name)| (named == Named::File).then_some(name))
        .collect();
    for media in &project.media {
        if media.start != Time::ZERO || media.reel.is_some() {
            return Err(format!(
                "media `{}` must be a portable slot with zero timecode",
                media.id
            ));
        }
        if is_made(media) {
            made_media(&project, media)?;
        } else if media.path != format!("{SLOT}{}", media.id) {
            return Err(format!(
                "media `{}` must be a portable slot with zero timecode, `path=\"{SLOT}{}\"`, or one the preset makes, `path=\"{MADE}{}.mp4\"` with the steps that make it under its line",
                media.id, media.id, media.id
            ));
        } else if !media.steps.is_empty() {
            return Err(format!(
                "slot `{}` has steps under it: a media the preset makes has the path `{MADE}{}` and its file's extension",
                media.id, media.id
            ));
        }
        let played = clips
            .iter()
            .any(|c| matches!(&c.content, Content::Media { media: id, .. } if *id == media.id));
        if !played && !read.contains(media.path.as_str()) {
            return Err(match is_made(media) {
                true => format!("unused made media `{}`: no clip plays it and no step reads it", media.id),
                false => format!("unused slot `{}`", media.id),
            });
        }
    }
    if order_all(&project, &roots(&project))?.len() != project.sequences.len() {
        return Err("the preset contains sequences outside its pattern".into());
    }
    Ok(Preset {
        pattern: project,
        looks,
        catalog,
    })
}

/// Where a pattern's sequences are reached from: its first, and each a step
/// of a media it makes draws or holds a clip a step names.
fn roots(pattern: &Project) -> Vec<Id> {
    let mut roots: Vec<Id> = pattern.sequences.first().map(|s| s.id.clone()).into_iter().collect();
    for (named, name) in made(pattern).flat_map(|m| &m.steps).flat_map(Step::names) {
        let found = match named {
            Named::Sequence => pattern.sequence(name).map(|s| s.id.clone()),
            Named::Clip => pattern
                .sequences
                .iter()
                .find(|s| s.tracks.iter().flat_map(|t| &t.clips).any(|c| c.id == name))
                .map(|s| s.id.clone()),
            Named::File => None,
        };
        if let Some(id) = found.filter(|id| !roots.contains(id)) {
            roots.push(id);
        }
    }
    roots
}

/// Check a media the preset makes: its path is `made:<id>` and an extension,
/// it names no recipe file, it has steps, and what they name is the pattern's.
fn made_media(pattern: &Project, media: &Media) -> Result<(), String> {
    let id = &media.id;
    let rest = &media.path[MADE.len()..];
    let fits = rest
        .strip_prefix(id.as_str())
        .is_some_and(|ext| ext.is_empty() || ext.strip_prefix('.').is_some_and(plain_extension));
    if !fits {
        return Err(format!(
            "made media `{id}` has the path `{MADE}{id}` and its file's extension, such as `{MADE}{id}.mp4`"
        ));
    }
    if media.recipe.is_some() {
        return Err(format!(
            "made media `{id}` names a recipe file: a preset carries the steps under its line"
        ));
    }
    if media.steps.is_empty() {
        return Err(format!(
            "made media `{id}` has no steps: a preset makes it from the steps under its line"
        ));
    }
    let has_clip = |name: &str| {
        pattern
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .any(|c| c.id == name)
    };
    for (named, name) in media.steps.iter().flat_map(Step::names) {
        let missing = match named {
            Named::Sequence => pattern.sequence(name).is_none().then_some("sequence"),
            Named::Clip => (!has_clip(name)).then_some("clip"),
            Named::File => match (name.strip_prefix(SLOT), name.strip_prefix(MADE)) {
                (Some(slot), _) => pattern
                    .media(slot)
                    .is_none_or(is_made)
                    .then_some("slot"),
                (_, Some(_)) => (!made(pattern).any(|m| m.path == name)).then_some("made media"),
                // A file of the project it is applied to, which its making reads there.
                _ => None,
            },
        };
        if let Some(kind) = missing {
            return Err(format!(
                "made media `{id}`'s steps name {kind} `{name}`, which the preset has not"
            ));
        }
    }
    Ok(())
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

/// Why a binding of a media the preset makes is refused.
pub fn made_takes_no_binding(id: &str) -> String {
    format!("`{id}` is made by the preset from the steps under its line, so it takes no binding; bind the slots its steps read")
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
    instantiate_with_ids(project, catalog, document, target, bindings, texts)
        .map(|(script, _)| script)
}

/// [`instantiate`], with the id each of the pattern's sequences, clips,
/// markers, transitions and links was given in the project, by the id the
/// pattern has for it: what was `sa` in the preset may be `p9` here, and
/// whoever applied it goes on by that name. A slot is given the media it
/// was bound to, and the pattern's first sequence the target's id. A media
/// the preset makes is given the id of the new media it became, whose file
/// is `shots/<id>` and the extension it had, and whose steps are added after
/// the sequences they draw, naming the ids those were given and the paths of
/// the media bound to the slots they read.
pub fn instantiate_with_ids(
    project: &Project,
    catalog: &Catalog,
    document: &str,
    target: &Id,
    bindings: &BTreeMap<String, Binding>,
    texts: &BTreeMap<String, String>,
) -> Result<(String, BTreeMap<String, String>), String> {
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
        match pattern.media(slot) {
            None => return Err(format!("unknown slot `{slot}`")),
            Some(media) if is_made(media) => return Err(made_takes_no_binding(slot)),
            Some(_) => {}
        }
    }
    for slot in slots(&pattern) {
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
    let sequence_order = order_all(&pattern, &roots(&pattern))?;
    let mut generator = project.id_gen();
    // The clips that play music a binding re-times, and its onsets.
    let mut beats = BTreeMap::<Id, &[Time]>::new();
    let mut ids = BTreeMap::new();
    ids.insert(root.clone(), target.clone());
    // A media the preset makes is a new one of the project's, its file by its id.
    let mut made_paths = BTreeMap::<String, String>::new();
    for media in made(&pattern) {
        let extension = &media.path[MADE.len() + media.id.as_str().len()..];
        let (id, path) = loop {
            let id = generator.next("p");
            let path = format!("{MADE_FOLDER}/{id}{extension}");
            if !project.media.iter().any(|m| m.path == path) {
                break (id, path);
            }
        };
        ids.insert(media.id.clone(), id);
        made_paths.insert(media.path.clone(), path);
    }
    for id in pattern.ids() {
        if !ids.contains_key(id) {
            ids.insert(id.clone(), generator.next("p"));
        }
    }
    // Each made media as the project has it: a matte or a plate of footage is
    // of the media bound to its slot, frame for frame, and its clips move with
    // that slot's.
    let mut made_shift = BTreeMap::<Id, Time>::new();
    let mut made_media = Vec::new();
    for media in made(&pattern) {
        let mut placed = media.clone();
        placed.id = ids[&media.id].clone();
        placed.path.clone_from(&made_paths[&media.path]);
        if let Some(slot) = footage_of(media) {
            let binding = &bindings[slot.as_str()];
            let bound = project.media(binding.media.as_str()).expect("checked binding");
            placed.rate = bound.rate;
            placed.drop_frame = bound.drop_frame;
            placed.duration = bound.duration;
            placed.video = bound.video;
            made_shift.insert(media.id.clone(), binding.from);
        }
        for step in &mut placed.steps {
            step.rename(|named, name| match named {
                Named::Sequence | Named::Clip => {
                    Id::new(name).ok().and_then(|id| ids.get(&id)).map(Id::to_string)
                }
                Named::File => match name.strip_prefix(SLOT) {
                    Some(slot) => bindings
                        .get(slot)
                        .and_then(|binding| project.media(binding.media.as_str()))
                        .map(|bound| bound.path.clone()),
                    None => made_paths.get(name).cloned(),
                },
            });
        }
        made_media.push(placed);
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
                    } => match bindings.get(media.as_str()) {
                        Some(binding) => {
                            *media = binding.media.clone();
                            *source_in += binding.from;
                            if !binding.beats.is_empty() {
                                beats.insert(clip.id.clone(), &binding.beats);
                            }
                        }
                        None => {
                            if let Some(shift) = made_shift.get(media) {
                                *source_in += *shift;
                            }
                            *media = ids[media].clone();
                        }
                    },
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
    // The media first, for the clips that play them; their steps last, once
    // the sequences they draw and the clips they name are there.
    let mut steps = Vec::new();
    for mut media in made_media {
        for step in std::mem::take(&mut media.steps) {
            steps.push(Op::AddStep {
                media: media.id.clone(),
                index: None,
                step,
            });
        }
        ops.push(Op::AddMedia(media));
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
                    color: Some(sequence.color),
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
    ops.extend(steps);
    let script = print_ops(project, catalog, &ops);
    apply_script(project, catalog, &script).map_err(diagnostics)?;
    // A slot became the media it was bound to.
    let ids = ids
        .into_iter()
        .map(|(pattern, placed)| {
            let placed = bindings
                .get(pattern.as_str())
                .map_or(placed, |binding| binding.media.clone());
            (pattern.to_string(), placed.to_string())
        })
        .collect();
    Ok((script, ids))
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
    // Every track before any clip: an effect may name another track, as a
    // track matte names the one above it.
    for track in &sequence.tracks {
        ops.push(Op::AddTrack {
            sequence: sequence.id.clone(),
            id: track.id.clone(),
            kind: track.kind,
            index: None,
        });
    }
    for track in &sequence.tracks {
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
    for bus in &sequence.buses {
        ops.push(Op::AddBus {
            sequence: sequence.id.clone(),
            bus: bus.clone(),
        });
    }
    // Lock only after all content and link groups have been installed.
    for track in &sequence.tracks {
        ops.push(Op::SetTrack {
            sequence: sequence.id.clone(),
            track: track.id.clone(),
            change: TrackChange {
                bus: Some(track.bus.clone()),
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
