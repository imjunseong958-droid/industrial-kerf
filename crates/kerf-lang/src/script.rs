//! Edit scripts: one operation per line, in the document's vocabulary.
//!
//! ```text
//! insert media=m1 src=14:22:35:00 dur=00:00:04:00 at=01:00:06:12 on=V1,A1 as=c9,a9
//! trim c9 out -6f
//! roll c1 +12f
//! set c9 opacity={0f:0 12f:1}
//! fx c9 add color.basic exposure=0.3
//! transition add from=c1 to=c9 fx=dissolve dur=12f
//! ```
//!
//! Lines run in order against the project as it stands after the previous
//! line, so later lines may name clips that earlier lines created. The whole
//! script applies atomically, and errors point at the line that failed.

use crate::doc::{
    content, content_text, mask_shape_text, mask_text, media_fields, media_line, params_text,
    parse_align, parse_marker_color, parse_mask_mode, parse_sample_rate, parse_size, pinned,
    sequence_fields, sequence_line, take_mask_shape,
};
use crate::lex::{lex, Arg, Atom, Line};
use crate::text::{parse_fx_ref, parse_param, print_fx_ref, quote, Clock, Fields};
use kerf_model::time::parse_timecode;
use kerf_model::{
    apply, Catalog, Clip, ClipChange, Content, Diagnostic, Domain, Edge, Effect, Id, MarkerChange,
    Mask, NewClip, Op, ParamDef, Params, Project, Rate, Ratio, Report, Sequence, SequenceChange,
    Time, TrackChange, TrackKind,
};
use std::fmt::Write as _;

/// The result of a script: the new project, the operations it ran (ready to
/// journal or replay), and what they did.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub project: Project,
    pub ops: Vec<Op>,
    pub report: Report,
}

pub fn apply_script(
    project: &Project,
    catalog: &Catalog,
    script: &str,
) -> Result<Applied, Vec<Diagnostic>> {
    let lines = lex(script)?;
    let mut state = Script {
        project: project.clone(),
        catalog,
        sequence: None,
    };
    let mut ops = Vec::new();
    for line in &lines {
        let line_ops = state.parse(line).map_err(|error| vec![error])?;
        if line_ops.is_empty() {
            continue;
        }
        match apply(&state.project, catalog, &line_ops) {
            Ok((next, _)) => state.project = next,
            Err(rejected) => {
                return Err(rejected
                    .diagnostics
                    .into_iter()
                    .map(|d| {
                        if d.span.is_some() {
                            d
                        } else {
                            d.at(line.number, 1)
                        }
                    })
                    .collect())
            }
        }
        ops.extend(line_ops);
    }
    let (project, report) =
        apply(project, catalog, &ops).map_err(|rejected| rejected.diagnostics)?;
    Ok(Applied {
        project,
        ops,
        report,
    })
}

const COMMANDS: &str = "use, media, sequence, track, insert, overwrite, delete, ripple-delete, lift, extract, trim, roll, slip, slide, split, blade, move, set, speed, mask, fx, transition, marker, link, unlink";

struct Script<'c> {
    project: Project,
    catalog: &'c Catalog,
    sequence: Option<Id>,
}

type Parsed<T = Vec<Op>> = Result<T, Diagnostic>;

/// Positions on one sequence. Besides timecode, frames, and seconds, a
/// position may be `end`, the end of the sequence, or `<clip>.start` or
/// `<clip>.end` of a clip on it, each with an optional amount: `c3.end-6f`.
struct Positions<'a> {
    clock: Clock,
    sequence: &'a Sequence,
}

impl<'a> Positions<'a> {
    fn of(sequence: &'a Sequence) -> Self {
        Self {
            clock: Clock::of_sequence(sequence),
            sequence,
        }
    }

    fn position(&self, text: &str) -> Result<Time, String> {
        let anchor = [(".start", false), (".end", true)]
            .into_iter()
            .find_map(|(suffix, end)| {
                let at = text.find(suffix)?;
                Some((&text[..at], end, &text[at + suffix.len()..]))
            });
        let (base, rest) = match anchor {
            Some((id, end, rest)) => {
                let clip = self
                    .sequence
                    .tracks
                    .iter()
                    .find_map(|track| track.clip(id))
                    .ok_or_else(|| format!("no clip `{id}` in sequence `{}`", self.sequence.id))?;
                (if end { clip.end() } else { clip.start }, rest)
            }
            None => match text.strip_prefix("end") {
                Some(rest) => (self.sequence.duration(), rest),
                None => return self.clock.position(text),
            },
        };
        match rest {
            "" => Ok(base),
            amount if amount.starts_with(['+', '-']) => Ok(base + self.clock.delta(amount)?),
            _ => Err(format!(
                "`{text}`: after an anchor comes an amount such as +12f or -0.5s"
            )),
        }
    }
}

/// Everything else a sequence clock reads and prints.
impl std::ops::Deref for Positions<'_> {
    type Target = Clock;

    fn deref(&self) -> &Clock {
        &self.clock
    }
}

impl Script<'_> {
    fn parse(&mut self, line: &Line) -> Parsed {
        let mut fields = Fields::new(line)?;
        let command = fields.word(0, "a command")?;
        match command {
            "use" => {
                let id = fields.id(1, "a sequence id")?;
                if self.project.sequence(id.as_str()).is_none() {
                    return Err(
                        line.error(fields.positional()[1].column, format!("no sequence `{id}`"))
                    );
                }
                fields.finish(2)?;
                self.sequence = Some(id);
                Ok(Vec::new())
            }
            "media" => self.media(fields),
            "sequence" => self.sequence_command(fields),
            "track" => self.track(fields),
            "insert" => self.place(fields, true),
            "overwrite" => self.place(fields, false),
            "delete" | "ripple-delete" | "unlink" => {
                let clips = clip_list(&fields, 1)?;
                let count = fields.positional().len();
                fields.finish(count)?;
                Ok(vec![match command {
                    "delete" => Op::Delete { clips },
                    "ripple-delete" => Op::RippleDelete { clips },
                    _ => Op::Unlink { clips },
                }])
            }
            "link" => {
                let clips = clip_list(&fields, 1)?;
                let group = fields.take_id("as")?;
                let count = fields.positional().len();
                fields.finish(count)?;
                Ok(vec![Op::Link { clips, group }])
            }
            "lift" | "extract" => {
                let s = self.current(line)?;
                let clock = Positions::of(&self.project.sequences[s]);
                let from = required(&mut fields, "from", |t| clock.position(t))?;
                let to = required(&mut fields, "to", |t| clock.position(t))?;
                let tracks = fields.take_ids("tracks")?;
                fields.finish(1)?;
                let sequence = self.project.sequences[s].id.clone();
                Ok(vec![if command == "lift" {
                    Op::Lift {
                        sequence,
                        from,
                        to,
                        tracks,
                    }
                } else {
                    Op::Extract {
                        sequence,
                        from,
                        to,
                        tracks,
                    }
                }])
            }
            "trim" => self.trim(fields),
            "roll" | "slip" | "slide" => {
                let clip = fields.id(1, "a clip id")?;
                let (found, clock) = self.clip(&clip, line)?;
                let to = fields.take_with("to", |t| clock.position(t))?;
                if to.is_some() && fields.positional().len() > 2 {
                    return Err(
                        fields.error(fields.positional()[2], "give an amount or to=, not both")
                    );
                }
                let delta = match to {
                    Some(to) if command == "roll" => to - found.end(),
                    Some(to) if command == "slide" => to - found.start,
                    Some(_) => return Err(line.error(1, "slip takes an amount like +12f, not to=")),
                    None => clock
                        .delta(fields.word(2, "an amount like +12f or -0.5s")?)
                        .map_err(|m| line.error(fields.positional()[2].column, m))?,
                };
                let used = if fields.positional().len() > 2 { 3 } else { 2 };
                fields.finish(used)?;
                Ok(vec![match command {
                    "roll" => Op::Roll { clip, delta },
                    "slip" => Op::Slip { clip, delta },
                    _ => Op::Slide { clip, delta },
                }])
            }
            "split" => {
                let clip = fields.id(1, "a clip id")?;
                let (_, clock) = self.clip(&clip, line)?;
                let at = required(&mut fields, "at", |t| clock.position(t))?;
                let tail = fields.take_id("as")?;
                fields.finish(2)?;
                Ok(vec![Op::Split { clip, at, tail }])
            }
            "blade" => {
                let s = self.current(line)?;
                let clock = Positions::of(&self.project.sequences[s]);
                let at = required(&mut fields, "at", |t| clock.position(t))?;
                let tracks = fields.take_ids("tracks")?;
                fields.finish(1)?;
                Ok(vec![Op::Blade {
                    sequence: self.project.sequences[s].id.clone(),
                    at,
                    tracks,
                }])
            }
            "move" => {
                let clip = fields.id(1, "a clip id")?;
                let (found, clock) = self.clip(&clip, line)?;
                let track = fields.take_id("on")?;
                let at = fields.take_with("at", |t| clock.position(t))?;
                let by = fields.take_with("by", |t| clock.delta(t))?;
                let delta = match (at, by) {
                    (Some(_), Some(_)) => return Err(line.error(1, "give at= or by=, not both")),
                    (Some(at), None) => at - found.start,
                    (None, Some(by)) => by,
                    (None, None) if track.is_some() => Time::ZERO,
                    (None, None) => {
                        return Err(
                            line.error(1, "move needs on=<track>, at=<time>, or by=<amount>")
                        )
                    }
                };
                fields.finish(2)?;
                Ok(vec![Op::Move { clip, track, delta }])
            }
            "set" => self.set(fields),
            "speed" => {
                let clip = fields.id(1, "a clip id")?;
                let speed = fields.word(2, "a speed such as 2, 0.5, or -1")?;
                let speed = Ratio::parse(speed)
                    .map_err(|e| line.error(fields.positional()[2].column, e.to_string()))?;
                fields.finish(3)?;
                Ok(vec![Op::Speed { clip, speed }])
            }
            "mask" => self.mask(fields),
            "fx" => self.fx(fields),
            "transition" => self.transition(fields),
            "marker" => self.marker(fields),
            other => Err(line.error(
                1,
                format!("unknown command `{other}`; commands are {COMMANDS}"),
            )),
        }
    }

    /// The sequence that `use` selected, or the only one.
    fn current(&self, line: &Line) -> Parsed<usize> {
        match &self.sequence {
            Some(id) => self
                .project
                .sequence_index(id.as_str())
                .ok_or_else(|| line.error(1, format!("sequence `{id}` no longer exists"))),
            None => match self.project.sequences.len() {
                1 => Ok(0),
                0 => Err(line.error(1, "there is no sequence yet; add one with `sequence add`")),
                _ => Err(line.error(
                    1,
                    "the project has several sequences; choose one with `use <id>`",
                )),
            },
        }
    }

    fn clip(&self, id: &Id, line: &Line) -> Parsed<(&Clip, Positions<'_>)> {
        let at = self
            .project
            .locate(id.as_str())
            .ok_or_else(|| line.error(1, format!("no clip `{id}`")))?;
        Ok((
            self.project.clip_at(at),
            Positions::of(&self.project.sequences[at.sequence]),
        ))
    }

    fn domain_of(&self, clip: &Id) -> Option<Domain> {
        let at = self.project.locate(clip.as_str())?;
        self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain()
    }

    fn media(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, relink, or remove")?;
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
            "remove" => Op::RemoveMedia { media: id },
            other => {
                return Err(line.error(1, format!("media {other}? use add, relink, or remove")))
            }
        };
        fields.finish(3)?;
        Ok(vec![op])
    }

    fn sequence_command(&mut self, mut fields: Fields<'_>) -> Parsed {
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
                    .ok_or_else(|| line.error(1, format!("no sequence `{id}`")))?;
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

    fn track(&mut self, mut fields: Fields<'_>) -> Parsed {
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
                        .ok_or_else(|| line.error(1, format!("no track `{anchor}`")))
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
            };
            if change != TrackChange::default() {
                ops.push(Op::SetTrack {
                    sequence,
                    track: id,
                    change,
                });
            }
        }
        fields.finish(used)?;
        Ok(ops)
    }

    fn place(&mut self, mut fields: Fields<'_>, ripple: bool) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let (mut content, source) = content(&mut fields, &self.project, self.catalog)?;
        let positions = Positions::of(sequence);
        let at = required(&mut fields, "at", |t| positions.position(t))?;
        let duration = match (
            fields.take_with("dur", |t| clock.duration(t))?,
            fields.take("out"),
        ) {
            (Some(_), Some(_)) => return Err(line.error(1, "give dur= or out=, not both")),
            (Some(duration), None) => duration,
            (None, Some(out)) => {
                let source =
                    source.ok_or_else(|| fields.error(out, "out= needs media= or seq="))?;
                let out_time = source
                    .position(fields.text(out)?)
                    .map_err(|m| fields.error(out, m))?;
                let (Content::Media {
                    source_in, speed, ..
                }
                | Content::Sequence {
                    source_in, speed, ..
                }) = &content
                else {
                    unreachable!("a source clock implies source content")
                };
                let span = out_time - *source_in;
                if !span.is_positive() {
                    return Err(fields.error(out, "out= must come after src="));
                }
                let frames = clock.rate.frames_round(speed.timeline_span(span)).max(1);
                clock.rate.time_of(frames)
            }
            (None, None) => {
                return Err(line.error(1, "missing `dur=` (or `out=` for a source out point)"))
            }
        };
        let tracks = fields.take_ids("on")?.ok_or_else(|| {
            line.error(
                1,
                "missing `on=` with the target track, such as on=V1 or on=V1,A1",
            )
        })?;
        let names = fields.take_ids("as")?.unwrap_or_default();
        if names.len() > tracks.len() {
            return Err(line.error(1, "as= names more clips than on= has tracks"));
        }
        let name = fields.take_text("name")?.map(str::to_owned);
        let domains: Vec<Option<Domain>> = tracks
            .iter()
            .map(|track| sequence.track(track.as_str()).and_then(|t| t.kind.domain()))
            .collect();
        let mut props: Vec<Params> = vec![Params::new(); tracks.len()];
        for (key, arg) in fields.rest() {
            if let Content::Generator { fx, params, .. } = &mut content {
                if let Some(def) = self.catalog.fx(fx.as_str()).and_then(|d| d.find(key)) {
                    params.insert(
                        key.to_owned(),
                        parse_param(&arg.value, def, clock.rate)
                            .map_err(|m| fields.error(arg, m))?,
                    );
                    continue;
                }
            }
            let mut accepted = false;
            for (target, domain) in props.iter_mut().zip(&domains) {
                if let Some(def) = domain.and_then(|d| self.catalog.prop(d, key)) {
                    target.insert(
                        key.to_owned(),
                        parse_param(&arg.value, def, clock.rate)
                            .map_err(|m| fields.error(arg, m))?,
                    );
                    accepted = true;
                }
            }
            if !accepted {
                return Err(fields.error(arg, format!("no target track takes `{key}`")));
            }
        }
        fields.finish(1)?;
        let targets = tracks
            .into_iter()
            .zip(props)
            .enumerate()
            .map(|(i, (track, props))| {
                (
                    track,
                    NewClip {
                        id: names.get(i).cloned(),
                        content: content.clone(),
                        name: name.clone(),
                        props,
                    },
                )
            })
            .collect();
        let sequence = sequence.id.clone();
        Ok(vec![if ripple {
            Op::Insert {
                sequence,
                at,
                duration,
                targets,
            }
        } else {
            Op::Overwrite {
                sequence,
                at,
                duration,
                targets,
            }
        }])
    }

    fn trim(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let edge = match fields.word(2, "in or out")? {
            "in" => Edge::In,
            "out" => Edge::Out,
            other => {
                return Err(line.error(
                    fields.positional()[2].column,
                    format!("trim {other}? use in or out"),
                ))
            }
        };
        let (found, clock) = self.clip(&clip, line)?;
        let edge_time = if edge == Edge::In {
            found.start
        } else {
            found.end()
        };
        let rest: Vec<&Arg> = fields.positional()[3..].to_vec();
        let ripple = rest
            .iter()
            .any(|arg| arg.value == Atom::Word("ripple".into()));
        let amounts: Vec<&&Arg> = rest
            .iter()
            .filter(|arg| arg.value != Atom::Word("ripple".into()))
            .collect();
        let to = match (
            fields.take_with("to", |t| clock.position(t))?,
            fields.take("src"),
        ) {
            (Some(_), Some(_)) => return Err(line.error(1, "give to= or src=, not both")),
            (Some(to), None) => Some(to),
            (None, Some(src)) => {
                let text = fields.text(src)?;
                Some(
                    self.source_edge(found, clock.rate, text)
                        .map_err(|m| fields.error(src, m))?,
                )
            }
            (None, None) => None,
        };
        let delta = match (to, amounts.as_slice()) {
            (Some(to), []) => to - edge_time,
            (None, [amount]) => {
                let text = fields.text(amount)?;
                clock.delta(text).map_err(|m| fields.error(amount, m))?
            }
            (Some(_), _) => return Err(line.error(1, "give an amount, to=, or src=, not more")),
            (None, []) => {
                return Err(line.error(
                    1,
                    "trim needs an amount like -6f, to=<time>, or src=<source time>",
                ))
            }
            (None, [_, extra, ..]) => return Err(fields.error(extra, "unexpected argument")),
        };
        let count = fields.positional().len();
        fields.finish(count)?;
        Ok(vec![Op::Trim {
            clip,
            edge,
            delta,
            ripple,
        }])
    }

    /// Where on the sequence `clip` plays the source position `text`, on
    /// the nearest frame: where a trim to that source position puts the
    /// edge.
    fn source_edge(&self, clip: &Clip, rate: Rate, text: &str) -> Result<Time, String> {
        let (source, source_in, speed) = match &clip.content {
            Content::Media {
                media,
                source_in,
                speed,
            } => {
                let media = self
                    .project
                    .media(media.as_str())
                    .ok_or_else(|| format!("no media `{media}`"))?;
                (Clock::of_media(media), *source_in, *speed)
            }
            Content::Sequence {
                sequence,
                source_in,
                speed,
            } => {
                let nested = self
                    .project
                    .sequence(sequence.as_str())
                    .ok_or_else(|| format!("no sequence `{sequence}`"))?;
                (Clock::of_sequence(nested), *source_in, *speed)
            }
            _ => return Err(format!("clip `{}` has no source; use to=", clip.id)),
        };
        if speed.is_reverse() {
            return Err(format!("clip `{}` plays in reverse; use to=", clip.id));
        }
        let span = source.position(text)? - source_in;
        let offset = if span.is_negative() {
            -speed.timeline_span(-span)
        } else {
            speed.timeline_span(span)
        };
        Ok(rate.round(clip.start + offset))
    }

    fn set(&mut self, mut fields: Fields<'_>) -> Parsed {
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
            names.extend(["name", "enabled", "caption", "reset"]);
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

    fn mask(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (_, clock) = self.clip(&clip, line)?;
        let action = fields.word(2, "add, set, or remove")?;
        let name = fields.id(3, "a mask name")?;
        let defs = self.catalog.mask_props();
        let op = match action {
            "add" => {
                let shape = take_mask_shape(&mut fields)?
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
                let shape = take_mask_shape(&mut fields)?;
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

    fn fx(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        let action = fields.word(2, "add, set, remove, or move")?;
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
            let def = self
                .catalog
                .fx(fx.as_str())
                .ok_or_else(|| line.error(target_column, format!("unknown effect `{fx}`")))?;
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
            "move" => Op::FxMove {
                clip,
                index,
                to: required(&mut fields, "to", position_index)?,
            },
            other => {
                return Err(line.error(1, format!("fx {other}? use add, set, remove, or move")))
            }
        };
        fields.finish(4)?;
        Ok(vec![op])
    }

    fn transition(&mut self, mut fields: Fields<'_>) -> Parsed {
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
                let def = self
                    .catalog
                    .fx(fx.as_str())
                    .ok_or_else(|| line.error(1, format!("unknown transition `{fx}`")))?;
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
                    .ok_or_else(|| line.error(1, format!("no transition `{id}`")))?;
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

    fn marker(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let action = fields.word(1, "add, set, or remove")?;
        let (op, used) = match action {
            "add" => {
                let s = self.current(line)?;
                let clock = Positions::of(&self.project.sequences[s]);
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
                    .ok_or_else(|| line.error(1, format!("no marker `{id}`")))?;
                if action == "remove" {
                    (Op::RemoveMarker { id }, 3)
                } else {
                    let clock = Positions::of(&self.project.sequences[s]);
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

fn required<T>(
    fields: &mut Fields<'_>,
    key: &'static str,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Parsed<T> {
    let line = fields.line();
    fields
        .take_with(key, parse)?
        .ok_or_else(|| line.error(1, format!("missing `{key}=`")))
}

fn clip_list(fields: &Fields<'_>, from: usize) -> Parsed<Vec<Id>> {
    let count = fields.positional().len();
    if count <= from {
        return Err(fields.line().error(1, "name at least one clip"));
    }
    (from..count).map(|i| fields.id(i, "a clip id")).collect()
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

// ---- printing ---------------------------------------------------------------

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
        | Op::RemoveTrack { sequence, .. }
        | Op::Insert { sequence, .. }
        | Op::Overwrite { sequence, .. }
        | Op::Lift { sequence, .. }
        | Op::Extract { sequence, .. }
        | Op::Blade { sequence, .. }
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
    let sequence_clock = |id: &Id| project.sequence(id.as_str()).map(Clock::of_sequence);
    let clip_clock = |id: &Id| {
        project
            .locate(id.as_str())
            .map(|at| Clock::of_sequence(&project.sequences[at.sequence]))
    };
    let fallback = Clock {
        rate: Rate::FPS_24,
        origin: Time::ZERO,
        drop_frame: false,
    };
    let ids = |ids: &[Id]| ids.iter().map(Id::as_str).collect::<Vec<_>>().join(" ");
    let list = |ids: &[Id]| ids.iter().map(Id::as_str).collect::<Vec<_>>().join(",");
    let changes = |changes: &[(String, Option<kerf_model::Param>)], rate: Rate| {
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
            write!(text, " reset={}", reset.join(",")).unwrap();
        }
        text.push_str(&params_text(&set, &[], rate));
        text
    };
    match op {
        Op::AddMedia(media) => media_line(media).replacen("media ", "media add ", 1),
        Op::RelinkMedia { media, path } => format!("media relink {media} path={}", quote(path)),
        Op::RemoveMedia { media } => format!("media remove {media}"),
        Op::AddSequence(sequence) => {
            sequence_line(sequence).replacen("sequence ", "sequence add ", 1)
        }
        Op::SetSequence { sequence, change } => {
            let mut text = format!("sequence set {sequence}");
            if let Some(name) = &change.name {
                write!(text, " name={}", quote(name)).unwrap();
            }
            if let Some(rate) = change.rate {
                write!(text, " rate={rate}").unwrap();
            }
            if let Some((width, height)) = change.size {
                write!(text, " size={width}x{height}").unwrap();
            }
            if let Some(sample_rate) = change.sample_rate {
                write!(text, " audio={sample_rate}").unwrap();
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
                write!(text, " start={}", clock.print_position(start)).unwrap();
            }
            text
        }
        Op::RemoveSequence { sequence } => format!("sequence remove {sequence}"),
        Op::AddTrack {
            sequence,
            id,
            kind,
            index,
        } => {
            let mut text = format!("track add {id} {}", kind.name());
            let tracks = project
                .sequence(sequence.as_str())
                .map(|s| s.tracks.as_slice())
                .unwrap_or_default();
            if let Some(anchor) = index.and_then(|i| tracks.get(i)) {
                write!(text, " before={}", anchor.id).unwrap();
            }
            text
        }
        Op::SetTrack { track, change, .. } => {
            let mut text = format!("track set {track}");
            if let Some(name) = &change.name {
                write!(text, " name={}", quote(name.as_deref().unwrap_or_default())).unwrap();
            }
            for (flag, value) in [
                ("enabled", change.enabled),
                ("locked", change.locked),
                ("muted", change.muted),
            ] {
                if let Some(value) = value {
                    write!(text, " {flag}={value}").unwrap();
                }
            }
            text
        }
        Op::RemoveTrack { track, .. } => format!("track remove {track}"),
        Op::Insert {
            sequence,
            at,
            duration,
            targets,
        }
        | Op::Overwrite {
            sequence,
            at,
            duration,
            targets,
        } => {
            let clock = sequence_clock(sequence).unwrap_or(fallback);
            let command = if matches!(op, Op::Insert { .. }) {
                "insert"
            } else {
                "overwrite"
            };
            let Some((_, first)) = targets.first() else {
                return format!("{command} at={}", clock.print_position(*at));
            };
            let mut text = format!(
                "{command} {} at={} dur={}",
                content_text(project, &first.content),
                clock.print_position(*at),
                clock.print_duration(*duration)
            );
            if let Content::Media { speed, .. } | Content::Sequence { speed, .. } = &first.content {
                if *speed != Ratio::ONE {
                    write!(text, " speed={speed}").unwrap();
                }
            }
            let tracks: Vec<Id> = targets.iter().map(|(t, _)| t.clone()).collect();
            write!(text, " on={}", list(&tracks)).unwrap();
            let names: Vec<Id> = targets.iter().map_while(|(_, c)| c.id.clone()).collect();
            if !names.is_empty() {
                write!(text, " as={}", list(&names)).unwrap();
            }
            if let Some(name) = &first.name {
                write!(text, " name={}", quote(name)).unwrap();
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
        Op::Delete { clips } => format!("delete {}", ids(clips)),
        Op::RippleDelete { clips } => format!("ripple-delete {}", ids(clips)),
        Op::Lift {
            sequence,
            from,
            to,
            tracks,
        }
        | Op::Extract {
            sequence,
            from,
            to,
            tracks,
        } => {
            let clock = sequence_clock(sequence).unwrap_or(fallback);
            let command = if matches!(op, Op::Lift { .. }) {
                "lift"
            } else {
                "extract"
            };
            let mut text = format!(
                "{command} from={} to={}",
                clock.print_position(*from),
                clock.print_position(*to)
            );
            if let Some(tracks) = tracks {
                write!(text, " tracks={}", list(tracks)).unwrap();
            }
            text
        }
        Op::Trim {
            clip,
            edge,
            delta,
            ripple,
        } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let edge = if *edge == Edge::In { "in" } else { "out" };
            let ripple = if *ripple { " ripple" } else { "" };
            format!("trim {clip} {edge} {}{ripple}", clock.print_delta(*delta))
        }
        Op::Roll { clip, delta } => format!(
            "roll {clip} {}",
            clip_clock(clip).unwrap_or(fallback).print_delta(*delta)
        ),
        Op::Slip { clip, delta } => format!(
            "slip {clip} {}",
            clip_clock(clip).unwrap_or(fallback).print_delta(*delta)
        ),
        Op::Slide { clip, delta } => format!(
            "slide {clip} {}",
            clip_clock(clip).unwrap_or(fallback).print_delta(*delta)
        ),
        Op::Split { clip, at, tail } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("split {clip} at={}", clock.print_position(*at));
            if let Some(tail) = tail {
                write!(text, " as={tail}").unwrap();
            }
            text
        }
        Op::Blade {
            sequence,
            at,
            tracks,
        } => {
            let clock = sequence_clock(sequence).unwrap_or(fallback);
            let mut text = format!("blade at={}", clock.print_position(*at));
            if let Some(tracks) = tracks {
                write!(text, " tracks={}", list(tracks)).unwrap();
            }
            text
        }
        Op::Move { clip, track, delta } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("move {clip}");
            if let Some(track) = track {
                write!(text, " on={track}").unwrap();
            }
            if *delta != Time::ZERO || track.is_none() {
                write!(text, " by={}", clock.print_delta(*delta)).unwrap();
            }
            text
        }
        Op::Set {
            clip,
            changes: list,
        } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("set {clip}");
            let mut params = Vec::new();
            for change in list {
                match change {
                    ClipChange::Name(name) => {
                        write!(text, " name={}", quote(name.as_deref().unwrap_or_default()))
                            .unwrap()
                    }
                    ClipChange::Enabled(enabled) => write!(text, " enabled={enabled}").unwrap(),
                    ClipChange::Caption(caption) => {
                        write!(text, " caption={}", quote(caption)).unwrap()
                    }
                    ClipChange::Param(name, value) => params.push((name.clone(), value.clone())),
                }
            }
            text.push_str(&changes(&params, clock.rate));
            text
        }
        Op::Speed { clip, speed } => format!("speed {clip} {speed}"),
        Op::FxAdd {
            clip,
            effect,
            index,
        } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("fx {clip} add {}", print_fx_ref(&effect.fx, effect.version));
            if let Some(index) = index {
                write!(text, " at={}", index + 1).unwrap();
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
        Op::FxSet {
            clip,
            index,
            enabled,
            params,
        } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("fx {clip} set {}", index + 1);
            if let Some(enabled) = enabled {
                write!(text, " enabled={enabled}").unwrap();
            }
            text.push_str(&changes(params, clock.rate));
            text
        }
        Op::MaskAdd { clip, mask, index } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("mask {clip} add {}", mask.name);
            text.push_str(&mask_text(mask, catalog, clock.rate));
            if let Some(index) = index {
                write!(text, " at={}", index + 1).unwrap();
            }
            text
        }
        Op::MaskSet {
            clip,
            name,
            shape,
            mode,
            inverted,
            params,
        } => {
            let clock = clip_clock(clip).unwrap_or(fallback);
            let mut text = format!("mask {clip} set {name}");
            if let Some(shape) = shape {
                text.push_str(&mask_shape_text(shape));
            }
            if let Some(mode) = mode {
                write!(text, " mode={}", mode.name()).unwrap();
            }
            if let Some(inverted) = inverted {
                write!(text, " inverted={inverted}").unwrap();
            }
            text.push_str(&changes(params, clock.rate));
            text
        }
        Op::MaskRemove { clip, name } => format!("mask {clip} remove {name}"),
        Op::FxRemove { clip, index } => format!("fx {clip} remove {}", index + 1),
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
        } => {
            let clock = clip_clock(from).unwrap_or(fallback);
            let mut text = String::from("transition add");
            if let Some(id) = id {
                write!(text, " as={id}").unwrap();
            }
            write!(
                text,
                " from={from} to={to} fx={} dur={}",
                print_fx_ref(fx, *version),
                clock.print_duration(*duration)
            )
            .unwrap();
            if *align != kerf_model::Align::Center {
                write!(text, " align={}", align.name()).unwrap();
            }
            let defs = catalog
                .fx(fx.as_str())
                .map(|d| d.params.as_slice())
                .unwrap_or_default();
            text.push_str(&params_text(params, defs, clock.rate));
            text
        }
        Op::SetTransition {
            id,
            duration,
            align,
            params,
        } => {
            let clock = project
                .sequences
                .iter()
                .find(|s| s.tracks.iter().any(|t| t.transition(id.as_str()).is_some()))
                .map_or(fallback, Clock::of_sequence);
            let mut text = format!("transition set {id}");
            if let Some(duration) = duration {
                write!(text, " dur={}", clock.print_duration(*duration)).unwrap();
            }
            if let Some(align) = align {
                write!(text, " align={}", align.name()).unwrap();
            }
            text.push_str(&changes(params, clock.rate));
            text
        }
        Op::RemoveTransition { id } => format!("transition remove {id}"),
        Op::AddMarker {
            sequence,
            id,
            at,
            duration,
            name,
            color,
            note,
        } => {
            let clock = sequence_clock(sequence).unwrap_or(fallback);
            let mut text = String::from("marker add");
            if let Some(id) = id {
                write!(text, " as={id}").unwrap();
            }
            write!(text, " at={}", clock.print_position(*at)).unwrap();
            if *duration != Time::ZERO {
                write!(text, " dur={}", clock.print_duration(*duration)).unwrap();
            }
            if let Some(name) = name {
                write!(text, " name={}", quote(name)).unwrap();
            }
            if let Some(color) = color {
                write!(text, " color={}", color.name()).unwrap();
            }
            if let Some(note) = note {
                write!(text, " note={}", quote(note)).unwrap();
            }
            text
        }
        Op::SetMarker { id, change } => {
            let clock = project
                .sequences
                .iter()
                .find(|s| s.marker(id.as_str()).is_some())
                .map_or(fallback, Clock::of_sequence);
            let mut text = format!("marker set {id}");
            if let Some(at) = change.at {
                write!(text, " at={}", clock.print_position(at)).unwrap();
            }
            if let Some(duration) = change.duration {
                write!(text, " dur={}", clock.print_duration(duration)).unwrap();
            }
            if let Some(name) = &change.name {
                write!(text, " name={}", quote(name.as_deref().unwrap_or_default())).unwrap();
            }
            if let Some(color) = change.color {
                write!(text, " color={}", color.map_or("none", |c| c.name())).unwrap();
            }
            if let Some(note) = &change.note {
                write!(text, " note={}", quote(note.as_deref().unwrap_or_default())).unwrap();
            }
            text
        }
        Op::RemoveMarker { id } => format!("marker remove {id}"),
        Op::Link { clips, group } => {
            let mut text = format!("link {}", ids(clips));
            if let Some(group) = group {
                write!(text, " as={group}").unwrap();
            }
            text
        }
        Op::Unlink { clips } => format!("unlink {}", ids(clips)),
    }
}

#[cfg(test)]
mod tests;
