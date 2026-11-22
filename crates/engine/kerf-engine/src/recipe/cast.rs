//! A project's cast: who and what its shots show, and how they look, said
//! once for every recipe.
//!
//! A model keeps a face, a boat, or a room from shot to shot only when each
//! shot is asked for with the same words and the same pictures. So the
//! project says them once, in `cast.kerfcast` in its own folder:
//!
//! ```text
//! kerf-cast 1
//! # Who and what the shots show.
//! cast Ada "an old keeper in a wool coat" ref=refs/ada-front.png ref=refs/ada-side.png
//! cast lamp "the lamp room of a lighthouse, brass and salt-stained glass" ref=refs/lamp.png
//! # How every shot looks.
//! style film "shot on 35mm colour negative, a cold blue-steel grade, blue-hour light"
//! ```
//!
//! and a `generate` line names what it shows: `cast=Ada cast=lamp style=film`.
//! The model is then asked with the line's prompt followed by each one's
//! words, and handed each one's pictures after the line's own `ref=`. A
//! picture is a path from the project's folder.
//!
//! A take is held to each one it names, by its words and its pictures
//! ([`mod@super::takes`]): changing how Ada is described asks again for the
//! shots Ada is in and no other, and adding someone to the file asks for
//! none.

use super::model::{Ask, Source};
use super::{within, words, Recipe, Step};
use crate::error::{EngineError, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// The file a project's cast is kept in, in the project's folder.
pub const FILE: &str = "cast.kerfcast";

/// The first line of a cast file.
const HEADER: &str = "kerf-cast 1";

/// What a cast file says of two kinds: who and what is in a shot, and how a
/// shot looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Cast,
    Style,
}

impl Kind {
    /// The word a cast file's line starts with and a `generate` line names
    /// one by.
    pub fn word(self) -> &'static str {
        match self {
            Self::Cast => "cast",
            Self::Style => "style",
        }
    }
}

/// One line of a cast file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// What a model is told of it.
    pub words: String,
    /// Pictures of it, each a path from the project's folder.
    pub references: Vec<String>,
}

impl Entry {
    /// A hash of what a model is told and handed of it. What its pictures
    /// hold is hashed as a file a recipe reads is.
    fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.words.as_bytes());
        for reference in &self.references {
            hasher.update([0]);
            hasher.update(reference.as_bytes());
        }
        hex::encode(hasher.finalize())
    }
}

/// A project's cast, as its file says it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cast {
    /// Each one, in the order the file says them.
    entries: Vec<(Kind, String, Entry)>,
    /// Whether the project has the file at all.
    there: bool,
}

/// A cast file as it was when it was read: where, how long, and when written.
type Read = (std::path::PathBuf, u64, Option<std::time::SystemTime>);

/// The cast last read, and the file it was read from.
fn known() -> &'static Mutex<Option<(Read, Cast)>> {
    static KNOWN: OnceLock<Mutex<Option<(Read, Cast)>>> = OnceLock::new();
    KNOWN.get_or_init(Mutex::default)
}

/// Read the next cast asked for from its file: one is about to be written.
fn forget() {
    if let Ok(mut known) = known().lock() {
        *known = None;
    }
}

/// Whether `name` can name a cast or a style: letters, digits, `-` and `_`.
pub(crate) use kerf_model::making::names_cast as names_one;

impl Cast {
    /// Read a cast file's text. What is wrong is said with its line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        match lines.next() {
            Some((_, HEADER)) => {}
            _ => return Err(EngineError::msg(format!("`{FILE}` starts with `{HEADER}`"))),
        }
        let mut cast = Self {
            entries: Vec::new(),
            there: true,
        };
        for (number, line) in lines {
            let wrong =
                |what: String| EngineError::msg(format!("line {number} of `{FILE}`: {what}"));
            let mut words = words(line).map_err(wrong)?.into_iter();
            let kind = match words.next().as_deref() {
                Some("cast") => Kind::Cast,
                Some("style") => Kind::Style,
                other => {
                    return Err(wrong(format!(
                        "`{}` is not a cast file's word; a line is `cast <name> \"<who or what it is>\" [ref=<picture> …]` or `style <name> \"<how a shot looks>\" [ref=<picture> …]`",
                        other.unwrap_or_default()
                    )))
                }
            };
            let name = words.next().filter(|name| names_one(name)).ok_or_else(|| {
                wrong(format!(
                    "`{}` is followed by a name of letters, digits, `-` and `_`, such as `{} Ada \"an old keeper\"`",
                    kind.word(),
                    kind.word()
                ))
            })?;
            let mut entry = Entry::default();
            for word in words {
                match word.strip_prefix("ref=") {
                    Some("") => return Err(wrong("`ref=` names no picture".to_owned())),
                    Some(picture) => {
                        // A picture is a file of the project, as every file a recipe reads is.
                        within(Path::new(""), picture, "picture")
                            .map_err(|error| wrong(error.to_string()))?;
                        if !entry.references.iter().any(|given| given == picture) {
                            entry.references.push(picture.to_owned());
                        }
                    }
                    None if entry.words.is_empty() && entry.references.is_empty() => {
                        entry.words = word.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                    None => {
                        return Err(wrong(format!(
                            "`{word}` comes after what `{name}` is: its words are one quoted text, and what follows is `ref=<picture>`"
                        )))
                    }
                }
            }
            if entry.words.is_empty() && entry.references.is_empty() {
                return Err(wrong(format!(
                    "`{name}` says nothing: give it words in quotes, a `ref=<picture>`, or both"
                )));
            }
            if cast.find(kind, &name).is_some() {
                return Err(wrong(format!("{} `{name}` is said twice", kind.word())));
            }
            cast.entries.push((kind, name, entry));
        }
        Ok(cast)
    }

    /// The cast of the project kept in `directory`. A project with no cast
    /// file has none.
    pub fn read(directory: &Path) -> Result<Self> {
        // A making, a look, and a list of shots each ask for it several times over: it is
        // read once for as long as the file's length and time stay what they were.
        let file = directory.join(FILE);
        let key = match std::fs::metadata(&file) {
            Ok(metadata) => (file.clone(), metadata.len(), metadata.modified().ok()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(EngineError::msg(format!("cannot read `{FILE}`: {error}"))),
        };
        let known = known();
        if let Some(cast) = known.lock().ok().and_then(|known| {
            known
                .as_ref()
                .filter(|(read, _)| *read == key)
                .map(|(_, cast)| cast.clone())
        }) {
            return Ok(cast);
        }
        let cast = match std::fs::read_to_string(&file) {
            Ok(text) => Self::parse(&text)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(EngineError::msg(format!("cannot read `{FILE}`: {error}"))),
        };
        if let Ok(mut known) = known.lock() {
            *known = Some((key, cast.clone()));
        }
        Ok(cast)
    }

    /// A cast of `entries`, in their order, read as a file of them would be: what is wrong
    /// with one is said as it is of a file's line.
    pub fn of_entries(entries: impl IntoIterator<Item = (Kind, String, Entry)>) -> Result<Self> {
        let cast = Self {
            entries: entries.into_iter().collect(),
            there: true,
        };
        Self::parse(&cast.text())
    }

    /// The cast as its file says it.
    pub fn text(&self) -> String {
        let mut text = format!("{HEADER}\n");
        for (kind, name, entry) in &self.entries {
            text.push_str(&line(*kind, name, entry));
            text.push('\n');
        }
        text
    }

    /// The cast as a file that held `was` says it: what a person wrote there that is not one
    /// of the cast, its comments and the lines left empty, stays where it is. One the file
    /// has stays on its line, as it was written where it is unchanged; one it has no more
    /// leaves its line; and one it had not follows what the file holds.
    fn text_over(&self, was: &str) -> String {
        let mut text = String::new();
        let mut headed = false;
        let mut kept = Vec::new();
        for written in was.lines() {
            let trimmed = written.trim();
            let keep = if trimmed.is_empty() || trimmed.starts_with('#') {
                Some(written.to_owned())
            } else if !headed {
                if trimmed != HEADER {
                    // Not a cast file: there is nothing of it to keep.
                    return self.text();
                }
                headed = true;
                Some(written.to_owned())
            } else {
                // A line that does not read is not one of the cast, and is not kept.
                let had = Self::parse(&format!("{HEADER}\n{trimmed}\n"))
                    .ok()
                    .and_then(|one| one.entries.into_iter().next());
                had.and_then(|(kind, name, had)| {
                    let entry = self.find(kind, &name)?;
                    if kept.contains(&(kind, name.clone())) {
                        return None;
                    }
                    let kept_line = if *entry == had {
                        written.to_owned()
                    } else {
                        line(kind, &name, entry)
                    };
                    kept.push((kind, name));
                    Some(kept_line)
                })
            };
            if let Some(keep) = keep {
                text.push_str(&keep);
                text.push('\n');
            }
        }
        if !headed {
            return self.text();
        }
        for (kind, name, entry) in &self.entries {
            if !kept.contains(&(*kind, name.clone())) {
                text.push_str(&line(*kind, name, entry));
                text.push('\n');
            }
        }
        text
    }

    /// Write the cast as the cast file of the project in `directory`. The comments of the
    /// file there are kept, and so is the order it says its cast in.
    pub fn write(&self, directory: &Path) -> Result<()> {
        let file = directory.join(FILE);
        let text = match std::fs::read_to_string(&file) {
            Ok(was) => self.text_over(&was),
            Err(_) => self.text(),
        };
        forget();
        std::fs::write(file, text)
            .map_err(|error| EngineError::msg(format!("cannot write `{FILE}`: {error}")))
    }

    /// Add `lines`, each a cast file's line, to the cast file of the project in `directory`,
    /// after what it holds, and say the names added. Only ones the file has not are added:
    /// a line for one it has is refused, since what a shot was made from is not changed by
    /// adding to it, and is by saying one again.
    pub fn add(directory: &Path, lines: &[String]) -> Result<Vec<String>> {
        let has = Self::read(directory)?;
        let adds = Self::parse(&format!("{HEADER}\n{}\n", lines.join("\n")))?;
        for (kind, name, _) in &adds.entries {
            if has.find(*kind, name).is_some() {
                return Err(EngineError::msg(format!(
                    "`{FILE}` has a {} `{name}` already, and changing one asks again for the shots it is in: that is the person's to do. Name it as it is, or add one by another name",
                    kind.word()
                )));
            }
        }
        let file = directory.join(FILE);
        let mut text = match std::fs::read_to_string(&file) {
            Ok(text) if text.is_empty() || text.ends_with('\n') => text,
            Ok(text) => format!("{text}\n"),
            Err(_) => String::new(),
        };
        if !has.there {
            text = format!("{HEADER}\n");
        }
        for (kind, name, entry) in &adds.entries {
            text.push_str(&line(*kind, name, entry));
            text.push('\n');
        }
        forget();
        std::fs::write(&file, text)
            .map_err(|error| EngineError::msg(format!("cannot write `{FILE}`: {error}")))?;
        Ok(adds
            .entries
            .iter()
            .map(|(kind, name, _)| format!("{} {name}", kind.word()))
            .collect())
    }

    /// Every one, with its kind and name, in the order the file says them.
    pub fn entries(&self) -> impl Iterator<Item = (Kind, &str, &Entry)> {
        self.entries
            .iter()
            .map(|(kind, name, entry)| (*kind, name.as_str(), entry))
    }

    fn find(&self, kind: Kind, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|(of, named, _)| *of == kind && named == name)
            .map(|(_, _, entry)| entry)
    }

    /// Every one of `kind`, by name, in the order the file says them.
    pub fn of(&self, kind: Kind) -> impl Iterator<Item = (&str, &Entry)> {
        self.entries()
            .filter(move |(of, _, _)| *of == kind)
            .map(|(_, name, entry)| (name, entry))
    }

    /// The one of `kind` named `name`, or an error that says which there are
    /// and the line that adds it.
    /// What a take's record keeps of the one `word` names of the kind `word` says, `cast`
    /// or `style`, as it stands: none where the file has it not.
    pub(crate) fn held(&self, word: &str, name: &str) -> Option<String> {
        let kind = [Kind::Cast, Kind::Style]
            .into_iter()
            .find(|kind| kind.word() == word)?;
        self.find(kind, name).map(Entry::hash)
    }

    fn named(&self, kind: Kind, name: &str) -> Result<&Entry> {
        self.find(kind, name)
            .ok_or_else(|| {
                let word = kind.word();
                let line = match kind {
                    Kind::Cast => format!(
                        "`cast {name} \"<who or what it is>\" ref=<a picture of it>`"
                    ),
                    Kind::Style => format!("`style {name} \"<how a shot looks>\"`"),
                };
                let have: Vec<&str> = self.of(kind).map(|(name, _)| name).collect();
                EngineError::msg(if !self.there {
                    format!(
                        "a step names `{word}={name}`, and the project has no `{FILE}`: a file in the project's folder that starts with `{HEADER}` and has the line {line}"
                    )
                } else if have.is_empty() {
                    format!(
                        "a step names `{word}={name}`, and `{FILE}` has no {word}: add the line {line}"
                    )
                } else {
                    format!(
                        "a step names `{word}={name}`, which `{FILE}` has not; it has {}. Name one of those, or add the line {line}",
                        have.join(", ")
                    )
                })
            })
    }
}

/// `word` as a line writes it: in quotes where it holds a space or a quote.
fn quoted(word: &str) -> String {
    if word.is_empty() || word.contains(|c: char| c.is_whitespace() || c == '"') {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        word.to_owned()
    }
}

/// One of a cast as a line of its file.
fn line(kind: Kind, name: &str, entry: &Entry) -> String {
    let mut line = format!("{} {name}", kind.word());
    if !entry.words.is_empty() {
        line.push_str(&format!(
            " \"{}\"",
            entry.words.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    for reference in &entry.references {
        line.push_str(&format!(" ref={}", quoted(reference)));
    }
    line
}

/// `prompt` and then each of `more` as a sentence of its own.
fn sentences(prompt: &str, more: impl IntoIterator<Item = String>) -> String {
    let mut told = String::new();
    for sentence in std::iter::once(prompt.to_owned()).chain(more) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        if !told.is_empty() {
            told.push(' ');
        }
        told.push_str(sentence);
        if !sentence.ends_with(['.', '!', '?']) {
            told.push('.');
        }
    }
    told
}

/// `ask` as its model is asked: its prompt with the words of each cast and
/// style it names after it, and their pictures after its own, each named from
/// the project's folder as a step names a file.
fn told(ask: &Ask, cast: &Cast, directory: &Path) -> Result<Ask> {
    let mut asked = ask.clone();
    let folder = Path::new("");
    let mut more = Vec::new();
    for (kind, names) in [(Kind::Cast, &ask.cast), (Kind::Style, &ask.styles)] {
        for name in names {
            let entry = cast.named(kind, name)?;
            if !entry.words.is_empty() {
                more.push(match kind {
                    // A name ties what the prompt says someone does to what they look like.
                    Kind::Cast => format!("{name}: {}", entry.words),
                    Kind::Style => entry.words.clone(),
                });
            }
            for reference in &entry.references {
                if !directory.join(reference).is_file() {
                    return Err(EngineError::msg(format!(
                        "{} `{name}` in `{FILE}` names the picture `{reference}`, which is not there: a picture is named from the project's folder",
                        kind.word()
                    )));
                }
                let source = Source::File(reference.clone());
                let same = |given: &Source| {
                    let inside = |source: &Source| match source {
                        Source::File(file) => within(folder, file, "picture").ok(),
                        Source::Made(_) => None,
                    };
                    inside(given).is_some() && inside(given) == inside(&source)
                };
                if !asked.references.iter().any(same) {
                    asked.references.push(source);
                }
            }
        }
    }
    asked.prompt = sentences(&ask.prompt, more);
    Ok(asked)
}

/// The cast and the styles the steps that make `media`, of the project in `directory`,
/// name, each once, in the order the steps give them: none where they name none or do
/// not read. The names are the steps' own, so one the cast file has not is among them.
pub fn named_by(media: &kerf_model::Media) -> (Vec<String>, Vec<String>) {
    let (mut cast, mut styles) = (Vec::new(), Vec::new());
    let read = (!media.steps.is_empty()).then(|| Recipe::of_steps(&media.steps));
    for ask in read.iter().flat_map(Recipe::asks) {
        for (names, of) in [(&mut cast, &ask.cast), (&mut styles, &ask.styles)] {
            for name in of {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
    }
    (cast, styles)
}

impl Recipe {
    /// Whether a step names any of the project's cast.
    pub fn names_cast(&self) -> bool {
        self.asks()
            .any(|ask| !ask.cast.is_empty() || !ask.styles.is_empty())
    }

    /// Each cast and style its steps name, once.
    fn cast_named(&self) -> Vec<(Kind, &str)> {
        let mut named: Vec<(Kind, &str)> = Vec::new();
        for ask in self.asks() {
            named.extend(ask.cast.iter().map(|name| (Kind::Cast, name.as_str())));
            named.extend(ask.styles.iter().map(|name| (Kind::Style, name.as_str())));
        }
        named.sort_unstable();
        named.dedup();
        named
    }

    fn asks(&self) -> impl Iterator<Item = &Ask> {
        self.steps.iter().filter_map(|step| match step {
            Step::Generate(ask) => Some(ask.as_ref()),
            _ => None,
        })
    }

    /// The steps as their models are asked: each `generate` step with what
    /// it names of the cast of the project in `directory` written out. Steps
    /// that name none are as they were, and no file is read for them.
    pub(crate) fn told(self, directory: &Path) -> Result<Self> {
        if !self.names_cast() {
            return Ok(self);
        }
        let cast = Cast::read(directory)?;
        let steps = self
            .steps
            .into_iter()
            .map(|step| match step {
                Step::Generate(ask) => Ok(Step::Generate(Box::new(told(&ask, &cast, directory)?))),
                other => Ok(other),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { steps })
    }

    /// What a take of this recipe is held to of the cast of the project in
    /// `directory`: each one it names, by `cast <name>` or `style <name>`,
    /// with a hash of its words and its pictures' names.
    pub(crate) fn cast_held(&self, directory: &Path) -> Result<Vec<(String, String)>> {
        if !self.names_cast() {
            return Ok(Vec::new());
        }
        let cast = Cast::read(directory)?;
        self.cast_named()
            .into_iter()
            .map(|(kind, name)| {
                Ok((
                    format!("{} {name}", kind.word()),
                    cast.named(kind, name)?.hash(),
                ))
            })
            .collect()
    }
}
