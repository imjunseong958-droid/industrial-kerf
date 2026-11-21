//! Captions from timed words: what a clip says, as script lines that put
//! caption clips on a subtitle track, timed to the speech as the clip
//! plays it. And captions out: a sequence's caption clips as SubRip
//! (`.srt`) and WebVTT (`.vtt`) files.

use crate::text::{quote, Clock};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Content, Id, Project, Rate, Sequence, Time, TrackKind};

/// A spoken word, timed in seconds of its media.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A pause longer than this starts a new caption.
const PAUSE_SECONDS: f64 = 0.5;

/// The most characters a viewer reads in a second (Netflix's limit for
/// adult programmes). `captions` keeps what it makes to it where the speech
/// leaves room, and a check warns of a caption that asks for more.
pub const READING_SPEED: f64 = 17.0;
/// The least time a caption may stay up, in sixths of a second (5/6 s).
pub const SHORT_CAPTION_SIXTHS: i64 = 5;
/// The least gap between two captions, in frames; back to back is fine.
pub const CAPTION_GAP_FRAMES: i64 = 2;
/// The most lines `captions` gives a caption.
const CAPTION_LINES: usize = 2;

/// What a viewer reads of a caption: its characters, with each run of
/// spaces and line breaks one space.
pub fn caption_characters(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| word.chars().count())
        .sum::<usize>()
        + text.split_whitespace().count().saturating_sub(1)
}

/// The frames a caption of `characters` has to stay up at `rate` to be
/// read: no faster than [`READING_SPEED`], and 5/6 of a second at least.
fn reading_frames(rate: Rate, characters: usize) -> i64 {
    let frame = rate.frame_duration().flicks();
    let shortest = FLICKS_PER_SECOND * SHORT_CAPTION_SIXTHS / 6;
    let read = (characters as f64 / READING_SPEED * FLICKS_PER_SECOND as f64).ceil() as i64;
    // Up to the next whole frame.
    (shortest.max(read) + frame - 1) / frame
}

/// A caption on its way to the track: its lines, each the words of one
/// run of speech, where it starts, and where what it says ends.
struct Made<'w> {
    lines: Vec<Vec<&'w Word>>,
    start: Time,
    spoken: Time,
}

impl Made<'_> {
    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|word| word.text.trim())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether it can be read in the time up to `until`.
    fn fits(&self, rate: Rate, until: Time) -> bool {
        rate.frames_round(until - self.start)
            >= reading_frames(rate, caption_characters(&self.text()))
    }

    fn ends_a_sentence(&self) -> bool {
        self.lines
            .last()
            .and_then(|line| line.last())
            .is_some_and(|word| ends_a_sentence(&word.text))
    }
}

fn ends_a_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['.', '?', '!', '。', '？', '！'])
}

/// Whether `text` is a transcriber's note of a sound, such as whisper's
/// `[BLANK_AUDIO]` or `[MUSIC]`, and not something said.
pub fn is_a_sound_note(text: &str) -> bool {
    let text = text.trim();
    text.len() > 2 && text.starts_with('[') && text.ends_with(']')
}

/// Words a caption holds unless `captions … words=<n>` says otherwise.
pub(crate) const DEFAULT_CAPTION_WORDS: usize = 4;

/// Script lines that caption what `clip` says onto the subtitle `track`
/// of its sequence, from `words` timed on its media. A line of a caption
/// holds at most `max_words` words and ends at a pause or the end of a
/// sentence.
///
/// A caption stays up past its last word for as long as it takes to read
/// ([`READING_SPEED`], and 5/6 of a second at least), where the next one
/// leaves the time. One that still cannot be read in its time becomes the
/// second line of the caption before it, or takes the next as its second
/// line, where the two together can be and no pause parts them. What is
/// left too fast is speech faster than a viewer reads, which a check says.
///
/// The lines overwrite what the track has there and begin with `use` when
/// the project has several sequences.
pub fn caption_script(
    project: &Project,
    clip: &str,
    track: &str,
    words: &[Word],
    max_words: usize,
) -> Result<String, String> {
    let at = project
        .locate(clip)
        .ok_or_else(|| format!("no clip `{clip}`"))?;
    let sequence = &project.sequences[at.sequence];
    let found = project.clip_at(at);
    let Content::Media { speed, .. } = &found.content else {
        return Err(format!("clip `{clip}` does not play media"));
    };
    if speed.is_reverse() {
        return Err(format!("clip `{clip}` plays in reverse"));
    }
    if sequence
        .track(track)
        .is_none_or(|track| track.kind != TrackKind::Subtitle)
    {
        return Err(format!(
            "`{track}` is not a subtitle track of `{}`",
            sequence.id
        ));
    }
    let (low, high) = found.source_range().unwrap_or_default();
    let (from, to) = (low.seconds(), high.seconds());
    let mut spoken: Vec<&Word> = words
        .iter()
        // A word the clip cuts belongs to whichever side has most of it.
        .filter(|word| (from..to).contains(&((word.start + word.end) / 2.0)))
        .filter(|word| !word.text.trim().is_empty())
        .filter(|word| !is_a_sound_note(&word.text))
        .collect();
    spoken.sort_by(|a, b| a.start.total_cmp(&b.start));
    // Transcripts of overlapping ranges repeat words; keep the first.
    let mut kept: Vec<&Word> = Vec::new();
    for word in spoken {
        if kept.last().is_none_or(|last| word.start >= last.end - 0.05) {
            kept.push(word);
        }
    }
    if kept.is_empty() {
        return Err(format!("nothing is said in clip `{clip}`"));
    }
    let mut groups: Vec<Vec<&Word>> = Vec::new();
    for word in kept {
        let starts_new = match groups.last() {
            None => true,
            Some(group) => {
                let last = group.last().expect("groups are never empty");
                group.len() >= max_words.max(1)
                    || word.start - last.end > PAUSE_SECONDS
                    || ends_a_sentence(&last.text)
            }
        };
        if starts_new {
            groups.push(vec![word]);
        } else {
            groups.last_mut().expect("just pushed").push(word);
        }
    }
    let rate = sequence.rate;
    let clock = Clock::of_sequence(sequence);
    // Where the clip plays `seconds` of its media, on the nearest frame.
    let place = |seconds: f64| {
        let source = Time::from_seconds(seconds).map_or(low, |time| time.clamp(low, high));
        rate.round(found.played_at(source).unwrap_or(found.start))
    };
    let mut lines = Vec::new();
    if project.sequences.len() > 1 {
        lines.push(format!("use {}", sequence.id));
    }
    // Two captions cannot start on one frame: the second would replace the
    // first, and what it said would be gone. Words timed too closely to
    // tell apart stay in one caption, however many it then holds.
    let mut placed: Vec<(Time, Vec<&Word>)> = Vec::new();
    for group in groups {
        let start = place(group[0].start);
        match placed.last_mut() {
            Some((before, words)) if start <= *before => words.extend(group),
            _ => placed.push((start, group)),
        }
    }
    let mut made: Vec<Made> = placed
        .into_iter()
        .map(|(start, group)| Made {
            start,
            spoken: place(group.last().expect("groups are never empty").end),
            lines: vec![group],
        })
        .collect();
    // No caption outlasts the clip whose speech it is.
    let last_frame = rate.round(found.end());
    let until =
        |made: &[Made], index: usize| made.get(index + 1).map_or(last_frame, |next| next.start);
    // Two that follow each other in one breath can be one caption of two
    // lines.
    let joins = |made: &[Made], first: usize| {
        let (a, b) = (&made[first], &made[first + 1]);
        a.lines.len() + b.lines.len() <= CAPTION_LINES
            && (b.start - a.spoken).seconds() <= PAUSE_SECONDS
    };
    let join = |made: &mut Vec<Made>, first: usize| {
        let second = made.remove(first + 1);
        made[first].lines.extend(second.lines);
        made[first].spoken = second.spoken;
    };
    let mut index = 0;
    while index < made.len() {
        if made[index].fits(rate, until(&made, index)) {
            index += 1;
            continue;
        }
        // With the one after it, or the one before: whichever can be read
        // in the time the two have, and first the one in its own sentence.
        let after = (index + 1 < made.len() && joins(&made, index)).then_some(index);
        let before = (index > 0 && joins(&made, index - 1)).then(|| index - 1);
        let order = if made[index].ends_a_sentence() {
            [before, after]
        } else {
            [after, before]
        };
        let joined = order.into_iter().flatten().find(|first| {
            let both = Made {
                lines: made[*first..=*first + 1]
                    .iter()
                    .flat_map(|made| made.lines.clone())
                    .collect(),
                start: made[*first].start,
                spoken: made[*first + 1].spoken,
            };
            both.fits(rate, until(&made, *first + 1))
        });
        match joined {
            Some(first) => {
                join(&mut made, first);
                index = first + 1;
            }
            None => index += 1,
        }
    }
    for (index, caption) in made.iter().enumerate() {
        let text = caption.text();
        let next = until(&made, index);
        // It stays for as long as it takes to read, the next one allowing.
        let read = caption.start + rate.time_of(reading_frames(rate, caption_characters(&text)));
        let mut end = caption.spoken.max(read).min(next);
        // Back to back with the next rather than a frame short of it.
        if index + 1 < made.len() {
            let gap = rate.frames_round(next - end);
            if gap > 0 && gap < CAPTION_GAP_FRAMES {
                end = next;
            }
        }
        let frames = rate.frames_round(end - caption.start).max(1);
        lines.push(format!(
            "overwrite caption={} at={} dur={frames}f on={track}",
            quote(&text),
            clock.print_position(caption.start),
        ));
    }
    Ok(lines.join("\n"))
}

/// One caption as a file carries it: sequence offsets and its lines.
struct Cue {
    start: Time,
    end: Time,
    lines: Vec<String>,
}

/// The caption clips of `sequence`'s subtitle tracks, or of `track` alone,
/// in the order they appear. Disabled tracks and clips, and captions with
/// no text, are left out; blank lines inside a caption are dropped, as both
/// formats end a cue at one.
fn cues(sequence: &Sequence, track: Option<&Id>) -> Vec<Cue> {
    let mut cues: Vec<(usize, Cue)> = Vec::new();
    for (index, found) in sequence.tracks.iter().enumerate() {
        if found.kind != TrackKind::Subtitle
            || !found.enabled
            || track.is_some_and(|track| *track != found.id)
        {
            continue;
        }
        for clip in found.clips.iter().filter(|clip| clip.enabled) {
            let Content::Caption { text } = &clip.content else {
                continue;
            };
            let lines: Vec<String> = text
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect();
            if lines.is_empty() {
                continue;
            }
            cues.push((
                index,
                Cue {
                    start: clip.start,
                    end: clip.end(),
                    lines,
                },
            ));
        }
    }
    cues.sort_by_key(|(index, cue)| (cue.start, *index));
    cues.into_iter().map(|(_, cue)| cue).collect()
}

/// `HH:MM:SS` then `separator` and milliseconds, rounded to the nearest.
fn cue_time(time: Time, separator: char) -> String {
    let flicks = i128::from(time.flicks().max(0));
    let per_second = i128::from(FLICKS_PER_SECOND);
    let ms = (flicks * 1000 + per_second / 2) / per_second;
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The captions of `sequence` (every subtitle track, or `track` alone) as
/// a SubRip file, timed from the sequence's first frame.
pub fn to_srt(sequence: &Sequence, track: Option<&Id>) -> String {
    let mut out = String::new();
    for (number, cue) in cues(sequence, track).iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            number + 1,
            cue_time(cue.start, ','),
            cue_time(cue.end, ','),
            cue.lines.join("\n")
        ));
    }
    out
}

/// The captions of `sequence` (every subtitle track, or `track` alone) as
/// a WebVTT file, timed from the sequence's first frame.
pub fn to_vtt(sequence: &Sequence, track: Option<&Id>) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues(sequence, track) {
        let lines: Vec<String> = cue
            .lines
            .iter()
            .map(|line| {
                line.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            })
            .collect();
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            cue_time(cue.start, '.'),
            cue_time(cue.end, '.'),
            lines.join("\n")
        ));
    }
    out
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use crate::parse_document;
    use kerf_model::Catalog;

    const DOCUMENT: &str = "kerf 1
project name=\"Captions\"
sequence s1 rate=24 size=1920x1080 start=01:00:00:00
  track V1 video
  track S1 subtitle
    clip k1 caption=\"So here we\\n\\n  are. \" at=01:00:00:06 dur=00:00:02:00
    clip k2 caption=\"Fish & <chips>\" at=01:00:03:00 dur=00:00:01:12
    clip k4 caption=\"Off\" at=01:00:06:00 dur=00:00:01:00 enabled=false
  track S2 subtitle
    clip k3 caption=\"[music]\" at=01:00:01:00 dur=00:00:00:13
    clip k5 caption=\"  \" at=01:00:08:00 dur=00:00:01:00
  track S3 subtitle enabled=false
    clip k6 caption=\"Hidden\" at=01:00:00:00 dur=00:00:01:00
";

    fn sequence() -> Sequence {
        let project = parse_document(DOCUMENT, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        project.sequences[0].clone()
    }

    #[test]
    fn vtt_escapes_markup_and_can_take_one_track() {
        assert_eq!(
            to_vtt(&sequence(), None),
            "WEBVTT

00:00:00.250 --> 00:00:02.250
So here we
are.

00:00:01.000 --> 00:00:01.542
[music]

00:00:03.000 --> 00:00:04.500
Fish &amp; &lt;chips&gt;

"
        );
        let s2 = Id::new("S2").unwrap();
        assert_eq!(
            to_vtt(&sequence(), Some(&s2)),
            "WEBVTT\n\n00:00:01.000 --> 00:00:01.542\n[music]\n\n"
        );
        assert_eq!(to_srt(&sequence(), Some(&Id::new("V1").unwrap())), "");
    }
}
