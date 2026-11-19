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
//! Lines that read analysis, such as `captions`, read it from the facts the
//! script runs with.

use crate::captions::{caption_script, is_a_sound_note, Word, DEFAULT_CAPTION_WORDS};
use crate::doc::{
    content, content_text, mask_shape_text, mask_text, media_fields, media_line, params_text,
    parse_align, parse_marker_color, parse_mask_mode, parse_retime, parse_sample_rate,
    parse_sequence_color, parse_size, pinned, sequence_fields, sequence_line, take_mask_shape,
    take_role, take_sync, word_or_quote,
};
use crate::facts::{airs, clear, Barred, Breaks, Facts, NoFacts, Place, Snap, Snaps, Speech, BEAT};
use crate::sound::Heard;
use crate::lex::{lex, Arg, Atom, Line};
use crate::making::print_step;
use crate::text::{
    parse_fx_ref, parse_param, parse_speed, print_fx_ref, print_speed, quote, Clock, Fields,
};
use kerf_model::time::{parse_timecode, FLICKS_PER_SECOND};
use kerf_model::{
    apply, Catalog, Clip, ClipChange, Content, Diagnostic, Domain, Edge, Effect, Id, MarkerChange,
    Mask, Media, NewClip, Op, ParamDef, Params, Project, Rate, Report, Sequence, SequenceChange,
    Time, Track, TrackChange, TrackKind, TrackRoleChange,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

/// The result of a script: the new project, the operations it ran (ready to
/// journal or replay), and what they did.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub project: Project,
    pub ops: Vec<Op>,
    pub report: Report,
    /// Each edge `snap=` moved, in the order the script moved them: how
    /// far it went, into what, and between which words where a transcript
    /// has them. An edge that was already in place is not said. And what
    /// each `said=` or `pause=` took out: from where to where, and the
    /// pause it leaves.
    pub snapped: Vec<String>,
}

/// Run `script` with no analysis: lines that read facts are refused.
pub fn apply_script(
    project: &Project,
    catalog: &Catalog,
    script: &str,
) -> Result<Applied, Vec<Diagnostic>> {
    apply_script_with(project, catalog, &NoFacts, script)
}

/// Run `script`, reading what analysis knows from `facts`.
pub fn apply_script_with(
    project: &Project,
    catalog: &Catalog,
    facts: &dyn Facts,
    script: &str,
) -> Result<Applied, Vec<Diagnostic>> {
    let Lines { lines, snapped } = lines_with(project, catalog, facts, script)?;
    let ops: Vec<Op> = lines.into_iter().flat_map(|(_, ops)| ops).collect();
    let (project, report) =
        apply(project, catalog, &ops).map_err(|rejected| rejected.diagnostics)?;
    Ok(Applied {
        project,
        ops,
        report,
        snapped,
    })
}

/// A script as its lines ran.
pub(crate) struct Lines {
    /// Each line that ran operations: its number, and them.
    pub lines: Vec<(usize, Vec<Op>)>,
    /// See [`Applied::snapped`].
    pub snapped: Vec<String>,
}

/// The operations each line of `script` runs as on `project`. Each line has
/// been applied to the project the lines before it leave; the whole has not
/// been applied as one.
pub(crate) fn lines_with(
    project: &Project,
    catalog: &Catalog,
    facts: &dyn Facts,
    script: &str,
) -> Result<Lines, Vec<Diagnostic>> {
    let lines = lex(script)?;
    let mut state = Script {
        project: project.clone(),
        catalog,
        facts,
        sequence: None,
        breaks: RefCell::default(),
        speech: RefCell::default(),
        snapped: RefCell::default(),
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
        ops.push((line.number, line_ops));
    }
    Ok(Lines {
        lines: ops,
        snapped: state.snapped.into_inner(),
    })
}

const COMMANDS: &str = "use, media, step, sequence, track, bus, insert, overwrite, delete, ripple-delete, lift, extract, trim, roll, slip, slide, split, blade, move, set, speed, mask, fx, transition, marker, captions, link, unlink";

struct Script<'c> {
    project: Project,
    catalog: &'c Catalog,
    facts: &'c dyn Facts,
    sequence: Option<Id>,
    /// What the facts said of each media a line has snapped to, so that a
    /// script of many snapped lines reads each once.
    breaks: RefCell<HashMap<Id, Option<Rc<Breaks>>>>,
    speech: RefCell<HashMap<Id, Rc<Speech>>>,
    /// See [`Applied::snapped`].
    snapped: RefCell<Vec<String>>,
}

type Parsed<T = Vec<Op>> = Result<T, Diagnostic>;

/// Positions on one sequence. Besides timecode, frames, and seconds, a
/// position may be `end`, the end of the sequence; `<clip>.start` or
/// `<clip>.end` of a clip on it; a marker's id, or `<marker>.end` where a
/// marker with a duration ends; or `<clip>@<source position>`, where that
/// clip plays a position of its media. Each takes an optional amount:
/// `c3.end-6f`, `beat1+2f`, `c3@14:24:05:02+1f`.
struct Positions<'a> {
    clock: Clock,
    project: &'a Project,
    sequence: &'a Sequence,
}

impl<'a> Positions<'a> {
    fn of(project: &'a Project, sequence: usize) -> Self {
        let sequence = &project.sequences[sequence];
        Self {
            clock: Clock::of_sequence(sequence),
            project,
            sequence,
        }
    }

    fn position(&self, text: &str) -> Result<Time, String> {
        let (base, rest) = if let Some((clip, source)) = text.split_once('@') {
            // Source positions carry no sign, so one starts the amount.
            let split = source
                .char_indices()
                .skip(1)
                .find(|(_, c)| matches!(c, '+' | '-'))
                .map_or(source.len(), |(at, _)| at);
            (self.played(clip, &source[..split])?, &source[split..])
        } else if let Some(found) = self.marker(text) {
            found
        } else if let Some((id, end, rest)) = [(".start", false), (".end", true)]
            .into_iter()
            .find_map(|(suffix, end)| {
                let at = text.find(suffix)?;
                Some((&text[..at], end, &text[at + suffix.len()..]))
            })
        {
            (self.edge(id, end)?, rest)
        } else if let Some(rest) = text
            .strip_prefix("end")
            .filter(|rest| rest.is_empty() || rest.starts_with(['+', '-']))
        {
            (self.sequence.duration(), rest)
        } else {
            return self.clock.position(text);
        };
        match rest {
            "" => Ok(base),
            amount if amount.starts_with(['+', '-']) => Ok(base + self.clock.delta(amount)?),
            _ => Err(format!(
                "`{text}`: after an anchor comes an amount such as +12f or -0.5s"
            )),
        }
    }

    /// A marker of this sequence named by `text`, or by `text` up to an
    /// amount, with the rest. Ids may hold `-`, so the longest id that
    /// leaves an amount wins, and `end` is always the end of the sequence.
    fn marker<'t>(&self, text: &'t str) -> Option<(Time, &'t str)> {
        let splits = text
            .char_indices()
            .filter(|(_, c)| matches!(c, '+' | '-'))
            .map(|(at, _)| at);
        std::iter::once(text.len())
            .chain(splits.collect::<Vec<_>>().into_iter().rev())
            .map(|at| text.split_at(at))
            .filter(|(id, _)| *id != "end")
            .find_map(|(id, rest)| {
                let marker = self.sequence.marker(id)?;
                (rest.is_empty() || self.clock.delta(rest).is_ok()).then_some((marker.at, rest))
            })
    }

    /// The start or end of a clip or marker of this sequence.
    fn edge(&self, id: &str, end: bool) -> Result<Time, String> {
        if let Some(clip) = self.sequence.tracks.iter().find_map(|track| track.clip(id)) {
            return Ok(if end { clip.end() } else { clip.start });
        }
        let marker = self.sequence.marker(id).ok_or_else(|| {
            format!(
                "no clip or marker `{id}` in sequence `{}`",
                self.sequence.id
            )
        })?;
        Ok(if end {
            marker.at + marker.duration
        } else {
            marker.at
        })
    }

    /// Where the clip `id` of this sequence plays `source`, a position on
    /// its media or nested sequence, on the nearest frame.
    fn played(&self, id: &str, source: &str) -> Result<Time, String> {
        let clip = self
            .sequence
            .tracks
            .iter()
            .find_map(|track| track.clip(id))
            .ok_or_else(|| format!("no clip `{id}` in sequence `{}`", self.sequence.id))?;
        let (clock, (low, high)) = source_clock(self.project, clip)?
            .zip(clip.source_range())
            .ok_or_else(|| format!("clip `{id}` plays no media; use {id}.start or {id}.end"))?;
        let at = clock.position(source)?;
        if at < low || at > high {
            return Err(format!(
                "clip `{id}` plays {} to {}, not `{source}`",
                clock.print_position(low),
                clock.print_position(high),
            ));
        }
        let played = clip
            .played_at(at)
            .expect("a clip with a source range plays it");
        Ok(self.clock.rate.round(played))
    }
}

/// Where `text` is on `sequence` of `project`, read as a script reads a
/// position: timecode, frames or seconds, or an anchor with an optional
/// amount, such as `end`, `c3.end-1f`, `beat1+2f` or `c3@14:24:05:02`.
pub fn sequence_position(
    project: &Project,
    sequence: &Sequence,
    text: &str,
) -> Result<Time, String> {
    Positions {
        clock: Clock::of_sequence(sequence),
        project,
        sequence,
    }
    .position(text)
}

/// The clock of what `clip` plays: its media's or its nested sequence's;
/// `None` for generators and captions.
fn source_clock(project: &Project, clip: &Clip) -> Result<Option<Clock>, String> {
    match &clip.content {
        Content::Media { media, .. } => project
            .media(media.as_str())
            .map(|media| Some(Clock::of_media(media)))
            .ok_or_else(|| format!("no media `{media}`")),
        Content::Sequence { sequence, .. } => project
            .sequence(sequence.as_str())
            .map(|nested| Some(Clock::of_sequence(nested)))
            .ok_or_else(|| format!("no sequence `{sequence}`")),
        Content::Generator { .. } | Content::Caption { .. } => Ok(None),
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
        // A step's words are a making's own, a name given as often as it needs: they are
        // not read as a command's fields are.
        if matches!(line.args.first(), Some(Arg { key: None, value: Atom::Word(word), .. }) if word == "step")
        {
            return self.step(line);
        }
        let spoken = self.spoken(line)?;
        let line = spoken.as_ref().unwrap_or(line);
        let mut fields = Fields::new(line)?;
        let command = fields.word(0, "a command")?;
        match command {
            "use" => {
                let id = fields.id(1, "a sequence id")?;
                if self.project.sequence(id.as_str()).is_none() {
                    return Err(
                        line.missing(fields.positional()[1].column, format!("no sequence `{id}`"))
                    );
                }
                fields.finish(2)?;
                self.sequence = Some(id);
                Ok(Vec::new())
            }
            "media" => self.media(fields),
            "sequence" => self.sequence_command(fields),
            "track" => self.track(fields),
            "bus" => self.bus(fields),
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
            "lift" | "extract" => self.take_out(fields, command == "extract"),
            "trim" => self.trim(fields),
            "roll" | "slip" | "slide" => {
                let clip = fields.id(1, "a clip id")?;
                let (found, clock) = self.clip(&clip, line)?;
                // A roll of the clip's own track alone, its linked clips' cut staying: a J or L cut.
                let alone_word = Atom::Word("alone".into());
                let alone = command == "roll"
                    && fields.positional()[2..]
                        .iter()
                        .any(|arg| arg.value == alone_word);
                let rest: Vec<&Arg> = fields.positional()[2..]
                    .iter()
                    .copied()
                    .filter(|arg| !(alone && arg.value == alone_word))
                    .collect();
                let to = fields.take_with("to", |t| clock.position(t))?;
                if let (Some(_), Some(extra)) = (to, rest.first()) {
                    return Err(fields.error(extra, "give an amount or to=, not both"));
                }
                let delta = match (to, rest.as_slice()) {
                    (Some(to), _) if command == "roll" => to - found.end(),
                    (Some(to), _) if command == "slide" => to - found.start,
                    (Some(_), _) => {
                        return Err(line.error(1, "slip takes an amount like +12f, not to="))
                    }
                    (None, [amount, ..]) => {
                        let text = fields.text(amount)?;
                        clock.delta(text).map_err(|m| fields.error(amount, m))?
                    }
                    (None, []) => clock
                        .delta(fields.word(2, "an amount like +12f or -0.5s")?)
                        .map_err(|m| line.error(fields.positional()[2].column, m))?,
                };
                let count = fields.positional().len();
                if let Some(extra) = rest.get(1) {
                    return Err(fields.error(extra, "unexpected argument"));
                }
                fields.finish(count)?;
                Ok(vec![match command {
                    "roll" => Op::Roll { clip, delta, alone },
                    "slip" => Op::Slip { clip, delta },
                    _ => Op::Slide { clip, delta },
                }])
            }
            "split" => {
                let clip = fields.id(1, "a clip id")?;
                let (found, clock) = self.clip(&clip, line)?;
                let mut at = required(&mut fields, "at", |t| clock.position(t))?;
                let tail = fields.take_id("as")?;
                if let Some(snap) = take_snap(&mut fields, &clock)? {
                    // A split must fall inside the clip.
                    let what = format!("the split of `{clip}`");
                    at = self
                        .snap_edge(
                            found,
                            at,
                            snap,
                            clock.rate,
                            |at| found.start < at && at < found.end(),
                            &what,
                        )
                        .map_err(|m| line.error(1, m))?;
                }
                fields.finish(2)?;
                Ok(vec![Op::Split { clip, at, tail }])
            }
            "blade" => {
                let s = self.current(line)?;
                let clock = Positions::of(&self.project, s);
                let mut at = required(&mut fields, "at", |t| clock.position(t))?;
                let tracks = fields.take_ids("tracks")?;
                if let Some(snap) = take_snap(&mut fields, &clock)? {
                    at = self
                        .snap_blade(s, at, tracks.as_deref(), snap)
                        .map_err(|m| line.error(1, m))?;
                }
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
                let arg = fields.positional().get(2).copied().ok_or_else(|| {
                    line.error(1, "expected a speed such as 2, 0.5, -1, or {0f:1 12f:1/2}")
                })?;
                let rate = self
                    .project
                    .locate(clip.as_str())
                    .map_or(Rate::FPS_24, |at| self.project.sequences[at.sequence].rate);
                let speed = parse_speed(&arg.value, rate).map_err(|m| fields.error(arg, m))?;
                fields.finish(3)?;
                Ok(vec![Op::Speed { clip, speed }])
            }
            "mask" => self.mask(fields),
            "fx" => self.fx(fields),
            "transition" => self.transition(fields),
            "marker" => self.marker(fields),
            "captions" => self.captions(fields),
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
            .ok_or_else(|| line.missing(1, self.project.no_clip(id.as_str())))?;
        Ok((
            self.project.clip_at(at),
            Positions::of(&self.project, at.sequence),
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
    fn step(&mut self, line: &Line) -> Parsed {
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
            _ => return Err(line.error(column(2), format!("expected add, set, or remove: {SHAPE}"))),
        };
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
    fn bus(&mut self, mut fields: Fields<'_>) -> Parsed {
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

    fn place(&mut self, mut fields: Fields<'_>, ripple: bool) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let (mut content, source) = content(&mut fields, &self.project, self.catalog, clock.rate)?;
        let snap = take_snap(&mut fields, &clock)?;
        let wrote = |key: &str| line.args.iter().any(|arg| arg.key.as_deref() == Some(key));
        let media = match (&content, snap) {
            (_, None) => None,
            (Content::Media { .. }, Some(_)) if !wrote("src") && !wrote("out") => {
                return Err(line.error(1, "snap= moves src= and out=; give one"))
            }
            (Content::Media { media, .. }, Some(_)) => self.project.media(media.as_str()),
            (_, Some(_)) => return Err(line.error(1, "snap= needs media=")),
        };
        if let (
            Some(media),
            Some((to, within)),
            Content::Media {
                source_in, speed, ..
            },
            true,
        ) = (media, snap, &mut content, wrote("src"))
        {
            *source_in = self
                .snap_source(media, *source_in, to, speed.played(within), "src=")
                .map_err(|m| line.error(1, m))?;
        }
        let positions = Positions::of(&self.project, s);
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
                let mut out_time = source
                    .position(fields.text(out)?)
                    .map_err(|m| fields.error(out, m))?;
                if let (Some(media), Some((to, within)), Content::Media { speed, .. }) =
                    (media, snap, &content)
                {
                    out_time = self
                        .snap_source(media, out_time, to, speed.played(within), "out=")
                        .map_err(|m| fields.error(out, m))?;
                }
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
                let frames = clock.rate.frames_round(speed.time_for(span)).max(1);
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
            if accepted {
                continue;
            }
            // A track that is not there takes nothing, and that is what is
            // wrong with the line: placing the clip says so, with the tracks
            // there are and the line that adds this one.
            if tracks
                .iter()
                .any(|track| sequence.track(track.as_str()).is_none())
            {
                continue;
            }
            // What `set` writes on a clip that is there, not a property a
            // track takes.
            if matches!(key, "parent" | "enabled" | "caption" | "retime" | "angle") {
                let clip = names.first().map_or("<clip>", |id| id.as_str());
                return Err(fields.error(
                    arg,
                    format!(
                        "`{key}=` is set on a clip once it is placed, not where it is placed: \
                         after this line, `set {clip} {key}={}`",
                        fields.text(arg).unwrap_or_default()
                    ),
                ));
            }
            return Err(fields.error(arg, format!("no target track takes `{key}`")));
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
        let delta = match take_snap(&mut fields, &clock)? {
            Some(snap) => {
                // The trimmed clip must keep a length.
                let keeps = |at: Time| match edge {
                    Edge::In => at < found.end(),
                    Edge::Out => at > found.start,
                };
                let what = format!(
                    "the {} of `{clip}`",
                    if edge == Edge::In { "in" } else { "out" }
                );
                self.snap_edge(found, edge_time + delta, snap, clock.rate, keeps, &what)
                    .map_err(|m| line.error(1, m))?
                    - edge_time
            }
            None => delta,
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

    /// `line` with each quoted phrase in its `src=` or `out=` replaced by
    /// when the phrase was said, from transcripts of the media: an in point
    /// on the frame its first word starts, an out point past the frame its
    /// last word ends. `None` when the line has no phrase.
    fn spoken(&self, line: &Line) -> Parsed<Option<Line>> {
        let is_phrase = |arg: &Arg| {
            matches!(arg.value, Atom::Str(_)) && matches!(arg.key.as_deref(), Some("src" | "out"))
        };
        if !line.args.iter().any(is_phrase) {
            return Ok(None);
        }
        // Positional arguments count as fields counts them, between keyed ones.
        let positional: Vec<&Arg> = line.args.iter().filter(|arg| arg.key.is_none()).collect();
        let word = |index: usize| match positional.get(index) {
            Some(Arg {
                value: Atom::Word(word),
                ..
            }) => Some(word.as_str()),
            _ => None,
        };
        let keyed = |key: &str| {
            line.args.iter().find_map(|arg| match &arg.value {
                Atom::Word(word) if arg.key.as_deref() == Some(key) => Some(word.as_str()),
                _ => None,
            })
        };
        let (media, trims_out) = match word(0) {
            Some("insert" | "overwrite") => {
                (keyed("media").and_then(|id| self.project.media(id)), false)
            }
            Some("trim") => (
                word(1)
                    .and_then(|id| self.project.locate(id))
                    .and_then(|at| self.media_of(self.project.clip_at(at))),
                word(2) == Some("out"),
            ),
            // Other commands read their fields as they always do.
            _ => return Ok(None),
        };
        // Without media, the line reads a quoted time as it always has, and
        // says so when it is not one.
        let Some(media) = media else {
            return Ok(None);
        };
        let clock = Clock::of_media(media);
        let speech = self.facts.speech(media);
        let mut spoken = line.clone();
        for arg in spoken.args.iter_mut().filter(|arg| is_phrase(arg)) {
            let Atom::Str(phrase) = &arg.value else {
                continue;
            };
            // A quoted time is still a time.
            if clock.position(phrase).is_ok() {
                continue;
            }
            let (from, to) = match speech.said(phrase)[..] {
                [once] => once,
                [] => {
                    return Err(line.error(
                        arg.column,
                        format!(
                            "no transcript of `{}` heard \"{phrase}\"; look at it with transcript first",
                            media.id
                        ),
                    ))
                }
                ref many => {
                    let times: Vec<String> = many
                        .iter()
                        .map(|(from, _)| clock.print_position(source_time(*from, clock.rate, false)))
                        .collect();
                    return Err(line.error(
                        arg.column,
                        format!(
                            "\"{phrase}\" is said {} times in `{}`, at {}; give more words or the time",
                            many.len(),
                            media.id,
                            times.join(", ")
                        ),
                    ));
                }
            };
            let out = arg.key.as_deref() == Some("out") || trims_out;
            let time = source_time(if out { to } else { from }, clock.rate, out);
            arg.value = Atom::Word(clock.print_position(time));
        }
        Ok(Some(spoken))
    }

    /// `lift` and `extract`: a range of the sequence between two positions,
    /// or, with a clip named, what the clip says (`said=`), the pause after
    /// what it says (`pause=`), or two positions moved by its sound
    /// (`snap=`).
    fn take_out(&mut self, mut fields: Fields<'_>, close: bool) -> Parsed {
        let line = fields.line();
        let named = fields.positional().len() > 1;
        let (s, from, to) = if named {
            let clip = fields.id(1, "a clip id")?;
            let (found, positions) = self.clip(&clip, line)?;
            let clock = &positions.clock;
            let (said, pause) = (fields.take("said"), fields.take("pause"));
            let from = fields.take_with("from", |t| positions.position(t))?;
            let to = fields.take_with("to", |t| positions.position(t))?;
            let leave = fields.take_with("leave", |t| clock.duration(t))?;
            let range = match (said, pause, from, to) {
                (Some(_), Some(_), ..) => {
                    return Err(line.error(1, "give said= or pause=, not both"))
                }
                (Some(arg), None, None, None) | (None, Some(arg), None, None) => {
                    if pause.is_some() && !close {
                        return Err(fields.error(arg, "pause= closes a pause up: use extract"));
                    }
                    let within = fields
                        .take_with("within", |text| clock.duration(text))?
                        .unwrap_or_else(|| Time::from_flicks(FLICKS_PER_SECOND / 2));
                    let cut = Said {
                        phrase: fields.text(arg)?,
                        pause: pause.is_some(),
                        leave: leave.map_or(BEAT, Time::seconds),
                        within,
                    };
                    self.said(found, clock, &cut)
                        .map_err(|m| fields.error(arg, m))?
                }
                (None, None, Some(from), Some(to)) => {
                    if leave.is_some() {
                        return Err(line.error(1, "leave= goes with said= or pause="));
                    }
                    let snap = take_snap(&mut fields, clock)?.ok_or_else(|| {
                        line.error(
                            1,
                            format!(
                                "naming `{clip}` puts the edges by its sound: add snap=pause or \
                                 snap=cut, or leave the clip out"
                            ),
                        )
                    })?;
                    for (key, at) in [("from", from), ("to", to)] {
                        if at < found.start || at > found.end() {
                            return Err(line.error(
                                1,
                                format!(
                                    "{key}= is not in `{clip}`, which plays from {} to {}",
                                    clock.print_position(found.start),
                                    clock.print_position(found.end())
                                ),
                            ));
                        }
                    }
                    let rate = clock.rate;
                    let starts = |at: Time| found.start <= at && at < found.end();
                    let ends = |at: Time| found.start < at && at <= found.end();
                    (
                        self.snap_edge(found, from, snap, rate, starts, "from=")
                            .map_err(|m| line.error(1, m))?,
                        self.snap_edge(found, to, snap, rate, ends, "to=")
                            .map_err(|m| line.error(1, m))?,
                    )
                }
                _ => {
                    return Err(line.error(
                        1,
                        "with a clip, give said=\"…\", pause=\"…\", or from= and to= with snap=",
                    ))
                }
            };
            let at = self.project.locate(clip.as_str());
            (at.expect("the clip was found").sequence, range.0, range.1)
        } else {
            for key in ["snap", "said", "pause", "leave"] {
                if let Some(arg) = fields.take(key) {
                    return Err(fields.error(
                        arg,
                        format!(
                            "{key}= goes by a clip's sound and words: name the clip, as in \
                             `extract c1 said=\"…\"` or `extract c1 from=… to=… snap=pause`"
                        ),
                    ));
                }
            }
            let s = self.current(line)?;
            let clock = Positions::of(&self.project, s);
            let from = required(&mut fields, "from", |t| clock.position(t))?;
            let to = required(&mut fields, "to", |t| clock.position(t))?;
            (s, from, to)
        };
        let tracks = fields.take_ids("tracks")?;
        fields.finish(if named { 2 } else { 1 })?;
        let sequence = self.project.sequences[s].id.clone();
        Ok(vec![if close {
            Op::Extract {
                sequence,
                from,
                to,
                tracks,
            }
        } else {
            Op::Lift {
                sequence,
                from,
                to,
                tracks,
            }
        }])
    }

    /// The range of its sequence that takes what `clip` says out of it, as
    /// `cut` asks: the words of a phrase, or the pause after them, with
    /// each edge in the quiet beside what stays, so that the pause the cut
    /// leaves is `cut.leave` long where the quiet there allows it. What is
    /// taken is kept for [`Applied::snapped`].
    fn said(&self, clip: &Clip, clock: &Clock, cut: &Said<'_>) -> Result<(Time, Time), String> {
        let media = self
            .media_of(clip)
            .ok_or_else(|| format!("clip `{}` plays no media to hear", clip.id))?;
        if clip.speed().is_reverse() {
            return Err(format!(
                "clip `{}` plays in reverse; use from= and to=",
                clip.id
            ));
        }
        let (source_in, source_out) = clip.source_range().expect("a media clip plays a source");
        let (source_in, source_out) = (source_in.seconds(), source_out.seconds());
        let speech = self.heard(media);
        let media_clock = Clock::of_media(media);
        let phrase = cut.phrase;
        // Where the clip says it, give or take what a transcript is out by.
        let everywhere = speech.said(phrase);
        let here: Vec<(f64, f64)> = everywhere
            .iter()
            .copied()
            .filter(|(start, end)| *start >= source_in - 0.05 && *end <= source_out + 0.05)
            .collect();
        let (start, end) = match (&here[..], &everywhere[..]) {
            ([once], _) => *once,
            ([], []) => {
                return Err(format!(
                    "no transcript of `{}` heard \"{phrase}\"; look at it with transcript first",
                    media.id
                ))
            }
            ([], elsewhere) => {
                let times: Vec<String> = elsewhere
                    .iter()
                    .map(|(at, _)| {
                        media_clock.print_position(source_time(*at, media_clock.rate, false))
                    })
                    .collect();
                return Err(format!(
                    "`{}` does not play \"{phrase}\", which `{}` has at {}",
                    clip.id,
                    media.id,
                    times.join(", ")
                ));
            }
            (many, _) => {
                let times: Vec<String> = many
                    .iter()
                    .filter_map(|(at, _)| clip.played_at(media_time(*at)))
                    .map(|at| clock.print_position(clock.rate.round(at)))
                    .collect();
                return Err(format!(
                    "\"{phrase}\" is said {} times in `{}`, at {}; give more words, or from= and to=",
                    many.len(),
                    clip.id,
                    times.join(", ")
                ));
            }
        };
        let mut words = speech.heard();
        words.retain(|word| !is_a_sound_note(&word.text));
        // The quiet before what goes, and the quiet after it.
        let (before, after) = if cut.pause {
            let before = self
                .quiet_beside(clip, media, end, cut.within)
                .map_err(|why| format!("no pause after \"{phrase}\": {why}"))?;
            // The pause runs to the next word the clip plays, or its end.
            let next = words
                .iter()
                .find(|word| (word.start + word.end) / 2.0 > end && word.start < source_out);
            let after = match next {
                Some(word) => Some(
                    self.quiet_beside(clip, media, word.start, cut.within)
                        .map_err(|why| {
                            format!("no pause before {}: {why}", quote(word.text.trim()))
                        })?,
                ),
                None => None,
            };
            // What sounds in the wait and has no word: a breath goes with
            // the pause, and what may be a word nobody transcribed does not.
            let waits_to = after.map_or(source_out, |after| after.quiet.0);
            let sounds = self
                .facts
                .sound(media)
                .filter(|_| waits_to > before.quiet.1)
                .and_then(|sound| sound.heard(before.quiet.1, waits_to));
            if sounds == Some(Heard::Sound) {
                let position = |seconds: f64| {
                    media_clock.print_position(media.rate.round(media_time(seconds)))
                };
                return Err(format!(
                    "the wait after \"{phrase}\" holds sound at the level of speech, from {} to {} \
                     of `{}`, which no word accounts for: listen to it, and take it out by from= \
                     and to= if it is to go",
                    position(before.quiet.1),
                    position(waits_to),
                    media.id
                ));
            }
            (Some(before), after)
        } else {
            (
                Some(
                    self.quiet_beside(clip, media, start, cut.within)
                        .map_err(|why| format!("nowhere to cut before \"{phrase}\": {why}"))?,
                ),
                Some(
                    self.quiet_beside(clip, media, end, cut.within)
                        .map_err(|why| format!("nowhere to cut after \"{phrase}\": {why}"))?,
                ),
            )
        };
        let before = before.expect("a cut starts in a quiet");
        // Words sound between the quiet before them and the quiet after.
        if !cut.pause && after.is_some_and(|after| after.quiet.0 < before.quiet.1) {
            return Err(format!(
                "`{}` has no sound for \"{phrase}\" between two quiets, so its transcript's times \
                 are out there: use from= and to=",
                media.id
            ));
        }
        let (air_after, air_before) = airs(cut.leave);
        let played = |seconds: f64| {
            clip.played_at(media_time(seconds))
                .expect("a media clip plays a source")
        };
        // A frame of the sequence in a quiet, nearest a moment of the media.
        let frame = |ideal: f64, place: &Place| {
            frame_within(
                clock.rate,
                played(ideal),
                (played(place.room.0), played(place.room.1)),
            )
            .ok_or_else(|| {
                format!(
                    "no frame falls in the {:.2}s quiet of `{}` at {}",
                    place.quiet.1 - place.quiet.0,
                    media.id,
                    media_clock.print_position(media.rate.round(media_time(place.quiet.0)))
                )
            })
        };
        // An edge the clip does not play is the clip's own: what goes runs
        // from its start, or to its end.
        let from = frame(before.quiet.0 + air_after, &before)?.max(clip.start);
        let to = match &after {
            Some(after) => frame(after.quiet.1 - air_before, after)?.min(clip.end()),
            None => clip.end(),
        };
        if to <= from {
            return Err(if cut.pause {
                format!(
                    "the pause after \"{phrase}\" is {:.2}s, which leave={:.2}s does not shorten",
                    after.map_or(source_out, |after| after.quiet.1) - before.quiet.0,
                    cut.leave
                )
            } else {
                format!("nothing of \"{phrase}\" is left to take out of `{}`", clip.id)
            });
        }
        // What is said of it: where, what it leaves, and between which words.
        let source = |at: Time| clip.source_at(at).map_or(0.0, |at| at.seconds());
        let heard_end = after.map_or(source_out, |after| after.quiet.1);
        // No more of a quiet than the clip plays.
        let left = (source(from) - before.quiet.0.max(source_in)).max(0.0)
            + (heard_end.min(source_out) - source(to)).max(0.0);
        let (last, next) = beside(&words, before.quiet.0, heard_end);
        let between = match (last, next) {
            (Some(last), Some(next)) if next.start < source_out && last.end > source_in => format!(
                " between {} and {}",
                quote(last.text.trim()),
                quote(next.text.trim())
            ),
            _ => String::new(),
        };
        let range = format!(
            "{} to {}",
            clock.print_position(from),
            clock.print_position(to)
        );
        self.snapped.borrow_mut().push(if cut.pause {
            format!(
                "the pause after \"{phrase}\" in `{}`, {:.2}s, comes down to {left:.2}s{between}: \
                 {range} goes",
                clip.id,
                heard_end - before.quiet.0,
            )
        } else {
            format!(
                "\"{phrase}\" goes from `{}`, {range}, which leaves {left:.2}s{between}",
                clip.id
            )
        });
        Ok((from, to))
    }

    /// The quiet an edge asked for at `seconds` of `media` would go into,
    /// within `within` of where `clip` plays it: the one it is in, or the
    /// one at the nearer end of the sound it is in, past no word.
    fn quiet_beside(
        &self,
        clip: &Clip,
        media: &Media,
        seconds: f64,
        within: Time,
    ) -> Result<Place, String> {
        let at = media_time(seconds);
        let offset = clip.played_at(at).expect("a media clip plays a source") - clip.start;
        let speed = clip.speed();
        let reach = speed.played(offset + within) - speed.played(offset);
        let (found, _) = self.snaps(media, at, Snap::Pause, reach)?;
        found.places.first().copied().ok_or_else(|| {
            nothing_near(
                media,
                media.rate.round(at),
                Snap::Pause,
                reach,
                &found.barred,
            )
        })
    }

    /// What transcripts of `media` heard, read once for a script.
    fn heard(&self, media: &Media) -> Rc<Speech> {
        self.speech
            .borrow_mut()
            .entry(media.id.clone())
            .or_insert_with(|| Rc::new(self.facts.speech(media)))
            .clone()
    }

    fn media_of(&self, clip: &Clip) -> Option<&Media> {
        match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()),
            _ => None,
        }
    }

    /// `edge`, a time on the sequence where `clip` plays, moved to the
    /// nearest place `snap` allows in the clip's media that `keeps`, on a
    /// frame of the sequence: the frame nearest a cut, or the frame nearest
    /// the place in a pause that the clip still plays inside that pause.
    /// `what` names the edge for what is said of the move.
    fn snap_edge(
        &self,
        clip: &Clip,
        edge: Time,
        (snap, within): (Snap, Time),
        rate: Rate,
        keeps: impl Fn(Time) -> bool,
        what: &str,
    ) -> Result<Time, String> {
        let media = self
            .media_of(clip)
            .ok_or_else(|| format!("clip `{}` plays no media to snap to", clip.id))?;
        let source = clip.source_at(edge).expect("a media clip plays a source");
        let speed = clip.speed();
        let offset = edge - clip.start;
        let reach = speed.played(offset + within) - speed.played(offset);
        let played = |seconds: f64| {
            clip.played_at(media_time(seconds))
                .expect("a media clip plays a source")
        };
        let (found, speech) = self.snaps(media, source, snap, reach)?;
        let (at, place) = found
            .places
            .iter()
            .find_map(|place| {
                let at = match snap {
                    // An index's times are rounded to microseconds.
                    Snap::Cut => rate.round(
                        clip.played_at(media.rate.round(media_time(place.at)))
                            .expect("a media clip plays a source"),
                    ),
                    Snap::Pause => {
                        // A clip in reverse plays the end of a pause first.
                        let (from, to) = (played(place.room.0), played(place.room.1));
                        frame_within(rate, played(place.at), (from.min(to), from.max(to)))?
                    }
                };
                keeps(at).then_some((at, place))
            })
            .ok_or_else(|| nothing_near(media, source, snap, reach, &found.barred))?;
        let clock = Clock::of_sequence(
            &self.project.sequences[self
                .project
                .locate(clip.id.as_str())
                .expect("a clip is on a sequence")
                .sequence],
        );
        self.say_snapped(what, &clock, (edge, at), media, (snap, place), &speech);
        Ok(at)
    }

    /// `at`, a time on `media`, moved to the nearest place `snap` allows
    /// within `reach`, on a frame of the media: the frame a cut starts, or
    /// the frame nearest the place in a pause that is still inside it.
    /// `what` names the edge for what is said of the move.
    fn snap_source(
        &self,
        media: &Media,
        at: Time,
        snap: Snap,
        reach: Time,
        what: &str,
    ) -> Result<Time, String> {
        let (found, speech) = self.snaps(media, at, snap, reach)?;
        let (to, place) = found
            .places
            .iter()
            .find_map(|place| {
                let to = match snap {
                    Snap::Cut => media.rate.round(media_time(place.at)),
                    Snap::Pause => frame_within(
                        media.rate,
                        media_time(place.at),
                        (media_time(place.room.0), media_time(place.room.1)),
                    )?,
                };
                Some((to, place))
            })
            .ok_or_else(|| nothing_near(media, at, snap, reach, &found.barred))?;
        let clock = Clock::of_media(media);
        self.say_snapped(what, &clock, (at, to), media, (snap, place), &speech);
        Ok(to)
    }

    /// The places `snap` allows within `reach` of `at` on `media`, nearest
    /// first, in seconds of the media, with the quiets beside `at` that
    /// would not do, and what the media's transcripts heard where a pause
    /// is asked for: an edge is taken past no word of them.
    fn snaps(
        &self,
        media: &Media,
        at: Time,
        snap: Snap,
        reach: Time,
    ) -> Result<(Snaps, Rc<Speech>), String> {
        let breaks = self
            .breaks
            .borrow_mut()
            .entry(media.id.clone())
            .or_insert_with(|| self.facts.breaks(media).map(Rc::new))
            .clone()
            .ok_or_else(|| {
                format!(
                    "`{}` is not indexed yet; look at it with media first",
                    media.id
                )
            })?;
        let speech = match snap {
            Snap::Pause => self.heard(media),
            // A cut of the picture is where it is, whatever is said.
            Snap::Cut => Rc::default(),
        };
        let found = breaks.snaps(snap, at.seconds(), reach.seconds(), &speech);
        Ok((found, speech))
    }

    /// Keep for [`Applied::snapped`] where `snap` put the edge `what`,
    /// which was asked for at `from` and is now at `to` on `clock`, in
    /// `place` of `media`, when it moved: a move is silent in the document,
    /// and one that left a word in or out is not what was asked for.
    fn say_snapped(
        &self,
        what: &str,
        clock: &Clock,
        moved: (Time, Time),
        media: &Media,
        (snap, place): (Snap, &Place),
        speech: &Speech,
    ) {
        let onto = match snap {
            Snap::Cut => "onto",
            Snap::Pause => "into",
        };
        let place = place_said(media, (snap, place), speech);
        self.say_went(what, clock, moved, &format!("{onto} {place}"));
    }

    /// Keep for [`Applied::snapped`] that `what`, asked for at `from`, went
    /// to `to` on `clock`, `into` what, when it moved.
    fn say_went(&self, what: &str, clock: &Clock, (from, to): (Time, Time), into: &str) {
        let frames = clock.rate.frames_round(to - from);
        if frames == 0 {
            return;
        }
        self.snapped.borrow_mut().push(format!(
            "{what} went {} frame{} {}, to {}: {into}",
            frames.abs(),
            if frames.abs() == 1 { "" } else { "s" },
            if frames < 0 { "earlier" } else { "later" },
            clock.print_position(to),
        ));
    }

    /// `at`, where a blade was asked for on sequence `s`, moved to the
    /// nearest place `snap` allows where it still cuts what it cut there and
    /// nothing else, on one frame for every track: a blade is one cut.
    ///
    /// A pause is a place every sound the blade cuts is quiet in at once.
    /// Those are the clips it cuts on sound tracks, or where it cuts none
    /// there, the media of the picture clips it cuts, as a split of one goes
    /// by. Each takes the blade past no word of its own, as it would a split
    /// of it, so sounds with no quiet in common refuse the blade, and what
    /// each would take, and what of another is in the way there, is said. A
    /// cut is the nearest of any picture the blade cuts.
    fn snap_blade(
        &self,
        s: usize,
        at: Time,
        tracks: Option<&[Id]>,
        (snap, within): (Snap, Time),
    ) -> Result<Time, String> {
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let rate = sequence.rate;
        // The tracks it cuts, as the kernel takes them: one that is missing
        // or locked is the kernel's to refuse.
        let cut_tracks: Vec<&Track> = match tracks {
            None => sequence.tracks.iter().filter(|track| !track.locked).collect(),
            Some(ids) => {
                let named: Vec<&Track> = ids
                    .iter()
                    .filter_map(|id| sequence.track(id.as_str()))
                    .filter(|track| !track.locked)
                    .collect();
                if named.len() < ids.len() {
                    return Ok(at);
                }
                named
            }
        };
        // It stays inside each clip it was asked to cut and goes into no
        // other, so that it cuts the same clips wherever it lands.
        let crosses = |clip: &Clip| clip.start < at && at < clip.end();
        let frame = rate.frame_duration();
        let mut cut: Vec<(&Track, &Clip)> = Vec::new();
        let mut low: Option<(Time, &Clip)> = None;
        let mut high: Option<(Time, &Clip)> = None;
        for &track in &cut_tracks {
            for clip in &track.clips {
                let (from, to) = if crosses(clip) {
                    cut.push((track, clip));
                    (Some(clip.start + frame), Some(clip.end() - frame))
                } else if clip.end() <= at {
                    (Some(clip.end()), None)
                } else {
                    (None, Some(clip.start))
                };
                if let Some(from) = from.filter(|from| low.is_none_or(|(low, _)| *from > low)) {
                    low = Some((from, clip));
                }
                if let Some(to) = to.filter(|to| high.is_none_or(|(high, _)| *to < high)) {
                    high = Some((to, clip));
                }
            }
        }
        let with = |has: fn(&Media) -> bool| -> Vec<(&Clip, &Media)> {
            let media = |clip| self.media_of(clip).filter(|media| has(media));
            cut.iter()
                .filter_map(|(_, clip)| Some((*clip, media(clip)?)))
                .collect()
        };
        let goes_by: Vec<(&Clip, &Media)> = match snap {
            Snap::Pause => {
                let mut sounds = Vec::new();
                for (track, clip) in &cut {
                    if track.kind != TrackKind::Audio {
                        continue;
                    }
                    // What sounds and has no index cannot be said to be quiet.
                    let media = self.media_of(clip).ok_or_else(|| {
                        format!(
                            "clip `{}` sounds and plays no media to snap to: leave its track \
                             out with tracks=",
                            clip.id
                        )
                    })?;
                    sounds.push((*clip, media));
                }
                if sounds.is_empty() {
                    sounds = with(|media| media.audio.is_some());
                }
                sounds
            }
            Snap::Cut => with(|media| media.video.is_some()),
        };
        let mut bladed: Vec<Bladed<'_>> = Vec::new();
        for (clip, media) in goes_by {
            let source = clip.source_at(at).expect("a media clip plays a source");
            let speed = clip.speed();
            // Clips that play one media together, as a picture and its
            // sound do, are one to go by.
            if bladed.iter().any(|other| {
                other.media.id == media.id && other.source == source && other.clip.speed() == speed
            }) {
                continue;
            }
            let offset = at - clip.start;
            let reach = speed.played(offset + within) - speed.played(offset);
            let (found, speech) = self.snaps(media, source, snap, reach)?;
            bladed.push(Bladed {
                clip,
                media,
                source,
                reach,
                found,
                speech,
            });
        }
        let Some((low, high)) = low.zip(high).filter(|_| !bladed.is_empty()) else {
            return Err(format!(
                "nothing the blade cuts at {} has {} to snap to",
                clock.print_position(at),
                match snap {
                    Snap::Pause => "a sound",
                    Snap::Cut => "a picture",
                }
            ));
        };
        let span = Span { at, low, high };
        let (to, into) = match snap {
            Snap::Pause => quiet_together(&bladed, &span, &clock, within)?,
            Snap::Cut => {
                let cuts = bladed.iter().flat_map(|one| {
                    one.found.places.iter().map(move |place| {
                        // An index's times are rounded to microseconds.
                        let cut = one.media.rate.round(media_time(place.at));
                        let to = one.clip.played_at(cut);
                        let to = rate.round(to.expect("a media clip plays a source"));
                        (to, one, place)
                    })
                });
                let (to, one, place) = cuts
                    .filter(|(to, ..)| span.holds(*to))
                    .min_by_key(|(to, ..)| (*to - at).abs())
                    .ok_or_else(|| {
                        let none: Vec<String> = bladed.iter().map(|one| one.nowhere(snap)).collect();
                        none.join("; ")
                    })?;
                let place = place_said(one.media, (snap, place), &one.speech);
                (to, format!("onto {place}"))
            }
        };
        self.say_went("the blade", &clock, (at, to), &into);
        Ok(to)
    }

    /// Where on the sequence `clip` plays the source position `text`, on
    /// the nearest frame: where a trim to that source position puts the
    /// edge.
    fn source_edge(&self, clip: &Clip, rate: Rate, text: &str) -> Result<Time, String> {
        let source = source_clock(&self.project, clip)?
            .ok_or_else(|| format!("clip `{}` has no source; use to=", clip.id))?;
        if clip.speed().is_reverse() {
            return Err(format!("clip `{}` plays in reverse; use to=", clip.id));
        }
        let played = clip
            .played_at(source.position(text)?)
            .expect("a clip with a source clock has a source range");
        Ok(rate.round(played))
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

    fn mask(&mut self, mut fields: Fields<'_>) -> Parsed {
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

    fn fx(&mut self, mut fields: Fields<'_>) -> Parsed {
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

    fn marker(&mut self, mut fields: Fields<'_>) -> Parsed {
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

    /// `captions <clip> on=<track> [words=<n>]`: the caption lines for what
    /// the clip says, from the words transcripts of its media heard.
    fn captions(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let track = fields
            .take_id("on")?
            .ok_or_else(|| line.error(1, "captions needs on=<subtitle track>"))?;
        let most = fields
            .take_with("words", |text| {
                text.parse()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| format!("`{text}` is not a word count"))
            })?
            .unwrap_or(DEFAULT_CAPTION_WORDS);
        fields.finish(2)?;
        let found = self
            .project
            .locate(clip.as_str())
            .map(|at| self.project.clip_at(at));
        let Some((found, media)) = found.and_then(|found| match &found.content {
            Content::Media { media, .. } => Some((found, self.project.media(media.as_str())?)),
            _ => None,
        }) else {
            return Err(line.missing(1, format!("no media clip `{clip}`")));
        };
        let speech = self.facts.speech(media);
        let (source_in, source_out) = found.source_range().unwrap_or_default();
        if !speech.covers(source_in.seconds(), source_out.seconds()) {
            let clock = Clock::of_media(media);
            return Err(line.error(
                1,
                format!(
                    "no transcript of `{}` covers `{clip}` ({}–{}); look at it with transcript, from, and to first",
                    media.id,
                    clock.print_position(source_in),
                    clock.print_position(source_out),
                ),
            ));
        }
        let script = caption_script(
            &self.project,
            clip.as_str(),
            track.as_str(),
            &speech.words,
            most,
        )
        .map_err(|message| line.error(1, message))?;
        // The caption lines run in the clip's sequence; the script's own
        // choice of sequence holds after them.
        let lines = lex(&script).map_err(|mut errors| line.error(1, errors.remove(0).message))?;
        let chosen = self.sequence.clone();
        let mut ops = Vec::new();
        let mut failed = None;
        for generated in &lines {
            match self.parse(generated) {
                Ok(more) => ops.extend(more),
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
        }
        self.sequence = chosen;
        match failed {
            Some(error) => Err(line.error(1, error.message)),
            None => Ok(ops),
        }
    }
}

/// Why nothing near `at` on `media` would do for `snap`.
/// What `extract <clip> said=` or `pause=` asks for.
struct Said<'a> {
    /// The words, as a transcript heard them.
    phrase: &'a str,
    /// Whether it is the pause after them that goes, and not the words.
    pause: bool,
    /// How long a pause the cut leaves, in seconds.
    leave: f64,
    /// How far from a word's time the quiet beside it is looked for.
    within: Time,
}

/// Of `words` in the order they were said, the last whose middle is at or
/// before `from` seconds and the first whose middle is after `to`.
fn beside<'w>(words: &[&'w Word], from: f64, to: f64) -> (Option<&'w Word>, Option<&'w Word>) {
    let middle = |word: &Word| (word.start + word.end) / 2.0;
    (
        words.iter().rev().find(|word| middle(word) <= from).copied(),
        words.iter().find(|word| middle(word) > to).copied(),
    )
}

/// A clip a blade cuts and goes by: where the blade was asked for in its
/// media, and the places beside that.
struct Bladed<'a> {
    clip: &'a Clip,
    media: &'a Media,
    source: Time,
    reach: Time,
    found: Snaps,
    speech: Rc<Speech>,
}

impl Bladed<'_> {
    /// Where the clip plays `seconds` of its media.
    fn played(&self, seconds: f64) -> Time {
        self.clip
            .played_at(media_time(seconds))
            .expect("a media clip plays a source")
    }

    /// Where the clip plays a stretch of its media, the earlier end first:
    /// a clip in reverse plays the end of a pause first.
    fn plays(&self, (from, to): (f64, f64)) -> (Time, Time) {
        let (from, to) = (self.played(from), self.played(to));
        (from.min(to), from.max(to))
    }

    fn nowhere(&self, snap: Snap) -> String {
        nothing_near(self.media, self.source, snap, self.reach, &self.found.barred)
    }

    /// What of this clip is in the way of a blade at `to` on its sequence,
    /// where that is no place for it: the word a quiet there is on the
    /// other side of, or the word its transcript has it saying there.
    fn in_the_way(&self, to: Time) -> Option<String> {
        let takes = self.found.places.iter().any(|place| {
            let room = self.plays(place.room);
            room.0 <= to && to <= room.1
        });
        if takes {
            return None;
        }
        let playing = self.clip.source_at(to)?.seconds();
        let across = self.found.barred.iter().find_map(|barred| match barred {
            Barred::Across { quiet, word, .. } if quiet.0 <= playing && playing <= quiet.1 => {
                Some(word)
            }
            _ => None,
        });
        let heard = self.speech.heard();
        let says = heard.iter().find(|word| {
            !is_a_sound_note(&word.text) && word.start <= playing && playing <= word.end
        });
        let id = &self.clip.id;
        Some(match (across, says) {
            (Some(word), _) => format!(
                ", which is on the other side of {} in `{id}`",
                quote(word.text.trim())
            ),
            (None, Some(word)) => format!(", while `{id}` says {}", quote(word.text.trim())),
            (None, None) => format!(", which is no place for `{id}`"),
        })
    }
}

/// How far a blade asked for at `at` may go and still cut the clips it cuts
/// there and no other: from `low` to `high`, each with the clip that says
/// so, one the blade would come out of or one it would go into.
struct Span<'a> {
    at: Time,
    low: (Time, &'a Clip),
    high: (Time, &'a Clip),
}

impl Span<'_> {
    fn holds(&self, to: Time) -> bool {
        self.low.0 <= to && to <= self.high.0
    }

    /// Why a blade at `to`, which the span does not hold, is refused: what
    /// it would cut there that it was not asked to, or would not that it was.
    fn refuses(&self, to: Time) -> String {
        let (_, clip) = if to < self.low.0 { self.low } else { self.high };
        let would = if clip.start < self.at && self.at < clip.end() {
            format!("not cut `{}`", clip.id)
        } else {
            format!("cut `{}` as well", clip.id)
        };
        format!("where it would {would}: blade there with at=, or leave a track out with tracks=")
    }
}

/// Where a blade goes that is to be in a quiet of every clip of `bladed` at
/// once, within `span`: the frame of `clock`, and what is said of where it
/// went. Each clip takes it as it would a split of itself; where the clips
/// are quiet together it keeps clear of the sound either side in each, and
/// a quiet too short together for that is divided as a short pause of one.
fn quiet_together(
    bladed: &[Bladed<'_>],
    span: &Span<'_>,
    clock: &Clock,
    within: Time,
) -> Result<(Time, String), String> {
    let (rate, at) = (clock.rate, span.at);
    let said =
        |one: &Bladed<'_>, place: &Place| place_said(one.media, (Snap::Pause, place), &one.speech);
    // What each would take by itself, as a split of it would.
    let mut alone: Vec<(Time, &Place)> = Vec::new();
    let mut nowhere: Vec<String> = Vec::new();
    for one in bladed {
        let mut frames = one.found.places.iter().filter_map(|place| {
            let to = frame_within(rate, one.played(place.at), one.plays(place.room))?;
            Some((to, place))
        });
        let first = frames.clone().next();
        match (frames.find(|(to, _)| span.holds(*to)), first) {
            (Some(took), _) => alone.push(took),
            (None, Some((to, place))) => nowhere.push(format!(
                "`{}` would take the blade to {}, into {}, {}",
                one.clip.id,
                clock.print_position(to),
                said(one, place),
                span.refuses(to)
            )),
            (None, None) => nowhere.push(format!(
                "in `{}`, {}",
                one.clip.id,
                one.nowhere(Snap::Pause)
            )),
        }
    }
    if !nowhere.is_empty() {
        return Err(nowhere.join("; "));
    }
    // The stretches a frame would do in for all of them: in a quiet of
    // each, the `nth` of its places, where those are quiet together.
    let ever = (Time::from_flicks(i64::MIN), Time::from_flicks(i64::MAX));
    let mut together: Vec<((Time, Time), Vec<usize>)> = vec![(ever, Vec::new())];
    for one in bladed {
        together = together
            .iter()
            .flat_map(|(all, taken)| {
                let places = one.found.places.iter().enumerate();
                places.filter_map(move |(nth, place)| {
                    let room = one.plays(place.room);
                    let both = (all.0.max(room.0), all.1.min(room.1));
                    let taken = taken.iter().copied().chain([nth]).collect();
                    (both.0 <= both.1).then_some((both, taken))
                })
            })
            .collect();
    }
    let mut frames: Vec<_> = together
        .iter()
        .filter_map(|(room, taken)| {
            let places: Vec<(&Bladed<'_>, &Place)> = bladed
                .iter()
                .zip(taken)
                .map(|(one, nth)| (one, &one.found.places[*nth]))
                .collect();
            let all = |stretch: fn(&Place) -> (f64, f64)| {
                let each = places.iter().map(|(one, place)| one.plays(stretch(place)));
                let from = each.clone().map(|(from, _)| from).max()?;
                Some((from, each.map(|(_, to)| to).min()?))
            };
            let quiet = all(|place| place.quiet)?;
            let (from, to) = all(|place| clear(place.quiet))?;
            let (from, to) = if from <= to {
                (from, to)
            } else {
                let (from, to) = clear((quiet.0.seconds(), quiet.1.seconds()));
                let time = |seconds: f64| Time::from_seconds(seconds).unwrap_or(at);
                (time(from), time(to))
            };
            let to = frame_within(rate, at.clamp(from, to), *room)?;
            Some((to, places, quiet, taken.iter().max().copied()))
        })
        .collect();
    // The nearer end of the sound each is in first, as for one of them;
    // else the nearer frame.
    frames.sort_by_key(|(to, _, _, furthest)| (*furthest, (*to - at).abs()));
    if let Some((to, places, quiet, _)) = frames.iter().find(|(to, ..)| span.holds(*to)) {
        let places: Vec<String> = places.iter().map(|(one, place)| said(one, place)).collect();
        let mut into = format!("into {}", places.join(", and "));
        if places.len() > 1 {
            let together = (quiet.1 - quiet.0).seconds();
            write!(into, ", quiet together for {together:.2}s").unwrap();
        }
        return Ok((*to, into));
    }
    if let Some((to, ..)) = frames.first() {
        return Err(format!(
            "the clips this blade cuts are quiet together at {}, {}",
            clock.print_position(*to),
            span.refuses(*to)
        ));
    }
    let each: Vec<String> = bladed
        .iter()
        .zip(&alone)
        .map(|(one, (to, place))| {
            let place = said(one, place);
            let way = bladed.iter().find_map(|other| other.in_the_way(*to));
            let way = way.unwrap_or_default();
            if *to == at {
                format!("`{}` is quiet there, in {place}{way}", one.clip.id)
            } else {
                format!(
                    "`{}` would take it to {}, into {place}{way}",
                    one.clip.id,
                    clock.print_position(*to)
                )
            }
        })
        .collect();
    Err(format!(
        "the clips this blade cuts have no quiet in common within {:.2}s of {}: {}: cut them one \
         at a time with tracks=",
        within.seconds(),
        clock.print_position(at),
        each.join("; ")
    ))
}

/// A place of `media` that `snap` put an edge in, as it is said of the edge:
/// the cut, or the pause or gap and the words either side of it.
fn place_said(media: &Media, (snap, place): (Snap, &Place), speech: &Speech) -> String {
    let position = |seconds: f64| {
        Clock::of_media(media).print_position(media.rate.round(media_time(seconds)))
    };
    let (start, end) = place.quiet;
    match snap {
        Snap::Cut => format!("the cut of `{}` at {}", media.id, position(start)),
        Snap::Pause => {
            // The words either side of where it is now, where a
            // transcript heard them near.
            let mut heard = speech.heard();
            heard.retain(|word| !is_a_sound_note(&word.text));
            let (before, after) = beside(&heard, place.at, place.at);
            let before = before.filter(|word| word.end > start - WORDS_BESIDE_SECONDS);
            let after = after.filter(|word| word.start < end + WORDS_BESIDE_SECONDS);
            let said = |word: &Word| quote(word.text.trim());
            let (quiet, beside) = match (place.gap, before.map(said), after.map(said)) {
                (true, Some(before), Some(after)) => {
                    ("gap", format!(", where {before} and {after} run together"))
                }
                (true, ..) => ("gap", String::new()),
                (false, Some(before), Some(after)) => {
                    ("pause", format!(", between {before} and {after}"))
                }
                (false, Some(before), None) => ("pause", format!(", after {before}")),
                (false, None, Some(after)) => ("pause", format!(", before {after}")),
                (false, None, None) => ("pause", String::new()),
            };
            let cut = if place.on_cut {
                format!(", on the cut of its picture at {}", position(place.at))
            } else {
                String::new()
            };
            format!(
                "the {:.2}s {quiet} of `{}` at {}{beside}{cut}",
                end - start,
                media.id,
                position(start)
            )
        }
    }
}

fn nothing_near(media: &Media, at: Time, snap: Snap, reach: Time, barred: &[Barred]) -> String {
    let what = match snap {
        Snap::Pause => "pause",
        Snap::Cut => "cut",
    };
    let clock = Clock::of_media(media);
    let position = |seconds: f64| clock.print_position(media.rate.round(media_time(seconds)));
    let why: Vec<String> = barred
        .iter()
        .map(|barred| match barred {
            Barred::Across {
                quiet: at,
                gap,
                word,
            } => format!(
                "the {} at {} is on the other side of {}",
                if *gap { "gap" } else { "pause" },
                position(at.0),
                quote(word.text.trim())
            ),
            Barred::Unheard { quiet: at } => format!(
                "the sound parts for {:.2}s at {}, which no transcript covers to say is \
                 between two words: look at `{}` with transcript there first",
                at.1 - at.0,
                position(at.0),
                media.id
            ),
            Barred::Inside { quiet: at, word } => format!(
                "the sound parts for {:.2}s at {} inside {}, by its transcript",
                at.1 - at.0,
                position(at.0),
                quote(word.text.trim())
            ),
        })
        .collect();
    let mut said = format!(
        "no {what} of `{}` within {:.2}s of {} would do here",
        media.id,
        reach.seconds(),
        clock.print_position(at)
    );
    if !why.is_empty() {
        said.push_str(": ");
        said.push_str(&why.join("; "));
    }
    said
}

/// `seconds` of a media on its frame grid: the frame it falls in, or for an
/// out point the end of that frame, so a word at an edge is kept whole.
fn source_time(seconds: f64, rate: Rate, out: bool) -> Time {
    let time = Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO);
    let frames = rate.frames_floor(time);
    let frames = if out && !rate.is_aligned(time) {
        frames + 1
    } else {
        frames
    };
    rate.time_of(frames)
}

/// `seconds` of a media as a time, none of it before its start.
fn media_time(seconds: f64) -> Time {
    Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO)
}

/// The frame of `rate` nearest `at` that is no earlier than `from` and no
/// later than `to`; `None` when no frame falls between them.
fn frame_within(rate: Rate, at: Time, (from, to): (Time, Time)) -> Option<Time> {
    let first = rate.frames_floor(from) + i64::from(!rate.is_aligned(from));
    let last = rate.frames_floor(to);
    (first <= last).then(|| rate.time_of(rate.frames_round(at).clamp(first, last)))
}

/// How near a quiet a word has to be for it to be named as beside it, in
/// seconds.
const WORDS_BESIDE_SECONDS: f64 = 2.0;

/// `snap=pause|cut` and `within=<amount>`, half a second unless given: what
/// an edge snaps to and how far it may move.
fn take_snap(fields: &mut Fields<'_>, clock: &Clock) -> Parsed<Option<(Snap, Time)>> {
    let line = fields.line();
    let snap = fields.take_with("snap", |text| match text {
        "pause" => Ok(Snap::Pause),
        "cut" => Ok(Snap::Cut),
        other => Err(format!("snap={other}? use pause or cut")),
    })?;
    let within = fields.take_with("within", |text| clock.duration(text))?;
    match (snap, within) {
        (Some(snap), within) => Ok(Some((
            snap,
            within.unwrap_or_else(|| Time::from_flicks(FLICKS_PER_SECOND / 2)),
        ))),
        (None, Some(_)) => Err(line.error(1, "within= goes with snap=")),
        (None, None) => Ok(None),
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
        Op::AddMedia(media) => {
            // A media that comes with its steps is its line and a line for each of them.
            let mut text = media_line(media).replacen("media ", "media add ", 1);
            for step in &media.steps {
                write!(text, "\nstep {} add {}", media.id, print_step(step)).unwrap();
            }
            text
        }
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
            if let Some(color) = change.color {
                write!(text, " color={}", color.id()).unwrap();
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
        Op::AddBus { sequence, bus } => {
            let rate = sequence_clock(sequence).unwrap_or(fallback).rate;
            let mut text = format!("bus add {}", bus.id);
            if let Some(name) = &bus.name {
                write!(text, " name={}", quote(name)).unwrap();
            }
            if let Some(to) = &bus.to {
                write!(text, " to={to}").unwrap();
            }
            text.push_str(&params_text(
                &bus.params,
                catalog.props(Domain::Audio),
                rate,
            ));
            text
        }
        Op::SetBus {
            sequence,
            bus,
            change,
        } => {
            let rate = sequence_clock(sequence).unwrap_or(fallback).rate;
            let mut text = format!("bus set {bus}");
            if let Some(name) = &change.name {
                write!(text, " name={}", quote(name.as_deref().unwrap_or_default())).unwrap();
            }
            match &change.to {
                Some(Some(to)) => write!(text, " to={to}").unwrap(),
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
                write!(text, " reset={}", resets.join(",")).unwrap();
            }
            for (name, value) in &change.params {
                if let Some(value) = value {
                    write!(text, " {name}={}", crate::text::print_param(value, rate)).unwrap();
                }
            }
            text
        }
        Op::RemoveBus { bus, .. } => format!("bus remove {bus}"),
        Op::BusFx {
            sequence,
            bus,
            change,
        } => {
            let rate = sequence_clock(sequence).unwrap_or(fallback).rate;
            match change {
                kerf_model::BusFx::Add { effect, index } => {
                    let mut text = format!(
                        "bus fx {bus} add {}",
                        print_fx_ref(&effect.fx, effect.version)
                    );
                    if let Some(index) = index {
                        write!(text, " at={}", index + 1).unwrap();
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
                        write!(text, " enabled={enabled}").unwrap();
                    }
                    let resets: Vec<&str> = params
                        .iter()
                        .filter(|(_, value)| value.is_none())
                        .map(|(name, _)| name.as_str())
                        .collect();
                    if !resets.is_empty() {
                        write!(text, " reset={}", resets.join(",")).unwrap();
                    }
                    for (name, value) in params {
                        if let Some(value) = value {
                            write!(text, " {name}={}", crate::text::print_param(value, rate))
                                .unwrap();
                        }
                    }
                    text
                }
                kerf_model::BusFx::Remove { index } => {
                    format!("bus fx {bus} remove {}", index + 1)
                }
            }
        }
        Op::SetTrack { track, change, .. } => {
            let mut text = format!("track set {track}");
            match &change.bus {
                Some(Some(bus)) => write!(text, " bus={bus}").unwrap(),
                Some(None) => text.push_str(" bus=\"\""),
                None => {}
            }
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
        Op::SetTrackRole { track, change, .. } => {
            let mut text = format!("track set {track}");
            if let Some(role) = &change.role {
                match role {
                    Some(role) => write!(text, " role={}", word_or_quote(role)).unwrap(),
                    None => text.push_str(" role=\"\""),
                }
            }
            if let Some(sync) = change.sync_locked {
                write!(text, " sync={}", if sync { "on" } else { "off" }).unwrap();
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
                if !speed.is_one() {
                    write!(text, " speed={}", print_speed(speed, clock.rate)).unwrap();
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
        Op::Roll { clip, delta, alone } => format!(
            "roll {clip} {}{}",
            clip_clock(clip).unwrap_or(fallback).print_delta(*delta),
            if *alone { " alone" } else { "" }
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
                    ClipChange::Parent(parent) => write!(
                        text,
                        " parent={}",
                        parent
                            .as_ref()
                            .map_or_else(|| quote(""), |parent| parent.to_string())
                    )
                    .unwrap(),
                    ClipChange::Retime(retime) => write!(text, " retime={}", retime.id()).unwrap(),
                    ClipChange::Angle(Some(angle)) => write!(text, " angle={angle}").unwrap(),
                    ClipChange::Angle(None) => text.push_str(" angle=\"\""),
                    ClipChange::Caption(caption) => {
                        write!(text, " caption={}", quote(caption)).unwrap()
                    }
                    ClipChange::Param(name, value) => params.push((name.clone(), value.clone())),
                }
            }
            text.push_str(&changes(&params, clock.rate));
            text
        }
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
                text.push_str(&mask_shape_text(shape, clock.rate));
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
