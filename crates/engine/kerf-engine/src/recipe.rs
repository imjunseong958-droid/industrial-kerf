//! A recipe: the text file that says how what is outside Kerf, a renderer
//! or a model, makes a media's file, and the making of it. A media line
//! names its recipe (`recipe="shots/open.kerfrecipe"`); `kerf make` runs it.
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
//!   reaches a model.
//! - `generate` asks a model ([`model`]) for the file: what it should show
//!   (`prompt=`), what guides it (`guide=`, `first=`, `last=`, `ref=`), and
//!   for how long and at what size (`seconds=`, `size=`), with a `seed=`.
//!
//! `{out}` stands for the media's file, and `{name}` for a file a step
//! makes on the way and a later one reads. `needs` names the files a recipe
//! reads, so that a change to one is seen ([`takes`]).
//!
//! Each making is a take, kept beside the ones before it; the document
//! says which one the media is. A recipe is a command someone wrote. Kerf
//! runs one only when a person asks for that media to be made, never on
//! opening or drawing a project.

pub mod model;
pub mod takes;

use crate::error::{EngineError, Result};
use crate::render::{render_program_with_cancellation, RenderOptions};
use crate::sidecar::{self, CancellationToken};
use model::{Ask, Models, Source};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub use takes::{take, takes, unmade, Take, Unmade};

/// The first line of a recipe.
const HEADER: &str = "kerf-recipe 1";

/// What stands for the media's file, in an argument.
const OUT: &str = "out";

/// One thing a recipe does on the way to the media's file.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A program and its arguments.
    Run {
        program: String,
        arguments: Vec<String>,
    },
    /// A sequence of the project, drawn by Kerf into the file `to` names.
    Render { sequence: String, to: String },
    /// A model asked for a file.
    Generate(Box<Ask>),
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
            let mut words = words(line).map_err(&wrong)?;
            let verb = words.remove(0);
            if verb != "needs" && recipe.writes_out() {
                return Err(wrong(format!(
                    "the step before it writes `{{{OUT}}}`, the media's file, so it is the last"
                )));
            }
            match verb.as_str() {
                "needs" if !words.is_empty() => recipe.needs.extend(words),
                "run" if !words.is_empty() => {
                    let program = words.remove(0);
                    made.extend(words.iter().flat_map(|word| stands_for(word)));
                    recipe.steps.push(Step::Run {
                        program,
                        arguments: words,
                    });
                }
                "render" if words.len() == 2 => {
                    let to = whole(&words[1]).ok_or_else(|| {
                        wrong(format!(
                            "`render <sequence> {{name}}` names the file it makes, such as `{{guide.mp4}}`, not `{}`",
                            words[1]
                        ))
                    })?;
                    made.push(to.clone());
                    recipe.steps.push(Step::Render {
                        sequence: words.swap_remove(0),
                        to,
                    });
                }
                "generate" if !words.is_empty() => {
                    let ask = Ask::parse(&words, &made).map_err(&wrong)?;
                    made.push(ask.out.clone());
                    recipe.steps.push(Step::Generate(Box::new(ask)));
                }
                "needs" | "run" | "render" | "generate" => {
                    return Err(wrong(format!(
                        "`{verb}` is `needs <file> …`, `run <program> <arguments>`, `render <sequence> {{name}}`, or `generate model=<name> prompt=\"…\" …`"
                    )))
                }
                other => {
                    return Err(wrong(format!(
                        "`{other}` is not a recipe's word; it has `needs`, `run`, `render`, and `generate`"
                    )))
                }
            }
        }
        if recipe.steps.is_empty() {
            return Err(EngineError::msg(
                "the recipe has no step: a `run`, a `render`, or a `generate` line",
            ));
        }
        if !recipe.writes_out() {
            return Err(EngineError::msg(format!(
                "the recipe's last step names no `{{{OUT}}}`, so nothing says where its file goes"
            )));
        }
        Ok(recipe)
    }

    /// Whether the last step so far writes the media's file.
    fn writes_out(&self) -> bool {
        match self.steps.last() {
            Some(Step::Run { arguments, .. }) => arguments
                .iter()
                .any(|argument| stands_for(argument).any(|name| name == OUT)),
            Some(Step::Render { to, .. }) => to == OUT,
            Some(Step::Generate(ask)) => ask.out == OUT,
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

/// The names in braces an argument holds: `out` and `guide.mp4` of
/// `--out={out}` and `{guide.mp4}`. Braces around anything but a file's
/// name, such as a line of JSON, stand for themselves.
fn stands_for(argument: &str) -> impl Iterator<Item = String> + '_ {
    argument.split('{').skip(1).filter_map(|rest| {
        let name = rest.split_once('}')?.0;
        file_name(name).then(|| name.to_owned())
    })
}

/// The name an argument that is nothing but `{name}` holds.
pub(crate) fn whole(argument: &str) -> Option<String> {
    let name = argument.strip_prefix('{')?.strip_suffix('}')?;
    file_name(name).then(|| name.to_owned())
}

/// Whether `name` can be a file a step makes: letters, digits, and `. - _`.
fn file_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
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
    /// The recipe's file.
    pub recipe: PathBuf,
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
    /// The media made, and its recipe as the document names it.
    media: String,
    name: String,
    /// The recipe as it was read for this plan, which is what runs.
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

/// A media's recipe, read: its name in the document, its text, and what
/// the text says.
pub(crate) struct Read {
    pub name: String,
    pub text: String,
    pub recipe: Recipe,
}

/// Read the recipe media `id` of `project` names.
pub(crate) fn read(project: &kerf_model::Project, directory: &Path, id: &str) -> Result<Read> {
    let media = project
        .media(id)
        .ok_or_else(|| EngineError::msg(format!("no media `{id}` in this project")))?;
    let name = media.recipe.as_deref().ok_or_else(|| {
        EngineError::msg(format!(
            "media `{id}` names no recipe; add `recipe=\"<file>\"` to its line with `media replace`"
        ))
    })?;
    let file = inside_project(directory, name, "recipe")?;
    let text = std::fs::read_to_string(&file).map_err(|error| {
        EngineError::msg(format!(
            "cannot read the recipe {}: {error}",
            file.display()
        ))
    })?;
    let recipe = Recipe::parse(&text)?;
    Ok(Read {
        name: name.to_owned(),
        text,
        recipe,
    })
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
    let Read { name, text, recipe } = read(project, directory, id)?;
    let media = project.media(id).expect("read found it");
    let folder = folder_of(&name);
    for sequence in recipe.sequences() {
        if project.sequence(sequence).is_none() {
            return Err(EngineError::msg(format!(
                "the recipe renders sequence `{sequence}`, which this project has not"
            )));
        }
    }
    for file in recipe.reads() {
        let inside = within(&folder, file, "file the recipe needs")?;
        if !directory.join(&inside).is_file() {
            return Err(EngineError::msg(format!(
                "the recipe needs `{}`, which is not there",
                stored(&inside)
            )));
        }
    }
    for model in recipe.models() {
        models.command(model)?;
    }
    let place = takes::Place::of(directory, &name, &media.path)?;
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
            Step::Render { sequence, to } => format!(
                "Kerf renders sequence {sequence} to {}",
                place.file_for(take, to).display()
            ),
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
        recipe: directory.join(&name),
        take,
        file: place.file(take),
        stored: place.stored(take),
        command: steps.join("\n"),
        steps,
        media: id.to_owned(),
        name,
        text,
    })
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
pub fn make(
    project: &kerf_model::Project,
    directory: &Path,
    plan: &Plan,
    models: &Models,
    cancellation: &CancellationToken,
    mut on_line: impl FnMut(&str),
) -> Result<Made> {
    let recipe = Recipe::parse(&plan.text)?;
    let media = project
        .media(&plan.media)
        .ok_or_else(|| EngineError::msg(format!("no media `{}` in this project", plan.media)))?;
    let folder = folder_of(&plan.name);
    let place = takes::Place::of(directory, &plan.name, &media.path)?;
    // What the take is made from, read before anything runs.
    let from = takes::from(project, directory, &folder, &plan.text, &recipe)?;
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
    let ran =
        making_it
            .run(&recipe, &mut on_line)
            .and_then(|()| match std::fs::metadata(&making) {
                Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Ok(()),
                _ => Err(EngineError::msg(format!(
                    "the recipe ended well and wrote nothing to {}",
                    making.display()
                ))),
            });
    if let Err(error) = ran {
        let _ = std::fs::remove_file(&making);
        let _ = std::fs::remove_dir_all(&working);
        return Err(error);
    }
    std::fs::rename(&making, place.file(take))?;
    takes::record(&place, take, from)?;
    Ok(Made {
        take,
        stored: place.stored(take),
    })
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
                Step::Render { sequence, to } => {
                    let mut resolved = self.project.clone();
                    resolved.resolve_paths(kerf_model::Catalog::builtin(), self.directory);
                    render_program_with_cancellation(
                        &resolved,
                        Some(sequence),
                        None,
                        &RenderOptions {
                            output: self.file_for(to).to_string_lossy().into_owned(),
                            project_dir: Some(self.directory.to_path_buf()),
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
                    let asked = ask.request(
                        &|file| {
                            within(self.folder, file, "file the recipe needs")
                                .map(|inside| self.directory.join(inside))
                        },
                        &|name| self.file_for(name),
                    )?;
                    std::fs::write(&request, serde_json::to_vec_pretty(&asked)?)?;
                    let words = self.models.command(&ask.model)?;
                    let mut command = Command::new(&words[0]);
                    command.args(
                        words[1..]
                            .iter()
                            .map(|word| word.replace(model::REQUEST, &request.to_string_lossy())),
                    );
                    self.supervised(command, &words[0], on_line)?;
                }
            }
        }
        Ok(())
    }
}

/// A line's words: split at spaces, with `"…"` keeping what it holds as
/// one word, `\"` and `\\` inside it standing for themselves.
fn words(line: &str) -> std::result::Result<Vec<String>, String> {
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
