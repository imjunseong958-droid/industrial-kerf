//! Edits to sequences and what they hold besides clips: their tracks, buses, and markers.

use super::{
    fail, BusChange, BusFx, Editor, MarkerChange, Outcome, SequenceChange, TrackChange,
    TrackRoleChange,
};
use crate::diag::Diagnostic;
use crate::id::Id;
use crate::model::{Bus, Content, Marker, MarkerColor, Sequence, Track, TrackKind};
use crate::time::Time;

/// A role as a track keeps it ([`crate::model::track_role`]).
fn track_role(role: Option<&str>) -> Outcome<Option<String>> {
    crate::model::track_role(role.unwrap_or_default())
        .map_err(|message| fail("track.role", message))
}

impl Editor<'_> {
    pub(super) fn add_sequence(&mut self, sequence: &Sequence) -> Outcome {
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

    pub(super) fn set_sequence(&mut self, sequence: &Id, change: &SequenceChange) -> Outcome {
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

    pub(super) fn remove_sequence(&mut self, sequence: &Id) -> Outcome {
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

    pub(super) fn add_bus(&mut self, sequence: &Id, bus: &Bus) -> Outcome {
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

    pub(super) fn set_bus(&mut self, sequence: &Id, bus: &Id, change: &BusChange) -> Outcome {
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

    pub(super) fn remove_bus(&mut self, sequence: &Id, bus: &Id) -> Outcome {
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

    pub(super) fn bus_fx(&mut self, sequence: &Id, bus: &Id, change: &BusFx) -> Outcome {
        let at = self.bus_at(sequence, bus)?;
        let found = &mut self.project.sequences[at.0].buses[at.1];
        let missing =
            |index: &usize| fail("bus.effect", format!("`{bus}` has no effect {}", index + 1));
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

    fn bus_at(&mut self, sequence: &Id, bus: &Id) -> Result<(usize, usize), Diagnostic> {
        let s = self.sequence(sequence)?;
        let index = self.project.sequences[s]
            .buses
            .iter()
            .position(|found| found.id == *bus)
            .ok_or_else(|| fail("bus.unknown", format!("`{sequence}` has no bus `{bus}`")))?;
        Ok((s, index))
    }

    pub(super) fn add_track(
        &mut self,
        sequence: &Id,
        id: &Id,
        kind: TrackKind,
        index: Option<usize>,
    ) -> Outcome {
        let s = self.sequence(sequence)?;
        if self.project.sequences[s].track(id.as_str()).is_some() {
            return Err(fail(
                "track.duplicate",
                format!("track `{id}` already exists"),
            ));
        }
        let tracks = &mut self.project.sequences[s].tracks;
        let index = index.unwrap_or(tracks.len()).min(tracks.len());
        tracks.insert(index, Track::new(id.clone(), kind));
        Ok(())
    }

    pub(super) fn set_track(&mut self, sequence: &Id, track: &Id, change: &TrackChange) -> Outcome {
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

    pub(super) fn remove_track(&mut self, sequence: &Id, track: &Id) -> Outcome {
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

    pub(super) fn set_track_role(
        &mut self,
        sequence: &Id,
        track: &Id,
        change: &TrackRoleChange,
    ) -> Outcome {
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

    pub(super) fn add_marker(
        &mut self,
        sequence: &Id,
        id: Option<&Id>,
        (at, duration): (Time, Time),
        name: &Option<String>,
        color: Option<MarkerColor>,
        note: &Option<String>,
    ) -> Outcome {
        let s = self.sequence(sequence)?;
        self.aligned(s, at, "marker")?;
        self.aligned(s, duration, "marker duration")?;
        let id = match id {
            Some(id) => {
                self.claim(id)?;
                id.clone()
            }
            None => self.ids.next("mk"),
        };
        let markers = &mut self.project.sequences[s].markers;
        let index = markers.partition_point(|m| m.at <= at);
        markers.insert(
            index,
            Marker {
                id,
                at,
                duration,
                name: name.clone(),
                color,
                note: note.clone(),
            },
        );
        Ok(())
    }

    pub(super) fn set_marker(&mut self, id: &Id, change: &MarkerChange) -> Outcome {
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

    pub(super) fn remove_marker(&mut self, id: &Id) -> Outcome {
        let (s, index) = self.marker(id)?;
        self.project.sequences[s].markers.remove(index);
        Ok(())
    }
}
