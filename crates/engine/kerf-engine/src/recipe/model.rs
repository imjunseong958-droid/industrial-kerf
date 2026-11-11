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
//!   "more": { "strength": "0.6" },
//!   "out": "/project/shots/open.takes/.making-003.mp4"
//! }
//! ```
//!
//! The program runs as a recipe's `run` does, supervised and never through
//! a shell, writes what it has to say to stderr, and ends well once the
//! file at `out` is written. A project cannot name a model's program: that
//! file is the person's, outside any project.

use crate::error::{EngineError, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What stands for the request's file in a model's command.
pub const REQUEST: &str = "{request}";

/// The request's file, among what a take's steps made.
pub const REQUEST_FILE: &str = "request.json";

/// The version a request says it is.
const REQUEST_VERSION: u32 = 1;

/// A file a model is handed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file of the project, as the recipe names it: from its own folder.
    File(String),
    /// A file a step before made, by the name it has in braces.
    Made(String),
}

impl Source {
    fn shown(&self) -> String {
        match self {
            Self::File(file) => file.clone(),
            Self::Made(name) => format!("{{{name}}}"),
        }
    }
}

/// What a `generate` line asks of a model.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    /// The model, by the name the person's models file gives it.
    pub model: String,
    /// What the file should show, in words.
    pub prompt: String,
    /// A picture in motion to follow: a blocking, a rough render, a take.
    pub guide: Option<Source>,
    /// The first and the last frame.
    pub first: Option<Source>,
    pub last: Option<Source>,
    /// Pictures of who and what is in it, and of how it should look.
    pub references: Vec<Source>,
    pub seconds: Option<f64>,
    pub size: Option<(u32, u32)>,
    pub seed: Option<u64>,
    /// Anything else the line says, which only that model reads.
    pub more: BTreeMap<String, String>,
    /// The file it writes: `out`, or the name of one a later step reads.
    pub out: String,
}

impl Ask {
    /// Read the words after `generate`. `made` holds the files the steps
    /// before it make.
    pub(crate) fn parse(words: &[String], made: &[String]) -> std::result::Result<Self, String> {
        let mut ask = Self {
            model: String::new(),
            prompt: String::new(),
            guide: None,
            first: None,
            last: None,
            references: Vec::new(),
            seconds: None,
            size: None,
            seed: None,
            more: BTreeMap::new(),
            out: "out".to_owned(),
        };
        let source = |value: &str| match super::whole(value) {
            Some(name) if made.contains(&name) => Ok(Source::Made(name)),
            Some(name) => Err(format!("no step before this one makes `{{{name}}}`")),
            None if value.is_empty() => Err("a file's name is empty".to_owned()),
            None => Ok(Source::File(value.to_owned())),
        };
        for word in words {
            let (key, value) = word
                .split_once('=')
                .ok_or_else(|| format!("`{word}` is not `<name>=<value>`"))?;
            match key {
                "model" => ask.model = value.to_owned(),
                "prompt" => ask.prompt = value.to_owned(),
                "guide" => ask.guide = Some(source(value)?),
                "first" => ask.first = Some(source(value)?),
                "last" => ask.last = Some(source(value)?),
                "ref" => ask.references.push(source(value)?),
                "seconds" => {
                    ask.seconds = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|seconds: &f64| seconds.is_finite() && *seconds > 0.0)
                            .ok_or_else(|| {
                                format!("`seconds={value}` is not a length in seconds")
                            })?,
                    );
                }
                "size" => {
                    ask.size = Some(
                        value
                            .split_once('x')
                            .and_then(|(width, height)| {
                                Some((width.parse().ok()?, height.parse().ok()?))
                            })
                            .filter(|(width, height): &(u32, u32)| *width > 0 && *height > 0)
                            .ok_or_else(|| {
                                format!("`size={value}` is not a size such as 1920x1080")
                            })?,
                    );
                }
                "seed" => {
                    ask.seed = Some(
                        value
                            .parse()
                            .map_err(|_| format!("`seed={value}` is not a whole number"))?,
                    );
                }
                "out" => {
                    ask.out = super::whole(value).ok_or_else(|| {
                        format!("`out={value}` names the file it makes in braces, such as `{{first.png}}`")
                    })?;
                }
                "" => return Err(format!("`{word}` has no name before its `=`")),
                other => {
                    ask.more.insert(other.to_owned(), value.to_owned());
                }
            }
        }
        if ask.model.is_empty() {
            return Err("`generate` names its model: `model=<name>`".to_owned());
        }
        Ok(ask)
    }

    /// Every file it hands the model.
    pub fn sources(&self) -> impl Iterator<Item = &Source> {
        self.guide
            .iter()
            .chain(&self.first)
            .chain(&self.last)
            .chain(&self.references)
    }

    /// What it asks, as a person reads it.
    pub fn shown(&self) -> String {
        let mut parts = vec![format!("prompt \"{}\"", self.prompt)];
        let mut named =
            |name: &str, source: &Source| parts.push(format!("{name} {}", source.shown()));
        if let Some(guide) = &self.guide {
            named("guide", guide);
        }
        if let Some(first) = &self.first {
            named("first", first);
        }
        if let Some(last) = &self.last {
            named("last", last);
        }
        for reference in &self.references {
            named("ref", reference);
        }
        if let Some(seconds) = self.seconds {
            parts.push(format!("{seconds} s"));
        }
        if let Some((width, height)) = self.size {
            parts.push(format!("{width}x{height}"));
        }
        if let Some(seed) = self.seed {
            parts.push(format!("seed {seed}"));
        }
        parts.extend(
            self.more
                .iter()
                .map(|(name, value)| format!("{name} {value}")),
        );
        parts.join(", ")
    }

    /// The request a model's program reads, with every file where it is:
    /// `file` says where one of the project is, and `made` where one a step
    /// made is, the one this writes among them.
    pub(crate) fn request(
        &self,
        file: &dyn Fn(&str) -> Result<PathBuf>,
        made: &dyn Fn(&str) -> PathBuf,
    ) -> Result<Value> {
        let place = |source: &Source| -> Result<Value> {
            Ok(Value::String(
                match source {
                    Source::File(named) => file(named)?,
                    Source::Made(name) => made(name),
                }
                .to_string_lossy()
                .into_owned(),
            ))
        };
        let optional = |source: &Option<Source>| source.as_ref().map(place).transpose();
        Ok(json!({
            "kerf-request": REQUEST_VERSION,
            "model": self.model,
            "prompt": self.prompt,
            "guide": optional(&self.guide)?,
            "first": optional(&self.first)?,
            "last": optional(&self.last)?,
            "references": self.references.iter().map(place).collect::<Result<Vec<_>>>()?,
            "seconds": self.seconds,
            "size": self.size.map(|(width, height)| [width, height]),
            "seed": self.seed,
            "more": self.more,
            "out": made(&self.out).to_string_lossy(),
        }))
    }
}

/// The models a person has set up, each a program that is handed a request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Models {
    /// The file that names them, there or not.
    file: PathBuf,
    commands: BTreeMap<String, Vec<String>>,
}

impl Models {
    /// The person's models: the file `KERF_MODELS` names, or
    /// `~/.kerf/models.json`. No file is no models.
    pub fn load() -> Result<Self> {
        let file = std::env::var_os("KERF_MODELS")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                // Windows sets no HOME.
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .filter(|home| !home.is_empty())
                    .map(|home| PathBuf::from(home).join(".kerf").join("models.json"))
            })
            .unwrap_or_else(|| PathBuf::from("models.json"));
        Self::read(&file)
    }

    /// The models `file` names. What is wrong in it is said with the model.
    pub fn read(file: &Path) -> Result<Self> {
        let none = Self {
            file: file.to_path_buf(),
            commands: BTreeMap::new(),
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
        for (name, model) in named {
            let command: Vec<String> = model
                .get("run")
                .and_then(|run| serde_json::from_value(run.clone()).ok())
                .filter(|run: &Vec<String>| !run.is_empty())
                .ok_or_else(|| {
                    wrong(format!(
                        "model `{name}` has no `run`, a program and its arguments as a list"
                    ))
                })?;
            if !command.iter().any(|word| word.contains(REQUEST)) {
                return Err(wrong(format!(
                    "model `{name}`'s `run` names no `{REQUEST}`, so its program is never told what is asked"
                )));
            }
            commands.insert(name, command);
        }
        Ok(Self { commands, ..none })
    }

    /// The names of the models set up.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(String::as_str)
    }

    /// The program and arguments that reach model `name`.
    pub fn command(&self, name: &str) -> Result<&[String]> {
        self.commands.get(name).map(Vec::as_slice).ok_or_else(|| {
            let have = if self.commands.is_empty() {
                "names none".to_owned()
            } else {
                format!("names {}", self.names().collect::<Vec<_>>().join(", "))
            };
            EngineError::msg(format!(
                "no model `{name}` is set up: {} {have}. A model is a program named there, as `{{\"{name}\": {{\"run\": [\"<program>\", \"{REQUEST}\"]}}}}` (docs/shots.md)",
                self.file.display()
            ))
        })
    }
}
