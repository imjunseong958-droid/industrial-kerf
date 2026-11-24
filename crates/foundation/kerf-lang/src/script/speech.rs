//! What lines read of what transcripts heard: a quoted phrase given for
//! `src=` or `out=`, the words or the pause after them that `extract <clip>
//! said=` and `pause=` take out, and the caption lines of `captions`.

use super::snap::{frame_within, media_time, nothing_near};
use super::{Parsed, Script};
use crate::captions::{caption_script, is_a_sound_note, Word, DEFAULT_CAPTION_WORDS};
use crate::facts::{airs, Place, Snap, Speech};
use crate::lex::{lex, Arg, Atom, Line};
use crate::sound::Heard;
use crate::text::{quote, Clock, Fields};
use kerf_model::{Clip, Content, Diagnostic, Media, Rate, Time};
use std::rc::Rc;

impl Script<'_> {
    /// `line` with each quoted phrase in its `src=` or `out=` replaced by
    /// when the phrase was said, from transcripts of the media: an in point
    /// on the frame its first word starts, an out point past the frame its
    /// last word ends. `None` when the line has no phrase.
    pub(super) fn spoken(&self, line: &Line) -> Parsed<Option<Line>> {
        let is_phrase = |arg: &Arg| {
            matches!(arg.value, Atom::Str(_)) && matches!(arg.key.as_deref(), Some("src" | "out"))
        };
        if !line.args.iter().any(is_phrase) {
            return Ok(None);
        }
        // Positional arguments count as fields counts them, between keyed ones.
        let positional: Vec<&Arg> = line.args.iter().filter(|arg| arg.key.is_none()).collect();
        let word = |index: usize| match positional.get(index) {
            Some(Arg {
                value: Atom::Word(word),
                ..
            }) => Some(word.as_str()),
            _ => None,
        };
        let keyed = |key: &str| {
            line.args.iter().find_map(|arg| match &arg.value {
                Atom::Word(word) if arg.key.as_deref() == Some(key) => Some(word.as_str()),
                _ => None,
            })
        };
        let (media, trims_out) = match word(0) {
            Some("insert" | "overwrite") => {
                (keyed("media").and_then(|id| self.project.media(id)), false)
            }
            Some("trim") => (
                word(1)
                    .and_then(|id| self.project.locate(id))
                    .and_then(|at| self.media_of(self.project.clip_at(at))),
                word(2) == Some("out"),
            ),
            // Other commands read their fields as they always do.
            _ => return Ok(None),
        };
        // Without media, the line reads a quoted time as it always has, and
        // says so when it is not one.
        let Some(media) = media else {
            return Ok(None);
        };
        let clock = Clock::of_media(media);
        let speech = self.facts.speech(media);
        let mut spoken = line.clone();
        for arg in spoken.args.iter_mut().filter(|arg| is_phrase(arg)) {
            let Atom::Str(phrase) = &arg.value else {
                continue;
            };
            // A quoted time is still a time.
            if clock.position(phrase).is_ok() {
                continue;
            }
            let (from, to) = match speech.said(phrase)[..] {
                [once] => once,
                [] => {
                    return Err(line.error(
                        arg.column,
                        format!(
                            "no transcript of `{}` heard \"{phrase}\"; look at it with transcript first",
                            media.id
                        ),
                    ))
                }
                ref many => {
                    let times: Vec<String> = many
                        .iter()
                        .map(|(from, _)| clock.print_position(source_time(*from, clock.rate, false)))
                        .collect();
                    return Err(line.error(
                        arg.column,
                        format!(
                            "\"{phrase}\" is said {} times in `{}`, at {}; give more words or the time",
                            many.len(),
                            media.id,
                            times.join(", ")
                        ),
                    ));
                }
            };
            let out = arg.key.as_deref() == Some("out") || trims_out;
            let time = source_time(if out { to } else { from }, clock.rate, out);
            arg.value = Atom::Word(clock.print_position(time));
        }
        Ok(Some(spoken))
    }

    /// The range of its sequence that takes what `clip` says out of it, as
    /// `cut` asks: the words of a phrase, or the pause after them, with
    /// each edge in the quiet beside what stays, so that the pause the cut
    /// leaves is `cut.leave` long where the quiet there allows it. What is
    /// taken is kept for [`Applied::snapped`](super::Applied::snapped).
    pub(super) fn said(
        &self,
        clip: &Clip,
        clock: &Clock,
        cut: &Said<'_>,
    ) -> Result<(Time, Time), String> {
        let media = self
            .media_of(clip)
            .ok_or_else(|| format!("clip `{}` plays no media to hear", clip.id))?;
        if clip.speed().is_reverse() {
            return Err(format!(
                "clip `{}` plays in reverse; use from= and to=",
                clip.id
            ));
        }
        let (source_in, source_out) = clip
            .source_range()
            .expect("media_of found media for it, so it plays a source");
        let (source_in, source_out) = (source_in.seconds(), source_out.seconds());
        let speech = self.heard(media);
        let media_clock = Clock::of_media(media);
        let phrase = cut.phrase;
        // Where the clip says it, give or take what a transcript is out by.
        let everywhere = speech.said(phrase);
        let here: Vec<(f64, f64)> = everywhere
            .iter()
            .copied()
            .filter(|(start, end)| *start >= source_in - 0.05 && *end <= source_out + 0.05)
            .collect();
        let (start, end) = match (&here[..], &everywhere[..]) {
            ([once], _) => *once,
            ([], []) => {
                return Err(format!(
                    "no transcript of `{}` heard \"{phrase}\"; look at it with transcript first",
                    media.id
                ))
            }
            ([], elsewhere) => {
                let times: Vec<String> = elsewhere
                    .iter()
                    .map(|(at, _)| {
                        media_clock.print_position(source_time(*at, media_clock.rate, false))
                    })
                    .collect();
                return Err(format!(
                    "`{}` does not play \"{phrase}\", which `{}` has at {}",
                    clip.id,
                    media.id,
                    times.join(", ")
                ));
            }
            (many, _) => {
                let times: Vec<String> = many
                    .iter()
                    .filter_map(|(at, _)| clip.played_at(media_time(*at)))
                    .map(|at| clock.print_position(clock.rate.round(at)))
                    .collect();
                return Err(format!(
                    "\"{phrase}\" is said {} times in `{}`, at {}; give more words, or from= and to=",
                    many.len(),
                    clip.id,
                    times.join(", ")
                ));
            }
        };
        let mut words = speech.heard();
        words.retain(|word| !is_a_sound_note(&word.text));
        // The quiet before what goes, and the quiet after it.
        let (before, after) = if cut.pause {
            let before = self
                .quiet_beside(clip, media, end, cut.within)
                .map_err(|why| format!("no pause after \"{phrase}\": {why}"))?;
            // The pause runs to the next word the clip plays, or its end.
            let next = words
                .iter()
                .find(|word| (word.start + word.end) / 2.0 > end && word.start < source_out);
            let after = match next {
                Some(word) => Some(
                    self.quiet_beside(clip, media, word.start, cut.within)
                        .map_err(|why| {
                            format!("no pause before {}: {why}", quote(word.text.trim()))
                        })?,
                ),
                None => None,
            };
            // What sounds in the wait and has no word: a breath goes with
            // the pause, and what may be a word nobody transcribed does not.
            let waits_to = after.map_or(source_out, |after| after.quiet.0);
            let sounds = self
                .facts
                .sound(media)
                .filter(|_| waits_to > before.quiet.1)
                .and_then(|sound| sound.heard(before.quiet.1, waits_to));
            if sounds == Some(Heard::Sound) {
                let position = |seconds: f64| {
                    media_clock.print_position(media.rate.round(media_time(seconds)))
                };
                return Err(format!(
                    "the wait after \"{phrase}\" holds sound at the level of speech, from {} to {} \
                     of `{}`, which no word accounts for: listen to it, and take it out by from= \
                     and to= if it is to go",
                    position(before.quiet.1),
                    position(waits_to),
                    media.id
                ));
            }
            (Some(before), after)
        } else {
            (
                Some(
                    self.quiet_beside(clip, media, start, cut.within)
                        .map_err(|why| format!("nowhere to cut before \"{phrase}\": {why}"))?,
                ),
                Some(
                    self.quiet_beside(clip, media, end, cut.within)
                        .map_err(|why| format!("nowhere to cut after \"{phrase}\": {why}"))?,
                ),
            )
        };
        let before = before.expect("both branches above find the quiet before");
        // Words sound between the quiet before them and the quiet after.
        if !cut.pause && after.is_some_and(|after| after.quiet.0 < before.quiet.1) {
            return Err(format!(
                "`{}` has no sound for \"{phrase}\" between two quiets, so its transcript's times \
                 are out there: use from= and to=",
                media.id
            ));
        }
        let (air_after, air_before) = airs(cut.leave);
        let played = |seconds: f64| {
            clip.played_at(media_time(seconds))
                .expect("media_of found media for it, so it plays a source")
        };
        // A frame of the sequence in a quiet, nearest a moment of the media.
        let frame = |ideal: f64, place: &Place| {
            frame_within(
                clock.rate,
                played(ideal),
                (played(place.room.0), played(place.room.1)),
            )
            .ok_or_else(|| {
                format!(
                    "no frame falls in the {:.2}s quiet of `{}` at {}",
                    place.quiet.1 - place.quiet.0,
                    media.id,
                    media_clock.print_position(media.rate.round(media_time(place.quiet.0)))
                )
            })
        };
        // An edge the clip does not play is the clip's own: what goes runs
        // from its start, or to its end.
        let from = frame(before.quiet.0 + air_after, &before)?.max(clip.start);
        let to = match &after {
            Some(after) => frame(after.quiet.1 - air_before, after)?.min(clip.end()),
            None => clip.end(),
        };
        if to <= from {
            return Err(if cut.pause {
                format!(
                    "the pause after \"{phrase}\" is {:.2}s, which leave={:.2}s does not shorten",
                    after.map_or(source_out, |after| after.quiet.1) - before.quiet.0,
                    cut.leave
                )
            } else {
                format!(
                    "nothing of \"{phrase}\" is left to take out of `{}`",
                    clip.id
                )
            });
        }
        // What is said of it: where, what it leaves, and between which words.
        let source = |at: Time| clip.source_at(at).map_or(0.0, |at| at.seconds());
        let heard_end = after.map_or(source_out, |after| after.quiet.1);
        // No more of a quiet than the clip plays.
        let left = (source(from) - before.quiet.0.max(source_in)).max(0.0)
            + (heard_end.min(source_out) - source(to)).max(0.0);
        let (last, next) = beside(&words, before.quiet.0, heard_end);
        let between = match (last, next) {
            (Some(last), Some(next)) if next.start < source_out && last.end > source_in => format!(
                " between {} and {}",
                quote(last.text.trim()),
                quote(next.text.trim())
            ),
            _ => String::new(),
        };
        let range = format!(
            "{} to {}",
            clock.print_position(from),
            clock.print_position(to)
        );
        self.snapped.borrow_mut().push(if cut.pause {
            format!(
                "the pause after \"{phrase}\" in `{}`, {:.2}s, comes down to {left:.2}s{between}: \
                 {range} goes",
                clip.id,
                heard_end - before.quiet.0,
            )
        } else {
            format!(
                "\"{phrase}\" goes from `{}`, {range}, which leaves {left:.2}s{between}",
                clip.id
            )
        });
        Ok((from, to))
    }

    /// The quiet an edge asked for at `seconds` of `media` would go into,
    /// within `within` of where `clip` plays it: the one it is in, or the
    /// one at the nearer end of the sound it is in, past no word.
    fn quiet_beside(
        &self,
        clip: &Clip,
        media: &Media,
        seconds: f64,
        within: Time,
    ) -> Result<Place, String> {
        let at = media_time(seconds);
        let offset = clip
            .played_at(at)
            .expect("said passes only a clip media_of found media for")
            - clip.start;
        let speed = clip.speed();
        let reach = speed.played(offset + within) - speed.played(offset);
        let (found, _) = self.snaps(media, at, Snap::Pause, reach)?;
        found.places.first().copied().ok_or_else(|| {
            nothing_near(
                media,
                media.rate.round(at),
                Snap::Pause,
                reach,
                &found.barred,
            )
        })
    }

    /// What transcripts of `media` heard, read once for a script.
    pub(super) fn heard(&self, media: &Media) -> Rc<Speech> {
        self.speech
            .borrow_mut()
            .entry(media.id.clone())
            .or_insert_with(|| Rc::new(self.facts.speech(media)))
            .clone()
    }

    /// `captions <clip> on=<track> [words=<n>]`: the caption lines for what
    /// the clip says, from the words transcripts of its media heard.
    pub(super) fn captions(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let track = fields
            .take_id("on")?
            .ok_or_else(|| line.error(1, "captions needs on=<subtitle track>"))?;
        let most = fields
            .take_with("words", |text| {
                text.parse()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| format!("`{text}` is not a word count"))
            })?
            .unwrap_or(DEFAULT_CAPTION_WORDS);
        fields.finish(2)?;
        let found = self
            .project
            .locate(clip.as_str())
            .map(|at| self.project.clip_at(at));
        let Some((found, media)) = found.and_then(|found| match &found.content {
            Content::Media { media, .. } => Some((found, self.project.media(media.as_str())?)),
            _ => None,
        }) else {
            return Err(line.missing(1, format!("no media clip `{clip}`")));
        };
        let speech = self.facts.speech(media);
        let (source_in, source_out) = found.source_range().unwrap_or_default();
        if !speech.covers(source_in.seconds(), source_out.seconds()) {
            let clock = Clock::of_media(media);
            return Err(line.error(
                1,
                format!(
                    "no transcript of `{}` covers `{clip}` ({}–{}); look at it with transcript, from, and to first",
                    media.id,
                    clock.print_position(source_in),
                    clock.print_position(source_out),
                ),
            ));
        }
        let script = caption_script(
            &self.project,
            clip.as_str(),
            track.as_str(),
            &speech.words,
            most,
        )
        .map_err(|message| line.error(1, message))?;
        // The caption lines run in the clip's sequence; the script's own
        // choice of sequence holds after them.
        let lines = lex(&script).map_err(|mut errors| line.error(1, errors.remove(0).message))?;
        let chosen = self.sequence.clone();
        let mut ops = Vec::new();
        let mut failed = None;
        for generated in &lines {
            match self.parse(generated) {
                Ok(more) => ops.extend(more),
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
        }
        self.sequence = chosen;
        match failed {
            Some(error) => Err(Diagnostic {
                missing: error.missing,
                ..line.error(1, error.message)
            }),
            None => Ok(ops),
        }
    }
}

/// What `extract <clip> said=` or `pause=` asks for.
pub(super) struct Said<'a> {
    /// The words, as a transcript heard them.
    pub(super) phrase: &'a str,
    /// Whether it is the pause after them that goes, and not the words.
    pub(super) pause: bool,
    /// How long a pause the cut leaves, in seconds.
    pub(super) leave: f64,
    /// How far from a word's time the quiet beside it is looked for.
    pub(super) within: Time,
}

/// Of `words` in the order they were said, the last whose middle is at or
/// before `from` seconds and the first whose middle is after `to`.
pub(super) fn beside<'w>(
    words: &[&'w Word],
    from: f64,
    to: f64,
) -> (Option<&'w Word>, Option<&'w Word>) {
    let middle = |word: &Word| (word.start + word.end) / 2.0;
    (
        words
            .iter()
            .rev()
            .find(|word| middle(word) <= from)
            .copied(),
        words.iter().find(|word| middle(word) > to).copied(),
    )
}

/// `seconds` of a media on its frame grid: the frame it falls in, or for an
/// out point the end of that frame, so a word at an edge is kept whole.
fn source_time(seconds: f64, rate: Rate, out: bool) -> Time {
    let time = Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO);
    let frames = rate.frames_floor(time);
    let frames = if out && !rate.is_aligned(time) {
        frames + 1
    } else {
        frames
    };
    rate.time_of(frames)
}
