//! Clips that follow another in time ([`Clip::follows`]).
//!
//! A clip that follows another is where it was put until an edit moves the one it follows:
//! then it moves as far. So what moves a clip without changing what it shows when (a move, a
//! slide, and the time a ripple edit opens or closes before it) takes its followers along,
//! and what changes the clip where it stands (a trim, a roll, a slip, a change of speed)
//! leaves them where they are.
//!
//! A ripple edit passes under a follower: for where the follower is itself, the edit neither
//! cuts it, nor moves it, nor is stopped by it. So a title stays on its shot and music under
//! its scene while the shots before them come and go, and a follower's own trim or removal
//! ripples nothing.
//!
//! When the clip a follower follows is cut in two, the follower follows the part its own
//! start is over. When that clip is removed, the follower stays where it is and follows the
//! clip that is in its place on its track, or failing one the next there, or the last; but a
//! ripple delete, which takes a clip and its time out of the program, takes with it the
//! followers that were wholly over it.

use super::{fail, Editor, Outcome};
use crate::id::Id;
use crate::time::Time;
use std::collections::{HashMap, HashSet};

/// Where each clip that is followed stood before an operation: its track and its start.
pub(super) type Followed = HashMap<Id, (Id, Time)>;

impl Editor<'_> {
    /// Where every clip that another follows stands now, in every sequence.
    pub(super) fn followed(&self) -> Followed {
        let mut places = Followed::new();
        for sequence in &self.project.sequences {
            let followed: HashSet<&Id> = sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .filter_map(|clip| clip.follows.as_ref())
                .collect();
            for track in &sequence.tracks {
                for clip in &track.clips {
                    if followed.contains(&clip.id) {
                        places.insert(clip.id.clone(), (track.id.clone(), clip.start));
                    }
                }
            }
        }
        places
    }

    /// Move what follows the clips of `moved` in sequence `s` by `delta`, and what follows
    /// those, each once: none that `moved` already holds, which has moved, and none on a
    /// locked track, which nothing moves. Every track is left in start order. Refused where
    /// one would start before the sequence does.
    pub(super) fn carry(&mut self, s: usize, moved: HashSet<Id>, delta: Time) -> Outcome {
        self.carry_from(s, moved, delta, None)
    }

    /// [`Self::carry`], of what follows the clips of `moved` only those that start at or
    /// after `from`: when what a clip shows moved from there on and what it showed before
    /// is gone, as when time is taken off its head and closed.
    pub(super) fn carry_from(
        &mut self,
        s: usize,
        moved: HashSet<Id>,
        delta: Time,
        from: Option<Time>,
    ) -> Outcome {
        self.carry_past(s, moved.clone(), moved, delta, from)
    }

    /// [`Self::carry_from`], of what follows the clips of `ahead`, where `done` holds every
    /// clip that has moved already, those of `ahead` among them: none of `done` is moved.
    pub(super) fn carry_past(
        &mut self,
        s: usize,
        ahead: HashSet<Id>,
        mut done: HashSet<Id>,
        delta: Time,
        from: Option<Time>,
    ) -> Outcome {
        if delta != Time::ZERO && !ahead.is_empty() {
            // Of what has moved, the ones whose followers have yet to.
            let (mut ahead, mut from) = (ahead, from);
            loop {
                let mut carried = HashSet::new();
                for track in &mut self.project.sequences[s].tracks {
                    if track.locked {
                        continue;
                    }
                    for clip in &mut track.clips {
                        let Some(followed) = clip.follows.as_ref() else {
                            continue;
                        };
                        if ahead.contains(followed)
                            && !done.contains(&clip.id)
                            && from.is_none_or(|from| clip.start >= from)
                        {
                            clip.start += delta;
                            if clip.start.is_negative() {
                                return Err(fail(
                                    "follow.too_far",
                                    format!(
                                        "`{}`, which follows `{followed}`, would start before the sequence; move or detach it first",
                                        clip.id
                                    ),
                                ));
                            }
                            carried.insert(clip.id.clone());
                        }
                    }
                }
                if carried.is_empty() {
                    break;
                }
                done.extend(carried.iter().cloned());
                // What follows a follower goes wherever it does.
                (ahead, from) = (carried, None);
            }
        }
        self.in_start_order(s);
        Ok(())
    }

    /// Put the clips of each track of sequence `s` in start order again, after some of them
    /// moved and others did not.
    pub(super) fn in_start_order(&mut self, s: usize) {
        for track in &mut self.project.sequences[s].tracks {
            track.clips.sort_by_key(|clip| clip.start);
        }
    }

    /// After the clip `head` of sequence `s` was cut at `at` and `tail` is the part after:
    /// what followed it and starts at or after the cut follows the tail.
    pub(super) fn follow_tail(&mut self, s: usize, head: &Id, tail: &Id, at: Time) {
        for clip in self.project.sequences[s]
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
        {
            if clip.follows.as_ref() == Some(head) && clip.start >= at && clip.id != *tail {
                clip.follows = Some(tail.clone());
            }
        }
    }

    /// Have each of `clips`, and the clips linked to it, follow `to`.
    pub(super) fn attach(&mut self, clips: &[Id], to: &Id) -> Outcome {
        let lead = self.locate(to)?;
        let selection = self.selection(clips)?;
        for at in &selection {
            let id = &self.clip(*at).id;
            if at.sequence != lead.sequence {
                return Err(fail(
                    "clip.follows",
                    format!("`{id}` cannot follow `{to}`, a clip of another sequence"),
                ));
            }
            if *at == lead {
                return Err(fail(
                    "clip.follows",
                    if clips.contains(to) {
                        format!("`{to}` cannot follow itself")
                    } else {
                        format!(
                            "`{to}` is linked to what would follow it, and moves with it already"
                        )
                    },
                ));
            }
            self.unlocked(at.sequence, at.track)?;
        }
        // None of them is followed by `to`, through however many.
        let attached: HashSet<Id> = selection
            .iter()
            .map(|at| self.clip(*at).id.clone())
            .collect();
        let clips_there: usize = self.project.sequences[lead.sequence]
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum();
        let mut ahead = Some(to.clone());
        let mut steps = 0;
        while let Some(id) = ahead {
            if attached.contains(&id) {
                return Err(fail(
                    "clip.follows",
                    format!("`{id}` cannot follow `{to}`, which follows it"),
                ));
            }
            steps += 1;
            ahead = self
                .project
                .locate(id.as_str())
                .filter(|_| steps <= clips_there)
                .and_then(|at| self.clip(at).follows.clone());
        }
        for at in selection {
            self.project.clip_at_mut(at).follows = Some(to.clone());
        }
        Ok(())
    }

    /// Have each of `clips`, and the clips linked to it, follow nothing.
    pub(super) fn detach(&mut self, clips: &[Id]) -> Outcome {
        let selection = self.selection(clips)?;
        for at in &selection {
            self.unlocked(at.sequence, at.track)?;
        }
        for at in selection {
            self.project.clip_at_mut(at).follows = None;
        }
        Ok(())
    }

    /// Give each clip whose followed clip is gone another to follow: the clip of the track
    /// the gone one was on that is where it started, or failing one the next after, or the
    /// last before, so long as that one does not follow the follower itself. With none, it
    /// follows nothing. `before` is where the followed clips stood before the operation.
    pub(super) fn refollow(&mut self, before: &Followed) {
        for s in 0..self.project.sequences.len() {
            let sequence = &self.project.sequences[s];
            let mut follows: HashMap<Id, Option<Id>> = sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .map(|clip| (clip.id.clone(), clip.follows.clone()))
                .collect();
            let mut orphans: Vec<(Id, Id)> = follows
                .iter()
                .filter_map(|(clip, followed)| {
                    let followed = followed.as_ref()?;
                    (!follows.contains_key(followed)).then(|| (clip.clone(), followed.clone()))
                })
                .collect();
            orphans.sort();
            let mut changes: Vec<(Id, Id, Option<Id>)> = Vec::new();
            for (clip, gone) in orphans {
                let stand_in = before.get(&gone).and_then(|(track, start)| {
                    let track = sequence.tracks.iter().find(|t| t.id == *track)?;
                    let others = || track.clips.iter().filter(|other| other.id != clip);
                    others()
                        .find(|other| other.end() > *start)
                        .or_else(|| others().next_back())
                        .map(|other| other.id.clone())
                });
                // Not one that follows this clip, which would have each follow the other: by
                // what each follows as this goes, so that two that lost the same clip do not
                // take each other.
                let stand_in = stand_in.filter(|candidate| {
                    let mut ahead = Some(candidate.clone());
                    let mut steps = 0;
                    while let Some(id) = ahead {
                        if id == clip || steps > follows.len() {
                            return false;
                        }
                        steps += 1;
                        ahead = follows.get(&id).cloned().flatten();
                    }
                    true
                });
                follows.insert(clip.clone(), stand_in.clone());
                changes.push((clip, gone, stand_in));
            }
            for (clip, gone, stand_in) in changes {
                self.notes.push(match &stand_in {
                    Some(other) => format!("`{clip}` follows `{other}` now: `{gone}` is gone"),
                    None => format!("`{clip}` follows nothing now: `{gone}` is gone"),
                });
                let at = self.find(&clip);
                self.project.clip_at_mut(at).follows = stand_in;
            }
        }
    }
}
