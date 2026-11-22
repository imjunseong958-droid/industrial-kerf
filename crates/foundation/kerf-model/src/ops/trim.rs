//! Edits that move a clip's edges or what it shows of its source: trims, rolls, slips, slides,
//! and changes of speed, which keep the source range and change the duration.

use super::{fail, keep_looks, Edge, Editor, Outcome};
use crate::id::Id;
use crate::model::{Clip, Content};
use crate::speed::Speed;
use crate::time::Time;
use std::collections::HashSet;

impl Editor<'_> {
    pub(super) fn trim(&mut self, clip: &Id, edge: Edge, delta: Time, ripple: bool) -> Outcome {
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
        let (rate, catalog) = (self.project.sequences[s].rate, self.catalog);
        for id in &members {
            let member = self.find(id);
            let found = self.project.clip_at_mut(member);
            match edge {
                Edge::Out => found.move_tail(delta),
                Edge::In => {
                    found.move_head(delta);
                    keep_looks(found, delta, rate, catalog);
                    if ripple {
                        found.start -= delta;
                    }
                }
            }
            self.check_source(member)?;
        }
        Ok(())
    }

    pub(super) fn roll(&mut self, clip: &Id, delta: Time, alone: bool) -> Outcome {
        let at = self.locate(clip)?;
        let s = at.sequence;
        self.aligned(s, delta, "roll amount")?;
        let cut = self.clip(at).end();
        let mut pairs = Vec::new();
        let members = if alone { vec![at] } else { self.linked(at) };
        for member in members {
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
            // `pairs` was read from this track, which nothing has changed since.
            let (a, b) = (
                track
                    .clip(outgoing.as_str())
                    .expect("the outgoing clip was read from it"),
                track
                    .clip(incoming.as_str())
                    .expect("the incoming clip was read from it"),
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
            let (rate, catalog) = (self.project.sequences[b.sequence].rate, self.catalog);
            let found = self.project.clip_at_mut(b);
            found.move_head(delta);
            keep_looks(found, delta, rate, catalog);
            self.check_source(b)?;
        }
        Ok(())
    }

    pub(super) fn slip(&mut self, clip: &Id, delta: Time) -> Outcome {
        let at = self.locate(clip)?;
        self.aligned(at.sequence, delta, "slip")?;
        for member in self.linked(at) {
            self.unlocked(member.sequence, member.track)?;
            self.project.clip_at_mut(member).slip(delta);
            self.check_source(member)?;
        }
        Ok(())
    }

    pub(super) fn slide(&mut self, clip: &Id, delta: Time) -> Outcome {
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
                let (rate, catalog) = (self.project.sequences[n.sequence].rate, self.catalog);
                let found = self.project.clip_at_mut(n);
                match edge {
                    Edge::In => {
                        found.move_head(delta);
                        keep_looks(found, delta, rate, catalog);
                    }
                    Edge::Out => found.move_tail(delta),
                }
                self.check_source(n)?;
            }
            let member = self.find(id);
            self.project.clip_at_mut(member).start += delta;
        }
        Ok(())
    }

    pub(super) fn speed(&mut self, clip: &Id, speed: &Speed) -> Outcome {
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
                let how = match speed.constant() {
                    Some(_) => format!("at speed {speed}"),
                    None => "with that ramp".to_owned(),
                };
                let into = track
                    .clips
                    .iter()
                    .find(|other| {
                        other.id != *id
                            && other.start >= found.start + found.duration
                            && other.start < found.start + duration
                    })
                    .map_or_else(
                        || "the next clip".to_owned(),
                        |other| format!("`{}`", other.id),
                    );
                return Err(fail(
                    "speed.collision",
                    format!(
                        "`{id}` {how} would be {frames} frames long, {} more than it is, and run \
                         into {into} on track {}; make room after it first, or trim it shorter",
                        frames - rate.frames_round(found.duration),
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
}
