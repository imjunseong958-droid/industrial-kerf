//! Repairs to what the merge kept: what one side removed and the other
//! still uses comes back, clips that overlap move to a free track, and
//! transitions follow their clips or go.

use super::collisions::prefix;
use super::elements::{clip_reads, landings, track_reads, Flat, Placed};
use super::Conflict;
use kerf_model::{Catalog, Clip, Content, Id, IdGen, Project, Track, Transition};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Put back what one side removed while the merged project still uses it:
/// a sequence or track the other side put clips on, media its clips play,
/// markers its looks land on, and, whole with what is in them, a sequence
/// its clips nest and a track its effects read. It comes from the side that
/// kept it.
pub(super) fn restore(
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
pub(super) fn separate_overlaps(
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
                                let last = kept.pop().expect("`kept.last()` just matched a clip");
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
pub(super) fn repair_transitions(
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
