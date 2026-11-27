//! A model a recipe asks for a file, and how it is reached.
//!
//! Kerf has no model of its own and knows none by name. A recipe's
//! `generate` line says what is wanted in words any model takes: a prompt,
//! what guides it, how long, what size, a seed. Kerf writes that as a
//! request, a JSON file, and hands it to the program the person's own
//! `~/.kerf/models.json` names for that model, which asks the model however
//! that model is asked and writes the file:
//!
//! ```json
//! { "dawn": { "run": ["dawn-adapter", "--request", "{request}"] } }
//! ```
//!
//! ```json
//! {
//!   "kerf-request": 1,
//!   "model": "dawn",
//!   "prompt": "a boat leaves the harbour at dawn",
//!   "guide": "/project/shots/open.takes/003.work/guide.mp4",
//!   "first": "/project/refs/boat.png",
//!   "last": null,
//!   "references": [],
//!   "seconds": 4.0,
//!   "size": [1920, 1080],
//!   "seed": 7,
//!   "signals": [{ "name": "swell", "rate": 24.0, "values": [0.0, 0.25, 0.5] }],
//!   "workflow": null,
//!   "more": { "strength": "0.6" },
//!   "out": "/project/shots/open.takes/.making-003.mp4"
//! }
//! ```
//!
//! `signals` holds each signal the line names ([`super::signal`]): a number
//! for each frame of the clip's sequence, at that sequence's rate.
//!
//! For a model whose `reads` has `regions`, `regions` holds what the clips of
//! the guide Kerf draws say they are ([`super::regions`]): each prompt once,
//! with a grey video drawn as the guide is, white where the guide shows the
//! clips that say it: `[{"prompt": "a red car", "mask": "…/region1.mp4"}]`.
//!
//! The program runs as a recipe's `run` does, supervised and never through
//! a shell, writes what it has to say to stderr, and ends well once the
//! file at `out` is written. A project cannot name a model's program: that
//! file is the person's, outside any project.
//!
//! A program that was not written for Kerf, a renderer or a script, is named
//! the same way and handed its arguments in place of a request
//! ([`command_for`]): `{"previs": {"run": ["vork", "render", "{ref}",
//! "--out", "{out}"]}}`, asked by `generate model=previs
//! ref=scenes/open.scene.json out={guide.mp4}`. So what runs is always a
//! program the person named, and a document only says which and with what.
//!
//! Beside `run`, a model's entry can say what the model takes ([`Takes`]):
//! the picture it makes, its rate, the longest shot, the files it reads.
//! Kerf asks no program for it, so whoever writes a recipe, an agent
//! included, learns it without anything being run:
//!
//! ```json
//! { "dawn": { "run": ["dawn-adapter", "--request", "{request}"],
//!             "takes": { "size": [832, 480], "rate": 16, "seconds": 5,
//!                        "reads": ["guide", "ref"] } } }
//! ```

use super::{moves::Moves, signal::Signal};
use crate::error::{EngineError, Result};
use crate::said;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{PoisonError, RwLock};

/// The models file this process reads in place of the one the environment
/// names, set by [`Models::use_file`].
static FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// What this process gives the programs models run over its own environment:
/// variables set (`Some`) or taken away (`None`), by [`set_program_env`] and
/// [`remove_program_env`].
static PROGRAM_ENV: RwLock<Vec<(OsString, Option<OsString>)>> = RwLock::new(Vec::new());

/// Run the programs models run from here on with `name` set to `value`,
/// leaving this process's own environment as it is: for a host that hands
/// them a setting of its own, as a test does.
pub fn set_program_env(name: impl Into<OsString>, value: impl Into<OsString>) {
    give_programs(name.into(), Some(value.into()));
}

/// Run the programs models run from here on without `name`, whatever this
/// process's environment has: for a host that keeps a person's own setting
/// from them, as a test does.
pub fn remove_program_env(name: impl Into<OsString>) {
    give_programs(name.into(), None);
}

fn give_programs(name: OsString, value: Option<OsString>) {
    let mut given = PROGRAM_ENV.write().unwrap_or_else(PoisonError::into_inner);
    given.retain(|(known, _)| *known != name);
    given.push((name, value));
}

/// A command for `program`, which a model runs, with what
/// [`set_program_env`] and [`remove_program_env`] give it.
pub fn program(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    for (name, value) in PROGRAM_ENV
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
    {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command
}

/// What stands for the request's file in a model's command.
pub const REQUEST: &str = "{request}";

/// What stands for where the file goes in the command of a program that is handed its
/// arguments and no request: a renderer, or any program that was not written for Kerf.
pub const OUT: &str = "{out}";

/// The request's file, among what a take's steps made.
pub const REQUEST_FILE: &str = "request.json";

/// The version a request says it is.
const REQUEST_VERSION: u32 = 1;

pub use kerf_model::making::{names_a_guide, Ask, Source, FILES, MOVES, REGIONS, SIGNAL, WORKFLOW};

/// The request a model's program reads, as [`REQUEST_FILE`] holds it: what a `generate` step
/// asks, with every file it names where that file is.
#[derive(Debug, Serialize)]
pub(crate) struct Request<'a> {
    #[serde(rename = "kerf-request")]
    version: u32,
    model: &'a str,
    prompt: &'a str,
    guide: Option<String>,
    /// The other guides, by what each is of, in the order the step names them.
    #[serde(serialize_with = "in_order")]
    guides: Vec<(String, String)>,
    /// The first and the last frame, which a picture in motion given as one is made a still
    /// for.
    pub(crate) first: Option<String>,
    pub(crate) last: Option<String>,
    references: Vec<String>,
    seconds: Option<f64>,
    size: Option<[u32; 2]>,
    seed: Option<u64>,
    signals: &'a [Signal],
    moves: &'a [Moves],
    /// What clips of the guide say they are, each with where the guide shows them: said only
    /// where some do, so a request of none reads as it did.
    #[serde(skip_serializing_if = "<[Region]>::is_empty")]
    regions: &'a [Region],
    /// Under [`WORKFLOW`], the name a step hands a model a graph of its own by.
    workflow: Option<String>,
    more: &'a BTreeMap<String, String>,
    out: String,
    /// Said only of a part of a longer shot: a request for a whole one reads as it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<f64>,
}

// serde names a field by a literal: `workflow` is held to the name kerf-model gives it.
const _: () = assert!(matches!(WORKFLOW.as_bytes(), b"workflow"));
// And so is `regions`.
const _: () = assert!(matches!(REGIONS.as_bytes(), b"regions"));

/// Words clips of a guide say they are (`prompt=`), and a grey video of where the guide shows
/// those clips, frame by frame: white where they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Region {
    pub prompt: String,
    pub mask: String,
}

/// Names and their files as an object's entries, in the order they are said. As
/// [`Request::file`] writes them, into a JSON object, a name said twice is there once, with
/// the file it was last given.
fn in_order<S: serde::Serializer>(
    guides: &[(String, String)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_map(guides.iter().map(|(name, file)| (name, file)))
}

impl Request<'_> {
    /// The file of the guide of what `name` is, the last one the step gives it.
    fn guide_of(&self, name: &str) -> Option<&str> {
        self.guides
            .iter()
            .rev()
            .find(|(said, _)| said == name)
            .map(|(_, file)| file.as_str())
    }

    /// The request as its file holds it. Made a JSON object first, so that its keys are in
    /// the order every object this build writes is: sorted, or, where serde_json keeps order
    /// (the editor's build), the order of the fields above.
    pub(crate) fn file(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(&serde_json::to_value(self)?)?)
    }
}

/// The request a model's program reads of what `ask` asks, with every file where it is:
/// `file` says where one of the project is, and `made` where one a step
/// made is, the one this writes among them. `signals` and `moves` are the ones it
/// names, read from the project, and `regions` what clips of its guide say they are.
pub(crate) fn request<'a>(
    ask: &'a Ask,
    file: &dyn Fn(&str) -> Result<PathBuf>,
    made: &dyn Fn(&str) -> PathBuf,
    signals: &'a [Signal],
    moves: &'a [Moves],
    regions: &'a [Region],
) -> Result<Request<'a>> {
    let place = |source: &Source| -> Result<String> {
        Ok(match source {
            Source::File(named) => file(named)?,
            Source::Made(name) => made(name),
        }
        .to_string_lossy()
        .into_owned())
    };
    let optional = |source: &Option<Source>| source.as_ref().map(place).transpose();
    Ok(Request {
        version: REQUEST_VERSION,
        model: &ask.model,
        prompt: &ask.prompt,
        guide: optional(&ask.guide)?,
        guides: ask
            .guides
            .iter()
            .map(|(name, source)| Ok((name.clone(), place(source)?)))
            .collect::<Result<_>>()?,
        first: optional(&ask.first)?,
        last: optional(&ask.last)?,
        references: ask.references.iter().map(place).collect::<Result<_>>()?,
        seconds: ask.seconds,
        size: ask.size.map(|(width, height)| [width, height]),
        seed: ask.seed,
        signals,
        moves,
        regions,
        workflow: optional(&ask.workflow)?,
        more: &ask.more,
        out: made(&ask.out).to_string_lossy().into_owned(),
        from: ask.from,
    })
}

/// A number as a blank is filled with it: a whole one without a point.
fn number_text(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(whole) if whole.fract() == 0.0 && whole.abs() < 1e15 => format!("{whole:.0}"),
        _ => number.to_string(),
    }
}

/// The command that asks a model: `words`, as the person's models file gives its `run`,
/// with each blank filled from `asked`, the request a step comes to, whose file is
/// `request`.
///
/// A program written for Kerf reads the request, and its command names it (`{request}`).
/// One that was not, a renderer, a script, is handed what it needs as its arguments, by
/// the names a step gives them: `{out}`, where its file goes; `{prompt}`; the files,
/// `{guide}`, `{first}`, `{last}` and `{ref}` (`{ref2}` for the second); `{seconds}`,
/// `{seed}`, `{size}`, `{width}` and `{height}`; and any other name the step sets
/// (`profile=std/previs` fills `{profile}`), or the value the entry's `takes` gives it
/// under `more` where the step does not. A blank with nothing to fill it is refused, with
/// the word that fills it.
///
/// The program and the shape of its command are the person's own, and a project only
/// fills the blanks: an argument that is a blank alone is refused where what fills it
/// would read as an option (`-…`), so a step cannot hand the program a switch its
/// owner did not write.
pub(crate) fn command_for(
    model: &str,
    words: &[String],
    request: &Path,
    asked: &Request,
    takes: Option<&Takes>,
) -> Result<Vec<String>> {
    // What the person's file gives a name under `more`, as they wrote it.
    let text = |value: &Value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number_text(number)),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    };
    let filled = |name: &str| -> Option<String> {
        let size = |at: usize| asked.size.map(|size| size[at]);
        match name {
            "request" => Some(request.to_string_lossy().into_owned()),
            "out" => Some(asked.out.clone()),
            "prompt" => Some(asked.prompt.to_owned()),
            "guide" => asked.guide.clone(),
            "first" => asked.first.clone(),
            "last" => asked.last.clone(),
            "seconds" => asked
                .seconds
                .and_then(serde_json::Number::from_f64)
                .map(|seconds| number_text(&seconds)),
            "seed" => asked.seed.map(|seed| number_text(&seed.into())),
            "size" => Some(format!("{}x{}", size(0)?, size(1)?)),
            "width" => size(0).map(|side| side.to_string()),
            "height" => size(1).map(|side| side.to_string()),
            other => {
                // `ref` is the first picture, `ref2` the second.
                if let Some(count) = other.strip_prefix("ref") {
                    let at = match count {
                        "" => Some(0),
                        number => number
                            .parse::<usize>()
                            .ok()
                            .filter(|at| *at >= 2)
                            .map(|at| at - 1),
                    };
                    if let Some(at) = at {
                        return asked.references.get(at).cloned();
                    }
                }
                if let Some(guide) = other.strip_prefix("guide_") {
                    return asked.guide_of(guide).map(str::to_owned);
                }
                asked
                    .more
                    .get(other)
                    .cloned()
                    .or_else(|| takes.and_then(|takes| takes.more.get(other)).and_then(text))
            }
        }
    };
    words
        .iter()
        .map(|word| {
            let mut out = String::new();
            let mut rest = word.as_str();
            while let Some(open) = rest.find('{') {
                out.push_str(&rest[..open]);
                let after = &rest[open + 1..];
                let named = after
                    .split_once('}')
                    .filter(|(name, _)| kerf_model::making::file_name(name));
                match named {
                    Some((name, tail)) => {
                        let value = filled(name).ok_or_else(|| {
                            EngineError::msg(format!(
                                "model `{model}`'s program is handed `{{{name}}}`, and the step gives none: add `{}` to its step",
                                match name {
                                    "out" => "out={<name>}".to_owned(),
                                    "width" | "height" => "size=<W>x<H>".to_owned(),
                                    numbered if numbered.starts_with("ref") => "ref=<file>".to_owned(),
                                    other => format!("{other}=<value>"),
                                }
                            ))
                        })?;
                        if word.len() == name.len() + 2 && value.starts_with('-') {
                            return Err(EngineError::msg(format!(
                                "`{name}={value}` would read as an option of model `{model}`'s program, which its owner did not write: a value does not start with `-`"
                            )));
                        }
                        out.push_str(&value);
                        rest = tail;
                    }
                    None => {
                        out.push('{');
                        rest = after;
                    }
                }
            }
            out.push_str(rest);
            Ok(out)
        })
        .collect()
}

/// What a model takes, as the person's models file says it under `takes`.
/// Each part is there only where the file says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Takes {
    /// What it makes, in a line.
    pub about: Option<String>,
    /// The picture it makes: the size a guide for it is drawn at, and the
    /// most a `size=` asks of it.
    pub size: Option<(u32, u32)>,
    /// The frames a second it makes.
    pub rate: Option<f64>,
    /// The longest shot it makes at once, in seconds.
    pub seconds: Option<f64>,
    /// What it has a use for of what a line hands it, the [`FILES`] and
    /// [`SIGNAL`]: none is a model that takes a prompt alone.
    pub reads: Option<Vec<String>>,
    /// The other names a `generate` line can set for it, each with the
    /// value it has when the line does not.
    pub more: serde_json::Map<String, Value>,
    /// Whatever else the file says of it, as it is written.
    pub other: serde_json::Map<String, Value>,
}

impl Takes {
    /// Read a model's `takes`. What is wrong is said by its name, in words a person who
    /// wrote the file follows, not serde's: so it is read part by part, as JSON.
    fn parse(value: &Value) -> std::result::Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or("`takes` is not an object, such as {\"size\": [832, 480], \"rate\": 16}")?;
        let mut takes = Self::default();
        let positive = |name: &str, value: &Value| {
            value
                .as_f64()
                .filter(|number| number.is_finite() && *number > 0.0)
                .ok_or_else(|| format!("`{name}` in `takes` is not a number above 0"))
        };
        for (name, value) in fields {
            match name.as_str() {
                "about" => {
                    takes.about = Some(
                        value
                            .as_str()
                            .ok_or("`about` in `takes` is not text")?
                            .to_owned(),
                    );
                }
                "size" => {
                    takes.size = Some(
                        serde_json::from_value::<(u32, u32)>(value.clone())
                            .ok()
                            .filter(|(width, height)| *width > 0 && *height > 0)
                            .ok_or(
                                "`size` in `takes` is not a width and a height, such as [832, 480]",
                            )?,
                    );
                }
                "rate" => takes.rate = Some(positive(name, value)?),
                "seconds" => takes.seconds = Some(positive(name, value)?),
                "reads" => {
                    let reads: Vec<String> = serde_json::from_value(value.clone())
                        .ok()
                        .filter(|reads: &Vec<String>| {
                            reads
                                .iter()
                                .all(|read| {
                                    FILES.contains(&read.as_str())
                                        || read == SIGNAL
                                        || read == MOVES
                                        || read == REGIONS
                                        || read.strip_prefix("guide_").is_some_and(names_a_guide)
                                })
                        })
                        .ok_or_else(|| {
                            format!(
                                "`reads` in `takes` is not a list of what a `generate` line hands a model: {}, {SIGNAL}, {MOVES}, {REGIONS}, and a guide by what it is of, as `guide_depth`",
                                FILES.join(", ")
                            )
                        })?;
                    takes.reads = Some(reads);
                }
                "more" => {
                    takes.more = value
                        .as_object()
                        .ok_or("`more` in `takes` is not an object of names and their values")?
                        .clone();
                }
                _ => {
                    takes.other.insert(name.clone(), value.clone());
                }
            }
        }
        Ok(takes)
    }

    /// Whether it follows a guide, where the file says what it reads.
    pub fn follows_a_guide(&self) -> Option<bool> {
        self.reads
            .as_ref()
            .map(|reads| reads.iter().any(|read| read == "guide"))
    }
}

/// A model's entry in the models file, read as [`said`] reads.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    /// Its program and arguments: none where they are not a list of words.
    #[serde(deserialize_with = "said::as_said")]
    run: Vec<String>,
    /// What it takes, said at all, `null` included. JSON, as [`Takes::parse`] reads it.
    #[serde(deserialize_with = "said::present")]
    takes: Option<Value>,
}

/// The models a person has set up, each a program that is handed a request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Models {
    /// The file that names them, there or not.
    file: PathBuf,
    commands: BTreeMap<String, Vec<String>>,
    /// What each takes, for those the file says it of.
    takes: BTreeMap<String, Takes>,
}

impl Models {
    /// The person's models: the file [`Models::default_file`] says. No file
    /// is no models.
    pub fn load() -> Result<Self> {
        Self::read(&Self::default_file())
    }

    /// Read the person's models from `file` from here on, in place of the
    /// file `KERF_MODELS` names or `~/.kerf/models.json`, leaving this
    /// process's environment as it is: for a host that keeps them apart, as
    /// a test does.
    pub fn use_file(file: impl Into<PathBuf>) {
        *FILE.write().unwrap_or_else(PoisonError::into_inner) = Some(file.into());
    }

    /// Where [`Models::load`] reads: the file [`Models::use_file`] named,
    /// else the file `KERF_MODELS` names, or `~/.kerf/models.json`.
    pub fn default_file() -> PathBuf {
        if let Some(file) = FILE.read().unwrap_or_else(PoisonError::into_inner).clone() {
            return file;
        }
        std::env::var_os("KERF_MODELS")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| kerf_paths::user_dir().map(|folder| folder.join("models.json")))
            .unwrap_or_else(|| PathBuf::from("models.json"))
    }

    /// The models `file` names. What is wrong in it is said with the model.
    pub fn read(file: &Path) -> Result<Self> {
        let none = Self {
            file: file.to_path_buf(),
            commands: BTreeMap::new(),
            takes: BTreeMap::new(),
        };
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(none),
            Err(error) => {
                return Err(EngineError::msg(format!(
                    "cannot read the models file {}: {error}",
                    file.display()
                )))
            }
        };
        let wrong =
            |what: String| EngineError::msg(format!("the models file {}: {what}", file.display()));
        let named: BTreeMap<String, Value> =
            serde_json::from_str(&text).map_err(|error| wrong(error.to_string()))?;
        let mut commands = BTreeMap::new();
        let mut takes = BTreeMap::new();
        for (name, model) in named {
            let model: Entry = said::object_of(model);
            let command = Some(model.run)
                .filter(|run| !run.is_empty())
                .ok_or_else(|| {
                    wrong(format!(
                        "model `{name}` has no `run`, a program and its arguments as a list"
                    ))
                })?;
            if !command
                .iter()
                .any(|word| word.contains(REQUEST) || word.contains(OUT))
            {
                return Err(wrong(format!(
                    "model `{name}`'s `run` names neither `{REQUEST}` nor `{OUT}`, so its program is never told what is asked or where its file goes"
                )));
            }
            if let Some(said) = &model.takes {
                let said =
                    Takes::parse(said).map_err(|what| wrong(format!("model `{name}`: {what}")))?;
                takes.insert(name.clone(), said);
            }
            commands.insert(name, command);
        }
        Ok(Self {
            commands,
            takes,
            ..none
        })
    }

    /// The file that names them, there or not.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// What model `name` takes, where the file says.
    pub fn takes(&self, name: &str) -> Option<&Takes> {
        self.takes.get(name)
    }

    /// The names of the models set up.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(String::as_str)
    }

    /// The program and arguments that reach model `name`.
    pub fn command(&self, name: &str) -> Result<&[String]> {
        self.commands
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| EngineError::NoModel {
                name: name.to_owned(),
                file: self.file.clone(),
                have: self.names().map(str::to_owned).collect(),
            })
    }
}
