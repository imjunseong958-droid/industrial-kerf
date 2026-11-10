//! Captions from timed words: what a clip says, as script lines that put
//! caption clips on a subtitle track, timed to the speech as the clip
//! plays it. And captions out: a sequence's caption clips as SubRip
//! (`.srt`) and WebVTT (`.vtt`) files.

use crate::text::{quote, Clock};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Content, Id, Project, Sequence, Time, TrackKind};

/// A spoken word, timed in seconds of its media.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A pause longer than this starts a new caption.
const PAUSE_SECONDS: f64 = 0.5;

/// Words a caption holds unless `captions … words=<n>` says otherwise.
pub(crate) const DEFAULT_CAPTION_WORDS: usize = 4;

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
    let (low, high) = found.source_range().unwrap_or_default();
    let (from, to) = (low.seconds(), high.seconds());
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
        let source = Time::from_seconds(seconds).map_or(low, |time| time.clamp(low, high));
        rate.round(found.played_at(source).unwrap_or(found.start))
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
overwrite caption=\"are.\" at=00:00:03:12 dur=00:00:00:12 on=S1
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
