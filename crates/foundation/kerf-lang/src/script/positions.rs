//! Positions on a sequence as a script reads them: timecode, frames, and
//! seconds, or an anchor with an optional amount, such as `end`, a clip's
//! start or end, a marker, or where a clip plays a position of its media.

use crate::text::{Clock, Unread};
use kerf_model::{Clip, Content, MissingKind, Project, Sequence, Time};

/// Positions on one sequence. Besides timecode, frames, and seconds, a
/// position may be `end`, the end of the sequence; `<clip>.start` or
/// `<clip>.end` of a clip on it; a marker's id, or `<marker>.end` where a
/// marker with a duration ends; or `<clip>@<source position>`, where that
/// clip plays a position of its media. Each takes an optional amount:
/// `c3.end-6f`, `beat1+2f`, `c3@14:24:05:02+1f`.
pub(super) struct Positions<'a> {
    pub(super) clock: Clock,
    project: &'a Project,
    sequence: &'a Sequence,
}

impl<'a> Positions<'a> {
    pub(super) fn of(project: &'a Project, sequence: usize) -> Self {
        let sequence = &project.sequences[sequence];
        Self {
            clock: Clock::of_sequence(sequence),
            project,
            sequence,
        }
    }

    pub(super) fn position(&self, text: &str) -> Result<Time, Unread> {
        let (base, rest) = if let Some((clip, source)) = text.split_once('@') {
            // Source positions carry no sign, so one starts the amount.
            let split = source
                .char_indices()
                .skip(1)
                .find(|(_, c)| matches!(c, '+' | '-'))
                .map_or(source.len(), |(at, _)| at);
            (self.played(clip, &source[..split])?, &source[split..])
        } else if let Some(found) = self.marker(text) {
            found
        } else if let Some((id, end, rest)) = [(".start", false), (".end", true)]
            .into_iter()
            .find_map(|(suffix, end)| {
                let at = text.find(suffix)?;
                Some((&text[..at], end, &text[at + suffix.len()..]))
            })
        {
            (self.edge(id, end)?, rest)
        } else if let Some(rest) = text
            .strip_prefix("end")
            .filter(|rest| rest.is_empty() || rest.starts_with(['+', '-']))
        {
            (self.sequence.duration(), rest)
        } else {
            return Ok(self.clock.position(text)?);
        };
        match rest {
            "" => Ok(base),
            amount if amount.starts_with(['+', '-']) => Ok(base + self.clock.delta(amount)?),
            _ => Err(
                format!("`{text}`: after an anchor comes an amount such as +12f or -0.5s").into(),
            ),
        }
    }

    /// A marker of this sequence named by `text`, or by `text` up to an
    /// amount, with the rest. Ids may hold `-`, so the longest id that
    /// leaves an amount wins, and `end` is always the end of the sequence.
    fn marker<'t>(&self, text: &'t str) -> Option<(Time, &'t str)> {
        let splits = text
            .char_indices()
            .filter(|(_, c)| matches!(c, '+' | '-'))
            .map(|(at, _)| at);
        std::iter::once(text.len())
            .chain(splits.collect::<Vec<_>>().into_iter().rev())
            .map(|at| text.split_at(at))
            .filter(|(id, _)| *id != "end")
            .find_map(|(id, rest)| {
                let marker = self.sequence.marker(id)?;
                (rest.is_empty() || self.clock.delta(rest).is_ok()).then_some((marker.at, rest))
            })
    }

    /// The start or end of a clip or marker of this sequence.
    fn edge(&self, id: &str, end: bool) -> Result<Time, Unread> {
        if let Some(clip) = self.sequence.tracks.iter().find_map(|track| track.clip(id)) {
            return Ok(if end { clip.end() } else { clip.start });
        }
        let marker = self.sequence.marker(id).ok_or_else(|| {
            Unread::missing(
                format!(
                    "no clip or marker `{id}` in sequence `{}`",
                    self.sequence.id
                ),
                MissingKind::ClipOrMarker,
                id,
            )
        })?;
        Ok(if end {
            marker.at + marker.duration
        } else {
            marker.at
        })
    }

    /// Where the clip `id` of this sequence plays `source`, a position on
    /// its media or nested sequence, on the nearest frame.
    fn played(&self, id: &str, source: &str) -> Result<Time, Unread> {
        let clip = self
            .sequence
            .tracks
            .iter()
            .find_map(|track| track.clip(id))
            .ok_or_else(|| {
                Unread::missing(
                    format!("no clip `{id}` in sequence `{}`", self.sequence.id),
                    MissingKind::Clip,
                    id,
                )
            })?;
        let (clock, (low, high)) = source_clock(self.project, clip)?
            .zip(clip.source_range())
            .ok_or_else(|| format!("clip `{id}` plays no media; use {id}.start or {id}.end"))?;
        let at = clock.position(source)?;
        if at < low || at > high {
            return Err(format!(
                "clip `{id}` plays {} to {}, not `{source}`",
                clock.print_position(low),
                clock.print_position(high),
            )
            .into());
        }
        let played = clip
            .played_at(at)
            .expect("source_range was Some above, and played_at is Some exactly when it is");
        Ok(self.clock.rate.round(played))
    }
}

/// Where `text` is on `sequence` of `project`, read as a script reads a
/// position: timecode, frames or seconds, or an anchor with an optional
/// amount, such as `end`, `c3.end-1f`, `beat1+2f` or `c3@14:24:05:02`; or what is said of
/// why not.
pub fn sequence_position(
    project: &Project,
    sequence: &Sequence,
    text: &str,
) -> Result<Time, String> {
    Positions {
        clock: Clock::of_sequence(sequence),
        project,
        sequence,
    }
    .position(text)
    .map_err(|unread| unread.message)
}

/// The clock of what `clip` plays: its media's or its nested sequence's;
/// `None` for generators and captions.
pub(super) fn source_clock(project: &Project, clip: &Clip) -> Result<Option<Clock>, String> {
    match &clip.content {
        Content::Media { media, .. } => project
            .media(media.as_str())
            .map(|media| Some(Clock::of_media(media)))
            .ok_or_else(|| format!("no media `{media}`")),
        Content::Sequence { sequence, .. } => project
            .sequence(sequence.as_str())
            .map(|nested| Some(Clock::of_sequence(nested)))
            .ok_or_else(|| format!("no sequence `{sequence}`")),
        Content::Generator { .. } | Content::Caption { .. } => Ok(None),
    }
}

/// Everything else a sequence clock reads and prints.
impl std::ops::Deref for Positions<'_> {
    type Target = Clock;

    fn deref(&self) -> &Clock {
        &self.clock
    }
}
