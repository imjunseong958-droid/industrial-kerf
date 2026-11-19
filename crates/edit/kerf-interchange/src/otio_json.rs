//! An `.otio` file written and read here: OpenTimelineIO's JSON, with no
//! Python.
//!
//! The schema names and versions are those of OpenTimelineIO's serialized
//! data documentation (`docs/tutorials/otio-serialized-schema.md`) as
//! released from 0.15 through 0.18.1: `Timeline.1`, `Stack.1`, `Track.1`,
//! `Clip.2`, `Gap.1`, `Transition.1`, `ExternalReference.1`, `Marker.2`,
//! `LinearTimeWarp.1`, `RationalTime.1`, and `TimeRange.1`. A reader upgrades
//! a schema older than its own and refuses one newer, so a file is written
//! in the oldest form every one of those releases reads: with the fields of
//! 0.15, leaving out what later releases added and default when it is absent
//! (an item's `color`, an effect's `enabled`), and with `Marker.2`, whose
//! color is a name, where the unreleased `Marker.3` holds an object.
//!
//! Reading takes any version of those schemas, `Clip.1` with its single
//! `media_reference` and `Marker.3` among them, and reports what it does not
//! import. Both directions hold what the Python bridge holds, so a file means
//! the same whichever wrote or read it.

use crate::otio::{Item, Timeline, WireKind, WireMarker, WireMedia, WireSequence, WireTrack};
use crate::{issue, InterchangeError, InterchangeIssue, IssueSeverity};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The marker colors OpenTimelineIO names.
const MARKER_COLORS: [&str; 11] = [
    "PINK", "RED", "ORANGE", "YELLOW", "GREEN", "CYAN", "BLUE", "PURPLE", "MAGENTA", "BLACK",
    "WHITE",
];
/// The color a marker has in OpenTimelineIO when none is given.
const DEFAULT_MARKER_COLOR: &str = "RED";

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// JSON whose objects keep the order their fields are given in, as
/// OpenTimelineIO writes them.
enum Json {
    Null,
    Bool(bool),
    Float(f64),
    Integer(i64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(&'static str, Json)>),
}

impl Json {
    fn text(text: &str) -> Self {
        Self::Text(text.to_owned())
    }

    /// Four spaces a level, as OpenTimelineIO's own files are.
    fn print(&self, out: &mut String, depth: usize) {
        let indent = |out: &mut String, depth: usize| {
            out.push('\n');
            out.push_str(&"    ".repeat(depth));
        };
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::Float(value) => {
                let number = serde_json::Number::from_f64(*value)
                    .map_or_else(|| "0.0".to_owned(), |number| number.to_string());
                out.push_str(&number);
            }
            Self::Integer(value) => {
                let _ = write!(out, "{value}");
            }
            Self::Text(text) => out.push_str(&Value::from(text.as_str()).to_string()),
            Self::List(items) if items.is_empty() => out.push_str("[]"),
            Self::Object(fields) if fields.is_empty() => out.push_str("{}"),
            Self::List(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    indent(out, depth + 1);
                    item.print(out, depth + 1);
                }
                indent(out, depth);
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (index, (name, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    indent(out, depth + 1);
                    let _ = write!(out, "\"{name}\": ");
                    value.print(out, depth + 1);
                }
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

/// An OpenTimelineIO object: its schema, its metadata and name, then `rest`.
fn object(schema: &'static str, kerf: Vec<(&'static str, Json)>, name: &str, rest: Vec<(&'static str, Json)>) -> Json {
    let metadata = if kerf.is_empty() {
        Json::Object(Vec::new())
    } else {
        Json::Object(vec![("kerf", Json::Object(kerf))])
    };
    let mut fields = vec![
        ("OTIO_SCHEMA", Json::text(schema)),
        ("metadata", metadata),
        ("name", Json::text(name)),
    ];
    fields.extend(rest);
    Json::Object(fields)
}

/// `metadata["kerf"]` of something with an id.
fn tagged(id: Option<&str>) -> Vec<(&'static str, Json)> {
    id.map(|id| ("id", Json::text(id))).into_iter().collect()
}

fn rational(value: f64, rate: f64) -> Json {
    Json::Object(vec![
        ("OTIO_SCHEMA", Json::text("RationalTime.1")),
        ("rate", Json::Float(rate)),
        ("value", Json::Float(value)),
    ])
}

fn range(start: f64, duration: f64, rate: f64) -> Json {
    Json::Object(vec![
        ("OTIO_SCHEMA", Json::text("TimeRange.1")),
        ("duration", rational(duration, rate)),
        ("start_time", rational(start, rate)),
    ])
}

/// What an item of a track holds besides its own fields.
fn item_fields(source_range: Json, effects: Vec<Json>, enabled: bool) -> Vec<(&'static str, Json)> {
    vec![
        ("source_range", source_range),
        ("effects", Json::List(effects)),
        ("markers", Json::List(Vec::new())),
        ("enabled", Json::Bool(enabled)),
    ]
}

fn marker(marker: &WireMarker, rate: f64) -> Json {
    let color = marker
        .color
        .as_deref()
        .map(str::to_ascii_uppercase)
        .filter(|color| MARKER_COLORS.contains(&color.as_str()));
    let mut kerf = tagged(marker.id.as_deref());
    if color.is_none() {
        // OpenTimelineIO has no marker without a color.
        kerf.push(("uncolored", Json::Bool(true)));
    }
    object(
        "Marker.2",
        kerf,
        marker.name.as_deref().unwrap_or_default(),
        vec![
            ("color", Json::text(color.as_deref().unwrap_or(DEFAULT_MARKER_COLOR))),
            ("marked_range", range(marker.at as f64, marker.duration as f64, rate)),
            ("comment", Json::text(marker.note.as_deref().unwrap_or_default())),
        ],
    )
}

fn reference(media: &WireMedia) -> Json {
    let available = match media.duration {
        Some(duration) => range(media.start as f64, duration as f64, media.rate),
        None => Json::Null,
    };
    object(
        "ExternalReference.1",
        tagged(media.id.as_deref()),
        "",
        vec![
            ("available_range", available),
            ("available_image_bounds", Json::Null),
            ("target_url", Json::text(&media.path)),
        ],
    )
}

fn track(track: &WireTrack, timeline: &Timeline) -> Result<Json, InterchangeError> {
    let rate = timeline.rate;
    let mut children = Vec::new();
    let mut cursor = 0;
    for item in &track.items {
        match item {
            Item::Transition { id, before, after } => children.push(object(
                "Transition.1",
                tagged(id.as_deref()),
                "",
                vec![
                    ("in_offset", rational(*before as f64, rate)),
                    ("out_offset", rational(*after as f64, rate)),
                    ("transition_type", Json::text("SMPTE_Dissolve")),
                ],
            )),
            Item::Clip {
                id,
                name,
                media: index,
                source_in,
                start,
                duration,
                speed,
                enabled,
            } => {
                let media = timeline.media.get(*index).ok_or_else(|| {
                    InterchangeError::new(
                        "INTERCHANGE_PROJECT_INVALID",
                        format!("a clip names media {index}, which the timeline does not have"),
                    )
                })?;
                if *start > cursor {
                    children.push(object(
                        "Gap.1",
                        Vec::new(),
                        "",
                        item_fields(range(0.0, (*start - cursor) as f64, rate), Vec::new(), true),
                    ));
                }
                // How long it plays, counted in its media's frames: a part of
                // one where the two rates differ, so that the cut stays where
                // the sequence has it.
                let played = if media.rate == rate {
                    *duration as f64
                } else {
                    *duration as f64 * media.rate / rate
                };
                let effects = speed
                    .map(|speed| {
                        object(
                            "LinearTimeWarp.1",
                            Vec::new(),
                            "",
                            vec![
                                ("effect_name", Json::text("LinearTimeWarp")),
                                ("time_scalar", Json::Float(speed)),
                            ],
                        )
                    })
                    .into_iter()
                    .collect();
                let mut fields = item_fields(
                    range(*source_in as f64, played, media.rate),
                    effects,
                    *enabled,
                );
                fields.push((
                    "media_references",
                    Json::Object(vec![("DEFAULT_MEDIA", reference(media))]),
                ));
                fields.push(("active_media_reference_key", Json::text("DEFAULT_MEDIA")));
                children.push(object(
                    "Clip.2",
                    tagged(id.as_deref()),
                    name.as_deref().unwrap_or_else(|| file_name(&media.path)),
                    fields,
                ));
                cursor = start + duration;
            }
        }
    }
    let mut fields = item_fields(Json::Null, Vec::new(), track.enabled);
    fields.push(("children", Json::List(children)));
    fields.push((
        "kind",
        Json::text(match track.kind {
            WireKind::Video => "Video",
            WireKind::Audio => "Audio",
        }),
    ));
    Ok(object(
        "Track.1",
        tagged(track.id.as_deref()),
        track
            .name
            .as_deref()
            .or(track.id.as_deref())
            .unwrap_or_default(),
        fields,
    ))
}

/// `timeline` as the text of an `.otio` file.
pub fn write(timeline: &Timeline) -> Result<String, InterchangeError> {
    let rate = timeline.rate;
    let tracks = timeline
        .tracks
        .iter()
        .map(|wire| track(wire, timeline))
        .collect::<Result<Vec<_>, _>>()?;
    let mut stack = vec![
        ("source_range", Json::Null),
        ("effects", Json::List(Vec::new())),
        (
            "markers",
            Json::List(timeline.markers.iter().map(|wire| marker(wire, rate)).collect()),
        ),
        ("enabled", Json::Bool(true)),
    ];
    stack.push(("children", Json::List(tracks)));
    let mut kerf = Vec::new();
    if let Some(sequence) = &timeline.sequence {
        kerf = tagged(sequence.id.as_deref());
        if let Some(name) = &sequence.name {
            kerf.push(("name", Json::text(name)));
        }
        if let Some(width) = sequence.width {
            kerf.push(("width", Json::Integer(i64::from(width))));
        }
        if let Some(height) = sequence.height {
            kerf.push(("height", Json::Integer(i64::from(height))));
        }
        if sequence.drop_frame {
            kerf.push(("dropFrame", Json::Bool(true)));
        }
    }
    let root = object(
        "Timeline.1",
        kerf,
        &timeline.name,
        vec![
            ("global_start_time", rational(timeline.start as f64, rate)),
            ("tracks", object("Stack.1", Vec::new(), "tracks", stack)),
        ],
    );
    let mut text = String::new();
    root.print(&mut text, 0);
    Ok(text)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// A `RationalTime`: its value and the rate it counts at.
#[derive(Clone, Copy)]
struct Moment {
    value: f64,
    rate: f64,
}

impl Moment {
    /// The frame of `rate` it is nearest.
    fn frames(self, rate: f64) -> i64 {
        self.at(rate).round() as i64
    }

    fn at(self, rate: f64) -> f64 {
        self.value * rate / self.rate
    }
}

#[derive(Clone, Copy)]
struct Span {
    start: Moment,
    duration: Moment,
}

/// The schema `value` is of, without its version.
fn schema(value: &Value) -> &str {
    value
        .get("OTIO_SCHEMA")
        .and_then(Value::as_str)
        .map_or("", |name| name.split('.').next().unwrap_or(name))
}

fn moment(value: &Value) -> Option<Moment> {
    let rate = value.get("rate")?.as_f64()?;
    let value = value.get("value")?.as_f64()?;
    (rate.is_finite() && rate > 0.0 && value.is_finite()).then_some(Moment { value, rate })
}

fn span(value: &Value) -> Option<Span> {
    Some(Span {
        start: moment(value.get("start_time")?)?,
        duration: moment(value.get("duration")?)?,
    })
}

fn list<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `metadata["kerf"]` of an object.
fn kerf(value: &Value) -> Option<&Value> {
    value.get("metadata")?.get("kerf").filter(|kerf| kerf.is_object())
}

fn kerf_id(value: &Value) -> Option<String> {
    Some(kerf(value)?.get("id")?.as_str()?.to_owned())
}

/// The media reference a clip plays: the active one of `Clip.2`, or the one
/// `Clip.1` has.
fn active_reference(clip: &Value) -> &Value {
    let key = text(clip, "active_media_reference_key").unwrap_or("DEFAULT_MEDIA");
    clip.get("media_references")
        .and_then(|references| references.get(key))
        .or_else(|| clip.get("media_reference"))
        .unwrap_or(&Value::Null)
}

/// What a clip plays of its media: its own range, or all the media has.
fn clip_span(clip: &Value) -> Option<Span> {
    clip.get("source_range")
        .and_then(span)
        .or_else(|| active_reference(clip).get("available_range").and_then(span))
}

/// How long `item` lasts in its parent, in frames of `rate`: a clip or gap
/// its range, a track its children end to end, a stack its longest, a
/// transition nothing.
fn length(item: &Value, rate: f64) -> f64 {
    if let Some(span) = item.get("source_range").and_then(span) {
        return span.duration.at(rate);
    }
    let children = list(item, "children").iter().map(|child| length(child, rate));
    match schema(item) {
        "Clip" => clip_span(item).map_or(0.0, |span| span.duration.at(rate)),
        "Track" => children.sum(),
        "Stack" => children.fold(0.0, f64::max),
        _ => 0.0,
    }
}

/// The first rate anything in `item` counts at.
fn first_rate(item: &Value) -> Option<f64> {
    match item.get("source_range").and_then(span) {
        Some(span) => Some(span.duration.rate),
        None => clip_span(item)
            .map(|span| span.duration.rate)
            .or_else(|| list(item, "children").iter().find_map(first_rate)),
    }
}

/// The file a `target_url` names: a `file:` URL's path, or the text as it is.
fn path_from_url(target: &str) -> String {
    let Some(rest) = target
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file:"))
        .map(|_| &target[5..])
    else {
        return target.to_owned();
    };
    let (host, path) = match rest.strip_prefix("//") {
        Some(after) => after.split_at(after.find('/').unwrap_or(after.len())),
        None => ("", rest),
    };
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let [first, tail @ ..] = rest {
        let escaped = match (*first, tail) {
            (b'%', [high, low, ..]) => std::str::from_utf8(&[*high, *low])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok()),
            _ => None,
        };
        match escaped {
            Some(byte) => {
                bytes.push(byte);
                rest = &tail[2..];
            }
            None => {
                bytes.push(*first);
                rest = tail;
            }
        }
    }
    let mut path = String::from_utf8_lossy(&bytes).into_owned();
    // `/C:/footage/a.mov` is `C:/footage/a.mov`.
    if let [b'/', drive, b':', ..] = path.as_bytes() {
        if drive.is_ascii_alphabetic() {
            path.remove(0);
        }
    }
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        path = format!("//{host}{path}");
    }
    if path.is_empty() {
        target.to_owned()
    } else {
        path
    }
}

/// The last part of a path, whichever way its separators lean.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

fn read_marker(marker: &Value, rate: f64, offset: i64) -> Option<WireMarker> {
    let marked = marker.get("marked_range").and_then(span)?;
    let uncolored = kerf(marker)
        .and_then(|kerf| kerf.get("uncolored"))
        .and_then(Value::as_bool)
        == Some(true);
    // `Marker.2` names its color; `Marker.3` holds a color object with a name.
    let color = marker.get("color").and_then(|color| {
        color
            .as_str()
            .or_else(|| color.get("name").and_then(Value::as_str))
    });
    Some(WireMarker {
        id: kerf_id(marker),
        at: offset + marked.start.frames(rate),
        duration: marked.duration.frames(rate),
        name: text(marker, "name").map(str::to_owned),
        color: color
            .filter(|color| !color.is_empty() && !uncolored)
            .map(str::to_ascii_lowercase),
        note: text(marker, "comment").map(str::to_owned),
    })
}

/// The timeline an `.otio` file's text holds, and what of the file it does
/// not.
pub fn read(text_of_file: &str) -> Result<(Timeline, Vec<InterchangeIssue>), InterchangeError> {
    let invalid = |message: String| InterchangeError::new("INTERCHANGE_INPUT_INVALID", message);
    let root: Value = serde_json::from_str(text_of_file)
        .map_err(|error| invalid(format!("the file is not OpenTimelineIO JSON: {error}")))?;
    if schema(&root) != "Timeline" {
        let found = root.get("OTIO_SCHEMA").and_then(Value::as_str);
        return Err(invalid(match found {
            Some(found) => format!("the file holds a `{found}`, not a Timeline"),
            None => "the file has no OTIO_SCHEMA: it is not an OpenTimelineIO file".into(),
        }));
    }
    let mut issues = Vec::new();
    let mut loss = |code: &str, message: String, path: &str| {
        issues.push(issue(code, message, IssueSeverity::Loss, Some(path)));
    };
    let stack = root.get("tracks").unwrap_or(&Value::Null);
    let start = root.get("global_start_time").and_then(moment);
    // A timeline counts at its start time's rate; one with no start time, at
    // the rate of the first thing on it.
    let rate = start
        .map(|start| start.rate)
        .or_else(|| first_rate(stack))
        .unwrap_or(24.0);

    let mut media: Vec<WireMedia> = Vec::new();
    let mut media_index: HashMap<String, usize> = HashMap::new();
    let mut tracks = Vec::new();
    let mut markers: Vec<WireMarker> = list(stack, "markers")
        .iter()
        .filter_map(|marker| read_marker(marker, rate, 0))
        .collect();

    for (track_index, track) in list(stack, "children").iter().enumerate() {
        let path = format!("tracks[{track_index}]");
        let kind = match (schema(track), track.get("kind").and_then(Value::as_str)) {
            ("Track", Some("Video")) => WireKind::Video,
            ("Track", Some("Audio")) => WireKind::Audio,
            _ => {
                loss(
                    "track_not_imported",
                    "only video and audio tracks are imported".into(),
                    &path,
                );
                continue;
            }
        };
        let mut items = Vec::new();
        // Where the next item starts, in frames of the timeline.
        let mut position = 0.0_f64;
        for (child_index, child) in list(track, "children").iter().enumerate() {
            let child_path = format!("{path}.children[{child_index}]");
            match schema(child) {
                "Clip" => {
                    let Some(source) = clip_span(child) else {
                        loss(
                            "item_not_imported",
                            "a clip with neither a source range nor media of a known length was not imported".into(),
                            &child_path,
                        );
                        continue;
                    };
                    let reference = active_reference(child);
                    let mut target = path_from_url(text(reference, "target_url").unwrap_or_default());
                    if target.is_empty() {
                        loss(
                            "missing_media_reference",
                            "a clip without a media file was imported as a placeholder".into(),
                            &child_path,
                        );
                        target = format!("missing-{}", media.len() + 1);
                    }
                    let index = match media_index.get(&target) {
                        Some(index) => *index,
                        None => {
                            let available = reference.get("available_range").and_then(span);
                            let media_rate = available.unwrap_or(source).start.rate;
                            let (from, lasting) = match available {
                                Some(available) => (
                                    available.start.frames(media_rate),
                                    available.duration.frames(media_rate),
                                ),
                                None => (
                                    0,
                                    (source.start.at(media_rate) + source.duration.at(media_rate))
                                        .round() as i64,
                                ),
                            };
                            media.push(WireMedia {
                                id: kerf_id(reference),
                                path: target.clone(),
                                rate: media_rate,
                                start: from,
                                duration: Some(lasting),
                            });
                            media_index.insert(target, media.len() - 1);
                            media.len() - 1
                        }
                    };
                    let source_file = &media[index];
                    let start = position.round() as i64;
                    let mut speed = None;
                    let mut other_effects = false;
                    for effect in list(child, "effects") {
                        match (schema(effect), effect.get("time_scalar").and_then(Value::as_f64)) {
                            ("LinearTimeWarp", Some(scalar)) if scalar > 0.0 => {
                                speed = (scalar != 1.0).then_some(scalar);
                            }
                            ("LinearTimeWarp", Some(scalar)) if scalar < 0.0 => {
                                loss(
                                    "reverse_not_imported",
                                    "a reversed clip was imported playing forward".into(),
                                    &child_path,
                                );
                                speed = (scalar != -1.0).then_some(-scalar);
                            }
                            _ => other_effects = true,
                        }
                    }
                    if other_effects {
                        loss(
                            "effects_not_imported",
                            "clip effects and time warps were not imported".into(),
                            &child_path,
                        );
                    }
                    let id = kerf_id(child);
                    // A clip named after itself or its file has no name of its own.
                    let name = text(child, "name").filter(|name| {
                        Some(*name) != id.as_deref()
                            && *name != file_name(&source_file.path)
                            && *name != file_stem(&source_file.path)
                    });
                    let offset = start - source.start.frames(rate);
                    markers.extend(
                        list(child, "markers")
                            .iter()
                            .filter_map(|marker| read_marker(marker, rate, offset)),
                    );
                    items.push(Item::Clip {
                        id,
                        name: name.map(str::to_owned),
                        media: index,
                        source_in: source.start.frames(source_file.rate),
                        start,
                        duration: source.duration.frames(rate),
                        speed,
                        enabled: child.get("enabled").and_then(Value::as_bool) != Some(false),
                    });
                    position += source.duration.at(rate);
                }
                "Transition" => {
                    let frames = |field: &str| {
                        child
                            .get(field)
                            .and_then(moment)
                            .map_or(0, |offset| offset.frames(rate))
                    };
                    items.push(Item::Transition {
                        id: kerf_id(child),
                        before: frames("in_offset"),
                        after: frames("out_offset"),
                    });
                }
                "Gap" => position += length(child, rate),
                other => {
                    loss(
                        "item_not_imported",
                        format!("{} items are not imported", if other.is_empty() { "unknown" } else { other }),
                        &child_path,
                    );
                    position += length(child, rate);
                }
            }
        }
        markers.extend(
            list(track, "markers")
                .iter()
                .filter_map(|marker| read_marker(marker, rate, 0)),
        );
        let id = kerf_id(track);
        let name = text(track, "name").filter(|name| Some(*name) != id.as_deref());
        tracks.push(WireTrack {
            name: name.map(str::to_owned),
            id,
            kind,
            enabled: track.get("enabled").and_then(Value::as_bool) != Some(false),
            items,
        });
    }

    let sequence = kerf(&root).map(|kerf| {
        let size = |field: &str| {
            kerf.get(field)
                .and_then(Value::as_u64)
                .and_then(|size| u32::try_from(size).ok())
        };
        WireSequence {
            id: text(kerf, "id").map(str::to_owned),
            name: text(kerf, "name").map(str::to_owned),
            width: size("width"),
            height: size("height"),
            drop_frame: kerf.get("dropFrame").and_then(Value::as_bool) == Some(true),
        }
    });
    let timeline = Timeline {
        name: text(&root, "name").unwrap_or_default().to_owned(),
        rate,
        start: start.map_or(0, |start| start.frames(rate)),
        sequence,
        media,
        tracks,
        markers,
    };
    Ok((timeline, issues))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(path_from_url("/media/a.mov"), "/media/a.mov");
        assert_eq!(path_from_url("media/a b.mov"), "media/a b.mov");
        assert_eq!(path_from_url("file:///media/a%20b.mov"), "/media/a b.mov");
        assert_eq!(path_from_url("FILE://localhost/media/a.mov"), "/media/a.mov");
        assert_eq!(path_from_url("file://server/share/a.mov"), "//server/share/a.mov");
        assert_eq!(path_from_url("file:///C:/footage/a.mov"), "C:/footage/a.mov");
        assert_eq!(path_from_url("file:///media/%EC%98%81%EC%83%81.mov"), "/media/영상.mov");
        assert_eq!(path_from_url("file:///media/100%.mov"), "/media/100%.mov");
    }

    #[test]
    fn a_file_that_is_not_a_timeline_is_refused_by_what_it_is() {
        let error = read("{\"OTIO_SCHEMA\": \"SerializableCollection.1\"}").unwrap_err();
        assert_eq!(error.code, "INTERCHANGE_INPUT_INVALID");
        assert!(error.message.contains("SerializableCollection.1"), "{error}");
        assert!(read("<fcpxml/>").unwrap_err().message.contains("not OpenTimelineIO JSON"));
        assert!(read("{}").unwrap_err().message.contains("no OTIO_SCHEMA"));
    }
}
