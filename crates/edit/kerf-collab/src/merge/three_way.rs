//! Three-way merges: of one field, of one kind of element by key, and of
//! the fields of media, sequences, tracks, and clips.

use super::elements::{At, Placed, Written};
use super::Conflict;
use kerf_model::{Clip, Id, Media, Sequence, Time, Track};
use std::collections::{BTreeMap, BTreeSet};

/// One field merged three ways: the side that changed it, or ours when both
/// changed it differently, which names it in `clashes`.
pub(super) fn pick<T: Clone + PartialEq>(
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
pub(super) fn whole<T: Clone + PartialEq>(
    base: &T,
    ours: &T,
    theirs: &T,
    clashes: &mut Vec<String>,
) -> T {
    pick("", base, ours, theirs, clashes)
}

/// "start, duration changed on both sides"; an element merged whole names
/// no field.
pub(super) fn changed_on_both(clashes: &[String]) -> String {
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
pub(super) fn merge_list<K: Ord + Clone, T: Clone + PartialEq>(
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
/// both. The reel and the recipe are labels, and each merges on its own, as do the
/// steps that say how the file is made and the recording it is lined up with.
pub(super) fn merge_media(
    base: &Media,
    ours: &Media,
    theirs: &Media,
    clashes: &mut Vec<String>,
) -> Media {
    let file = |media: &Media| Media {
        reel: None,
        recipe: None,
        steps: Vec::new(),
        sync: None,
        ..media.clone()
    };
    Media {
        sync: pick("sync", &base.sync, &ours.sync, &theirs.sync, clashes),
        reel: pick("reel", &base.reel, &ours.reel, &theirs.reel, clashes),
        recipe: pick(
            "recipe",
            &base.recipe,
            &ours.recipe,
            &theirs.recipe,
            clashes,
        ),
        // How it is made is its steps together: a step changed on each side is one clash.
        steps: pick("steps", &base.steps, &ours.steps, &theirs.steps, clashes),
        ..pick("file", &file(base), &file(ours), &file(theirs), clashes)
    }
}

pub(super) fn merge_sequence(
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

pub(super) fn merge_track(
    base: &Track,
    ours: &Track,
    theirs: &Track,
    clashes: &mut Vec<String>,
) -> Track {
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

/// A clip's fields merged three ways. Where it is merges as its distance from the clip it
/// follows when that is how all three say where it is, and else as its start.
pub(super) fn merge_clip(
    base: &Written,
    ours: &Written,
    theirs: &Written,
    clashes: &mut Vec<String>,
) -> Written {
    let place = |written: &Written| {
        (
            written.placed.sequence.clone(),
            written.placed.track.clone(),
        )
    };
    let (sequence, track) = pick("track", &place(base), &place(ours), &place(theirs), clashes);
    let (b, o, t) = (&base.placed.item, &ours.placed.item, &theirs.placed.item);
    let at_name = match &base.at {
        At::After(followed, _) => format!("distance from {followed}"),
        At::Start(_) => start_name(b.follows.as_ref(), o.follows.as_ref(), t.follows.as_ref()),
    };
    let at = pick(&at_name, &base.at, &ours.at, &theirs.at, clashes);
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
    let follows = pick("follows", &b.follows, &o.follows, &t.follows, clashes);
    let retime = pick("retime", &b.retime, &o.retime, &t.retime, clashes);
    let angle = pick("angle", &b.angle, &o.angle, &t.angle, clashes);
    let effects = pick("effects", &b.effects, &o.effects, &t.effects, clashes);
    Written {
        at,
        placed: Placed {
            sequence,
            track,
            item: Clip {
                id: o.id.clone(),
                start: Time::ZERO,
                duration,
                content,
                enabled,
                name,
                parent,
                follows,
                retime,
                angle,
                link,
                props,
                masks,
                effects,
            },
        },
    }
}

/// The name of a clip's start, merged as a time, for a clash: with what the sides make it
/// follow when they differ, which is why it is not merged as a distance.
fn start_name(base: Option<&Id>, ours: Option<&Id>, theirs: Option<&Id>) -> String {
    let clip = |id: Option<&Id>| id.map_or_else(|| "nothing".to_owned(), Id::to_string);
    let why = match (base, ours, theirs) {
        _ if base == ours && ours == theirs => return "start".to_owned(),
        (Some(was), None, None) => format!("detached from {was} on both sides"),
        (Some(was), None, _) if theirs == base => format!("detached from {was} here"),
        (Some(was), _, None) if ours == base => format!("detached from {was} there"),
        (None, Some(now), _) if ours == theirs => format!("attached to {now} on both sides"),
        (None, Some(now), None) => format!("attached to {now} here"),
        (None, None, Some(now)) => format!("attached to {now} there"),
        _ if ours == theirs => {
            format!("following {} on both sides, not {}", clip(ours), clip(base))
        }
        _ => format!("following {} here, {} there", clip(ours), clip(theirs)),
    };
    format!("start ({why})")
}
