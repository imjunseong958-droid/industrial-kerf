//! How a media's file is made, where Kerf or what is outside it, a model or a
//! renderer, makes it, and the making of it.
//!
//! The steps are written in one of two places ([`Home`]). They are the
//! document's own, under the media's line, where an edit writes them:
//!
//! ```text
//! media m3 path="shots/open.mp4" rate=16 dur=65f video=832x480
//!   render s2 {guide.mp4} pass=depth size=832x480 rate=16
//!   generate model=dawn prompt="a boat leaves the harbour at dawn" guide={guide.mp4} seed=7
//! ```
//!
//! Or they are in a recipe, a text file beside the project that the media's
//! line names (`recipe="shots/open.kerfrecipe"`), which is how they were
//! written before the document held them, and the one place a step that
//! runs a program of the recipe's own choosing (`run`) can be:
//!
//! ```text
//! kerf-recipe 1
//! # The opening shot: blocked in a renderer, then filmed by a model.
//! needs open.scene.json
//! run vork render open.scene.json --profile std/previs --out {guide.mp4}
//! generate model=dawn prompt="a boat leaves the harbour at dawn" guide={guide.mp4} first=refs/boat.png seed=7
//! ```
//!
//! A recipe is steps, run in order, and the last one writes the media's
//! file:
//!
//! - `run` is a program and its arguments, quoted where they hold spaces.
//!   It runs in the recipe's folder, as a supervised sidecar and never
//!   through a shell, so nothing in an argument is expanded or run.
//! - `render <sequence> {name}` has Kerf draw a sequence of the project,
//!   which is how a guide blocked with Kerf's own clips, cameras, and lights
//!   reaches a model. `size=WxH` and `rate=<fps>` draw it at a model's own
//!   size and rate, not the sequence's: a picture of another shape fills
//!   the size, and what is over is cut off evenly. The file has no sound.
//!   `pass=ids` or `pass=depth` draws, in place of the picture, which clip
//!   each pixel is of or how near it is ([`kerf_compose::Pass`]).
//! - `generate` asks a model ([`model`]) for the file: what it should show
//!   (`prompt=`), what guides it (`guide=`, `first=`, `last=`, `ref=`), when
//!   it happens (`signal=`, a signal clip of the project: [`signal`]), and
//!   for how long and at what size (`seconds=`, `size=`), with a `seed=`.
//!   `workflow=` hands a model whose program takes one a graph of its own to
//!   run, a file of the project ([`model`]).
//!   `cast=` and `style=` name who and what it shows and how it looks, as
//!   the project says them once for every recipe ([`cast`]).
//!
//! `{out}` stands for the media's file, and `{name}` for a file a step
//! makes on the way and a later one reads. `needs` names the files a recipe
//! reads, so that a change to one is seen ([`takes`]).
//!
//! Each making is a take, kept beside the ones before it; the document
//! says which one the media is. A recipe that is written again is kept the
//! same way ([`version`]): beside the one before it, as `open@2.kerfrecipe`,
//! with the takes of both in one folder. A recipe is a command someone wrote. Kerf
//! runs one only when a person asks for that media to be made, never on
//! opening or drawing a project.

pub mod cast;
pub mod graph;
pub mod model;
pub mod moves;
pub mod shots;
pub mod signal;
pub mod takes;
pub mod text;
pub mod told;

use crate::error::{EngineError, Result};
use crate::render::{render_program_with_cancellation, RenderOptions};
use crate::sidecar::{self, CancellationToken};
use kerf_compose::Pass;
use kerf_model::making::file_name;
pub(crate) use kerf_model::making::whole;
use kerf_model::{Media, Rate};
use model::{Ask, Models, Source};
use moves::Moves;
use signal::Signal;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub use graph::{graph, Graph};
pub use shots::{shots, Shot};
pub use takes::{take, takes, unmade, Take, Unmade};
pub use told::{ask_to_stop, told, Going, Told};

/// The first line of a recipe.
const HEADER: &str = "kerf-recipe 1";

/// What stands for the media's file, in an argument.
const OUT: &str = kerf_model::making::OUT;

/// Where the steps that make a media are written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Home {
    /// Under the media's line in the document, as its own code: an edit writes them,
    /// undo takes them back, and the files they name are named from the project's
    /// folder.
    Document,
    /// In a recipe's file, which the media's line names from the project's folder.
    /// The files it names are named from its own folder.
    File(String),
}

impl Home {
    /// Where the steps that make `media` are written: nowhere for a media nothing
    /// makes, which is footage.
    pub fn of(media: &Media) -> Option<Self> {
        if !media.steps.is_empty() {
            Some(Self::Document)
        } else {
            media.recipe.clone().map(Self::File)
        }
    }

    /// The folder its steps name files from, from the project's.
    pub(crate) fn folder(&self) -> PathBuf {
        match self {
            Self::Document => PathBuf::new(),
            Self::File(name) => folder_of(name),
        }
    }

    /// The steps of `media`, written here, as the text a recipe's file holds: read
    /// from the file, or written out from the document.
    pub(crate) fn text(&self, directory: &Path, media: &Media) -> Result<String> {
        match self {
            Self::Document => Ok(steps_text(&media.steps)),
            Self::File(name) => {
                let file = inside_project(directory, name, "recipe")?;
                std::fs::read_to_string(&file).map_err(|error| {
                    EngineError::msg(format!(
                        "cannot read the recipe {}: {error}",
                        file.display()
                    ))
                })
            }
        }
    }

    /// Where the takes of a media made from here, whose file is at `media`, are kept.
    pub(crate) fn place(&self, directory: &Path, media: &str) -> Result<takes::Place> {
        match self {
            Self::Document => takes::Place::of_media(directory, media),
            Self::File(name) => takes::Place::of(directory, name, media),
        }
    }

    /// What is written here, as an error names it: `the recipe` or `the steps`, and
    /// what a verb that follows it ends with.
    fn called(&self) -> (&'static str, &'static str) {
        match self {
            Self::Document => ("the steps", ""),
            Self::File(_) => ("the recipe", "s"),
        }
    }

    /// The name a shot made from here goes by: its recipe's, without its version, or
    /// the media's own id.
    pub fn shot(&self, media: &str) -> String {
        match self {
            Self::Document => media.to_owned(),
            Self::File(name) => shot_name(name),
        }
    }
}

/// The folder of the project the shot that is `media` keeps files of its own in, a copy
/// of a model's workflow among them, from the project's folder: its recipe's folder, or
/// for steps the document holds the folder its takes' folder is in. `None` for a media
/// nothing makes.
pub fn shot_folder(directory: &Path, media: &Media) -> Result<Option<PathBuf>> {
    match Home::of(media) {
        None => Ok(None),
        Some(Home::File(name)) => within(&folder_of(&name), ".", "recipe").map(Some),
        Some(home @ Home::Document) => Ok(Some(home.place(directory, &media.path)?.beside())),
    }
}

/// Steps the document holds, as the text a recipe's file holds them: what everything
/// that reads a making reads, a take's own record of what it was made from among them.
/// A step's line is its number and one, the first line saying what the text is.
pub fn steps_text(steps: &[kerf_model::making::Step]) -> String {
    let mut text = format!("{HEADER}\n");
    for step in steps {
        text.push_str(&kerf_lang::print_step(step));
        text.push('\n');
    }
    text
}

/// The steps a recipe's text says, as the document holds them under a media. A text
/// with a line the document has no place for is refused: a `run`, which names a program
/// to run, and a `needs`, which says what one reads.
pub fn document_steps(text: &str) -> Result<Vec<kerf_model::making::Step>> {
    let (steps, needs) = steps_of_text(text, None)?;
    match needs.first() {
        Some(file) => Err(EngineError::msg(format!(
            "`needs {file}` is a recipe file's alone: the steps a document holds are `render`, `generate`, and `join`, and what a sequence they draw reads is found from its clips"
        ))),
        None => Ok(steps),
    }
}

/// The steps a recipe's text says, as the document holds them, and the files its `needs`
/// lines name. With `folder`, the recipe's own from the project's, each file a step names
/// is named from the project's folder as a document names it: `../refs/boat.png` in
/// `shots` is `refs/boat.png`. A `run` is refused: a document names no program.
fn steps_of_text(
    text: &str,
    folder: Option<&Path>,
) -> Result<(Vec<kerf_model::making::Step>, Vec<String>)> {
    use kerf_model::making::Step as Written;
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
    if !matches!(lines.next(), Some((_, HEADER))) {
        return Err(EngineError::msg(format!("a recipe starts with `{HEADER}`")));
    }
    let from_project = |file: &str| -> Result<String> {
        match folder {
            Some(folder) => within(folder, file, "file the recipe names").map(|inside| stored(&inside)),
            None => Ok(file.to_owned()),
        }
    };
    let mut steps: Vec<Written> = Vec::new();
    let mut needs = Vec::new();
    for (number, line) in lines {
        let wrong = |what: String| EngineError::msg(format!("line {number}: {what}"));
        let words = words(line).map_err(wrong)?;
        match words.first().map(String::as_str) {
            Some("run") => {
                return Err(wrong(format!(
                    "`run {}` runs a program, and a document names none: the steps it holds are `render`, `generate`, and `join`. A `run` is a recipe file's alone",
                    words.get(1).map_or("", String::as_str)
                )))
            }
            Some("needs") => {
                for file in &words[1..] {
                    needs.push(from_project(file)?);
                }
                continue;
            }
            _ => {}
        }
        let made: Vec<String> = steps.iter().map(|step| step.makes().to_owned()).collect();
        let mut step = Written::parse(&words, &made).map_err(wrong)?;
        if let Written::Generate(ask) = &mut step {
            let sources = ask
                .guide
                .iter_mut()
                .chain(ask.guides.iter_mut().map(|(_, source)| source))
                .chain(&mut ask.first)
                .chain(&mut ask.last)
                .chain(&mut ask.references)
                .chain(&mut ask.workflow);
            for source in sources {
                if let Source::File(file) = source {
                    *file = from_project(file)?;
                }
            }
        }
        steps.push(step);
    }
    kerf_model::making::check(&steps).map_err(EngineError::msg)?;
    Ok((steps, needs))
}

/// What carrying a media's recipe into the document comes to: the edit that writes its
/// steps under its line, and what is worth knowing of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Carry {
    /// The recipe the media named, as the document named it.
    pub recipe: String,
    /// The lines of the edit: the media's line without `recipe=`, and a `step` line for
    /// each step, its files named from the project's folder.
    pub script: Vec<String>,
    /// What does not come with it, each as a line for whoever carries it.
    pub notes: Vec<String>,
}

/// Carry the recipe media `id` of `project` names into the document: its `render`,
/// `generate` and `join` lines as steps under the media, each file named from the project's
/// folder in place of the recipe's, and `recipe=` taken off the media's line. Nothing is
/// changed here: what comes back is the edit that does it. The recipe's file is left where
/// it is, and so are its takes, whose folder is the same. A recipe with a `run` is refused:
/// a document names no program, and it goes on being made from its file.
///
/// Once the edit is in the project, [`takes::mark_carried`] has the takes made from the
/// recipe held to the steps it became, so that none is asked for again for being carried.
pub fn carry(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<Carry> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let name = match Home::of(media) {
        Some(Home::File(name)) => name,
        Some(Home::Document) => {
            return Err(EngineError::msg(format!(
                "media `{id}` has its steps under its line in the document already"
            )))
        }
        None => return Err(not_made(id)),
    };
    let home = Home::File(name.clone());
    let text = home.text(directory, media)?;
    let (steps, needs) = steps_of_text(&text, Some(&home.folder()))
        .map_err(|error| EngineError::msg(format!("{name}: {error}")))?;
    if !steps.iter().any(|step| step.makes() == OUT) {
        return Err(EngineError::msg(format!(
            "{name}: the recipe's last step names no `{{{OUT}}}`, so nothing says where its file goes"
        )));
    }
    let mut notes = Vec::new();
    // What the recipe said it reads beside what its steps name: a mesh a sequence it draws
    // reads is found from the clip; anything else is no longer watched.
    let recipe = Recipe::of_steps(&steps);
    let meshes: Vec<String> = recipe
        .sequences()
        .flat_map(|sequence| takes::mesh_files(project, sequence))
        .collect();
    for file in needs.iter().filter(|file| !meshes.contains(file)) {
        notes.push(format!(
            "`{file}`, which the recipe said it reads (`needs`), is no longer watched: no step of the document reads it, so changing it does not ask for `{id}` again"
        ));
    }
    // The takes a recipe made are in a folder named for the recipe, and those of steps in
    // the document in one named for the media's file: the same, for a shot named as its
    // recipe is.
    let (was, now) = (
        home.place(directory, &media.path)?.stored(1),
        Home::Document.place(directory, &media.path)?.stored(1),
    );
    if was != now {
        notes.push(format!(
            "the takes made so far are kept as `{was}` and those made from now on as `{now}`: the media's file is not named as its recipe is. Have the media be one of its takes first (`kerf takes {id} --use <n>`) and they stay together"
        ));
    }
    let mut line = Media {
        recipe: None,
        ..media.clone()
    };
    line.steps.clear();
    let mut script = vec![kerf_lang::print_op(
        project,
        kerf_model::Catalog::builtin(),
        &kerf_model::Op::ReplaceMedia(line),
    )];
    script.extend(
        steps
            .iter()
            .map(|step| format!("step {id} add {}", kerf_lang::print_step(step))),
    );
    Ok(Carry {
        recipe: name,
        script,
        notes,
    })
}

/// The step lines a script gets to have media `id`, whose steps the document holds, made
/// by `now` in place of `was`, each a recipe's text of them ([`steps_text`]): what a change
/// made to the text, a name set or a step added, comes to as the edit that makes it. One
/// step that differs is that step set; anything else is every step written again.
pub fn step_lines(id: &str, was: &str, now: &str) -> Result<Vec<String>> {
    let (was, now) = (document_steps(was)?, document_steps(now)?);
    let differing: Vec<usize> = (0..was.len().max(now.len()))
        .filter(|index| was.get(*index) != now.get(*index))
        .collect();
    Ok(match differing.as_slice() {
        [] => Vec::new(),
        [one] if was.len() == now.len() => vec![format!(
            "step {id} set {} {}",
            one + 1,
            kerf_lang::print_step(&now[*one])
        )],
        _ => (1..=was.len())
            .rev()
            .map(|number| format!("step {id} remove {number}"))
            .chain(
                now.iter()
                    .map(|step| format!("step {id} add {}", kerf_lang::print_step(step))),
            )
            .collect(),
    })
}

/// Where the steps that make media `id` of `project` are written, and their text
/// ([`Home::text`]). A media nothing makes is refused, with how one is made.
pub fn written(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<(Home, String)> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let home = Home::of(media).ok_or_else(|| not_made(id))?;
    let text = home.text(directory, media)?;
    Ok((home, text))
}

/// That nothing says how media `id` is made, and how that is said.
fn not_made(id: &str) -> EngineError {
    EngineError::msg(format!(
        "media `{id}` has no steps that make it: write them under it with an edit, `step {id} add render <sequence> {{guide.mp4}}` and `step {id} add generate model=<name> prompt=\"…\" guide={{guide.mp4}}`"
    ))
}

/// One thing a recipe does on the way to the media's file.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A program and its arguments.
    Run {
        program: String,
        arguments: Vec<String>,
    },
    /// A sequence of the project, drawn by Kerf into the file `to` names,
    /// with no sound: at the sequence's size and rate, or at the ones named,
    /// and its picture, or the pass of it named.
    Render {
        sequence: String,
        to: String,
        size: Option<(u32, u32)>,
        rate: Option<Rate>,
        pass: Pass,
        /// A clip of the sequence drawn alone, white on black, in place of a pass.
        matte: Option<String>,
        /// The part of the sequence drawn, in seconds from its start and for
        /// how long: the whole of it where neither is said. A shot longer
        /// than a model makes at once is drawn a part at a time.
        from: Option<f64>,
        seconds: Option<f64>,
    },
    /// A model asked for a file.
    Generate(Box<Ask>),
    /// Pictures in motion steps before it made, set end to end into the
    /// file `to` names: the parts of a shot longer than a model makes at
    /// once. Each part after the first loses its first `lap` frames, the
    /// ones it has of the part before it where it went on from that one's
    /// last frame.
    Join {
        parts: Vec<String>,
        to: String,
        lap: u32,
    },
}

impl From<kerf_model::making::Step> for Step {
    fn from(step: kerf_model::making::Step) -> Self {
        use kerf_model::making::Step as Written;
        match step {
            Written::Render {
                sequence,
                to,
                size,
                rate,
                pass,
                matte,
                from,
                seconds,
            } => Self::Render {
                sequence,
                to,
                size,
                rate,
                pass,
                matte,
                from,
                seconds,
            },
            Written::Generate(ask) => Self::Generate(ask),
            Written::Join { parts, to, lap } => Self::Join { parts, to, lap },
        }
    }
}

/// A recipe as its file says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    /// The files it reads, as it names them: from its own folder.
    pub needs: Vec<String>,
    pub steps: Vec<Step>,
}

impl Recipe {
    /// Read a recipe's text. What is wrong is said with its line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        match lines.next() {
            Some((_, HEADER)) => {}
            _ => return Err(EngineError::msg(format!("a recipe starts with `{HEADER}`"))),
        }
        let mut recipe = Self {
            needs: Vec::new(),
            steps: Vec::new(),
        };
        // The files steps have made so far, which later steps may read.
        let mut made: Vec<String> = Vec::new();
        for (number, line) in lines {
            let wrong =
                |what: String| EngineError::msg(format!("line {number} of the recipe: {what}"));
            let mut words = words(line).map_err(wrong)?;
            let verb = words.first().cloned().unwrap_or_default();
            if verb != "needs" && recipe.writes_out() {
                return Err(wrong(format!(
                    "the step before it writes `{{{OUT}}}`, the media's file, so it is the last"
                )));
            }
            match verb.as_str() {
                "needs" if words.len() > 1 => recipe.needs.extend(words.drain(1..)),
                "run" if words.len() > 1 => {
                    let program = words.remove(1);
                    words.remove(0);
                    made.extend(words.iter().flat_map(|word| stands_for(word)));
                    recipe.steps.push(Step::Run {
                        program,
                        arguments: words,
                    });
                }
                // What the document holds too, read as it reads them.
                "render" if words.len() > 2 => step(&mut recipe, &mut made, &words).map_err(wrong)?,
                "join" | "generate" if words.len() > 1 => {
                    step(&mut recipe, &mut made, &words).map_err(wrong)?;
                }
                "needs" | "run" | "render" | "generate" | "join" => {
                    return Err(wrong(format!(
                        "`{verb}` is `needs <file> …`, `run <program> <arguments>`, `render <sequence> {{name}} [size=WxH] [rate=<fps>] [pass=ids|depth|relative|normals] [from=<seconds>] [seconds=<seconds>]`, `generate model=<name> prompt=\"…\" …`, or `join {{part}} {{part}} … {{name}} [lap=<frames>]`"
                    )))
                }
                other => {
                    return Err(wrong(format!(
                        "`{other}` is not a recipe's word; it has `needs`, `run`, `render`, `generate`, and `join`"
                    )))
                }
            }
        }
        if recipe.steps.is_empty() {
            return Err(EngineError::msg(
                "the recipe has no step: a `run`, a `render`, a `generate`, or a `join` line",
            ));
        }
        if !recipe.writes_out() {
            return Err(EngineError::msg(format!(
                "the recipe's last step names no `{{{OUT}}}`, so nothing says where its file goes"
            )));
        }
        Ok(recipe)
    }

    /// The steps a document holds under a media, as a recipe's file would say them.
    pub fn of_steps(steps: &[kerf_model::making::Step]) -> Self {
        Self {
            needs: Vec::new(),
            steps: steps.iter().cloned().map(Step::from).collect(),
        }
    }

    /// Whether the last step so far writes the media's file.
    fn writes_out(&self) -> bool {
        match self.steps.last() {
            Some(Step::Run { arguments, .. }) => arguments
                .iter()
                .any(|argument| stands_for(argument).any(|name| name == OUT)),
            Some(Step::Render { to, .. }) => to == OUT,
            Some(Step::Generate(ask)) => ask.out == OUT,
            Some(Step::Join { to, .. }) => to == OUT,
            None => false,
        }
    }

    /// The models its steps ask.
    pub fn models(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().filter_map(|step| match step {
            Step::Generate(ask) => Some(ask.model.as_str()),
            _ => None,
        })
    }

    /// The clips whose way across the frame its steps hand a model, by their ids, each
    /// once.
    pub fn moves(&self) -> Vec<&str> {
        let mut moves: Vec<&str> = self
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Generate(ask) => Some(&ask.moves),
                _ => None,
            })
            .flatten()
            .map(String::as_str)
            .collect();
        moves.sort_unstable();
        moves.dedup();
        moves
    }

    /// The signals its steps hand a model: clips of the project, by their
    /// ids, each once.
    pub fn signals(&self) -> Vec<&str> {
        let mut signals: Vec<&str> = self
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Generate(ask) => Some(&ask.signals),
                _ => None,
            })
            .flatten()
            .map(String::as_str)
            .collect();
        signals.sort_unstable();
        signals.dedup();
        signals
    }

    /// The sequences its steps have Kerf draw.
    pub fn sequences(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().filter_map(|step| match step {
            Step::Render { sequence, .. } => Some(sequence.as_str()),
            _ => None,
        })
    }

    /// Every file it reads, as it names them: what `needs` lists and what
    /// a model is handed that no step made.
    pub fn reads(&self) -> Vec<&str> {
        let mut files: Vec<&str> = self.needs.iter().map(String::as_str).collect();
        for step in &self.steps {
            if let Step::Generate(ask) = step {
                files.extend(ask.sources().filter_map(|source| match source {
                    Source::File(file) => Some(file.as_str()),
                    Source::Made(_) => None,
                }));
            }
        }
        files.sort_unstable();
        files.dedup();
        files
    }

    /// The files its steps make on the way, by the names they have in
    /// braces, the media's own left out.
    pub fn makes(&self) -> Vec<String> {
        let mut made: Vec<String> = self
            .steps
            .iter()
            .flat_map(|step| match step {
                Step::Run { arguments, .. } => arguments
                    .iter()
                    .flat_map(|argument| stands_for(argument))
                    .collect(),
                Step::Render { to, .. } | Step::Join { to, .. } => vec![to.clone()],
                Step::Generate(ask) => vec![ask.out.clone()],
            })
            .filter(|name| name != OUT)
            .collect();
        made.sort_unstable();
        made.dedup();
        made
    }

    /// The recipe as roll `roll` of it is asked: each seed a `generate` line gives, that
    /// many on. A model that gives the same file for the same seed gives another for
    /// another roll, with nothing else of the shot changed.
    fn rolled(mut self, roll: u32) -> Self {
        for step in &mut self.steps {
            if let Step::Generate(ask) = step {
                ask.seed = ask.seed.map(|seed| seed.wrapping_add(u64::from(roll)));
            }
        }
        self
    }

    /// The recipe with no comments and no spare space, which is what a
    /// take is held to: a comment changed is not the recipe changed.
    pub fn essence(text: &str) -> String {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Read `words`, a `render`, a `generate` or a `join` line, as the step the document
/// would hold ([`kerf_model::making::Step::parse`]), and add it to `recipe` with the file
/// it makes among `made`.
fn step(recipe: &mut Recipe, made: &mut Vec<String>, words: &[String]) -> std::result::Result<(), String> {
    let step = kerf_model::making::Step::parse(words, made)?;
    made.push(step.makes().to_owned());
    recipe.steps.push(step.into());
    Ok(())
}

/// The names in braces an argument holds: `out` and `guide.mp4` of
/// `--out={out}` and `{guide.mp4}`. Braces around anything but a file's
/// name, such as a line of JSON, stand for themselves.
fn stands_for(argument: &str) -> impl Iterator<Item = String> + '_ {
    argument.split('{').skip(1).filter_map(|rest| {
        let name = rest.split_once('}')?.0;
        file_name(name).then(|| name.to_owned())
    })
}

/// `argument` with each `{name}` it holds as the file `place` gives it.
fn placed(argument: &str, place: &dyn Fn(&str) -> PathBuf) -> String {
    let mut out = String::new();
    let mut rest = argument;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.split_once('}') {
            Some((name, tail)) if file_name(name) => {
                out.push_str(&place(name).to_string_lossy());
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out + rest
}

/// `words` as a person reads a command, quoted where one holds a space.
fn shown(words: impl IntoIterator<Item = String>) -> String {
    words
        .into_iter()
        .map(|word| {
            if word.is_empty() || word.contains(char::is_whitespace) {
                format!("\"{word}\"")
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What making a media runs and writes, for a person to see before it is
/// run.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Where the steps are written.
    pub home: Home,
    /// The recipe's file, where they are in one.
    pub recipe: Option<PathBuf>,
    /// The take this making would be.
    pub take: u32,
    /// The take's file, which the run writes.
    pub file: PathBuf,
    /// The take's path as the document will have it.
    pub stored: String,
    /// Each step as a person reads it, in order.
    pub steps: Vec<String>,
    /// The steps, a line each.
    pub command: String,
    /// What is worth knowing before it is run, where a model has said what
    /// it takes and the recipe asks it for something else: a guide drawn at
    /// another size, a shot longer than it makes, a file it does not read.
    /// None of these stops a making.
    pub notes: Vec<String>,
    /// Which roll of the recipe this making is: how far on each seed is from the one its
    /// line gives. The recipe as it is written is roll 0.
    pub roll: u32,
    /// The media made.
    media: String,
    /// The steps as they were read for this plan, which is what runs.
    text: String,
}

/// `path`, which a project names, as a file inside `directory`: a recipe is
/// a command to run and a media's file is written over, so neither may be
/// somewhere else.
fn inside_project(directory: &Path, path: &str, what: &str) -> Result<PathBuf> {
    within(Path::new(""), path, what).map(|inside| directory.join(inside))
}

/// `path`, as a file in `folder` of a project names it, from the project's
/// folder: `../refs/boat.png` from `shots` is `refs/boat.png`. A path that
/// leaves the project is refused.
pub(crate) fn within(folder: &Path, path: &str, what: &str) -> Result<PathBuf> {
    let outside = || {
        EngineError::msg(format!(
            "the {what} `{path}` is not inside the project, so it is left alone"
        ))
    };
    let mut inside = PathBuf::new();
    for part in folder.components().chain(Path::new(path).components()) {
        match part {
            Component::Normal(name) => inside.push(name),
            Component::CurDir => {}
            Component::ParentDir if inside.pop() => {}
            _ => return Err(outside()),
        }
    }
    Ok(inside)
}

/// The file a recipe's text is kept in when it is written as the recipe
/// `asked` names, and whether it has to be written. A recipe is never
/// written over: a text that is not the one there is kept beside it, as its
/// next version, so the document names the version a media is made from and
/// undo goes back to the one before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// The file, from the project's folder, as a document names it.
    pub path: String,
    /// Whether it is a new file, to be written; else it holds the text.
    pub new: bool,
    /// What the file `asked` names held, when it was there and is not the text.
    pub before: Option<String>,
}

/// Where `text` is kept, written as the recipe `asked` names in the project
/// in `directory`: there, when nothing is; in the version of it that holds
/// the same text already, `shots/open@2.kerfrecipe` or `shots/open.kerfrecipe`
/// itself; or in the next version. The text is read as a recipe first, and
/// what is wrong in it said with its line.
pub fn version(directory: &Path, asked: &str, text: &str) -> Result<Version> {
    let inside = within(Path::new(""), asked, "recipe")?;
    if inside.extension().and_then(|extension| extension.to_str()) != Some(RECIPE_EXTENSION) {
        return Err(EngineError::msg(format!(
            "a recipe is a `.{RECIPE_EXTENSION}` file, not `{asked}`"
        )));
    }
    Recipe::parse(text).map_err(|error| EngineError::msg(format!("{asked}: {error}")))?;
    let read = |path: &Path| std::fs::read_to_string(directory.join(path)).ok();
    let Some(there) = read(&inside) else {
        return Ok(Version {
            path: stored(&inside),
            new: true,
            before: None,
        });
    };
    if there == text {
        return Ok(Version {
            path: stored(&inside),
            new: false,
            before: None,
        });
    }
    let held = versions(directory, &inside);
    if let Some((path, _, _)) = held.iter().find(|(_, _, held)| *held == text) {
        return Ok(Version {
            path: stored(path),
            new: false,
            before: Some(there),
        });
    }
    let last = held.iter().map(|(_, number, _)| *number).max().unwrap_or(1);
    Ok(Version {
        path: stored(&version_named(&inside, last + 1)),
        new: true,
        before: Some(there),
    })
}

/// The file of version `number` of the recipe at `recipe`, from the project's folder.
fn version_named(recipe: &Path, number: u32) -> PathBuf {
    let (base, _) = versioned(recipe);
    let name = if number == 1 {
        format!("{base}.{RECIPE_EXTENSION}")
    } else {
        format!("{base}@{number}.{RECIPE_EXTENSION}")
    };
    recipe.with_file_name(name)
}

/// The versions of the recipe at `recipe` that are there, in the project in `directory`: each
/// file from the project's folder, its number, and its text.
pub(crate) fn versions(directory: &Path, recipe: &Path) -> Vec<(PathBuf, u32, String)> {
    let mut found = Vec::new();
    let mut last = 1;
    for number in 1.. {
        let path = version_named(recipe, number);
        match std::fs::read_to_string(directory.join(&path)) {
            Ok(text) => {
                last = number;
                found.push((path, number, text));
            }
            // A version is there or not; numbers past a gap of a few are not looked for.
            Err(_) if number > last + 8 => break,
            Err(_) => {}
        }
    }
    found
}

/// Write `text` as `version` says, in the project in `directory`: a new file
/// only, never one that is there.
pub fn write_version(directory: &Path, version: &Version, text: &str) -> Result<()> {
    if !version.new {
        return Ok(());
    }
    let file = directory.join(&version.path);
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    use std::io::Write;
    let mut written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .map_err(|error| {
            EngineError::msg(format!(
                "cannot write the recipe {}: {error}",
                file.display()
            ))
        })?;
    written.write_all(text.as_bytes())?;
    Ok(())
}

/// Whether the recipes `a` and `b` name, from the project's folder, are versions of one recipe:
/// `shots/open.kerfrecipe` and `shots/open@2.kerfrecipe` are.
pub fn same_recipe(a: &str, b: &str) -> bool {
    let family = |name: &str| {
        within(Path::new(""), name, "recipe")
            .ok()
            .map(|path| (path.parent().map(Path::to_path_buf), versioned(&path).0))
    };
    family(a).is_some_and(|a| Some(a) == family(b))
}

/// A recipe file's name without its extension, split into the name its
/// versions share and its version: `open@2.kerfrecipe` is `open`, 2.
pub(crate) fn versioned(recipe: &Path) -> (String, u32) {
    let stem = recipe
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    match stem.rsplit_once('@') {
        Some((base, number))
            if !base.is_empty()
                && !number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            match number.parse::<u32>() {
                Ok(number) if number >= 2 => (base.to_owned(), number),
                _ => (stem, 1),
            }
        }
        _ => (stem, 1),
    }
}

/// The name the versions of the recipe `recipe` names share, which is the
/// shot's: `open` of `shots/open@2.kerfrecipe`.
pub fn shot_name(recipe: &str) -> String {
    versioned(Path::new(recipe)).0
}

/// Where the file `file` is, named as the steps that make media `id` name
/// their files, from a recipe's own folder or, for steps in the document, the
/// project's: its place on this machine, and its path from the project's
/// folder as a document would write it. A path that leaves the project is
/// refused, and so is a media nothing makes.
pub fn beside(
    project: &kerf_model::Project,
    directory: &Path,
    id: &str,
    file: &str,
) -> Result<(PathBuf, String)> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let home = Home::of(media).ok_or_else(|| not_made(id))?;
    let inside = within(&home.folder(), file, "file its steps name")?;
    Ok((directory.join(&inside), stored(&inside)))
}

/// What a recipe's file ends with.
const RECIPE_EXTENSION: &str = "kerfrecipe";

/// `directory` as a whole path, from the root.
fn rooted(directory: &Path) -> Result<PathBuf> {
    std::path::absolute(directory).map_err(|error| {
        EngineError::msg(format!(
            "cannot find the project {}: {error}",
            directory.display()
        ))
    })
}

/// A project's path with `/` between its parts, as documents write them.
pub(crate) fn stored(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The folder of the project a recipe is in, from the project's folder.
fn folder_of(recipe: &str) -> PathBuf {
    Path::new(recipe)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The steps that make a media, read: where they are written, their text, and what
/// the text says.
pub(crate) struct Read {
    pub home: Home,
    pub text: String,
    pub recipe: Recipe,
}

/// Read the steps that make media `id` of `project`, from its recipe or from the document.
pub(crate) fn read(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<Read> {
    let (home, text) = written(project, directory, id)?;
    let recipe = Recipe::parse(&text)
        .map_err(|error| in_document(&home, error))?
        .told(directory, &home.folder())?;
    Ok(Read { home, text, recipe })
}

/// `error`, which reading a recipe's text gave, as it is said of steps the document
/// holds: there is no recipe to name, and their last step is what is missing.
fn in_document(home: &Home, error: EngineError) -> EngineError {
    match home {
        Home::File(_) => error,
        Home::Document => EngineError::msg(
            error
                .to_string()
                .replace("the recipe's last step names", "the last of its steps names")
                .replace("the recipe has no step", "it has no step"),
        ),
    }
}

/// What making media `id` of `project`, kept in `directory`, would run:
/// its recipe read, and nothing run. A recipe that could not run is
/// refused here, with why: a model that is not set up, a sequence the
/// project has not, a file it needs that is not there.
pub fn plan(
    project: &kerf_model::Project,
    directory: &Path,
    id: &str,
    models: &Models,
) -> Result<Plan> {
    plan_rolled(project, directory, id, models, 0)
}

/// The roll a making of media `id` that is to be another of the same shot takes: one past
/// the last roll a take of its recipe, as the recipe is written now, was made with, and 0
/// where it has none.
pub fn next_roll(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<u32> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    Ok(takes(project, directory, media)?
        .iter()
        .filter(|take| take.current)
        .map(|take| take.rolled + 1)
        .max()
        .unwrap_or(0))
}

/// [`plan`], of roll `roll` of the recipe: the same shot asked for with each seed its
/// lines give `roll` on, so that several takes of one recipe are not one picture.
pub fn plan_rolled(
    project: &kerf_model::Project,
    directory: &Path,
    id: &str,
    models: &Models,
    roll: u32,
) -> Result<Plan> {
    // Each path a plan holds is whole: a step runs in its recipe's folder, where a path from
    // wherever this was asked does not lead.
    let directory = &rooted(directory)?;
    let Read { home, text, recipe } = read(project, directory, id)?;
    let recipe = recipe.rolled(roll);
    let media = project.media(id).expect("read found it");
    let folder = home.folder();
    let (what, s) = home.called();
    for sequence in recipe.sequences() {
        if project.sequence(sequence).is_none() {
            return Err(EngineError::msg(format!(
                "{what} render{s} sequence `{sequence}`, which this project has not"
            )));
        }
    }
    let made = recipe.makes();
    for file in recipe.reads() {
        let inside = within(&folder, file, "file the making needs")?;
        if !directory.join(&inside).is_file() {
            // The two ways a file that is there is named so that it is not found.
            let why = if made.iter().any(|name| name == file) {
                format!(
                    ": `{file}` is a file a step makes, which a later step names in braces, `{{{file}}}`"
                )
            } else if stored(&inside) != file {
                match &home {
                    Home::File(_) => format!(
                        ": it names `{file}`, and a file is named from the recipe's own folder, `{}`",
                        stored(&folder)
                    ),
                    Home::Document => format!(
                        ": it names `{file}`, and a file is named from the project's folder"
                    ),
                }
            } else {
                String::new()
            };
            return Err(EngineError::msg(format!(
                "{what} need{s} `{}`, which is not there{why}",
                stored(&inside)
            )));
        }
    }
    for signal in recipe.signals() {
        Signal::read(project, signal).map_err(EngineError::msg)?;
    }
    for clip in recipe.moves() {
        Moves::sequence_of(project, clip).map_err(EngineError::msg)?;
    }
    for model in recipe.models() {
        models.command(model)?;
    }
    let place = home.place(directory, &media.path)?;
    let take = place.next();
    let working = place.working(take);
    let steps = recipe
        .steps
        .iter()
        .map(|step| match step {
            Step::Run { program, arguments } => shown(
                std::iter::once(program.clone()).chain(
                    arguments
                        .iter()
                        .map(|argument| placed(argument, &|name| place.file_for(take, name))),
                ),
            ),
            Step::Join { parts, to, lap } => format!(
                "Kerf joins {} end to end into {}{}",
                parts
                    .iter()
                    .map(|part| format!("{{{part}}}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                place.file_for(take, to).display(),
                match lap {
                    0 => String::new(),
                    1 => ", each part after the first without its first frame".to_owned(),
                    lap => format!(", each part after the first without its first {lap} frames"),
                }
            ),
            Step::Render {
                sequence,
                to,
                size,
                rate,
                pass,
                matte,
                from,
                seconds,
            } => {
                let what = match (pass, matte) {
                    (_, Some(clip)) => format!("the matte of `{clip}` in "),
                    (Pass::Picture, None) => String::new(),
                    (pass, None) => format!("the {} of ", pass.name()),
                };
                let mut at = Vec::new();
                if let Some((width, height)) = size {
                    at.push(format!("{width}x{height}"));
                }
                if let Some(rate) = rate {
                    at.push(format!("{rate} frames a second"));
                }
                let part = match (from, seconds) {
                    (None, None) => String::new(),
                    (from, Some(seconds)) => format!(
                        ", {} s of it from {} s",
                        plain(*seconds),
                        plain(from.unwrap_or(0.0))
                    ),
                    (Some(from), None) => format!(", from {} s on", plain(*from)),
                };
                format!(
                    "Kerf renders {what}sequence {sequence}{part}{} to {}",
                    if at.is_empty() {
                        String::new()
                    } else {
                        format!(" at {}", at.join(", "))
                    },
                    place.file_for(take, to).display()
                )
            }
            Step::Generate(ask) => {
                let request = working.join(model::REQUEST_FILE);
                let command = models.command(&ask.model).expect("checked above");
                format!(
                    "{} asks model `{}` for {}",
                    shown(
                        command
                            .iter()
                            .map(|word| word.replace(model::REQUEST, &request.to_string_lossy()))
                    ),
                    ask.model,
                    ask.shown()
                )
            }
        })
        .collect::<Vec<_>>();
    Ok(Plan {
        recipe: match &home {
            Home::File(name) => Some(directory.join(name)),
            Home::Document => None,
        },
        take,
        file: place.file(take),
        stored: place.stored(take),
        command: steps.join("\n"),
        steps,
        notes: notes(project, &recipe, models),
        roll,
        media: id.to_owned(),
        home,
        text,
    })
}

/// How a shot longer than a model makes at once is made, as a note says it.
const IN_PARTS: &str = "a longer shot is made in parts, each a guide of its part (`render <sequence> {b.mp4} from=<seconds> seconds=<seconds>`) and a `generate` that goes on from the part before (`first={a.mp4} from=<seconds>`), set end to end by `join {a.mp4} {b.mp4} {out} lap=1`";

/// A number as a line writes it: whole where it is, else to four places,
/// which holds 81 frames at 16 a second, 5.0625 s, as it is.
fn plain(number: f64) -> String {
    let text = format!("{number:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Whether a pass sees `clip` in depth, as the compositor does: it says
/// `three_d=true`, or it is a mesh, which is in depth whether it says so or not.
fn in_depth(clip: &kerf_model::Clip) -> bool {
    use kerf_model::{Content, Param, Value};
    matches!(
        clip.props.get("three_d"),
        Some(Param::Static(Value::Bool(true)))
    ) || matches!(&clip.content, Content::Generator { fx, .. } if fx.as_str() == "mesh")
}

/// What `recipe` asks of its models that they have said they do not take,
/// each as a line for whoever is about to make it, and what it hands one
/// that says nothing. A model that has not said what it takes
/// ([`model::Takes`]) is asked nothing of here.
fn notes(project: &kerf_model::Project, recipe: &Recipe, models: &Models) -> Vec<String> {
    let mut notes = Vec::new();
    // A pass of how near things are, or which way they face, of a sequence with nothing in
    // depth is black on every frame: what a model is then handed tells it nothing.
    for step in &recipe.steps {
        let Step::Render {
            sequence, to, pass, ..
        } = step
        else {
            continue;
        };
        if !matches!(pass, Pass::Depth | Pass::Relative | Pass::Normals) {
            continue;
        }
        let Some(drawn) = project.sequence(sequence) else {
            continue;
        };
        let clips = || drawn.tracks.iter().flat_map(|track| &track.clips);
        if clips().any(in_depth) {
            continue;
        }
        let nested: Vec<&str> = clips()
            .filter_map(|clip| match &clip.content {
                kerf_model::Content::Sequence { sequence, .. } => Some(sequence.as_str()),
                _ => None,
            })
            .collect();
        notes.push(match nested.first() {
            Some(inner) => format!(
                "the {} of sequence `{sequence}` into `{{{to}}}` is black: none of its clips is in depth, and a nested sequence is drawn flat, as one clip: render `{inner}` itself",
                pass.name()
            ),
            None => format!(
                "the {} of sequence `{sequence}` into `{{{to}}}` is black: none of its clips is in depth (`three_d=true`, or a `gen=mesh` clip)",
                pass.name()
            ),
        });
    }
    for step in &recipe.steps {
        let Step::Generate(ask) = step else {
            continue;
        };
        // A signal that never changes says nothing of when, whatever the model.
        for signal in &ask.signals {
            if let Some(held) = Signal::read(project, signal)
                .ok()
                .and_then(|signal| signal.steady())
            {
                notes.push(format!(
                    "signal `{signal}` is {} on every frame, which says nothing of when: keyframe its `{value}`, as `set {signal} {value}={{0f:0 24f:1}}`",
                    plain(held),
                    value = kerf_model::SIGNAL_VALUE,
                ));
            }
        }
        let Some(takes) = models.takes(&ask.model) else {
            continue;
        };
        let model = &ask.model;
        // A guide Kerf draws, at another size or rate than the model reads it.
        let guide = ask.guide.as_ref().and_then(|guide| match guide {
            Source::Made(name) => recipe.steps.iter().find_map(|step| match step {
                Step::Render {
                    sequence,
                    to,
                    size,
                    rate,
                    from,
                    seconds,
                    ..
                } if to == name => Some((sequence, to, size, rate, from, seconds)),
                _ => None,
            }),
            Source::File(_) => None,
        });
        if let Some((sequence, to, size, rate, from, part)) = guide {
            // A guide longer than the model makes at once: what is past that is not read.
            if let (Some(most), Some(drawn)) = (takes.seconds, project.sequence(sequence)) {
                let long = part.unwrap_or(drawn.duration().seconds() - from.unwrap_or(0.0));
                if long > most + 1.0 / drawn.rate.fps().max(1.0) + 1e-6 {
                    notes.push(format!(
                        "the guide `{{{to}}}` is {} s and model `{model}` makes {} s at once, so the rest is left out: {IN_PARTS}",
                        plain(long),
                        plain(most)
                    ));
                }
            }
            let other_size = takes.size.is_some_and(|wanted| *size != Some(wanted));
            let other_rate = takes
                .rate
                .is_some_and(|wanted| rate.is_none_or(|rate| (rate.fps() - wanted).abs() > 1e-3));
            if other_size || other_rate {
                let mut line = format!("render {sequence} {{{to}}}");
                let mut what = Vec::new();
                if let Some((width, height)) = takes.size {
                    line.push_str(&format!(" size={width}x{height}"));
                    what.push(format!("{width}x{height}"));
                }
                if let Some(wanted) = takes.rate {
                    line.push_str(&format!(" rate={}", plain(wanted)));
                    what.push(format!("{} frames a second", plain(wanted)));
                }
                notes.push(format!(
                    "the guide `{{{to}}}` is not drawn as model `{model}` reads it, {}: `{line}` draws only what it reads, in less time",
                    what.join(" at ")
                ));
            }
            // A sequence of another shape than the picture the model makes loses its edges.
            let drawn = size.or(takes.size);
            if let (Some((width, height)), Some(sequence)) = (drawn, project.sequence(sequence)) {
                let (own, wanted) = (
                    f64::from(sequence.width) / f64::from(sequence.height.max(1)),
                    f64::from(width) / f64::from(height),
                );
                if (own / wanted - 1.0).abs() > 0.01 {
                    notes.push(format!(
                        "sequence `{}` is {}x{}, another shape than the {width}x{height} of its guide: the guide fills that size, and the {} of the sequence are cut off",
                        sequence.id,
                        sequence.width,
                        sequence.height,
                        if own > wanted { "sides" } else { "top and bottom" }
                    ));
                }
            }
        }
        if let (Some(asked), Some(most)) = (ask.seconds, takes.seconds) {
            if asked > most + 1e-6 {
                notes.push(format!(
                    "`seconds={}` is more than model `{model}` makes at once, {} s: {IN_PARTS}",
                    plain(asked),
                    plain(most)
                ));
            }
        }
        if let Some(reads) = &takes.reads {
            let named: Vec<String> = ask
                .guides
                .iter()
                .map(|(name, _)| format!("guide_{name}"))
                .collect();
            let given = [
                ("guide", ask.guide.is_some()),
                ("first", ask.first.is_some()),
                ("last", ask.last.is_some()),
                ("ref", !ask.references.is_empty()),
                (model::SIGNAL, !ask.signals.is_empty()),
                (model::MOVES, !ask.moves.is_empty()),
            ]
            .into_iter()
            .chain(named.iter().map(|name| (name.as_str(), true)))
            .collect::<Vec<_>>();
            for (name, _) in given
                .iter()
                .filter(|(name, given)| *given && !reads.iter().any(|read| read == name))
            {
                notes.push(format!(
                    "model `{model}` has no use for `{name}=`: {}",
                    if reads.is_empty() {
                        "it takes a prompt alone".to_owned()
                    } else {
                        format!("of what a line hands it, it reads {}", reads.join(", "))
                    }
                ));
            }
        }
        if !takes.more.is_empty() {
            for name in ask
                .more
                .keys()
                .filter(|name| !takes.more.contains_key(*name))
            {
                notes.push(format!(
                    "model `{model}` has nothing named `{name}` to set: beside what every model is asked, a line sets its {}",
                    takes.more.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    notes
}

/// What a making made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    /// The take it is.
    pub take: u32,
    /// The take's path as the document names it: what the media's line
    /// takes for the media to be this take.
    pub stored: String,
}

/// Run `plan`: each step of the recipe as it was read for the plan, in
/// order. Each line a program writes to stderr goes to `on_line`. The take
/// is there only when every step has ended well and the last has written
/// something: a failed run leaves no take, and the media as it was.
///
/// While it runs it says what it is doing where another program can see
/// it ([`told`]), and stops when one asks it to ([`ask_to_stop`]).
pub fn make(
    project: &kerf_model::Project,
    directory: &Path,
    plan: &Plan,
    models: &Models,
    cancellation: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<Made> {
    let directory = &rooted(directory)?;
    let media = project
        .media(&plan.media)
        .ok_or_else(|| EngineError::msg(format!("no media `{}` in this project", plan.media)))?;
    let folder = plan.home.folder();
    let recipe = Recipe::parse(&plan.text)?
        .told(directory, &folder)?
        .rolled(plan.roll);
    let place = plan.home.place(directory, &media.path)?;
    // What the take is made from, read before anything runs.
    let from = takes::from(project, directory, &plan.home, &plan.text, &recipe)?;
    for model in recipe.models() {
        models.command(model)?;
    }
    // A take made since the plan was shown keeps its number.
    let take = place.next().max(plan.take);
    let working = place.working(take);
    let making = place.making(take);
    std::fs::create_dir_all(&working)?;
    // What an earlier run left must not pass for this one's.
    let _ = std::fs::remove_file(&making);
    let making_it = Making {
        project,
        directory,
        folder: &folder,
        place: &place,
        take,
        models,
        cancellation,
    };
    let telling = told::Telling::begin(directory, &plan.media, take, cancellation);
    let made = (|| {
        let ran = making_it
            .run(&recipe, &mut |line| {
                telling.say(line);
                on_line(line);
            })
            .and_then(|()| match std::fs::metadata(&making) {
                Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Ok(()),
                _ => Err(EngineError::msg(format!(
                    "{} ended well and wrote nothing to {}",
                    plan.home.called().0,
                    making.display()
                ))),
            });
        if let Err(error) = ran {
            let _ = std::fs::remove_file(&making);
            let _ = std::fs::remove_dir_all(&working);
            return Err(error);
        }
        std::fs::rename(&making, place.file(take))?;
        takes::record(&place, take, from, plan.roll)?;
        // What it was made from, to go back to: not what the take stands on, so one that
        // cannot be kept is still a take.
        let _ = takes::keep(&place, take, &plan.text, plan.roll, &|named| {
            within(&folder, named, "file")
                .ok()
                .and_then(|inside| std::fs::read_to_string(directory.join(inside)).ok())
        });
        Ok(Made {
            take,
            stored: place.stored(take),
        })
    })();
    telling.end(made.as_ref().map(|made| made.take));
    made
}

/// One making under way: what it reads, where it runs, and what it writes.
struct Making<'a> {
    project: &'a kerf_model::Project,
    /// The project's folder, and the recipe's folder in it.
    directory: &'a Path,
    folder: &'a Path,
    place: &'a takes::Place,
    take: u32,
    models: &'a Models,
    cancellation: &'a CancellationToken,
}

impl Making<'_> {
    /// Run `command` in the recipe's folder to its end; one that does not
    /// end well is an error with what it said.
    fn supervised(
        &self,
        mut command: Command,
        program: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<()> {
        command.current_dir(self.directory.join(self.folder));
        let finished = sidecar::run(command, self.cancellation, on_line)?;
        if finished.status.success() {
            Ok(())
        } else {
            Err(EngineError::msg(format!(
                "{program} failed ({}): {}",
                finished.status,
                finished.stderr.trim()
            )))
        }
    }

    /// The file `{name}` stands for in this take's steps.
    fn file_for(&self, name: &str) -> PathBuf {
        self.place.file_for(self.take, name)
    }

    /// Run each step of `recipe`, in order, stopping at the first that fails.
    fn run(&self, recipe: &Recipe, on_line: &mut dyn FnMut(&str)) -> Result<()> {
        for step in &recipe.steps {
            match step {
                Step::Run { program, arguments } => {
                    let mut command = Command::new(program);
                    command.args(
                        arguments
                            .iter()
                            .map(|argument| placed(argument, &|name| self.file_for(name))),
                    );
                    self.supervised(command, program, on_line)?;
                }
                Step::Join { parts, to, lap } => {
                    on_line(&format!("joining {} parts", parts.len()));
                    let files: Vec<PathBuf> = parts.iter().map(|part| self.file_for(part)).collect();
                    let command = join_command(&files, &self.file_for(to), *lap);
                    self.supervised(command, "ffmpeg", &mut |_| {})?;
                }
                Step::Render {
                    sequence,
                    to,
                    size,
                    rate,
                    pass,
                    matte,
                    from,
                    seconds,
                } => {
                    let mut resolved = self.project.clone();
                    resolved.resolve_paths(kerf_model::Catalog::builtin(), self.directory);
                    let range = part_of(&resolved, sequence, *from, *seconds)?;
                    render_program_with_cancellation(
                        &resolved,
                        Some(sequence),
                        range,
                        &RenderOptions {
                            output: self.file_for(to).to_string_lossy().into_owned(),
                            project_dir: Some(self.directory.to_path_buf()),
                            size: *size,
                            rate: *rate,
                            pass: *pass,
                            matte: matte.clone(),
                            // What a step makes is for a model to follow, or a shot of its
                            // own: a picture, with no sound.
                            sound: false,
                            ..RenderOptions::default()
                        },
                        self.cancellation,
                        Some(|done: f64| {
                            on_line(&format!("rendering {sequence}: {:.0}%", done * 100.0));
                        }),
                    )?;
                }
                Step::Generate(ask) => {
                    let request = self.place.working(self.take).join(model::REQUEST_FILE);
                    let signals = ask
                        .signals
                        .iter()
                        .map(|signal| Signal::read(self.project, signal))
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(EngineError::msg)?;
                    let moves = if ask.moves.is_empty() {
                        Vec::new()
                    } else {
                        // Where a clip is is read from the frame, which wants its media.
                        let mut resolved = self.project.clone();
                        resolved.resolve_paths(kerf_model::Catalog::builtin(), self.directory);
                        on_line("reading where what moves goes");
                        ask.moves
                            .iter()
                            .map(|clip| Moves::read(&resolved, clip))
                            .collect::<std::result::Result<Vec<_>, _>>()
                            .map_err(EngineError::msg)?
                    };
                    let mut asked = model::request(
                        ask,
                        &|file| {
                            within(self.folder, file, "file the making needs")
                                .map(|inside| self.directory.join(inside))
                        },
                        &|name| self.file_for(name),
                        &signals,
                        &moves,
                    )?;
                    // A picture in motion given as the first frame is where it ended, and as
                    // the last, where it began: a shot that goes on from another, or into one.
                    for (name, end) in [("first", true), ("last", false)] {
                        let moving = asked[name]
                            .as_str()
                            .map(PathBuf::from)
                            .filter(|file| in_motion(file));
                        if let Some(file) = moving {
                            let still = self.place.working(self.take).join(format!(
                                "{}.{}.png",
                                file.file_stem().unwrap_or_default().to_string_lossy(),
                                if end { "last" } else { "first" }
                            ));
                            on_line(&format!(
                                "taking the {} frame of {}",
                                if end { "last" } else { "first" },
                                file.file_name().unwrap_or_default().to_string_lossy()
                            ));
                            self.supervised(still_command(&file, &still, end), "ffmpeg", &mut |_| {})?;
                            asked[name] = serde_json::Value::String(still.to_string_lossy().into_owned());
                        }
                    }
                    std::fs::write(&request, serde_json::to_vec_pretty(&asked)?)?;
                    let words = model::command_for(
                        &ask.model,
                        self.models.command(&ask.model)?,
                        &request,
                        &asked,
                        self.models.takes(&ask.model),
                    )?;
                    let mut command = Command::new(&words[0]);
                    command.args(&words[1..]);
                    self.supervised(command, &words[0], on_line)?;
                }
            }
        }
        Ok(())
    }
}

/// Whether `file` is named as a picture in motion is.
pub(crate) fn in_motion(file: &Path) -> bool {
    file.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["mp4", "mov", "mkv", "webm", "m4v"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// The command that writes the last frame of `file`, or its first, to `still`.
fn still_command(file: &Path, still: &Path, last: bool) -> Command {
    let mut command = crate::ffmpeg::hidden_command(crate::ffmpeg::resolve_ffmpeg_bin());
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if last {
        // The last second of it, each frame written over the one before: what is left is the last.
        command.args(["-sseof", "-1"]).arg("-i").arg(file).args(["-update", "1"]);
    } else {
        command.arg("-i").arg(file).args(["-frames:v", "1"]);
    }
    command.arg(still);
    command
}

/// The command that sets `parts` end to end into `to`, each after the first without its
/// first `lap` frames, as a picture with no sound.
fn join_command(parts: &[PathBuf], to: &Path, lap: u32) -> Command {
    let mut command = crate::ffmpeg::hidden_command(crate::ffmpeg::resolve_ffmpeg_bin());
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for part in parts {
        command.arg("-i").arg(part);
    }
    let mut filter = String::new();
    let mut joined = String::new();
    for index in 0..parts.len() {
        if index > 0 && lap > 0 {
            filter.push_str(&format!(
                "[{index}:v]trim=start_frame={lap},setpts=PTS-STARTPTS[p{index}];"
            ));
            joined.push_str(&format!("[p{index}]"));
        } else {
            joined.push_str(&format!("[{index}:v]"));
        }
    }
    filter.push_str(&format!("{joined}concat=n={}:v=1:a=0[v]", parts.len()));
    command
        .args(["-filter_complex", &filter, "-map", "[v]", "-an"])
        .args(["-c:v", "libx264", "-preset", "medium", "-crf", "14", "-pix_fmt", "yuv420p"])
        .args(["-f", "mp4"])
        .arg(to);
    command
}

/// The part of `sequence` a `render` step draws: none for the whole of it.
fn part_of(
    project: &kerf_model::Project,
    sequence: &str,
    from: Option<f64>,
    seconds: Option<f64>,
) -> Result<Option<(kerf_model::Time, kerf_model::Time)>> {
    if from.is_none() && seconds.is_none() {
        return Ok(None);
    }
    let drawn = project
        .sequence(sequence)
        .ok_or_else(|| EngineError::msg(format!("no sequence `{sequence}`")))?;
    let ends = drawn.duration();
    let time = |seconds: f64| {
        kerf_model::Time::from_seconds(seconds)
            .ok_or_else(|| EngineError::msg(format!("{seconds} s is not a time")))
    };
    let start = time(from.unwrap_or(0.0))?;
    if start >= ends {
        return Err(EngineError::msg(format!(
            "`render {sequence} … from={}` starts where the sequence ends, at {} s",
            plain(from.unwrap_or(0.0)),
            plain(ends.seconds())
        )));
    }
    let end = match seconds {
        Some(seconds) => time(from.unwrap_or(0.0) + seconds)?.min(ends),
        None => ends,
    };
    Ok(Some((start, end)))
}

/// A line's words: split at spaces, with `"…"` keeping what it holds as
/// one word, `\"` and `\\` inside it standing for themselves. A recipe's lines are read so, and
/// those of the other text files a project keeps beside its document.
pub fn words(line: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                started = true;
                loop {
                    match characters.next() {
                        Some('"') => break,
                        Some('\\') => match characters.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => return Err("a quote is not closed".to_owned()),
                        },
                        Some(other) => word.push(other),
                        None => return Err("a quote is not closed".to_owned()),
                    }
                }
            }
            space if space.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            other => {
                started = true;
                word.push(other);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

#[cfg(test)]
mod tests;
