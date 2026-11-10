//! Structural three-way merges of `project.kerf`.
//!
//! Git merges lines, and two people editing one project collide on lines
//! constantly: both append a clip to a track, both allocate `c7`. Here the
//! three versions are read into the model and merged element by element, by
//! id, and the result is printed as the canonical document again.

use kerf_lang::{parse_document, print_document, PrintOptions};
use kerf_model::{
    Catalog, Clip, Content, Diagnostic, Effect, Id, IdGen, Marker, Media, Param, ParamDef,
    ParamType, Params, Project, Sequence, Track, Transition, Value,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Something both sides changed that the merge settled one way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// Where, as a path of ids: "media m2", "sequence s1", "s1/V1", "s1/V1/c3", "s1/marker beat1".
    pub path: String,
    /// What collided, in plain words: "start, duration changed on both sides",
    /// "removed here, changed there", "overlapped c5 on V1; moved to V3".
    pub what: String,
    /// "ours" or "theirs": whose version the merged project keeps.
    pub kept: &'static str,
}

/// A merged project, what collided in it, and what was renamed.
#[derive(Debug, Clone)]
pub struct Merged {
    pub project: Project,
    pub conflicts: Vec<Conflict>,
    /// Ids added on both sides for different things: (theirs' old id, its new id).
    pub renamed: Vec<(Id, Id)>,
}

/// A merged project as its canonical document.
#[derive(Debug, Clone)]
pub struct MergedDocument {
    pub text: String,
    pub conflicts: Vec<Conflict>,
    pub renamed: Vec<(Id, Id)>,
}

/// Parse the three documents (an empty `base` means the file was added on
/// both sides, from an empty project), merge, and print the canonical text.
pub fn merge_documents(
    base: &str,
    ours: &str,
    theirs: &str,
    catalog: &Catalog,
) -> Result<MergedDocument, String> {
    let parse = |text: &str, side: &str| {
        parse_document(text, catalog)
            .map(|document| document.project)
            .map_err(|errors| format!("{side} does not parse: {}", list(&errors)))
    };
    let base = if base.trim().is_empty() {
        Project::default()
    } else {
        parse(base, "the base")?
    };
    let merged = merge_projects(
        &base,
        &parse(ours, "ours")?,
        &parse(theirs, "theirs")?,
        catalog,
    )?;
    Ok(MergedDocument {
        text: print_document(&merged.project, catalog, PrintOptions::default()),
        conflicts: merged.conflicts,
        renamed: merged.renamed,
    })
}

/// Merge `ours` and `theirs`, both edited from `base`, element by element.
/// `Err` says why when the result cannot be made valid.
pub fn merge_projects(
    base: &Project,
    ours: &Project,
    theirs: &Project,
    catalog: &Catalog,
) -> Result<Merged, String> {
    let mut renamed_theirs = theirs.clone();
    let renamed = rename_collisions(base, ours, &mut renamed_theirs, catalog);
    let (b, o, t) = (Flat::of(base), Flat::of(ours), Flat::of(&renamed_theirs));
    let mut conflicts = Vec::new();
    let mut clashes = Vec::new();
    let name = pick("name", &base.name, &ours.name, &theirs.name, &mut clashes);
    if !clashes.is_empty() {
        conflicts.push(Conflict {
            path: "project".to_owned(),
            what: changed_on_both(&clashes),
            kept: "ours",
        });
    }
    let mut flat = Flat {
        media: merge_list(
            [&b.media, &o.media, &t.media],
            merge_media,
            |id, _| format!("media {id}"),
            &mut conflicts,
        ),
        sequences: merge_list(
            [&b.sequences, &o.sequences, &t.sequences],
            merge_sequence,
            |id, _| format!("sequence {id}"),
            &mut conflicts,
        ),
        tracks: merge_list(
            [&b.tracks, &o.tracks, &t.tracks],
            merge_track,
            |(sequence, track), _| format!("{sequence}/{track}"),
            &mut conflicts,
        ),
        clips: merge_list(
            [&b.clips, &o.clips, &t.clips],
            merge_clip,
            Placed::path,
            &mut conflicts,
        ),
        transitions: merge_list(
            [&b.transitions, &o.transitions, &t.transitions],
            whole,
            Placed::path,
            &mut conflicts,
        ),
        markers: merge_list(
            [&b.markers, &o.markers, &t.markers],
            whole,
            |(sequence, id), _| format!("{sequence}/marker {id}"),
            &mut conflicts,
        ),
    };
    restore(&mut flat, &o, &t, catalog, &mut conflicts);
    let mut project = flat.build(name)?;
    let mut track_ids: HashMap<Id, Vec<Id>> = HashMap::new();
    for ((sequence, track), _) in b.tracks.iter().chain(&o.tracks).chain(&t.tracks) {
        track_ids
            .entry(sequence.clone())
            .or_default()
            .push(track.clone());
    }
    separate_overlaps(&mut project, &o, &track_ids, &mut conflicts);
    repair_transitions(&mut project, &o, catalog, &mut conflicts);
    let live: HashSet<&Id> = project.ids().collect();
    let retired = [base, ours, theirs]
        .iter()
        .flat_map(|side| side.retired.iter().chain(side.ids()))
        .filter(|id| !live.contains(id))
        .cloned()
        .collect();
    project.retired = retired;
    let errors = kerf_model::validate::errors(&project, catalog);
    if !errors.is_empty() {
        return Err(format!(
            "the merged project is not valid: {}",
            list(&errors)
        ));
    }
    Ok(Merged {
        project,
        conflicts,
        renamed,
    })
}

fn list(diagnostics: &[Diagnostic]) -> String {
    let lines: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
    lines.join("; ")
}

// ---- elements ---------------------------------------------------------------

/// A clip or transition with the sequence and track it sits on.
#[derive(Clone, Debug, PartialEq)]
struct Placed<T> {
    sequence: Id,
    track: Id,
    item: T,
}

impl<T> Placed<T> {
    fn new(sequence: &Id, track: &Id, item: T) -> Self {
        Self {
            sequence: sequence.clone(),
            track: track.clone(),
            item,
        }
    }

    /// `s1/V1/c3`.
    fn path(id: &Id, placed: &Self) -> String {
        format!("{}/{}/{id}", placed.sequence, placed.track)
    }
}

/// A project as lists of elements keyed by identity, in document order.
#[derive(Default)]
struct Flat {
    media: Vec<(Id, Media)>,
    /// Without their tracks and markers.
    sequences: Vec<(Id, Sequence)>,
    /// By sequence and track, without their clips and transitions.
    tracks: Vec<((Id, Id), Track)>,
    clips: Vec<(Id, Placed<Clip>)>,
    transitions: Vec<(Id, Placed<Transition>)>,
    /// By sequence and marker.
    markers: Vec<((Id, Id), Marker)>,
}

impl Flat {
    fn of(project: &Project) -> Self {
        let mut flat = Self {
            media: project
                .media
                .iter()
                .map(|media| (media.id.clone(), media.clone()))
                .collect(),
            ..Self::default()
        };
        for sequence in &project.sequences {
            let id = &sequence.id;
            let head = Sequence {
                id: id.clone(),
                name: sequence.name.clone(),
                buses: sequence.buses.clone(),
                tracks: Vec::new(),
                markers: Vec::new(),
                ..*sequence
            };
            flat.sequences.push((id.clone(), head));
            for track in &sequence.tracks {
                flat.tracks
                    .push(((id.clone(), track.id.clone()), track_head(track)));
                flat.clips.extend(
                    track
                        .clips
                        .iter()
                        .map(|clip| (clip.id.clone(), Placed::new(id, &track.id, clip.clone()))),
                );
                flat.transitions
                    .extend(track.transitions.iter().map(|transition| {
                        let placed = Placed::new(id, &track.id, transition.clone());
                        (transition.id.clone(), placed)
                    }));
            }
            flat.markers.extend(
                sequence
                    .markers
                    .iter()
                    .map(|marker| ((id.clone(), marker.id.clone()), marker.clone())),
            );
        }
        flat
    }

    /// The project these elements make: clips in start order, markers in
    /// time order, nothing retired yet.
    fn build(self, name: String) -> Result<Project, String> {
        let lost = |what: &Id, container: &Id| {
            format!("the merge kept {what} but neither side has {container}, which holds it")
        };
        let mut project = Project {
            name,
            media: self.media.into_iter().map(|(_, media)| media).collect(),
            sequences: self.sequences.into_iter().map(|(_, s)| s).collect(),
            retired: BTreeSet::new(),
        };
        for ((sequence, id), track) in self.tracks {
            sequence_mut(&mut project, &sequence)
                .ok_or_else(|| lost(&id, &sequence))?
                .tracks
                .push(track);
        }
        for (id, placed) in self.clips {
            track_mut(&mut project, &placed.sequence, &placed.track)
                .ok_or_else(|| lost(&id, &placed.track))?
                .clips
                .push(placed.item);
        }
        for (id, placed) in self.transitions {
            track_mut(&mut project, &placed.sequence, &placed.track)
                .ok_or_else(|| lost(&id, &placed.track))?
                .transitions
                .push(placed.item);
        }
        for ((sequence, id), marker) in self.markers {
            sequence_mut(&mut project, &sequence)
                .ok_or_else(|| lost(&id, &sequence))?
                .markers
                .push(marker);
        }
        for sequence in &mut project.sequences {
            sequence.markers.sort_by_key(|marker| marker.at);
            for track in &mut sequence.tracks {
                track.clips.sort_by_key(|clip| clip.start);
            }
        }
        Ok(project)
    }
}

/// A track without its clips and transitions.
fn track_head(track: &Track) -> Track {
    Track {
        name: track.name.clone(),
        enabled: track.enabled,
        locked: track.locked,
        muted: track.muted,
        role: track.role.clone(),
        sync_locked: track.sync_locked,
        ..Track::new(track.id.clone(), track.kind)
    }
}

fn sequence_mut<'a>(project: &'a mut Project, id: &Id) -> Option<&'a mut Sequence> {
    project
        .sequences
        .iter_mut()
        .find(|sequence| sequence.id == *id)
}

fn track_mut<'a>(project: &'a mut Project, sequence: &Id, track: &Id) -> Option<&'a mut Track> {
    sequence_mut(project, sequence)?
        .tracks
        .iter_mut()
        .find(|found| found.id == *track)
}

/// The parameter definitions of a catalog effect, transition, or generator.
fn params_of<'a>(catalog: &'a Catalog, fx: &Id) -> &'a [ParamDef] {
    catalog
        .fx(fx.as_str())
        .map_or(&[], |def| def.params.as_slice())
}

/// The ids that parameters of the types `names` picks hold.
fn named<'a>(
    defs: &'a [ParamDef],
    params: &'a Params,
    names: fn(&ParamType) -> bool,
) -> impl Iterator<Item = Id> + 'a {
    params.iter().filter_map(move |(name, param)| {
        match (defs.iter().find(|def| def.name == *name), param) {
            (Some(def), Param::Static(Value::Text(text))) if names(&def.ty) => {
                Id::new(text.as_str()).ok()
            }
            _ => None,
        }
    })
}

/// The markers an effect's times land on.
fn landings<'a>(catalog: &'a Catalog, effect: &'a Effect) -> impl Iterator<Item = Id> + 'a {
    let defs = params_of(catalog, &effect.fx);
    named(defs, &effect.params, |ty| *ty == ParamType::Time)
}

/// The tracks of its sequence that a clip's effects and generator read.
fn clip_reads<'a>(catalog: &'a Catalog, clip: &'a Clip) -> impl Iterator<Item = Id> + 'a {
    let generator = match &clip.content {
        Content::Generator { fx, params, .. } => Some((fx, params)),
        _ => None,
    };
    clip.effects
        .iter()
        .map(|effect| (&effect.fx, &effect.params))
        .chain(generator)
        .flat_map(|(fx, params)| track_reads(catalog, fx, params))
}

/// The tracks an effect, generator, or transition reads.
fn track_reads<'a>(
    catalog: &'a Catalog,
    fx: &Id,
    params: &'a Params,
) -> impl Iterator<Item = Id> + 'a {
    named(params_of(catalog, fx), params, |ty| {
        matches!(ty, ParamType::Track | ParamType::Sound)
    })
}

// ---- three-way merge --------------------------------------------------------

/// One field merged three ways: the side that changed it, or ours when both
/// changed it differently, which names it in `clashes`.
fn pick<T: Clone + PartialEq>(
    name: &str,
    base: &T,
    ours: &T,
    theirs: &T,
    clashes: &mut Vec<String>,
) -> T {
    if ours == base {
        return theirs.clone();
    }
    if theirs != base && theirs != ours {
        clashes.push(name.to_owned());
    }
    ours.clone()
}

/// An element merged as one unit.
fn whole<T: Clone + PartialEq>(base: &T, ours: &T, theirs: &T, clashes: &mut Vec<String>) -> T {
    pick("", base, ours, theirs, clashes)
}

/// "start, duration changed on both sides"; an element merged whole names
/// no field.
fn changed_on_both(clashes: &[String]) -> String {
    let named: Vec<&str> = clashes
        .iter()
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .collect();
    if named.is_empty() {
        "changed on both sides".to_owned()
    } else {
        format!("{} changed on both sides", named.join(", "))
    }
}

fn index<K: Ord, T>(list: &[(K, T)]) -> BTreeMap<&K, &T> {
    list.iter().map(|(key, item)| (key, item)).collect()
}

/// Merge one kind of element by key. An element changed on one side takes
/// that side's version, including its removal; changed on both, its fields
/// merge; removed on one side and changed on the other, ours wins. Ours'
/// order holds, and theirs' additions follow the element before them in
/// theirs, after any of ours' own additions there.
fn merge_list<K: Ord + Clone, T: Clone + PartialEq>(
    [base, ours, theirs]: [&[(K, T)]; 3],
    fields: impl Fn(&T, &T, &T, &mut Vec<String>) -> T,
    path: impl Fn(&K, &T) -> String,
    conflicts: &mut Vec<Conflict>,
) -> Vec<(K, T)> {
    let (in_base, in_theirs) = (index(base), index(theirs));
    let mut conflict = |key: &K, item: &T, what: String| {
        conflicts.push(Conflict {
            path: path(key, item),
            what,
            kept: "ours",
        });
    };
    let mut merged = Vec::with_capacity(ours.len());
    for (key, o) in ours {
        let kept = match (in_base.get(key), in_theirs.get(key)) {
            (None, None) => Some(o.clone()),
            (None, Some(&t)) => {
                if o != t {
                    conflict(key, o, "added differently on both sides".to_owned());
                }
                Some(o.clone())
            }
            (Some(&b), None) if o == b => None,
            (Some(_), None) => {
                conflict(key, o, "changed here, removed there".to_owned());
                Some(o.clone())
            }
            (Some(&b), Some(&t)) if o == b => Some(t.clone()),
            (Some(&b), Some(&t)) if t == b || t == o => Some(o.clone()),
            (Some(&b), Some(&t)) => {
                let mut clashes = Vec::new();
                let item = fields(b, o, t, &mut clashes);
                if !clashes.is_empty() {
                    conflict(key, o, changed_on_both(&clashes));
                }
                Some(item)
            }
        };
        merged.extend(kept.map(|item| (key.clone(), item)));
    }
    // Slot `i` holds theirs' additions that go before `merged[i]`.
    let after: BTreeMap<&K, usize> = merged
        .iter()
        .enumerate()
        .map(|(i, (key, _))| (key, i + 1))
        .collect();
    let ours_only = |i: usize| {
        merged
            .get(i)
            .is_some_and(|(key, _)| !in_base.contains_key(key) && !in_theirs.contains_key(key))
    };
    let mut slots: Vec<Vec<(K, T)>> = (0..=merged.len()).map(|_| Vec::new()).collect();
    let mut slot = 0;
    for (key, t) in theirs {
        if let Some(&next) = after.get(key) {
            slot = next;
            continue;
        }
        match in_base.get(key) {
            None => {
                while ours_only(slot) {
                    slot += 1;
                }
                slots[slot].push((key.clone(), t.clone()));
            }
            Some(&b) if t != b => conflict(key, t, "removed here, changed there".to_owned()),
            Some(_) => {}
        }
    }
    let mut slots = slots.into_iter();
    let mut out = slots.next().unwrap_or_default();
    for (element, added) in merged.into_iter().zip(slots) {
        out.push(element);
        out.extend(added);
    }
    out
}

/// The file merges as one unit, path and what it holds together: a relink
/// on one side and a replacement on the other must not make one record of
/// both. The reel and the recipe are labels, and each merges on its own.
fn merge_media(base: &Media, ours: &Media, theirs: &Media, clashes: &mut Vec<String>) -> Media {
    let file = |media: &Media| Media {
        reel: None,
        recipe: None,
        ..media.clone()
    };
    Media {
        reel: pick("reel", &base.reel, &ours.reel, &theirs.reel, clashes),
        recipe: pick(
            "recipe",
            &base.recipe,
            &ours.recipe,
            &theirs.recipe,
            clashes,
        ),
        ..pick("file", &file(base), &file(ours), &file(theirs), clashes)
    }
}

fn merge_sequence(
    base: &Sequence,
    ours: &Sequence,
    theirs: &Sequence,
    clashes: &mut Vec<String>,
) -> Sequence {
    let size = |s: &Sequence| (s.width, s.height);
    let (width, height) = pick("size", &size(base), &size(ours), &size(theirs), clashes);
    Sequence {
        id: ours.id.clone(),
        name: pick("name", &base.name, &ours.name, &theirs.name, clashes),
        rate: pick("rate", &base.rate, &ours.rate, &theirs.rate, clashes),
        drop_frame: pick(
            "drop frame",
            &base.drop_frame,
            &ours.drop_frame,
            &theirs.drop_frame,
            clashes,
        ),
        width,
        height,
        color: pick("color", &base.color, &ours.color, &theirs.color, clashes),
        sample_rate: pick(
            "sample rate",
            &base.sample_rate,
            &ours.sample_rate,
            &theirs.sample_rate,
            clashes,
        ),
        start: pick("start", &base.start, &ours.start, &theirs.start, clashes),
        // Buses merge as one, since what one feeds depends on the others.
        buses: pick("buses", &base.buses, &ours.buses, &theirs.buses, clashes),
        tracks: Vec::new(),
        markers: Vec::new(),
    }
}

fn merge_track(base: &Track, ours: &Track, theirs: &Track, clashes: &mut Vec<String>) -> Track {
    Track {
        id: ours.id.clone(),
        kind: pick("kind", &base.kind, &ours.kind, &theirs.kind, clashes),
        name: pick("name", &base.name, &ours.name, &theirs.name, clashes),
        enabled: pick(
            "enabled",
            &base.enabled,
            &ours.enabled,
            &theirs.enabled,
            clashes,
        ),
        locked: pick(
            "locked",
            &base.locked,
            &ours.locked,
            &theirs.locked,
            clashes,
        ),
        muted: pick("muted", &base.muted, &ours.muted, &theirs.muted, clashes),
        bus: pick("bus", &base.bus, &ours.bus, &theirs.bus, clashes),
        role: pick("role", &base.role, &ours.role, &theirs.role, clashes),
        sync_locked: pick(
            "sync lock",
            &base.sync_locked,
            &ours.sync_locked,
            &theirs.sync_locked,
            clashes,
        ),
        clips: Vec::new(),
        transitions: Vec::new(),
    }
}

fn merge_clip(
    base: &Placed<Clip>,
    ours: &Placed<Clip>,
    theirs: &Placed<Clip>,
    clashes: &mut Vec<String>,
) -> Placed<Clip> {
    let place = |placed: &Placed<Clip>| (placed.sequence.clone(), placed.track.clone());
    let (sequence, track) = pick("track", &place(base), &place(ours), &place(theirs), clashes);
    let (b, o, t) = (&base.item, &ours.item, &theirs.item);
    let start = pick("start", &b.start, &o.start, &t.start, clashes);
    let duration = pick("duration", &b.duration, &o.duration, &t.duration, clashes);
    let content = pick("content", &b.content, &o.content, &t.content, clashes);
    let enabled = pick("enabled", &b.enabled, &o.enabled, &t.enabled, clashes);
    let name = pick("name", &b.name, &o.name, &t.name, clashes);
    let link = pick("link", &b.link, &o.link, &t.link, clashes);
    let keys: BTreeSet<&String> = b
        .props
        .keys()
        .chain(o.props.keys())
        .chain(t.props.keys())
        .collect();
    let props = keys
        .into_iter()
        .filter_map(|key| {
            let [b, o, t] = [b, o, t].map(|clip| clip.props.get(key));
            pick(key, &b, &o, &t, clashes).map(|param| (key.clone(), param.clone()))
        })
        .collect();
    let masks = if o.masks == b.masks || t.masks == b.masks || t.masks == o.masks {
        pick("", &b.masks, &o.masks, &t.masks, clashes)
    } else {
        let named = |clip: &Clip| -> Vec<(Id, kerf_model::Mask)> {
            clip.masks
                .iter()
                .map(|mask| (mask.name.clone(), mask.clone()))
                .collect()
        };
        let mut masks = Vec::new();
        let merged = merge_list(
            [&named(b), &named(o), &named(t)],
            whole,
            |name, _| format!("mask {name}"),
            &mut masks,
        );
        clashes.extend(masks.into_iter().map(|conflict| conflict.path));
        merged.into_iter().map(|(_, mask)| mask).collect()
    };
    let parent = pick("parent", &b.parent, &o.parent, &t.parent, clashes);
    let retime = pick("retime", &b.retime, &o.retime, &t.retime, clashes);
    let angle = pick("angle", &b.angle, &o.angle, &t.angle, clashes);
    let effects = pick("effects", &b.effects, &o.effects, &t.effects, clashes);
    Placed {
        sequence,
        track,
        item: Clip {
            id: o.id.clone(),
            start,
            duration,
            content,
            enabled,
            name,
            parent,
            retime,
            angle,
            link,
            props,
            masks,
            effects,
        },
    }
}

// ---- ids added on both sides ------------------------------------------------

/// What an id names, compared to tell whether both sides added the same thing.
#[derive(PartialEq)]
enum Thing<'a> {
    Media(&'a Media),
    Sequence(&'a Sequence),
    Marker(&'a Id, &'a Marker),
    Clip(&'a Placed<Clip>),
    Transition(&'a Placed<Transition>),
    Link(BTreeSet<&'a Id>),
}

impl Thing<'_> {
    /// The pass that renames it: what it names is renamed in earlier passes.
    /// Tracks, named per sequence, are pass 1.
    fn pass(&self) -> usize {
        match self {
            Self::Media(_) | Self::Sequence(_) => 0,
            Self::Marker(..) => 2,
            Self::Clip(_) => 3,
            Self::Transition(_) | Self::Link(_) => 4,
        }
    }

    /// Whether both sides added the same thing under one id. Link groups
    /// that share a clip are one group that each side linked its own clips
    /// to: renaming theirs would pull the shared clip apart.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Link(mine), Self::Link(theirs)) => !mine.is_disjoint(theirs),
            _ => self == other,
        }
    }
}

/// Everything in the project-wide namespace, in document order.
fn things(flat: &Flat) -> Vec<(&Id, Thing<'_>)> {
    let mut things: Vec<(&Id, Thing)> = Vec::new();
    things.extend(
        flat.media
            .iter()
            .map(|(id, media)| (id, Thing::Media(media))),
    );
    things.extend(
        flat.sequences
            .iter()
            .map(|(id, s)| (id, Thing::Sequence(s))),
    );
    things.extend(
        flat.markers
            .iter()
            .map(|((sequence, id), marker)| (id, Thing::Marker(sequence, marker))),
    );
    things.extend(flat.clips.iter().map(|(id, clip)| (id, Thing::Clip(clip))));
    things.extend(
        flat.transitions
            .iter()
            .map(|(id, transition)| (id, Thing::Transition(transition))),
    );
    let mut links: Vec<(&Id, BTreeSet<&Id>)> = Vec::new();
    let mut groups: HashMap<&Id, usize> = HashMap::new();
    for (id, placed) in &flat.clips {
        let Some(link) = &placed.item.link else {
            continue;
        };
        let group = *groups.entry(link).or_insert_with(|| {
            links.push((link, BTreeSet::new()));
            links.len() - 1
        });
        links[group].1.insert(id);
    }
    things.extend(
        links
            .into_iter()
            .map(|(id, members)| (id, Thing::Link(members))),
    );
    things
}

/// Theirs' renames, applied wherever theirs names the ids.
#[derive(Default)]
struct Renames {
    ids: HashMap<Id, Id>,
    /// Track renames by sequence.
    tracks: HashMap<Id, HashMap<Id, Id>>,
}

impl Renames {
    fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.tracks.is_empty()
    }

    fn apply(&self, project: &mut Project, catalog: &Catalog) {
        let rename = |id: &mut Id| {
            if let Some(new) = self.ids.get(id) {
                *id = new.clone();
            }
        };
        for media in &mut project.media {
            rename(&mut media.id);
        }
        for sequence in &mut project.sequences {
            rename(&mut sequence.id);
            let tracks = self.tracks.get(&sequence.id);
            let params = |params: &mut Params, defs: &[ParamDef]| self.params(params, defs, tracks);
            for marker in &mut sequence.markers {
                rename(&mut marker.id);
            }
            for track in &mut sequence.tracks {
                if let Some(new) = tracks.and_then(|tracks| tracks.get(&track.id)) {
                    track.id = new.clone();
                }
                for clip in &mut track.clips {
                    rename(&mut clip.id);
                    if let Some(link) = &mut clip.link {
                        rename(link);
                    }
                    match &mut clip.content {
                        Content::Media { media, .. } => rename(media),
                        Content::Sequence { sequence, .. } => rename(sequence),
                        Content::Generator {
                            fx, params: values, ..
                        } => {
                            params(values, params_of(catalog, fx));
                        }
                        Content::Caption { .. } => {}
                    }
                    if let Some(domain) = track.kind.domain() {
                        params(&mut clip.props, catalog.props(domain));
                    }
                    for effect in &mut clip.effects {
                        params(&mut effect.params, params_of(catalog, &effect.fx));
                    }
                }
                for transition in &mut track.transitions {
                    rename(&mut transition.id);
                    rename(&mut transition.from);
                    rename(&mut transition.to);
                    params(&mut transition.params, params_of(catalog, &transition.fx));
                }
            }
        }
    }

    /// Rewrite parameters that name a renamed marker or track.
    fn params(&self, params: &mut Params, defs: &[ParamDef], tracks: Option<&HashMap<Id, Id>>) {
        for (name, param) in params.iter_mut() {
            let (Some(def), Param::Static(Value::Text(text))) =
                (defs.iter().find(|def| def.name == *name), param)
            else {
                continue;
            };
            let new = match def.ty {
                ParamType::Time => self.ids.get(text.as_str()),
                ParamType::Track | ParamType::Sound => {
                    tracks.and_then(|tracks| tracks.get(text.as_str()))
                }
                _ => None,
            };
            if let Some(new) = new {
                *text = new.to_string();
            }
        }
    }
}

/// What a fresh id in place of `id` starts with: `id` without its number.
fn prefix(id: &Id) -> &str {
    let text = id.as_str().trim_end_matches(|c: char| c.is_ascii_digit());
    &text[..text.len().min(Id::MAX_LEN - 8)]
}

/// Give theirs' additions that collide with ours' different additions fresh
/// ids, and rewrite theirs' references to them. Returns the renames.
fn rename_collisions(
    base: &Project,
    ours: &Project,
    theirs: &mut Project,
    catalog: &Catalog,
) -> Vec<(Id, Id)> {
    let in_base: HashSet<&Id> = base.ids().collect();
    let ours_flat = Flat::of(ours);
    let ours_things: HashMap<&Id, Thing> = things(&ours_flat).into_iter().collect();
    let mut ids = IdGen::new(base.ids().chain(ours.ids()).chain(theirs.ids()));
    for id in base
        .retired
        .iter()
        .chain(&ours.retired)
        .chain(&theirs.retired)
    {
        ids.retire(id);
    }
    let mut renamed = Vec::new();
    for pass in 0..5 {
        let mut renames = Renames::default();
        if pass == 1 {
            renames.tracks = track_collisions(base, ours, theirs, &mut renamed);
        } else {
            let flat = Flat::of(theirs);
            for (id, thing) in things(&flat) {
                if thing.pass() != pass || in_base.contains(id) {
                    continue;
                }
                let collides = match ours_things.get(id) {
                    Some(other) => !other.same(&thing),
                    None => ours.retired.contains(id),
                };
                if collides {
                    let new = ids.next(prefix(id));
                    renamed.push((id.clone(), new.clone()));
                    renames.ids.insert(id.clone(), new);
                }
            }
        }
        if !renames.is_empty() {
            renames.apply(theirs, catalog);
        }
    }
    renamed
}

/// Fresh track ids for tracks both sides added to a sequence under one id
/// but made differently.
fn track_collisions(
    base: &Project,
    ours: &Project,
    theirs: &Project,
    renamed: &mut Vec<(Id, Id)>,
) -> HashMap<Id, HashMap<Id, Id>> {
    let mut renames: HashMap<Id, HashMap<Id, Id>> = HashMap::new();
    for sequence in &theirs.sequences {
        let Some(other) = ours.sequence(sequence.id.as_str()) else {
            continue;
        };
        let before = base.sequence(sequence.id.as_str());
        let mut ids = IdGen::new(
            [before, Some(other), Some(sequence)]
                .into_iter()
                .flatten()
                .flat_map(|s| s.tracks.iter().map(|track| &track.id)),
        );
        for track in &sequence.tracks {
            let added = before.is_none_or(|s| s.track(track.id.as_str()).is_none());
            let collides = other
                .track(track.id.as_str())
                .is_some_and(|mine| track_head(mine) != track_head(track));
            if added && collides {
                let new = ids.next(prefix(&track.id));
                renamed.push((track.id.clone(), new.clone()));
                renames
                    .entry(sequence.id.clone())
                    .or_default()
                    .insert(track.id.clone(), new);
            }
        }
    }
    renames
}

// ---- repairs ----------------------------------------------------------------

/// Put back what one side removed while the merged project still uses it:
/// a sequence or track the other side put clips on, media its clips play,
/// markers its looks land on, and, whole with what is in them, a sequence
/// its clips nest and a track its effects read. It comes from the side that
/// kept it.
fn restore(
    flat: &mut Flat,
    ours: &Flat,
    theirs: &Flat,
    catalog: &Catalog,
    conflicts: &mut Vec<Conflict>,
) {
    let merged = (
        flat.sequences.iter().map(|(id, _)| id.clone()).collect(),
        flat.tracks.iter().map(|(key, _)| key.clone()).collect(),
    );
    // What comes back can use more, as a nested sequence's clips can.
    while restore_used(flat, &merged, [ours, theirs], catalog, conflicts) {}
}

/// One pass of `restore`, given the sequences and tracks the merge kept.
/// Whether anything came back.
fn restore_used(
    flat: &mut Flat,
    (merged_sequences, merged_tracks): &(HashSet<Id>, HashSet<(Id, Id)>),
    [ours, theirs]: [&Flat; 2],
    catalog: &Catalog,
    conflicts: &mut Vec<Conflict>,
) -> bool {
    let mut restored = false;
    let nested: BTreeSet<Id> = flat
        .clips
        .iter()
        .filter_map(|(_, placed)| match &placed.item.content {
            Content::Sequence { sequence, .. } => Some(sequence.clone()),
            _ => None,
        })
        .filter(|id| !merged_sequences.contains(id))
        .collect();
    for id in nested {
        let path = || format!("sequence {id}");
        restored |= bring_back(
            &mut flat.sequences,
            [&ours.sequences, &theirs.sequences],
            &id,
            path,
            conflicts,
        );
        restored |= put_back_contents(flat, [ours, theirs], &id, None);
    }
    let read: BTreeSet<(Id, Id)> = flat
        .clips
        .iter()
        .flat_map(|(_, placed)| {
            clip_reads(catalog, &placed.item).map(move |track| (placed.sequence.clone(), track))
        })
        .chain(flat.transitions.iter().flat_map(|(_, placed)| {
            let transition = &placed.item;
            track_reads(catalog, &transition.fx, &transition.params)
                .map(move |track| (placed.sequence.clone(), track))
        }))
        .filter(|key| !merged_tracks.contains(key))
        .collect();
    for key in read {
        let path = || format!("{}/{}", key.0, key.1);
        restored |= bring_back(
            &mut flat.tracks,
            [&ours.tracks, &theirs.tracks],
            &key,
            path,
            conflicts,
        );
        restored |= put_back_contents(flat, [ours, theirs], &key.0, Some(&key.1));
    }
    let sequences: BTreeSet<Id> = flat
        .tracks
        .iter()
        .map(|((sequence, _), _)| sequence)
        .chain(flat.clips.iter().map(|(_, placed)| &placed.sequence))
        .chain(flat.transitions.iter().map(|(_, placed)| &placed.sequence))
        .chain(flat.markers.iter().map(|((sequence, _), _)| sequence))
        .cloned()
        .collect();
    for id in sequences {
        let path = || format!("sequence {id}");
        restored |= bring_back(
            &mut flat.sequences,
            [&ours.sequences, &theirs.sequences],
            &id,
            path,
            conflicts,
        );
    }
    let tracks: BTreeSet<(Id, Id)> = flat
        .clips
        .iter()
        .map(|(_, placed)| (placed.sequence.clone(), placed.track.clone()))
        .chain(
            flat.transitions
                .iter()
                .map(|(_, placed)| (placed.sequence.clone(), placed.track.clone())),
        )
        .collect();
    for key in tracks {
        let path = || format!("{}/{}", key.0, key.1);
        restored |= bring_back(
            &mut flat.tracks,
            [&ours.tracks, &theirs.tracks],
            &key,
            path,
            conflicts,
        );
    }
    let media: BTreeSet<Id> = flat
        .clips
        .iter()
        .filter_map(|(_, placed)| match &placed.item.content {
            Content::Media { media, .. } => Some(media.clone()),
            _ => None,
        })
        .collect();
    for id in media {
        let path = || format!("media {id}");
        restored |= bring_back(
            &mut flat.media,
            [&ours.media, &theirs.media],
            &id,
            path,
            conflicts,
        );
    }
    let markers: BTreeSet<(Id, Id)> = flat
        .clips
        .iter()
        .flat_map(|(_, placed)| {
            placed
                .item
                .effects
                .iter()
                .flat_map(move |effect| landings(catalog, effect))
                .map(move |marker| (placed.sequence.clone(), marker))
        })
        .collect();
    for key in markers {
        let path = || format!("{}/marker {}", key.0, key.1);
        restored |= bring_back(
            &mut flat.markers,
            [&ours.markers, &theirs.markers],
            &key,
            path,
            conflicts,
        );
    }
    restored
}

fn has<K: PartialEq, T>(list: &[(K, T)], key: &K) -> bool {
    list.iter().any(|(found, _)| found == key)
}

/// Put `key` back in `list` from ours or theirs, after the element before
/// it on that side, unless the list has it, and report it. Whether it did.
fn bring_back<K: Clone + PartialEq, T: Clone>(
    list: &mut Vec<(K, T)>,
    [ours, theirs]: [&[(K, T)]; 2],
    key: &K,
    path: impl FnOnce() -> String,
    conflicts: &mut Vec<Conflict>,
) -> bool {
    let (side, what, kept) = if has(ours, key) {
        (ours, "removed there, used here; restored", "ours")
    } else {
        (theirs, "removed here, used there; restored", "theirs")
    };
    if !put_back(list, side, key) {
        return false;
    }
    conflicts.push(Conflict {
        path: path(),
        what: what.to_owned(),
        kept,
    });
    true
}

/// Put `key` back in `list` from `side`, after the element before it
/// there, unless the list has it or the side does not. Whether it did.
fn put_back<K: Clone + PartialEq, T: Clone>(
    list: &mut Vec<(K, T)>,
    side: &[(K, T)],
    key: &K,
) -> bool {
    let position = |side: &[(K, T)], key: &K| side.iter().position(|(found, _)| found == key);
    if has(list, key) {
        return false;
    }
    let Some(at) = position(side, key) else {
        return false;
    };
    let index = side[..at]
        .iter()
        .rev()
        .find_map(|(before, _)| position(list, before))
        .map_or(0, |index| index + 1);
    list.insert(index, side[at].clone());
    true
}

/// Put back what is in a sequence, or in one of its tracks, from the side
/// that kept it: tracks, clips, transitions, and, for a whole sequence,
/// markers. The merged project keeps its own version of anything it has.
/// Whether anything came back.
fn put_back_contents(
    flat: &mut Flat,
    [ours, theirs]: [&Flat; 2],
    sequence: &Id,
    track: Option<&Id>,
) -> bool {
    let ours_kept = match track {
        None => has(&ours.sequences, sequence),
        Some(track) => has(&ours.tracks, &(sequence.clone(), track.clone())),
    };
    let side = if ours_kept { ours } else { theirs };
    let within = |s: &Id, t: &Id| s == sequence && track.is_none_or(|track| t == track);
    let mut restored = false;
    for (key, _) in side.tracks.iter().filter(|((s, t), _)| within(s, t)) {
        restored |= put_back(&mut flat.tracks, &side.tracks, key);
    }
    for (id, _) in side
        .clips
        .iter()
        .filter(|(_, p)| within(&p.sequence, &p.track))
    {
        restored |= put_back(&mut flat.clips, &side.clips, id);
    }
    for (id, _) in side
        .transitions
        .iter()
        .filter(|(_, p)| within(&p.sequence, &p.track))
    {
        restored |= put_back(&mut flat.transitions, &side.transitions, id);
    }
    if track.is_none() {
        for (key, _) in side.markers.iter().filter(|((s, _), _)| s == sequence) {
            restored |= put_back(&mut flat.markers, &side.markers, key);
        }
    }
    restored
}

/// Move clips that overlap on a track after merging, each side having put
/// one there, to the nearest free track of the same kind after it (above
/// for pictures, below for sound), adding one when none is free. The clip
/// that moves is the one ours did not put there.
fn separate_overlaps(
    project: &mut Project,
    ours: &Flat,
    track_ids: &HashMap<Id, Vec<Id>>,
    conflicts: &mut Vec<Conflict>,
) {
    let ours_clips: HashMap<&Id, &Placed<Clip>> =
        ours.clips.iter().map(|(id, placed)| (id, placed)).collect();
    for sequence in &mut project.sequences {
        let mut ids = IdGen::new(
            track_ids
                .get(&sequence.id)
                .into_iter()
                .flatten()
                .chain(sequence.tracks.iter().map(|track| &track.id)),
        );
        let mut index = 0;
        while index < sequence.tracks.len() {
            let from = sequence.tracks[index].id.clone();
            // 2: where ours put it; 1: ours', moved by the merge; 0: theirs'.
            let rank = |clip: &Clip| match ours_clips.get(&clip.id) {
                Some(mine)
                    if mine.sequence == sequence.id
                        && mine.track == from
                        && mine.item.start == clip.start
                        && mine.item.duration == clip.duration =>
                {
                    2
                }
                Some(_) => 1,
                None => 0,
            };
            let clips = std::mem::take(&mut sequence.tracks[index].clips);
            let mut kept: Vec<Clip> = Vec::with_capacity(clips.len());
            let mut moving: Vec<(Clip, Id)> = Vec::new();
            for clip in clips {
                let mut next = Some(clip);
                while let Some(clip) = next.take() {
                    match kept.last() {
                        Some(last) if last.end() > clip.start => {
                            if rank(last) < rank(&clip) {
                                let last = kept.pop().expect("a clip before");
                                moving.push((last, clip.id.clone()));
                                next = Some(clip);
                            } else {
                                let overlapped = last.id.clone();
                                moving.push((clip, overlapped));
                            }
                        }
                        _ => kept.push(clip),
                    }
                }
            }
            sequence.tracks[index].clips = kept;
            let kind = sequence.tracks[index].kind;
            for (clip, overlapped) in moving {
                let free = (index + 1..sequence.tracks.len()).find(|&i| {
                    let track = &sequence.tracks[i];
                    track.kind == kind
                        && !track.locked
                        && track.is_free(clip.start, clip.end(), &[])
                });
                let to = free.unwrap_or_else(|| {
                    let at = sequence
                        .tracks
                        .iter()
                        .rposition(|track| track.kind == kind)
                        .map_or(sequence.tracks.len(), |last| last + 1);
                    sequence
                        .tracks
                        .insert(at, Track::new(ids.next(prefix(&from)), kind));
                    at
                });
                conflicts.push(Conflict {
                    path: format!("{}/{from}/{}", sequence.id, clip.id),
                    what: format!(
                        "overlapped {overlapped} on {from}; moved to {}",
                        sequence.tracks[to].id
                    ),
                    kept: "theirs",
                });
                sequence.tracks[to].insert_sorted(clip);
            }
            index += 1;
        }
    }
}

/// Validation codes of transitions that no longer fit their cut.
const BROKEN_TRANSITIONS: [&str; 5] = [
    "transition.ref",
    "transition.not_adjacent",
    "transition.duplicate",
    "transition.too_long",
    "transition.handles",
];

/// Carry transitions to the track their clips moved to together, and drop
/// the ones that no longer fit their cut.
fn repair_transitions(
    project: &mut Project,
    ours: &Flat,
    catalog: &Catalog,
    conflicts: &mut Vec<Conflict>,
) {
    let joins = |track: &Track, transition: &Transition| {
        track.clip(transition.from.as_str()).is_some()
            && track.clip(transition.to.as_str()).is_some()
    };
    for sequence in &mut project.sequences {
        let mut stray = Vec::new();
        for (index, track) in sequence.tracks.iter_mut().enumerate() {
            let (stay, go): (Vec<_>, Vec<_>) = std::mem::take(&mut track.transitions)
                .into_iter()
                .partition(|transition| joins(track, transition));
            track.transitions = stay;
            stray.extend(go.into_iter().map(|transition| (index, transition)));
        }
        for (index, transition) in stray {
            let home = sequence
                .tracks
                .iter()
                .position(|track| joins(track, &transition))
                .unwrap_or(index);
            sequence.tracks[home].transitions.push(transition);
        }
    }
    let ours_transitions: HashMap<&Id, &Placed<Transition>> = ours
        .transitions
        .iter()
        .map(|(id, placed)| (id, placed))
        .collect();
    loop {
        let broken: HashMap<Id, &str> = kerf_model::validate::errors(project, catalog)
            .into_iter()
            .filter(|error| BROKEN_TRANSITIONS.contains(&error.code))
            .filter_map(|error| Some((error.entity?, error.code)))
            .collect();
        let before = conflicts.len();
        for sequence in &mut project.sequences {
            for track in &mut sequence.tracks {
                track.transitions.retain(|transition| {
                    let Some(code) = broken.get(&transition.id) else {
                        return true;
                    };
                    let (from, to) = (&transition.from, &transition.to);
                    let what = match *code {
                        "transition.duplicate" => {
                            format!(
                                "the cut between {from} and {to} already has a transition; removed"
                            )
                        }
                        "transition.too_long" | "transition.handles" => {
                            format!("no longer fits the cut between {from} and {to}; removed")
                        }
                        _ => format!("joined {from} and {to}, which no longer meet; removed"),
                    };
                    let ours_own = ours_transitions.get(&transition.id).is_some_and(|mine| {
                        mine.sequence == sequence.id
                            && mine.track == track.id
                            && mine.item == *transition
                    });
                    conflicts.push(Conflict {
                        path: format!("{}/{}/{}", sequence.id, track.id, transition.id),
                        what,
                        kept: if ours_own { "theirs" } else { "ours" },
                    });
                    false
                });
            }
        }
        if conflicts.len() == before {
            return;
        }
    }
}

// ---- summaries --------------------------------------------------------------

/// Human-readable lines describing how `after` differs from `before`, for
/// commit messages and pull-request bodies: media, then sequences in order,
/// then each sequence's tracks, clips, transitions, and markers.
pub fn summarize(before: &Project, after: &Project) -> Vec<String> {
    let mut lines = Vec::new();
    if before.name != after.name {
        lines.push(format!("Renamed the project to \"{}\"", after.name));
    }
    for media in &after.media {
        match before.media(media.id.as_str()) {
            None => lines.push(format!("Added media {} ({})", media.id, media.path)),
            Some(was) => changed(
                &mut lines,
                format!("media {}", media.id),
                media_changes(was, media),
            ),
        }
    }
    for media in &before.media {
        if after.media(media.id.as_str()).is_none() {
            lines.push(format!("Removed media {}", media.id));
        }
    }
    let (clips_before, clips_after) = (clips_by_id(before), clips_by_id(after));
    for sequence in &after.sequences {
        let Some(was) = before.sequence(sequence.id.as_str()) else {
            lines.push(if sequence.name.is_empty() {
                format!("Added sequence {}", sequence.id)
            } else {
                format!("Added sequence {} ({})", sequence.id, sequence.name)
            });
            continue;
        };
        changed(
            &mut lines,
            format!("sequence {}", sequence.id),
            sequence_changes(was, sequence),
        );
        for track in &sequence.tracks {
            match was.track(track.id.as_str()) {
                None => lines.push(format!("Added track {}", track.id)),
                Some(old) => changed(
                    &mut lines,
                    format!("track {}", track.id),
                    track_changes(old, track),
                ),
            }
        }
        for track in &was.tracks {
            if sequence.track(track.id.as_str()).is_none() {
                lines.push(format!("Removed track {}", track.id));
            }
        }
        for track in &sequence.tracks {
            for clip in &track.clips {
                match clips_before.get(&clip.id) {
                    None => lines.push(format!("Added clip {} on {}", clip.id, track.id)),
                    Some(&old) => changed(
                        &mut lines,
                        format!("clip {}", clip.id),
                        clip_changes(old, (&sequence.id, &track.id, clip)),
                    ),
                }
            }
        }
        for clip in was.tracks.iter().flat_map(|track| &track.clips) {
            if !clips_after.contains_key(&clip.id) {
                lines.push(format!("Removed clip {}", clip.id));
            }
        }
        for track in &sequence.tracks {
            for transition in &track.transitions {
                match transition_in(was, &transition.id) {
                    None => lines.push(format!(
                        "Added transition {} on {}",
                        transition.id, track.id
                    )),
                    Some(old) => changed(
                        &mut lines,
                        format!("transition {}", transition.id),
                        transition_changes(old, (&track.id, transition)),
                    ),
                }
            }
        }
        for transition in was.tracks.iter().flat_map(|track| &track.transitions) {
            if transition_in(sequence, &transition.id).is_none() {
                lines.push(format!("Removed transition {}", transition.id));
            }
        }
        for marker in &sequence.markers {
            match was.marker(marker.id.as_str()) {
                None => lines.push(format!("Added marker {}", marker.id)),
                Some(old) => changed(
                    &mut lines,
                    format!("marker {}", marker.id),
                    marker_changes(old, marker),
                ),
            }
        }
        for marker in &was.markers {
            if sequence.marker(marker.id.as_str()).is_none() {
                lines.push(format!("Removed marker {}", marker.id));
            }
        }
    }
    for sequence in &before.sequences {
        if after.sequence(sequence.id.as_str()).is_none() {
            lines.push(format!("Removed sequence {}", sequence.id));
        }
    }
    lines
}

/// "Changed clip c3: start, duration", when anything changed.
fn changed(lines: &mut Vec<String>, what: String, fields: Vec<String>) {
    if !fields.is_empty() {
        lines.push(format!("Changed {what}: {}", fields.join(", ")));
    }
}

fn differs<T: PartialEq>(fields: &mut Vec<String>, name: &str, before: &T, after: &T) {
    if before != after {
        fields.push(name.to_owned());
    }
}

/// The names of the parameters that differ.
fn param_changes(fields: &mut Vec<String>, before: &Params, after: &Params) {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        differs(fields, key, &before.get(key), &after.get(key));
    }
}

fn media_changes(before: &Media, after: &Media) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "path", &before.path, &after.path);
    differs(&mut fields, "rate", &before.rate, &after.rate);
    differs(
        &mut fields,
        "drop frame",
        &before.drop_frame,
        &after.drop_frame,
    );
    differs(&mut fields, "timecode", &before.start, &after.start);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "video", &before.video, &after.video);
    differs(&mut fields, "audio", &before.audio, &after.audio);
    differs(&mut fields, "reel", &before.reel, &after.reel);
    differs(&mut fields, "recipe", &before.recipe, &after.recipe);
    fields
}

fn sequence_changes(before: &Sequence, after: &Sequence) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "rate", &before.rate, &after.rate);
    differs(
        &mut fields,
        "drop frame",
        &before.drop_frame,
        &after.drop_frame,
    );
    differs(
        &mut fields,
        "size",
        &(before.width, before.height),
        &(after.width, after.height),
    );
    differs(
        &mut fields,
        "sample rate",
        &before.sample_rate,
        &after.sample_rate,
    );
    differs(&mut fields, "color", &before.color, &after.color);
    differs(&mut fields, "start", &before.start, &after.start);
    fields
}

fn track_changes(before: &Track, after: &Track) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "kind", &before.kind, &after.kind);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "enabled", &before.enabled, &after.enabled);
    differs(&mut fields, "locked", &before.locked, &after.locked);
    differs(&mut fields, "muted", &before.muted, &after.muted);
    differs(&mut fields, "role", &before.role, &after.role);
    differs(
        &mut fields,
        "sync lock",
        &before.sync_locked,
        &after.sync_locked,
    );
    fields
}

/// A clip with its sequence and track.
type ClipIn<'a> = (&'a Id, &'a Id, &'a Clip);

fn clips_by_id(project: &Project) -> HashMap<&Id, ClipIn<'_>> {
    let mut clips = HashMap::new();
    for sequence in &project.sequences {
        for track in &sequence.tracks {
            for clip in &track.clips {
                clips.insert(&clip.id, (&sequence.id, &track.id, clip));
            }
        }
    }
    clips
}

fn clip_changes(
    (sequence, track, before): ClipIn<'_>,
    (now_sequence, now_track, after): ClipIn<'_>,
) -> Vec<String> {
    let mut fields = Vec::new();
    differs(
        &mut fields,
        "track",
        &(sequence, track),
        &(now_sequence, now_track),
    );
    differs(&mut fields, "start", &before.start, &after.start);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    match (&before.content, &after.content) {
        (
            Content::Media {
                media: source,
                source_in,
                speed,
            },
            Content::Media {
                media: now_source,
                source_in: now_in,
                speed: now_speed,
            },
        )
        | (
            Content::Sequence {
                sequence: source,
                source_in,
                speed,
            },
            Content::Sequence {
                sequence: now_source,
                source_in: now_in,
                speed: now_speed,
            },
        ) => {
            differs(&mut fields, "source", source, now_source);
            differs(&mut fields, "source in", source_in, now_in);
            differs(&mut fields, "speed", speed, now_speed);
        }
        (
            Content::Generator {
                fx,
                version,
                params,
            },
            Content::Generator {
                fx: now_fx,
                version: now_version,
                params: now_params,
            },
        ) => {
            differs(
                &mut fields,
                "generator",
                &(fx, version),
                &(now_fx, now_version),
            );
            param_changes(&mut fields, params, now_params);
        }
        (Content::Caption { text }, Content::Caption { text: now_text }) => {
            differs(&mut fields, "text", text, now_text);
        }
        (before, after) => differs(&mut fields, "content", before, after),
    }
    differs(&mut fields, "enabled", &before.enabled, &after.enabled);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "link", &before.link, &after.link);
    differs(&mut fields, "parent", &before.parent, &after.parent);
    differs(&mut fields, "retime", &before.retime, &after.retime);
    differs(&mut fields, "angle", &before.angle, &after.angle);
    param_changes(&mut fields, &before.props, &after.props);
    differs(&mut fields, "masks", &before.masks, &after.masks);
    differs(&mut fields, "effects", &before.effects, &after.effects);
    fields
}

/// A transition of `sequence` with its track.
fn transition_in<'a>(sequence: &'a Sequence, id: &Id) -> Option<(&'a Id, &'a Transition)> {
    sequence.tracks.iter().find_map(|track| {
        track
            .transition(id.as_str())
            .map(|transition| (&track.id, transition))
    })
}

fn transition_changes(
    (track, before): (&Id, &Transition),
    (now_track, after): (&Id, &Transition),
) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "track", track, now_track);
    differs(
        &mut fields,
        "clips",
        &(&before.from, &before.to),
        &(&after.from, &after.to),
    );
    differs(
        &mut fields,
        "fx",
        &(&before.fx, before.version),
        &(&after.fx, after.version),
    );
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "align", &before.align, &after.align);
    param_changes(&mut fields, &before.params, &after.params);
    fields
}

fn marker_changes(before: &Marker, after: &Marker) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "at", &before.at, &after.at);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "color", &before.color, &after.color);
    differs(&mut fields, "note", &before.note, &after.note);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
  marker mk1 at=01:00:02:00 name="beat"
"#;

    const C1: &str = "clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00";
    const C2: &str = "    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00\n";
    const C2_END: &str = "dur=00:00:03:00\n";
    const A1_END: &str = "link=L1\n  marker";

    /// `text` with each `(from, to)` replaced once; every `from` must be there.
    fn edit(text: &str, edits: &[(&str, &str)]) -> String {
        edits.iter().fold(text.to_owned(), |text, (from, to)| {
            assert!(text.contains(from), "no `{from}` in\n{text}");
            text.replacen(from, to, 1)
        })
    }

    fn project(text: &str) -> Project {
        parse_document(text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
            .project
    }

    /// Merge, and check the result reads back to the same canonical text.
    fn merge(base: &str, ours: &str, theirs: &str) -> MergedDocument {
        let merged = merge_documents(base, ours, theirs, Catalog::builtin())
            .unwrap_or_else(|error| panic!("{error}"));
        let again = print_document(
            &project(&merged.text),
            Catalog::builtin(),
            PrintOptions::default(),
        );
        assert_eq!(again, merged.text);
        merged
    }

    fn conflict(path: &str, what: &str, kept: &'static str) -> Conflict {
        Conflict {
            path: path.to_owned(),
            what: what.to_owned(),
            kept,
        }
    }

    fn renames(pairs: &[(&str, &str)]) -> Vec<(Id, Id)> {
        pairs
            .iter()
            .map(|(old, new)| (Id::new(*old).unwrap(), Id::new(*new).unwrap()))
            .collect()
    }

    #[test]
    fn disjoint_edits_merge_cleanly() {
        let trim = (
            C1,
            "clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:03:00",
        );
        let path = ("path=\"b.mov\"", "path=\"b2.mov\"");
        let name = (C2_END, "dur=00:00:03:00 name=\"Wide\"\n");
        let marker = ("mk1 at=01:00:02:00", "mk1 at=01:00:03:00");
        let merged = merge(
            BASE,
            &edit(BASE, &[trim, path]),
            &edit(BASE, &[name, marker]),
        );
        assert_eq!(merged.text, edit(BASE, &[trim, path, name, marker]));
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(merged.renamed, vec![]);
    }

    #[test]
    fn clips_appended_on_both_sides_under_one_id_are_both_kept() {
        let ours = edit(
            BASE,
            &[
                (
                    C2_END,
                    "dur=00:00:03:00\n    \
                     clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n    \
                     transition t1 from=c2 to=c3 fx=dissolve@1 dur=00:00:00:12\n",
                ),
                (
                    A1_END,
                    "link=L1\n    \
                     clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n  marker",
                ),
            ],
        );
        let theirs = edit(
            BASE,
            &[
                (
                    C2_END,
                    "dur=00:00:03:00\n    \
                     clip c3 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L2\n    \
                     clip c4 media=m2 src=00:02:00:00 at=01:00:11:00 dur=00:00:02:00\n    \
                     transition t1 from=c3 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
                ),
                (
                    A1_END,
                    "link=L1\n    \
                     clip a2 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L2\n  marker",
                ),
            ],
        );
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(
            merged.text,
            r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
    clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2
    clip c5 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L3
    clip c4 media=m2 src=00:02:00:00 at=01:00:11:00 dur=00:00:02:00
    transition t1 from=c2 to=c3 fx=dissolve@1 dur=00:00:00:12
    transition t2 from=c5 to=c4 fx=dissolve@1 dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2
    clip a3 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L3
  marker mk1 at=01:00:02:00 name="beat"
"#
        );
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(
            merged.renamed,
            renames(&[("c3", "c5"), ("a2", "a3"), ("t1", "t2"), ("L2", "L3")])
        );
    }

    #[test]
    fn a_trim_on_one_side_and_a_move_on_the_other_both_apply() {
        let trimmed = |text: &str| edit(text, &[("dur=00:00:03:00", "dur=00:00:02:00")]);
        let moved = (
            C2,
            "  track V2 video\n    clip c2 media=m2 src=00:01:00:00 at=01:00:08:00 dur=00:00:03:00\n",
        );
        let merged = merge(BASE, &trimmed(BASE), &edit(BASE, &[moved]));
        assert_eq!(merged.text, trimmed(&edit(BASE, &[moved])));
        assert_eq!(merged.conflicts, vec![]);
    }

    #[test]
    fn a_field_changed_differently_on_both_sides_keeps_ours() {
        let ours = edit(BASE, &[(C2_END, "dur=00:00:02:00\n")]);
        let theirs = edit(BASE, &[(C2_END, "dur=00:00:02:12 name=\"Wide\"\n")]);
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(
            merged.text,
            edit(BASE, &[(C2_END, "dur=00:00:02:00 name=\"Wide\"\n")])
        );
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/V1/c2",
                "duration changed on both sides",
                "ours"
            )]
        );
    }

    #[test]
    fn a_media_file_changed_on_both_sides_keeps_ours_whole() {
        // Ours relinks m1 to a consolidated copy; theirs replaces it.
        let ours = edit(BASE, &[("path=\"a.mov\"", "path=\"media/a-1a2b3c4d.mov\"")]);
        let theirs = edit(
            BASE,
            &[(
                "m1 path=\"a.mov\" rate=24 dur=00:10:00:00 video=1920x1080",
                "m1 path=\"media/new-9f8e7d6c.mov\" rate=25 dur=00:20:00:00 video=3840x2160",
            )],
        );
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(merged.text, ours);
        assert_eq!(
            merged.conflicts,
            vec![conflict("media m1", "file changed on both sides", "ours")]
        );
        // A clip theirs slipped past the end of ours' file stops the merge.
        let slipped = edit(
            &theirs,
            &[(
                C1,
                "clip c1 media=m1 src=00:15:00:00 at=01:00:00:00 dur=00:00:04:00",
            )],
        );
        let error = merge_documents(BASE, &ours, &slipped, Catalog::builtin()).unwrap_err();
        assert!(
            error.contains("clip.source_range") && error.contains("c1"),
            "{error}"
        );
        // The reel is not the file: it merges on its own.
        let reel = (
            "audio=2ch@48000\nmedia m2",
            "audio=2ch@48000 reel=\"A001\"\nmedia m2",
        );
        let merged = merge(BASE, &ours, &edit(BASE, &[reel]));
        assert_eq!(merged.text, edit(&ours, &[reel]));
        assert_eq!(merged.conflicts, vec![]);
        // Nor is what the file was made from.
        let recipe = (
            "audio=2ch@48000\nmedia m2",
            "audio=2ch@48000 recipe=\"shots/open.json\"\nmedia m2",
        );
        let merged = merge(BASE, &ours, &edit(BASE, &[recipe]));
        assert_eq!(merged.text, edit(&ours, &[recipe]));
        assert_eq!(merged.conflicts, vec![]);
    }

    #[test]
    fn removed_against_changed_keeps_ours_either_way() {
        let retire = ("project name=\"Film\"", "project name=\"Film\" retired=c2");
        let removed = edit(BASE, &[retire, (C2, "")]);
        let named = edit(BASE, &[(C2_END, "dur=00:00:03:00 name=\"Wide\"\n")]);

        let merged = merge(BASE, &removed, &named);
        assert_eq!(merged.text, removed);
        assert_eq!(
            merged.conflicts,
            vec![conflict("s1/V1/c2", "removed here, changed there", "ours")]
        );

        let merged = merge(BASE, &named, &removed);
        assert_eq!(merged.text, named);
        assert_eq!(
            merged.conflicts,
            vec![conflict("s1/V1/c2", "changed here, removed there", "ours")]
        );

        // Removed against unchanged is removed, without a conflict.
        let close = (C1, &format!("{C1} name=\"Close\"")[..]);
        let merged = merge(BASE, &removed, &edit(BASE, &[close]));
        assert_eq!(merged.text, edit(&removed, &[close]));
        assert_eq!(merged.conflicts, vec![]);
    }

    #[test]
    fn clips_overlapping_after_the_merge_move_theirs_to_a_free_track() {
        let ours = edit(
            BASE,
            &[(
                C2_END,
                "dur=00:00:03:00\n    clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:04:00\n",
            )],
        );
        let theirs = edit(
            BASE,
            &[(
                C2_END,
                "dur=00:00:03:00\n    \
                 clip c3 media=m2 src=00:02:00:00 at=01:00:07:00 dur=00:00:02:00\n    \
                 clip c4 media=m2 src=00:03:00:00 at=01:00:09:00 dur=00:00:02:00\n    \
                 transition t1 from=c3 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
            )],
        );
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(
            merged.text,
            edit(
                BASE,
                &[(
                    C2_END,
                    "dur=00:00:03:00\n    \
                     clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:04:00\n  \
                     track V2 video\n    \
                     clip c5 media=m2 src=00:02:00:00 at=01:00:07:00 dur=00:00:02:00\n    \
                     clip c4 media=m2 src=00:03:00:00 at=01:00:09:00 dur=00:00:02:00\n    \
                     transition t1 from=c5 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
                )]
            )
        );
        assert_eq!(
            merged.conflicts,
            vec![
                conflict("s1/V1/c5", "overlapped c3 on V1; moved to V2", "theirs"),
                conflict("s1/V1/c4", "overlapped c3 on V1; moved to V2", "theirs"),
            ]
        );
        assert_eq!(merged.renamed, renames(&[("c3", "c5")]));
    }

    #[test]
    fn tracks_added_on_both_sides_merge_or_get_a_fresh_id() {
        let ours = edit(
            BASE,
            &[
                (
                    "  track A1 audio\n",
                    "  track V2 video\n    \
                     clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:02:00\n  track A1 audio\n",
                ),
                (
                    A1_END,
                    "link=L1\n  track A2 audio\n    \
                     clip a2 media=m1 src=00:01:00:00 at=01:00:00:00 dur=00:00:02:00\n  marker",
                ),
            ],
        );
        let theirs = edit(
            BASE,
            &[
                (
                    "link=L1\n    clip c2",
                    "link=L1\n      fx key.trackmatte@1 matte=V2\n    clip c2",
                ),
                (
                    "  track A1 audio\n",
                    "  track V2 video name=\"Matte\"\n    \
                     clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:04:00\n  track A1 audio\n",
                ),
                (
                    A1_END,
                    "link=L1\n  track A2 audio\n    \
                     clip a2 media=m1 src=00:02:00:00 at=01:00:05:00 dur=00:00:02:00\n  marker",
                ),
            ],
        );
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(
            merged.text,
            r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
      fx key.trackmatte@1 matte=V3
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
  track V2 video
    clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:02:00
  track V3 video name="Matte"
    clip c4 gen=solid@1 at=01:00:00:00 dur=00:00:04:00
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
  track A2 audio
    clip a2 media=m1 src=00:01:00:00 at=01:00:00:00 dur=00:00:02:00
    clip a3 media=m1 src=00:02:00:00 at=01:00:05:00 dur=00:00:02:00
  marker mk1 at=01:00:02:00 name="beat"
"#
        );
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(
            merged.renamed,
            renames(&[("V2", "V3"), ("c3", "c4"), ("a2", "a3")])
        );
    }

    #[test]
    fn a_renamed_marker_takes_the_looks_that_land_on_it_along() {
        let beat = "name=\"beat\"\n";
        let ours = edit(
            BASE,
            &[(
                beat,
                "name=\"beat\"\n  marker mk2 at=01:00:03:00 name=\"ours\"\n",
            )],
        );
        let theirs = edit(
            BASE,
            &[
                (C2_END, "dur=00:00:03:00\n      fx look.flash@1 hit=mk2\n"),
                (
                    beat,
                    "name=\"beat\"\n  marker mk2 at=01:00:05:00 name=\"theirs\"\n",
                ),
            ],
        );
        let merged = merge(BASE, &ours, &theirs);
        assert_eq!(
            merged.text,
            edit(
                BASE,
                &[
                    (C2_END, "dur=00:00:03:00\n      fx look.flash@1 hit=mk3\n"),
                    (
                        beat,
                        "name=\"beat\"\n  marker mk2 at=01:00:03:00 name=\"ours\"\n  \
                         marker mk3 at=01:00:05:00 name=\"theirs\"\n",
                    ),
                ]
            )
        );
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(merged.renamed, renames(&[("mk2", "mk3")]));
    }

    #[test]
    fn properties_merge_key_by_key() {
        let set = |props: &str| {
            let line = format!("link=L1\n      set {props}\n    clip c2");
            edit(BASE, &[("link=L1\n    clip c2", &line)])
        };
        let merged = merge(BASE, &set("scale=1.2 opacity=0.5"), &set("x=0.1 scale=0.8"));
        assert_eq!(merged.text, set("x=0.1 scale=1.2 opacity=0.5"));
        assert_eq!(
            merged.conflicts,
            vec![conflict("s1/V1/c1", "scale changed on both sides", "ours")]
        );
    }

    #[test]
    fn track_roles_and_sync_locks_merge_by_track() {
        let role = ("  track V1 video\n", "  track V1 video role=titles\n");
        let sync = (
            "  track A1 audio\n",
            "  track A1 audio role=music sync=off\n",
        );
        let merged = merge(BASE, &edit(BASE, &[role]), &edit(BASE, &[sync]));
        assert_eq!(merged.text, edit(BASE, &[role, sync]));
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(
            summarize(&project(BASE), &project(&merged.text)),
            [
                "Changed track V1: role",
                "Changed track A1: role, sync lock"
            ]
        );

        // The same field set apart on both sides keeps ours.
        let theirs = (
            "  track V1 video\n",
            "  track V1 video role=\"B roll\" sync=off\n",
        );
        let merged = merge(BASE, &edit(BASE, &[role]), &edit(BASE, &[theirs]));
        assert_eq!(
            merged.text,
            edit(
                BASE,
                &[(
                    "  track V1 video\n",
                    "  track V1 video role=titles sync=off\n"
                )]
            )
        );
        assert_eq!(
            merged.conflicts,
            vec![conflict("s1/V1", "role changed on both sides", "ours")]
        );
    }

    #[test]
    fn a_file_added_on_both_sides_merges_from_an_empty_base() {
        let ours = r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00
"#;
        let theirs = r#"kerf 1
project name="Film"

media m1 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:00:00 at=01:00:04:00 dur=00:00:04:00
"#;
        let merged = merge("", ours, theirs);
        assert_eq!(
            merged.text,
            r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00
    clip c2 media=m2 src=00:00:00:00 at=01:00:04:00 dur=00:00:04:00
"#
        );
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(merged.renamed, renames(&[("m1", "m2"), ("c1", "c2")]));
        assert_eq!(merge(" \n", ours, ours).text, ours);
    }

    #[test]
    fn merging_identical_sides_changes_nothing() {
        let ours = edit(
            BASE,
            &[
                ("project name=\"Film\"", "project name=\"Film\" retired=c2"),
                (
                    C2,
                    "    clip c3 media=m1 src=00:00:20:00 at=01:00:05:00 dur=00:00:02:00\n",
                ),
                (C1, &format!("{C1} name=\"Close\"")),
            ],
        );
        for theirs in [ours.as_str(), BASE] {
            let merged = merge(BASE, &ours, theirs);
            assert_eq!(merged.text, ours);
            assert_eq!(merged.conflicts, vec![]);
            assert_eq!(merged.renamed, vec![]);
        }
        assert_eq!(merge(BASE, BASE, &ours).text, ours);
        assert_eq!(merge(BASE, BASE, BASE).text, BASE);
        let project = project(&ours);
        let merged = merge_projects(&project, &project, &project, Catalog::builtin()).unwrap();
        assert_eq!(merged.project, project);
    }

    #[test]
    fn a_track_one_side_removed_comes_back_for_clips_the_other_put_on_it() {
        let base = edit(
            BASE,
            &[("  track A1 audio\n", "  track V2 video\n  track A1 audio\n")],
        );
        let theirs = edit(
            &base,
            &[(
                "  track V2 video\n",
                "  track V2 video\n    clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:01:00\n",
            )],
        );
        let merged = merge(&base, BASE, &theirs);
        assert_eq!(merged.text, theirs);
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/V2",
                "removed here, used there; restored",
                "theirs"
            )]
        );
    }

    #[test]
    fn a_sequence_one_side_removed_comes_back_whole_for_a_clip_the_other_nests_it_in() {
        // s2 nests s3, which comes back too once s2's clips do.
        let reel = |id: &str, clip: &str| {
            format!(
                "\nsequence {id} name=\"Reel\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n  \
                 track V1 video\n    \
                 clip {clip} src=00:00:00:00 at=00:00:00:00 dur=00:00:05:00\n"
            )
        };
        let base = format!(
            "{BASE}{}{}",
            reel("s2", "c7 seq=s3"),
            reel("s3", "c9 media=m2")
        );
        let theirs = edit(
            &base,
            &[(
                C2_END,
                "dur=00:00:03:00\n    clip c8 seq=s2 src=00:00:01:00 at=01:00:07:00 dur=00:00:02:00\n",
            )],
        );
        let restored = |what: &str, kept| {
            ["sequence s2", "sequence s3"]
                .map(|path| conflict(path, what, kept))
                .to_vec()
        };
        let merged = merge(&base, BASE, &theirs);
        assert_eq!(merged.text, theirs);
        assert_eq!(
            merged.conflicts,
            restored("removed here, used there; restored", "theirs")
        );
        // From the other side, ours' clip brings them back from ours.
        let merged = merge(&base, &theirs, BASE);
        assert_eq!(merged.text, theirs);
        assert_eq!(
            merged.conflicts,
            restored("removed there, used here; restored", "ours")
        );
    }

    #[test]
    fn a_track_one_side_removed_comes_back_whole_for_an_effect_the_other_reads_it_with() {
        let base = edit(
            BASE,
            &[(
                "  track A1 audio\n",
                "  track V2 video\n    \
                 clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:04:00\n  track A1 audio\n",
            )],
        );
        let matte = (
            "link=L1\n    clip c2",
            "link=L1\n      fx key.trackmatte@1 matte=V2\n    clip c2",
        );
        let theirs = edit(&base, &[matte]);
        let merged = merge(&base, BASE, &theirs);
        assert_eq!(merged.text, theirs);
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/V2",
                "removed here, used there; restored",
                "theirs"
            )]
        );
        // A sound track an audio spectrum draws comes back the same way.
        let base = edit(
            BASE,
            &[(
                A1_END,
                "link=L1\n  track A2 audio\n    \
                 clip a2 media=m2 src=00:00:00:00 at=01:00:00:00 dur=00:00:04:00\n  marker",
            )],
        );
        let spectrum = (
            "link=L1\n    clip c2",
            "link=L1\n      fx generate.audiospectrum@1 audioLayer=A2\n    clip c2",
        );
        let theirs = edit(&base, &[spectrum]);
        let merged = merge(&base, BASE, &theirs);
        assert_eq!(merged.text, theirs);
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/A2",
                "removed here, used there; restored",
                "theirs"
            )]
        );
    }

    #[test]
    fn a_clip_both_sides_added_alike_keeps_its_link_group() {
        let a2 = "    clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n";
        let added = |audio: &str| {
            edit(
                BASE,
                &[
                    (
                        C2_END,
                        "dur=00:00:03:00\n    \
                         clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n",
                    ),
                    (A1_END, &format!("link=L1\n{audio}  marker")),
                ],
            )
        };
        let ours = added(a2);
        let theirs = added(&format!("{a2}      set gain=-3\n"));
        let merged = merge(BASE, &ours, &theirs);
        let a3 = a2.replace("a2", "a3");
        assert_eq!(
            merged.text,
            added(&format!("{a2}  track A2 audio\n{a3}      set gain=-3\n"))
        );
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/A1/a3",
                "overlapped a2 on A1; moved to A2",
                "theirs"
            )]
        );
        assert_eq!(merged.renamed, renames(&[("a2", "a3")]));
    }

    #[test]
    fn a_second_transition_on_one_cut_is_dropped() {
        let transition = |id: &str, frames: &str| {
            let line = format!(
                "dur=00:00:03:00\n    transition {id} from=c1 to=c2 fx=dissolve@1 dur=00:00:00:{frames}\n"
            );
            edit(BASE, &[(C2_END, &line)])
        };
        let merged = merge(BASE, &transition("t1", "12"), &transition("t1", "06"));
        assert_eq!(merged.text, transition("t1", "12"));
        assert_eq!(
            merged.conflicts,
            vec![conflict(
                "s1/V1/t2",
                "the cut between c1 and c2 already has a transition; removed",
                "ours"
            )]
        );
        assert_eq!(merged.renamed, renames(&[("t1", "t2")]));
    }

    #[test]
    fn a_merge_that_cannot_be_valid_says_why() {
        let ours = edit(BASE, &[("src=00:01:00:00", "src=00:09:57:00")]);
        let theirs = edit(BASE, &[(C2_END, "dur=00:00:04:00\n")]);
        let error = merge_documents(BASE, &ours, &theirs, Catalog::builtin()).unwrap_err();
        assert!(
            error.contains("clip.source_range") && error.contains("c2"),
            "{error}"
        );
        let error = merge_documents(BASE, "kerf 1\nclip\n", BASE, Catalog::builtin()).unwrap_err();
        assert!(error.starts_with("ours does not parse"), "{error}");
    }

    #[test]
    fn summaries_list_adds_removes_and_changes() {
        let after = edit(
            BASE,
            &[
                (
                    "project name=\"Film\"",
                    "project name=\"Film\" retired=c2,mk1",
                ),
                (
                    "audio=2ch@48000\n\n",
                    "audio=2ch@48000\nmedia m3 path=\"c.mov\" rate=24 dur=00:01:00:00 audio=2ch@48000\n\n",
                ),
                ("size=1920x1080", "size=1280x720"),
                (
                    C1,
                    "clip c1 media=m1 src=00:00:10:00 at=01:00:01:00 dur=00:00:03:00",
                ),
                (
                    C2,
                    "    clip c3 media=m2 src=00:01:00:00 at=01:00:07:00 dur=00:00:03:00\n  track V2 video\n",
                ),
                (
                    "  marker mk1 at=01:00:02:00 name=\"beat\"\n",
                    "  marker mk2 at=01:00:06:00\n",
                ),
            ],
        );
        let (before, after) = (project(BASE), project(&after));
        assert_eq!(
            summarize(&before, &after),
            [
                "Added media m3 (c.mov)",
                "Changed sequence s1: size",
                "Added track V2",
                "Changed clip c1: start, duration",
                "Added clip c3 on V1",
                "Removed clip c2",
                "Added marker mk2",
                "Removed marker mk1",
            ]
        );
        assert_eq!(summarize(&after, &after), Vec::<String>::new());
        assert_eq!(
            summarize(&Project::default(), &before),
            [
                "Renamed the project to \"Film\"",
                "Added media m1 (a.mov)",
                "Added media m2 (b.mov)",
                "Added sequence s1 (Main)",
            ]
        );
    }
}
