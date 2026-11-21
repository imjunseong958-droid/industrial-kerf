//! A script brought forward: written on one project, and run on that
//! project as it has been edited since.
//!
//! Running the lines again is the right edit only while each still means
//! what it meant. So the two projects are stepped through together, one
//! operation at a time, and each operation is held to three things:
//!
//! - A clip it names is still there, on its track, playing the same length
//!   of the same source. Where it starts may have moved.
//! - A place it names is carried by the clips that held it: every clip that
//!   was under that time, or met it with an edge, is still placed as it was,
//!   and they have all moved by the same amount. The place moves with them.
//! - It changes what it changed. Every clip, transition, marker, media,
//!   track, bus and sequence the operation changed when it was written, it
//!   changes now, from the same state to the same state, and it changes
//!   nothing else. A clip that only moves along its track, as a ripple moves
//!   every clip after an edit, is not held to this: a ripple follows the
//!   timeline as it is.
//!
//! An operation that fails any of them stops the script, with a diagnostic
//! that names the line and what stood in its way.

use crate::facts::NoFacts;
use crate::script::lines_with;
use crate::text::Clock;
use kerf_model::{
    Catalog, Clip, ClipChange, Content, Diagnostic, Effect, Id, Mask, Media, Op, Params, Project,
    Rate, Report, Retime, Speed, Steps, Time, TrackKind, Transition,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What [`bring_forward`] made of a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Forwarded {
    /// The edited project after the script.
    pub project: Project,
    /// The operations as they ran on it: the script's, with each place and
    /// each clip the script itself made named as the edited project has it.
    pub ops: Vec<Op>,
    pub report: Report,
}

/// Run `script`, the operations of an edit written on `base`, on `current`,
/// which is `base` as it was edited since. Refused, changing nothing, where
/// a line would not be the edit it was (see the module's notes).
pub fn bring_forward(
    base: &Project,
    current: &Project,
    catalog: &Catalog,
    script: &str,
) -> Result<Forwarded, Vec<Diagnostic>> {
    let lines = lines_with(base, catalog, &NoFacts, script)?.lines;
    let mut was = Steps::new(base, catalog);
    let mut now = Steps::new(current, catalog);
    let mut names = Names::default();
    let mut before = (
        parts(was.project(), &names),
        parts(now.project(), &Names::default()),
    );
    let mut ops = Vec::new();
    for (line, line_ops) in &lines {
        let at = |diagnostic: Diagnostic| vec![diagnostic.at(*line, 1)];
        let rejected = |rejected: kerf_model::Rejected| -> Vec<Diagnostic> {
            rejected
                .diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.at(*line, 1))
                .collect()
        };
        for op in line_ops {
            let carried = Place {
                was: was.project(),
                now: now.project(),
                names: &names,
            }
            .op(op)
            .map_err(at)?;
            let ids = (all_ids(was.project()), all_ids(now.project()));
            was.step(op).map_err(rejected)?;
            now.step(&carried).map_err(rejected)?;
            names.learn(op, (&ids.0, was.project()), (&ids.1, now.project()));
            let after = (
                parts(was.project(), &names),
                parts(now.project(), &Names::default()),
            );
            same_change((&before.0, &after.0), (&before.1, &after.1)).map_err(at)?;
            before = after;
            ops.push(carried);
        }
    }
    let (project, report) = now.finish().map_err(|rejected| rejected.diagnostics)?;
    Ok(Forwarded {
        project,
        ops,
        report,
    })
}

fn all_ids(project: &Project) -> HashSet<Id> {
    project.ids().cloned().collect()
}

/// The ids the script's own operations made on the project it was written
/// on, each with the id the same operation made on the edited project. An
/// id the script does not make names the same thing in both: ids are never
/// used again.
#[derive(Default)]
struct Names(HashMap<Id, Id>);

impl Names {
    fn now(&self, id: &Id) -> Id {
        self.0.get(id).cloned().unwrap_or_else(|| id.clone())
    }

    /// Pair the ids `op` made on each side. Those the operation names are
    /// the same on both; those it left to be made are numbered upwards as
    /// they are made, so they pair in that order. Where one side made more,
    /// the rest pair with nothing, and [`same_change`] says what they are.
    fn learn(&mut self, op: &Op, was: (&HashSet<Id>, &Project), now: (&HashSet<Id>, &Project)) {
        let named = named_new(op);
        let made = |(before, after): (&HashSet<Id>, &Project)| {
            let mut made: BTreeMap<String, BTreeSet<(u64, Id)>> = BTreeMap::new();
            for id in after.ids() {
                if before.contains(id) || named.contains(&id) {
                    continue;
                }
                let text = id.as_str();
                let split = text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                let number = text[split..].parse().unwrap_or_default();
                made.entry(text[..split].to_owned())
                    .or_default()
                    .insert((number, id.clone()));
            }
            made
        };
        let (was, mut now) = (made(was), made(now));
        for (kind, was) in was {
            let now = now.remove(&kind).unwrap_or_default();
            for ((_, was), (_, now)) in was.into_iter().zip(now) {
                self.0.insert(was, now);
            }
        }
    }
}

/// The ids `op` gives to what it makes.
fn named_new(op: &Op) -> Vec<&Id> {
    match op {
        Op::AddMedia(media) | Op::ReplaceMedia(media) => vec![&media.id],
        Op::AddSequence(sequence) => vec![&sequence.id],
        Op::Insert { targets, .. } | Op::Overwrite { targets, .. } => targets
            .iter()
            .filter_map(|(_, clip)| clip.id.as_ref())
            .collect(),
        Op::Split { tail, .. } => tail.iter().collect(),
        Op::AddTransition { id, .. } | Op::AddMarker { id, .. } => id.iter().collect(),
        Op::Link { group, .. } => group.iter().collect(),
        _ => Vec::new(),
    }
}

/// Where a clip plays, for how long, and what of its source: all of a clip
/// that a move along its track leaves as it was.
#[derive(Clone, Debug, PartialEq)]
struct Placed {
    sequence: Id,
    track: Id,
    duration: Time,
    source: Option<(Id, Time, Speed)>,
}

fn placed(sequence: &Id, track: &Id, clip: &Clip) -> Placed {
    Placed {
        sequence: sequence.clone(),
        track: track.clone(),
        duration: clip.duration,
        source: match &clip.content {
            Content::Media {
                media: source,
                source_in,
                speed,
            }
            | Content::Sequence {
                sequence: source,
                source_in,
                speed,
            } => Some((source.clone(), *source_in, speed.clone())),
            Content::Generator { .. } | Content::Caption { .. } => None,
        },
    }
}

/// A clip of `project` by id, as [`placed`] has it, and where it starts.
fn find(project: &Project, id: &Id) -> Option<(Placed, Time)> {
    let at = project.locate(id.as_str())?;
    let sequence = &project.sequences[at.sequence];
    let track = &sequence.tracks[at.track];
    let clip = &track.clips[at.clip];
    Some((placed(&sequence.id, &track.id, clip), clip.start))
}

/// How a clip that is not placed as it was differs, for a sentence.
fn displaced(was: &Placed, now: &Placed) -> String {
    let speed = |placed: &Placed| placed.source.as_ref().map(|(_, _, speed)| speed.clone());
    if (&was.sequence, &was.track) != (&now.sequence, &now.track) {
        format!("was moved to track {}", now.track)
    } else if speed(was) != speed(now) {
        "was retimed".to_owned()
    } else if was.duration != now.duration {
        "was cut".to_owned()
    } else {
        "was slipped".to_owned()
    }
}

/// One operation on its way from the project it was written on to the
/// edited one, before either has run it.
struct Place<'a> {
    was: &'a Project,
    now: &'a Project,
    names: &'a Names,
}

type Carried<T> = Result<T, Diagnostic>;

impl Place<'_> {
    /// The id now of a clip an operation names, which must be placed as it
    /// was, and how far its start has moved.
    fn clip(&self, id: &Id) -> Carried<(Id, Time)> {
        let now = self.names.now(id);
        let Some((was, start)) = find(self.was, id) else {
            // The script's own project refuses the operation.
            return Ok((now, Time::ZERO));
        };
        let Some((found, started)) = find(self.now, &now) else {
            return Err(Diagnostic::error(
                "forward.gone",
                format!("clip `{now}` is gone since this was proposed"),
            )
            .entity(&now));
        };
        if was != found {
            return Err(Diagnostic::error(
                "forward.changed",
                format!(
                    "clip `{now}` {} since this was proposed",
                    displaced(&was, &found)
                ),
            )
            .entity(&now));
        }
        Ok((now, started - start))
    }

    fn clips(&self, ids: &[Id]) -> Carried<Vec<Id>> {
        ids.iter().map(|id| Ok(self.clip(id)?.0)).collect()
    }

    /// Where `at` on `sequence` is now. The clips that held it, each under
    /// it or meeting it with an edge, carry it: they are all placed as they
    /// were and have moved by one amount. A time no clip held stays.
    fn place(&self, sequence: &Id, at: Time) -> Carried<Time> {
        let Some(found) = self.was.sequence(sequence.as_str()) else {
            return Ok(at);
        };
        let clock = Clock::of_sequence(found);
        let said = clock.print_position(at);
        let mut carried: Option<(Time, Id)> = None;
        for track in &found.tracks {
            for clip in &track.clips {
                if clip.start > at || clip.end() < at {
                    continue;
                }
                let id = self.names.now(&clip.id);
                let Some((now, started)) = find(self.now, &id) else {
                    return Err(Diagnostic::error(
                        "forward.gone",
                        format!("{said} was on clip `{id}`, which is gone since this was proposed"),
                    )
                    .entity(&id));
                };
                let was = placed(&found.id, &track.id, clip);
                if was != now {
                    return Err(Diagnostic::error(
                        "forward.changed",
                        format!(
                            "{said} was on clip `{id}`, which {} since this was proposed",
                            displaced(&was, &now)
                        ),
                    )
                    .entity(&id));
                }
                let delta = started - clip.start;
                match &carried {
                    Some((other, with)) if *other != delta => {
                        return Err(Diagnostic::error(
                            "forward.place",
                            format!(
                                "{said} was on clips `{with}` and `{id}`, which are no longer together there"
                            ),
                        )
                        .entity(&id));
                    }
                    Some(_) => {}
                    None => carried = Some((delta, id)),
                }
            }
        }
        Ok(at + carried.map_or(Time::ZERO, |(delta, _)| delta))
    }

    /// The operation as the edited project is to run it.
    fn op(&self, op: &Op) -> Carried<Op> {
        Ok(match op {
            Op::AddMedia(_)
            | Op::RelinkMedia { .. }
            | Op::ReplaceMedia(_)
            | Op::RemoveMedia { .. }
            | Op::SyncMedia { .. }
            | Op::AddStep { .. }
            | Op::SetStep { .. }
            | Op::RemoveStep { .. }
            | Op::AddSequence(_)
            | Op::SetSequence { .. }
            | Op::RemoveSequence { .. }
            | Op::AddBus { .. }
            | Op::SetBus { .. }
            | Op::RemoveBus { .. }
            | Op::BusFx { .. }
            | Op::AddTrack { .. }
            | Op::SetTrack { .. }
            | Op::RemoveTrack { .. }
            | Op::SetTrackRole { .. } => op.clone(),
            Op::Insert {
                sequence,
                at,
                duration,
                targets,
            } => Op::Insert {
                sequence: sequence.clone(),
                at: self.place(sequence, *at)?,
                duration: *duration,
                targets: targets.clone(),
            },
            Op::Overwrite {
                sequence,
                at,
                duration,
                targets,
            } => Op::Overwrite {
                sequence: sequence.clone(),
                at: self.place(sequence, *at)?,
                duration: *duration,
                targets: targets.clone(),
            },
            Op::Delete { clips } => Op::Delete {
                clips: self.clips(clips)?,
            },
            Op::RippleDelete { clips } => Op::RippleDelete {
                clips: self.clips(clips)?,
            },
            Op::Lift {
                sequence,
                from,
                to,
                tracks,
            } => {
                let (from, to) = self.range(sequence, *from, *to)?;
                Op::Lift {
                    sequence: sequence.clone(),
                    from,
                    to,
                    tracks: tracks.clone(),
                }
            }
            Op::Extract {
                sequence,
                from,
                to,
                tracks,
            } => {
                let (from, to) = self.range(sequence, *from, *to)?;
                Op::Extract {
                    sequence: sequence.clone(),
                    from,
                    to,
                    tracks: tracks.clone(),
                }
            }
            Op::Trim {
                clip,
                edge,
                delta,
                ripple,
            } => Op::Trim {
                clip: self.clip(clip)?.0,
                edge: *edge,
                delta: *delta,
                ripple: *ripple,
            },
            Op::Roll { clip, delta, alone } => Op::Roll {
                clip: self.clip(clip)?.0,
                delta: *delta,
                alone: *alone,
            },
            Op::Slip { clip, delta } => Op::Slip {
                clip: self.clip(clip)?.0,
                delta: *delta,
            },
            Op::Slide { clip, delta } => Op::Slide {
                clip: self.clip(clip)?.0,
                delta: *delta,
            },
            Op::Split { clip, at, tail } => {
                // The clip it cuts carries the cut.
                let (clip, moved) = self.clip(clip)?;
                Op::Split {
                    clip,
                    at: *at + moved,
                    tail: tail.clone(),
                }
            }
            Op::Blade {
                sequence,
                at,
                tracks,
            } => Op::Blade {
                sequence: sequence.clone(),
                at: self.place(sequence, *at)?,
                tracks: tracks.clone(),
            },
            Op::Move { clip, track, delta } => Op::Move {
                clip: self.clip(clip)?.0,
                track: track.clone(),
                delta: *delta,
            },
            Op::Set { clip, changes } => Op::Set {
                clip: self.clip(clip)?.0,
                changes: changes
                    .iter()
                    .map(|change| match change {
                        ClipChange::Parent(Some(parent)) => {
                            ClipChange::Parent(Some(self.names.now(parent)))
                        }
                        other => other.clone(),
                    })
                    .collect(),
            },
            Op::Speed { clip, speed } => Op::Speed {
                clip: self.clip(clip)?.0,
                speed: speed.clone(),
            },
            Op::FxAdd {
                clip,
                effect,
                index,
            } => Op::FxAdd {
                clip: self.clip(clip)?.0,
                effect: effect.clone(),
                index: *index,
            },
            Op::FxSet {
                clip,
                index,
                enabled,
                params,
            } => Op::FxSet {
                clip: self.clip(clip)?.0,
                index: *index,
                enabled: *enabled,
                params: params.clone(),
            },
            Op::FxRemove { clip, index } => Op::FxRemove {
                clip: self.clip(clip)?.0,
                index: *index,
            },
            Op::FxFlatten { clip, index } => Op::FxFlatten {
                clip: self.clip(clip)?.0,
                index: *index,
            },
            Op::FxMove { clip, index, to } => Op::FxMove {
                clip: self.clip(clip)?.0,
                index: *index,
                to: *to,
            },
            Op::MaskAdd { clip, mask, index } => Op::MaskAdd {
                clip: self.clip(clip)?.0,
                mask: mask.clone(),
                index: *index,
            },
            Op::MaskSet {
                clip,
                name,
                shape,
                mode,
                inverted,
                params,
            } => Op::MaskSet {
                clip: self.clip(clip)?.0,
                name: name.clone(),
                shape: shape.clone(),
                mode: *mode,
                inverted: *inverted,
                params: params.clone(),
            },
            Op::MaskRemove { clip, name } => Op::MaskRemove {
                clip: self.clip(clip)?.0,
                name: name.clone(),
            },
            Op::AddTransition {
                id,
                from,
                to,
                fx,
                version,
                duration,
                align,
                params,
            } => Op::AddTransition {
                id: id.clone(),
                from: self.clip(from)?.0,
                to: self.clip(to)?.0,
                fx: fx.clone(),
                version: *version,
                duration: *duration,
                align: *align,
                params: params.clone(),
            },
            Op::SetTransition {
                id,
                duration,
                align,
                params,
            } => Op::SetTransition {
                id: self.names.now(id),
                duration: *duration,
                align: *align,
                params: params.clone(),
            },
            Op::RemoveTransition { id } => Op::RemoveTransition {
                id: self.names.now(id),
            },
            Op::AddMarker {
                sequence,
                id,
                at,
                duration,
                name,
                color,
                note,
            } => Op::AddMarker {
                sequence: sequence.clone(),
                id: id.clone(),
                at: self.place(sequence, *at)?,
                duration: *duration,
                name: name.clone(),
                color: *color,
                note: note.clone(),
            },
            Op::SetMarker { id, change } => {
                let mut change = change.clone();
                let sequence = self
                    .was
                    .sequences
                    .iter()
                    .find(|sequence| sequence.marker(id.as_str()).is_some())
                    .map(|sequence| sequence.id.clone());
                if let (Some(at), Some(sequence)) = (change.at, sequence) {
                    change.at = Some(self.place(&sequence, at)?);
                }
                Op::SetMarker {
                    id: self.names.now(id),
                    change,
                }
            }
            Op::RemoveMarker { id } => Op::RemoveMarker {
                id: self.names.now(id),
            },
            Op::Link { clips, group } => Op::Link {
                clips: self.clips(clips)?,
                group: group.as_ref().map(|group| self.names.now(group)),
            },
            Op::Unlink { clips } => Op::Unlink {
                clips: self.clips(clips)?,
            },
        })
    }

    /// Both ends of a range, each carried by what held it.
    fn range(&self, sequence: &Id, from: Time, to: Time) -> Carried<(Time, Time)> {
        let carried = (self.place(sequence, from)?, self.place(sequence, to)?);
        if carried.1 <= carried.0 {
            let clock = self.was.sequence(sequence.as_str()).map(Clock::of_sequence);
            let said = |time: Time| {
                clock
                    .as_ref()
                    .map_or_else(String::new, |clock| clock.print_position(time))
            };
            return Err(Diagnostic::error(
                "forward.place",
                format!(
                    "what was between {} and {} is no longer in that order",
                    said(from),
                    said(to)
                ),
            ));
        }
        Ok(carried)
    }
}

/// One thing an operation can change, named as the edited project names it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Media(Id),
    Sequence(Id),
    Track(Id, Id),
    Bus(Id, Id),
    Marker(Id),
    Transition(Id),
    /// Where a clip plays and what of its source ([`Placed`]).
    Placed(Id),
    Properties(Id),
    Effects(Id),
    Masks(Id),
}

impl Key {
    /// Whether it is how a clip looks or sounds, which a clip that an
    /// operation makes or removes has from that operation or from the clip
    /// it was cut from. Where such a clip plays is what is compared.
    fn is_look(&self) -> bool {
        matches!(
            self,
            Self::Properties(_) | Self::Effects(_) | Self::Masks(_)
        )
    }

    fn what(&self) -> String {
        match self {
            Self::Media(id) => format!("media `{id}`"),
            Self::Sequence(id) => format!("sequence `{id}`"),
            Self::Track(_, id) => format!("track {id}"),
            Self::Bus(_, id) => format!("bus `{id}`"),
            Self::Marker(id) => format!("marker `{id}`"),
            Self::Transition(id) => format!("transition `{id}`"),
            Self::Placed(id) => format!("clip `{id}`"),
            Self::Properties(id) => format!("the properties of clip `{id}`"),
            Self::Effects(id) => format!("the effects of clip `{id}`"),
            Self::Masks(id) => format!("the masks of clip `{id}`"),
        }
    }

    fn id(&self) -> &Id {
        match self {
            Self::Media(id)
            | Self::Sequence(id)
            | Self::Track(_, id)
            | Self::Bus(_, id)
            | Self::Marker(id)
            | Self::Transition(id)
            | Self::Placed(id)
            | Self::Properties(id)
            | Self::Effects(id)
            | Self::Masks(id) => id,
        }
    }

    /// That it is not as it was when the script was written.
    fn changed(&self) -> String {
        match self {
            Self::Placed(id) => format!("clip `{id}` was cut or moved since this was proposed"),
            Self::Properties(_) | Self::Effects(_) | Self::Masks(_) => {
                format!("{} have changed since this was proposed", self.what())
            }
            _ => format!("{} has changed since this was proposed", self.what()),
        }
    }
}

/// The state of what a [`Key`] names. A clip's start and a marker's time
/// are left out: a place is carried, not compared.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    Media(Media),
    Sequence {
        name: String,
        rate: Rate,
        drop_frame: bool,
        size: (u32, u32),
        sample_rate: u32,
        color: kerf_model::ColorSpace,
        start: Time,
    },
    Track {
        kind: TrackKind,
        name: Option<String>,
        enabled: bool,
        locked: bool,
        muted: bool,
        role: Option<String>,
        sync_locked: bool,
        bus: Option<Id>,
    },
    Bus(kerf_model::Bus),
    Marker {
        sequence: Id,
        duration: Time,
        name: Option<String>,
        color: Option<kerf_model::MarkerColor>,
        note: Option<String>,
    },
    Transition {
        track: Id,
        transition: Transition,
    },
    Placed(Placed),
    Properties {
        enabled: bool,
        name: Option<String>,
        parent: Option<Id>,
        retime: Retime,
        angle: Option<Id>,
        props: Params,
        /// A generator's own parameters, or a caption's text.
        made: Option<(Id, u32, Params)>,
        caption: Option<String>,
    },
    Effects(Vec<Effect>),
    Masks(Vec<Mask>),
}

type Parts = BTreeMap<Key, Part>;

/// Everything of `project` an operation can change, each id put through
/// `names`.
fn parts(project: &Project, names: &Names) -> Parts {
    let mut parts = Parts::new();
    for media in &project.media {
        parts.insert(Key::Media(media.id.clone()), Part::Media(media.clone()));
    }
    for sequence in &project.sequences {
        parts.insert(
            Key::Sequence(sequence.id.clone()),
            Part::Sequence {
                name: sequence.name.clone(),
                rate: sequence.rate,
                drop_frame: sequence.drop_frame,
                size: (sequence.width, sequence.height),
                sample_rate: sequence.sample_rate,
                color: sequence.color,
                start: sequence.start,
            },
        );
        for bus in &sequence.buses {
            parts.insert(
                Key::Bus(sequence.id.clone(), bus.id.clone()),
                Part::Bus(bus.clone()),
            );
        }
        for marker in &sequence.markers {
            parts.insert(
                Key::Marker(names.now(&marker.id)),
                Part::Marker {
                    sequence: sequence.id.clone(),
                    duration: marker.duration,
                    name: marker.name.clone(),
                    color: marker.color,
                    note: marker.note.clone(),
                },
            );
        }
        for track in &sequence.tracks {
            parts.insert(
                Key::Track(sequence.id.clone(), track.id.clone()),
                Part::Track {
                    kind: track.kind,
                    name: track.name.clone(),
                    enabled: track.enabled,
                    locked: track.locked,
                    muted: track.muted,
                    role: track.role.clone(),
                    sync_locked: track.sync_locked,
                    bus: track.bus.clone(),
                },
            );
            for transition in &track.transitions {
                parts.insert(
                    Key::Transition(names.now(&transition.id)),
                    Part::Transition {
                        track: track.id.clone(),
                        transition: Transition {
                            id: names.now(&transition.id),
                            from: names.now(&transition.from),
                            to: names.now(&transition.to),
                            ..transition.clone()
                        },
                    },
                );
            }
            for clip in &track.clips {
                let id = names.now(&clip.id);
                parts.insert(
                    Key::Placed(id.clone()),
                    Part::Placed(placed(&sequence.id, &track.id, clip)),
                );
                let (made, caption) = match &clip.content {
                    Content::Generator {
                        fx,
                        version,
                        params,
                    } => (Some((fx.clone(), *version, params.clone())), None),
                    Content::Caption { text } => (None, Some(text.clone())),
                    Content::Media { .. } | Content::Sequence { .. } => (None, None),
                };
                parts.insert(
                    Key::Properties(id.clone()),
                    Part::Properties {
                        enabled: clip.enabled,
                        name: clip.name.clone(),
                        parent: clip.parent.as_ref().map(|parent| names.now(parent)),
                        retime: clip.retime,
                        angle: clip.angle.clone(),
                        props: clip.props.clone(),
                        made,
                        caption,
                    },
                );
                parts.insert(
                    Key::Effects(id.clone()),
                    Part::Effects(clip.effects.clone()),
                );
                parts.insert(Key::Masks(id), Part::Masks(clip.masks.clone()));
            }
        }
    }
    parts
}

/// Hold one operation to having changed, on the edited project (`now`),
/// what it changed on the one it was written on (`was`), each given as its
/// parts before and after: the same things, from the same state to the same
/// state. Of several things that differ, the one said is the one that tells
/// most: what was changed since, then what is gone, then what the line would
/// newly change.
fn same_change(was: (&Parts, &Parts), now: (&Parts, &Parts)) -> Result<(), Diagnostic> {
    let keys: BTreeSet<&Key> = [was.0, was.1, now.0, now.1]
        .into_iter()
        .flat_map(BTreeMap::keys)
        .collect();
    let mut refused: Option<(u8, Diagnostic)> = None;
    for key in keys {
        let (was_before, was_after) = (was.0.get(key), was.1.get(key));
        let (now_before, now_after) = (now.0.get(key), now.1.get(key));
        if key.is_look()
            && [was_before, was_after, now_before, now_after]
                .iter()
                .any(Option::is_none)
        {
            continue;
        }
        let what = key.what();
        let (rank, code, message) = match (was_before != was_after, now_before != now_after) {
            (false, false) => continue,
            (true, _) if now_before.is_some() && was_before != now_before => {
                (0, "forward.changed", key.changed())
            }
            (true, false) if was_before.is_some() && now_before.is_none() => (
                1,
                "forward.gone",
                format!("{what} is gone since this was proposed"),
            ),
            (false, true) if now_before.is_some() => (
                2,
                "forward.differs",
                match was_before {
                    None => format!(
                        "this line would now change {what}, which was not there when it was proposed"
                    ),
                    Some(_) => format!(
                        "this line would now change {what}, which it left as it was when it was proposed"
                    ),
                },
            ),
            (false, true) => (
                3,
                "forward.differs",
                format!("this line would now make {what}, which it did not when it was proposed"),
            ),
            (true, true) if was_before != now_before || was_after != now_after => (
                4,
                "forward.differs",
                format!("this line would leave {what} other than it did when it was proposed"),
            ),
            (true, true) => continue,
            (true, false) => (
                5,
                "forward.differs",
                format!("this line changed {what} when it was proposed and would not now"),
            ),
        };
        if refused.as_ref().is_none_or(|(other, _)| rank < *other) {
            refused = Some((rank, Diagnostic::error(code, message).entity(key.id())));
        }
    }
    refused.map_or(Ok(()), |(_, diagnostic)| Err(diagnostic))
}

#[cfg(test)]
mod tests;
