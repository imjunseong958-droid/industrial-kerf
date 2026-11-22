//! The elements of a project that merge by id, as flat lists in document
//! order, and the markers and tracks their parameters name.

use kerf_model::{
    Catalog, Clip, Content, Effect, Id, Marker, Media, Param, ParamDef, ParamType, Params, Project,
    Sequence, Track, Transition, Value,
};
use std::collections::BTreeSet;

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
) -> impl Iterator<Item = Id> + 'a {
    named(params_of(catalog, fx), params, |ty| {
        matches!(ty, ParamType::Track | ParamType::Sound)
    })
}
