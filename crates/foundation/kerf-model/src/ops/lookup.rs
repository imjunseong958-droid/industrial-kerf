//! What an operation names, found in the project, and the checks it must pass before it changes
//! anything: that ids are free, times are on the frame grid, clips keep to their source, and
//! effects and clips suit where they go.

use super::{fail, Editor, Outcome};
use crate::catalog::{Domain, FxKind};
use crate::id::Id;
use crate::model::{Clip, ClipAt, Content, Effect, Params, Sequence};
use crate::time::{format_timecode, Time};
use crate::validate::accepts;
use std::collections::HashSet;

impl Editor<'_> {
    // ---- lookup -----------------------------------------------------------

    pub(super) fn sequence(&self, id: &Id) -> Outcome<usize> {
        self.project
            .sequence_index(id.as_str())
            .ok_or_else(|| fail("ref.missing", format!("no sequence `{id}`")))
    }

    pub(super) fn track(&self, s: usize, id: &Id) -> Outcome<usize> {
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

    pub(super) fn locate(&self, clip: &Id) -> Outcome<ClipAt> {
        self.project
            .locate(clip.as_str())
            .ok_or_else(|| fail("ref.missing", self.project.no_clip(clip.as_str())))
    }

    /// Locate a clip known to exist: one the operation read from the project and has not removed.
    pub(super) fn find(&self, clip: &Id) -> ClipAt {
        self.project
            .locate(clip.as_str())
            .expect("an operation finds only clips it read from the project and has not removed")
    }

    pub(super) fn clip(&self, at: ClipAt) -> &Clip {
        self.project.clip_at(at)
    }

    pub(super) fn editable(&self, clip: &Id) -> Outcome<ClipAt> {
        let at = self.locate(clip)?;
        self.unlocked(at.sequence, at.track)?;
        Ok(at)
    }

    pub(super) fn unlocked(&self, s: usize, t: usize) -> Outcome {
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
    pub(super) fn ripple_tracks(&self, s: usize, edited: &[usize]) -> Vec<usize> {
        self.unlocked_tracks(s)
            .into_iter()
            .filter(|t| edited.contains(t) || self.project.sequences[s].tracks[*t].sync_locked)
            .collect()
    }

    /// The named tracks, each unlocked, or every unlocked track.
    pub(super) fn track_set(&self, s: usize, tracks: Option<&[Id]>) -> Outcome<Vec<usize>> {
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

    pub(super) fn domain(&self, at: ClipAt) -> Outcome<Domain> {
        self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain()
            .ok_or_else(|| fail("clip.domain", "captions carry no properties or effects"))
    }

    /// The index of the clip's mask `name`.
    pub(super) fn mask(&self, at: ClipAt, name: &Id) -> Outcome<usize> {
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

    pub(super) fn effect(&self, at: ClipAt, index: usize) -> Outcome<&Effect> {
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

    pub(super) fn transition(&self, id: &Id) -> Outcome<(usize, usize, usize)> {
        for (s, sequence) in self.project.sequences.iter().enumerate() {
            for (t, track) in sequence.tracks.iter().enumerate() {
                if let Some(index) = track.transitions.iter().position(|tr| tr.id == *id) {
                    return Ok((s, t, index));
                }
            }
        }
        Err(fail("ref.missing", format!("no transition `{id}`")))
    }

    pub(super) fn marker(&self, id: &Id) -> Outcome<(usize, usize)> {
        for (s, sequence) in self.project.sequences.iter().enumerate() {
            if let Some(index) = sequence.markers.iter().position(|m| m.id == *id) {
                return Ok((s, index));
            }
        }
        Err(fail("ref.missing", format!("no marker `{id}`")))
    }

    /// The clip and every clip linked to it, in track order.
    pub(super) fn linked(&self, at: ClipAt) -> Vec<ClipAt> {
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
    pub(super) fn selection(&self, clips: &[Id]) -> Outcome<Vec<ClipAt>> {
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

    pub(super) fn selection_ids(&self, clips: &[Id]) -> Outcome<Vec<Id>> {
        Ok(self
            .selection(clips)?
            .into_iter()
            .map(|at| self.clip(at).id.clone())
            .collect())
    }

    // ---- checks -----------------------------------------------------------

    pub(super) fn claim(&mut self, id: &Id) -> Outcome {
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

    pub(super) fn tc(&self, s: usize, time: Time) -> String {
        let sequence = &self.project.sequences[s];
        format_timecode(sequence.start + time, sequence.rate, sequence.drop_frame)
    }

    pub(super) fn aligned(&self, s: usize, time: Time, what: &str) -> Outcome {
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

    pub(super) fn check_source(&self, at: ClipAt) -> Outcome {
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

    pub(super) fn check_fx(
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

    pub(super) fn check_placeable(&self, s: usize, t: usize, content: &Content) -> Outcome {
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
}
