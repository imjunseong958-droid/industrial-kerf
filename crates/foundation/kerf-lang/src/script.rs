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

mod edits;
mod parts;
mod positions;
mod print;
mod snap;
mod speech;

pub use positions::sequence_position;
pub use print::{print_op, print_ops};

use crate::facts::{Breaks, Facts, NoFacts, Speech};
use crate::lex::{lex, Arg, Atom, Line};
use crate::text::{Fields, Unread};
use kerf_model::{
    apply, Catalog, Clip, Content, Diagnostic, Id, Media, MissingKind, Op, Project, Report,
};
use positions::Positions;
use std::cell::RefCell;
use std::collections::HashMap;
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
            "roll" | "slip" | "slide" => self.roll_slip_slide(fields, command),
            "split" => self.split(fields),
            "blade" => self.blade(fields),
            "move" => self.move_clip(fields),
            "set" => self.set(fields),
            "speed" => self.speed(fields),
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
        let at = self.project.locate(id.as_str()).ok_or_else(|| {
            line.missing(1, self.project.no_clip(id.as_str()))
                .missing(MissingKind::Clip, id.as_str())
        })?;
        Ok((
            self.project.clip_at(at),
            Positions::of(&self.project, at.sequence),
        ))
    }

    fn media_of(&self, clip: &Clip) -> Option<&Media> {
        match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()),
            _ => None,
        }
    }
}

fn required<T, E: Into<Unread>>(
    fields: &mut Fields<'_>,
    key: &'static str,
    parse: impl FnOnce(&str) -> Result<T, E>,
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

#[cfg(test)]
mod tests;
