//! What an editor would catch in a cut that validation does not: black
//! frames, picture that leaves the frame's edges uncovered, flash frames,
//! and sound cut inside a word.

use kerf_lang::Clock;
use kerf_model::{Clip, Content, Media, Param, Project, Sequence, Time, TrackKind, Value};
use serde_json::{json, Value as Json};

/// A picture or sound clip shorter than this many frames is a flash frame.
const FLASH_FRAMES: i64 = 3;
/// How far inside a word a cut must land to count as cutting through it,
/// in seconds. Word times from a transcript are approximate.
const WORD_MARGIN: f64 = 0.06;
/// Pixels of frame edge a picture may leave uncovered without a finding.
const EDGE_TOLERANCE: f64 = 0.5;

/// Findings for every sequence of `project`, whose media paths are
/// resolved. `words` gives the spoken words of a media, in seconds of the
/// media, as far as they are known without transcribing.
pub(crate) fn findings(
    project: &Project,
    words: impl Fn(&Media) -> Vec<(f64, f64, String)>,
) -> Vec<Json> {
    let mut findings = Vec::new();
    for sequence in &project.sequences {
        picture(project, sequence, &mut findings);
        flash_frames(sequence, &mut findings);
        cuts_in_words(project, sequence, &words, &mut findings);
    }
    findings
}

fn finding(code: &str, entity: &str, message: String) -> Json {
    json!({"severity": "warning", "code": code, "entity": entity, "message": message})
}

/// The clips that show a picture: enabled clips on enabled video tracks,
/// except adjustment layers and media without a picture.
fn pictures<'a>(project: &'a Project, sequence: &'a Sequence) -> impl Iterator<Item = &'a Clip> {
    sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video && track.enabled)
        .flat_map(|track| &track.clips)
        .filter(|clip| clip.enabled)
        .filter(move |clip| match &clip.content {
            Content::Media { media, .. } => project
                .media(media.as_str())
                .is_some_and(|media| media.video.is_some()),
            Content::Generator { fx, .. } => fx.as_str() != "adjustment",
            Content::Sequence { .. } => true,
            Content::Caption { .. } => false,
        })
}

/// Black stretches with no picture, and stretches where no picture covers
/// the whole frame.
fn picture(project: &Project, sequence: &Sequence, findings: &mut Vec<Json>) {
    let duration = sequence.duration();
    if duration <= Time::ZERO
        || !sequence
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Video)
    {
        return;
    }
    let clock = Clock::of_sequence(sequence);
    let shown: Vec<&Clip> = pictures(project, sequence).collect();
    let span = |clip: &&Clip| (clip.start, clip.end());
    let pictured = union(shown.iter().map(span));
    for (from, to) in complement(&pictured, duration) {
        findings.push(finding(
            "picture.black",
            sequence.id.as_str(),
            format!(
                "nothing is on screen in `{}` from {} to {}",
                sequence.id,
                clock.print_position(from),
                clock.print_position(to)
            ),
        ));
    }
    let covered = union(
        shown
            .iter()
            .filter(|clip| covers(project, sequence, clip))
            .map(span),
    );
    for (from, to) in subtract(&pictured, &covered) {
        let short: Vec<&str> = shown
            .iter()
            .filter(|clip| clip.start < to && clip.end() > from)
            .filter(|clip| !covers(project, sequence, clip))
            .map(|clip| clip.id.as_str())
            .collect();
        findings.push(finding(
            "picture.edges",
            short.first().copied().unwrap_or(sequence.id.as_str()),
            format!(
                "the frame's edges are black in `{}` from {} to {}: {} does not fill the {}x{} frame",
                sequence.id,
                clock.print_position(from),
                clock.print_position(to),
                short
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                sequence.width,
                sequence.height,
            ),
        ));
    }
}

/// Whether `clip` covers the whole frame. Pictures whose placement
/// changes over time, or that rotate, are given the benefit of the doubt.
fn covers(project: &Project, sequence: &Sequence, clip: &Clip) -> bool {
    let size = match &clip.content {
        Content::Media { media, .. } => project
            .media(media.as_str())
            .and_then(|media| media.video)
            .map(|video| (video.width, video.height)),
        Content::Sequence { sequence, .. } => project
            .sequence(sequence.as_str())
            .map(|nested| (nested.width, nested.height)),
        Content::Generator { .. } | Content::Caption { .. } => None,
    };
    let Some((width, height)) = size else {
        return true;
    };
    let mut values = [0.0; 10];
    let names = [
        "x", "y", "scale", "scale_x", "scale_y", "rotation", "crop_l", "crop_r", "crop_t", "crop_b",
    ];
    for (value, name) in values.iter_mut().zip(names) {
        let default = if name.starts_with("scale") { 1.0 } else { 0.0 };
        *value = match clip.props.get(name) {
            None => default,
            Some(Param::Static(Value::Number(number))) => *number,
            Some(_) => return true,
        };
    }
    let [x, y, scale, scale_x, scale_y, rotation, left, right, top, bottom] = values;
    if rotation != 0.0 {
        return true;
    }
    let (frame_width, frame_height) = (f64::from(sequence.width), f64::from(sequence.height));
    let cropped_width = f64::from(width) * (1.0 - left - right);
    let cropped_height = f64::from(height) * (1.0 - top - bottom);
    if cropped_width <= 0.0 || cropped_height <= 0.0 {
        return false;
    }
    // The renderer fits the picture inside the frame, then scales it about
    // its center and offsets it by fractions of the frame.
    let fit = (frame_width / cropped_width).min(frame_height / cropped_height);
    let shown_width = cropped_width * fit * (scale * scale_x).abs();
    let shown_height = cropped_height * fit * (scale * scale_y).abs();
    let center_x = frame_width / 2.0 + x * frame_width;
    let center_y = frame_height / 2.0 + y * frame_height;
    center_x - shown_width / 2.0 <= EDGE_TOLERANCE
        && center_x + shown_width / 2.0 >= frame_width - EDGE_TOLERANCE
        && center_y - shown_height / 2.0 <= EDGE_TOLERANCE
        && center_y + shown_height / 2.0 >= frame_height - EDGE_TOLERANCE
}

fn flash_frames(sequence: &Sequence, findings: &mut Vec<Json>) {
    let rate = sequence.rate;
    for track in sequence
        .tracks
        .iter()
        .filter(|track| track.kind != TrackKind::Subtitle)
    {
        for clip in &track.clips {
            let frames = rate.frames_round(clip.duration);
            if matches!(
                clip.content,
                Content::Media { .. } | Content::Sequence { .. }
            ) && frames < FLASH_FRAMES
            {
                findings.push(finding(
                    "clip.flash",
                    clip.id.as_str(),
                    format!(
                        "clip `{}` on {} is {frames} frame{} long",
                        clip.id,
                        track.id,
                        if frames == 1 { "" } else { "s" }
                    ),
                ));
            }
        }
    }
}

/// Sound clips whose first or last frame falls inside a word.
fn cuts_in_words(
    project: &Project,
    sequence: &Sequence,
    words: &impl Fn(&Media) -> Vec<(f64, f64, String)>,
    findings: &mut Vec<Json>,
) {
    let mut known = std::collections::HashMap::new();
    for track in sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio && track.enabled && !track.muted)
    {
        for clip in track.clips.iter().filter(|clip| clip.enabled) {
            let Content::Media { media, speed, .. } = &clip.content else {
                continue;
            };
            let Some(media) = project.media(media.as_str()) else {
                continue;
            };
            let Some((source_in, source_out)) = clip.source_range() else {
                continue;
            };
            if speed.is_reverse() {
                continue;
            }
            let spoken = known
                .entry(media.id.clone())
                .or_insert_with(|| words(media));
            let clock = Clock::of_media(media);
            for (end, at) in [("starts", source_in), ("ends", source_out)] {
                let at = at.seconds();
                let Some((from, to, word)) = spoken
                    .iter()
                    .find(|(from, to, _)| from + WORD_MARGIN < at && at < to - WORD_MARGIN)
                else {
                    continue;
                };
                let position = |seconds: f64| {
                    let time = Time::from_seconds(seconds).unwrap_or(Time::ZERO);
                    clock.print_position(clock.rate.round(time))
                };
                findings.push(finding(
                    "cut.in_word",
                    clip.id.as_str(),
                    format!(
                        "clip `{}` {end} inside the word \"{word}\" ({}–{} of `{}`, by its transcript)",
                        clip.id,
                        position(*from),
                        position(*to),
                        media.id,
                    ),
                ));
            }
        }
    }
}

/// Merge spans into sorted, disjoint spans.
fn union(spans: impl Iterator<Item = (Time, Time)>) -> Vec<(Time, Time)> {
    let mut spans: Vec<_> = spans.filter(|(from, to)| to > from).collect();
    spans.sort();
    let mut merged: Vec<(Time, Time)> = Vec::new();
    for (from, to) in spans {
        match merged.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    merged
}

/// The parts of `[0, duration)` outside `spans`, which are sorted and
/// disjoint.
fn complement(spans: &[(Time, Time)], duration: Time) -> Vec<(Time, Time)> {
    subtract(&[(Time::ZERO, duration)], spans)
}

/// The parts of `spans` outside `removed`; both are sorted and disjoint.
fn subtract(spans: &[(Time, Time)], removed: &[(Time, Time)]) -> Vec<(Time, Time)> {
    let mut left = Vec::new();
    for &(from, to) in spans {
        let mut start = from;
        for &(cut_from, cut_to) in removed {
            if cut_to <= start || cut_from >= to {
                continue;
            }
            if cut_from > start {
                left.push((start, cut_from));
            }
            start = start.max(cut_to);
        }
        if start < to {
            left.push((start, to));
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_lang::{apply_script, parse_document};
    use kerf_model::Catalog;

    const DOCUMENT: &str = "kerf 1
project name=\"Test\"
media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m1 src=00:00:10:00 at=00:00:04:00 dur=00:00:04:00 link=L2
  track A1 audio
    clip a1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
    clip a2 media=m1 src=00:00:10:00 at=00:00:04:00 dur=00:00:04:00 link=L2
";

    fn codes(script: &str, words: &[(f64, f64, &str)]) -> Vec<(String, String)> {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let project = apply_script(&project, catalog, script).unwrap().project;
        let words: Vec<_> = words
            .iter()
            .map(|(from, to, word)| (*from, *to, (*word).to_owned()))
            .collect();
        findings(&project, |_| words.clone())
            .into_iter()
            .map(|finding| {
                (
                    finding["code"].as_str().unwrap().to_owned(),
                    finding["message"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn a_clean_cut_has_no_findings() {
        assert!(codes("set c1 opacity=1", &[]).is_empty());
    }

    #[test]
    fn a_gap_in_the_picture_is_black() {
        let found = codes("delete c1", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "picture.black");
        assert!(
            found[0].1.contains("from 00:00:00:00 to 00:00:04:00"),
            "{found:?}"
        );
    }

    #[test]
    fn a_vertical_frame_shows_the_edges_of_a_wide_picture() {
        let found = codes("sequence set s1 size=1080x1920", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "picture.edges");
        assert!(found[0].1.contains("`c1`, `c2`"), "{found:?}");
        // Scaled up to fill the height, it covers the frame.
        let filled = codes(
            "sequence set s1 size=1080x1920\nset c1 scale=3.17\nset c2 scale=3.17",
            &[],
        );
        assert!(filled.is_empty(), "{filled:?}");
    }

    #[test]
    fn short_clips_are_flash_frames() {
        let found = codes("trim c1 out to=2f ripple", &[]);
        assert!(
            found.contains(&(
                "clip.flash".into(),
                "clip `c1` on V1 is 2 frames long".into()
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_sound_cut_inside_a_word_is_found() {
        // a1 ends at 4s of m1, inside "country"; a2 starts at 10s, between
        // words.
        let found = codes(
            "set c1 opacity=1",
            &[(3.8, 4.3, "country"), (9.5, 9.9, "ask")],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "cut.in_word");
        assert!(
            found[0].1.contains(
                "clip `a1` ends inside the word \"country\" (00:00:03:19–00:00:04:07 of `m1`"
            ),
            "{found:?}"
        );
    }
}
