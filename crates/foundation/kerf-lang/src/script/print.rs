//! Operations printed back as script: the line each operation is, read
//! against the project as it stands before it, and the `use` lines a script
//! of them needs.

use crate::doc::{
    content_text, mask_shape_text, mask_text, media_line, params_text, sequence_line, word_or_quote,
};
use crate::making::print_step;
use crate::text::{print_fx_ref, print_speed, quote, Clock};
use kerf_model::{
    apply, Align, Bus, BusChange, BusFx, Catalog, ClipChange, Content, Domain, Edge, Effect, Id,
    MarkerChange, MarkerColor, Mask, MaskMode, MaskShape, Media, NewClip, Op, Param, Params,
    Project, Rate, SequenceChange, Time, TrackChange, TrackKind, TrackRoleChange,
};
use std::fmt::Write as _;

/// Print `op` as script, reading clocks from `project` as it stands before
/// the operation. Applying the printed script repeats the operation. A
/// `use` line comes first when the project has several sequences.
pub fn print_op(project: &Project, catalog: &Catalog, op: &Op) -> String {
    print_in(project, catalog, op, &mut None)
}

/// Print a list of operations as a script, each against the project as the
/// earlier ones leave it, switching sequences with `use` only when needed.
pub fn print_ops(project: &Project, catalog: &Catalog, ops: &[Op]) -> String {
    let mut state = project.clone();
    let mut current = None;
    let mut out = String::new();
    for op in ops {
        out.push_str(&print_in(&state, catalog, op, &mut current));
        out.push('\n');
        if let Ok((next, _)) = apply(&state, catalog, std::slice::from_ref(op)) {
            state = next;
        }
    }
    out
}

/// The sequence an operation acts in through the script's `use` context.
fn context_of(op: &Op) -> Option<&Id> {
    match op {
        Op::AddTrack { sequence, .. }
        | Op::SetTrack { sequence, .. }
        | Op::SetTrackRole { sequence, .. }
        | Op::RemoveTrack { sequence, .. }
        | Op::Insert { sequence, .. }
        | Op::Overwrite { sequence, .. }
        | Op::Lift { sequence, .. }
        | Op::Extract { sequence, .. }
        | Op::Blade { sequence, .. }
        | Op::AddBus { sequence, .. }
        | Op::SetBus { sequence, .. }
        | Op::RemoveBus { sequence, .. }
        | Op::BusFx { sequence, .. }
        | Op::AddMarker { sequence, .. } => Some(sequence),
        _ => None,
    }
}

fn print_in(project: &Project, catalog: &Catalog, op: &Op, current: &mut Option<Id>) -> String {
    let line = op_line(project, catalog, op);
    if let Op::AddSequence(sequence) = op {
        current.get_or_insert_with(|| sequence.id.clone());
    }
    match context_of(op) {
        Some(sequence) if current.as_ref() != Some(sequence) => {
            let needed = project.sequences.len() > 1 || current.is_some();
            *current = Some(sequence.clone());
            if needed {
                format!("use {sequence}\n{line}")
            } else {
                line
            }
        }
        _ => line,
    }
}

fn op_line(project: &Project, catalog: &Catalog, op: &Op) -> String {
    match op {
        Op::AddMedia(media) => add_media_line(media),
        Op::RelinkMedia { media, path } => format!("media relink {media} path={}", quote(path)),
        Op::ReplaceMedia(media) => media_line(media).replacen("media ", "media replace ", 1),
        Op::RemoveMedia { media } => format!("media remove {media}"),
        Op::SyncMedia { media, sync } => format!(
            "media sync {media} with={}",
            sync.as_ref()
                .map_or_else(|| "\"\"".to_owned(), crate::doc::print_synced)
        ),
        Op::AddStep { media, index, step } => match index {
            Some(index) => format!("step {media} add {} {}", index + 1, print_step(step)),
            None => format!("step {media} add {}", print_step(step)),
        },
        Op::SetStep { media, index, step } => {
            format!("step {media} set {} {}", index + 1, print_step(step))
        }
        Op::RemoveStep { media, index } => format!("step {media} remove {}", index + 1),
        Op::AddSequence(sequence) => {
            sequence_line(sequence).replacen("sequence ", "sequence add ", 1)
        }
        Op::SetSequence { sequence, change } => set_sequence_line(project, sequence, change),
        Op::RemoveSequence { sequence } => format!("sequence remove {sequence}"),
        Op::AddTrack {
            sequence,
            id,
            kind,
            index,
        } => add_track_line(project, sequence, id, kind, index),
        Op::AddBus { sequence, bus } => add_bus_line(project, catalog, sequence, bus),
        Op::SetBus {
            sequence,
            bus,
            change,
        } => set_bus_line(project, sequence, bus, change),
        Op::RemoveBus { bus, .. } => format!("bus remove {bus}"),
        Op::BusFx {
            sequence,
            bus,
            change,
        } => bus_fx_line(project, catalog, sequence, bus, change),
        Op::SetTrack { track, change, .. } => set_track_line(track, change),
        Op::SetTrackRole { track, change, .. } => set_track_role_line(track, change),
        Op::RemoveTrack { track, .. } => format!("track remove {track}"),
        Op::Insert {
            sequence,
            at,
            duration,
            targets,
        } => place_line(project, catalog, "insert", sequence, at, duration, targets),
        Op::Overwrite {
            sequence,
            at,
            duration,
            targets,
        } => place_line(
            project,
            catalog,
            "overwrite",
            sequence,
            at,
            duration,
            targets,
        ),
        Op::Delete { clips } => format!("delete {}", id_words(clips)),
        Op::RippleDelete { clips } => format!("ripple-delete {}", id_words(clips)),
        Op::Lift {
            sequence,
            from,
            to,
            tracks,
        } => take_out_line(project, "lift", sequence, from, to, tracks),
        Op::Extract {
            sequence,
            from,
            to,
            tracks,
        } => take_out_line(project, "extract", sequence, from, to, tracks),
        Op::Trim {
            clip,
            edge,
            delta,
            ripple,
        } => trim_line(project, clip, edge, delta, ripple),
        Op::Roll { clip, delta, alone } => format!(
            "roll {clip} {}{}",
            clip_clock(project, clip)
                .unwrap_or(FALLBACK)
                .print_delta(*delta),
            if *alone { " alone" } else { "" }
        ),
        Op::Slip { clip, delta } => format!(
            "slip {clip} {}",
            clip_clock(project, clip)
                .unwrap_or(FALLBACK)
                .print_delta(*delta)
        ),
        Op::Slide { clip, delta } => format!(
            "slide {clip} {}",
            clip_clock(project, clip)
                .unwrap_or(FALLBACK)
                .print_delta(*delta)
        ),
        Op::Split { clip, at, tail } => split_line(project, clip, at, tail),
        Op::Blade {
            sequence,
            at,
            tracks,
        } => blade_line(project, sequence, at, tracks),
        Op::Move { clip, track, delta } => move_line(project, clip, track, delta),
        Op::Set { clip, changes } => set_line(project, clip, changes),
        Op::Speed { clip, speed } => {
            let rate = project
                .locate(clip.as_str())
                .map_or(Rate::FPS_24, |at| project.sequences[at.sequence].rate);
            format!("speed {clip} {}", print_speed(speed, rate))
        }
        Op::FxAdd {
            clip,
            effect,
            index,
        } => fx_add_line(project, catalog, clip, effect, index),
        Op::FxSet {
            clip,
            index,
            enabled,
            params,
        } => fx_set_line(project, clip, index, enabled, params),
        Op::MaskAdd { clip, mask, index } => mask_add_line(project, catalog, clip, mask, index),
        Op::MaskSet {
            clip,
            name,
            shape,
            mode,
            inverted,
            params,
        } => mask_set_line(project, clip, name, shape, mode, inverted, params),
        Op::MaskRemove { clip, name } => format!("mask {clip} remove {name}"),
        Op::FxRemove { clip, index } => format!("fx {clip} remove {}", index + 1),
        Op::FxFlatten { clip, index } => format!("fx {clip} flatten {}", index + 1),
        Op::FxMove { clip, index, to } => format!("fx {clip} move {} to={}", index + 1, to + 1),
        Op::AddTransition {
            id,
            from,
            to,
            fx,
            version,
            duration,
            align,
            params,
        } => add_transition_line(
            project,
            catalog,
            id,
            (from, to),
            (fx, version),
            (duration, align),
            params,
        ),
        Op::SetTransition {
            id,
            duration,
            align,
            params,
        } => set_transition_line(project, id, duration, align, params),
        Op::RemoveTransition { id } => format!("transition remove {id}"),
        Op::AddMarker {
            sequence,
            id,
            at,
            duration,
            name,
            color,
            note,
        } => add_marker_line(project, sequence, id, (at, duration), name, color, note),
        Op::SetMarker { id, change } => set_marker_line(project, id, change),
        Op::RemoveMarker { id } => format!("marker remove {id}"),
        Op::Link { clips, group } => link_line(clips, group),
        Op::Unlink { clips } => format!("unlink {}", id_words(clips)),
        Op::Attach { clips, to } => format!("attach {} to={to}", id_words(clips)),
        Op::Detach { clips } => format!("detach {}", id_words(clips)),
    }
}

/// `media add …`.
fn add_media_line(media: &Media) -> String {
    // A media that comes with its steps is its line and a line for each of them.
    let mut text = media_line(media).replacen("media ", "media add ", 1);
    for step in &media.steps {
        write!(text, "\nstep {} add {}", media.id, print_step(step))
            .expect("writing to a String cannot fail");
    }
    text
}

/// `sequence set …`.
fn set_sequence_line(project: &Project, sequence: &Id, change: &SequenceChange) -> String {
    let mut text = format!("sequence set {sequence}");
    if let Some(name) = &change.name {
        write!(text, " name={}", quote(name)).expect("writing to a String cannot fail");
    }
    if let Some(rate) = change.rate {
        write!(text, " rate={rate}").expect("writing to a String cannot fail");
    }
    if let Some((width, height)) = change.size {
        write!(text, " size={width}x{height}").expect("writing to a String cannot fail");
    }
    if let Some(sample_rate) = change.sample_rate {
        write!(text, " audio={sample_rate}").expect("writing to a String cannot fail");
    }
    if let Some(color) = change.color {
        write!(text, " color={}", color.id()).expect("writing to a String cannot fail");
    }
    if let Some((start, drop_frame)) = change.start {
        let rate = change
            .rate
            .or_else(|| project.sequence(sequence.as_str()).map(|s| s.rate))
            .unwrap_or(Rate::FPS_24);
        let clock = Clock {
            rate,
            origin: Time::ZERO,
            drop_frame,
        };
        write!(text, " start={}", clock.print_position(start))
            .expect("writing to a String cannot fail");
    }
    text
}

/// `track add …`, before the track that is at `index` now.
fn add_track_line(
    project: &Project,
    sequence: &Id,
    id: &Id,
    kind: &TrackKind,
    index: &Option<usize>,
) -> String {
    let mut text = format!("track add {id} {}", kind.name());
    let tracks = project
        .sequence(sequence.as_str())
        .map(|s| s.tracks.as_slice())
        .unwrap_or_default();
    if let Some(anchor) = index.and_then(|i| tracks.get(i)) {
        write!(text, " before={}", anchor.id).expect("writing to a String cannot fail");
    }
    text
}

/// `bus add …`.
fn add_bus_line(project: &Project, catalog: &Catalog, sequence: &Id, bus: &Bus) -> String {
    let rate = sequence_clock(project, sequence).unwrap_or(FALLBACK).rate;
    let mut text = format!("bus add {}", bus.id);
    if let Some(name) = &bus.name {
        write!(text, " name={}", quote(name)).expect("writing to a String cannot fail");
    }
    if let Some(to) = &bus.to {
        write!(text, " to={to}").expect("writing to a String cannot fail");
    }
    text.push_str(&params_text(
        &bus.params,
        catalog.props(Domain::Audio),
        rate,
    ));
    text
}

/// `bus set …`.
fn set_bus_line(project: &Project, sequence: &Id, bus: &Id, change: &BusChange) -> String {
    let rate = sequence_clock(project, sequence).unwrap_or(FALLBACK).rate;
    let mut text = format!("bus set {bus}");
    if let Some(name) = &change.name {
        write!(text, " name={}", quote(name.as_deref().unwrap_or_default()))
            .expect("writing to a String cannot fail");
    }
    match &change.to {
        Some(Some(to)) => write!(text, " to={to}").expect("writing to a String cannot fail"),
        Some(None) => text.push_str(" to=\"\""),
        None => {}
    }
    let resets: Vec<&str> = change
        .params
        .iter()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.as_str())
        .collect();
    if !resets.is_empty() {
        write!(text, " reset={}", resets.join(",")).expect("writing to a String cannot fail");
    }
    for (name, value) in &change.params {
        if let Some(value) = value {
            write!(text, " {name}={}", crate::text::print_param(value, rate))
                .expect("writing to a String cannot fail");
        }
    }
    text
}

/// `bus fx <bus> add|set|remove …`.
fn bus_fx_line(
    project: &Project,
    catalog: &Catalog,
    sequence: &Id,
    bus: &Id,
    change: &BusFx,
) -> String {
    let rate = sequence_clock(project, sequence).unwrap_or(FALLBACK).rate;
    match change {
        kerf_model::BusFx::Add { effect, index } => {
            let mut text = format!(
                "bus fx {bus} add {}",
                print_fx_ref(&effect.fx, effect.version)
            );
            if let Some(index) = index {
                write!(text, " at={}", index + 1).expect("writing to a String cannot fail");
            }
            if !effect.enabled {
                text.push_str(" enabled=false");
            }
            if let Some(def) = catalog.fx(effect.fx.as_str()) {
                text.push_str(&params_text(&effect.params, def.params.as_slice(), rate));
            }
            text
        }
        kerf_model::BusFx::Set {
            index,
            enabled,
            params,
        } => {
            let mut text = format!("bus fx {bus} set {}", index + 1);
            if let Some(enabled) = enabled {
                write!(text, " enabled={enabled}").expect("writing to a String cannot fail");
            }
            let resets: Vec<&str> = params
                .iter()
                .filter(|(_, value)| value.is_none())
                .map(|(name, _)| name.as_str())
                .collect();
            if !resets.is_empty() {
                write!(text, " reset={}", resets.join(","))
                    .expect("writing to a String cannot fail");
            }
            for (name, value) in params {
                if let Some(value) = value {
                    write!(text, " {name}={}", crate::text::print_param(value, rate))
                        .expect("writing to a String cannot fail");
                }
            }
            text
        }
        kerf_model::BusFx::Remove { index } => {
            format!("bus fx {bus} remove {}", index + 1)
        }
    }
}

/// `track set …` with what a track change sets.
fn set_track_line(track: &Id, change: &TrackChange) -> String {
    let mut text = format!("track set {track}");
    match &change.bus {
        Some(Some(bus)) => write!(text, " bus={bus}").expect("writing to a String cannot fail"),
        Some(None) => text.push_str(" bus=\"\""),
        None => {}
    }
    if let Some(name) = &change.name {
        write!(text, " name={}", quote(name.as_deref().unwrap_or_default()))
            .expect("writing to a String cannot fail");
    }
    for (flag, value) in [
        ("enabled", change.enabled),
        ("locked", change.locked),
        ("muted", change.muted),
    ] {
        if let Some(value) = value {
            write!(text, " {flag}={value}").expect("writing to a String cannot fail");
        }
    }
    text
}

/// `track set …` with what a change of a track's role sets.
fn set_track_role_line(track: &Id, change: &TrackRoleChange) -> String {
    let mut text = format!("track set {track}");
    if let Some(role) = &change.role {
        match role {
            Some(role) => write!(text, " role={}", word_or_quote(role))
                .expect("writing to a String cannot fail"),
            None => text.push_str(" role=\"\""),
        }
    }
    if let Some(sync) = change.sync_locked {
        write!(text, " sync={}", if sync { "on" } else { "off" })
            .expect("writing to a String cannot fail");
    }
    text
}

/// `insert` or `overwrite`, as `command` says, of what `targets` place: the
/// first one's content and name, and the tracks, clip ids, and properties of
/// them all.
fn place_line(
    project: &Project,
    catalog: &Catalog,
    command: &str,
    sequence: &Id,
    at: &Time,
    duration: &Time,
    targets: &[(Id, NewClip)],
) -> String {
    let clock = sequence_clock(project, sequence).unwrap_or(FALLBACK);
    let Some((_, first)) = targets.first() else {
        return format!("{command} at={}", clock.print_position(*at));
    };
    let mut text = format!(
        "{command} {} at={} dur={}",
        content_text(project, &first.content),
        clock.print_position(*at),
        clock.print_duration(*duration)
    );
    if let Content::Media { speed, .. } | Content::Sequence { speed, .. } = &first.content
        && !speed.is_one()
    {
        write!(text, " speed={}", print_speed(speed, clock.rate))
            .expect("writing to a String cannot fail");
    }
    let tracks: Vec<Id> = targets.iter().map(|(t, _)| t.clone()).collect();
    write!(text, " on={}", id_list(&tracks)).expect("writing to a String cannot fail");
    let names: Vec<Id> = targets.iter().map_while(|(_, c)| c.id.clone()).collect();
    if !names.is_empty() {
        write!(text, " as={}", id_list(&names)).expect("writing to a String cannot fail");
    }
    if let Some(name) = &first.name {
        write!(text, " name={}", quote(name)).expect("writing to a String cannot fail");
    }
    if let Content::Generator { fx, params, .. } = &first.content {
        let defs = catalog
            .fx(fx.as_str())
            .map(|d| d.params.as_slice())
            .unwrap_or_default();
        text.push_str(&params_text(params, defs, clock.rate));
    }
    let mut union = Params::new();
    for (_, clip) in targets {
        for (name, value) in &clip.props {
            union.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    text.push_str(&params_text(&union, &[], clock.rate));
    text
}

/// `lift` or `extract`, as `command` says.
fn take_out_line(
    project: &Project,
    command: &str,
    sequence: &Id,
    from: &Time,
    to: &Time,
    tracks: &Option<Vec<Id>>,
) -> String {
    let clock = sequence_clock(project, sequence).unwrap_or(FALLBACK);
    let mut text = format!(
        "{command} from={} to={}",
        clock.print_position(*from),
        clock.print_position(*to)
    );
    if let Some(tracks) = tracks {
        write!(text, " tracks={}", id_list(tracks)).expect("writing to a String cannot fail");
    }
    text
}

/// `trim …`.
fn trim_line(project: &Project, clip: &Id, edge: &Edge, delta: &Time, ripple: &bool) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let edge = if *edge == Edge::In { "in" } else { "out" };
    let ripple = if *ripple { " ripple" } else { "" };
    format!("trim {clip} {edge} {}{ripple}", clock.print_delta(*delta))
}

/// `split …`.
fn split_line(project: &Project, clip: &Id, at: &Time, tail: &Option<Id>) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("split {clip} at={}", clock.print_position(*at));
    if let Some(tail) = tail {
        write!(text, " as={tail}").expect("writing to a String cannot fail");
    }
    text
}

/// `blade …`.
fn blade_line(project: &Project, sequence: &Id, at: &Time, tracks: &Option<Vec<Id>>) -> String {
    let clock = sequence_clock(project, sequence).unwrap_or(FALLBACK);
    let mut text = format!("blade at={}", clock.print_position(*at));
    if let Some(tracks) = tracks {
        write!(text, " tracks={}", id_list(tracks)).expect("writing to a String cannot fail");
    }
    text
}

/// `move …`.
fn move_line(project: &Project, clip: &Id, track: &Option<Id>, delta: &Time) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("move {clip}");
    if let Some(track) = track {
        write!(text, " on={track}").expect("writing to a String cannot fail");
    }
    if *delta != Time::ZERO || track.is_none() {
        write!(text, " by={}", clock.print_delta(*delta)).expect("writing to a String cannot fail");
    }
    text
}

/// `set …` with each of `changes`.
fn set_line(project: &Project, clip: &Id, changes: &[ClipChange]) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("set {clip}");
    let mut params = Vec::new();
    for change in changes {
        match change {
            ClipChange::Name(name) => {
                write!(text, " name={}", quote(name.as_deref().unwrap_or_default()))
                    .expect("writing to a String cannot fail")
            }
            ClipChange::Enabled(enabled) => {
                write!(text, " enabled={enabled}").expect("writing to a String cannot fail")
            }
            ClipChange::Parent(parent) => write!(
                text,
                " parent={}",
                parent
                    .as_ref()
                    .map_or_else(|| quote(""), |parent| parent.to_string())
            )
            .expect("writing to a String cannot fail"),
            ClipChange::Retime(retime) => {
                write!(text, " retime={}", retime.id()).expect("writing to a String cannot fail")
            }
            ClipChange::Angle(Some(angle)) => {
                write!(text, " angle={angle}").expect("writing to a String cannot fail")
            }
            ClipChange::Angle(None) => text.push_str(" angle=\"\""),
            ClipChange::Caption(caption) => write!(text, " caption={}", quote(caption))
                .expect("writing to a String cannot fail"),
            ClipChange::Param(name, value) => params.push((name.clone(), value.clone())),
        }
    }
    text.push_str(&changes_text(&params, clock.rate));
    text
}

/// `fx <clip> add …`.
fn fx_add_line(
    project: &Project,
    catalog: &Catalog,
    clip: &Id,
    effect: &Effect,
    index: &Option<usize>,
) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("fx {clip} add {}", print_fx_ref(&effect.fx, effect.version));
    if let Some(index) = index {
        write!(text, " at={}", index + 1).expect("writing to a String cannot fail");
    }
    if !effect.enabled {
        text.push_str(" enabled=false");
    }
    let defs = catalog
        .fx(effect.fx.as_str())
        .map(|d| d.params.as_slice())
        .unwrap_or_default();
    text.push_str(&params_text(&effect.params, defs, clock.rate));
    text
}

/// `fx <clip> set …`.
fn fx_set_line(
    project: &Project,
    clip: &Id,
    index: &usize,
    enabled: &Option<bool>,
    params: &[(String, Option<Param>)],
) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("fx {clip} set {}", index + 1);
    if let Some(enabled) = enabled {
        write!(text, " enabled={enabled}").expect("writing to a String cannot fail");
    }
    text.push_str(&changes_text(params, clock.rate));
    text
}

/// `mask <clip> add …`.
fn mask_add_line(
    project: &Project,
    catalog: &Catalog,
    clip: &Id,
    mask: &Mask,
    index: &Option<usize>,
) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("mask {clip} add {}", mask.name);
    text.push_str(&mask_text(mask, catalog, clock.rate));
    if let Some(index) = index {
        write!(text, " at={}", index + 1).expect("writing to a String cannot fail");
    }
    text
}

/// `mask <clip> set …`.
fn mask_set_line(
    project: &Project,
    clip: &Id,
    name: &Id,
    shape: &Option<MaskShape>,
    mode: &Option<MaskMode>,
    inverted: &Option<bool>,
    params: &[(String, Option<Param>)],
) -> String {
    let clock = clip_clock(project, clip).unwrap_or(FALLBACK);
    let mut text = format!("mask {clip} set {name}");
    if let Some(shape) = shape {
        text.push_str(&mask_shape_text(shape, clock.rate));
    }
    if let Some(mode) = mode {
        write!(text, " mode={}", mode.name()).expect("writing to a String cannot fail");
    }
    if let Some(inverted) = inverted {
        write!(text, " inverted={inverted}").expect("writing to a String cannot fail");
    }
    text.push_str(&changes_text(params, clock.rate));
    text
}

/// `transition add …`: the transition `fx` at `version` over the cut from `from` to `to`.
fn add_transition_line(
    project: &Project,
    catalog: &Catalog,
    id: &Option<Id>,
    (from, to): (&Id, &Id),
    (fx, version): (&Id, &u32),
    (duration, align): (&Time, &Align),
    params: &Params,
) -> String {
    let clock = clip_clock(project, from).unwrap_or(FALLBACK);
    let mut text = String::from("transition add");
    if let Some(id) = id {
        write!(text, " as={id}").expect("writing to a String cannot fail");
    }
    write!(
        text,
        " from={from} to={to} fx={} dur={}",
        print_fx_ref(fx, *version),
        clock.print_duration(*duration)
    )
    .expect("writing to a String cannot fail");
    if *align != kerf_model::Align::Center {
        write!(text, " align={}", align.name()).expect("writing to a String cannot fail");
    }
    let defs = catalog
        .fx(fx.as_str())
        .map(|d| d.params.as_slice())
        .unwrap_or_default();
    text.push_str(&params_text(params, defs, clock.rate));
    text
}

/// `transition set …`.
fn set_transition_line(
    project: &Project,
    id: &Id,
    duration: &Option<Time>,
    align: &Option<Align>,
    params: &[(String, Option<Param>)],
) -> String {
    let clock = project
        .sequences
        .iter()
        .find(|s| s.tracks.iter().any(|t| t.transition(id.as_str()).is_some()))
        .map_or(FALLBACK, Clock::of_sequence);
    let mut text = format!("transition set {id}");
    if let Some(duration) = duration {
        write!(text, " dur={}", clock.print_duration(*duration))
            .expect("writing to a String cannot fail");
    }
    if let Some(align) = align {
        write!(text, " align={}", align.name()).expect("writing to a String cannot fail");
    }
    text.push_str(&changes_text(params, clock.rate));
    text
}

/// `marker add …`: a marker at `at`, `duration` long.
fn add_marker_line(
    project: &Project,
    sequence: &Id,
    id: &Option<Id>,
    (at, duration): (&Time, &Time),
    name: &Option<String>,
    color: &Option<MarkerColor>,
    note: &Option<String>,
) -> String {
    let clock = sequence_clock(project, sequence).unwrap_or(FALLBACK);
    let mut text = String::from("marker add");
    if let Some(id) = id {
        write!(text, " as={id}").expect("writing to a String cannot fail");
    }
    write!(text, " at={}", clock.print_position(*at)).expect("writing to a String cannot fail");
    if *duration != Time::ZERO {
        write!(text, " dur={}", clock.print_duration(*duration))
            .expect("writing to a String cannot fail");
    }
    if let Some(name) = name {
        write!(text, " name={}", quote(name)).expect("writing to a String cannot fail");
    }
    if let Some(color) = color {
        write!(text, " color={}", color.name()).expect("writing to a String cannot fail");
    }
    if let Some(note) = note {
        write!(text, " note={}", quote(note)).expect("writing to a String cannot fail");
    }
    text
}

/// `marker set …`.
fn set_marker_line(project: &Project, id: &Id, change: &MarkerChange) -> String {
    let clock = project
        .sequences
        .iter()
        .find(|s| s.marker(id.as_str()).is_some())
        .map_or(FALLBACK, Clock::of_sequence);
    let mut text = format!("marker set {id}");
    if let Some(at) = change.at {
        write!(text, " at={}", clock.print_position(at)).expect("writing to a String cannot fail");
    }
    if let Some(duration) = change.duration {
        write!(text, " dur={}", clock.print_duration(duration))
            .expect("writing to a String cannot fail");
    }
    if let Some(name) = &change.name {
        write!(text, " name={}", quote(name.as_deref().unwrap_or_default()))
            .expect("writing to a String cannot fail");
    }
    if let Some(color) = change.color {
        write!(text, " color={}", color.map_or("none", |c| c.name()))
            .expect("writing to a String cannot fail");
    }
    if let Some(note) = &change.note {
        write!(text, " note={}", quote(note.as_deref().unwrap_or_default()))
            .expect("writing to a String cannot fail");
    }
    text
}

/// `link …`.
fn link_line(clips: &[Id], group: &Option<Id>) -> String {
    let mut text = format!("link {}", id_words(clips));
    if let Some(group) = group {
        write!(text, " as={group}").expect("writing to a String cannot fail");
    }
    text
}

/// The clock of the sequence `id` of `project`.
fn sequence_clock(project: &Project, id: &Id) -> Option<Clock> {
    project.sequence(id.as_str()).map(Clock::of_sequence)
}

/// The clock of the sequence the clip `id` of `project` is on.
fn clip_clock(project: &Project, id: &Id) -> Option<Clock> {
    project
        .locate(id.as_str())
        .map(|at| Clock::of_sequence(&project.sequences[at.sequence]))
}

/// The clock a line is printed with where the project has none for it.
const FALLBACK: Clock = Clock {
    rate: Rate::FPS_24,
    origin: Time::ZERO,
    drop_frame: false,
};

/// Ids one after another, as a command takes them: `c1 c2`.
fn id_words(ids: &[Id]) -> String {
    ids.iter().map(Id::as_str).collect::<Vec<_>>().join(" ")
}

/// Ids as a field lists them: `V1,A1`.
fn id_list(ids: &[Id]) -> String {
    ids.iter().map(Id::as_str).collect::<Vec<_>>().join(",")
}

/// Changes of parameters as fields: `reset=` with those that go back to
/// their defaults, then those that are set.
fn changes_text(changes: &[(String, Option<Param>)], rate: Rate) -> String {
    let set: Params = changes
        .iter()
        .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
        .collect();
    let reset: Vec<&str> = changes
        .iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.as_str())
        .collect();
    let mut text = String::new();
    if !reset.is_empty() {
        write!(text, " reset={}", reset.join(",")).expect("writing to a String cannot fail");
    }
    text.push_str(&params_text(&set, &[], rate));
    text
}
