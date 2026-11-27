//! Lines that say how one project differs from another, for commit
//! messages and pull-request bodies.

use kerf_model::{Clip, Content, Id, Marker, Media, Params, Project, Sequence, Track, Transition};
use std::collections::{BTreeSet, HashMap};

/// Human-readable lines describing how `after` differs from `before`, for
/// commit messages and pull-request bodies: media, then sequences in order,
/// then each sequence's tracks, clips, transitions, and markers.
pub fn summarize(before: &Project, after: &Project) -> Vec<String> {
    let mut lines = Vec::new();
    if before.name != after.name {
        lines.push(format!("Renamed the project to \"{}\"", after.name));
    }
    for media in &after.media {
        match before.media(media.id.as_str()) {
            None => lines.push(format!("Added media {} ({})", media.id, media.path)),
            Some(was) => changed(
                &mut lines,
                format!("media {}", media.id),
                media_changes(was, media),
            ),
        }
    }
    for media in &before.media {
        if after.media(media.id.as_str()).is_none() {
            lines.push(format!("Removed media {}", media.id));
        }
    }
    let (clips_before, clips_after) = (clips_by_id(before), clips_by_id(after));
    for sequence in &after.sequences {
        let Some(was) = before.sequence(sequence.id.as_str()) else {
            lines.push(if sequence.name.is_empty() {
                format!("Added sequence {}", sequence.id)
            } else {
                format!("Added sequence {} ({})", sequence.id, sequence.name)
            });
            continue;
        };
        changed(
            &mut lines,
            format!("sequence {}", sequence.id),
            sequence_changes(was, sequence),
        );
        for track in &sequence.tracks {
            match was.track(track.id.as_str()) {
                None => lines.push(format!("Added track {}", track.id)),
                Some(old) => changed(
                    &mut lines,
                    format!("track {}", track.id),
                    track_changes(old, track),
                ),
            }
        }
        for track in &was.tracks {
            if sequence.track(track.id.as_str()).is_none() {
                lines.push(format!("Removed track {}", track.id));
            }
        }
        for track in &sequence.tracks {
            for clip in &track.clips {
                match clips_before.get(&clip.id) {
                    None => lines.push(format!("Added clip {} on {}", clip.id, track.id)),
                    Some(&old) => changed(
                        &mut lines,
                        format!("clip {}", clip.id),
                        clip_changes(old, (&sequence.id, &track.id, clip)),
                    ),
                }
            }
        }
        for clip in was.tracks.iter().flat_map(|track| &track.clips) {
            if !clips_after.contains_key(&clip.id) {
                lines.push(format!("Removed clip {}", clip.id));
            }
        }
        for track in &sequence.tracks {
            for transition in &track.transitions {
                match transition_in(was, &transition.id) {
                    None => lines.push(format!(
                        "Added transition {} on {}",
                        transition.id, track.id
                    )),
                    Some(old) => changed(
                        &mut lines,
                        format!("transition {}", transition.id),
                        transition_changes(old, (&track.id, transition)),
                    ),
                }
            }
        }
        for transition in was.tracks.iter().flat_map(|track| &track.transitions) {
            if transition_in(sequence, &transition.id).is_none() {
                lines.push(format!("Removed transition {}", transition.id));
            }
        }
        for marker in &sequence.markers {
            match was.marker(marker.id.as_str()) {
                None => lines.push(format!("Added marker {}", marker.id)),
                Some(old) => changed(
                    &mut lines,
                    format!("marker {}", marker.id),
                    marker_changes(old, marker),
                ),
            }
        }
        for marker in &was.markers {
            if sequence.marker(marker.id.as_str()).is_none() {
                lines.push(format!("Removed marker {}", marker.id));
            }
        }
    }
    for sequence in &before.sequences {
        if after.sequence(sequence.id.as_str()).is_none() {
            lines.push(format!("Removed sequence {}", sequence.id));
        }
    }
    lines
}

/// "Changed clip c3: start, duration", when anything changed.
fn changed(lines: &mut Vec<String>, what: String, fields: Vec<String>) {
    if !fields.is_empty() {
        lines.push(format!("Changed {what}: {}", fields.join(", ")));
    }
}

fn differs<T: PartialEq>(fields: &mut Vec<String>, name: &str, before: &T, after: &T) {
    if before != after {
        fields.push(name.to_owned());
    }
}

/// The names of the parameters that differ.
fn param_changes(fields: &mut Vec<String>, before: &Params, after: &Params) {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        differs(fields, key, &before.get(key), &after.get(key));
    }
}

fn media_changes(before: &Media, after: &Media) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "path", &before.path, &after.path);
    differs(&mut fields, "rate", &before.rate, &after.rate);
    differs(
        &mut fields,
        "drop frame",
        &before.drop_frame,
        &after.drop_frame,
    );
    differs(&mut fields, "timecode", &before.start, &after.start);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "video", &before.video, &after.video);
    differs(&mut fields, "audio", &before.audio, &after.audio);
    differs(&mut fields, "reel", &before.reel, &after.reel);
    differs(&mut fields, "recipe", &before.recipe, &after.recipe);
    differs(&mut fields, "steps", &before.steps, &after.steps);
    differs(&mut fields, "sync", &before.sync, &after.sync);
    fields
}

fn sequence_changes(before: &Sequence, after: &Sequence) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "rate", &before.rate, &after.rate);
    differs(
        &mut fields,
        "drop frame",
        &before.drop_frame,
        &after.drop_frame,
    );
    differs(
        &mut fields,
        "size",
        &(before.width, before.height),
        &(after.width, after.height),
    );
    differs(
        &mut fields,
        "sample rate",
        &before.sample_rate,
        &after.sample_rate,
    );
    differs(&mut fields, "color", &before.color, &after.color);
    differs(&mut fields, "start", &before.start, &after.start);
    fields
}

fn track_changes(before: &Track, after: &Track) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "kind", &before.kind, &after.kind);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "enabled", &before.enabled, &after.enabled);
    differs(&mut fields, "locked", &before.locked, &after.locked);
    differs(&mut fields, "muted", &before.muted, &after.muted);
    differs(&mut fields, "role", &before.role, &after.role);
    differs(
        &mut fields,
        "sync lock",
        &before.sync_locked,
        &after.sync_locked,
    );
    fields
}

/// A clip with its sequence and track.
type ClipIn<'a> = (&'a Id, &'a Id, &'a Clip);

fn clips_by_id(project: &Project) -> HashMap<&Id, ClipIn<'_>> {
    let mut clips = HashMap::new();
    for sequence in &project.sequences {
        for track in &sequence.tracks {
            for clip in &track.clips {
                clips.insert(&clip.id, (&sequence.id, &track.id, clip));
            }
        }
    }
    clips
}

fn clip_changes(
    (sequence, track, before): ClipIn<'_>,
    (now_sequence, now_track, after): ClipIn<'_>,
) -> Vec<String> {
    let mut fields = Vec::new();
    differs(
        &mut fields,
        "track",
        &(sequence, track),
        &(now_sequence, now_track),
    );
    differs(&mut fields, "start", &before.start, &after.start);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    match (&before.content, &after.content) {
        (
            Content::Media {
                media: source,
                source_in,
                speed,
            },
            Content::Media {
                media: now_source,
                source_in: now_in,
                speed: now_speed,
            },
        )
        | (
            Content::Sequence {
                sequence: source,
                source_in,
                speed,
            },
            Content::Sequence {
                sequence: now_source,
                source_in: now_in,
                speed: now_speed,
            },
        ) => {
            differs(&mut fields, "source", source, now_source);
            differs(&mut fields, "source in", source_in, now_in);
            differs(&mut fields, "speed", speed, now_speed);
        }
        (
            Content::Generator {
                fx,
                version,
                params,
            },
            Content::Generator {
                fx: now_fx,
                version: now_version,
                params: now_params,
            },
        ) => {
            differs(
                &mut fields,
                "generator",
                &(fx, version),
                &(now_fx, now_version),
            );
            param_changes(&mut fields, params, now_params);
        }
        (Content::Caption { text }, Content::Caption { text: now_text }) => {
            differs(&mut fields, "text", text, now_text);
        }
        (before, after) => differs(&mut fields, "content", before, after),
    }
    differs(&mut fields, "enabled", &before.enabled, &after.enabled);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "link", &before.link, &after.link);
    differs(&mut fields, "parent", &before.parent, &after.parent);
    differs(&mut fields, "follows", &before.follows, &after.follows);
    differs(&mut fields, "retime", &before.retime, &after.retime);
    differs(&mut fields, "angle", &before.angle, &after.angle);
    param_changes(&mut fields, &before.props, &after.props);
    differs(&mut fields, "masks", &before.masks, &after.masks);
    differs(&mut fields, "effects", &before.effects, &after.effects);
    fields
}

/// A transition of `sequence` with its track.
fn transition_in<'a>(sequence: &'a Sequence, id: &Id) -> Option<(&'a Id, &'a Transition)> {
    sequence.tracks.iter().find_map(|track| {
        track
            .transition(id.as_str())
            .map(|transition| (&track.id, transition))
    })
}

fn transition_changes(
    (track, before): (&Id, &Transition),
    (now_track, after): (&Id, &Transition),
) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "track", track, now_track);
    differs(
        &mut fields,
        "clips",
        &(&before.from, &before.to),
        &(&after.from, &after.to),
    );
    differs(
        &mut fields,
        "fx",
        &(&before.fx, before.version),
        &(&after.fx, after.version),
    );
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "align", &before.align, &after.align);
    param_changes(&mut fields, &before.params, &after.params);
    fields
}

fn marker_changes(before: &Marker, after: &Marker) -> Vec<String> {
    let mut fields = Vec::new();
    differs(&mut fields, "at", &before.at, &after.at);
    differs(&mut fields, "duration", &before.duration, &after.duration);
    differs(&mut fields, "name", &before.name, &after.name);
    differs(&mut fields, "color", &before.color, &after.color);
    differs(&mut fields, "note", &before.note, &after.note);
    fields
}
