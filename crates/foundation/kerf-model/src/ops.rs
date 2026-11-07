//! Edit operations.
//!
//! Every change to a project is a list of operations applied atomically: the
//! whole list succeeds or the project is unchanged. Operations use NLE
//! semantics. Ripple edits act on every unlocked, sync-locked track (and on
//! the tracks they edit) so picture and sound stay in sync, and linked clips
//! (a picture clip and its sound) move, trim, split, slip, and delete
//! together.

use crate::catalog::{Catalog, Domain, FxKind};
use crate::diag::Diagnostic;
use crate::id::{Id, IdGen};
use crate::model::{
    Align, Clip, ClipAt, Content, Effect, Marker, MarkerColor, Mask, MaskMode, MaskShape, Media,
    Params, Project, Sequence, Track, TrackKind, Transition,
};
use crate::speed::Speed;
use crate::time::{format_timecode, Rate, Time};
use crate::validate::{accepts, errors};
use crate::value::Param;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    In,
    Out,
}

/// A clip to be placed by `Insert` or `Overwrite`. Effects are added
/// afterwards with `FxAdd`.
#[derive(Clone, Debug, PartialEq)]
pub struct NewClip {
    pub id: Option<Id>,
    pub content: Content,
    pub name: Option<String>,
    pub props: Params,
}

impl NewClip {
    pub fn new(content: Content) -> Self {
        Self {
            id: None,
            content,
            name: None,
            props: Params::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipChange {
    /// An intrinsic property or a generator parameter; `None` restores the
    /// default.
    Param(String, Option<Param>),
    Name(Option<String>),
    Enabled(bool),
    Caption(String),
    /// The clip it moves with, or none.
    Parent(Option<Id>),
    Retime(crate::Retime),
    /// The one track of a nested sequence it shows; `None` for all of them.
    Angle(Option<Id>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SequenceChange {
    pub name: Option<String>,
    pub rate: Option<Rate>,
    pub size: Option<(u32, u32)>,
    pub sample_rate: Option<u32>,
    pub color: Option<crate::ColorSpace>,
    /// Start timecode and whether it is drop-frame.
    pub start: Option<(Time, bool)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackChange {
    pub name: Option<Option<String>>,
    pub enabled: Option<bool>,
    pub locked: Option<bool>,
    pub muted: Option<bool>,
    /// `Some(None)` sends it to the sequence's mix.
    pub bus: Option<Option<Id>>,
}

/// What [`Op::SetBus`] changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BusChange {
    pub name: Option<Option<String>>,
    /// `Some(None)` sends it to the sequence's mix.
    pub to: Option<Option<Id>>,
    /// `None` resets a property.
    pub params: Vec<(String, Option<Param>)>,
}

/// A change to a bus's effects, with [`Op::BusFx`].
#[derive(Clone, Debug, PartialEq)]
pub enum BusFx {
    Add {
        effect: Effect,
        index: Option<usize>,
    },
    Set {
        index: usize,
        enabled: Option<bool>,
        params: Vec<(String, Option<Param>)>,
    },
    Remove {
        index: usize,
    },
}

/// A track's role and sync lock, set with [`Op::SetTrackRole`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackRoleChange {
    /// `Some(None)` clears the role back to the kind's default.
    pub role: Option<Option<String>>,
    pub sync_locked: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkerChange {
    pub at: Option<Time>,
    pub duration: Option<Time>,
    pub name: Option<Option<String>>,
    pub color: Option<Option<MarkerColor>>,
    pub note: Option<Option<String>>,
}

/// One edit. Times are sequence offsets (`at`, `from`, `to`) or signed
/// timeline deltas (`delta`), already on the sequence's frame grid.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    AddMedia(Media),
    /// Point media at another path, keeping everything else it records.
    RelinkMedia {
        media: Id,
        path: String,
    },
    /// Swap the file behind the media with this `id` for the one described:
    /// its path, rate, timecode, duration, streams, and reel. Clips keep
    /// their timing; the edit is refused, naming them, when a clip no longer
    /// fits (its source range, a stream its track needs, transition
    /// handles).
    ReplaceMedia(Media),
    RemoveMedia {
        media: Id,
    },
    /// Add an empty sequence; tracks follow with `AddTrack`.
    AddSequence(Sequence),
    SetSequence {
        sequence: Id,
        change: SequenceChange,
    },
    RemoveSequence {
        sequence: Id,
    },
    AddBus {
        sequence: Id,
        bus: crate::model::Bus,
    },
    SetBus {
        sequence: Id,
        bus: Id,
        change: BusChange,
    },
    /// Remove a bus. What fed it goes to where it went.
    RemoveBus {
        sequence: Id,
        bus: Id,
    },
    BusFx {
        sequence: Id,
        bus: Id,
        change: BusFx,
    },
    AddTrack {
        sequence: Id,
        id: Id,
        kind: TrackKind,
        index: Option<usize>,
    },
    SetTrack {
        sequence: Id,
        track: Id,
        change: TrackChange,
    },
    RemoveTrack {
        sequence: Id,
        track: Id,
    },
    /// Set a track's role and whether ripple edits elsewhere move it.
    SetTrackRole {
        sequence: Id,
        track: Id,
        change: TrackRoleChange,
    },
    /// Open `duration` at `at` on the target tracks and every unlocked,
    /// sync-locked track, then place one clip per target track. Several
    /// targets are linked.
    Insert {
        sequence: Id,
        at: Time,
        duration: Time,
        targets: Vec<(Id, NewClip)>,
    },
    /// Replace whatever occupies `[at, at + duration)` on each target track.
    Overwrite {
        sequence: Id,
        at: Time,
        duration: Time,
        targets: Vec<(Id, NewClip)>,
    },
    /// Remove clips, leaving gaps.
    Delete {
        clips: Vec<Id>,
    },
    /// Remove clips and close the gap on their tracks and every unlocked,
    /// sync-locked track.
    RippleDelete {
        clips: Vec<Id>,
    },
    /// Clear a range, leaving a gap. `None` tracks means every unlocked track.
    Lift {
        sequence: Id,
        from: Time,
        to: Time,
        tracks: Option<Vec<Id>>,
    },
    /// Cut a range out and close it.
    Extract {
        sequence: Id,
        from: Time,
        to: Time,
        tracks: Option<Vec<Id>>,
    },
    Trim {
        clip: Id,
        edge: Edge,
        delta: Time,
        ripple: bool,
    },
    /// Move the cut after `clip`, trimming both sides.
    Roll {
        clip: Id,
        delta: Time,
    },
    Slip {
        clip: Id,
        delta: Time,
    },
    Slide {
        clip: Id,
        delta: Time,
    },
    Split {
        clip: Id,
        at: Time,
        tail: Option<Id>,
    },
    Blade {
        sequence: Id,
        at: Time,
        tracks: Option<Vec<Id>>,
    },
    Move {
        clip: Id,
        track: Option<Id>,
        delta: Time,
    },
    Set {
        clip: Id,
        changes: Vec<ClipChange>,
    },
    /// Change speed, keeping the source range and changing the duration.
    Speed {
        clip: Id,
        speed: Speed,
    },
    FxAdd {
        clip: Id,
        effect: Effect,
        index: Option<usize>,
    },
    FxSet {
        clip: Id,
        index: usize,
        enabled: Option<bool>,
        params: Vec<(String, Option<Param>)>,
    },
    FxRemove {
        clip: Id,
        index: usize,
    },
    /// Replace the look at `index` among the clip's effects with what it
    /// expands to, for hand tuning.
    FxFlatten {
        clip: Id,
        index: usize,
    },
    FxMove {
        clip: Id,
        index: usize,
        to: usize,
    },
    /// Add a mask at `index` (the end when absent) of a picture clip's masks.
    MaskAdd {
        clip: Id,
        mask: Mask,
        index: Option<usize>,
    },
    /// Change a mask's outline, mode, inversion, or properties (`None`
    /// resets a property).
    MaskSet {
        clip: Id,
        name: Id,
        shape: Option<MaskShape>,
        mode: Option<MaskMode>,
        inverted: Option<bool>,
        params: Vec<(String, Option<Param>)>,
    },
    MaskRemove {
        clip: Id,
        name: Id,
    },
    AddTransition {
        id: Option<Id>,
        from: Id,
        to: Id,
        fx: Id,
        version: u32,
        duration: Time,
        align: Align,
        params: Params,
    },
    SetTransition {
        id: Id,
        duration: Option<Time>,
        align: Option<Align>,
        params: Vec<(String, Option<Param>)>,
    },
    RemoveTransition {
        id: Id,
    },
    AddMarker {
        sequence: Id,
        id: Option<Id>,
        at: Time,
        duration: Time,
        name: Option<String>,
        color: Option<MarkerColor>,
        note: Option<String>,
    },
    SetMarker {
        id: Id,
        change: MarkerChange,
    },
    RemoveMarker {
        id: Id,
    },
    Link {
        clips: Vec<Id>,
        group: Option<Id>,
    },
    Unlink {
        clips: Vec<Id>,
    },
}

/// What an applied list of operations did beyond its literal request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Clips that exist now and did not before, including split tails.
    pub created: Vec<Id>,
    /// Clips that existed before and do not now.
    pub removed: Vec<Id>,
    /// Consequences worth telling the editor, such as dropped transitions.
    pub notes: Vec<String>,
}

/// Why a list of operations was not applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    /// The failing operation, or `None` when the final project was invalid.
    pub op: Option<usize>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Apply `ops` in order. On success returns the new project; on failure the
/// input is untouched. Validation errors already present in `project` do
/// not block an edit, but new ones do. Ids that stop naming anything are
/// added to `Project::retired`.
pub fn apply(
    project: &Project,
    catalog: &Catalog,
    ops: &[Op],
) -> Result<(Project, Report), Rejected> {
    let before: HashSet<(&'static str, Option<Id>)> = errors(project, catalog)
        .into_iter()
        .map(|d| (d.code, d.entity))
        .collect();
    let mut editor = Editor {
        project: project.clone(),
        catalog,
        ids: IdGen::default(),
        notes: Vec::new(),
    };
    for (index, op) in ops.iter().enumerate() {
        editor.ids = editor.project.id_gen();
        let before: Vec<Id> = editor.project.ids().cloned().collect();
        editor.apply(op).map_err(|diagnostic| Rejected {
            op: Some(index),
            diagnostics: vec![diagnostic],
        })?;
        editor.housekeep();
        editor.project.retire_missing(&before);
    }
    let introduced: Vec<_> = errors(&editor.project, catalog)
        .into_iter()
        .filter(|d| !before.contains(&(d.code, d.entity.clone())))
        .collect();
    if !introduced.is_empty() {
        return Err(Rejected {
            op: None,
            diagnostics: introduced,
        });
    }
    let old: HashSet<&Id> = clip_ids(project).collect();
    let new: HashSet<&Id> = clip_ids(&editor.project).collect();
    let mut report = Report {
        created: clip_ids(&editor.project)
            .filter(|id| !old.contains(id))
            .cloned()
            .collect(),
        removed: clip_ids(project)
            .filter(|id| !new.contains(id))
            .cloned()
            .collect(),
        notes: editor.notes,
    };
    report.created.sort();
    report.removed.sort();
    Ok((editor.project, report))
}

fn clip_ids(project: &Project) -> impl Iterator<Item = &Id> {
    project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .flat_map(|t| &t.clips)
        .map(|c| &c.id)
}

type Outcome<T = ()> = Result<T, Diagnostic>;

/// A role as a track keeps it ([`crate::model::track_role`]).
fn track_role(role: Option<&str>) -> Outcome<Option<String>> {
    crate::model::track_role(role.unwrap_or_default())
        .map_err(|message| fail("track.role", message))
}

fn fail(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, message)
}

struct Editor<'a> {
    project: Project,
    catalog: &'a Catalog,
    ids: IdGen,
    notes: Vec<String>,
}

impl Editor<'_> {
    fn apply(&mut self, op: &Op) -> Outcome {
        match op {
            Op::AddMedia(media) => {
                self.claim(&media.id)?;
                self.project.media.push(media.clone());
                Ok(())
            }
            Op::RelinkMedia { media, path } => {
                let found = self.project.media.iter_mut().find(|m| m.id == *media);
                let found =
                    found.ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
                found.path.clone_from(path);
                Ok(())
            }
            Op::ReplaceMedia(media) => {
                let found = self.project.media.iter_mut().find(|m| m.id == media.id);
                let found =
                    found.ok_or_else(|| fail("ref.missing", format!("no media `{}`", media.id)))?;
                *found = media.clone();
                Ok(())
            }
            Op::RemoveMedia { media } => self.remove_media(media),
            Op::AddSequence(sequence) => {
                if !sequence.tracks.is_empty() || !sequence.markers.is_empty() {
                    return Err(fail(
                        "sequence.not_empty",
                        "add the sequence empty, then add tracks",
                    ));
                }
                self.claim(&sequence.id).map_err(|mut error| {
                    if error.code == "id.taken" {
                        error.message = format!(
                            "{}; `sequence set {} …` changes the one there is",
                            error.message, sequence.id
                        );
                    }
                    error
                })?;
                self.project.sequences.push(sequence.clone());
                Ok(())
            }
            Op::SetSequence { sequence, change } => self.set_sequence(sequence, change),
            Op::RemoveSequence { sequence } => self.remove_sequence(sequence),
            Op::AddTrack {
                sequence,
                id,
                kind,
                index,
            } => {
                let s = self.sequence(sequence)?;
                if self.project.sequences[s].track(id.as_str()).is_some() {
                    return Err(fail(
                        "track.duplicate",
                        format!("track `{id}` already exists"),
                    ));
                }
                let tracks = &mut self.project.sequences[s].tracks;
                let index = index.unwrap_or(tracks.len()).min(tracks.len());
                tracks.insert(index, Track::new(id.clone(), *kind));
                Ok(())
            }
            Op::SetTrack {
                sequence,
                track,
                change,
            } => {
                let s = self.sequence(sequence)?;
                let t = self.track(s, track)?;
                let track = &mut self.project.sequences[s].tracks[t];
                if let Some(name) = &change.name {
                    track.name.clone_from(name);
                }
                track.enabled = change.enabled.unwrap_or(track.enabled);
                track.locked = change.locked.unwrap_or(track.locked);
                track.muted = change.muted.unwrap_or(track.muted);
                if let Some(bus) = &change.bus {
                    track.bus.clone_from(bus);
                }
                Ok(())
            }
            Op::AddBus { sequence, bus } => {
                let s = self.sequence(sequence)?;
                let found = &mut self.project.sequences[s];
                if found.bus(bus.id.as_str()).is_some() {
                    return Err(fail(
                        "bus.duplicate",
                        format!("`{sequence}` already has a bus `{}`", bus.id),
                    ));
                }
                found.buses.push(bus.clone());
                Ok(())
            }
            Op::SetBus {
                sequence,
                bus,
                change,
            } => {
                let at = self.bus_at(sequence, bus)?;
                let found = &mut self.project.sequences[at.0].buses[at.1];
                if let Some(name) = &change.name {
                    found.name.clone_from(name);
                }
                if let Some(to) = &change.to {
                    found.to.clone_from(to);
                }
                for (name, value) in &change.params {
                    match value {
                        Some(value) => found.params.insert(name.clone(), value.clone()),
                        None => found.params.remove(name),
                    };
                }
                Ok(())
            }
            Op::RemoveBus { sequence, bus } => {
                let at = self.bus_at(sequence, bus)?;
                let found = &mut self.project.sequences[at.0];
                let gone = found.buses.remove(at.1);
                for track in found
                    .tracks
                    .iter_mut()
                    .filter(|t| t.bus.as_ref() == Some(bus))
                {
                    track.bus.clone_from(&gone.to);
                }
                for other in found
                    .buses
                    .iter_mut()
                    .filter(|b| b.to.as_ref() == Some(bus))
                {
                    other.to.clone_from(&gone.to);
                }
                Ok(())
            }
            Op::BusFx {
                sequence,
                bus,
                change,
            } => {
                let at = self.bus_at(sequence, bus)?;
                let found = &mut self.project.sequences[at.0].buses[at.1];
                let missing = |index: &usize| {
                    fail("bus.effect", format!("`{bus}` has no effect {}", index + 1))
                };
                match change {
                    BusFx::Add { effect, index } => {
                        let index = index
                            .unwrap_or(found.effects.len())
                            .min(found.effects.len());
                        found.effects.insert(index, effect.clone());
                    }
                    BusFx::Set {
                        index,
                        enabled,
                        params,
                    } => {
                        let effect = found
                            .effects
                            .get_mut(*index)
                            .ok_or_else(|| missing(index))?;
                        effect.enabled = enabled.unwrap_or(effect.enabled);
                        for (name, value) in params {
                            match value {
                                Some(value) => effect.params.insert(name.clone(), value.clone()),
                                None => effect.params.remove(name),
                            };
                        }
                    }
                    BusFx::Remove { index } => {
                        if *index >= found.effects.len() {
                            return Err(missing(index));
                        }
                        found.effects.remove(*index);
                    }
                }
                Ok(())
            }
            Op::SetTrackRole {
                sequence,
                track,
                change,
            } => {
                let s = self.sequence(sequence)?;
                let t = self.track(s, track)?;
                let role = match &change.role {
                    Some(role) => Some(track_role(role.as_deref())?),
                    None => None,
                };
                let track = &mut self.project.sequences[s].tracks[t];
                if let Some(role) = role {
                    track.role = role;
                }
                track.sync_locked = change.sync_locked.unwrap_or(track.sync_locked);
                Ok(())
            }
            Op::RemoveTrack { sequence, track } => {
                let s = self.sequence(sequence)?;
                let t = self.track(s, track)?;
                let found = &self.project.sequences[s].tracks[t];
                if !found.clips.is_empty() {
                    return Err(fail(
                        "track.not_empty",
                        format!(
                            "track {track} still has {} clips; delete them first",
                            found.clips.len()
                        ),
                    ));
                }
                self.project.sequences[s].tracks.remove(t);
                Ok(())
            }
            Op::Insert {
                sequence,
                at,
                duration,
                targets,
            } => self.place(sequence, *at, *duration, targets, true),
            Op::Overwrite {
                sequence,
                at,
                duration,
                targets,
            } => self.place(sequence, *at, *duration, targets, false),
            Op::Delete { clips } => {
                for at in self.selection(clips)? {
                    self.unlocked(at.sequence, at.track)?;
                }
                for id in self.selection_ids(clips)? {
                    let at = self.find(&id);
                    self.project.sequences[at.sequence].tracks[at.track]
                        .clips
                        .remove(at.clip);
                }
                Ok(())
            }
            Op::RippleDelete { clips } => self.ripple_delete(clips),
            Op::Lift {
                sequence,
                from,
                to,
                tracks,
            } => self.clear(sequence, *from, *to, tracks.as_deref(), false),
            Op::Extract {
                sequence,
                from,
                to,
                tracks,
            } => self.clear(sequence, *from, *to, tracks.as_deref(), true),
            Op::Trim {
                clip,
                edge,
                delta,
                ripple,
            } => self.trim(clip, *edge, *delta, *ripple),
            Op::Roll { clip, delta } => self.roll(clip, *delta),
            Op::Slip { clip, delta } => {
                let at = self.locate(clip)?;
                self.aligned(at.sequence, *delta, "slip")?;
                for member in self.linked(at) {
                    self.unlocked(member.sequence, member.track)?;
                    self.project.clip_at_mut(member).slip(*delta);
                    self.check_source(member)?;
                }
                Ok(())
            }
            Op::Slide { clip, delta } => self.slide(clip, *delta),
            Op::Split { clip, at, tail } => {
                let found = self.locate(clip)?;
                let s = found.sequence;
                self.aligned(s, *at, "split point")?;
                let target = self.clip(found);
                if *at <= target.start || *at >= target.end() {
                    return Err(fail(
                        "split.outside",
                        format!(
                            "{} is not inside clip `{clip}` ({}–{})",
                            self.tc(s, *at),
                            self.tc(s, target.start),
                            self.tc(s, target.end())
                        ),
                    ));
                }
                if let Some(tail) = tail {
                    self.claim(tail)?;
                }
                let mut members: Vec<(usize, Id)> = Vec::new();
                for member in self.linked(found) {
                    let candidate = self.clip(member);
                    if candidate.start < *at && candidate.end() > *at {
                        self.unlocked(s, member.track)?;
                        members.push((member.track, candidate.id.clone()));
                    }
                }
                let named = tail.as_ref().map(|tail| (clip.clone(), tail.clone()));
                self.split_all(s, &members, *at, named);
                Ok(())
            }
            Op::Blade {
                sequence,
                at,
                tracks,
            } => {
                let s = self.sequence(sequence)?;
                self.aligned(s, *at, "blade point")?;
                let mut members = Vec::new();
                for t in self.track_set(s, tracks.as_deref())? {
                    for clip in &self.project.sequences[s].tracks[t].clips {
                        if clip.start < *at && clip.end() > *at {
                            members.push((t, clip.id.clone()));
                        }
                    }
                }
                self.split_all(s, &members, *at, None);
                Ok(())
            }
            Op::Move { clip, track, delta } => self.move_clip(clip, track.as_ref(), *delta),
            Op::Set { clip, changes } => self.set(clip, changes),
            Op::Speed { clip, speed } => self.speed(clip, speed),
            Op::FxAdd {
                clip,
                effect,
                index,
            } => {
                let at = self.editable(clip)?;
                let domain = self.domain(at)?;
                self.check_fx(
                    &effect.fx,
                    effect.version,
                    FxKind::Effect,
                    domain,
                    &effect.params,
                )?;
                let effects = &mut self.project.clip_at_mut(at).effects;
                let index = index.unwrap_or(effects.len()).min(effects.len());
                effects.insert(index, effect.clone());
                Ok(())
            }
            Op::FxSet {
                clip,
                index,
                enabled,
                params,
            } => {
                let at = self.editable(clip)?;
                let domain = self.domain(at)?;
                let effect = self.effect(at, *index)?.clone();
                let mut updated = effect.params.clone();
                for (name, value) in params {
                    match value {
                        Some(value) => updated.insert(name.clone(), value.clone()),
                        None => updated.remove(name),
                    };
                }
                self.check_fx(&effect.fx, effect.version, FxKind::Effect, domain, &updated)?;
                let target = &mut self.project.clip_at_mut(at).effects[*index];
                target.params = updated;
                target.enabled = enabled.unwrap_or(target.enabled);
                Ok(())
            }
            Op::FxRemove { clip, index } => {
                let at = self.editable(clip)?;
                self.effect(at, *index)?;
                self.project.clip_at_mut(at).effects.remove(*index);
                Ok(())
            }
            Op::FxFlatten { clip, index } => {
                let at = self.editable(clip)?;
                self.effect(at, *index)?;
                let sequence = &self.project.sequences[at.sequence];
                let (rate, markers) = (sequence.rate, sequence.markers.clone());
                let catalog = self.catalog;
                let clip = self.project.clip_at_mut(at);
                crate::look::flatten_into(clip, *index, rate, &markers, catalog)
                    .map_err(|message| fail("look.flatten", message))
            }
            Op::FxMove { clip, index, to } => {
                let at = self.editable(clip)?;
                self.effect(at, *index)?;
                let effects = &mut self.project.clip_at_mut(at).effects;
                let effect = effects.remove(*index);
                let to = (*to).min(effects.len());
                effects.insert(to, effect);
                Ok(())
            }
            // The outline, mode, and properties are checked with the rest of
            // the project when the edit is done.
            Op::MaskAdd { clip, mask, index } => {
                let at = self.editable(clip)?;
                let masks = &mut self.project.clip_at_mut(at).masks;
                if masks.iter().any(|other| other.name == mask.name) {
                    return Err(fail(
                        "mask.duplicate",
                        format!("clip `{clip}` already has a mask `{}`", mask.name),
                    ));
                }
                let index = index.unwrap_or(masks.len()).min(masks.len());
                masks.insert(index, mask.clone());
                Ok(())
            }
            Op::MaskSet {
                clip,
                name,
                shape,
                mode,
                inverted,
                params,
            } => {
                let at = self.editable(clip)?;
                let index = self.mask(at, name)?;
                let mask = &mut self.project.clip_at_mut(at).masks[index];
                if let Some(shape) = shape {
                    mask.shape = shape.clone();
                }
                mask.mode = mode.unwrap_or(mask.mode);
                mask.inverted = inverted.unwrap_or(mask.inverted);
                for (name, value) in params {
                    match value {
                        Some(value) => mask.params.insert(name.clone(), value.clone()),
                        None => mask.params.remove(name),
                    };
                }
                Ok(())
            }
            Op::MaskRemove { clip, name } => {
                let at = self.editable(clip)?;
                let index = self.mask(at, name)?;
                self.project.clip_at_mut(at).masks.remove(index);
                Ok(())
            }
            Op::AddTransition {
                id,
                from,
                to,
                fx,
                version,
                duration,
                align,
                params,
            } => self.add_transition(
                id.as_ref(),
                from,
                to,
                (fx, *version),
                *duration,
                *align,
                params,
            ),
            Op::SetTransition {
                id,
                duration,
                align,
                params,
            } => {
                let (s, t, index) = self.transition(id)?;
                self.unlocked(s, t)?;
                let mut transition = self.project.sequences[s].tracks[t].transitions[index].clone();
                if let Some(duration) = duration {
                    self.aligned(s, *duration, "transition duration")?;
                    transition.duration = *duration;
                }
                transition.align = align.unwrap_or(transition.align);
                for (name, value) in params {
                    match value {
                        Some(value) => transition.params.insert(name.clone(), value.clone()),
                        None => transition.params.remove(name),
                    };
                }
                let domain = self.project.sequences[s].tracks[t].kind.domain();
                let domain = domain.ok_or_else(|| {
                    fail("transition.domain", "subtitle tracks have no transitions")
                })?;
                self.check_fx(
                    &transition.fx,
                    transition.version,
                    FxKind::Transition,
                    domain,
                    &transition.params,
                )?;
                self.project.sequences[s].tracks[t].transitions[index] = transition;
                Ok(())
            }
            Op::RemoveTransition { id } => {
                let (s, t, index) = self.transition(id)?;
                self.unlocked(s, t)?;
                self.project.sequences[s].tracks[t]
                    .transitions
                    .remove(index);
                Ok(())
            }
            Op::AddMarker {
                sequence,
                id,
                at,
                duration,
                name,
                color,
                note,
            } => {
                let s = self.sequence(sequence)?;
                self.aligned(s, *at, "marker")?;
                self.aligned(s, *duration, "marker duration")?;
                let id = match id {
                    Some(id) => {
                        self.claim(id)?;
                        id.clone()
                    }
                    None => self.ids.next("mk"),
                };
                let markers = &mut self.project.sequences[s].markers;
                let index = markers.partition_point(|m| m.at <= *at);
                markers.insert(
                    index,
                    Marker {
                        id,
                        at: *at,
                        duration: *duration,
                        name: name.clone(),
                        color: *color,
                        note: note.clone(),
                    },
                );
                Ok(())
            }
            Op::SetMarker { id, change } => {
                let (s, index) = self.marker(id)?;
                for time in [change.at, change.duration].into_iter().flatten() {
                    self.aligned(s, time, "marker")?;
                }
                let markers = &mut self.project.sequences[s].markers;
                let marker = &mut markers[index];
                marker.at = change.at.unwrap_or(marker.at);
                marker.duration = change.duration.unwrap_or(marker.duration);
                if let Some(name) = &change.name {
                    marker.name.clone_from(name);
                }
                if let Some(color) = change.color {
                    marker.color = color;
                }
                if let Some(note) = &change.note {
                    marker.note.clone_from(note);
                }
                markers.sort_by_key(|m| m.at);
                Ok(())
            }
            Op::RemoveMarker { id } => {
                let (s, index) = self.marker(id)?;
                self.project.sequences[s].markers.remove(index);
                Ok(())
            }
            Op::Link { clips, group } => self.link(clips, group.as_ref()),
            Op::Unlink { clips } => {
                for id in clips {
                    let at = self.locate(id)?;
                    self.project.clip_at_mut(at).link = None;
                }
                Ok(())
            }
        }
    }

    // ---- lookup -----------------------------------------------------------

    fn sequence(&self, id: &Id) -> Outcome<usize> {
        self.project
            .sequence_index(id.as_str())
            .ok_or_else(|| fail("ref.missing", format!("no sequence `{id}`")))
    }

    fn track(&self, s: usize, id: &Id) -> Outcome<usize> {
        let sequence = &self.project.sequences[s];
        sequence
            .tracks
            .iter()
            .position(|track| track.id == *id)
            .ok_or_else(|| {
                let has: Vec<&str> = sequence.tracks.iter().map(|track| track.id.as_str()).collect();
                let kind = match id.as_str().chars().next() {
                    Some('A') => "audio",
                    Some('S') => "subtitle",
                    _ => "video",
                };
                fail(
                    "ref.missing",
                    format!(
                        "sequence `{}` has no track `{id}` (it has {}); `track add {id} {kind}` adds one",
                        sequence.id,
                        if has.is_empty() { "none".to_owned() } else { has.join(", ") }
                    ),
                )
            })
    }

    fn locate(&self, clip: &Id) -> Outcome<ClipAt> {
        self.project
            .locate(clip.as_str())
            .ok_or_else(|| fail("ref.missing", self.project.no_clip(clip.as_str())))
    }

    /// Locate a clip known to exist.
    fn find(&self, clip: &Id) -> ClipAt {
        self.project.locate(clip.as_str()).expect("clip exists")
    }

    fn clip(&self, at: ClipAt) -> &Clip {
        self.project.clip_at(at)
    }

    fn editable(&self, clip: &Id) -> Outcome<ClipAt> {
        let at = self.locate(clip)?;
        self.unlocked(at.sequence, at.track)?;
        Ok(at)
    }

    fn unlocked(&self, s: usize, t: usize) -> Outcome {
        let track = &self.project.sequences[s].tracks[t];
        if track.locked {
            return Err(fail(
                "track.locked",
                format!("track {} is locked", track.id),
            ));
        }
        Ok(())
    }

    fn unlocked_tracks(&self, s: usize) -> Vec<usize> {
        (0..self.project.sequences[s].tracks.len())
            .filter(|t| !self.project.sequences[s].tracks[*t].locked)
            .collect()
    }

    /// The tracks a ripple edit on `edited` moves: those and every
    /// unlocked track that is sync-locked.
    fn ripple_tracks(&self, s: usize, edited: &[usize]) -> Vec<usize> {
        self.unlocked_tracks(s)
            .into_iter()
            .filter(|t| edited.contains(t) || self.project.sequences[s].tracks[*t].sync_locked)
            .collect()
    }

    /// The named tracks, each unlocked, or every unlocked track.
    fn track_set(&self, s: usize, tracks: Option<&[Id]>) -> Outcome<Vec<usize>> {
        match tracks {
            None => Ok(self.unlocked_tracks(s)),
            Some(ids) => ids
                .iter()
                .map(|id| {
                    let t = self.track(s, id)?;
                    self.unlocked(s, t)?;
                    Ok(t)
                })
                .collect(),
        }
    }

    fn domain(&self, at: ClipAt) -> Outcome<Domain> {
        self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain()
            .ok_or_else(|| fail("clip.domain", "captions carry no properties or effects"))
    }

    /// The index of the clip's mask `name`.
    fn mask(&self, at: ClipAt, name: &Id) -> Outcome<usize> {
        let clip = self.clip(at);
        clip.masks
            .iter()
            .position(|mask| mask.name == *name)
            .ok_or_else(|| {
                fail(
                    "mask.missing",
                    format!("clip `{}` has no mask `{name}`", clip.id),
                )
            })
    }

    fn effect(&self, at: ClipAt, index: usize) -> Outcome<&Effect> {
        let clip = self.clip(at);
        clip.effects.get(index).ok_or_else(|| {
            fail(
                "fx.missing",
                format!(
                    "clip `{}` has {} effects; no effect {}",
                    clip.id,
                    clip.effects.len(),
                    index + 1
                ),
            )
        })
    }

    fn transition(&self, id: &Id) -> Outcome<(usize, usize, usize)> {
        for (s, sequence) in self.project.sequences.iter().enumerate() {
            for (t, track) in sequence.tracks.iter().enumerate() {
                if let Some(index) = track.transitions.iter().position(|tr| tr.id == *id) {
                    return Ok((s, t, index));
                }
            }
        }
        Err(fail("ref.missing", format!("no transition `{id}`")))
    }

    fn marker(&self, id: &Id) -> Outcome<(usize, usize)> {
        for (s, sequence) in self.project.sequences.iter().enumerate() {
            if let Some(index) = sequence.markers.iter().position(|m| m.id == *id) {
                return Ok((s, index));
            }
        }
        Err(fail("ref.missing", format!("no marker `{id}`")))
    }

    /// The clip and every clip linked to it, in track order.
    fn linked(&self, at: ClipAt) -> Vec<ClipAt> {
        let Some(link) = self.clip(at).link.clone() else {
            return vec![at];
        };
        let sequence = &self.project.sequences[at.sequence];
        let mut members = Vec::new();
        for (t, track) in sequence.tracks.iter().enumerate() {
            for (c, clip) in track.clips.iter().enumerate() {
                if clip.link.as_ref() == Some(&link) {
                    members.push(ClipAt {
                        sequence: at.sequence,
                        track: t,
                        clip: c,
                    });
                }
            }
        }
        members
    }

    /// Named clips plus their linked partners, without duplicates.
    fn selection(&self, clips: &[Id]) -> Outcome<Vec<ClipAt>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for id in clips {
            for member in self.linked(self.locate(id)?) {
                if seen.insert(member) {
                    out.push(member);
                }
            }
        }
        Ok(out)
    }

    fn selection_ids(&self, clips: &[Id]) -> Outcome<Vec<Id>> {
        Ok(self
            .selection(clips)?
            .into_iter()
            .map(|at| self.clip(at).id.clone())
            .collect())
    }

    // ---- checks -----------------------------------------------------------

    fn claim(&mut self, id: &Id) -> Outcome {
        if self.ids.is_retired(id.as_str()) {
            Err(fail(
                "id.retired",
                format!("`{id}` named something that was deleted; ids are never reused"),
            ))
        } else if self.ids.reserve(id) {
            Ok(())
        } else {
            Err(fail("id.taken", format!("`{id}` is already used")))
        }
    }

    fn tc(&self, s: usize, time: Time) -> String {
        let sequence = &self.project.sequences[s];
        format_timecode(sequence.start + time, sequence.rate, sequence.drop_frame)
    }

    fn aligned(&self, s: usize, time: Time, what: &str) -> Outcome {
        let rate = self.project.sequences[s].rate;
        if rate.is_aligned(time) {
            Ok(())
        } else {
            Err(fail(
                "time.misaligned",
                format!("{what} is not on a {rate} frame boundary"),
            ))
        }
    }

    fn check_source(&self, at: ClipAt) -> Outcome {
        let clip = self.clip(at);
        let Some((source_in, source_out)) = clip.source_range() else {
            return Ok(());
        };
        let available = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()).map(|m| m.duration),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map(Sequence::duration),
            _ => None,
        };
        let Some(available) = available else {
            return Ok(());
        };
        let (short_head, short_tail) = if clip.speed().is_reverse() {
            (source_out - available, -source_in)
        } else {
            (-source_in, source_out - available)
        };
        if short_head.is_positive() {
            return Err(fail(
                "clip.source_range",
                format!(
                    "clip `{}` runs out of media: it needs {} more before its first frame",
                    clip.id,
                    clip.time_before(short_head)
                ),
            ));
        }
        if short_tail.is_positive() {
            return Err(fail(
                "clip.source_range",
                format!(
                    "clip `{}` runs out of media: it needs {} more after its last frame",
                    clip.id,
                    clip.time_after(short_tail)
                ),
            ));
        }
        Ok(())
    }

    fn check_fx(
        &self,
        fx: &Id,
        version: u32,
        kind: FxKind,
        domain: Domain,
        params: &Params,
    ) -> Outcome {
        let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
            fail(
                "fx.unknown",
                crate::catalog::unknown(kind.name(), fx.as_str()),
            )
        })?;
        def.reads(version)
            .map_err(|message| fail("fx.version", message))?;
        if def.kind != kind || def.domain != domain {
            return Err(fail(
                "fx.kind",
                format!(
                    "`{fx}` is a {} {}, not a {} {}",
                    def.domain.name(),
                    def.kind.name(),
                    domain.name(),
                    kind.name()
                ),
            ));
        }
        for (name, param) in params {
            let param_def = def.find(name).ok_or_else(|| {
                let known: Vec<_> = def.params.iter().map(|p| p.name.as_str()).collect();
                fail(
                    "param.unknown",
                    format!(
                        "`{fx}` has no parameter `{name}` (has: {})",
                        known.join(", ")
                    ),
                )
            })?;
            param_def
                .check(param)
                .map_err(|message| fail("param.invalid", message))?;
        }
        Ok(())
    }

    fn check_placeable(&self, s: usize, t: usize, content: &Content) -> Outcome {
        let track = &self.project.sequences[s].tracks[t];
        if !accepts(track.kind, content) {
            return Err(fail(
                "clip.domain",
                format!(
                    "track {} is a {} track; captions go on subtitle tracks and media on video or audio tracks",
                    track.id,
                    track.kind.name()
                ),
            ));
        }
        let domain = track.kind.domain();
        match (content, domain) {
            (Content::Media { media, .. }, Some(domain)) => {
                let found = self
                    .project
                    .media(media.as_str())
                    .ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
                if !found.supports(domain) {
                    return Err(fail(
                        "clip.domain",
                        format!(
                            "media `{media}` has no {} for track {}",
                            domain.name(),
                            track.id
                        ),
                    ));
                }
            }
            (Content::Sequence { sequence, .. }, _) => {
                if self.project.sequence(sequence.as_str()).is_none() {
                    return Err(fail("ref.missing", format!("no sequence `{sequence}`")));
                }
            }
            (
                Content::Generator {
                    fx,
                    version,
                    params,
                },
                Some(domain),
            ) => {
                self.check_fx(fx, *version, FxKind::Generator, domain, params)?;
            }
            _ => {}
        }
        Ok(())
    }

    // ---- primitives -------------------------------------------------------

    /// Split each `(track, clip)` at `at`. Tails of clips that were linked
    /// together are linked together in a new group.
    fn split_all(
        &mut self,
        s: usize,
        members: &[(usize, Id)],
        at: Time,
        named: Option<(Id, Id)>,
    ) -> Vec<Id> {
        let mut tails_by_group: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
        let mut tails = Vec::new();
        for (t, id) in members {
            let tail_id = match &named {
                Some((clip, tail)) if clip == id => tail.clone(),
                _ => self.ids.next("c"),
            };
            let track = &mut self.project.sequences[s].tracks[*t];
            let c = track.clip_index(id.as_str()).expect("member exists");
            let offset = at - track.clips[c].start;
            let mut tail = track.clips[c].split_off(offset, tail_id.clone());
            if let Some(group) = tail.link.take() {
                tails_by_group
                    .entry(group)
                    .or_default()
                    .push(tail_id.clone());
            }
            for transition in &mut track.transitions {
                if transition.from == *id {
                    transition.from = tail_id.clone();
                }
            }
            track.clips.insert(c + 1, tail);
            tails.push(tail_id);
        }
        for group in tails_by_group.into_values().filter(|g| g.len() > 1) {
            let link = self.ids.next("L");
            for tail in group {
                let at = self.find(&tail);
                self.project.clip_at_mut(at).link = Some(link.clone());
            }
        }
        tails
    }

    /// Open `amount` of time at `at` on the ripple tracks of an edit on
    /// `edited`, splitting clips that span `at`. Clips in `exclude` neither
    /// split nor move.
    fn open_time(
        &mut self,
        s: usize,
        edited: &[usize],
        at: Time,
        amount: Time,
        exclude: &HashSet<Id>,
    ) {
        let tracks = self.ripple_tracks(s, edited);
        let mut spanning = Vec::new();
        for &t in &tracks {
            for clip in &self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id) && clip.start < at && clip.end() > at {
                    spanning.push((t, clip.id.clone()));
                }
            }
        }
        self.split_all(s, &spanning, at, None);
        for t in tracks {
            for clip in &mut self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id) && clip.start >= at {
                    clip.start += amount;
                }
            }
        }
    }

    /// Close `[from, to)` on the ripple tracks of an edit on `edited`.
    /// Fails when a clip outside `exclude` occupies the range, because
    /// closing it would cut that clip.
    fn close_time(
        &mut self,
        s: usize,
        edited: &[usize],
        from: Time,
        to: Time,
        exclude: &HashSet<Id>,
    ) -> Outcome {
        let tracks = self.ripple_tracks(s, edited);
        for &t in &tracks {
            let track = &self.project.sequences[s].tracks[t];
            if let Some(blocker) = track
                .clips
                .iter()
                .find(|c| !exclude.contains(&c.id) && c.start < to && c.end() > from)
            {
                return Err(fail(
                    "ripple.blocked",
                    format!(
                        "closing {}–{} would cut clip `{}` on track {}; lift it first or use extract to cut every track",
                        self.tc(s, from),
                        self.tc(s, to),
                        blocker.id,
                        track.id
                    ),
                ));
            }
        }
        for t in tracks {
            for clip in &mut self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id) && clip.start >= to {
                    clip.start -= to - from;
                }
            }
        }
        Ok(())
    }

    /// Remove `[from, to)` from one track, trimming or splitting clips at
    /// the edges.
    fn clear_track(&mut self, s: usize, t: usize, from: Time, to: Time) {
        let overlapping: Vec<Id> = self.project.sequences[s].tracks[t]
            .clips
            .iter()
            .filter(|c| c.start < to && c.end() > from)
            .map(|c| c.id.clone())
            .collect();
        for id in overlapping {
            let (start, end) = {
                let clip = self.project.sequences[s].tracks[t]
                    .clip(id.as_str())
                    .expect("exists");
                (clip.start, clip.end())
            };
            if end > to && start < from {
                self.split_all(s, &[(t, id.clone())], to, None);
            }
            let track = &mut self.project.sequences[s].tracks[t];
            let c = track.clip_index(id.as_str()).expect("exists");
            let clip = &mut track.clips[c];
            if clip.start >= from && clip.end() <= to {
                track.clips.remove(c);
            } else if clip.start < from {
                let end = clip.end();
                clip.move_tail(from - end);
            } else {
                let start = clip.start;
                clip.move_head(to - start);
            }
        }
    }

    // ---- operations -------------------------------------------------------

    fn place(
        &mut self,
        sequence: &Id,
        at: Time,
        duration: Time,
        targets: &[(Id, NewClip)],
        ripple: bool,
    ) -> Outcome {
        let s = self.sequence(sequence)?;
        self.aligned(s, at, "position")?;
        self.aligned(s, duration, "duration")?;
        if at.is_negative() || !duration.is_positive() {
            return Err(fail(
                "clip.time",
                "clips need a non-negative position and a positive duration",
            ));
        }
        if targets.is_empty() {
            return Err(fail("place.target", "name at least one target track"));
        }
        let mut placed = Vec::new();
        for (track, clip) in targets {
            let t = self.track(s, track)?;
            self.unlocked(s, t)?;
            if placed.iter().any(|(other, _)| *other == t) {
                return Err(fail(
                    "place.target",
                    format!("track {track} is named twice"),
                ));
            }
            self.check_placeable(s, t, &clip.content)?;
            let id = match &clip.id {
                Some(id) => {
                    self.claim(id)?;
                    id.clone()
                }
                None => self.ids.next("c"),
            };
            placed.push((t, id));
        }
        if ripple {
            let edited: Vec<usize> = placed.iter().map(|(t, _)| *t).collect();
            self.open_time(s, &edited, at, duration, &HashSet::new());
        } else {
            for (t, _) in &placed {
                self.clear_track(s, *t, at, at + duration);
            }
        }
        let link = (placed.len() > 1).then(|| self.ids.next("L"));
        for ((t, id), (_, spec)) in placed.into_iter().zip(targets) {
            let mut clip = Clip::new(id.clone(), spec.content.clone(), at, duration);
            clip.name.clone_from(&spec.name);
            clip.props.clone_from(&spec.props);
            clip.link.clone_from(&link);
            self.project.sequences[s].tracks[t].insert_sorted(clip);
            self.check_source(self.find(&id))?;
        }
        Ok(())
    }

    fn ripple_delete(&mut self, clips: &[Id]) -> Outcome {
        let selection = self.selection(clips)?;
        let Some(first) = selection.first() else {
            return Ok(());
        };
        let s = first.sequence;
        if selection.iter().any(|at| at.sequence != s) {
            return Err(fail(
                "ripple.sequences",
                "ripple-delete clips from one sequence at a time",
            ));
        }
        for at in &selection {
            self.unlocked(s, at.track)?;
        }
        let mut ranges: Vec<(Time, Time)> = selection
            .iter()
            .map(|at| (self.clip(*at).start, self.clip(*at).end()))
            .collect();
        ranges.sort();
        let mut merged: Vec<(Time, Time)> = Vec::new();
        for (from, to) in ranges {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        let ids: HashSet<Id> = selection
            .iter()
            .map(|at| self.clip(*at).id.clone())
            .collect();
        let edited: Vec<usize> = selection.iter().map(|at| at.track).collect();
        for &(from, to) in merged.iter().rev() {
            let here: HashSet<Id> = ids
                .iter()
                .filter(|id| {
                    let clip = self.clip(self.find(id));
                    clip.start >= from && clip.end() <= to
                })
                .cloned()
                .collect();
            self.close_time(s, &edited, from, to, &here)?;
            for id in &here {
                let at = self.find(id);
                self.project.sequences[s].tracks[at.track]
                    .clips
                    .remove(at.clip);
            }
        }
        Ok(())
    }

    fn clear(
        &mut self,
        sequence: &Id,
        from: Time,
        to: Time,
        tracks: Option<&[Id]>,
        close: bool,
    ) -> Outcome {
        let s = self.sequence(sequence)?;
        self.aligned(s, from, "range start")?;
        self.aligned(s, to, "range end")?;
        if from.is_negative() || to <= from {
            return Err(fail(
                "range.empty",
                "the range must start at or after 0 and end after it starts",
            ));
        }
        let tracks = self.track_set(s, tracks)?;
        for &t in &tracks {
            self.clear_track(s, t, from, to);
        }
        if close {
            for t in tracks {
                for clip in &mut self.project.sequences[s].tracks[t].clips {
                    if clip.start >= to {
                        clip.start -= to - from;
                    }
                }
            }
        }
        Ok(())
    }

    fn trim(&mut self, clip: &Id, edge: Edge, delta: Time, ripple: bool) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        self.aligned(s, delta, "trim amount")?;
        if delta == Time::ZERO {
            return Ok(());
        }
        let edge_time = |c: &Clip| match edge {
            Edge::In => c.start,
            Edge::Out => c.end(),
        };
        let target_edge = edge_time(self.clip(at));
        let members: Vec<Id> = self
            .linked(at)
            .into_iter()
            .filter(|m| edge_time(self.clip(*m)) == target_edge)
            .map(|m| self.clip(m).id.clone())
            .collect();
        for id in &members {
            let member = self.find(id);
            self.unlocked(s, member.track)?;
            let found = self.clip(member);
            let remaining = match edge {
                Edge::In => found.duration - delta,
                Edge::Out => found.duration + delta,
            };
            if !remaining.is_positive() {
                return Err(fail(
                    "trim.too_far",
                    format!("trimming `{id}` by that much would leave no frames"),
                ));
            }
            if edge == Edge::In && (found.start + delta).is_negative() && !ripple {
                return Err(fail(
                    "trim.too_far",
                    format!("`{id}` would start before the sequence"),
                ));
            }
        }
        let exclude: HashSet<Id> = members.iter().cloned().collect();
        if ripple {
            let edited: Vec<usize> = members.iter().map(|id| self.find(id).track).collect();
            match (edge, delta.is_positive()) {
                (Edge::Out, true) => self.open_time(s, &edited, target_edge, delta, &exclude),
                (Edge::Out, false) => {
                    self.close_time(s, &edited, target_edge + delta, target_edge, &exclude)?
                }
                (Edge::In, true) => {
                    self.close_time(s, &edited, target_edge, target_edge + delta, &exclude)?
                }
                (Edge::In, false) => self.open_time(s, &edited, target_edge, -delta, &exclude),
            }
        } else {
            for id in &members {
                let member = self.find(id);
                let found = self.clip(member);
                let (from, to) = match edge {
                    Edge::Out => (found.end(), found.end() + delta),
                    Edge::In => (found.start + delta, found.start),
                };
                let track = &self.project.sequences[s].tracks[member.track];
                if from < to && !track.is_free(from, to, &[id]) {
                    return Err(fail(
                        "trim.collision",
                        format!(
                            "extending `{id}` to {} runs into another clip on track {}; use ripple or trim the neighbor",
                            self.tc(s, if edge == Edge::Out { to } else { from }),
                            track.id
                        ),
                    ));
                }
            }
        }
        for id in &members {
            let member = self.find(id);
            let found = self.project.clip_at_mut(member);
            match edge {
                Edge::Out => found.move_tail(delta),
                Edge::In => {
                    found.move_head(delta);
                    if ripple {
                        found.start -= delta;
                    }
                }
            }
            self.check_source(member)?;
        }
        Ok(())
    }

    fn roll(&mut self, clip: &Id, delta: Time) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        self.aligned(s, delta, "roll amount")?;
        let cut = self.clip(at).end();
        let mut pairs = Vec::new();
        for member in self.linked(at) {
            let track = &self.project.sequences[s].tracks[member.track];
            let outgoing = &track.clips[member.clip];
            if outgoing.end() != cut {
                continue;
            }
            match track
                .clips
                .get(member.clip + 1)
                .filter(|next| next.start == cut)
            {
                Some(incoming) => {
                    pairs.push((member.track, outgoing.id.clone(), incoming.id.clone()))
                }
                None if member == at => {
                    return Err(fail(
                        "roll.no_cut",
                        format!(
                            "roll needs a clip that starts where `{clip}` ends on track {}",
                            track.id
                        ),
                    ));
                }
                None => {}
            }
        }
        for (t, outgoing, incoming) in &pairs {
            self.unlocked(s, *t)?;
            let track = &self.project.sequences[s].tracks[*t];
            let (a, b) = (
                track.clip(outgoing.as_str()).expect("exists"),
                track.clip(incoming.as_str()).expect("exists"),
            );
            if !(a.duration + delta).is_positive() || !(b.duration - delta).is_positive() {
                return Err(fail(
                    "roll.too_far",
                    format!("rolling the cut between `{outgoing}` and `{incoming}` that far leaves no frames"),
                ));
            }
        }
        for (_, outgoing, incoming) in pairs {
            let a = self.find(&outgoing);
            self.project.clip_at_mut(a).move_tail(delta);
            self.check_source(a)?;
            let b = self.find(&incoming);
            self.project.clip_at_mut(b).move_head(delta);
            self.check_source(b)?;
        }
        Ok(())
    }

    fn slide(&mut self, clip: &Id, delta: Time) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        self.aligned(s, delta, "slide amount")?;
        if delta == Time::ZERO {
            return Ok(());
        }
        let members: Vec<Id> = self
            .linked(at)
            .into_iter()
            .map(|m| self.clip(m).id.clone())
            .collect();
        for id in &members {
            let member = self.find(id);
            self.unlocked(s, member.track)?;
            let track = &self.project.sequences[s].tracks[member.track];
            let current = &track.clips[member.clip];
            let previous = member
                .clip
                .checked_sub(1)
                .map(|i| &track.clips[i])
                .filter(|p| p.end() == current.start);
            let next = track
                .clips
                .get(member.clip + 1)
                .filter(|n| n.start == current.end());
            let new_start = current.start + delta;
            if new_start.is_negative() {
                return Err(fail(
                    "slide.too_far",
                    format!("`{id}` would start before the sequence"),
                ));
            }
            let mut moves: Vec<(Id, Edge)> = Vec::new();
            if delta.is_positive() {
                match next {
                    Some(next) if next.duration > delta => moves.push((next.id.clone(), Edge::In)),
                    Some(next) => {
                        return Err(fail(
                            "slide.too_far",
                            format!("sliding `{id}` that far would consume `{}`", next.id),
                        ))
                    }
                    None if !track.is_free(current.end(), current.end() + delta, &[id]) => {
                        return Err(fail(
                            "slide.too_far",
                            format!("sliding `{id}` would overlap a clip on track {}", track.id),
                        ))
                    }
                    None => {}
                }
                if let Some(previous) = previous {
                    moves.push((previous.id.clone(), Edge::Out));
                }
            } else {
                match previous {
                    Some(previous) if previous.duration > -delta => {
                        moves.push((previous.id.clone(), Edge::Out))
                    }
                    Some(previous) => {
                        return Err(fail(
                            "slide.too_far",
                            format!("sliding `{id}` that far would consume `{}`", previous.id),
                        ))
                    }
                    None if !track.is_free(new_start, current.start, &[id]) => {
                        return Err(fail(
                            "slide.too_far",
                            format!("sliding `{id}` would overlap a clip on track {}", track.id),
                        ))
                    }
                    None => {}
                }
                if let Some(next) = next {
                    moves.push((next.id.clone(), Edge::In));
                }
            }
            for (neighbor, edge) in moves {
                let n = self.find(&neighbor);
                let found = self.project.clip_at_mut(n);
                match edge {
                    Edge::In => found.move_head(delta),
                    Edge::Out => found.move_tail(delta),
                }
                self.check_source(n)?;
            }
            let member = self.find(id);
            self.project.clip_at_mut(member).start += delta;
        }
        Ok(())
    }

    fn move_clip(&mut self, clip: &Id, track: Option<&Id>, delta: Time) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        self.aligned(s, delta, "move amount")?;
        let destination = match track {
            Some(track) => {
                let t = self.track(s, track)?;
                self.unlocked(s, t)?;
                self.check_placeable(s, t, &self.clip(at).content)?;
                t
            }
            None => at.track,
        };
        let members = self.linked(at);
        let mut moving = Vec::new();
        for member in &members {
            self.unlocked(s, member.track)?;
            let target = if *member == at {
                destination
            } else {
                member.track
            };
            moving.push((target, self.clip(*member).id.clone()));
        }
        let ids: Vec<&Id> = moving.iter().map(|(_, id)| id).collect();
        let mut lifted = Vec::new();
        for (target, id) in &moving {
            let found = self.clip(self.find(id));
            let (start, end) = (found.start + delta, found.end() + delta);
            if start.is_negative() {
                return Err(fail(
                    "move.too_far",
                    format!("`{id}` would start before the sequence"),
                ));
            }
            let track = &self.project.sequences[s].tracks[*target];
            if !track.is_free(start, end, &ids) {
                let blocker = track
                    .clips
                    .iter()
                    .find(|c| !ids.contains(&&c.id) && c.start < end && c.end() > start)
                    .map_or_else(String::new, |c| c.id.to_string());
                return Err(fail(
                    "move.collision",
                    format!(
                        "moving `{id}` to {} on track {} would overlap `{blocker}`; overwrite or make room first",
                        self.tc(s, start),
                        track.id
                    ),
                ));
            }
        }
        for (target, id) in moving {
            let found = self.find(&id);
            let mut clip = self.project.sequences[s].tracks[found.track]
                .clips
                .remove(found.clip);
            clip.start += delta;
            lifted.push((target, clip));
        }
        for (target, clip) in lifted {
            self.project.sequences[s].tracks[target].insert_sorted(clip);
        }
        Ok(())
    }

    fn set(&mut self, clip: &Id, changes: &[ClipChange]) -> Outcome {
        let at = self.editable(clip)?;
        let domain = self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain();
        for change in changes {
            match change {
                ClipChange::Name(name) => self.project.clip_at_mut(at).name.clone_from(name),
                ClipChange::Enabled(enabled) => self.project.clip_at_mut(at).enabled = *enabled,
                ClipChange::Parent(parent) => {
                    if let Some(parent) = parent {
                        self.check_parent(at, clip, parent)?;
                    }
                    self.project.clip_at_mut(at).parent.clone_from(parent);
                }
                ClipChange::Retime(retime) => {
                    let target = self.project.clip_at_mut(at);
                    if !matches!(target.content, Content::Media { .. }) {
                        return Err(fail(
                            "clip.domain",
                            format!("`{clip}` is not footage, so it has no frames to retime"),
                        ));
                    }
                    target.retime = *retime;
                }
                ClipChange::Angle(angle) => {
                    let target = self.project.clip_at_mut(at);
                    if !matches!(target.content, Content::Sequence { .. }) {
                        return Err(fail(
                            "clip.domain",
                            format!("`{clip}` holds no sequence to show one track of"),
                        ));
                    }
                    target.angle.clone_from(angle);
                }
                ClipChange::Caption(text) => match &mut self.project.clip_at_mut(at).content {
                    Content::Caption { text: current } => current.clone_from(text),
                    _ => return Err(fail("clip.domain", format!("`{clip}` is not a caption"))),
                },
                ClipChange::Param(name, value) => {
                    let domain = domain
                        .ok_or_else(|| fail("clip.domain", "captions carry no properties"))?;
                    if let Some(def) = self.catalog.prop(domain, name) {
                        if let Some(value) = value {
                            def.check(value).map_err(|m| fail("param.invalid", m))?;
                        }
                        let props = &mut self.project.clip_at_mut(at).props;
                        match value {
                            Some(value) => props.insert(name.clone(), value.clone()),
                            None => props.remove(name),
                        };
                        continue;
                    }
                    let generator = match &self.clip(at).content {
                        Content::Generator { fx, .. } => self.catalog.fx(fx.as_str()),
                        _ => None,
                    };
                    let def = generator.and_then(|g| g.find(name)).ok_or_else(|| {
                        let mut known: Vec<&str> = self
                            .catalog
                            .props(domain)
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect();
                        known.extend(
                            generator
                                .into_iter()
                                .flat_map(|g| g.params.iter().map(|p| p.name.as_str())),
                        );
                        fail(
                            "param.unknown",
                            format!(
                                "`{clip}` has no property `{name}` (has: {})",
                                known.join(", ")
                            ),
                        )
                    })?;
                    if let Some(value) = value {
                        def.check(value).map_err(|m| fail("param.invalid", m))?;
                    }
                    if let Content::Generator { params, .. } =
                        &mut self.project.clip_at_mut(at).content
                    {
                        match value {
                            Some(value) => params.insert(name.clone(), value.clone()),
                            None => params.remove(name),
                        };
                    }
                }
            }
        }
        Ok(())
    }

    fn speed(&mut self, clip: &Id, speed: &Speed) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        let rate = self.project.sequences[s].rate;
        let start = self.clip(at).start;
        let members: Vec<Id> = self
            .linked(at)
            .into_iter()
            .filter(|m| self.clip(*m).source_range().is_some())
            .map(|m| self.clip(m).id.clone())
            .collect();
        if self.clip(at).source_range().is_none() {
            return Err(fail(
                "speed.content",
                format!("`{clip}` has no source to retime"),
            ));
        }
        for id in &members {
            let member = self.find(id);
            self.unlocked(s, member.track)?;
            let found = self.clip(member);
            let span = found.speed().played(found.duration);
            // Keys are the clip's: a linked clip starting elsewhere changes speed at the same place.
            let mut speed = speed.clone();
            speed.shift(start - found.start);
            let mut frames = rate.frames_round(speed.time_for(span)).max(1);
            // A ramp plays its source unevenly, so its last frame is what must not run out.
            if speed.constant().is_none() && frames > 1 && speed.played(rate.time_of(frames)) > span
            {
                frames -= 1;
            }
            let duration = rate.time_of(frames);
            let track = &self.project.sequences[s].tracks[member.track];
            if !track.is_free(found.start, found.start + duration, &[id]) {
                return Err(fail(
                    "speed.collision",
                    format!(
                        "at speed {speed} `{id}` would run into the next clip on track {}",
                        track.id
                    ),
                ));
            }
            let found = self.project.clip_at_mut(member);
            found.duration = duration;
            if let Content::Media { speed: current, .. }
            | Content::Sequence { speed: current, .. } = &mut found.content
            {
                *current = speed;
            }
            self.check_source(member)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_transition(
        &mut self,
        id: Option<&Id>,
        from: &Id,
        to: &Id,
        (fx, version): (&Id, u32),
        duration: Time,
        align: Align,
        params: &Params,
    ) -> Outcome {
        let a = self.locate(from)?;
        let b = self.locate(to)?;
        let s = a.sequence;
        if a.sequence != b.sequence
            || a.track != b.track
            || b.clip != a.clip + 1
            || self.clip(a).end() != self.clip(b).start
        {
            return Err(fail(
                "transition.not_adjacent",
                format!("`{to}` must start exactly where `{from}` ends on the same track"),
            ));
        }
        self.unlocked(s, a.track)?;
        self.aligned(s, duration, "transition duration")?;
        let track = &self.project.sequences[s].tracks[a.track];
        if track
            .transitions
            .iter()
            .any(|t| t.from == *from && t.to == *to)
        {
            return Err(fail(
                "transition.duplicate",
                format!("the cut between `{from}` and `{to}` already has a transition"),
            ));
        }
        let domain = track
            .kind
            .domain()
            .ok_or_else(|| fail("transition.domain", "subtitle tracks have no transitions"))?;
        self.check_fx(fx, version, FxKind::Transition, domain, params)?;
        let id = match id {
            Some(id) => {
                self.claim(id)?;
                id.clone()
            }
            None => self.ids.next("t"),
        };
        self.project.sequences[s].tracks[a.track]
            .transitions
            .push(Transition {
                id,
                from: from.clone(),
                to: to.clone(),
                fx: fx.clone(),
                version,
                duration,
                align,
                params: params.clone(),
            });
        Ok(())
    }

    fn link(&mut self, clips: &[Id], group: Option<&Id>) -> Outcome {
        if clips.len() < 2 {
            return Err(fail("link.size", "link at least two clips"));
        }
        let mut sequence = None;
        for id in clips {
            let at = self.locate(id)?;
            if sequence.is_some_and(|s| s != at.sequence) {
                return Err(fail("link.sequences", "linked clips must share a sequence"));
            }
            sequence = Some(at.sequence);
        }
        let group = match group {
            Some(group) => {
                let in_use_as_group = self
                    .project
                    .sequences
                    .iter()
                    .flat_map(|s| &s.tracks)
                    .flat_map(|t| &t.clips)
                    .any(|c| c.link.as_ref() == Some(group));
                if !in_use_as_group {
                    self.claim(group)?;
                }
                group.clone()
            }
            None => self.ids.next("L"),
        };
        for id in clips {
            let at = self.find(id);
            self.project.clip_at_mut(at).link = Some(group.clone());
        }
        Ok(())
    }

    fn remove_media(&mut self, media: &Id) -> Outcome {
        let index = self
            .project
            .media
            .iter()
            .position(|m| m.id == *media)
            .ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
        let users: Vec<String> = self
            .project
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .filter(|c| matches!(&c.content, Content::Media { media: m, .. } if m == media))
            .map(|c| c.id.to_string())
            .collect();
        if !users.is_empty() {
            return Err(fail(
                "media.in_use",
                format!("media `{media}` is used by {}", users.join(", ")),
            ));
        }
        self.project.media.remove(index);
        Ok(())
    }

    fn bus_at(&mut self, sequence: &Id, bus: &Id) -> Result<(usize, usize), Diagnostic> {
        let s = self.sequence(sequence)?;
        let index = self.project.sequences[s]
            .buses
            .iter()
            .position(|found| found.id == *bus)
            .ok_or_else(|| fail("bus.unknown", format!("`{sequence}` has no bus `{bus}`")))?;
        Ok((s, index))
    }

    fn set_sequence(&mut self, sequence: &Id, change: &SequenceChange) -> Outcome {
        let s = self.sequence(sequence)?;
        let target = &mut self.project.sequences[s];
        if let Some(rate) = change.rate {
            if rate != target.rate
                && (target.tracks.iter().any(|t| !t.clips.is_empty()) || !target.markers.is_empty())
            {
                return Err(fail(
                    "sequence.conform",
                    format!("sequence `{sequence}` has content; changing its rate needs a conform"),
                ));
            }
            // The start keeps the timecode it reads, unless a new one is given.
            if rate != target.rate && change.start.is_none() {
                (target.start, target.drop_frame) = crate::time::restate_timecode(
                    target.start,
                    target.rate,
                    target.drop_frame,
                    rate,
                );
            }
            target.rate = rate;
        }
        if let Some(name) = &change.name {
            target.name.clone_from(name);
        }
        if let Some((width, height)) = change.size {
            target.width = width;
            target.height = height;
        }
        target.sample_rate = change.sample_rate.unwrap_or(target.sample_rate);
        target.color = change.color.unwrap_or(target.color);
        if let Some((start, drop_frame)) = change.start {
            target.start = start;
            target.drop_frame = drop_frame;
        }
        Ok(())
    }

    fn remove_sequence(&mut self, sequence: &Id) -> Outcome {
        let s = self.sequence(sequence)?;
        let nested_in: Vec<String> = self
            .project
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .filter(
                |c| matches!(&c.content, Content::Sequence { sequence: n, .. } if n == sequence),
            )
            .map(|c| c.id.to_string())
            .collect();
        if !nested_in.is_empty() {
            return Err(fail(
                "sequence.in_use",
                format!(
                    "sequence `{sequence}` is nested in {}",
                    nested_in.join(", ")
                ),
            ));
        }
        self.project.sequences.remove(s);
        Ok(())
    }

    /// Whether `clip`, at `at`, can move with `parent`: a clip of its sequence that does not
    /// already move with it.
    fn check_parent(&self, at: ClipAt, clip: &Id, parent: &Id) -> Result<(), Diagnostic> {
        let sequence = &self.project.sequences[at.sequence];
        let find = |id: &Id| {
            sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|clip| clip.id == *id)
        };
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if id == clip {
                return Err(fail(
                    "clip.parent",
                    format!("`{clip}` cannot move with `{parent}`, which moves with it"),
                ));
            }
            let found = find(id).ok_or_else(|| {
                fail(
                    "clip.parent",
                    format!("`{id}` is not a clip of the sequence `{clip}` is in"),
                )
            })?;
            ancestor = found.parent.as_ref();
        }
        Ok(())
    }

    /// Drop transitions whose clips no longer meet, link groups left with a single clip, and
    /// parents that no longer exist.
    fn housekeep(&mut self) {
        for sequence in &mut self.project.sequences {
            for track in &mut sequence.tracks {
                let clips = &track.clips;
                let notes = &mut self.notes;
                track.transitions.retain(|transition| {
                    let from = clips.iter().position(|c| c.id == transition.from);
                    let to = clips.iter().position(|c| c.id == transition.to);
                    let meets = matches!((from, to), (Some(a), Some(b)) if b == a + 1 && clips[a].end() == clips[b].start);
                    if !meets {
                        notes.push(format!(
                            "removed transition `{}`: `{}` and `{}` no longer meet",
                            transition.id, transition.from, transition.to
                        ));
                    }
                    meets
                });
            }
            let mut counts = BTreeMap::<Id, usize>::new();
            for clip in sequence.tracks.iter().flat_map(|t| &t.clips) {
                if let Some(link) = &clip.link {
                    *counts.entry(link.clone()).or_default() += 1;
                }
            }
            for clip in sequence.tracks.iter_mut().flat_map(|t| &mut t.clips) {
                if clip.link.as_ref().is_some_and(|link| counts[link] < 2) {
                    clip.link = None;
                }
            }
            let clips: BTreeSet<Id> = sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .map(|clip| clip.id.clone())
                .collect();
            for clip in sequence.tracks.iter_mut().flat_map(|t| &mut t.clips) {
                if clip.parent.as_ref().is_some_and(|id| !clips.contains(id)) {
                    self.notes.push(format!(
                        "`{}` no longer moves with `{}`, which is gone",
                        clip.id,
                        clip.parent.as_ref().map_or("", Id::as_str)
                    ));
                    clip.parent = None;
                }
            }
        }
    }
}
