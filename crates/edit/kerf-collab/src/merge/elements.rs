//! The elements of a project that merge by id, as flat lists in document
//! order, and the markers and tracks their parameters name.

use super::Conflict;
use kerf_model::{
    Catalog, Clip, Content, Effect, Id, Marker, Media, Param, ParamDef, ParamType, Params, Project,
    Sequence, Time, Track, Transition, Value,
};
use std::collections::{BTreeSet, HashMap};

/// A clip or transition with the sequence and track it sits on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Placed<T> {
    pub(super) sequence: Id,
    pub(super) track: Id,
    pub(super) item: T,
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
    pub(super) fn path(id: &Id, placed: &Self) -> String {
        format!("{}/{}/{id}", placed.sequence, placed.track)
    }
}

/// Where a clip is, as the merge compares it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum At {
    /// It starts then.
    Start(Time),
    /// It follows that clip, so long after it starts (before it, when negative), as its line in
    /// the document says: an edit that took it along with that clip has not changed it.
    After(Id, Time),
}

/// A clip as the merge compares it: where it is, and the rest of it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Written {
    pub(super) at: At,
    /// Its start is zero, and when `at` says what it follows, it follows nothing here.
    pub(super) placed: Placed<Clip>,
}

impl Written {
    fn new(placed: &Placed<Clip>, at: At) -> Self {
        let mut placed = placed.clone();
        placed.item.start = Time::ZERO;
        if let At::After(..) = at {
            placed.item.follows = None;
        }
        Self { at, placed }
    }

    /// `s1/V1/c3`.
    pub(super) fn path(id: &Id, written: &Self) -> String {
        Placed::path(id, &written.placed)
    }
}

/// The clips of each side as the merge compares them. A clip that follows the same clip on
/// every side that has it, a clip of its sequence on each, is where it is by its distance
/// from that one; any other by its start.
pub(super) fn written(sides: [&Flat; 3]) -> [Vec<(Id, Written)>; 3] {
    let clips = sides.map(Flat::clips_by_id);
    let followed = |id: &Id| {
        let mut versions = clips.iter().filter_map(|side| Some((side, *side.get(id)?)));
        let (_, first) = versions.clone().next()?;
        let followed = first.item.follows.as_ref()?;
        versions
            .all(|(side, placed)| {
                placed.item.follows.as_ref() == Some(followed)
                    && side
                        .get(followed)
                        .is_some_and(|other| other.sequence == placed.sequence)
            })
            .then_some(followed)
    };
    std::array::from_fn(|side| {
        sides[side]
            .clips
            .iter()
            .map(|(id, placed)| {
                let start = placed.item.start;
                let at = match followed(id)
                    .and_then(|followed| Some((followed, clips[side].get(followed)?)))
                {
                    Some((followed, other)) => {
                        At::After(followed.clone(), start - other.item.start)
                    }
                    None => At::Start(start),
                };
                (id.clone(), Written::new(placed, at))
            })
            .collect()
    })
}

/// The merged clips where they are. One merged by its distance from the clip it follows is
/// that far from where that one is now, those that are themselves followed placed first;
/// when that one is not in its sequence after the merge, from where it is on the side that
/// has it, ours first. One that would then start before its sequence starts with it.
pub(super) fn place(
    merged: Vec<(Id, Written)>,
    [ours, theirs, base]: [&Flat; 3],
    conflicts: &mut Vec<Conflict>,
) -> Vec<(Id, Placed<Clip>)> {
    let sides = [ours, theirs, base].map(Flat::clips_by_id);
    let was = |id: &Id| {
        sides
            .iter()
            .find_map(|side| side.get(id))
            .map_or(Time::ZERO, |placed| placed.item.start)
    };
    let by_id: HashMap<&Id, &Written> = merged.iter().map(|(id, written)| (id, written)).collect();
    let mut starts: HashMap<&Id, Time> = HashMap::new();
    for (id, _) in &merged {
        // This clip and those it follows, each the distance from the next, up to one whose
        // start is known.
        let mut chain: Vec<(&Id, Time)> = Vec::new();
        let mut at = id;
        let mut start = loop {
            if let Some(&start) = starts.get(at) {
                break start;
            }
            let written = by_id[at];
            let (followed, distance) = match &written.at {
                At::Start(start) => {
                    starts.insert(at, *start);
                    break *start;
                }
                At::After(followed, distance) => (followed, *distance),
            };
            chain.push((at, distance));
            let next = by_id.get(followed).is_some_and(|next| {
                next.placed.sequence == written.placed.sequence
                    && chain.iter().all(|(clip, _)| *clip != followed)
            });
            if !next {
                break was(followed);
            }
            at = followed;
        };
        for (clip, distance) in chain.into_iter().rev() {
            start += distance;
            starts.insert(clip, start);
        }
    }
    let starts: HashMap<Id, Time> = starts
        .into_iter()
        .map(|(id, start)| (id.clone(), start))
        .collect();
    merged
        .into_iter()
        .map(|(id, written)| {
            let mut placed = written.placed;
            let start = starts[&id];
            placed.item.start = start.max(Time::ZERO);
            if let At::After(followed, _) = written.at {
                if start.is_negative() {
                    conflicts.push(Conflict {
                        path: Placed::path(&id, &placed),
                        what: format!(
                            "its distance from {followed} and where {followed} is now put it \
                             before the sequence; starts with it"
                        ),
                        kept: "ours",
                    });
                }
                placed.item.follows = Some(followed);
            }
            (id, placed)
        })
        .collect()
}

/// A project as lists of elements keyed by identity, in document order.
#[derive(Default)]
pub(super) struct Flat {
    pub(super) media: Vec<(Id, Media)>,
    /// Without their tracks and markers.
    pub(super) sequences: Vec<(Id, Sequence)>,
    /// By sequence and track, without their clips and transitions.
    pub(super) tracks: Vec<((Id, Id), Track)>,
    pub(super) clips: Vec<(Id, Placed<Clip>)>,
    pub(super) transitions: Vec<(Id, Placed<Transition>)>,
    /// By sequence and marker.
    pub(super) markers: Vec<((Id, Id), Marker)>,
}

impl Flat {
    pub(super) fn of(project: &Project) -> Self {
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

    /// Its clips by id.
    pub(super) fn clips_by_id(&self) -> HashMap<&Id, &Placed<Clip>> {
        self.clips.iter().map(|(id, placed)| (id, placed)).collect()
    }

    /// The project these elements make: clips in start order, markers in
    /// time order, nothing retired yet.
    pub(super) fn build(self, name: String) -> Result<Project, String> {
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
pub(super) fn track_head(track: &Track) -> Track {
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
pub(super) fn params_of<'a>(catalog: &'a Catalog, fx: &Id) -> &'a [ParamDef] {
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
pub(super) fn landings<'a>(
    catalog: &'a Catalog,
    effect: &'a Effect,
) -> impl Iterator<Item = Id> + 'a {
    let defs = params_of(catalog, &effect.fx);
    named(defs, &effect.params, |ty| *ty == ParamType::Time)
}

/// The tracks of its sequence that a clip's effects and generator read.
pub(super) fn clip_reads<'a>(
    catalog: &'a Catalog,
    clip: &'a Clip,
) -> impl Iterator<Item = Id> + 'a {
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
pub(super) fn track_reads<'a>(
    catalog: &'a Catalog,
    fx: &Id,
    params: &'a Params,
) -> impl Iterator<Item = Id> + 'a + use<'a> {
    named(params_of(catalog, fx), params, |ty| {
        matches!(ty, ParamType::Track | ParamType::Sound)
    })
}
