//! How a media's file is made, where Kerf or a model makes it, and the making
//! of it.
//!
//! The steps are the document's own, under the media's line, where an edit
//! writes them ([`kerf_model::making`]):
//!
//! ```text
//! media m3 path="shots/open.mp4" rate=16 dur=65f video=832x480
//!   render s2 {guide.mp4} pass=depth size=832x480 rate=16
//!   generate model=dawn prompt="a boat leaves the harbour at dawn" guide={guide.mp4} seed=7
//! ```
//!
//! They run in order, and the last one writes the media's file:
//!
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
//!   the project says them once for every shot ([`cast`]).
//! - `join` sets files earlier steps made end to end.
//!
//! `{out}` stands for the media's file, and `{name}` for a file a step
//! makes on the way and a later one reads. A file of the project is named
//! from the project's folder.
//!
//! No step names a program. Kerf draws, and a model, or a renderer that is
//! handed its arguments, is a program the person names in a file of their
//! own ([`model`]). Kerf runs steps only when a media is asked to be made,
//! never on opening or drawing a project.
//!
//! Each making is a take, kept beside the ones before it; the document says
//! which one the media is ([`takes`]).
//!
//! Before the document held them, steps were written in a recipe, a text
//! file the media's line named (`recipe="shots/open.kerfrecipe"`). Nothing
//! is made from one now. [`carry`] reads one, once, to write its steps
//! under the media; a recipe that runs a program of its own choosing
//! (`run`) has no steps to carry, a document naming none.
//!
//! Where steps are handed about as text, to an editor that shows them or
//! kept with a take, it is as the lines the document writes after one that
//! says what they are ([`steps_text`]).

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
use kerf_model::Media;
use model::{Models, Source};
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

/// The recipe file `media` names and has not been carried from: its steps are there, and
/// nothing is made from them until they are the document's ([`carry`]).
pub fn uncarried(media: &Media) -> Option<&str> {
    media.recipe.as_deref().filter(|_| media.steps.is_empty())
}

/// Where the takes of `media` are kept: in the folder named for its file, or, for one
/// whose steps are still in a recipe's file, in the one named for the recipe, where the
/// makings of that file put them. `None` for footage, which nothing makes.
pub(crate) fn place_of(directory: &Path, media: &Media) -> Option<Result<takes::Place>> {
    if !media.steps.is_empty() {
        Some(takes::Place::of_media(directory, &media.path))
    } else {
        uncarried(media).map(|recipe| takes::Place::of(directory, recipe, &media.path))
    }
}

/// The folder of the project the shot that is `media` keeps files of its own in, a copy
/// of a model's workflow among them, from the project's folder: the one its takes'
/// folder is in.
pub fn shot_folder(directory: &Path, media: &Media) -> Result<PathBuf> {
    Ok(takes::Place::of_media(directory, &media.path)?.beside())
}

/// Steps as text: a line that says what the text is, then each step as the document
/// writes it. It is how steps are handed to what shows them and kept with a take, and
/// [`Recipe::parse`] of it is the steps again. A step's line is its number and one.
pub fn steps_text(steps: &[Step]) -> String {
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
            "`needs {file}` is not a step: the steps a document holds are `render`, `generate`, and `join`, and what a sequence they draw reads is found from its clips"
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
        return Err(EngineError::msg(format!("steps as text start with the line `{HEADER}`")));
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
                    "`run {program}` runs a program, and a document names none: the steps it holds are `render`, `generate`, and `join`. Name the program in your models file, with `{{out}}` where its file goes ({{\"{program}\": {{\"run\": [\"{program}\", …, \"{{out}}\"]}}}}), and write this line of the recipe as `generate model={program} … out={{<name>}}`: it is carried then",
                    program = words.get(1).map_or("", String::as_str)
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
    if !media.steps.is_empty() {
        return Err(EngineError::msg(format!(
            "media `{id}` has its steps under its line in the document already"
        )));
    }
    let name = uncarried(media).ok_or_else(|| not_made(id))?.to_owned();
    let file = inside_project(directory, &name, "recipe")?;
    let text = std::fs::read_to_string(&file).map_err(|error| {
        EngineError::msg(format!("cannot read the recipe {}: {error}", file.display()))
    })?;
    let (steps, needs) = steps_of_text(&text, Some(&folder_of(&name)))
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
        takes::Place::of(directory, &name, &media.path)?.stored(1),
        takes::Place::of_media(directory, &media.path)?.stored(1),
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

/// The steps that make media `id` of `project`, as text ([`steps_text`]). A media nothing
/// makes is refused, with how one is made; so is one whose steps are still in a recipe's
/// file, with how they are carried.
pub fn written(project: &kerf_model::Project, id: &str) -> Result<String> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    if media.steps.is_empty() {
        return Err(match uncarried(media) {
            Some(recipe) => not_carried(id, recipe),
            None => not_made(id),
        });
    }
    Ok(steps_text(&media.steps))
}

/// That nothing says how media `id` is made, and how that is said.
fn not_made(id: &str) -> EngineError {
    EngineError::msg(format!(
        "media `{id}` has no steps that make it: write them under it with an edit, `step {id} add render <sequence> {{guide.mp4}}` and `step {id} add generate model=<name> prompt=\"…\" guide={{guide.mp4}}`"
    ))
}

/// That media `id` still has its steps in the recipe file `recipe`, and how they come
/// into the document, where a making reads them.
pub fn not_carried(id: &str, recipe: &str) -> EngineError {
    EngineError::msg(format!(
        "the steps of media `{id}` are in the recipe file `{recipe}`, and a media is made from the steps under its line: `kerf carry {id}`, or an agent's `edit` with `carry: [\"{id}\"]`, writes them there"
    ))
}

pub use kerf_model::making::Step;

/// The steps that make a media, read for a making.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub steps: Vec<Step>,
}

impl Recipe {
    /// Read steps from their text ([`steps_text`]). What is wrong is said with its line,
    /// and steps that do not end in the media's file are refused: nothing would say
    /// where it goes.
    pub fn parse(text: &str) -> Result<Self> {
        let steps = document_steps(text)?;
        if steps.is_empty() {
            return Err(EngineError::msg(
                "it has no step: a `render`, a `generate`, or a `join` line",
            ));
        }
        if steps.last().is_some_and(|step| step.makes() != OUT) {
            return Err(EngineError::msg(format!(
                "the last of its steps names no `{{{OUT}}}`, so nothing says where its file goes"
            )));
        }
        Ok(Self { steps })
    }

    /// The steps a document holds under a media.
    pub fn of_steps(steps: &[Step]) -> Self {
        Self {
            steps: steps.to_vec(),
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

    /// Every file of the project it reads, as it names them: what a model is handed
    /// that no step made.
    pub fn reads(&self) -> Vec<&str> {
        let mut files: Vec<&str> = Vec::new();
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
            .map(|step| step.makes().to_owned())
            .filter(|name| name != OUT)
            .collect();
        made.sort_unstable();
        made.dedup();
        made
    }

    /// The steps as roll `roll` of them is asked: each seed a `generate` step gives, that
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

    /// Steps' text with no comments and no spare space, which is what a take is held
    /// to: how it was typed is not the steps.
    pub fn essence(text: &str) -> String {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }
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

/// Where the file `file` is, named as the steps that make media `id` name their files,
/// from the project's folder: its place on this machine, and its path as a document
/// writes it. A path that leaves the project is refused, and so is a media nothing makes.
pub fn beside(
    project: &kerf_model::Project,
    directory: &Path,
    id: &str,
    file: &str,
) -> Result<(PathBuf, String)> {
    written(project, id)?;
    let inside = within(Path::new(""), file, "file its steps name")?;
    Ok((directory.join(&inside), stored(&inside)))
}

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

/// The steps that make a media, read: their text, and what the text says, as its models
/// are asked.
pub(crate) struct Read {
    pub text: String,
    pub recipe: Recipe,
}

/// Read the steps that make media `id` of `project`.
pub(crate) fn read(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<Read> {
    let text = written(project, id)?;
    let recipe = Recipe::parse(&text)?.told(directory)?;
    Ok(Read { text, recipe })
}

/// What making media `id` of `project`, kept in `directory`, would run:
/// its steps read, and nothing run. Steps that could not run are
/// refused here, with why: a model that is not set up, a sequence the
/// project has not, a file they need that is not there.
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
    // Each path a plan holds is whole: a step runs in the project's folder, where a path
    // from wherever this was asked does not lead.
    let directory = &rooted(directory)?;
    let Read { text, recipe } = read(project, directory, id)?;
    let recipe = recipe.rolled(roll);
    let media = project.media(id).expect("read found it");
    for sequence in recipe.sequences() {
        if project.sequence(sequence).is_none() {
            return Err(EngineError::msg(format!(
                "the steps render sequence `{sequence}`, which this project has not"
            )));
        }
    }
    let made = recipe.makes();
    for file in recipe.reads() {
        let inside = within(Path::new(""), file, "file the making needs")?;
        if !directory.join(&inside).is_file() {
            // A file that is there, named so that it is not found.
            let why = if made.iter().any(|name| name == file) {
                format!(
                    ": `{file}` is a file a step makes, which a later step names in braces, `{{{file}}}`"
                )
            } else if stored(&inside) != file {
                format!(": it names `{file}`, and a file is named from the project's folder")
            } else {
                String::new()
            };
            return Err(EngineError::msg(format!(
                "the steps need `{}`, which is not there{why}",
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
    let place = takes::Place::of_media(directory, &media.path)?;
    let take = place.next();
    let working = place.working(take);
    let steps = recipe
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| match step {
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
                let request = request_file(&working, &recipe.steps, index);
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
        take,
        file: place.file(take),
        stored: place.stored(take),
        command: steps.join("\n"),
        steps,
        notes: notes(project, &recipe, models),
        roll,
        media: id.to_owned(),
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
        if clips().any(kerf_model::Clip::in_depth) {
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
    let recipe = Recipe::parse(&plan.text)?.told(directory)?.rolled(plan.roll);
    let place = takes::Place::of_media(directory, &media.path)?;
    // What the take is made from, read before anything runs.
    let from = takes::from(project, directory, &plan.text, &recipe)?;
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
                    "the steps ended well and wrote nothing to {}",
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
            within(Path::new(""), named, "file")
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
    /// The project's folder, which a step runs in and names its files from.
    directory: &'a Path,
    place: &'a takes::Place,
    take: u32,
    models: &'a Models,
    cancellation: &'a CancellationToken,
}

impl Making<'_> {
    /// Run `command` in the project's folder to its end; one that does not
    /// end well is an error with what it said.
    fn supervised(
        &self,
        mut command: Command,
        program: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<()> {
        command.current_dir(self.directory);
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
        for (index, step) in recipe.steps.iter().enumerate() {
            match step {
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
                    let request =
                        request_file(&self.place.working(self.take), &recipe.steps, index);
                    if let Some(folder) = request.parent() {
                        std::fs::create_dir_all(folder)?;
                    }
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
                            within(Path::new(""), file, "file the making needs")
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

/// Where the request of step `index` of `steps` is written, in a take's `working` folder,
/// with what its model's program keeps beside it: the guide and the graph as they went up.
/// A take one model makes has them in the folder itself. One that several make, a still and
/// then the shot of it, a matte and then a plate, the parts of a longer shot, has each
/// step's in a folder of the step's number (`step2/request.json`), where they were all
/// written to the one name and only the last step's were left to read.
fn request_file(working: &Path, steps: &[Step], index: usize) -> PathBuf {
    let asked = steps
        .iter()
        .filter(|step| matches!(step, Step::Generate(_)))
        .count();
    if asked > 1 {
        working
            .join(format!("step{}", index + 1))
            .join(model::REQUEST_FILE)
    } else {
        working.join(model::REQUEST_FILE)
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
