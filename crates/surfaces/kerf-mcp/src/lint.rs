//! What an editor would catch in a cut that validation does not: black
//! frames, picture that leaves the frame's edges uncovered, flash frames
//! (including a sliver of another shot at a clip's edge), sound cut
//! inside a word or through a sound, pauses and repeated words the cut
//! still plays (see `cuts`), and text and captions a viewer cannot read
//! (see `text_lint`).

use crate::cuts::{self, Known};
use crate::qc::{Finding, Severity};
use crate::text_lint;
use kerf_lang::{Breaks, Clock, Facts};
use kerf_model::{Clip, Content, Id, Param, Project, Sequence, Time, TrackKind, Value};

/// A picture or sound clip shorter than this many frames is a flash frame.
const FLASH_FRAMES: i64 = 3;
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
        let mut known = Known::new(facts);
        cuts_in_sound(project, sequence, &mut known, &mut findings);
        leftovers(project, sequence, &mut known, &mut findings);
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
        let draws_nothing = matches!(fx, Some("camera" | "light" | "null" | kerf_model::SIGNAL));
        if draws_nothing {
            if !clip.masks.is_empty() || !clip.effects.is_empty() {
                says("masks and effects on it do nothing, since it draws nothing".to_owned());
            }
        } else if fx == Some("mesh") {
            // It is in depth without saying so, and is a solid of its own.
            for name in ["depth", "bevel", "bevel_profile"] {
                if prop(name).is_some() {
                    says(format!(
                        "`{name}` does nothing on a mesh, whose `length` is how far back it goes"
                    ));
                }
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
            // An adjustment layer, a camera, a light, a null, and a signal draw nothing of
            // their own.
            Content::Generator { fx, .. } => !matches!(
                fx.as_str(),
                "adjustment" | "camera" | "light" | "null" | kerf_model::SIGNAL
            ),
            Content::Sequence { .. } => true,
            Content::Caption { .. } => false,
        })
}

/// Whether `clip` is laid over a picture and is not one that fills a frame: text, a shape,
/// particles, a mesh. It leaves no edge of the frame black, and covers none that another does.
fn laid_over(clip: &Clip) -> bool {
    matches!(&clip.content, Content::Generator { fx, .. }
        if matches!(fx.as_str(), "title" | "shape" | "particles" | "mesh"))
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

/// What a cut through a sound no transcript has a word for is called.
pub(crate) const CUT_IN_SOUND: &str = "cut.in_sound";

/// Sound clips whose first or last frame falls inside a word, by a transcript, or in the middle of
/// a sound, by the levels their media's index kept; and, as information, the cuts that could not be
/// checked because no transcript of their media covers them and its sound was not measured, once
/// for each media.
///
/// Where the two disagree the levels are right: a cut in a pause is not said to be in the word a
/// transcript stretches over it, and a cut through a breath or a word a transcriber missed is said
/// though no word is there.
fn cuts_in_sound(
    project: &Project,
    sequence: &Sequence,
    known: &mut Known<'_>,
    findings: &mut Vec<Finding>,
) {
    // For each media, the clips with a cut nothing heard, how many such cuts, and the first.
    let mut unheard: Vec<(&Id, Vec<&Id>, usize, Time)> = Vec::new();
    let frame = sequence.rate.frame_duration();
    for cut in cuts::sound_cuts(project, sequence) {
        let (clip, media, end) = (cut.clip, cut.media, cut.end.verb());
        let judged = cuts::judge(&cut, known);
        let clock = Clock::of_media(media);
        let position = |seconds: f64| {
            let time = Time::from_seconds(seconds).unwrap_or(Time::ZERO);
            clock.print_position(clock.rate.round(time))
        };
        if let Some(word) = &judged.word {
            findings.push(
                finding(
                    "cut.in_word",
                    clip.id.as_str(),
                    format!(
                        "clip `{}` {end} inside the word \"{}\" ({}–{} of `{}`, by its transcript)",
                        clip.id,
                        word.text.trim(),
                        position(word.start),
                        position(word.end),
                        media.id,
                    ),
                )
                .during(sequence, cut.frame, cut.frame + frame),
            );
        } else if let (true, Some(edge), Some(sound)) =
            (judged.in_sound(), judged.edge, known.sound(media))
        {
            let mut finding = finding(
                CUT_IN_SOUND,
                clip.id.as_str(),
                format!(
                    "clip `{}` {end} in the middle of a sound{}: {:.0} dB at {} of `{}`, which speaks at \
                     {:.0} dB and pauses under {:.0} dB; trim it with snap=pause to move the cut into \
                     the pause beside it",
                    clip.id,
                    if judged.transcribed {
                        " its transcript has no word for"
                    } else {
                        ""
                    },
                    edge.through(),
                    position(cut.source.seconds()),
                    media.id,
                    sound.speech(),
                    sound.quiet_below(),
                ),
            )
            .during(sequence, cut.frame, cut.frame + frame);
            finding.value = Some((edge.through() * 10.0).round() / 10.0);
            finding.unit = Some("dB".to_owned());
            findings.push(finding);
        } else if judged.unchecked() {
            // Nothing heard the media here: the cut is not checked, and that is said.
            match unheard.iter_mut().find(|(id, ..)| *id == &media.id) {
                Some((_, clips, cuts, _)) => {
                    if !clips.contains(&&clip.id) {
                        clips.push(&clip.id);
                    }
                    *cuts += 1;
                }
                None => unheard.push((&media.id, vec![&clip.id], 1, cut.frame)),
            }
        }
    }
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

/// What a pause the sequence still plays between two words is called, and a word it plays twice
/// in a row: information, since a pause may be meant and a word may have been said twice.
pub(crate) const PAUSE_KEPT: &str = "pause.kept";
pub(crate) const WORD_REPEATED: &str = "word.repeated";
/// What a phrase the sequence plays and takes again a moment later is called, with other words
/// between its two sayings or a word more or less in the second: information, as a repeat is.
pub(crate) const PHRASE_RESTARTED: &str = "phrase.restarted";
/// How many pauses or repeats one finding names.
const NAMED: usize = 4;

/// What the sound of `sequence` still plays, by the transcripts of its media: the pauses of three
/// quarters of a second or more between words, and the words and phrases said twice in a row.
/// A cut reads the footage and not what it leaves, so a wait it trimmed too little, a second take
/// it kept, and a word doubled across one of its own cuts are said here. Each repeat across a cut
/// is a finding, since the cut may have made it; the rest are counted in one.
fn leftovers(
    project: &Project,
    sequence: &Sequence,
    known: &mut Known<'_>,
    findings: &mut Vec<Finding>,
) {
    known.hear(project, sequence);
    let words = cuts::played_words(project, sequence, known);
    if words.is_empty() {
        return;
    }
    let clock = Clock::of_sequence(sequence);
    let at = |time: Time| clock.print_position(sequence.rate.round(time));
    let and_more = |named: usize, all: usize| match all - named {
        0 => String::new(),
        more => format!("; and {more} more"),
    };

    let pauses = cuts::kept_pauses(project, sequence, &words, known, cuts::KEPT_PAUSE_SECONDS);
    if let Some(longest) = pauses.first() {
        let named: Vec<String> = pauses
            .iter()
            .take(NAMED)
            .map(|pause| {
                format!(
                    "{:.1} s after \"{}\" at {} {}",
                    pause.seconds,
                    pause.after.text,
                    at(pause.after.end),
                    if pause.at_cut() {
                        format!(
                            "across the cut from `{}` to `{}`",
                            pause.after.clip.id, pause.before.clip.id
                        )
                    } else {
                        format!("in `{}`", pause.after.clip.id)
                    },
                )
            })
            .collect();
        let total: f64 = pauses.iter().map(|pause| pause.seconds).sum();
        let mut finding = Finding::new(
            Severity::Info,
            PAUSE_KEPT,
            Some(sequence.id.as_str()),
            format!(
                "`{}` plays {} of {} s or more between words, {total:.1} s in all: {}{}",
                sequence.id,
                match pauses.len() {
                    1 => "1 pause".to_owned(),
                    count => format!("{count} pauses"),
                },
                cuts::KEPT_PAUSE_SECONDS,
                named.join("; "),
                and_more(named.len(), pauses.len()),
            ),
        )
        .during(sequence, longest.after.end, longest.before.start);
        finding.value = Some((total * 10.0).round() / 10.0);
        finding.unit = Some("s".to_owned());
        findings.push(finding);
    }

    let repeats = cuts::repeats(&words);
    let (at_cuts, said): (Vec<_>, Vec<_>) = repeats.iter().partition(|repeat| repeat.at_cut());
    for repeat in at_cuts {
        let (first, last) = (&repeat.words[0], &repeat.words[repeat.words.len() - 1]);
        findings.push(
            Finding::new(
                Severity::Info,
                WORD_REPEATED,
                Some(last.clip.id.as_str()),
                format!(
                    "\"{}\" is said twice in a row at {}, across the cut from `{}` to `{}`: the cut \
                     may have kept two takes of it",
                    repeat.text(),
                    at(first.start),
                    first.clip.id,
                    last.clip.id,
                ),
            )
            .during(sequence, first.start, last.end),
        );
    }
    if let Some(first) = said.first() {
        let named: Vec<String> = said
            .iter()
            .take(NAMED)
            .map(|repeat| {
                format!(
                    "\"{}\" at {} in `{}`",
                    repeat.text(),
                    at(repeat.words[0].start),
                    repeat.words[0].clip.id
                )
            })
            .collect();
        let mut finding = Finding::new(
            Severity::Info,
            WORD_REPEATED,
            Some(sequence.id.as_str()),
            format!(
                "`{}` plays {} twice in a row, as it was said: {}{}",
                sequence.id,
                match said.len() {
                    1 => "1 word or phrase".to_owned(),
                    count => format!("{count} words or phrases"),
                },
                named.join("; "),
                and_more(named.len(), said.len()),
            ),
        )
        .during(
            sequence,
            first.words[0].start,
            first.words[first.words.len() - 1].end,
        );
        finding.value = Some(said.len() as f64);
        finding.unit = Some("repeats".to_owned());
        findings.push(finding);
    }

    // A phrase taken again a moment later, the same way: each across a cut by itself, since
    // the cut may have left both takes in, and the rest, which were said so, counted in one.
    let restarts = cuts::restarts(&words);
    let (at_cuts, said): (Vec<_>, Vec<_>) =
        restarts.iter().partition(|restart| restart.at_cut());
    let quoted = |words: &[cuts::Played<'_>]| {
        let said: Vec<&str> = words.iter().map(|word| word.text).collect();
        said.join(" ")
    };
    for restart in at_cuts {
        let (first, last) = (
            &restart.first[0],
            &restart.second[restart.second.len() - 1],
        );
        findings.push(
            Finding::new(
                Severity::Info,
                PHRASE_RESTARTED,
                Some(last.clip.id.as_str()),
                format!(
                    "\"{}\" is said at {} and taken again at {}, across the cut from `{}` to `{}`: \
                     the cut may have kept two takes of it",
                    quoted(restart.first),
                    at(first.start),
                    at(restart.second[0].start),
                    first.clip.id,
                    last.clip.id,
                ),
            )
            .during(sequence, first.start, last.end),
        );
    }
    if let Some(first) = said.first() {
        let named: Vec<String> = said
            .iter()
            .take(NAMED)
            .map(|restart| {
                format!(
                    "\"{}\" at {} in `{}`",
                    quoted(restart.first),
                    at(restart.first[0].start),
                    restart.first[0].clip.id
                )
            })
            .collect();
        let mut finding = Finding::new(
            Severity::Info,
            PHRASE_RESTARTED,
            Some(sequence.id.as_str()),
            format!(
                "`{}` plays {} that {} taken again a moment later, as said: {}{}",
                sequence.id,
                match said.len() {
                    1 => "1 phrase".to_owned(),
                    count => format!("{count} phrases"),
                },
                if said.len() == 1 { "is" } else { "are" },
                named.join("; "),
                and_more(named.len(), said.len()),
            ),
        )
        .during(
            sequence,
            first.first[0].start,
            first.second[first.second.len() - 1].end,
        );
        finding.value = Some(said.len() as f64);
        finding.unit = Some("restarts".to_owned());
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
    use kerf_lang::{apply_script, parse_document, Sound, Speech, Word};
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
        /// How loud each media is, a hundredth of a second at a time.
        sound: Option<Sound>,
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
                gaps: Vec::new(),
                cuts: self.cuts.clone(),
            })
        }

        fn unheard(&self, _: &Media) -> Vec<(f64, f64)> {
            self.unheard.clone()
        }

        fn sound(&self, _: &Media) -> Option<Sound> {
            self.sound.clone()
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
            ..Known::default()
        };
        findings(&project, &known)
            .into_iter()
            .map(|finding| (finding.code, finding.message))
            .collect()
    }

    fn words(words: &[(f64, f64, &str)]) -> Vec<Word> {
        words
            .iter()
            .map(|(start, end, text)| Word {
                start: *start,
                end: *end,
                text: (*text).to_owned(),
            })
            .collect()
    }

    /// A minute in a room at -55 dB where somebody speaks at -20 dB for the first 1.2 s of every
    /// two, and besides from `loud` to `quiet` seconds of it, each `(from, to)`.
    fn recording(loud: &[(f64, f64)], quiet: &[(f64, f64)]) -> Sound {
        let mut levels: Vec<f32> = (0..6000)
            .map(|hop| if hop % 200 < 120 { -20.0 } else { -55.0 })
            .collect();
        for (stretches, level) in [(loud, -20.0), (quiet, -55.0)] {
            for (from, to) in stretches {
                levels[(from * 100.0).round() as usize..(to * 100.0).round() as usize].fill(level);
            }
        }
        Sound::new(0.01, levels)
    }

    fn project() -> Project {
        parse_document(DOCUMENT, Catalog::builtin())
            .unwrap()
            .project
    }

    fn said(known: &Known) -> Vec<(String, String)> {
        findings(&project(), known)
            .into_iter()
            .map(|finding| (finding.code, finding.message))
            .collect()
    }

    #[test]
    fn a_cut_between_two_words_said_in_one_breath_is_in_neither() {
        // Somebody speaks up to 0.06 s before a1 ends at 4 s of m1 and again 0.06 s after: the
        // two words part for an eighth of a second, too short for a pause, and the cut is in it.
        let run_on = || Some(recording(&[(3.2, 3.94)], &[(4.0, 4.06)]));
        // A transcript ends the first word a tenth of a second late, past the cut. The cut
        // takes nothing off it: the word does not go through the gap, and `snap=pause` would
        // have put a cut there.
        let loose = Known {
            words: words(&[(3.5, 4.1, "country"), (4.1, 4.6, "side")]),
            covered: vec![(0.0, 60.0)],
            sound: run_on(),
            ..Known::default()
        };
        assert!(said(&loose).is_empty(), "{:?}", said(&loose));
        // One word timed right through the gap holds a consonant there, and is cut through.
        let held = Known {
            words: words(&[(3.5, 4.5, "countryside")]),
            covered: vec![(0.0, 60.0)],
            sound: run_on(),
            ..Known::default()
        };
        let found = said(&held);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "cut.in_word");
        // A quiet as short as a stopped consonant's is no gap: the cut is in the word a
        // transcript has there, as before.
        let stopped = Known {
            words: words(&[(3.5, 4.1, "country"), (4.1, 4.6, "side")]),
            covered: vec![(0.0, 60.0)],
            sound: Some(recording(&[(3.2, 3.97)], &[(4.0, 4.03)])),
            ..Known::default()
        };
        assert_eq!(said(&stopped)[0].0, "cut.in_word");
    }

    #[test]
    fn a_cut_is_judged_by_the_sound_where_a_transcript_is_loose_or_has_nothing() {
        // a1 ends at 4 s of m1, a2 starts at 10 s of it and ends at 14 s. The room is quiet for
        // 0.8 s before each of those, and somebody starts to speak at each.
        // A transcript stretches "country" over the pause a1 ends in: the cut is in the pause.
        let loose = Known {
            words: words(&[(3.6, 4.3, "country")]),
            covered: vec![(0.0, 60.0)],
            sound: Some(recording(&[], &[])),
            ..Known::default()
        };
        assert!(said(&loose).is_empty(), "{:?}", said(&loose));
        // Without the sound the transcript is all there is to go by.
        let unmeasured = Known {
            sound: None,
            ..loose
        };
        assert_eq!(said(&unmeasured)[0].0, "cut.in_word");
        // A word where the sound says there is one is still cut through.
        let through = Known {
            words: words(&[(3.6, 4.3, "country")]),
            covered: vec![(0.0, 60.0)],
            sound: Some(recording(&[(3.2, 4.0)], &[])),
            ..Known::default()
        };
        assert_eq!(said(&through)[0].0, "cut.in_word");

        // Somebody is already speaking where a2 starts, and the transcript has no word there.
        let missed = Known {
            covered: vec![(0.0, 60.0)],
            sound: Some(recording(&[(9.8, 10.0)], &[])),
            ..Known::default()
        };
        let found = said(&missed);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "cut.in_sound");
        assert_eq!(
            found[0].1,
            "clip `a2` starts in the middle of a sound its transcript has no word for: -20 dB at \
             00:00:10:00 of `m1`, which speaks at -20 dB and pauses under -47 dB; trim it with \
             snap=pause to move the cut into the pause beside it"
        );
        // With no transcript at all the sound still checks every cut: the one is found, and the
        // other two are not said to be unchecked.
        let untranscribed = Known {
            covered: Vec::new(),
            ..missed
        };
        let found = said(&untranscribed);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .1
                .starts_with("clip `a2` starts in the middle of a sound: -20 dB"),
            "{found:?}"
        );
        // A sound that holds one level judges nothing, and the cuts are said to be unchecked.
        let flat = Known {
            sound: Some(Sound::new(0.01, vec![-30.0; 6000])),
            ..Known::default()
        };
        assert_eq!(said(&flat)[0].0, "cut.unheard");
        // The two halves of a split play on: no cut is heard between them.
        let catalog = Catalog::builtin();
        let split = apply_script(&project(), catalog, "split a2 at=00:00:05:00")
            .unwrap()
            .project;
        let speaking = Known {
            words: words(&[(10.8, 11.3, "country")]),
            covered: vec![(0.0, 60.0)],
            sound: Some(recording(&[], &[])),
            ..Known::default()
        };
        assert!(findings(&split, &speaking).is_empty());
    }

    #[test]
    fn pauses_and_repeated_words_the_cut_still_plays_are_said() {
        // a1 plays "we went to" and stops 1.1 s later; a2 starts 0.2 s before the same words.
        let known = Known {
            words: words(&[
                (2.0, 2.3, "We"),
                (2.3, 2.6, "went"),
                (2.6, 2.9, "to"),
                (10.2, 10.5, "we"),
                (10.5, 10.8, "went"),
                (10.8, 11.0, "to"),
                (11.0, 11.2, "the"),
                (11.2, 11.6, "store."),
                (12.0, 12.2, "So"),
                (12.3, 12.5, "so"),
                (12.5, 12.9, "that."),
                (13.0, 13.4, "That"),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let found = said(&known);
        let codes: Vec<&str> = found.iter().map(|(code, _)| code.as_str()).collect();
        assert_eq!(
            codes,
            ["pause.kept", "word.repeated", "word.repeated"],
            "{found:?}"
        );
        assert_eq!(
            found[0].1,
            "`s1` plays 1 pause of 0.75 s or more between words, 1.3 s in all: 1.3 s after \"to\" \
             at 00:00:02:22 across the cut from `a1` to `a2`"
        );
        assert_eq!(
            found[1].1,
            "\"We went to we went to\" is said twice in a row at 00:00:02:00, across the cut from \
             `a1` to `a2`: the cut may have kept two takes of it"
        );
        // "that. That" ends one sentence and starts the next.
        assert_eq!(
            found[2].1,
            "`s1` plays 1 word or phrase twice in a row, as it was said: \"So so\" at 00:00:06:00 in `a2`"
        );
        // Sound in the pause is not a pause: somebody speaks from 3 s to 3.6 s of m1, which the
        // transcript has no words for.
        let spoken = Known {
            sound: Some(recording(
                &[(3.0, 3.6)],
                &[(2.9, 3.0), (3.6, 4.0), (10.0, 10.2)],
            )),
            ..known
        };
        let found = said(&spoken);
        assert!(
            !found.iter().any(|(code, _)| code == "pause.kept"),
            "{found:?}"
        );
    }

    #[test]
    fn cuts_are_counted_for_a_look() {
        let known = Known {
            words: words(&[
                (13.7, 14.3, "country"),
                (2.0, 2.3, "We"),
                (11.0, 11.4, "went"),
            ]),
            covered: vec![(0.0, 60.0)],
            // a1 ends 0.03 s after somebody stops, a2 starts in a sound and ends in a word.
            sound: Some(recording(&[(3.2, 3.97), (9.8, 10.0), (13.2, 14.0)], &[])),
            ..Known::default()
        };
        let project = project();
        let report = cuts::report(
            &project,
            &project.sequences[0],
            &known,
            &cuts::Asked::default(),
        );
        assert_eq!(
            report["summary"],
            serde_json::json!({
                "cuts": 3, "quiet": 1, "low": 0, "sound": 2, "inWords": 1, "tight": 1,
                "faded": 0, "notMeasured": 0, "air": {"least": 0.03, "median": 0.03},
            }),
            "{report:#}"
        );
        assert_eq!(
            report["cuts"],
            serde_json::json!([
                {"clip": "a1", "end": "ends", "at": "00:00:03:23", "media": "m1",
                 "source": "00:00:04:00", "heard": "quiet", "before": -55.0, "after": -20.0,
                 "air": 0.03},
                {"clip": "a2", "end": "starts", "at": "00:00:04:00", "media": "m1",
                 "source": "00:00:10:00", "heard": "sound", "before": -20.0, "after": -20.0},
                {"clip": "a2", "end": "ends", "at": "00:00:07:23", "media": "m1",
                 "source": "00:00:14:00", "heard": "sound", "before": -20.0, "after": -20.0,
                 "word": "country"},
            ]),
            "{report:#}"
        );
        assert_eq!(
            report["media"],
            serde_json::json!([{"media": "m1", "speech": -20.0, "quietBelow": -47.0}])
        );
        // Somebody speaks through what lies between the two words the transcript has.
        assert_eq!(report["pauses"]["count"], 0, "{report:#}");
        assert_eq!(report["repeats"]["count"], 0, "{report:#}");
        // Without a transcript the cuts are still measured, and the rest is said to need one.
        let report = cuts::report(
            &project,
            &project.sequences[0],
            &Known {
                sound: Some(recording(&[], &[])),
                ..Known::default()
            },
            &cuts::Asked::default(),
        );
        assert_eq!(report["summary"]["quiet"], 3, "{report:#}");
        // a2 starts on the very hop somebody starts to speak: nothing is cut through, and no
        // air is left before it.
        assert_eq!(report["summary"]["tight"], 1, "{report:#}");
        assert_eq!(report["cuts"].as_array().unwrap().len(), 1, "{report:#}");
        assert_eq!(report["cuts"][0]["air"], 0.0, "{report:#}");
        assert!(report["words"].as_str().unwrap().contains("no transcript"));
    }

    fn cuts_of(known: &Known, asked: &cuts::Asked) -> serde_json::Value {
        let project = project();
        cuts::report(&project, &project.sequences[0], known, asked)
    }

    #[test]
    fn a_join_says_what_it_plays_and_what_it_took() {
        // a1 stops at 4 s of m1 and a2 goes on from 10 s of it: the cut took "a very simple"
        // out of "I gave it a very simple prompt", which measures clean and reads wrong.
        let known = Known {
            words: words(&[
                (2.0, 2.4, "I"),
                (2.4, 2.9, "gave"),
                (3.0, 3.4, "it"),
                (4.2, 4.6, "a"),
                (4.6, 5.0, "very"),
                (5.0, 5.5, "simple"),
                (10.2, 10.6, "prompt"),
                (10.6, 11.0, "and"),
                (11.0, 11.4, "waited."),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let report = cuts_of(&known, &cuts::Asked::default());
        assert_eq!(
            report["joins"],
            serde_json::json!({"count": 1, "first": [{
                "at": "00:00:04:00", "from": "a1", "to": "a2",
                "plays": "I gave it ‖ prompt and waited.",
                "took": "a very simple", "tookSeconds": 6.0,
            }]}),
            "{report:#}"
        );
        // A long stretch is given by its ends, and one no transcript has words for by its
        // length alone.
        let long = Known {
            words: words(
                &(0..14)
                    .map(|at| (4.1 + 0.4 * f64::from(at), 4.4 + 0.4 * f64::from(at), "la"))
                    .chain([(10.2, 10.6, "prompt")])
                    .collect::<Vec<_>>(),
            ),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let report = cuts_of(&long, &cuts::Asked::default());
        assert_eq!(
            report["joins"]["first"][0]["took"],
            "la la la la la la … la la la la la la"
        );
        assert_eq!(report["joins"]["first"][0]["plays"], " ‖ prompt");
        let unheard = cuts_of(&Known::default(), &cuts::Asked::default());
        assert_eq!(
            unheard["joins"]["first"][0],
            serde_json::json!({"at": "00:00:04:00", "from": "a1", "to": "a2",
                "took": "", "tookSeconds": 6.0}),
            "{unheard:#}"
        );
        // Two halves of a split are no join.
        let project = apply_script(&project(), Catalog::builtin(), "split a2 at=00:00:05:00")
            .unwrap()
            .project;
        let report = cuts::report(
            &project,
            &project.sequences[0],
            &known,
            &cuts::Asked::default(),
        );
        assert_eq!(report["joins"]["count"], 1, "{report:#}");
    }

    #[test]
    fn a_phrase_taken_again_is_said_with_what_lies_between() {
        // a2 plays from 10 s of m1 at 4 s of the sequence.
        let known = Known {
            words: words(&[
                (10.0, 10.2, "there's"),
                (10.2, 10.4, "this"),
                (10.4, 10.6, "like"),
                (10.6, 10.9, "white,"),
                (10.9, 11.0, "I"),
                (11.0, 11.2, "don't"),
                (11.2, 11.5, "know,"),
                (11.6, 11.8, "there's"),
                (11.8, 12.0, "this"),
                (12.0, 12.2, "like"),
                (12.2, 12.5, "white"),
                (12.5, 12.9, "stuff."),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        // The check says it, as information.
        let found = said(&known);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, PHRASE_RESTARTED);
        assert_eq!(
            found[0].1,
            "`s1` plays 1 phrase that is taken again a moment later, as said: \"there's this \
             like white,\" at 00:00:04:00 in `a2`"
        );
        let report = cuts_of(&known, &cuts::Asked::default());
        assert_eq!(
            report["restarts"],
            serde_json::json!({"count": 1, "atCuts": 0, "first": [{
                "at": "00:00:04:00", "said": "there's this like white,",
                "again": "there's this like white", "againAt": "00:00:05:14",
                "between": "I don't know,", "clip": "a2",
            }]}),
            "{report:#}"
        );
        assert_eq!(report["repeats"]["count"], 0, "{report:#}");
        // Across a cut it is said by itself: a1 stops after one take and a2 has the next.
        let two_takes = Known {
            words: words(&[
                (1.0, 1.2, "so"),
                (1.2, 1.5, "this"),
                (1.5, 1.8, "is"),
                (1.8, 2.2, "the"),
                (2.2, 2.6, "plan"),
                (10.2, 10.4, "okay"),
                (10.4, 10.7, "this"),
                (10.7, 11.0, "is"),
                (11.0, 11.3, "the"),
                (11.3, 11.7, "plan"),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let found = said(&two_takes);
        let codes: Vec<&str> = found.iter().map(|(code, _)| code.as_str()).collect();
        assert_eq!(codes, ["pause.kept", "phrase.restarted"], "{found:?}");
        assert_eq!(
            found[1].1,
            "\"this is the plan\" is said at 00:00:01:05 and taken again at 00:00:04:10, across \
             the cut from `a1` to `a2`: the cut may have kept two takes of it"
        );
        let report = cuts_of(&two_takes, &cuts::Asked::default());
        assert_eq!(report["restarts"]["atCuts"], 1, "{report:#}");
        assert_eq!(report["restarts"]["first"][0]["cutTo"], "a2");
        // The second saying may have a word more, or less, than the first.
        let slipped = Known {
            words: words(&[
                (10.0, 10.3, "how"),
                (10.3, 10.8, "mechanical"),
                (10.8, 11.2, "watch,"),
                (11.3, 11.5, "how"),
                (11.5, 11.6, "a"),
                (11.6, 12.1, "mechanical"),
                (12.1, 12.5, "watch"),
                (12.5, 12.9, "works."),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let report = cuts_of(&slipped, &cuts::Asked::default());
        assert_eq!(report["restarts"]["count"], 1, "{report:#}");
        assert_eq!(
            report["restarts"]["first"][0]["said"],
            "how mechanical watch,"
        );
        assert_eq!(
            report["restarts"]["first"][0]["again"],
            "how a mechanical watch"
        );
        // Two words said again further on are how anybody speaks, and a phrase said twice
        // at once is a repeat, which is said as one.
        let spoken = Known {
            words: words(&[
                (10.0, 10.2, "you"),
                (10.2, 10.4, "can"),
                (10.4, 10.7, "see"),
                (10.7, 10.9, "it"),
                (10.9, 11.1, "and"),
                (11.1, 11.3, "you"),
                (11.3, 11.5, "can"),
                (11.5, 11.9, "hear"),
                (11.9, 12.1, "it."),
                (12.2, 12.4, "we"),
                (12.4, 12.6, "went"),
                (12.6, 12.8, "to"),
                (12.8, 13.0, "we"),
                (13.0, 13.2, "went"),
                (13.2, 13.4, "to"),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let report = cuts_of(&spoken, &cuts::Asked::default());
        assert_eq!(report["restarts"]["count"], 0, "{report:#}");
        assert_eq!(report["repeats"]["count"], 1, "{report:#}");
    }

    #[test]
    fn a_report_is_of_the_part_and_the_pauses_it_is_asked_for() {
        // Half a second between two words a1 plays, 0.8 s between two a2 plays, and 2.2 s
        // across the cut between them.
        let known = Known {
            words: words(&[
                (1.0, 1.4, "So"),
                (1.9, 2.3, "we"),
                (10.5, 10.9, "went"),
                (11.7, 12.1, "home."),
            ]),
            covered: vec![(0.0, 60.0)],
            ..Known::default()
        };
        let whole = cuts_of(&known, &cuts::Asked::default());
        assert_eq!(whole["pauses"]["from"], 0.75);
        assert_eq!(whole["pauses"]["count"], 2, "{whole:#}");
        assert_eq!(whole["summary"]["cuts"], 3, "{whole:#}");
        // What is dead air is the edit's to say.
        let tighter = cuts_of(
            &known,
            &cuts::Asked {
                pauses: 0.4,
                range: None,
            },
        );
        assert_eq!(tighter["pauses"]["count"], 3, "{tighter:#}");
        assert_eq!(tighter["pauses"]["longest"][2]["after"], "So");
        // The second half of the sequence: a2's cuts and its pause, and no join, which is
        // at the frame before.
        let second = Time::from_seconds(4.0).unwrap();
        let part = cuts_of(
            &known,
            &cuts::Asked {
                pauses: 0.4,
                range: Some((second + sequence_frame(), Time::from_seconds(8.0).unwrap())),
            },
        );
        assert_eq!(part["summary"]["cuts"], 1, "{part:#}");
        assert_eq!(part["pauses"]["count"], 1, "{part:#}");
        assert_eq!(part["joins"]["count"], 0, "{part:#}");
        assert_eq!(part["from"], "00:00:04:01");
    }

    fn sequence_frame() -> Time {
        project().sequences[0].rate.frame_duration()
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
