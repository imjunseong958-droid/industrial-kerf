//! What an editor would catch in a cut that validation does not: black
//! frames, picture that leaves the frame's edges uncovered, flash frames
//! (including a sliver of another shot at a clip's edge, and a picture the
//! tracks over it leave showing for a frame or two), sound cut inside a
//! word or through a sound, pauses and repeated words the cut still plays,
//! a word it took with a restart (see `cuts`), a picture and a sound of one
//! thing that play out of step, and text and captions a viewer cannot read,
//! or that no installed font can draw (see `text_lint`).

use crate::cuts::{self, Known};
use crate::qc::{Finding, Severity};
use crate::text_lint;
use kerf_lang::{Breaks, Clock, Facts};
use kerf_model::{
    Catalog, Clip, Content, Id, Media, Param, Project, Rate, Sequence, Time, Track, TrackKind,
    Value,
};
use std::collections::{BTreeMap, HashMap};

/// A picture or sound clip shorter than this many frames is a flash frame, and so is a
/// picture that shows for fewer.
const FLASH_FRAMES: i64 = 3;
/// Pixels of frame edge a picture may leave uncovered without a finding.
const EDGE_TOLERANCE: f64 = 0.5;

/// What the index of each media found of it, looked up once.
type Indexed = HashMap<Id, Option<Breaks>>;

/// Findings for every sequence of `project`, whose media paths are
/// resolved. `facts` knows what transcripts and indexes already made of its
/// media heard and found; nothing is analysed for a check.
pub(crate) fn findings(project: &Project, facts: &dyn Facts) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut indexed = Indexed::new();
    // What is laid over what is read as it is drawn: a look as the keys and effects it stands
    // for. One that does not expand is left as the effect it is, of which nothing is known.
    let drawn = kerf_model::look::expand(project, Catalog::builtin()).ok();
    let drawn = drawn.as_ref().unwrap_or(project);
    for sequence in &project.sequences {
        let said = picture(project, sequence, &mut findings);
        flash_frames(sequence, &mut findings);
        slivers(project, sequence, facts, &mut indexed, &mut findings);
        transitions_into_other_shots(project, sequence, facts, &mut indexed, &mut findings);
        if let Some(sequence) = drawn.sequence(sequence.id.as_str()) {
            picture_flashes(drawn, sequence, facts, &mut indexed, &said, &mut findings);
        }
        out_of_step(project, sequence, &mut findings);
        let mut known = Known::new(facts);
        cuts_in_sound(project, sequence, &mut known, &mut findings);
        leftovers(project, sequence, &mut known, &mut findings);
        speech_unheard(project, sequence, facts, &mut findings);
        text_lint::text(sequence, &mut findings);
        text_lint::undrawable(sequence, &mut findings);
        text_lint::captions(sequence, &mut findings);
        does_nothing(sequence, &mut findings);
    }
    findings
}

/// Numbers and choices set where nothing reads them, which validation has no reason to refuse and which
/// show nothing in a frame. As information: a person may mean to come back to them.
fn does_nothing(sequence: &Sequence, findings: &mut Vec<Finding>) {
    let clips = || sequence.tracks.iter().flat_map(|track| &track.clips);
    // A mesh among them: it is in depth without saying so.
    let in_depth_over = |clip: &Clip| {
        clips().any(|other| {
            other.id != clip.id
                && other.in_depth()
                && other.start < clip.start + clip.duration
                && clip.start < other.start + other.duration
        })
    };
    let parents = |clip| parents_of(sequence, clip);
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
            if fx == Some("null") {
                // A null is somewhere in depth for what moves with it, and for nothing else.
                if prop("three_d").is_some() {
                    says("`three_d` does nothing on a null, which takes its children in depth with it without it".to_owned());
                }
                let leads = clips().any(|other| {
                    follows_in_depth(other) && parents(other).any(|parent| parent.id == clip.id)
                });
                for name in IN_DEPTH {
                    if prop(name).is_some() && !leads {
                        says(format!(
                            "`{name}` on a null moves the clips in depth parented to it, and none is"
                        ));
                    }
                }
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
                && *depth > 0.0
                && *bevel > *depth / 2.0
            {
                says(format!(
                    "`bevel` stops at half its `depth`, {}",
                    depth / 2.0
                ));
            }
        }
        // A flat clip keeps to the frame where a parent goes off into depth, which is easy to
        // take for a rig that does not work. An adjustment layer is nowhere to be taken from.
        if !draws_nothing && !clip.in_depth() && fx != Some("adjustment") {
            let led = parents(clip).find(|parent| {
                leads_in_depth(parent)
                    && IN_DEPTH.iter().any(|name| parent.props.contains_key(*name))
            });
            if let Some(parent) = led {
                let set: Vec<String> = IN_DEPTH
                    .iter()
                    .filter(|name| parent.props.contains_key(**name))
                    .map(|name| format!("`{name}`"))
                    .collect();
                says(format!(
                    "the {} of `{}`, which it moves with, does nothing to it: only a clip in depth follows a parent there (`three_d=true`)",
                    set.join(", "),
                    parent.id
                ));
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
            if let Param::Animated(keys) = param
                && keys.last().is_some_and(|key| key.at > clip.duration)
            {
                says(format!(
                    "`{name}` has a key past the clip's end, whose value is not reached"
                ));
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
                        Some(Param::Static(Value::Text(font))) if !font.trim().is_empty() => {
                            font.as_str()
                        }
                        _ => "sans-serif",
                    };
                    let style = match params.get("style") {
                        Some(Param::Static(Value::Text(style))) => style.as_str(),
                        _ => "",
                    };
                    let weight = match params.get("weight") {
                        Some(Param::Static(Value::Number(weight))) => *weight,
                        _ => 0.0,
                    };
                    let style = kerf_model::title_face(style, weight);
                    let (family, unread) =
                        crate::text_lint::features_unread(font, &style, features);
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

/// The properties that say where a clip is in depth, which a parent hands on to its children there.
const IN_DEPTH: [&str; 3] = ["z", "rotate_x", "rotate_y"];

fn generator(clip: &Clip) -> Option<&str> {
    match &clip.content {
        Content::Generator { fx, .. } => Some(fx.as_str()),
        _ => None,
    }
}

/// The clips a clip moves with, from the one it names outward: no further than there are clips,
/// so that a document validation has not yet refused does not go round for ever.
fn parents_of<'a>(sequence: &'a Sequence, clip: &'a Clip) -> impl Iterator<Item = &'a Clip> {
    let clips = || sequence.tracks.iter().flat_map(|track| &track.clips);
    let mut parent = clip.parent.as_ref();
    let mut left = clips().count();
    std::iter::from_fn(move || {
        let id = parent?;
        left = left.checked_sub(1)?;
        let found = clips().find(|other| other.id == *id)?;
        parent = found.parent.as_ref();
        Some(found)
    })
}

/// Whether a clip's place in depth is its children's: a null's, and that of a clip in depth.
fn leads_in_depth(clip: &Clip) -> bool {
    generator(clip) == Some("null") || clip.in_depth()
}

/// Whether a clip goes where a parent in depth takes it: a clip in depth, a camera, and a light.
fn follows_in_depth(clip: &Clip) -> bool {
    clip.in_depth() || matches!(generator(clip), Some("camera" | "light"))
}

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
        .filter(move |clip| draws(project, clip))
}

/// Whether `clip` shows a picture where it is on a video track: it is enabled, and is not an
/// adjustment layer or media without one.
fn draws(project: &Project, clip: &Clip) -> bool {
    clip.enabled
        && match &clip.content {
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
        }
}

/// Whether `clip` is laid over a picture and is not one that fills a frame: text, a shape,
/// particles, a mesh. It leaves no edge of the frame black, and covers none that another does.
fn laid_over(clip: &Clip) -> bool {
    matches!(&clip.content, Content::Generator { fx, .. }
        if matches!(fx.as_str(), "title" | "shape" | "particles" | "mesh"))
}

/// Black stretches with no picture, and stretches where no picture covers
/// the whole frame. It gives back the stretches it said, which are not to be said again.
fn picture(
    project: &Project,
    sequence: &Sequence,
    findings: &mut Vec<Finding>,
) -> Vec<(Time, Time)> {
    let mut said = Vec::new();
    let duration = sequence.duration();
    if duration <= Time::ZERO
        || !sequence
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Video)
    {
        return said;
    }
    let clock = Clock::of_sequence(sequence);
    let shown: Vec<&Clip> = pictures(project, sequence).collect();
    let span = |clip: &&Clip| (clip.start, clip.end());
    let pictured = union(shown.iter().map(span));
    for (from, to) in complement(&pictured, duration) {
        said.push((from, to));
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
        said.push((from, to));
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
    said
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
        "x", "y", "scale_x", "scale_y", "rotation", "skew", "crop_l", "crop_r", "crop_t", "crop_b",
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
    let moves = PLACED.iter().any(|name| {
        clip.props
            .get(*name)
            .is_some_and(|param| !matches!(param, Param::Static(_)))
    });
    size.is_none_or(|size| moves || placed_over(sequence, clip, size, Time::ZERO).unwrap_or(true))
}

/// Whether a picture `width` by `height` covers the whole frame where `clip` puts it at `at`
/// of the clip, and `None` where that is not worked out: it is turned or slanted.
fn placed_over(
    sequence: &Sequence,
    clip: &Clip,
    (width, height): (u32, u32),
    at: Time,
) -> Option<bool> {
    let mut values = [0.0; 11];
    for (value, name) in values.iter_mut().zip(PLACED) {
        let default = if name.starts_with("scale") { 1.0 } else { 0.0 };
        *value = match clip.props.get(name) {
            None => default,
            Some(param) => param.value_at(at).as_number()?,
        };
    }
    let [x, y, scale, scale_x, scale_y, rotation, skew, left, right, top, bottom] = values;
    if rotation != 0.0 || skew != 0.0 {
        return None;
    }
    let (frame_width, frame_height) = (f64::from(sequence.width), f64::from(sequence.height));
    let cropped_width = f64::from(width) * (1.0 - left - right);
    let cropped_height = f64::from(height) * (1.0 - top - bottom);
    if cropped_width <= 0.0 || cropped_height <= 0.0 {
        return Some(false);
    }
    // The renderer fits the picture inside the frame, then scales it about
    // its center and offsets it by fractions of the frame.
    let fit = (frame_width / cropped_width).min(frame_height / cropped_height);
    let shown_width = cropped_width * fit * (scale * scale_x).abs();
    let shown_height = cropped_height * fit * (scale * scale_y).abs();
    let center_x = frame_width / 2.0 + x * frame_width;
    let center_y = frame_height / 2.0 + y * frame_height;
    Some(
        center_x - shown_width / 2.0 <= EDGE_TOLERANCE
            && center_x + shown_width / 2.0 >= frame_width - EDGE_TOLERANCE
            && center_y - shown_height / 2.0 <= EDGE_TOLERANCE
            && center_y + shown_height / 2.0 >= frame_height - EDGE_TOLERANCE,
    )
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
fn slivers(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    known: &mut Indexed,
    findings: &mut Vec<Finding>,
) {
    let rate = sequence.rate;
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

/// What a transition is called that plays frames of another shot from past where a clip ends
/// or from before where it starts, as its media's index has it.
pub(crate) const TRANSITION_OTHER_SHOT: &str = "transition.other_shot";

/// Transitions of a picture track that reach into another shot of a clip's media: what a
/// transition plays of the clip before the cut past its end, or of the clip after it before its
/// start, takes in a cut of that media's picture, so a frame or more of a shot the edit left out
/// shows through it. Each says how many, and what keeps it to the shots the clips play: the
/// transition put on the cut's other side, or no longer than the frames it has.
fn transitions_into_other_shots(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    known: &mut Indexed,
    findings: &mut Vec<Finding>,
) {
    let rate = sequence.rate;
    let tracks = sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video && track.enabled);
    for track in tracks {
        for transition in &track.transitions {
            let (Some(outgoing), Some(incoming)) = (
                track.clip(transition.from.as_str()),
                track.clip(transition.to.as_str()),
            ) else {
                continue;
            };
            let (before, after) = transition.split_at_cut(rate);
            // How many frames each side has before it reaches another shot, where it does, and
            // how many of what the transition plays of it are of that shot.
            let mut reach = |clip: &Clip, ends: bool| -> Option<(i64, i64, Time, &Media)> {
                let Content::Media { media, speed, .. } = &clip.content else {
                    return None;
                };
                if speed.is_reverse() {
                    return None;
                }
                let media = project.media(media.as_str())?;
                let breaks = known
                    .entry(media.id.clone())
                    .or_insert_with(|| facts.breaks(media))
                    .as_ref()?;
                let (low, high) = clip.source_range()?;
                // The clip as it would play on into the other's frames.
                let mut on = clip.clone();
                let edge = if ends {
                    on.move_tail(transition.duration);
                    clip.end()
                } else {
                    on.move_head(Time::ZERO - transition.duration);
                    clip.start
                };
                let cut = breaks
                    .cuts
                    .iter()
                    .filter_map(|cut| Time::from_seconds(*cut))
                    .filter(|at| if ends { *at >= high } else { *at <= low })
                    .filter_map(|at| Some((at, rate.round(on.played_at(at)?))))
                    .min_by_key(|(_, played)| (*played - edge).abs())?;
                let clean = rate.frames_round((cut.1 - edge).abs());
                let plays = rate.frames_round(if ends { after } else { before });
                Some((clean, plays - clean, cut.0, media))
            };
            let out = reach(outgoing, true);
            let into = reach(incoming, false);
            let total = rate.frames_round(transition.duration);
            let fits =
                |side: &Option<(i64, i64, Time, &Media)>| side.is_none_or(|side| side.0 >= total);
            for (side, clip, ends, other) in
                [(out, outgoing, true, into), (into, incoming, false, out)]
            {
                let Some((clean, shows, at, media)) = side else {
                    continue;
                };
                if shows <= 0 {
                    continue;
                }
                let longest = 2 * clean.min(other.map_or(clean, |other| other.0));
                let mend = if fits(&other) {
                    format!(
                        "align={} puts it all on the other side of the cut",
                        if ends { "end" } else { "start" }
                    )
                } else if longest > 0 {
                    format!("make it {longest} frames or shorter")
                } else {
                    "take it off, or move the cut".to_owned()
                };
                let clock = Clock::of_media(media);
                let (from, to) = if ends {
                    (clip.end(), clip.end() + after)
                } else {
                    (clip.start - before, clip.start)
                };
                let mut finding = finding(
                    TRANSITION_OTHER_SHOT,
                    transition.id.as_str(),
                    format!(
                        "transition `{}` plays {shows} frame{} of another shot {} `{}` {} (`{}` cuts \
                         at {}, by its index): {mend}",
                        transition.id,
                        if shows == 1 { "" } else { "s" },
                        if ends { "after" } else { "before" },
                        clip.id,
                        if ends { "ends" } else { "starts" },
                        media.id,
                        clock.print_position(media.rate.round(at)),
                    ),
                )
                .during(sequence, from, to);
                finding.value = Some(shows as f64);
                finding.unit = Some("frames".to_owned());
                findings.push(finding);
            }
        }
    }
}

/// What a picture that shows for a frame or two between what is over it and a cut is called.
pub(crate) const PICTURE_FLASH: &str = "picture.flash";

/// The categories of the catalog whose effects leave a picture as whole and as opaque as it
/// was: a grade, a blur, grain, a finish, a burn-in. Any other may cut it, key it, or move it.
const KEEPS_WHOLE: [&str; 5] = [
    "Color Correction",
    "Blur & Sharpen",
    "Noise & Grain",
    "Stylize",
    "Text",
];

/// Effects that only take away from a picture or draw at its edge: a keyer, a choker, a drop
/// shadow, a feather. A picture that does not cover the frame
/// without them does not with them. The first three are categories of the catalog.
const TRIMS: [&str; 4] = ["Keying", "Matte", "Perspective", "utility.edgefeather"];

/// How a clip lies over what is under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Over {
    /// It hides all of it.
    Hides,
    /// It leaves it showing, around it or through it.
    Shows,
    /// Whether it hides it is not known.
    Unknown,
}

/// A stretch of `clip` as a layer over what is under it, over which it lies one way.
#[derive(Clone, Copy)]
struct Layer<'a> {
    from: Time,
    to: Time,
    clip: &'a Clip,
    over: Over,
}

/// What a shot has at one end of it.
#[derive(Clone, Copy)]
enum Edge<'a> {
    /// A cut from or to the clip beside it on its track.
    Clip(&'a Clip),
    /// A cut in its media's picture, by the media's index, at that time of the media.
    Media(&'a Media, Time),
    /// Nothing beside it on its track: its clip starts or ends there.
    Open,
}

/// A stretch of one track's picture with no cut in it: a clip, the part of one between two
/// cuts of its media's picture, or several that a transition joins or that play on from one
/// another as the halves of a split do.
#[derive(Clone, Copy)]
struct Shot<'a> {
    from: Time,
    to: Time,
    enters: Edge<'a>,
    leaves: Edge<'a>,
}

/// Pictures that show for a frame or two because of what is over them, which nothing that
/// looks at one clip finds: an overlay that ends two frames before the cut under it shows two
/// frames of the shot that cut ends, and one that starts two frames after it, two of the shot
/// it starts, with every clip long enough.
///
/// The video tracks are laid over one another from the top, each clip read as it is drawn at
/// each frame ([`over_at`]), in `project` with its looks expanded. A shot shows wherever
/// nothing over it hides the whole frame, and a stretch of it shorter than [`FLASH_FRAMES`] is
/// a flash, said once whatever shows in it. So a clip that does not fill the frame leaves the
/// shot under it showing, and takes no flash from it.
///
/// Only what is certain is said. A fade and a transition are no cut, so a stretch that one of
/// them starts or ends is left; so is one under a clip that may or may not hide it; and so is
/// a shot that is itself that short, which is its clip's to say ([`flash_frames`],
/// [`slivers`]). Where nothing fills the frame for a frame or two and something laid over it
/// keeps [`picture`] from calling it black, that is said too; `said` are the stretches
/// [`picture`] did say.
fn picture_flashes(
    project: &Project,
    sequence: &Sequence,
    facts: &dyn Facts,
    indexed: &mut Indexed,
    said: &[(Time, Time)],
    findings: &mut Vec<Finding>,
) {
    let rate = sequence.rate;
    let clock = Clock::of_sequence(sequence);
    let tracks: Vec<&Track> = sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video && track.enabled)
        .collect();
    let layers: Vec<Vec<Layer>> = tracks
        .iter()
        .map(|track| layers(project, sequence, track))
        .collect();
    let hidden_by = |layers: &[Layer]| {
        union(
            layers
                .iter()
                .filter(|layer| layer.over == Over::Hides)
                .map(|layer| (layer.from, layer.to)),
        )
    };
    // How many frames a stretch is, where they are few enough to be a flash.
    let flash = |from: Time, to: Time| {
        Some(rate.frames_round(to - from)).filter(|frames| (1..FLASH_FRAMES).contains(frames))
    };
    // Each stretch that flashes: the clips it shows, from the top one down, and what is before
    // it and after it by the top one.
    let mut flashes: BTreeMap<(Time, Time), (Vec<&Id>, String, String)> = BTreeMap::new();
    for (index, track) in tracks.iter().enumerate().rev() {
        let over: Vec<Layer> = layers[index + 1..].iter().flatten().copied().collect();
        let hidden = hidden_by(&over);
        for shot in shots(project, track, rate, facts, indexed) {
            // A shot that is itself a flash is its clip's to say.
            if rate.frames_round(shot.to - shot.from) < FLASH_FRAMES {
                continue;
            }
            // Only what is hidden while the shot plays, of a long sequence's many stretches.
            let first = hidden.partition_point(|(_, to)| *to <= shot.from);
            let last = hidden.partition_point(|(from, _)| *from < shot.to);
            for (from, to) in subtract(&[(shot.from, shot.to)], &hidden[first..last]) {
                // The clip it shows, unless that is one said already for being as short.
                let shown = track
                    .clips
                    .iter()
                    .find(|clip| clip.start <= from && from < clip.end() && draws(project, clip))
                    .filter(|clip| rate.frames_round(clip.duration) >= FLASH_FRAMES);
                let (Some(_), Some(clip), Some((ends, starts))) =
                    (flash(from, to), shown, cut_by(&over, from, to))
                else {
                    continue;
                };
                // What stopped hiding it is what was seen before it, whatever its own track
                // has there.
                let before = match (ends, shot.enters) {
                    (Some(over), _) => format!("`{}` ending", over.id),
                    (None, Edge::Clip(other)) => format!("the cut from `{}`", other.id),
                    (None, Edge::Media(media, at)) => cut_in(media, at),
                    (None, Edge::Open) => format!("`{}` starting", clip.id),
                };
                let after = match (starts, shot.leaves) {
                    (Some(over), _) => format!("`{}` starting", over.id),
                    (None, Edge::Clip(other)) => format!("the cut to `{}`", other.id),
                    (None, Edge::Media(media, at)) => cut_in(media, at),
                    (None, Edge::Open) => format!("`{}` ending", clip.id),
                };
                flashes
                    .entry((from, to))
                    .or_insert((Vec::new(), before, after))
                    .0
                    .push(&clip.id);
            }
        }
    }
    // Where nothing fills the frame, it is black behind whatever else is on it.
    let layers: Vec<Layer> = layers.into_iter().flatten().collect();
    for (from, to) in complement(&hidden_by(&layers), sequence.duration()) {
        let known = said.iter().any(|(start, end)| *start < to && from < *end);
        let (Some(_), false, Some((ends, starts))) =
            (flash(from, to), known, cut_by(&layers, from, to))
        else {
            continue;
        };
        let before = ends.map_or_else(
            || format!("the start of `{}`", sequence.id),
            |clip| format!("`{}` ending", clip.id),
        );
        let after = starts.map_or_else(
            || format!("the end of `{}`", sequence.id),
            |clip| format!("`{}` starting", clip.id),
        );
        flashes
            .entry((from, to))
            .or_insert((Vec::new(), before, after));
    }
    for ((from, to), (clips, before, after)) in flashes {
        let frames = rate.frames_round(to - from);
        let shown = if clips.is_empty() {
            "black".to_owned()
        } else {
            clips
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        findings.push(
            finding(
                PICTURE_FLASH,
                clips.first().map_or(sequence.id.as_str(), |id| id.as_str()),
                format!(
                    "{frames} {} of {shown} {} at {}, between {before} and {after}",
                    if frames == 1 { "frame" } else { "frames" },
                    if frames == 1 { "shows" } else { "show" },
                    clock.print_position(from),
                ),
            )
            .during(sequence, from, to)
            .measured(frames as f64, FLASH_FRAMES as f64, "frames"),
        );
    }
}

/// What cuts to and from the stretch `from`–`to` among `over`, the clips that lie over it in
/// order from the lowest track up: the clip that stops hiding at `from` and the one that
/// starts hiding at `to`, the topmost of each.
///
/// Nothing where that is not certain. One of them stops or starts hiding at a moment that is
/// not its clip's own first or last frame: there it fades, or a transition takes it, and what
/// is under it comes up through it and is not cut to. Or a clip that may or may not hide is
/// over the stretch, and what is under it may not show at all.
fn cut_by<'a>(
    over: &[Layer<'a>],
    from: Time,
    to: Time,
) -> Option<(Option<&'a Clip>, Option<&'a Clip>)> {
    let hides = || over.iter().filter(|layer| layer.over == Over::Hides);
    let ends = || hides().filter(|layer| layer.to == from);
    let starts = || hides().filter(|layer| layer.from == to);
    let certain = ends().all(|layer| layer.to == layer.clip.end())
        && starts().all(|layer| layer.from == layer.clip.start)
        && !over
            .iter()
            .any(|layer| layer.over == Over::Unknown && layer.from < to && from < layer.to);
    certain.then(|| {
        (
            ends().next_back().map(|layer| layer.clip),
            starts().next_back().map(|layer| layer.clip),
        )
    })
}

/// How a cut in the picture of `media` at `at` of it is said.
fn cut_in(media: &Media, at: Time) -> String {
    format!(
        "a cut in its picture (`{}` cuts at {}, by its index)",
        media.id,
        Clock::of_media(media).print_position(media.rate.round(at)),
    )
}

/// How each clip of `track` lies over what is under it, in the order they play: a stretch for
/// each way it does, found a frame at a time where a property of it changes over time.
fn layers<'a>(project: &Project, sequence: &Sequence, track: &'a Track) -> Vec<Layer<'a>> {
    let rate = sequence.rate;
    let mut layers: Vec<Layer<'a>> = Vec::new();
    for clip in track.clips.iter().filter(|clip| draws(project, clip)) {
        let changes = clip.props.values().any(Param::varies)
            || matches!(&clip.content, Content::Generator { params, .. }
                if params.values().any(Param::varies));
        let frames = if changes {
            rate.frames_round(clip.duration)
        } else {
            1
        };
        let first = layers.len();
        for frame in 0..frames {
            let at = rate.time_of(frame);
            let over = over_at(project, sequence, clip, at);
            if layers[first..].last().is_none_or(|last| last.over != over) {
                if let Some(last) = layers[first..].last_mut() {
                    last.to = clip.start + at;
                }
                layers.push(Layer {
                    from: clip.start + at,
                    to: clip.end(),
                    clip,
                    over,
                });
            }
        }
    }
    // A transition mixes its two clips from before their cut to after it. Two that both hide
    // right up to the cut go on hiding through it. One that hides beside one that does not is
    // coming or going all that while, and what is under it shows through; beside one that may
    // or may not hide, whether it shows is not known either.
    for transition in &track.transitions {
        let (Some(from), Some(to)) = (
            track.clip(transition.from.as_str()),
            track.clip(transition.to.as_str()),
        ) else {
            continue;
        };
        let cut = from.end();
        let leaves = layers
            .iter()
            .find(|layer| layer.clip.id == from.id && layer.to == cut)
            .map(|layer| layer.over);
        let enters = layers
            .iter()
            .find(|layer| layer.clip.id == to.id && layer.from == cut)
            .map(|layer| layer.over);
        if (leaves, enters) == (Some(Over::Hides), Some(Over::Hides)) {
            continue;
        }
        let (before, after) = transition.split_at_cut(rate);
        for layer in layers.iter_mut().filter(|layer| layer.over == Over::Hides) {
            if layer.clip.id == from.id {
                layer.to = layer.to.min(cut - before);
            }
            if layer.clip.id == to.id {
                layer.from = layer.from.max(cut + after);
            }
        }
        if leaves == Some(Over::Unknown) || enters == Some(Over::Unknown) {
            layers.push(Layer {
                from: cut - before,
                to: cut + after,
                clip: from,
                over: Over::Unknown,
            });
        }
    }
    layers.retain(|layer| layer.from < layer.to);
    layers
}

/// The properties that place a picture on the frame, as [`placed_over`] reads them.
const PLACED: [&str; 11] = [
    "x", "y", "scale", "scale_x", "scale_y", "rotation", "skew", "crop_l", "crop_r", "crop_t",
    "crop_b",
];

/// How `clip` lies over what is under it at `at` of the clip, as the compositor draws it: its
/// picture fit inside the frame and cut by its masks and crop, its effects run on that, and
/// then scaled, moved, and laid on at its opacity (`kerf_compose::draw`).
///
/// It hides what is under it where it is a picture with no transparency of its own, or a
/// solid of a color with none, that covers the whole frame at full opacity and is laid
/// straight over it; a picture left where fitting puts it is padded to the frame with black,
/// and hides as much. It leaves it showing where it is seen through or does not cover the
/// frame: at less than full opacity, blended, a picture with transparency, text, a shape,
/// particles, a mesh, and a picture scaled down or moved aside, whatever is cut from it.
///
/// The benefit of the doubt goes the other way from [`covers`], which lets a picture that
/// moves pass for one that fills the frame. Here what is not worked out is not known: a clip
/// that is turned, moved by a parent or along a path, or in depth, one over the whole frame
/// with a mask or an effect that may cut or key it, one with an effect that may move or
/// redraw it, and a nested sequence, which is transparent wherever it has nothing and is not
/// read for where that is.
fn over_at(project: &Project, sequence: &Sequence, clip: &Clip, at: Time) -> Over {
    let prop = |name: &str| clip.props.get(name);
    let opacity = prop("opacity").map_or(Some(1.0), |param| param.value_at(at).as_number());
    let blended = prop("blend").is_some_and(
        |blend| !matches!(blend, Param::Static(Value::Enum(mode)) if mode == "normal"),
    );
    if laid_over(clip) || blended || opacity.is_some_and(|opacity| opacity < 1.0) {
        return Over::Shows;
    }
    let (size, media) = match &clip.content {
        Content::Media { media, .. } => {
            match project.media(media.as_str()).and_then(|media| media.video) {
                Some(video) if video.alpha => return Over::Shows,
                Some(video) => ((video.width, video.height), true),
                None => return Over::Unknown,
            }
        }
        // A solid is the size of the frame, and is placed as a picture is.
        Content::Generator { fx, params, .. } if fx.as_str() == "solid" => {
            match params.get("color").map(|color| color.value_at(at)) {
                Some(Value::Color(color)) if color.a < u8::MAX => return Over::Shows,
                None | Some(Value::Color(_)) => ((sequence.width, sequence.height), false),
                Some(_) => return Over::Unknown,
            }
        }
        _ => return Over::Unknown,
    };
    let moved = opacity.is_none()
        || clip.in_depth()
        || clip.parent.is_some()
        || ["path", "anchor_x", "anchor_y"]
            .iter()
            .any(|name| prop(name).is_some());
    let effects_of = |kinds: &[&str]| {
        clip.effects
            .iter()
            .filter(|effect| effect.enabled)
            .all(|effect| {
                kinds.contains(&effect.fx.as_str())
                    || Catalog::builtin()
                        .fx(effect.fx.as_str())
                        .is_some_and(|fx| kinds.contains(&fx.category.as_str()))
            })
    };
    let whole = clip.masks.is_empty() && effects_of(&KEEPS_WHOLE);
    match placed_over(sequence, clip, size, at) {
        _ if moved => Over::Unknown,
        Some(true) if whole => Over::Hides,
        Some(false) if media && whole && padded(clip) => Over::Hides,
        Some(false) if effects_of(&[KEEPS_WHOLE.as_slice(), &TRIMS].concat()) => Over::Shows,
        _ => Over::Unknown,
    }
}

/// Whether the compositor pads the picture of `clip` to the frame with black where it does
/// not fill it, as it does opaque media with no mask left where fitting puts it, flipped or
/// cropped as it may be (`letterboxed` in `kerf_compose::draw`).
fn padded(clip: &Clip) -> bool {
    let fixed = |name: &str, default: f64| match clip.props.get(name) {
        None => Some(default),
        Some(Param::Static(Value::Number(number))) => Some(*number),
        Some(_) => None,
    };
    let unit = |axis: &str| {
        fixed("scale", 1.0)
            .zip(fixed(axis, 1.0))
            .is_some_and(|(scale, factor)| ((scale * factor).abs() - 1.0).abs() < 5e-7)
    };
    clip.masks.is_empty()
        && ["x", "y", "rotation", "skew", "anchor_x", "anchor_y"]
            .iter()
            .all(|name| fixed(name, 0.0) == Some(0.0))
        && unit("scale_x")
        && unit("scale_y")
}

/// The shots of `track` in the order they play.
fn shots<'a>(
    project: &'a Project,
    track: &'a Track,
    rate: Rate,
    facts: &dyn Facts,
    indexed: &mut Indexed,
) -> Vec<Shot<'a>> {
    let mut shots = Vec::new();
    let mut open: Option<Shot<'a>> = None;
    let mut before: Option<&'a Clip> = None;
    for clip in track.clips.iter().filter(|clip| draws(project, clip)) {
        let beside = before.filter(|before| before.end() == clip.start);
        let joined = beside.is_some_and(|before| {
            plays_on(before, clip)
                || track
                    .transitions
                    .iter()
                    .any(|transition| transition.from == before.id && transition.to == clip.id)
        });
        let mut shot = match open.take() {
            Some(shot) if joined => shot,
            last => {
                shots.extend(last.map(|last| Shot {
                    leaves: beside.map_or(Edge::Open, |_| Edge::Clip(clip)),
                    ..last
                }));
                Shot {
                    from: clip.start,
                    to: clip.end(),
                    enters: beside.map_or(Edge::Open, Edge::Clip),
                    leaves: Edge::Open,
                }
            }
        };
        for (at, cut) in cuts_within(project, clip, rate, facts, indexed) {
            shots.push(Shot {
                to: at,
                leaves: cut,
                ..shot
            });
            shot = Shot {
                from: at,
                enters: cut,
                ..shot
            };
        }
        open = Some(Shot {
            to: clip.end(),
            ..shot
        });
        before = Some(clip);
    }
    shots.extend(open);
    shots
}

/// The cuts of its media's picture that `clip` plays, by the media's index: the frame of the
/// sequence each falls on, in the order they play.
fn cuts_within<'a>(
    project: &'a Project,
    clip: &Clip,
    rate: Rate,
    facts: &dyn Facts,
    indexed: &mut Indexed,
) -> Vec<(Time, Edge<'a>)> {
    let Content::Media { media, .. } = &clip.content else {
        return Vec::new();
    };
    let (Some(media), Some((low, high))) = (project.media(media.as_str()), clip.source_range())
    else {
        return Vec::new();
    };
    let Some(breaks) = indexed
        .entry(media.id.clone())
        .or_insert_with(|| facts.breaks(media))
    else {
        return Vec::new();
    };
    let mut cuts: Vec<(Time, Edge<'a>)> = breaks
        .cuts
        .iter()
        .filter_map(|cut| Time::from_seconds(*cut))
        .filter(|at| low < *at && *at < high)
        .filter_map(|at| Some((rate.round(clip.played_at(at)?), Edge::Media(media, at))))
        .filter(|(played, _)| clip.start < *played && *played < clip.end())
        .collect();
    cuts.sort_by_key(|(played, _)| *played);
    cuts.dedup_by_key(|(played, _)| *played);
    cuts
}

/// Whether `next` plays on from where `clip` stops, as the two halves of a split do: no cut
/// is seen between them.
fn plays_on(clip: &Clip, next: &Clip) -> bool {
    let same = match (&clip.content, &next.content) {
        (Content::Media { media, .. }, Content::Media { media: other, .. }) => media == other,
        (
            Content::Sequence { sequence, .. },
            Content::Sequence {
                sequence: other, ..
            },
        ) => sequence == other && clip.angle == next.angle,
        _ => false,
    };
    let (Some((low, high)), Some((next_low, next_high))) =
        (clip.source_range(), next.source_range())
    else {
        return false;
    };
    same && clip.speed() == next.speed()
        && if clip.speed().is_reverse() {
            low == next_high
        } else {
            high == next_low
        }
}

/// What a clip that plays out of step with the one it belongs with is called.
pub(crate) const SYNC_SLIPPED: &str = "sync.slipped";

/// Clips that play one thing at once and play it more than a frame apart.
///
/// Two clips play one thing where they play one media or one nested sequence, or two media the
/// document says are recordings of one thing (`sync=`, which says how far into the one the
/// other begins). Of one file, a picture and a sound that are linked belong together, as they
/// were laid. Of two
/// files, a camera and the recorder beside it or two cameras, clips belong together where they
/// are linked, and where they play some of the same moment at once whether linked or not. Two
/// files that play at once and further apart than the time they share play different moments,
/// which is a choice and no slip; so are a picture and a sound of one file that are not linked,
/// a voice over other pictures of its own take, and two pictures of one file that are, which
/// is a way to keep shots together.
///
/// Read from the document alone: where each clip starts and what it starts on. Two clips of one
/// file were made on the same frames, so any distance is the edit's; two files are each placed
/// on a whole frame from an offset finer than frames, which leaves them up to a frame apart
/// when they are in step, and that much is not said of either. A clip played at another speed
/// is not held to anything.
fn out_of_step(project: &Project, sequence: &Sequence, findings: &mut Vec<Finding>) {
    struct Plays<'a> {
        clip: &'a Clip,
        picture: bool,
        /// The media it plays, or the nested sequence.
        media: &'a Id,
        /// The recording its media is lined up with in the end, and how far into that it begins.
        of: &'a Id,
        begins: Time,
        /// Where on the sequence the first moment of that recording plays, or would.
        zero: Time,
    }
    let plays: Vec<Plays<'_>> = sequence
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
        .filter(|(_, clip)| clip.enabled && clip.speed().is_one())
        .filter_map(|(track, clip)| {
            let (media, source_in, (of, begins)) = match &clip.content {
                Content::Media {
                    media, source_in, ..
                } => (media, source_in, project.synced_to(media.as_str())?),
                // A nested sequence is one thing as a file is, with a picture and a sound.
                Content::Sequence {
                    sequence,
                    source_in,
                    ..
                } => (sequence, source_in, (sequence, Time::ZERO)),
                Content::Generator { .. } | Content::Caption { .. } => return None,
            };
            Some(Plays {
                clip,
                picture: track.kind == TrackKind::Video,
                media,
                of,
                begins,
                zero: clip.start - (*source_in + begins),
            })
        })
        .collect();
    let frame = sequence.rate.frame_duration();
    let linked = |a: &Plays<'_>, b: &Plays<'_>| a.clip.link.is_some() && a.clip.link == b.clip.link;
    let mut says = |slipped: &Plays<'_>, held_to: &Plays<'_>, linked: bool| {
        let late = slipped.zero - held_to.zero;
        let frames = late.abs().seconds() / frame.seconds();
        let (id, other) = (&slipped.clip.id, &held_to.clip.id);
        // To a tenth, and never the one frame that is let pass.
        let frames = ((frames * 10.0).round() / 10.0).max(1.1);
        let apart = format!("{frames} frames");
        let way = if late.is_positive() {
            "later"
        } else {
            "earlier"
        };
        // Other source frames in the same place undo it, or the same frames in another place.
        let by = sequence.rate.frames_round(late);
        let back = format!("`slip {id} {by:+}f` or `move {id} by={:+}f`", -by);
        let message =
            if slipped.media == held_to.media {
                format!(
                    "clip `{id}` plays `{}` {apart} {way} than `{other}`, which it is linked to: \
                 they are out of step. Linked clips slip and move together, so `unlink {id}` \
                 first; then {back} brings it back, to link again",
                    slipped.media,
                )
            } else {
                let into = (slipped.begins - held_to.begins).seconds();
                let into = (into * 1000.0).round() / 1000.0;
                format!(
                "clip `{id}` plays {apart} {way} than `{other}`, which {}: `{}` begins {} s {} \
                 `{}` (`sync=`), and they play what both recorded that far out of step. {} {back} \
                 brings it back",
                if linked { "it is linked to" } else { "plays beside it" },
                slipped.media,
                into.abs(),
                if into < 0.0 { "before" } else { "into" },
                held_to.media,
                if linked {
                    format!("Linked clips slip and move together, so `unlink {id}` first; then")
                } else {
                    "Either of".to_owned()
                },
            ) + if linked { ", to link again" } else { "" }
            };
        // Said where both play.
        let (from, to) = (
            slipped.clip.start.max(held_to.clip.start),
            slipped.clip.end().min(held_to.clip.end()),
        );
        findings.push(
            finding(SYNC_SLIPPED, id.as_str(), message)
                .during(sequence, from, to)
                .measured(frames, 1.0, "frames"),
        );
    };
    // Each clip against those on the tracks before it, and said once, of the first it is out
    // with. A clip already said to be out is not what another is held to.
    let mut said: Vec<&Id> = Vec::new();
    for (index, later) in plays.iter().enumerate() {
        let out_with = plays[..index].iter().find(|earlier| {
            let shared =
                later.clip.end().min(earlier.clip.end()) - later.clip.start.max(earlier.clip.start);
            let apart = (later.zero - earlier.zero).abs();
            let belong = if earlier.media == later.media {
                linked(earlier, later) && earlier.picture != later.picture
            } else {
                linked(earlier, later) || apart < shared
            };
            earlier.of == later.of
                && shared.is_positive()
                && apart > frame
                && belong
                && !said.contains(&&earlier.clip.id)
        });
        let Some(earlier) = out_with else {
            continue;
        };
        // A sound is held to its picture; of two files, the one on the later track to the other.
        let same_file = earlier.media == later.media;
        let (slipped, held_to) = if same_file && later.picture {
            (earlier, later)
        } else {
            (later, earlier)
        };
        // A camera moved with its own sound is one slip, said of the first of the two.
        let went_with_one_said = !same_file
            && plays.iter().any(|other| {
                said.contains(&&other.clip.id)
                    && linked(other, slipped)
                    && other.media == slipped.media
                    && other.zero == slipped.zero
            });
        if !went_with_one_said && !said.contains(&&slipped.clip.id) {
            says(slipped, held_to, linked(earlier, later));
            said.push(&slipped.clip.id);
        }
    }
}

/// What cuts no transcript covers are called: information, since nothing is known to be wrong with
/// them, only that nobody looked.
pub(crate) const CUT_UNHEARD: &str = "cut.unheard";

/// What a cut inside a word a transcript heard is called.
pub(crate) const CUT_IN_WORD: &str = "cut.in_word";

/// What a cut through a sound no transcript has a word for is called.
pub(crate) const CUT_IN_SOUND: &str = "cut.in_sound";

/// What a transition is called that plays sound past a cut which is itself in quiet.
pub(crate) const CUT_FADE_IN_SOUND: &str = "cut.fade_in_sound";

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
                    CUT_IN_WORD,
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
        } else if let (Some(fade), Some(sound)) = (judged.fade, known.sound(media)) {
            // The cut is in quiet and the transition across it is not: it plays on past the
            // cut, or from before it, into what the cut left out.
            let mut finding = finding(
                CUT_FADE_IN_SOUND,
                clip.id.as_str(),
                format!(
                    "the transition where clip `{}` {end} plays {:.2} s {} the cut, at {} of `{}`, \
                     into a sound at {:.0} dB, where `{}` speaks at {:.0} dB: what the cut left out \
                     is heard fading {}; shorten the transition, put it on the cut's other side \
                     with align=, or move the cut",
                    clip.id,
                    fade.seconds,
                    if cut.end == cuts::End::Ends { "past" } else { "before" },
                    position(cut.source.seconds()),
                    media.id,
                    fade.level,
                    media.id,
                    sound.speech(),
                    if cut.end == cuts::End::Ends { "out" } else { "in" },
                ),
            )
            .during(sequence, cut.frame, cut.frame + frame);
            finding.value = Some((fade.level * 10.0).round() / 10.0);
            finding.unit = Some("dB".to_owned());
            findings.push(finding);
        } else if judged.unchecked() && !cut.silenced {
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
/// What a join is called that took a restart out and a word or two with it, which nothing beside
/// it says again: information, as a restart is, since the sentence may do without them and only
/// reading it tells.
pub(crate) const CUT_TOOK_MORE: &str = "cut.took_more";
/// How many pauses or repeats one finding names.
const NAMED: usize = 4;

/// What the sound of `sequence` still plays, by the transcripts of its media: the pauses of three
/// quarters of a second or more between words, and the words and phrases said twice in a row.
/// A cut reads the footage and not what it leaves, so a wait it trimmed too little, a second take
/// it kept, and a word doubled across one of its own cuts are said here. Each repeat across a cut
/// is a finding, since the cut may have made it; the rest are counted in one. And what it no
/// longer plays where a cut took a restart out: a word or two the cut took with it.
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
    let (at_cuts, said): (Vec<_>, Vec<_>) = restarts.iter().partition(|restart| restart.at_cut());
    let quoted = |words: &[cuts::Played<'_>]| {
        let said: Vec<&str> = words.iter().map(|word| word.text).collect();
        said.join(" ")
    };
    for restart in at_cuts {
        let (first, last) = (&restart.first[0], &restart.second[restart.second.len() - 1]);
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

    // And the other way about: a cut that took a restart out, and with it a word or two that
    // nothing beside it says again. Each by itself, since the cut did it, and as information,
    // since the sentence may do without them, as it does without a correction.
    for join in cuts::joins(project, sequence, &words, known) {
        let (out, into) = (&join.more.out, &join.more.into);
        let ends: Vec<String> = [
            out.as_ref().map(|took| {
                format!(
                    "\"{took}\" with it, the end of what `{}` was saying",
                    join.from.id
                )
            }),
            into.as_ref().map(|took| {
                format!(
                    "\"{took}\", the start of the take `{}` goes on with",
                    join.to.id
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect();
        if ends.is_empty() {
            continue;
        }
        let trims = match (out, into) {
            (Some(_), Some(_)) => format!("`{}` out and `{}` in", join.from.id, join.to.id),
            (Some(_), None) => format!("`{}` out", join.from.id),
            _ => format!("`{}` in", join.to.id),
        };
        let one = out
            .iter()
            .chain(into)
            .flat_map(|took| took.split(' '))
            .count()
            == 1;
        let it = if one { "it" } else { "them" };
        findings.push(
            Finding::new(
                Severity::Info,
                CUT_TOOK_MORE,
                Some(join.to.id.as_str()),
                format!(
                    "the cut from `{}` to `{}` at {} took a restart out and {}, which nothing \
                     beside the cut says again: it plays \"{}\"; if the sentence needs {it}, trim \
                     {trims} to play {it}",
                    join.from.id,
                    join.to.id,
                    at(join.to.start),
                    ends.join(" and "),
                    join.plays(),
                ),
            )
            .during(
                sequence,
                join.before.first().map_or(join.to.start, |word| word.start),
                join.after.last().map_or(join.to.start, |word| word.end),
            ),
        );
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
    let mut known: std::collections::HashMap<Id, Vec<(f64, f64)>> =
        std::collections::HashMap::new();
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
                    sequence
                        .rate
                        .round(clip.played_at(source).unwrap_or(clip.start))
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

    fn cuts_of(known: &Known, asked: &cuts::Asked) -> serde_json::Value {
        let project = project();
        cuts::report(&project, &project.sequences[0], known, asked)
    }

    /// Every name an answer holds: its keys, all the way down.
    fn names_in(value: &serde_json::Value, names: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(fields) => {
                for (name, value) in fields {
                    names.push(name.clone());
                    names_in(value, names);
                }
            }
            serde_json::Value::Array(values) => {
                values.iter().for_each(|value| names_in(value, names))
            }
            _ => {}
        }
    }

    /// The playbooks for speech put what they name in backticks: a line of a script, a word of
    /// one, an argument of `look` or `edit`, a field of an answer, a check's code. Each is one
    /// the grammar, a tool's schema, a report, or a check has, so that a name changed there
    /// fails here, and not in an edit an agent wrote by a playbook.
    #[test]
    fn the_playbooks_for_speech_name_only_what_the_tools_have() {
        let grammar = kerf_lang::reference(Catalog::builtin());
        // What `look` and `edit` take, and what their descriptions and the guide name.
        let mut said = crate::harness::system_prompt(&[]);
        let mut taken = Vec::new();
        for tool in ["look", "edit"] {
            let (about, schema, _) = crate::harness::metadata(tool).unwrap();
            said.push_str(about);
            for (name, property) in schema["properties"].as_object().unwrap() {
                taken.push(name.clone());
                said.push_str(property["description"].as_str().unwrap_or_default());
            }
        }
        // A report with one of everything it lists: a1 stops at 4 s of m1 and a2 goes on from
        // 10 s, a take either side of the cut, a word doubled, and a wait before the last.
        let known = Known {
            words: words(&[
                (1.0, 1.2, "so"),
                (1.2, 1.5, "this"),
                (1.5, 1.8, "is"),
                (1.8, 2.2, "the"),
                (2.2, 2.6, "plan"),
                (5.0, 5.3, "um"),
                (10.2, 10.4, "okay"),
                (10.4, 10.7, "this"),
                (10.7, 11.0, "is"),
                (11.0, 11.3, "the"),
                (11.3, 11.6, "plan"),
                (11.6, 11.9, "plan"),
                (13.0, 13.3, "done"),
            ]),
            covered: vec![(0.0, 60.0)],
            // a2 starts in the middle of a sound, so a cut is listed too.
            sound: Some(recording(&[(9.8, 10.0)], &[])),
            ..Known::default()
        };
        let report = cuts_of(&known, &cuts::Asked::default());
        for list in ["pauses", "repeats", "restarts", "joins"] {
            assert!(
                report[list]["count"].as_u64() > Some(0),
                "{list}: {report:#}"
            );
        }
        assert_eq!(report["restarts"]["atCuts"], 1, "{report:#}");
        assert!(!report["cuts"].as_array().unwrap().is_empty(), "{report:#}");
        let mut answered = Vec::new();
        names_in(&report, &mut answered);
        // How a cut is heard is a value of its entry, and a count of the summary.
        for heard in ["quiet", "low", "sound"] {
            assert!(report["summary"][heard].is_number(), "{heard}");
        }
        let codes = [
            CUT_IN_WORD,
            CUT_IN_SOUND,
            CUT_FADE_IN_SOUND,
            PAUSE_KEPT,
            WORD_REPEATED,
            PHRASE_RESTARTED,
        ];
        let document = kerf_lang::print_document(
            &project(),
            Catalog::builtin(),
            kerf_lang::PrintOptions { annotate: true },
        );
        let skills = kerf_skills::bundled_skills();
        // A line of the grammar starts with its operation, and gives each name with `=`.
        let operation = |word: &str| grammar.contains(&format!("\n{word} "));
        let given = |name: &str| grammar.contains(&format!("{name}="));
        let mut read = 0;
        for playbook in ["speech-cleanup", "target-length", "wait-speedup"] {
            let body = &kerf_skills::find_skill(&skills, playbook).unwrap().body;
            let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
            for span in body.split('`').skip(1).step_by(2) {
                read += 1;
                let mut parts = span.split(' ');
                let first = parts.next().unwrap();
                let there = if span.starts_with('{') {
                    // What `cuts` is given in place of `true`.
                    let asked: serde_json::Value = serde_json::from_str(span).unwrap();
                    let cuts = &crate::harness::metadata("look").unwrap().1["properties"]["cuts"];
                    asked["pauses"].as_f64() >= Some(cuts::LEAST_PAUSE_SECONDS)
                        && cuts["description"]
                            .as_str()
                            .unwrap()
                            .contains("{\"pauses\": <seconds>}")
                } else if span.contains(' ') && operation(first) {
                    // A script line: the grammar has its operation and each name it gives.
                    parts
                        .filter_map(|part| part.split_once('='))
                        .all(|(name, _)| given(name))
                } else if let Some((name, value)) = span.split_once('=') {
                    given(name) && (value.is_empty() || grammar.contains(span))
                } else if span.ends_with(".start") || span.ends_with(".end") {
                    grammar.contains("`c3.start`") && grammar.contains("`c3.end`")
                } else if span.contains('@') {
                    grammar.contains("`c3@<media timecode>`")
                } else if span.starts_with('#') {
                    document.contains(span)
                } else if span.contains('.') {
                    codes.contains(&span)
                } else {
                    operation(span)
                        || taken.iter().any(|name| name == span)
                        || answered.iter().any(|name| name == span)
                        || said.contains(&format!("`{span}`"))
                        || skills.iter().any(|skill| skill.name == span)
                };
                assert!(there, "`{span}`, which `{playbook}` names, is not there");
            }
        }
        assert!(read > 100, "{read}");
    }

    #[test]
    fn a_clean_cut_has_no_findings() {
        assert!(codes("set c1 opacity=1", &[]).is_empty());
    }

    /// A second picture, with no sound, and a track over V1 to lay it on.
    const OVER: &str = "media add m2 path=\"b.mov\" rate=24 dur=00:01:00:00 video=1920x1080\n\
                        track add V2 video\n";

    /// What `script`, after [`OVER`], leaves showing for a frame or two.
    fn flashes(script: &str) -> Vec<String> {
        codes(&format!("{OVER}{script}"), &[])
            .into_iter()
            .filter(|(code, _)| code == PICTURE_FLASH)
            .map(|(_, message)| message)
            .collect()
    }

    #[test]
    fn nothing_is_said_of_what_may_be_hidden() {
        // o1 ends two frames before the cut, under a picture that is turned over: whether
        // that fills the frame is not worked out, so whether c1 shows at all is not known.
        let o1 = "overwrite media=m2 at=00:00:02:00 dur=00:00:01:22 on=V2 as=o1\n";
        let over = "track add V3 video\n\
                    overwrite media=m2 src=00:00:30:00 at=00:00:00:00 dur=00:00:08:00 on=V3 as=o2\n";
        let turned = flashes(&format!("{o1}{over}set o2 rotation=180"));
        assert!(turned.is_empty(), "{turned:?}");
        // In a corner it leaves c1 showing, and the flash is seen beside it, whatever is cut
        // from it there: a feathered edge, a mask, and a key take nothing off c1.
        let corner = format!("{o1}{over}set o2 scale=0.5\n");
        assert_eq!(flashes(&corner).len(), 1);
        let dressed = flashes(&format!(
            "{corner}fx o2 add utility.edgefeather\nfx o2 add key.luma\n\
             mask o2 add m ellipse=0.5,0.5,0.4,0.4"
        ));
        assert_eq!(dressed.len(), 1, "{dressed:?}");
        // An effect that may move it or draw it larger may put it over c1 again.
        let moved = flashes(&format!("{corner}fx o2 add distort.lens"));
        assert!(moved.is_empty(), "{moved:?}");
        // Nor is it known while such a picture is dissolved into, before its own first frame:
        // o1 on the top track ends two frames before the cut, and on the track between a
        // picture that hides c1 is by then mixing into one that is turned over.
        let mixed = "track add V3 video\n\
                     overwrite media=m2 at=00:00:02:00 dur=00:00:01:22 on=V3 as=o1\n\
                     overwrite media=m2 src=00:00:10:00 at=00:00:01:00 dur=00:00:03:02 on=V2 as=o2\n\
                     overwrite media=m2 src=00:00:30:00 at=00:00:04:02 dur=00:00:02:00 on=V2 as=o3\n\
                     transition add from=o2 to=o3 fx=dissolve dur=12f\n";
        let turned = flashes(&format!("{mixed}set o3 rotation=180"));
        assert!(turned.is_empty(), "{turned:?}");
        // Mixing into one in a corner, it lets c1 through, and those two frames are seen.
        assert_eq!(
            flashes(&format!("{mixed}set o3 scale=0.5")),
            ["2 frames of `c1` show at 00:00:03:22, between `o1` ending and the cut to `c2`"]
        );
    }
}
