//! Edits that place, remove, cut, move, and link clips, and the primitives they and the trims
//! are built from: splitting clips, and opening, closing, and clearing time on tracks.

use super::{fail, keep_looks, Editor, NewClip, Outcome};
use crate::id::Id;
use crate::model::Clip;
use crate::time::Time;
use std::collections::{BTreeMap, HashSet};

impl Editor<'_> {
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
        let (rate, catalog) = (self.project.sequences[s].rate, self.catalog);
        for (t, id) in members {
            let tail_id = match &named {
                Some((clip, tail)) if clip == id => tail.clone(),
                _ => self.ids.next("c"),
            };
            let track = &mut self.project.sequences[s].tracks[*t];
            let c = track
                .clip_index(id.as_str())
                .expect("each member was read from its track, and splitting a clip keeps its id");
            let offset = at - track.clips[c].start;
            let mut tail = track.clips[c].split_off(offset, tail_id.clone());
            keep_looks(&mut tail, offset, rate, catalog);
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
        // Once every one is cut, so that the tail of a follower cut with the clip it follows
        // is there to follow that clip's tail.
        for ((_, head), tail) in members.iter().zip(&tails) {
            self.follow_tail(s, head, tail, at);
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
    /// split nor move, and nor does a clip that follows another, for where it
    /// is itself: it goes as far as the one it follows.
    pub(super) fn open_time(
        &mut self,
        s: usize,
        edited: &[usize],
        at: Time,
        amount: Time,
        exclude: &HashSet<Id>,
    ) -> Outcome {
        let tracks = self.ripple_tracks(s, edited);
        let mut spanning = Vec::new();
        for &t in &tracks {
            for clip in &self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id)
                    && clip.follows.is_none()
                    && clip.start < at
                    && clip.end() > at
                {
                    spanning.push((t, clip.id.clone()));
                }
            }
        }
        self.split_all(s, &spanning, at, None);
        let mut moved = HashSet::new();
        for t in tracks {
            for clip in &mut self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id) && clip.follows.is_none() && clip.start >= at {
                    clip.start += amount;
                    moved.insert(clip.id.clone());
                }
            }
        }
        self.carry(s, moved, amount)
    }

    /// Close `[from, to)` on the ripple tracks of an edit on `edited`.
    /// Fails when a clip outside `exclude` occupies the range, because
    /// closing it would cut that clip. A clip that follows another is not
    /// in the way, and goes as far as the one it follows.
    pub(super) fn close_time(
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
            if let Some(blocker) = track.clips.iter().find(|c| {
                !exclude.contains(&c.id) && c.follows.is_none() && c.start < to && c.end() > from
            }) {
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
        let mut moved = HashSet::new();
        for t in tracks {
            for clip in &mut self.project.sequences[s].tracks[t].clips {
                if !exclude.contains(&clip.id) && clip.follows.is_none() && clip.start >= to {
                    clip.start -= to - from;
                    moved.insert(clip.id.clone());
                }
            }
        }
        self.carry(s, moved, from - to)
    }

    /// Remove `[from, to)` from `tracks`, trimming or splitting clips at
    /// the edges. A clip the range falls inside is cut in two, and the far
    /// parts of clips that were linked together are linked together, as a
    /// blade's are: so they are cut on every track at once, and not a
    /// track at a time, which would leave each far part with nothing to be
    /// linked to.
    fn clear_tracks(&mut self, s: usize, tracks: &[usize], from: Time, to: Time) {
        let mut inside = Vec::new();
        for &t in tracks {
            for clip in &self.project.sequences[s].tracks[t].clips {
                if clip.start < from && clip.end() > to {
                    inside.push((t, clip.id.clone()));
                }
            }
        }
        self.split_all(s, &inside, to, None);
        let (rate, catalog) = (self.project.sequences[s].rate, self.catalog);
        for &t in tracks {
            let overlapping: Vec<Id> = self.project.sequences[s].tracks[t]
                .clips
                .iter()
                .filter(|c| c.start < to && c.end() > from)
                .map(|c| c.id.clone())
                .collect();
            for id in overlapping {
                let track = &mut self.project.sequences[s].tracks[t];
                let c = track
                    .clip_index(id.as_str())
                    .expect("each was read from this track, and only its own turn removes it");
                let clip = &mut track.clips[c];
                if clip.start >= from && clip.end() <= to {
                    track.clips.remove(c);
                } else if clip.start < from {
                    let end = clip.end();
                    clip.move_tail(from - end);
                } else {
                    let start = clip.start;
                    clip.move_head(to - start);
                    keep_looks(clip, to - start, rate, catalog);
                }
            }
        }
    }

    // ---- operations -------------------------------------------------------

    pub(super) fn place(
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
        let edited: Vec<usize> = placed.iter().map(|(t, _)| *t).collect();
        if ripple {
            self.open_time(s, &edited, at, duration, &HashSet::new())?;
        } else {
            self.clear_tracks(s, &edited, at, at + duration);
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

    pub(super) fn delete(&mut self, clips: &[Id]) -> Outcome {
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

    pub(super) fn ripple_delete(&mut self, clips: &[Id]) -> Outcome {
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
        // What follows another is taken out and closes nothing: a ripple edit passes under it.
        let rippling = || {
            selection
                .iter()
                .filter(|at| self.clip(**at).follows.is_none())
        };
        let mut ranges: Vec<(Time, Time)> = rippling()
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
        // The clips not yet removed. The ranges go latest first, so closing one leaves the
        // earlier ones where they were.
        let mut ids: HashSet<Id> = selection
            .iter()
            .map(|at| self.clip(*at).id.clone())
            .collect();
        // What follows one of them and is wholly over it goes with it: a title over a shot
        // that is cut out has nothing left to be over. What runs on past it stays, as music
        // under the next shots does.
        let riders: Vec<(Id, Id)> = self.project.sequences[s]
            .tracks
            .iter()
            .filter(|track| !track.locked)
            .flat_map(|track| &track.clips)
            .filter(|clip| !ids.contains(&clip.id))
            .filter_map(|clip| {
                let followed = rippling()
                    .map(|at| self.clip(*at))
                    .find(|followed| clip.follows.as_ref() == Some(&followed.id))?;
                (clip.start >= followed.start && clip.end() <= followed.end())
                    .then(|| (clip.id.clone(), followed.id.clone()))
            })
            .collect();
        let edited: Vec<usize> = rippling().map(|at| at.track).collect();
        for (rider, followed) in riders {
            self.notes.push(format!(
                "took `{rider}` out with `{followed}`, which it followed and was wholly over"
            ));
            ids.insert(rider);
        }
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
            ids.retain(|id| !here.contains(id));
        }
        for id in ids {
            let at = self.find(&id);
            self.project.sequences[s].tracks[at.track]
                .clips
                .remove(at.clip);
        }
        Ok(())
    }

    pub(super) fn clear(
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
        // The clips the end of the range is inside. Of those, the ones whose head is in the
        // range too: what each shows from the range's end on comes earlier when it closes,
        // and what it showed before that is gone.
        let across = |clip: &&Clip| clip.start < to && clip.end() > to;
        let clips = || {
            tracks
                .iter()
                .flat_map(|t| &self.project.sequences[s].tracks[*t].clips)
        };
        let cut_through: HashSet<Id> = clips().filter(across).map(|c| c.id.clone()).collect();
        let headless: HashSet<Id> = clips()
            .filter(across)
            .filter(|clip| clip.start >= from)
            .map(|clip| clip.id.clone())
            .collect();
        let was_there: HashSet<Id> = clips().map(|clip| clip.id.clone()).collect();
        self.clear_tracks(s, &tracks, from, to);
        if close {
            // The range is cut out of every clip in it, followers too, and the part of each
            // after it closes up to the part before. What follows another and was whole
            // after the range goes as far as the one it follows.
            let (mut whole, mut cut) = (HashSet::new(), HashSet::new());
            for t in tracks {
                for clip in &mut self.project.sequences[s].tracks[t].clips {
                    let part_after = clip.start == to
                        && (cut_through.contains(&clip.id) || !was_there.contains(&clip.id));
                    if clip.start >= to && (clip.follows.is_none() || part_after) {
                        clip.start -= to - from;
                        if headless.contains(&clip.id) {
                            cut.insert(clip.id.clone());
                        } else {
                            whole.insert(clip.id.clone());
                        }
                    }
                }
            }
            let moved: HashSet<Id> = whole.union(&cut).cloned().collect();
            self.carry_past(s, whole, moved.clone(), from - to, None)?;
            // A follower that stood over a head that went stays where it is.
            self.carry_past(s, cut, moved, from - to, Some(to))?;
        }
        Ok(())
    }

    pub(super) fn split(&mut self, clip: &Id, at: Time, tail: Option<&Id>) -> Outcome {
        let found = self.locate(clip)?;
        let s = found.sequence;
        self.aligned(s, at, "split point")?;
        let target = self.clip(found);
        if at <= target.start || at >= target.end() {
            return Err(fail(
                "split.outside",
                format!(
                    "{} is not inside clip `{clip}` ({}–{})",
                    self.tc(s, at),
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
            if candidate.start < at && candidate.end() > at {
                self.unlocked(s, member.track)?;
                members.push((member.track, candidate.id.clone()));
            }
        }
        let named = tail.map(|tail| (clip.clone(), tail.clone()));
        self.split_all(s, &members, at, named);
        Ok(())
    }

    pub(super) fn blade(&mut self, sequence: &Id, at: Time, tracks: Option<&[Id]>) -> Outcome {
        let s = self.sequence(sequence)?;
        self.aligned(s, at, "blade point")?;
        let mut members = Vec::new();
        for t in self.track_set(s, tracks)? {
            for clip in &self.project.sequences[s].tracks[t].clips {
                if clip.start < at && clip.end() > at {
                    members.push((t, clip.id.clone()));
                }
            }
        }
        self.split_all(s, &members, at, None);
        Ok(())
    }

    pub(super) fn move_clip(&mut self, clip: &Id, track: Option<&Id>, delta: Time) -> Outcome {
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
        let mut moved = HashSet::new();
        for (target, clip) in lifted {
            moved.insert(clip.id.clone());
            self.project.sequences[s].tracks[target].insert_sorted(clip);
        }
        self.carry(s, moved, delta)
    }

    pub(super) fn link(&mut self, clips: &[Id], group: Option<&Id>) -> Outcome {
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

    pub(super) fn unlink(&mut self, clips: &[Id]) -> Outcome {
        for id in clips {
            let at = self.locate(id)?;
            self.project.clip_at_mut(at).link = None;
        }
        Ok(())
    }
}
