//! What an editor would catch in a cut that validation does not: black
//! frames, picture that leaves the frame's edges uncovered, flash frames
//! (including a sliver of another shot at a clip's edge), sound cut
//! inside a word, and text and captions a viewer cannot read (see
//! `text_lint`).

use crate::qc::{Finding, Severity};
use crate::text_lint;
use kerf_lang::{Breaks, Clock, Facts, Speech};
use kerf_model::{Clip, Content, Id, Param, Project, Sequence, Time, TrackKind, Value};

/// A picture or sound clip shorter than this many frames is a flash frame.
const FLASH_FRAMES: i64 = 3;
/// How far inside a word a cut must land to count as cutting through it,
/// in seconds. Word times from a transcript are approximate.
const WORD_MARGIN: f64 = 0.06;
/// Pixels of frame edge a picture may leave uncovered without a finding.
const EDGE_TOLERANCE: f64 = 0.5;

/// Findings for every sequence of `project`, whose media paths are
/// resolved. `facts` knows what transcripts and indexes already made of its
/// media heard and found; nothing is analysed for a check.
pub(crate) fn findings(project: &Project, facts: &dyn Facts) -> Vec<Finding> {
    let mut findings = Vec::new();
    for sequence in &project.sequences {
        picture(project, sequence, &mut findings);
        flash_frames(sequence, &mut findings);
        slivers(project, sequence, facts, &mut findings);
        cuts_in_words(project, sequence, facts, &mut findings);
        speech_unheard(project, sequence, facts, &mut findings);
        text_lint::text(sequence, &mut findings);
        text_lint::captions(sequence, &mut findings);
        does_nothing(sequence, &mut findings);
    }
    findings
}

/// Numbers and choices set where nothing reads them, which validation has no reason to refuse and which
/// show nothing in a frame. As information: a person may mean to come back to them.
fn does_nothing(sequence: &Sequence, findings: &mut Vec<Finding>) {
    let clips = || sequence.tracks.iter().flat_map(|track| &track.clips);
    let in_depth_over = |clip: &Clip| {
        clips().any(|other| {
            other.id != clip.id
                && is_set_true(other.props.get("three_d"))
                && other.start < clip.start + clip.duration
                && clip.start < other.start + other.duration
        })
    };
    for clip in clips() {
        let id = clip.id.as_str();
        let mut says = |message: String| {
            findings.push(
                Finding::new(
                    Severity::Info,
                    NO_EFFECT,
                    Some(id),
                    format!("`{id}`: {message}"),
                )
                .during(sequence, clip.start, clip.start + clip.duration),
            );
        };
        let prop = |name: &str| clip.props.get(name);
        let generator = match &clip.content {
            Content::Generator { fx, params, .. } => Some((fx.as_str(), params)),
            _ => None,
        };
        let fx = generator.map(|(fx, _)| fx);
        let draws_nothing = matches!(fx, Some("camera" | "light" | "null"));
        if draws_nothing {
            if !clip.masks.is_empty() || !clip.effects.is_empty() {
                says("masks and effects on it do nothing, since it draws nothing".to_owned());
            }
        } else {
            if !is_set_true(prop("three_d")) {
                for name in ["z", "rotate_x", "rotate_y", "depth", "bevel"] {
                    if prop(name).is_some() {
                        says(format!("`{name}` does nothing without `three_d`"));
                    }
                }
            } else if ever_above_zero(prop("bevel")) && !ever_above_zero(prop("depth")) {
                says("`bevel` does nothing without a `depth` to cut".to_owned());
            }
            if prop("bevel_profile").is_some() && !ever_above_zero(prop("bevel")) {
                says("`bevel_profile` does nothing without a `bevel`".to_owned());
            }
            if let (
                Some(Param::Static(Value::Number(bevel))),
                Some(Param::Static(Value::Number(depth))),
            ) = (prop("bevel"), prop("depth"))
            {
                if *depth > 0.0 && *bevel > *depth / 2.0 {
                    says(format!(
                        "`bevel` stops at half its `depth`, {}",
                        depth / 2.0
                    ));
                }
            }
        }
        if prop("path").is_none() {
            for name in ["along", "orient"] {
                if prop(name).is_some() {
                    says(format!("`{name}` does nothing without a `path`"));
                }
            }
        } else if prop("along").is_none() {
            says("`path` holds at its first point until `along` moves".to_owned());
        }
        for (name, param) in clip.props.iter() {
            if let Param::Animated(keys) = param {
                if keys.last().is_some_and(|key| key.at > clip.duration) {
                    says(format!(
                        "`{name}` has a key past the clip's end, whose value is not reached"
                    ));
                }
            }
        }
        let Some((fx, params)) = generator else {
            continue;
        };
        match fx {
            "light" => {
                let kind = match params.get("kind") {
                    Some(Param::Static(Value::Enum(kind))) => kind.as_str(),
                    _ => "point",
                };
                let casts = kind == "point" || kind == "directional";
                if !casts && params.get("shadow").is_some() {
                    says(format!("`shadow` does nothing on a {kind} light"));
                }
                if !casts && params.get("size").is_some() {
                    says(format!("`size` does nothing on a {kind} light"));
                }
                if casts && params.get("size").is_some() && !ever_above_zero(params.get("shadow")) {
                    says("`size` softens a shadow, and there is no `shadow`".to_owned());
                }
                if kind != "environment" && params.get("image").is_some() {
                    says("`image` does nothing unless `kind=environment`".to_owned());
                }
                if !in_depth_over(clip) {
                    says("a light falls only on clips in depth, and none shows with it".to_owned());
                }
            }
            "title" => {
                if let Some(Param::Static(Value::Text(features))) = params.get("features") {
                    let font = match params.get("font") {
                        Some(Param::Static(Value::Text(font))) if !font.trim().is_empty() => font.as_str(),
                        _ => "sans-serif",
                    };
                    let (family, unread) = crate::text_lint::features_unread(font, features);
                    for tag in unread {
                        says(format!("`features`: {family} has no `{tag}`, which draws nothing (`look` `font` names the ones it has)"));
                    }
                }
            }
            "camera" => {
                if params.get("focus").is_some() && !ever_above_zero(params.get("aperture")) {
                    says("`focus` does nothing without an `aperture`".to_owned());
                }
                if ever_above_zero(params.get("aperture")) && !in_depth_over(clip) {
                    says("`aperture` blurs only clips in depth, and none shows with it".to_owned());
                }
            }
            _ => {}
        }
    }
}

/// What [`does_nothing`] calls what it finds.
const NO_EFFECT: &str = "no_effect";

fn is_set_true(param: Option<&Param>) -> bool {
    matches!(param, Some(Param::Static(Value::Bool(true))))
}

/// A number above 0 at some moment, so one that grows as it goes counts.
fn ever_above_zero(param: Option<&Param>) -> bool {
    match param {
        Some(Param::Static(Value::Number(number))) => *number > 0.0,
        Some(Param::Animated(keys)) => keys
            .iter()
            .any(|key| matches!(key.value, Value::Number(number) if number > 0.0)),
        // What an expression gives is not known without running it: take it that it does.
        Some(Param::Expr(_)) => true,
        _ => false,
    }
}

fn finding(code: &str, entity: &str, message: String) -> Finding {
    Finding::warning(code, entity, message)
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
            // An adjustment layer, a camera, a light, and a null draw nothing of their own.
            Content::Generator { fx, .. } => {
                !matches!(fx.as_str(), "adjustment" | "camera" | "light" | "null")
            }
            Content::Sequence { .. } => true,
            Content::Caption { .. } => false,
        })
}

/// Whether `clip` is laid over a picture and is not one that fills a frame: text, a shape,
/// particles. It leaves no edge of the frame black, and covers none that another does.
fn laid_over(clip: &Clip) -> bool {
    matches!(&clip.content, Content::Generator { fx, .. }
        if matches!(fx.as_str(), "title" | "shape" | "particles"))
}

/// Black stretches with no picture, and stretches where no picture covers
/// the whole frame.
fn picture(project: &Project, sequence: &Sequence, findings: &mut Vec<Finding>) {
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
        findings.push(
            finding(
                "picture.black",
                sequence.id.as_str(),
                format!(
                    "nothing is on screen in `{}` from {} to {}",
                    sequence.id,
                    clock.print_position(from),
                    clock.print_position(to)
                ),
            )
            .during(sequence, from, to),
        );
    }
    // A title, a shape, or particles over a picture hide none of the frame's edges, and alone
    // over nothing they are not a picture that was to fill it: the edges are of the others.
    let framed: Vec<&Clip> = shown
        .iter()
        .copied()
        .filter(|clip| !laid_over(clip))
        .collect();
    let covered = union(
        framed
            .iter()
            .filter(|clip| covers(project, sequence, clip))
            .map(span),
    );
    for (from, to) in subtract(&union(framed.iter().map(span)), &covered) {
        let short: Vec<&Clip> = framed
            .iter()
            .copied()
            .filter(|clip| clip.start < to && clip.end() > from)
            .filter(|clip| !covers(project, sequence, clip))
            .collect();
        let ids: Vec<&str> = short.iter().map(|clip| clip.id.as_str()).collect();
        findings.push(finding(
            "picture.edges",
            ids.first().copied().unwrap_or(sequence.id.as_str()),
            format!(
                "the frame's edges are black in `{}` from {} to {}: {} {} not fill the {}x{} frame{}",
                sequence.id,
                clock.print_position(from),
                clock.print_position(to),
                ids.iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if ids.len() == 1 { "does" } else { "do" },
                sequence.width,
                sequence.height,
                fills(project, sequence, &short),
            ),
        )
        .during(sequence, from, to));
    }
}

/// What to say after a finding that `short` do not fill the frame: the `scale` at which each
/// does. A picture is fit inside the frame before its `scale`, so what fills the frame is
/// seldom what the sizes alone say. Nothing for a clip that is placed otherwise.
fn fills(project: &Project, sequence: &Sequence, short: &[&Clip]) -> String {
    let scales: Vec<(&str, f64)> = short
        .iter()
        .filter_map(|clip| Some((clip.id.as_str(), fill_scale(project, sequence, clip)?)))
        .collect();
    let cut = "cutting off what is over evenly";
    match scales.as_slice() {
        [] => String::new(),
        [(id, scale)] if short.len() == 1 => format!("; `set {id} scale={scale}` fills it, {cut}"),
        // A cut of clips of one shape wants one number.
        [(_, scale), rest @ ..]
            if scales.len() == short.len() && rest.iter().all(|(_, other)| other == scale) =>
        {
            format!("; `scale={scale}` on each fills it, {cut}")
        }
        _ => format!(
            "; {} {} it, {cut}",
            scales
                .iter()
                .map(|(id, scale)| format!("`set {id} scale={scale}`"))
                .collect::<Vec<_>>()
                .join(", "),
            if scales.len() == 1 { "fills" } else { "fill" },
        ),
    }
}

/// The `scale`, to two places and no less, at which `clip`, centered and not turned, cropped,
/// or stretched, fills the frame: none for a clip that is placed otherwise, or whose picture's
/// size is not known.
fn fill_scale(project: &Project, sequence: &Sequence, clip: &Clip) -> Option<f64> {
    let (width, height) = match &clip.content {
        Content::Media { media, .. } => project
            .media(media.as_str())
            .and_then(|media| media.video)
            .map(|video| (video.width, video.height))?,
        Content::Sequence { sequence, .. } => project
            .sequence(sequence.as_str())
            .map(|nested| (nested.width, nested.height))?,
        Content::Generator { .. } | Content::Caption { .. } => return None,
    };
    let placed = [
        "x", "y", "scale_x", "scale_y", "rotation", "crop_l", "crop_r", "crop_t", "crop_b",
    ];
    if placed.iter().any(|name| clip.props.contains_key(*name)) || width == 0 || height == 0 {
        return None;
    }
    let (frame_width, frame_height) = (f64::from(sequence.width), f64::from(sequence.height));
    let (width, height) = (f64::from(width), f64::from(height));
    let fit = (frame_width / width).min(frame_height / height);
    let fills = (frame_width / (width * fit)).max(frame_height / (height * fit));
    Some((fills * 100.0).ceil() / 100.0)
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

fn flash_frames(sequence: &Sequence, findings: &mut Vec<Finding>) {
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
                findings.push(
                    finding(
                        "clip.flash",
                        clip.id.as_str(),
                        format!(
                            "clip `{}` on {} is {frames} frame{} long",
                            clip.id,
                            track.id,
                            if frames == 1 { "" } else { "s" }
                        ),
                    )
                    .during(sequence, clip.start, clip.end()),
                );
            }
        }
    }
}

/// Picture clips that show a frame or two of another shot at an edge: a
/// cut of their media's picture just inside the clip, which plays as a
/// flash frame.
fn slivers(project: &Project, sequence: &Sequence, facts: &dyn Facts, findings: &mut Vec<Finding>) {
    let rate = sequence.rate;
    let mut known: std::collections::HashMap<_, Option<Breaks>> = std::collections::HashMap::new();
    for clip in pictures(project, sequence) {
        let Content::Media { media, .. } = &clip.content else {
            continue;
        };
        let Some(media) = project.media(media.as_str()) else {
            continue;
        };
        let Some((low, high)) = clip.source_range() else {
            continue;
        };
        let Some(breaks) = known
            .entry(media.id.clone())
            .or_insert_with(|| facts.breaks(media))
        else {
            continue;
        };
        for at in breaks
            .cuts
            .iter()
            .filter_map(|cut| Time::from_seconds(*cut))
        {
            if at <= low || at >= high {
                continue;
            }
            let Some(played) = clip.played_at(at).map(|time| rate.round(time)) else {
                continue;
            };
            for (edge, frames, from, to) in [
                (
                    "starts",
                    rate.frames_round(played - clip.start),
                    clip.start,
                    played,
                ),
                (
                    "ends",
                    rate.frames_round(clip.end() - played),
                    played,
                    clip.end(),
                ),
            ] {
                if frames <= 0 || frames >= FLASH_FRAMES {
                    continue;
                }
                let clock = Clock::of_media(media);
                findings.push(finding(
                    "cut.near_scene_cut",
                    clip.id.as_str(),
                    format!(
                        "clip `{}` {edge} with {frames} frame{} of another shot (`{}` cuts at {}, by its index)",
                        clip.id,
                        if frames == 1 { "" } else { "s" },
                        media.id,
                        clock.print_position(media.rate.round(at)),
                    ),
                )
                .during(sequence, from, to));
            }
        }
    }
}

/// What cuts no transcript covers are called: information, since nothing is known to be wrong with
/// them, only that nobody looked.
pub(crate) const CUT_UNHEARD: &str = "cut.unheard";

/// Sound clips whose first or last frame falls inside a word; and, as information, the cuts that
/// could not be checked because no transcript of their media covers them, once for each media.
fn cuts_in_words(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    findings: &mut Vec<Finding>,
) {
    let mut known: std::collections::HashMap<_, Speech> = std::collections::HashMap::new();
    // For each media, the clips with a cut nothing heard, how many such cuts, and the first.
    let mut unheard: Vec<(&Id, Vec<&Id>, usize, Time)> = Vec::new();
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
                .or_insert_with(|| facts.speech(media));
            let clock = Clock::of_media(media);
            let frame = sequence.rate.frame_duration();
            for (end, at, cut) in [
                ("starts", source_in, clip.start),
                ("ends", source_out, clip.end() - frame),
            ] {
                // Nothing heard the media here: the cut is not checked, and that is said. A clip
                // that starts where its media does, or ends where it ends, cuts into nothing there.
                if !spoken.reaches(at.seconds()) {
                    if at > Time::ZERO && at < media.duration {
                        match unheard.iter_mut().find(|(id, ..)| *id == &media.id) {
                            Some((_, clips, cuts, _)) => {
                                if !clips.contains(&&clip.id) {
                                    clips.push(&clip.id);
                                }
                                *cuts += 1;
                            }
                            None => unheard.push((&media.id, vec![&clip.id], 1, cut)),
                        }
                    }
                    continue;
                }
                let at = at.seconds();
                let Some((from, to, word)) = spoken
                    .words
                    .iter()
                    .map(|word| (&word.start, &word.end, word.text.trim()))
                    .find(|(from, to, _)| *from + WORD_MARGIN < at && at < *to - WORD_MARGIN)
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
                )
                .during(sequence, cut, cut + frame));
            }
        }
    }
    let frame = sequence.rate.frame_duration();
    for (media, clips, cuts, first) in unheard {
        let named: Vec<String> = clips.iter().take(3).map(|id| format!("`{id}`")).collect();
        let more = match clips.len() - named.len() {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        let mut finding = Finding::new(
            Severity::Info,
            CUT_UNHEARD,
            Some(media.as_str()),
            format!(
                "{cuts} {} of `{media}` {} not checked for words, since no transcript covers {} ({}{more}); \
                 look at `{media}` with transcript, from, and to around {} first",
                if cuts == 1 { "cut" } else { "cuts" },
                if cuts == 1 { "was" } else { "were" },
                if cuts == 1 { "it" } else { "them" },
                named.join(", "),
                if cuts == 1 { "it" } else { "them" },
            ),
        )
        .during(sequence, first, first + frame);
        finding.value = Some(cuts as f64);
        finding.unit = Some("cuts".to_owned());
        findings.push(finding);
    }
}

/// What a stretch that sounds like speech and has no words is called: information, since a
/// transcriber leaving something out is not the cut's fault, and what reads as speech may be a beat.
pub(crate) const SPEECH_UNHEARD: &str = "speech.unheard";
/// How much of such a stretch a clip has to play for it to be said, in seconds.
const UNHEARD_PLAYED: f64 = 1.0;

/// Sound clips that play a stretch of their media that sounds like speech and that its
/// transcripts have no words for: what a caption or a cut made from the transcript would miss.
fn speech_unheard(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    findings: &mut Vec<Finding>,
) {
    let mut known: std::collections::HashMap<Id, Vec<(f64, f64)>> = std::collections::HashMap::new();
    let clock = Clock::of_sequence(sequence);
    for track in sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio && track.enabled && !track.muted)
    {
        for clip in track.clips.iter().filter(|clip| clip.enabled) {
            let Content::Media { media, speed, .. } = &clip.content else {
                continue;
            };
            let (Some(media), Some((source_in, source_out))) =
                (project.media(media.as_str()), clip.source_range())
            else {
                continue;
            };
            if speed.is_reverse() {
                continue;
            }
            let unheard = known
                .entry(media.id.clone())
                .or_insert_with(|| facts.unheard(media));
            for (from, to) in unheard.iter() {
                let (from, to) = (from.max(source_in.seconds()), to.min(source_out.seconds()));
                if to - from < UNHEARD_PLAYED {
                    continue;
                }
                let at = |seconds: f64| {
                    let source = Time::from_seconds(seconds).unwrap_or(source_in);
                    sequence.rate.round(clip.played_at(source).unwrap_or(clip.start))
                };
                let (start, end) = (at(from), at(to));
                let mut finding = Finding::new(
                    Severity::Info,
                    SPEECH_UNHEARD,
                    Some(clip.id.as_str()),
                    format!(
                        "clip `{}` sounds like speech from {} to {} and the transcript of `{}` has no words there; \
                         the transcriber may have missed it, so listen before trusting captions or cuts made from it",
                        clip.id,
                        clock.print_position(start),
                        clock.print_position(end),
                        media.id,
                    ),
                )
                .during(sequence, start, end);
                finding.value = Some(((to - from) * 10.0).round() / 10.0);
                finding.unit = Some("s".to_owned());
                findings.push(finding);
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
    use kerf_lang::{apply_script, parse_document, Word};
    use kerf_model::{Catalog, Media};

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

    /// What every media's transcripts heard and its index found.
    #[derive(Default)]
    struct Known {
        words: Vec<Word>,
        /// The seconds of each media those transcripts cover.
        covered: Vec<(f64, f64)>,
        cuts: Vec<f64>,
        /// The seconds of each media that sound like speech and have no words.
        unheard: Vec<(f64, f64)>,
    }

    impl Facts for Known {
        fn speech(&self, _: &Media) -> Speech {
            Speech {
                words: self.words.clone(),
                covered: self.covered.clone(),
            }
        }

        fn breaks(&self, _: &Media) -> Option<Breaks> {
            Some(Breaks {
                pauses: Vec::new(),
                cuts: self.cuts.clone(),
            })
        }

        fn unheard(&self, _: &Media) -> Vec<(f64, f64)> {
            self.unheard.clone()
        }
    }

    fn codes(script: &str, words: &[(f64, f64, &str)]) -> Vec<(String, String)> {
        found(script, words, &[])
    }

    /// With a transcript of each media, whole.
    fn found(script: &str, words: &[(f64, f64, &str)], cuts: &[f64]) -> Vec<(String, String)> {
        heard(script, words, cuts, &[(0.0, 60.0)])
    }

    fn heard(
        script: &str,
        words: &[(f64, f64, &str)],
        cuts: &[f64],
        covered: &[(f64, f64)],
    ) -> Vec<(String, String)> {
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let project = apply_script(&project, catalog, script).unwrap().project;
        let known = Known {
            words: words
                .iter()
                .map(|(start, end, text)| Word {
                    start: *start,
                    end: *end,
                    text: (*text).to_owned(),
                })
                .collect(),
            covered: covered.to_vec(),
            cuts: cuts.to_vec(),
            unheard: Vec::new(),
        };
        findings(&project, &known)
            .into_iter()
            .map(|finding| (finding.code, finding.message))
            .collect()
    }

    #[test]
    fn speech_a_transcript_has_no_words_for_is_said_where_a_clip_plays_it() {
        // a1 plays 0–4 s of m1 and a2 plays 10–14 s of it at 4–8 s of the sequence. The transcripts
        // have no words for 2.5–3.75 s, 9–11.5 s and 30–40 s of what sounds like speech.
        let catalog = Catalog::builtin();
        let project = parse_document(DOCUMENT, catalog).unwrap().project;
        let known = Known {
            covered: vec![(0.0, 60.0)],
            unheard: vec![(2.5, 3.75), (9.0, 11.5), (30.0, 40.0)],
            ..Known::default()
        };
        let found: Vec<Finding> = findings(&project, &known)
            .into_iter()
            .filter(|finding| finding.code == SPEECH_UNHEARD)
            .collect();
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].severity, Severity::Info);
        assert_eq!(found[0].entity.as_deref(), Some("a1"));
        assert!(
            found[0]
                .message
                .starts_with("clip `a1` sounds like speech from 00:00:02:12 to 00:00:03:18 and the transcript of `m1` has no words there"),
            "{found:?}"
        );
        // Of 9–11.5 s, a2 plays what is after 10 s, a second and a half, from where it starts.
        assert_eq!(found[1].entity.as_deref(), Some("a2"));
        assert!(found[1].message.contains("from 00:00:04:00 to 00:00:05:12"), "{found:?}");
        assert_eq!(found[1].value, Some(1.5));
        // What no clip plays, or plays less than a second of, is nobody's to hear.
        let little = Known {
            covered: vec![(0.0, 60.0)],
            unheard: vec![(3.5, 4.5), (30.0, 40.0)],
            ..Known::default()
        };
        assert!(findings(&project, &little).iter().all(|finding| finding.code != SPEECH_UNHEARD));
        // It is information: the check is as valid as it was.
        let report = crate::qc::CheckReport::new(findings(&project, &known), crate::qc::ROWS);
        assert!(report.valid);
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
    fn the_scale_that_fills_the_frame_is_said_with_one_clip() {
        let found = codes("sequence set s1 size=1080x1920\nset c2 scale=3.17", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        let (_, scale) = found[0]
            .1
            .split_once("`set c1 scale=")
            .unwrap_or_else(|| panic!("{found:?}"));
        let scale = scale.split_once('`').unwrap().0;
        assert!(
            found[0]
                .1
                .ends_with("fills it, cutting off what is over evenly"),
            "{found:?}"
        );
        // What it says fills it.
        let filled = codes(
            &format!("sequence set s1 size=1080x1920\nset c2 scale=3.17\nset c1 scale={scale}"),
            &[],
        );
        assert!(filled.is_empty(), "{filled:?}");
        // One placed otherwise is told no number.
        let placed = codes(
            "sequence set s1 size=1080x1920\nset c2 scale=3.17\nset c1 x=0.1",
            &[],
        );
        assert!(!placed[0].1.contains("scale="), "{placed:?}");
    }

    #[test]
    fn a_title_over_a_picture_hides_none_of_its_edges() {
        // c1 does not fill the tall frame, and a title is over its first two seconds.
        let found = codes(
            "sequence set s1 size=1080x1920\nset c2 scale=3.17\ntrack add V2 video\n\
             overwrite gen=title at=00:00:00:00 dur=00:00:02:00 on=V2 as=t1\nset t1 text=\"Hi\"",
            &[],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "picture.edges");
        // All of c1 is said, not what the title leaves of it, and with the scale that fills it.
        assert!(
            found[0]
                .1
                .contains("from 00:00:00:00 to 00:00:04:00: `c1` does not fill"),
            "{found:?}"
        );
        assert!(
            found[0].1.contains("`set c1 scale=3.17` fills it"),
            "{found:?}"
        );
    }

    #[test]
    fn a_title_over_nothing_is_a_picture_and_leaves_no_edge() {
        // A title card: nothing under it was to fill the frame, and the screen is not black.
        let found = codes(
            "delete c1\ntrack add V2 video\n\
             overwrite gen=title at=00:00:00:00 dur=00:00:04:00 on=V2 as=t1\nset t1 text=\"Hi\"",
            &[],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn what_draws_nothing_does_not_keep_the_screen_from_being_black() {
        let found = codes(
            "delete c1\ntrack add V2 video\n\
             overwrite gen=camera at=00:00:00:00 dur=00:00:08:00 on=V2 as=cam",
            &[],
        );
        let black: Vec<_> = found
            .iter()
            .filter(|(code, _)| code == "picture.black")
            .collect();
        assert_eq!(black.len(), 1, "{found:?}");
        assert!(
            black[0].1.contains("from 00:00:00:00 to 00:00:04:00"),
            "{found:?}"
        );
    }

    #[test]
    fn the_scale_that_fills_the_frame_is_said_for_several_clips() {
        // Two clips of one shape want one number.
        let found = codes("sequence set s1 size=1080x1920", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .1
                .contains("`c1`, `c2` do not fill the 1080x1920 frame"),
            "{found:?}"
        );
        assert!(
            found[0]
                .1
                .ends_with("; `scale=3.17` on each fills it, cutting off what is over evenly"),
            "{found:?}"
        );
        // Two of different shapes, each its own: a square picture fills a tall frame at 1.78.
        let found = codes(
            "media add m2 path=\"b.mov\" rate=24 dur=00:01:00:00 video=1080x1080\n\
             sequence set s1 size=1080x1920\n\
             overwrite media=m2 at=00:00:04:00 dur=00:00:04:00 on=V1 as=sq",
            &[],
        );
        let edges: Vec<_> = found
            .iter()
            .filter(|(code, _)| code == "picture.edges")
            .collect();
        assert_eq!(edges.len(), 1, "{found:?}");
        assert!(
            edges[0]
                .1
                .contains("; `set c1 scale=3.17`, `set sq scale=1.78` fill it, cutting off"),
            "{found:?}"
        );
        // What they say fills it.
        let filled = codes(
            "media add m2 path=\"b.mov\" rate=24 dur=00:01:00:00 video=1080x1080\n\
             sequence set s1 size=1080x1920\n\
             overwrite media=m2 at=00:00:04:00 dur=00:00:04:00 on=V1 as=sq\n\
             set c1 scale=3.17\nset sq scale=1.78",
            &[],
        );
        assert!(
            !filled.iter().any(|(code, _)| code == "picture.edges"),
            "{filled:?}"
        );
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
    fn a_sliver_of_another_shot_is_a_flash_frame() {
        // c1 plays m1 from 0s to 4s; the picture cuts a frame after its
        // start and a frame before its end, and in the middle of c2.
        let found = found("set c1 opacity=1", &[], &[0.05, 3.95, 12.0]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|(code, _)| code == "cut.near_scene_cut"));
        assert!(
            found[0].1.contains(
                "clip `c1` starts with 1 frame of another shot (`m1` cuts at 00:00:00:01"
            ),
            "{found:?}"
        );
        assert!(
            found[1].1.contains("clip `c1` ends with 1 frame"),
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

    #[test]
    fn cuts_no_transcript_covers_are_said_not_to_be_checked() {
        // Nothing has heard m1: a1 ends inside it at 4s, and a2 starts at 10s and ends at 14s.
        // That a1 starts where m1 does is no cut into it.
        let found = heard("set c1 opacity=1", &[], &[], &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "cut.unheard");
        assert_eq!(
            found[0].1,
            "3 cuts of `m1` were not checked for words, since no transcript covers them \
             (`a1`, `a2`); look at `m1` with transcript, from, and to around them first"
        );
        // A transcript of its first five seconds checks the one cut it reaches, and finds it in a
        // word; the other two are still said.
        let found = heard(
            "set c1 opacity=1",
            &[(3.8, 4.3, "country")],
            &[],
            &[(0.0, 5.0)],
        );
        let codes: Vec<&str> = found.iter().map(|(code, _)| code.as_str()).collect();
        assert_eq!(codes, ["cut.in_word", "cut.unheard"], "{found:?}");
        assert!(
            found[1]
                .1
                .starts_with("2 cuts of `m1` were not checked for words"),
            "{found:?}"
        );
        assert!(found[1].1.contains("(`a2`)"), "{found:?}");
        // One that covers them all leaves nothing to say.
        assert!(heard("set c1 opacity=1", &[], &[], &[(0.0, 60.0)]).is_empty());
        // A clip of the whole of its media has no cut into it.
        let whole = heard("delete a2\ntrim a1 out to=00:01:00:00", &[], &[], &[]);
        assert!(
            !whole.iter().any(|(code, _)| code == "cut.unheard"),
            "{whole:?}"
        );
    }
}
