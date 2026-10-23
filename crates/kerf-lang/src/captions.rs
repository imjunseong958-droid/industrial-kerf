//! Captions from timed words: what a clip says, as script lines that put
//! caption clips on a subtitle track, timed to the speech as the clip
//! plays it.

use crate::text::{quote, Clock};
use kerf_model::{Content, Project, Time, TrackKind};

/// A spoken word, timed in seconds of its media.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A pause longer than this starts a new caption.
const PAUSE_SECONDS: f64 = 0.5;

/// Script lines that caption what `clip` says onto the subtitle `track`
/// of its sequence, from `words` timed on its media. A caption holds at
/// most `max_words` words and ends at a pause or the end of a sentence.
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
    let (from, to) = found.source_range().unwrap_or_default();
    let (from, to) = (from.seconds(), to.seconds());
    let mut spoken: Vec<&Word> = words
        .iter()
        // A word the clip cuts belongs to whichever side has most of it.
        .filter(|word| (from..to).contains(&((word.start + word.end) / 2.0)))
        .filter(|word| !word.text.trim().is_empty())
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
                    || last
                        .text
                        .trim_end()
                        .ends_with(['.', '?', '!', '。', '？', '！'])
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
        let span =
            Time::from_seconds((seconds.clamp(from, to) - from).max(0.0)).unwrap_or(Time::ZERO);
        rate.round(found.start + speed.timeline_span(span))
    };
    let mut lines = Vec::new();
    if project.sequences.len() > 1 {
        lines.push(format!("use {}", sequence.id));
    }
    let starts: Vec<Time> = groups.iter().map(|group| place(group[0].start)).collect();
    for (index, group) in groups.iter().enumerate() {
        let start = starts[index];
        let mut end = place(group.last().expect("groups are never empty").end);
        if let Some(next) = starts.get(index + 1) {
            end = end.min(*next);
        }
        let frames = rate.frames_round(end - start).max(1);
        let text = group
            .iter()
            .map(|word| word.text.trim())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!(
            "overwrite caption={} at={} dur={frames}f on={track}",
            quote(&text),
            clock.print_position(start),
        ));
    }
    Ok(lines.join("\n"))
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
        assert_eq!(
            script,
            "overwrite caption=\"So here we\" at=00:00:02:06 dur=30f on=S1
overwrite caption=\"are.\" at=00:00:03:12 dur=12f on=S1
overwrite caption=\"Next\" at=00:00:04:00 dur=12f on=S1"
        );
        let captioned = apply_script(&project, catalog, &script).unwrap().project;
        // "line" is cut in half at the clip's out point and left out.
        assert_eq!(captioned.sequences[0].tracks[1].clips.len(), 3);

        // At double speed the same words take half the time.
        let fast = apply_script(&project, catalog, "speed c1 2")
            .unwrap()
            .project;
        let script = caption_script(&fast, "c1", "S1", &words[..3], 3).unwrap();
        assert_eq!(
            script,
            "overwrite caption=\"So\" at=00:00:02:03 dur=3f on=S1"
        );
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
