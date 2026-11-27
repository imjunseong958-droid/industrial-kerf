//! Edit operations.
//!
//! Every change to a project is a list of operations applied atomically: the
//! whole list succeeds or the project is unchanged. Operations use NLE
//! semantics. Ripple edits act on every unlocked, sync-locked track (and on
//! the tracks they edit) so picture and sound stay in sync, and linked clips
//! (a picture clip and its sound) move, trim, split, slip, and delete
//! together.

use crate::catalog::Catalog;
use crate::diag::Diagnostic;
use crate::id::{Id, IdGen};
use crate::making::Step;
use crate::model::{
    Align, Clip, Content, Effect, MarkerColor, Mask, MaskMode, MaskShape, Media, Params, Project,
    Sequence, Synced, TrackKind,
};
use crate::speed::Speed;
use crate::time::{Rate, Time};
use crate::validate::errors;
use crate::value::Param;
use std::collections::{BTreeMap, BTreeSet, HashSet};

mod clip;
mod follow;
mod lookup;
mod media;
mod sequence;
mod timeline;
mod trim;

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
    /// its path, rate, timecode, duration, streams, reel, and recipe. Clips keep
    /// their timing; the edit is refused, naming them, when a clip no longer
    /// fits (its source range, a stream its track needs, transition
    /// handles). The steps that say how the media is made stay as they are:
    /// they change by their own operations.
    ReplaceMedia(Media),
    RemoveMedia {
        media: Id,
    },
    /// Say that a media is a recording of the same thing as another, and where it begins in
    /// it, or with none that it is lined up with nothing.
    SyncMedia {
        media: Id,
        sync: Option<Synced>,
    },
    /// Write a step of how a media's file is made ([`crate::making`]): the
    /// last of its steps, or the one that becomes step `index`, from 0.
    AddStep {
        media: Id,
        index: Option<usize>,
        step: Step,
    },
    /// Have step `index` of a media's making, from 0, be `step`.
    SetStep {
        media: Id,
        index: usize,
        step: Step,
    },
    /// Take step `index`, from 0, out of a media's making.
    RemoveStep {
        media: Id,
        index: usize,
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
    /// Move the cut after `clip`, trimming both sides: on every track its linked clips end with
    /// it, or `alone` on its own, which leaves theirs where it is, as a J or an L cut does.
    Roll {
        clip: Id,
        delta: Time,
        alone: bool,
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
    /// Have clips, and the clips linked to them, follow `to` ([`Clip::follows`]): they stay
    /// where they are, and from then on move when it moves.
    Attach {
        clips: Vec<Id>,
        to: Id,
    },
    /// Have clips, and the clips linked to them, follow nothing.
    Detach {
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
    let mut steps = Steps::new(project, catalog);
    for op in ops {
        steps.step(op)?;
    }
    steps.finish()
}

/// Operations applied one at a time, for a caller that reads the project
/// between them: what each one names is there to be looked at as it stood
/// when that operation ran. [`apply`] is every step, then
/// [`Steps::finish`].
pub struct Steps<'a> {
    from: &'a Project,
    editor: Editor<'a>,
    done: usize,
}

impl<'a> Steps<'a> {
    pub fn new(project: &'a Project, catalog: &'a Catalog) -> Self {
        Self {
            from: project,
            editor: Editor {
                project: project.clone(),
                catalog,
                ids: IdGen::default(),
                notes: Vec::new(),
                followed: follow::Followed::new(),
            },
            done: 0,
        }
    }

    /// The project as the operations so far leave it. It is checked as a
    /// whole only by [`Steps::finish`].
    pub fn project(&self) -> &Project {
        &self.editor.project
    }

    /// Apply the next operation. One that is refused leaves the steps of
    /// no further use.
    pub fn step(&mut self, op: &Op) -> Result<(), Rejected> {
        let editor = &mut self.editor;
        editor.ids = editor.project.id_gen();
        let before: Vec<Id> = editor.project.ids().cloned().collect();
        editor.followed = editor.followed();
        editor.apply(op).map_err(|diagnostic| Rejected {
            op: Some(self.done),
            diagnostics: vec![diagnostic],
        })?;
        editor.housekeep();
        editor.project.retire_missing(&before);
        self.done += 1;
        Ok(())
    }

    /// The project every step made and what they did, when they leave no
    /// error the project did not start with.
    pub fn finish(self) -> Result<(Project, Report), Rejected> {
        let Self { from, editor, .. } = self;
        let before: HashSet<(&'static str, Option<Id>)> = errors(from, editor.catalog)
            .into_iter()
            .map(|d| (d.code, d.entity))
            .collect();
        let introduced: Vec<_> = errors(&editor.project, editor.catalog)
            .into_iter()
            .filter(|d| !before.contains(&(d.code, d.entity.clone())))
            .collect();
        if !introduced.is_empty() {
            return Err(Rejected {
                op: None,
                diagnostics: introduced,
            });
        }
        let old: HashSet<&Id> = clip_ids(from).collect();
        let new: HashSet<&Id> = clip_ids(&editor.project).collect();
        let mut report = Report {
            created: clip_ids(&editor.project)
                .filter(|id| !old.contains(id))
                .cloned()
                .collect(),
            removed: clip_ids(from)
                .filter(|id| !new.contains(id))
                .cloned()
                .collect(),
            notes: editor.notes,
        };
        report.created.sort();
        report.removed.sort();
        Ok((editor.project, report))
    }
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

fn fail(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, message)
}

struct Editor<'a> {
    project: Project,
    catalog: &'a Catalog,
    ids: IdGen,
    notes: Vec<String>,
    /// Where each clip another follows stood before the operation at hand, for what follows
    /// one it removes.
    followed: follow::Followed,
}

impl Editor<'_> {
    fn apply(&mut self, op: &Op) -> Outcome {
        match op {
            Op::AddMedia(media) => self.add_media(media),
            Op::RelinkMedia { media, path } => self.relink_media(media, path),
            Op::ReplaceMedia(media) => self.replace_media(media),
            Op::RemoveMedia { media } => self.remove_media(media),
            Op::SyncMedia { media, sync } => self.sync_media(media, sync.as_ref()),
            Op::AddStep { media, index, step } => self.add_step(media, *index, step),
            Op::SetStep { media, index, step } => self.set_step(media, *index, step),
            Op::RemoveStep { media, index } => self.remove_step(media, *index),
            Op::AddSequence(sequence) => self.add_sequence(sequence),
            Op::SetSequence { sequence, change } => self.set_sequence(sequence, change),
            Op::RemoveSequence { sequence } => self.remove_sequence(sequence),
            Op::AddTrack {
                sequence,
                id,
                kind,
                index,
            } => self.add_track(sequence, id, *kind, *index),
            Op::SetTrack {
                sequence,
                track,
                change,
            } => self.set_track(sequence, track, change),
            Op::AddBus { sequence, bus } => self.add_bus(sequence, bus),
            Op::SetBus {
                sequence,
                bus,
                change,
            } => self.set_bus(sequence, bus, change),
            Op::RemoveBus { sequence, bus } => self.remove_bus(sequence, bus),
            Op::BusFx {
                sequence,
                bus,
                change,
            } => self.bus_fx(sequence, bus, change),
            Op::SetTrackRole {
                sequence,
                track,
                change,
            } => self.set_track_role(sequence, track, change),
            Op::RemoveTrack { sequence, track } => self.remove_track(sequence, track),
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
            Op::Delete { clips } => self.delete(clips),
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
            Op::Roll { clip, delta, alone } => self.roll(clip, *delta, *alone),
            Op::Slip { clip, delta } => self.slip(clip, *delta),
            Op::Slide { clip, delta } => self.slide(clip, *delta),
            Op::Split { clip, at, tail } => self.split(clip, *at, tail.as_ref()),
            Op::Blade {
                sequence,
                at,
                tracks,
            } => self.blade(sequence, *at, tracks.as_deref()),
            Op::Move { clip, track, delta } => self.move_clip(clip, track.as_ref(), *delta),
            Op::Set { clip, changes } => self.set(clip, changes),
            Op::Speed { clip, speed } => self.speed(clip, speed),
            Op::FxAdd {
                clip,
                effect,
                index,
            } => self.fx_add(clip, effect, *index),
            Op::FxSet {
                clip,
                index,
                enabled,
                params,
            } => self.fx_set(clip, *index, *enabled, params),
            Op::FxRemove { clip, index } => self.fx_remove(clip, *index),
            Op::FxFlatten { clip, index } => self.fx_flatten(clip, *index),
            Op::FxMove { clip, index, to } => self.fx_move(clip, *index, *to),
            Op::MaskAdd { clip, mask, index } => self.mask_add(clip, mask, *index),
            Op::MaskSet {
                clip,
                name,
                shape,
                mode,
                inverted,
                params,
            } => self.mask_set(clip, name, shape.as_ref(), *mode, *inverted, params),
            Op::MaskRemove { clip, name } => self.mask_remove(clip, name),
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
            } => self.set_transition(id, *duration, *align, params),
            Op::RemoveTransition { id } => self.remove_transition(id),
            Op::AddMarker {
                sequence,
                id,
                at,
                duration,
                name,
                color,
                note,
            } => self.add_marker(sequence, id.as_ref(), (*at, *duration), name, *color, note),
            Op::SetMarker { id, change } => self.set_marker(id, change),
            Op::RemoveMarker { id } => self.remove_marker(id),
            Op::Link { clips, group } => self.link(clips, group.as_ref()),
            Op::Unlink { clips } => self.unlink(clips),
            Op::Attach { clips, to } => self.attach(clips, to),
            Op::Detach { clips } => self.detach(clips),
        }
    }

    /// Drop transitions whose clips no longer meet, link groups left with a single clip, and
    /// parents that no longer exist, and give what followed a clip that is gone another to
    /// follow.
    fn housekeep(&mut self) {
        let followed = std::mem::take(&mut self.followed);
        self.refollow(&followed);
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

/// Keep each look of `clip` on the moment of its picture it was on, once the clip's first frame
/// has come `delta` later in it: a zoom pulse's `hit` on a split's second part, or on a clip whose
/// head was trimmed, is where its frame still is, as keyframes are ([`Look::shift_moments`]).
///
/// [`Look::shift_moments`]: crate::look::Look::shift_moments
fn keep_looks(clip: &mut Clip, delta: Time, rate: Rate, catalog: &Catalog) {
    for effect in &mut clip.effects {
        if let Some(look) = catalog.look(effect.fx.as_str(), effect.version) {
            look.shift_moments(&mut effect.params, delta, rate);
        }
    }
}
