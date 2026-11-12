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
    text.trim_end()
        .ends_with(['.', '?', '!', '。', '？', '！'])
}

/// Whether `text` is a transcriber's note of a sound, such as whisper's
/// `[BLANK_AUDIO]` or `[MUSIC]`, and not something said.
fn is_a_sound_note(text: &str) -> bool {
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
    let until = |made: &[Made], index: usize| made.get(index + 1).map_or(last_frame, |next| next.start);
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
mod tests {
    use super::*;
    use crate::{apply_script, parse_document};
    use kerf_model::Catalog;

    const DOCUMENT: &str = "kerf 1
project name=\"Captions\"
media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=00:00:02:00 dur=00:00:04:00
  track S1 subtitle
";

    fn word(start: f64, end: f64, text: &str) -> Word {
        Word {
            start,
            end,
            text: text.into(),
        }
    }

    #[test]
    fn captions_follow_the_speech_as_the_clip_plays_it() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let words = [
            word(9.0, 9.5, "before"),
            word(9.8, 10.1, "cut"),
            word(10.25, 10.5, "So"),
            word(10.5, 11.0, "here"),
            word(11.0, 11.5, "we"),
            word(11.5, 12.0, "are."),
            word(12.0, 12.5, "Next"),
            word(13.5, 14.5, "line"),
            word(13.6, 14.4, "line"),
        ];
        let script = caption_script(&project, "c1", "S1", &words, 3).unwrap();
        // "are." is said in half a second with "Next" straight after it,
        // too short to read on its own: it is the second line of the
        // caption before it. "Next" stays its 5/6 of a second.
        assert_eq!(
            script,
            "overwrite caption=\"So here we\\nare.\" at=00:00:02:06 dur=42f on=S1
overwrite caption=\"Next\" at=00:00:04:00 dur=20f on=S1"
        );
        let captioned = apply_script(&project, catalog, &script).unwrap().project;
        // "line" is cut in half at the clip's out point and left out.
        assert_eq!(captioned.sequences[0].tracks[1].clips.len(), 2);

        // At double speed the same words take half the time.
        let fast = apply_script(&project, catalog, "speed c1 2")
            .unwrap()
            .project;
        let script = caption_script(&fast, "c1", "S1", &words[..3], 3).unwrap();
        assert_eq!(
            script,
            "overwrite caption=\"So\" at=00:00:02:03 dur=20f on=S1"
        );
    }

    /// What `captions` makes is held to what a check holds a caption to.
    fn flagged(script: &str, fps: f64) -> Vec<String> {
        let mut flagged = Vec::new();
        let mut before: Option<i64> = None;
        for line in script.lines() {
            let text = line.split('"').nth(1).unwrap().replace("\\n", "\n");
            let field = |key: &str| line.split(key).nth(1).unwrap().split(' ').next().unwrap();
            let at = field(" at=");
            let parts: Vec<i64> = at.split(':').map(|part| part.parse().unwrap()).collect();
            let start = ((parts[0] * 60 + parts[1]) * 60 + parts[2]) * fps as i64 + parts[3];
            let frames: i64 = field(" dur=").trim_end_matches('f').parse().unwrap();
            let seconds = frames as f64 / fps;
            if caption_characters(&text) as f64 / seconds > READING_SPEED + 1e-9 {
                flagged.push(format!("fast: {line}"));
            }
            if seconds < SHORT_CAPTION_SIXTHS as f64 / 6.0 - 1e-9 {
                flagged.push(format!("short: {line}"));
            }
            if let Some(end) = before {
                if start > end && start - end < CAPTION_GAP_FRAMES {
                    flagged.push(format!("gap: {line}"));
                }
                assert!(start >= end, "captions overlap: {line}");
            }
            before = Some(start + frames);
        }
        flagged
    }

    #[test]
    fn captions_stay_up_long_enough_to_read_where_the_speech_leaves_room() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        // A sentence said quickly, four words to a caption: 19 characters a
        // second in the middle of it, a one-word end, then a pause.
        let words = [
            word(10.0, 10.3, "Some"),
            word(10.3, 10.7, "animals"),
            word(10.7, 10.9, "such"),
            word(10.9, 11.3, "as"),
            word(11.3, 11.8, "elephants"),
            word(11.8, 12.0, "and"),
            word(12.0, 12.4, "giraffes"),
            word(12.4, 12.6, "come"),
            word(12.6, 12.8, "near."),
            word(13.3, 13.5, "Yes."),
        ];
        let before = "overwrite caption=\"Some animals such as\" at=00:00:02:00 dur=31f on=S1
overwrite caption=\"elephants and giraffes come\" at=00:00:03:07 dur=31f on=S1
overwrite caption=\"near.\" at=00:00:04:14 dur=5f on=S1
overwrite caption=\"Yes.\" at=00:00:05:07 dur=5f on=S1";
        assert_eq!(flagged(before, 24.0).len(), 5, "as it was made before");
        let script = caption_script(&project, "c1", "S1", &words, 4).unwrap();
        assert_eq!(
            script,
            "overwrite caption=\"Some animals such as\" at=00:00:02:00 dur=31f on=S1
overwrite caption=\"elephants and giraffes come\\nnear.\" at=00:00:03:07 dur=48f on=S1
overwrite caption=\"Yes.\" at=00:00:05:07 dur=17f on=S1"
        );
        // "Yes." has 17 frames before the clip ends, which no caption
        // outlasts: the one thing left to say of it.
        assert_eq!(flagged(&script, 24.0), ["short: overwrite caption=\"Yes.\" at=00:00:05:07 dur=17f on=S1"]);
        let captioned = apply_script(&project, catalog, &script).unwrap().project;
        assert_eq!(captioned.sequences[0].tracks[1].clips.len(), 3);
    }

    #[test]
    fn a_caption_meets_the_next_rather_than_stop_a_frame_short_of_it() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        // The first is read in 20 frames and the second starts on the 21st.
        let words = [
            word(10.0, 10.4, "One"),
            word(10.4, 10.8, "two."),
            word(10.875, 11.3, "Three"),
            word(11.3, 12.0, "four."),
        ];
        let script = caption_script(&project, "c1", "S1", &words, 4).unwrap();
        assert_eq!(
            script,
            "overwrite caption=\"One two.\" at=00:00:02:00 dur=21f on=S1
overwrite caption=\"Three four.\" at=00:00:02:21 dur=27f on=S1"
        );
        assert!(flagged(&script, 24.0).is_empty());
    }

    #[test]
    fn speech_faster_than_a_viewer_reads_is_left_to_a_check_to_say() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        // Twenty characters a second with no pause anywhere: no timing of
        // these words can be read, and none is said twice or left out.
        let words: Vec<Word> = (0..12)
            .map(|index| {
                let start = 10.0 + f64::from(index) * 0.25;
                word(start, start + 0.25, "word")
            })
            .collect();
        let script = caption_script(&project, "c1", "S1", &words, 4).unwrap();
        assert_eq!(script.matches("word").count(), 12);
        assert!(flagged(&script, 24.0).iter().any(|line| line.starts_with("fast")));
    }

    #[test]
    fn a_transcribers_note_of_a_sound_is_not_captioned() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let words = [
            word(10.0, 10.5, "Good"),
            word(10.5, 11.0, "night."),
            word(12.0, 12.0, "[BLANK_AUDIO]"),
            word(12.5, 13.0, "[MUSIC]"),
        ];
        let script = caption_script(&project, "c1", "S1", &words, 4).unwrap();
        assert_eq!(
            script,
            "overwrite caption=\"Good night.\" at=00:00:02:00 dur=24f on=S1"
        );
        assert!(caption_script(&project, "c1", "S1", &words[2..], 4).is_err());
    }

    #[test]
    fn words_at_one_moment_stay_in_one_caption() {
        // The first six words were timed at the same moment, as whisper
        // times the start of a sentence. Three to a caption, the first two
        // captions would start on one frame and the second replace the
        // first: they are one caption, and nothing said is lost.
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let words = [
            word(10.0, 10.0, "The"),
            word(10.0, 10.0, "term"),
            word(10.0, 10.0, "bug"),
            word(10.0, 10.0, "is"),
            word(10.0, 10.0, "used"),
            word(10.0, 10.5, "by"),
            word(10.5, 11.0, "people"),
            word(11.0, 11.5, "here."),
        ];
        let script = caption_script(&project, "c1", "S1", &words, 3).unwrap();
        // Half a second is not time to read the first, so the two are one
        // caption of two lines, up for as long as it takes to read.
        assert_eq!(
            script,
            "overwrite caption=\"The term bug is used by\\npeople here.\" at=00:00:02:00 dur=51f on=S1"
        );
        let captioned = apply_script(&project, catalog, &script).unwrap().project;
        let captions = &captioned.sequences[0].tracks[1].clips;
        assert_eq!(captions.len(), 1);
        assert!(
            matches!(&captions[0].content, Content::Caption { text } if text == "The term bug is used by\npeople here."),
            "{captions:?}"
        );
        assert!(captioned.retired.is_empty());
    }

    /// Facts that heard `words` over the whole of every media.
    struct Heard(Vec<Word>);

    impl crate::Facts for Heard {
        fn speech(&self, _: &kerf_model::Media) -> crate::Speech {
            crate::Speech {
                words: self.0.clone(),
                covered: vec![(0.0, 60.0)],
            }
        }
    }

    #[test]
    fn a_captions_line_becomes_the_caption_lines() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let heard = Heard(vec![
            word(10.25, 10.5, "So"),
            word(10.5, 11.0, "here"),
            word(11.0, 11.5, "we"),
            word(11.5, 12.0, "are."),
        ]);
        let script = "captions c1 on=S1 words=3\nmarker add at=c1.end as=done";
        let applied = crate::apply_script_with(&project, catalog, &heard, script).unwrap();
        let captions = &applied.project.sequences[0].tracks[1].clips;
        assert_eq!(captions.len(), 2);
        assert_eq!(
            crate::print_ops(&project, catalog, &applied.ops),
            "overwrite caption=\"So here we\" at=00:00:02:06 dur=00:00:01:06 on=S1
overwrite caption=\"are.\" at=00:00:03:12 dur=00:00:00:20 on=S1
marker add as=done at=00:00:06:00
"
        );

        // Without transcripts the line says what is missing, where it is.
        let error = apply_script(&project, catalog, script)
            .unwrap_err()
            .remove(0);
        assert!(
            error.message.contains("no transcript of `m1` covers `c1`"),
            "{error:?}"
        );
        assert_eq!(error.span.map(|span| span.line), Some(1));
        let error = crate::apply_script_with(&project, catalog, &heard, "captions c1 on=S1 size=2")
            .unwrap_err()
            .remove(0);
        assert!(error.message.contains("unknown key `size`"), "{error:?}");
    }

    #[test]
    fn captions_need_words_and_a_subtitle_track() {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        assert!(caption_script(&project, "c1", "V1", &[word(10.5, 11.0, "x")], 3).is_err());
        assert!(caption_script(&project, "c1", "S1", &[word(30.0, 31.0, "x")], 3).is_err());
        assert!(caption_script(&project, "c9", "S1", &[], 3).is_err());
    }
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

    #[test]
    fn cue_times_round_to_the_millisecond_on_ntsc_rates() {
        let rate = kerf_model::Rate::FPS_29_97;
        // Frame 1 of 29.97 starts at 33.3667 ms; frame 1800 at 60.06 s.
        assert_eq!(cue_time(rate.time_of(1), ','), "00:00:00,033");
        assert_eq!(cue_time(rate.time_of(1800), '.'), "00:01:00.060");
        assert_eq!(
            cue_time(Time::from_seconds(3723.5).unwrap(), ','),
            "01:02:03,500"
        );
    }
}
