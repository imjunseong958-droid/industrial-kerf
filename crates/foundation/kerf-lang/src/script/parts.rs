//! The commands that add, set, and remove the parts of a project: media and
//! the steps that make them, sequences, buses and their effects, tracks, a
//! clip's properties (`set`), masks, and effects, transitions, and markers.

use super::positions::Positions;
use super::{required, Parsed, Script};
use crate::doc::{
    media_fields, parse_align, parse_marker_color, parse_mask_mode, parse_retime,
    parse_sample_rate, parse_sequence_color, parse_size, pinned, sequence_fields, take_mask_shape,
    take_role, take_sync,
};
use crate::lex::{Arg, Atom, Line};
use crate::text::{parse_fx_ref, parse_param, Clock, Fields};
use kerf_model::time::parse_timecode;
use kerf_model::{
    Clip, ClipChange, Content, Domain, Effect, Id, MarkerChange, Mask, Op, ParamDef, Params, Rate,
    SequenceChange, Time, TrackChange, TrackKind, TrackRoleChange,
};

impl Script<'_> {
    pub(super) fn media(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, relink, replace, sync, or remove")?;
        let id = fields.id(2, "a media id")?;
        let op = match action {
            "add" => Op::AddMedia(media_fields(&mut fields, id)?),
            "relink" => Op::RelinkMedia {
                media: id,
                path: fields
                    .require("path")
                    .and_then(|arg| fields.text(arg))?
                    .to_owned(),
            },
            "replace" => Op::ReplaceMedia(media_fields(&mut fields, id)?),
            "remove" => Op::RemoveMedia { media: id },
            // `with=""` lets go, as `parent=""` does.
            "sync" => {
                let arg = fields.require("with")?;
                let with = fields.text(arg)?;
                let sync = if with.is_empty() {
                    None
                } else {
                    Some(crate::doc::parse_synced(with).map_err(|why| fields.error(arg, why))?)
                };
                Op::SyncMedia { media: id, sync }
            }
            other => {
                return Err(line.error(
                    1,
                    format!("media {other}? use add, relink, replace, sync, or remove"),
                ))
            }
        };
        fields.finish(3)?;
        Ok(vec![op])
    }

    /// `step <media> add [<n>] <step>`, `step <media> set <n> <step, or name=value …>`,
    /// `step <media> remove <n>`: the steps that say how a media's file is made
    /// ([`crate::making`]), counted from 1.
    pub(super) fn step(&mut self, line: &Line) -> Parsed {
        const SHAPE: &str = "`step <media> add [<n>] <step>`, `step <media> set <n> <step, or name=value …>`, or `step <media> remove <n>`, a step being `render`, `generate`, or `join` and what follows it";
        let word = |index: usize| match line.args.get(index) {
            Some(Arg {
                key: None,
                value: Atom::Word(word),
                ..
            }) => Some(word.as_str()),
            _ => None,
        };
        let column = |index: usize| line.args.get(index).map_or(1, |arg| arg.column);
        let id = word(1)
            .and_then(|word| Id::new(word).ok())
            .ok_or_else(|| line.error(column(1), format!("expected a media id: {SHAPE}")))?;
        let media = self
            .project
            .media(id.as_str())
            .ok_or_else(|| line.missing(column(1), format!("no media `{id}`")))?;
        let steps = &media.steps;
        // The files the steps before the one at `index` make, which that one may read.
        let made = |index: usize| -> Vec<String> {
            steps
                .iter()
                .take(index)
                .map(|step| step.makes().to_owned())
                .collect()
        };
        let number = |index: usize| {
            word(index)
                .and_then(|word| word.parse::<usize>().ok())
                .filter(|number| *number >= 1)
        };
        // A step that is there, by its number.
        let there = |index: usize| {
            let number = number(index).ok_or_else(|| {
                line.error(
                    column(index),
                    format!("expected a step's number, from 1: {SHAPE}"),
                )
            })?;
            if number > steps.len() {
                return Err(line.error(
                    column(index),
                    format!(
                        "media `{id}` has {} and no step {number}",
                        match steps.len() {
                            0 => "no steps".to_owned(),
                            1 => "1 step".to_owned(),
                            more => format!("{more} steps"),
                        }
                    ),
                ));
            }
            Ok(number - 1)
        };
        let op = match word(2) {
            Some("add") => {
                let (index, from) = match number(3) {
                    Some(number) => (Some(number - 1), 4),
                    None => (None, 3),
                };
                let at = index.unwrap_or(steps.len()).min(steps.len());
                Op::AddStep {
                    step: crate::making::read(line, from, &made(at))?,
                    media: id,
                    index,
                }
            }
            Some("set") => {
                let index = there(3)?;
                let made = made(index);
                let step = match crate::making::names(line, 4)? {
                    None => crate::making::read(line, 4, &made)?,
                    Some(names) if names.is_empty() => {
                        return Err(line.error(
                            column(3),
                            format!("say what step {} is to be: names to set, as `seed=8`, or the step whole", index + 1),
                        ))
                    }
                    Some(names) => crate::making::with_names(&steps[index], &names, &made)
                        .map_err(|why| line.error(column(4), why))?,
                };
                Op::SetStep {
                    media: id,
                    index,
                    step,
                }
            }
            Some("remove") => {
                let index = there(3)?;
                if let Some(extra) = line.args.get(4) {
                    return Err(line.error(extra.column, "unexpected argument"));
                }
                Op::RemoveStep { media: id, index }
            }
            _ => {
                return Err(line.error(column(2), format!("expected add, set, or remove: {SHAPE}")))
            }
        };
        Ok(vec![op])
    }

    pub(super) fn sequence_command(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, set, or remove")?;
        let id = fields.id(2, "a sequence id")?;
        let op = match action {
            "add" => {
                let sequence = sequence_fields(&mut fields, id.clone())?;
                if self.sequence.is_none() {
                    self.sequence = Some(id);
                }
                Op::AddSequence(sequence)
            }
            "set" => {
                let existing = self
                    .project
                    .sequence(id.as_str())
                    .ok_or_else(|| line.missing(1, format!("no sequence `{id}`")))?;
                let rate =
                    fields.take_with("rate", |t| Rate::parse(t).map_err(|e| e.to_string()))?;
                let start_rate = rate.unwrap_or(existing.rate);
                Op::SetSequence {
                    sequence: id,
                    change: SequenceChange {
                        name: fields.take_text("name")?.map(str::to_owned),
                        rate,
                        size: fields.take_with("size", parse_size)?,
                        sample_rate: fields.take_with("audio", parse_sample_rate)?,
                        color: fields.take_with("color", parse_sequence_color)?,
                        start: fields.take_with("start", |t| {
                            parse_timecode(t, start_rate).map_err(|e| e.to_string())
                        })?,
                    },
                }
            }
            "remove" => Op::RemoveSequence { sequence: id },
            other => {
                return Err(line.error(1, format!("sequence {other}? use add, set, or remove")))
            }
        };
        fields.finish(3)?;
        Ok(vec![op])
    }

    /// `bus add|set|remove <id> …` and `bus fx <id> add|set|remove …`.
    pub(super) fn bus(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let sequence = self.project.sequences[s].id.clone();
        let rate = self.project.sequences[s].rate;
        let action = fields.word(1, "add, set, remove, or fx")?;
        let id = fields.id(2, "a bus id")?;
        let defs = self.catalog.props(Domain::Audio);
        let take_to = |fields: &mut Fields<'_>| -> Parsed<Option<Option<Id>>> {
            fields
                .take_text("to")?
                .map(|to| (!to.is_empty()).then(|| Id::new(to)).transpose())
                .transpose()
                .map_err(|_| line.error(1, "`to=` names a bus"))
        };
        match action {
            "add" => {
                let mut bus = kerf_model::Bus::new(id.clone());
                bus.name = fields.take_text("name")?.map(str::to_owned);
                bus.to = take_to(&mut fields)?.flatten();
                bus.params = params(&mut fields, defs, &id, rate)?;
                fields.finish(3)?;
                Ok(vec![Op::AddBus { sequence, bus }])
            }
            "set" => {
                let name = fields
                    .take_text("name")?
                    .map(|n| (!n.is_empty()).then(|| n.to_owned()));
                let to = take_to(&mut fields)?;
                let mut changes = Vec::new();
                if let Some(reset) = fields.take("reset") {
                    for property in fields.text(reset)?.split(',') {
                        changes.push((property.to_owned(), None));
                    }
                }
                changes.extend(
                    params(&mut fields, defs, &id, rate)?
                        .into_iter()
                        .map(|(k, v)| (k, Some(v))),
                );
                fields.finish(3)?;
                Ok(vec![Op::SetBus {
                    sequence,
                    bus: id,
                    change: kerf_model::BusChange {
                        name,
                        to,
                        params: changes,
                    },
                }])
            }
            "remove" => {
                fields.finish(3)?;
                Ok(vec![Op::RemoveBus { sequence, bus: id }])
            }
            "fx" => self.bus_fx(fields, sequence, id, rate),
            other => Err(line.error(1, format!("bus {other}? use add, set, remove, or fx"))),
        }
    }

    fn bus_fx(&mut self, mut fields: Fields<'_>, sequence: Id, bus: Id, rate: Rate) -> Parsed {
        let line = fields.line();
        let action = fields.word(3, "add, set, or remove")?;
        let target = fields.word(4, "an effect")?;
        let column = fields.positional()[4].column;
        let change = match action {
            "add" => {
                let (fx, version) = parse_fx_ref(target).map_err(|m| line.error(column, m))?;
                let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
                    line.error(column, kerf_model::catalog::unknown("effect", fx.as_str()))
                })?;
                let version = pinned(def, version).map_err(|m| line.error(column, m))?;
                let index = fields.take_with("at", position_index)?;
                let enabled = fields.take_bool("enabled")?.unwrap_or(true);
                let params = params(&mut fields, &def.params, &fx, rate)?;
                kerf_model::BusFx::Add {
                    effect: Effect {
                        fx,
                        version,
                        enabled,
                        params,
                    },
                    index,
                }
            }
            "set" | "remove" => {
                let found = self
                    .project
                    .sequence(sequence.as_str())
                    .and_then(|s| s.bus(bus.as_str()))
                    .ok_or_else(|| line.error(1, format!("no bus `{bus}`")))?;
                let index = position_index(target)
                    .ok()
                    .or_else(|| {
                        found
                            .effects
                            .iter()
                            .position(|effect| effect.fx.as_str() == target)
                    })
                    .filter(|index| *index < found.effects.len())
                    .ok_or_else(|| {
                        line.missing(column, format!("`{bus}` has no effect `{target}`"))
                    })?;
                if action == "remove" {
                    kerf_model::BusFx::Remove { index }
                } else {
                    let fx = found.effects[index].fx.clone();
                    let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
                        line.error(column, kerf_model::catalog::unknown("effect", fx.as_str()))
                    })?;
                    let enabled = fields.take_bool("enabled")?;
                    let mut changes = Vec::new();
                    if let Some(reset) = fields.take("reset") {
                        for property in fields.text(reset)?.split(',') {
                            changes.push((property.to_owned(), None));
                        }
                    }
                    changes.extend(
                        params(&mut fields, &def.params, &fx, rate)?
                            .into_iter()
                            .map(|(k, v)| (k, Some(v))),
                    );
                    kerf_model::BusFx::Set {
                        index,
                        enabled,
                        params: changes,
                    }
                }
            }
            other => return Err(line.error(1, format!("bus fx {other}? use add, set, or remove"))),
        };
        fields.finish(5)?;
        Ok(vec![Op::BusFx {
            sequence,
            bus,
            change,
        }])
    }

    pub(super) fn track(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let sequence = self.project.sequences[s].id.clone();
        let action = fields.word(1, "add, set, or remove")?;
        let id = fields.id(2, "a track id")?;
        let mut ops = Vec::new();
        let mut used = 3;
        match action {
            "add" => {
                let kind_word = fields.word(3, "a track kind (video, audio, subtitle)")?;
                let kind = TrackKind::parse(kind_word).ok_or_else(|| {
                    line.error(
                        fields.positional()[3].column,
                        format!("`{kind_word}` is not video, audio, or subtitle"),
                    )
                })?;
                used = 4;
                let tracks = &self.project.sequences[s].tracks;
                let position = |anchor: Id, offset: usize| {
                    tracks
                        .iter()
                        .position(|t| t.id == anchor)
                        .map(|i| i + offset)
                        .ok_or_else(|| line.missing(1, format!("no track `{anchor}`")))
                };
                let index = match (fields.take_id("before")?, fields.take_id("after")?) {
                    (Some(_), Some(_)) => {
                        return Err(line.error(1, "give before= or after=, not both"))
                    }
                    (Some(anchor), None) => Some(position(anchor, 0)?),
                    (None, Some(anchor)) => Some(position(anchor, 1)?),
                    (None, None) => None,
                };
                ops.push(Op::AddTrack {
                    sequence: sequence.clone(),
                    id: id.clone(),
                    kind,
                    index,
                });
            }
            "set" => {}
            "remove" => ops.push(Op::RemoveTrack {
                sequence: sequence.clone(),
                track: id.clone(),
            }),
            other => return Err(line.error(1, format!("track {other}? use add, set, or remove"))),
        }
        if action != "remove" {
            let change = TrackChange {
                name: fields
                    .take_text("name")?
                    .map(|n| (!n.is_empty()).then(|| n.to_owned())),
                enabled: fields.take_bool("enabled")?,
                locked: fields.take_bool("locked")?,
                muted: fields.take_bool("muted")?,
                bus: fields
                    .take_text("bus")?
                    .map(|bus| (!bus.is_empty()).then(|| Id::new(bus)).transpose())
                    .transpose()
                    .map_err(|_| line.error(1, "`bus=` names a bus"))?,
            };
            if change != TrackChange::default() {
                ops.push(Op::SetTrack {
                    sequence: sequence.clone(),
                    track: id.clone(),
                    change,
                });
            }
            let role = TrackRoleChange {
                role: take_role(&mut fields)?,
                sync_locked: take_sync(&mut fields)?,
            };
            if role != TrackRoleChange::default() {
                ops.push(Op::SetTrackRole {
                    sequence,
                    track: id,
                    change: role,
                });
            }
        }
        fields.finish(used)?;
        Ok(ops)
    }

    fn domain_of(&self, clip: &Id) -> Option<Domain> {
        let at = self.project.locate(clip.as_str())?;
        self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain()
    }

    pub(super) fn set(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        let generator = match &found.content {
            Content::Generator { fx, .. } => self.catalog.fx(fx.as_str()),
            _ => None,
        };
        let domain = self.domain_of(&clip);
        let lookup = |name: &str| -> Option<&ParamDef> {
            domain
                .and_then(|d| self.catalog.prop(d, name))
                .or_else(|| generator.and_then(|g| g.find(name)))
        };
        let known = || {
            let mut names: Vec<&str> = domain
                .map(|d| {
                    self.catalog
                        .props(d)
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect()
                })
                .unwrap_or_default();
            names.extend(
                generator
                    .into_iter()
                    .flat_map(|g| g.params.iter().map(|p| p.name.as_str())),
            );
            names.extend([
                "name", "enabled", "caption", "parent", "retime", "angle", "reset",
            ]);
            names.join(", ")
        };
        let mut changes = Vec::new();
        if let Some(name) = fields.take_text("name")? {
            changes.push(ClipChange::Name(
                (!name.is_empty()).then(|| name.to_owned()),
            ));
        }
        if let Some(enabled) = fields.take_bool("enabled")? {
            changes.push(ClipChange::Enabled(enabled));
        }
        if let Some(text) = fields.take_text("caption")? {
            changes.push(ClipChange::Caption(text.to_owned()));
        }
        if let Some(parent) = fields.take("parent") {
            let text = fields.text(parent)?;
            changes.push(ClipChange::Parent(if text.is_empty() {
                None
            } else {
                Some(Id::new(text).map_err(|error| fields.error(parent, error.to_string()))?)
            }));
        }
        if let Some(angle) = fields.take_text("angle")? {
            let angle = (!angle.is_empty())
                .then(|| Id::new(angle))
                .transpose()
                .map_err(|_| line.error(1, format!("`{angle}` is not a track id")))?;
            changes.push(ClipChange::Angle(angle));
        }
        if let Some(retime) = fields.take_with("retime", parse_retime)? {
            changes.push(ClipChange::Retime(retime));
        }
        if let Some(reset) = fields.take("reset") {
            for name in fields.text(reset)?.split(',') {
                if lookup(name).is_none() {
                    return Err(
                        fields.error(reset, format!("no property `{name}` (has: {})", known()))
                    );
                }
                changes.push(ClipChange::Param(name.to_owned(), None));
            }
        }
        for (key, arg) in fields.rest() {
            let def = lookup(key).ok_or_else(|| {
                fields.error(arg, format!("no property `{key}` (has: {})", known()))
            })?;
            let param =
                parse_param(&arg.value, def, clock.rate).map_err(|m| fields.error(arg, m))?;
            changes.push(ClipChange::Param(key.to_owned(), Some(param)));
        }
        fields.finish(2)?;
        if changes.is_empty() {
            return Err(line.error(
                1,
                format!("set needs at least one change (has: {})", known()),
            ));
        }
        Ok(vec![Op::Set { clip, changes }])
    }

    pub(super) fn mask(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (_, clock) = self.clip(&clip, line)?;
        let action = fields.word(2, "add, set, or remove")?;
        let name = fields.id(3, "a mask name")?;
        let defs = self.catalog.mask_props();
        let op = match action {
            "add" => {
                let shape = take_mask_shape(&mut fields, clock.rate)?
                    .ok_or_else(|| line.error(1, "mask add needs rect=, ellipse=, or points="))?;
                let index = fields.take_with("at", position_index)?;
                let mode = fields
                    .take_with("mode", parse_mask_mode)?
                    .unwrap_or_default();
                let inverted = fields.take_bool("inverted")?.unwrap_or(false);
                let params = params(&mut fields, defs, &name, clock.rate)?;
                Op::MaskAdd {
                    clip,
                    mask: Mask {
                        name,
                        shape,
                        mode,
                        inverted,
                        params,
                    },
                    index,
                }
            }
            "set" => {
                let shape = take_mask_shape(&mut fields, clock.rate)?;
                let mode = fields.take_with("mode", parse_mask_mode)?;
                let inverted = fields.take_bool("inverted")?;
                let mut changes = Vec::new();
                if let Some(reset) = fields.take("reset") {
                    for property in fields.text(reset)?.split(',') {
                        changes.push((property.to_owned(), None));
                    }
                }
                changes.extend(
                    params(&mut fields, defs, &name, clock.rate)?
                        .into_iter()
                        .map(|(k, v)| (k, Some(v))),
                );
                Op::MaskSet {
                    clip,
                    name,
                    shape,
                    mode,
                    inverted,
                    params: changes,
                }
            }
            "remove" => Op::MaskRemove { clip, name },
            other => return Err(line.error(1, format!("mask {other}? use add, set, or remove"))),
        };
        fields.finish(4)?;
        Ok(vec![op])
    }

    pub(super) fn fx(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        let action = fields.word(2, "add, set, remove, move, or flatten")?;
        let target_word = fields.word(
            3,
            if action == "add" {
                "an effect id"
            } else {
                "an effect position or id"
            },
        )?;
        let target_column = fields.positional()[3].column;
        if action == "add" {
            let (fx, version) =
                parse_fx_ref(target_word).map_err(|m| line.error(target_column, m))?;
            let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
                line.error(
                    target_column,
                    kerf_model::catalog::unknown("effect", fx.as_str()),
                )
            })?;
            let version = pinned(def, version).map_err(|m| line.error(target_column, m))?;
            let index = fields.take_with("at", position_index)?;
            let enabled = fields.take_bool("enabled")?.unwrap_or(true);
            let params = params(&mut fields, &def.params, &fx, clock.rate)?;
            fields.finish(4)?;
            return Ok(vec![Op::FxAdd {
                clip,
                effect: Effect {
                    fx,
                    version,
                    enabled,
                    params,
                },
                index,
            }]);
        }
        let index = effect_index(found, target_word).map_err(|m| line.error(target_column, m))?;
        let fx = found.effects[index].fx.clone();
        let op = match action {
            "set" => {
                let def = self.catalog.fx(fx.as_str());
                let defs = def.map(|d| d.params.as_slice()).unwrap_or_default();
                let enabled = fields.take_bool("enabled")?;
                let mut changes = Vec::new();
                if let Some(reset) = fields.take("reset") {
                    for name in fields.text(reset)?.split(',') {
                        changes.push((name.to_owned(), None));
                    }
                }
                changes.extend(
                    params(&mut fields, defs, &fx, clock.rate)?
                        .into_iter()
                        .map(|(k, v)| (k, Some(v))),
                );
                Op::FxSet {
                    clip,
                    index,
                    enabled,
                    params: changes,
                }
            }
            "remove" => Op::FxRemove { clip, index },
            "flatten" => Op::FxFlatten { clip, index },
            "move" => Op::FxMove {
                clip,
                index,
                to: required(&mut fields, "to", position_index)?,
            },
            other => {
                return Err(line.error(
                    1,
                    format!("fx {other}? use add, set, remove, move, or flatten"),
                ))
            }
        };
        fields.finish(4)?;
        Ok(vec![op])
    }

    pub(super) fn transition(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, set, or remove")?;
        let op = match action {
            "add" => {
                let id = fields.take_id("as")?;
                let from = fields
                    .take_id("from")?
                    .ok_or_else(|| line.error(1, "missing `from=`"))?;
                let to = fields
                    .take_id("to")?
                    .ok_or_else(|| line.error(1, "missing `to=`"))?;
                let (fx, version) = fields
                    .take_fx_ref("fx")?
                    .ok_or_else(|| line.error(1, "missing `fx=`"))?;
                let (_, clock) = self.clip(&from, line)?;
                let duration = required(&mut fields, "dur", |t| clock.duration(t))?;
                let align = fields.take_with("align", parse_align)?.unwrap_or_default();
                let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
                    line.error(1, kerf_model::catalog::unknown("transition", fx.as_str()))
                })?;
                let version = pinned(def, version).map_err(|m| line.error(1, m))?;
                let params = params(&mut fields, &def.params, &fx, clock.rate)?;
                fields.finish(2)?;
                return Ok(vec![Op::AddTransition {
                    id,
                    from,
                    to,
                    fx,
                    version,
                    duration,
                    align,
                    params,
                }]);
            }
            "set" | "remove" => {
                let id = fields.id(2, "a transition id")?;
                let (s, transition) = self
                    .project
                    .sequences
                    .iter()
                    .enumerate()
                    .find_map(|(s, seq)| {
                        seq.tracks
                            .iter()
                            .find_map(|t| t.transition(id.as_str()))
                            .map(|tr| (s, tr))
                    })
                    .ok_or_else(|| line.missing(1, format!("no transition `{id}`")))?;
                if action == "remove" {
                    Op::RemoveTransition { id }
                } else {
                    let clock = Clock::of_sequence(&self.project.sequences[s]);
                    let duration = fields.take_with("dur", |t| clock.duration(t))?;
                    let align = fields.take_with("align", parse_align)?;
                    let fx = transition.fx.clone();
                    let mut changes = Vec::new();
                    if let Some(reset) = fields.take("reset") {
                        for name in fields.text(reset)?.split(',') {
                            changes.push((name.to_owned(), None));
                        }
                    }
                    let defs = self
                        .catalog
                        .fx(fx.as_str())
                        .map(|d| d.params.as_slice())
                        .unwrap_or_default();
                    changes.extend(
                        params(&mut fields, defs, &fx, clock.rate)?
                            .into_iter()
                            .map(|(k, v)| (k, Some(v))),
                    );
                    Op::SetTransition {
                        id,
                        duration,
                        align,
                        params: changes,
                    }
                }
            }
            other => {
                return Err(line.error(1, format!("transition {other}? use add, set, or remove")))
            }
        };
        fields.finish(3)?;
        Ok(vec![op])
    }

    pub(super) fn marker(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, set, or remove")?;
        let (op, used) = match action {
            "add" => {
                let s = self.current(line)?;
                let clock = Positions::of(&self.project, s);
                let op = Op::AddMarker {
                    sequence: self.project.sequences[s].id.clone(),
                    id: fields.take_id("as")?,
                    at: required(&mut fields, "at", |t| clock.position(t))?,
                    duration: fields
                        .take_with("dur", |t| clock.duration(t))?
                        .unwrap_or(Time::ZERO),
                    name: fields.take_text("name")?.map(str::to_owned),
                    color: fields.take_with("color", parse_marker_color)?,
                    note: fields.take_text("note")?.map(str::to_owned),
                };
                (op, 2)
            }
            "set" | "remove" => {
                let id = fields.id(2, "a marker id")?;
                let s = self
                    .project
                    .sequences
                    .iter()
                    .position(|seq| seq.marker(id.as_str()).is_some())
                    .ok_or_else(|| line.missing(1, format!("no marker `{id}`")))?;
                if action == "remove" {
                    (Op::RemoveMarker { id }, 3)
                } else {
                    let clock = Positions::of(&self.project, s);
                    let optional = |text: &str| (!text.is_empty()).then(|| text.to_owned());
                    let change = MarkerChange {
                        at: fields.take_with("at", |t| clock.position(t))?,
                        duration: fields.take_with("dur", |t| clock.duration(t))?,
                        name: fields.take_text("name")?.map(optional),
                        color: fields.take_with("color", |t| {
                            if t == "none" {
                                Ok(None)
                            } else {
                                parse_marker_color(t).map(Some)
                            }
                        })?,
                        note: fields.take_text("note")?.map(optional),
                    };
                    (Op::SetMarker { id, change }, 3)
                }
            }
            other => return Err(line.error(1, format!("marker {other}? use add, set, or remove"))),
        };
        fields.finish(used)?;
        Ok(vec![op])
    }
}

/// A 1-based stack position.
fn position_index(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
        .map(|n| n - 1)
        .ok_or_else(|| format!("`{text}` is not a stack position (1, 2, …)"))
}

/// An effect by 1-based position, or by id when it appears once.
fn effect_index(clip: &Clip, target: &str) -> Result<usize, String> {
    if let Ok(index) = position_index(target) {
        return if index < clip.effects.len() {
            Ok(index)
        } else {
            Err(format!("`{}` has {} effects", clip.id, clip.effects.len()))
        };
    }
    // An id may name its version too, as `blur.gaussian@1`.
    let (id, version) = parse_fx_ref(target)?;
    let matches: Vec<usize> = (0..clip.effects.len())
        .filter(|i| {
            let effect = &clip.effects[*i];
            effect.fx == id && version.is_none_or(|version| effect.version == version)
        })
        .collect();
    match matches[..] {
        [index] => Ok(index),
        [] => Err(format!("`{}` has no `{target}` effect", clip.id)),
        _ => Err(format!(
            "`{}` has {} `{target}` effects; name one by position ({})",
            clip.id,
            matches.len(),
            matches
                .iter()
                .map(|i| (i + 1).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn params(fields: &mut Fields<'_>, defs: &[ParamDef], fx: &Id, rate: Rate) -> Parsed<Params> {
    let mut out = Params::new();
    for (key, arg) in fields.rest() {
        let def = defs.iter().find(|d| d.name == key).ok_or_else(|| {
            let known: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            fields.error(
                arg,
                format!(
                    "`{fx}` has no parameter `{key}` (has: {})",
                    known.join(", ")
                ),
            )
        })?;
        out.insert(
            key.to_owned(),
            parse_param(&arg.value, def, rate).map_err(|m| fields.error(arg, m))?,
        );
    }
    Ok(out)
}
