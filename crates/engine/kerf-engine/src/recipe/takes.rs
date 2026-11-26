//! The takes of a media its steps make, and what each was made from.
//!
//! A model does not give the same shot twice, so a making never writes
//! over the one before it. Each is a take, kept in a folder with a record
//! of what it was made from and what its steps made on the way:
//!
//! ```text
//! shots/open.takes/001.mp4         the first take
//! shots/open.takes/001.take.json   what it was made from
//! shots/open.takes/001.work/       the guide its steps drew, the request its model read
//! shots/open.takes/002.mp4
//! ```
//!
//! The folder is named for the file the media's line first names
//! (`path="shots/open.mp4"`), or, where the steps are in a recipe's file,
//! for the recipe (`shots/open.kerfrecipe`); the two are the same folder
//! for a shot named as its recipe is, so a shot carried from a recipe into
//! the document keeps its takes.
//!
//! The media's line names the take it is (`path="shots/open.takes/002.mp4"`),
//! so choosing a take is an edit like any other: it is undone, proposed, and
//! merged as one.
//!
//! A take's record holds a hash of each thing it was made from: the recipe,
//! each file the recipe reads, each sequence it has Kerf render, and each
//! signal it hands a model, as the model is handed it, and each of the
//! project's cast and styles it names ([`super::cast`]). A media
//! wants making again when one of those is no longer what its take was made
//! from, and says which. Nothing else about a project is in the record, so
//! trimming, moving, grading, or titling a made shot never asks for it
//! again.

use super::{moves::Moves, signal::Signal, stored, within, Recipe};
use crate::error::{EngineError, Result};
use kerf_model::{Content, Media, Project};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The version a take's record says it is.
const RECORD_VERSION: u32 = 1;

/// What the recipe itself is called among what a take was made from.
const RECIPE: &str = "recipe";

/// What the steps a document holds are called there.
const STEPS: &str = "steps";

/// What ends the name of a folder of takes.
const TAKES: &str = ".takes";

/// Where a recipe's takes are kept.
pub(crate) struct Place {
    /// The project's folder.
    directory: PathBuf,
    /// The takes' folder, from the project's.
    takes: PathBuf,
    /// The extension a take's file has, the media's own.
    extension: Option<String>,
}

impl Place {
    /// The takes a recipe's file made, for a media whose file is at `media` in the
    /// project kept in `directory`: where the makings of recipe files put them, which
    /// is where they are until the recipe is carried into the document and still are
    /// after, the folder being the same for a shot named as its recipe was.
    pub fn of(directory: &Path, recipe: &str, media: &str) -> Result<Self> {
        // Every version of a recipe, `open.kerfrecipe` and `open@2.kerfrecipe`, keeps its takes
        // in one folder, so a take is never lost to the recipe being written again.
        let recipe = within(Path::new(""), recipe, "recipe")?;
        let (base, _) = versioned(&recipe);
        let takes = recipe.with_file_name(format!("{base}{TAKES}"));
        Ok(Self {
            directory: directory.to_path_buf(),
            takes,
            extension: Path::new(media)
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned()),
        })
    }

    /// The takes of a media whose steps the document holds, whose file is at `media`
    /// in the project kept in `directory`. The folder is the one the media's file is
    /// in, when the media is a take already (`shots/open.takes/002.mp4`), and else
    /// the one named for the file its line names (`shots/open.mp4`), so that it is
    /// the same before the first making and after any.
    pub fn of_media(directory: &Path, media: &str) -> Result<Self> {
        let file = within(Path::new(""), media, "media")?;
        let is_take = |name: &std::ffi::OsStr| {
            let name = name.to_string_lossy();
            let number = name.split('.').next().unwrap_or_default();
            number.len() >= 3 && number.bytes().all(|byte| byte.is_ascii_digit())
        };
        let in_takes = file
            .parent()
            .filter(|folder| {
                folder
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(TAKES))
            })
            .filter(|_| file.file_name().is_some_and(is_take));
        let takes = match in_takes {
            Some(folder) => folder.to_path_buf(),
            None => {
                let stem = file.file_stem().ok_or_else(|| {
                    EngineError::msg(format!("the media's path `{media}` names no file"))
                })?;
                file.with_file_name(format!("{}{TAKES}", stem.to_string_lossy()))
            }
        };
        Ok(Self {
            directory: directory.to_path_buf(),
            takes,
            extension: file
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned()),
        })
    }

    fn folder(&self) -> PathBuf {
        self.directory.join(&self.takes)
    }

    /// The folder the takes' folder is in, from the project's.
    pub(crate) fn beside(&self) -> PathBuf {
        self.takes
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    fn name(&self, take: u32) -> String {
        match &self.extension {
            Some(extension) => format!("{take:03}.{extension}"),
            None => format!("{take:03}"),
        }
    }

    /// The take's file.
    pub fn file(&self, take: u32) -> PathBuf {
        self.folder().join(self.name(take))
    }

    /// The take's path as a document names it.
    pub fn stored(&self, take: u32) -> String {
        stored(&self.takes.join(self.name(take)))
    }

    /// Where a take is written before it is one: beside it, with its
    /// extension, so a program can tell the format from the name.
    pub fn making(&self, take: u32) -> PathBuf {
        self.folder().join(format!(".making-{}", self.name(take)))
    }

    /// The folder of what the take's steps make on the way.
    pub fn working(&self, take: u32) -> PathBuf {
        self.folder().join(format!("{take:03}.work"))
    }

    /// The take's record, named so that no take's own file can be it.
    fn record(&self, take: u32) -> PathBuf {
        self.folder().join(format!("{take:03}.take.json"))
    }

    /// The file `{name}` stands for in the take's steps.
    pub fn file_for(&self, take: u32, name: &str) -> PathBuf {
        if name == super::OUT {
            self.making(take)
        } else {
            self.working(take).join(name)
        }
    }

    /// The takes there are, in order. A take is what its number names a
    /// file of, a record of, or a working folder of.
    fn numbers(&self) -> BTreeSet<u32> {
        std::fs::read_dir(self.folder())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let number = name.split('.').next()?;
                (number.len() >= 3).then(|| number.parse().ok()).flatten()
            })
            .collect()
    }

    /// The number the next take gets: one past the last, never one a take
    /// has had.
    pub fn next(&self) -> u32 {
        self.numbers().last().map_or(1, |last| last + 1)
    }
}

/// What a take was made from, and when.
#[derive(Serialize, Deserialize)]
struct Record {
    #[serde(rename = "kerf-take")]
    version: u32,
    /// Seconds since 1970.
    made: u64,
    from: BTreeMap<String, String>,
    /// Which roll of the recipe it is: how far on its seeds were from the ones the recipe
    /// gives. A take is not held to it, being another of the same shot.
    #[serde(default, skip_serializing_if = "is_zero")]
    rolled: u32,
    /// Said of a take a recipe's file made, once that recipe was carried into the
    /// document ([`mark_carried`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carried: Option<Carried>,
}

/// What a take a recipe's file made says once the recipe's steps are the document's.
#[derive(Clone, Serialize, Deserialize)]
struct Carried {
    /// The recipe it was made from, as the document named it: the folder the files of
    /// what it keeps of its making ([`kept`]) are named from.
    recipe: String,
    /// What it is held to as a take of the steps the recipe became, in place of `from`,
    /// where it was made from the recipe and everything it read as they stood when it
    /// was carried: such a take is not asked for again for being carried. None for a
    /// take that was stale already, which stays so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<BTreeMap<String, String>>,
}

impl Record {
    /// What the take is held to as a take of steps the document holds: what it was made
    /// from, or, for one a recipe's file made, what that came to when it was carried.
    fn held(&self) -> &BTreeMap<String, String> {
        match &self.carried {
            Some(Carried {
                from: Some(from), ..
            }) => from,
            _ => &self.from,
        }
    }
}

/// A recipe file's name without its extension, split into the name its versions shared
/// and its version: `open@2.kerfrecipe` was `open`, 2. Every version kept its takes in
/// one folder, named for what they shared.
fn versioned(recipe: &Path) -> (String, u32) {
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

fn is_zero(number: &u32) -> bool {
    *number == 0
}

/// Write the record of `take`, made just now from `from` as roll `rolled` of its recipe.
pub(crate) fn record(
    place: &Place,
    take: u32,
    from: BTreeMap<String, String>,
    rolled: u32,
) -> Result<()> {
    let record = Record {
        version: RECORD_VERSION,
        made: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
        from,
        rolled,
        carried: None,
    };
    std::fs::write(place.record(take), serde_json::to_vec_pretty(&record)?)?;
    Ok(())
}

/// Have the takes of media `id` say that the recipe they were made from was carried into
/// the document: `before` is the project while the media named its recipe, and `after`
/// once its steps are under its line ([`super::carry`]). A take made from the recipe and
/// what it read as they stand is then held to the steps as they stand, so that it is not
/// asked for again for being carried; one that was stale stays so. Each says which
/// recipe it was, so that what it keeps of its making still reads. How many takes are
/// held to the steps now.
///
/// A record gains this and loses nothing.
pub fn mark_carried(
    before: &Project,
    after: &Project,
    directory: &Path,
    id: &str,
) -> Result<usize> {
    let no_media = || EngineError::NoMedia(id.to_string());
    let (was, now) = (
        before.media(id).ok_or_else(no_media)?,
        after.media(id).ok_or_else(no_media)?,
    );
    let (Some(recipe), false) = (super::uncarried(was), now.steps.is_empty()) else {
        return Err(EngineError::msg(format!(
            "media `{id}` was not carried from a recipe into the document"
        )));
    };
    // What a take's record holds of each thing, as that thing stands now: a take all of
    // whose things are as they were is one made from the recipe as it stands.
    let text = within(Path::new(""), recipe, "recipe")
        .ok()
        .and_then(|inside| std::fs::read_to_string(directory.join(inside)).ok());
    let cast = super::cast::Cast::read(directory).ok();
    // A sequence is held as its guide is drawn, which the steps say.
    let drawn = Recipe::of_steps(&now.steps);
    let stands = |key: &str| -> Option<String> {
        if key == RECIPE {
            let mut hasher = Sha256::new();
            hasher.update(Recipe::essence(text.as_deref()?));
            return Some(hex_of(hasher));
        }
        let (kind, name) = key.split_once(' ')?;
        match kind {
            "file" => Some(file_hash(&directory.join(name))),
            "sequence" => Some(sequence_hash(
                before,
                directory,
                name,
                drawn.only_filmed(name),
            )),
            "signal" => Signal::read(before, name).ok().map(|signal| signal.hash()),
            _ => cast.as_ref()?.held(kind, name),
        }
    };
    let steps = Recipe::of_steps(&now.steps).told(directory)?;
    let held = from(after, directory, &super::steps_text(&now.steps), &steps)?;
    let place = Place::of(directory, recipe, &was.path)?;
    let mut marked = 0;
    for take in place.numbers() {
        let file = place.record(take);
        let Some(mut record) = read_record(&file) else {
            continue;
        };
        let current = record
            .from
            .iter()
            .all(|(key, hash)| stands(key).as_deref() == Some(hash.as_str()));
        marked += usize::from(current);
        record.carried = Some(Carried {
            recipe: recipe.to_owned(),
            from: current.then(|| held.clone()),
        });
        // Whole or not at all: a record half written is no record.
        let beside = file.with_extension(format!("{}.writing", std::process::id()));
        std::fs::write(&beside, serde_json::to_vec_pretty(&record)?)?;
        std::fs::rename(&beside, &file)?;
    }
    Ok(marked)
}

fn read_record(file: &Path) -> Option<Record> {
    let record: Record = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    (record.version == RECORD_VERSION).then_some(record)
}

fn hex_of(hasher: Sha256) -> String {
    hex::encode(hasher.finalize())
}

/// A hash of what `file` holds, or that it is not there. A file is read
/// once for as long as its length and its time stay what they were.
fn file_hash(file: &Path) -> String {
    /// A file as it was when it was read: where, how long, and when written.
    type Read = (PathBuf, u64, Option<SystemTime>);
    static KNOWN: OnceLock<Mutex<HashMap<Read, String>>> = OnceLock::new();
    let Ok(metadata) = std::fs::metadata(file) else {
        return "absent".to_owned();
    };
    let key = (file.to_path_buf(), metadata.len(), metadata.modified().ok());
    let known = KNOWN.get_or_init(Mutex::default);
    if let Some(hash) = known.lock().ok().and_then(|known| known.get(&key).cloned()) {
        return hash;
    }
    let mut hasher = Sha256::new();
    let read = std::fs::File::open(file).and_then(|mut file| std::io::copy(&mut file, &mut hasher));
    if read.is_err() {
        return "absent".to_owned();
    }
    let hash = hex_of(hasher);
    if let Ok(mut known) = known.lock() {
        known.insert(key, hash.clone());
    }
    hash
}

/// A hash of what drawing sequence `id` of `project` reads of the
/// document: the sequence, the sequences its clips nest, and the media
/// lines they play, with how long each media's file is. A media's file
/// changed in place to another of the same length is not seen. With `only_filmed`, the
/// sequence is as a guide of only what a model films is drawn from
/// ([`kerf_model::Sequence::only_filmed`]), so what changes in a clip Kerf draws itself
/// changes nothing here.
fn sequence_hash(project: &Project, directory: &Path, id: &str, only_filmed: bool) -> String {
    let mut part = Project::default();
    let mut waiting = vec![id.to_owned()];
    let mut media: BTreeSet<String> = BTreeSet::new();
    while let Some(next) = waiting.pop() {
        if part
            .sequences
            .iter()
            .any(|sequence| sequence.id == next.as_str())
        {
            continue;
        }
        let Some(sequence) = project.sequence(&next) else {
            continue;
        };
        let sequence = match only_filmed && next == id {
            true => sequence.only_filmed(),
            false => sequence.clone(),
        };
        for clip in sequence.tracks.iter().flat_map(|track| &track.clips) {
            match &clip.content {
                Content::Media { media: played, .. } => {
                    media.insert(played.to_string());
                }
                Content::Sequence {
                    sequence: nested, ..
                } => waiting.push(nested.to_string()),
                _ => {}
            }
        }
        part.sequences.push(sequence);
    }
    part.sequences
        .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    part.media = project
        .media
        .iter()
        .filter(|candidate| media.contains(candidate.id.as_str()))
        .cloned()
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(kerf_lang::print_document(
        &part,
        kerf_model::Catalog::builtin(),
        kerf_lang::PrintOptions::default(),
    ));
    for played in &part.media {
        let length =
            std::fs::metadata(directory.join(&played.path)).map_or(0, |metadata| metadata.len());
        hasher.update(length.to_le_bytes());
    }
    hex_of(hasher)
}

/// The files the mesh clips of sequence `id` of `project` take their meshes from
/// (`file=`), and those of the sequences it nests, as the document names them. Drawing
/// the sequence reads them, and nothing of the document says when one changes.
pub(crate) fn mesh_files(project: &Project, id: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut waiting = vec![id.to_owned()];
    while let Some(next) = waiting.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(sequence) = project.sequence(&next) else {
            continue;
        };
        for clip in sequence.tracks.iter().flat_map(|track| &track.clips) {
            match &clip.content {
                Content::Sequence {
                    sequence: nested, ..
                } => waiting.push(nested.to_string()),
                Content::Generator { fx, params, .. } if fx.as_str() == "mesh" => {
                    if let Some(kerf_model::Param::Static(kerf_model::Value::Text(file))) =
                        params.get("file")
                        && !file.is_empty()
                    {
                        files.insert(file.clone());
                    }
                }
                _ => {}
            }
        }
    }
    files
}

/// What a take of `recipe`, whose text is `text`, is made from as things stand: each
/// thing by name, with a hash of it. `recipe` is as its models are asked
/// ([`Recipe::told`]), so the pictures of the cast it names are among the files it reads.
pub(crate) fn from(
    project: &Project,
    directory: &Path,
    text: &str,
    recipe: &Recipe,
) -> Result<BTreeMap<String, String>> {
    let folder = Path::new("");
    let mut from = BTreeMap::new();
    let mut hasher = Sha256::new();
    hasher.update(Recipe::essence(text));
    from.insert(STEPS.to_owned(), hex_of(hasher));
    // A sequence a step draws reads the files its mesh clips name, and nothing of the
    // document says when one changes: a take is held to them, found from the clips.
    for file in recipe
        .sequences()
        .flat_map(|sequence| mesh_files(project, sequence))
    {
        if let Ok(inside) = within(Path::new(""), &file, "mesh") {
            from.insert(
                format!("file {}", stored(&inside)),
                file_hash(&directory.join(&inside)),
            );
        }
    }
    for file in recipe.reads() {
        let inside = within(folder, file, "file a step names")?;
        from.insert(
            format!("file {}", stored(&inside)),
            file_hash(&directory.join(&inside)),
        );
    }
    for sequence in recipe.sequences() {
        from.insert(
            format!("sequence {sequence}"),
            sequence_hash(project, directory, sequence, recipe.only_filmed(sequence)),
        );
    }
    // Where a clip goes is what its sequence draws of it, so a take is held to that sequence.
    for clip in recipe.moves() {
        let sequence = Moves::sequence_of(project, clip).map_err(EngineError::msg)?;
        from.insert(
            format!("sequence {sequence}"),
            sequence_hash(project, directory, &sequence, recipe.only_filmed(&sequence)),
        );
    }
    for signal in recipe.signals() {
        from.insert(
            format!("signal {signal}"),
            Signal::read(project, signal)
                .map_err(EngineError::msg)?
                .hash(),
        );
    }
    from.extend(recipe.cast_held(directory)?);
    Ok(from)
}

/// What is not the same in `now` as in `then`, as a person is told it.
fn changed(then: &BTreeMap<String, String>, now: &BTreeMap<String, String>) -> Vec<String> {
    then.keys()
        .chain(now.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| then.get(*name) != now.get(*name))
        .map(|name| match name.as_str() {
            RECIPE => "the recipe".to_owned(),
            STEPS => "its steps".to_owned(),
            other => other.strip_prefix("file ").unwrap_or(other).to_owned(),
        })
        .collect()
}

/// Why a media its steps make wants making.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unmade {
    /// Its file is not there.
    Missing,
    /// What its file was made from has changed since: each thing that has.
    Stale(Vec<String>),
    /// Its steps cannot run as they are written, with why: said whole, as
    /// `its recipe cannot run: …` or `its steps cannot run: …`.
    Unreadable(String),
}

impl Unmade {
    /// The reason, as a person or an agent is told it.
    pub fn reason(&self) -> String {
        match self {
            Self::Missing => "its file is not there yet".to_owned(),
            Self::Stale(what) => {
                let what = match what.as_slice() {
                    [] => "what it was made from".to_owned(),
                    [one] => one.clone(),
                    [most @ .., last] => format!("{} and {last}", most.join(", ")),
                };
                format!("{what} changed after its file was made")
            }
            Self::Unreadable(why) => why.clone(),
        }
    }
}

/// The record of the take `media`'s line names, if it names one of
/// `place`'s takes and that take has a record.
fn record_of(place: &Place, media: &Media) -> Option<Record> {
    let file = Path::new(&media.path);
    let take = file
        .file_name()?
        .to_string_lossy()
        .split('.')
        .next()?
        .parse()
        .ok()?;
    (file == Path::new(&place.stored(take)))
        .then(|| read_record(&place.record(take)))
        .flatten()
}

/// The steps that make a media as things stand, read once for whoever asks more than one
/// thing of them: whether it wants making and what its takes are each hash what they read.
pub(crate) struct Standing<'a> {
    project: &'a Project,
    directory: &'a Path,
    media: &'a Media,
    /// Their text.
    text: String,
    /// The steps as their models are asked, or why they cannot run.
    pub(crate) recipe: Result<Recipe>,
    /// What a take would be made from now, once it has been asked for.
    now: std::cell::OnceCell<Option<std::result::Result<BTreeMap<String, String>, String>>>,
}

impl<'a> Standing<'a> {
    /// Read the steps that make `media`. `None` for a media that has none under its line.
    pub(crate) fn of(project: &'a Project, directory: &'a Path, media: &'a Media) -> Option<Self> {
        if media.steps.is_empty() {
            return None;
        }
        let text = super::steps_text(&media.steps);
        let recipe = Recipe::parse(&text).and_then(|recipe| recipe.told(directory));
        Some(Self {
            project,
            directory,
            media,
            text,
            recipe,
            now: std::cell::OnceCell::new(),
        })
    }

    /// What a take would be made from now: none where the steps cannot be read.
    fn now(&self) -> Option<&std::result::Result<BTreeMap<String, String>, String>> {
        self.now
            .get_or_init(|| match &self.recipe {
                Ok(recipe) => Some(
                    from(self.project, self.directory, &self.text, recipe)
                        .map_err(|error| error.to_string()),
                ),
                Err(_) => None,
            })
            .as_ref()
    }

    /// See [`unmade`].
    pub(crate) fn unmade(&self) -> Option<Unmade> {
        let cannot = |why: &str| Unmade::Unreadable(format!("its steps cannot run: {why}"));
        if let Err(error) = &self.recipe {
            return Some(cannot(&error.to_string()));
        }
        let file = self.directory.join(&self.media.path);
        if !file.is_file() {
            return Some(Unmade::Missing);
        }
        let place = Place::of_media(self.directory, &self.media.path).ok()?;
        // A file no take made is taken as it is: nothing says what it was made from.
        let record = record_of(&place, self.media)?;
        match self.now()? {
            Ok(now) => {
                let what = changed(record.held(), now);
                (!what.is_empty()).then_some(Unmade::Stale(what))
            }
            Err(error) => Some(cannot(error)),
        }
    }

    /// See [`takes`].
    pub(crate) fn takes(&self) -> Result<Vec<Take>> {
        let place = Place::of_media(self.directory, &self.media.path)?;
        let now = self.now().and_then(|now| now.as_ref().ok());
        Ok(place
            .numbers()
            .into_iter()
            .filter(|take| place.file(*take).is_file())
            .map(|take| {
                let record = read_record(&place.record(take));
                let stored = place.stored(take);
                Take {
                    number: take,
                    chosen: Path::new(&self.media.path) == Path::new(&stored),
                    made: record.as_ref().map(|record| record.made),
                    changed: match (&record, now) {
                        (Some(record), Some(now)) => changed(record.held(), now),
                        _ => Vec::new(),
                    },
                    // Made from the steps as the document has them now.
                    current: match (&record, now) {
                        (Some(record), Some(now)) => {
                            let held = record.held();
                            held.contains_key(STEPS) && held.get(STEPS) == now.get(STEPS)
                        }
                        _ => false,
                    },
                    rolled: record.as_ref().map_or(0, |record| record.rolled),
                    kept: place.working(take).join(KEPT_RECIPE).is_file(),
                    stored,
                }
            })
            .collect())
    }
}

/// Whether `media` of `project`, kept in `directory`, wants making from its
/// steps, and why. `None` when it is made from things as they stand, and when
/// nothing makes it. A media whose steps are still in a recipe's file is said
/// to want carrying: nothing is made from a recipe.
pub fn unmade(project: &Project, directory: &Path, media: &Media) -> Option<Unmade> {
    if let Some(recipe) = super::uncarried(media) {
        return Some(Unmade::Unreadable(
            super::not_carried(media.id.as_str(), recipe).to_string(),
        ));
    }
    Standing::of(project, directory, media)?.unmade()
}

/// One making of a media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    pub number: u32,
    /// Its path as a document names it.
    pub stored: String,
    /// When it was made, in seconds since 1970, where its record says.
    pub made: Option<u64>,
    /// Whether it is the one the media's line names.
    pub chosen: bool,
    /// What has changed since it was made, of what it was made from.
    pub changed: Vec<String>,
    /// Whether it was made from the steps as the document has them now. Another making
    /// of those is another roll of this shot.
    pub current: bool,
    /// Which roll of its recipe it is: how far on its seeds were from the ones the recipe
    /// gives.
    pub rolled: u32,
    /// Whether the recipe it was made from is kept with it, to go back to ([`kept`]).
    pub kept: bool,
}

/// What a take keeps of the steps it was made from, in its working folder: named as it
/// was when steps were a recipe's, so that a take made then and one made now read alike.
const KEPT_RECIPE: &str = "made-from.kerfrecipe";
/// What a file its recipe named is kept as there, before its own name.
const KEPT_FILE: &str = "made-from.";

/// What a take was made from, kept with it: the recipe's text as it was asked, each seed as
/// that take had it, and the files its lines named as `workflow=`, by the names they had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub recipe: String,
    /// Each file a `generate` line named as its workflow: its name from the recipe's
    /// folder, and what it held.
    pub files: Vec<(String, String)>,
}

/// Keep with `take` what it was made from: `text`, the recipe as the making read it with
/// `roll` added to each seed its lines give, and each workflow file a line names. A take
/// made before this kept none, and has nothing to go back to but its record's hashes.
pub(crate) fn keep(
    place: &Place,
    take: u32,
    text: &str,
    roll: u32,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<()> {
    use super::text::{changed, step_lines, Change};
    use super::Step;
    let steps = Recipe::parse(text)?.steps;
    let mut kept = text.to_owned();
    let working = place.working(take);
    for (line, step) in step_lines(text).into_iter().zip(&steps) {
        let Step::Generate(ask) = step else {
            continue;
        };
        if let (Some(seed), true) = (ask.seed, roll > 0) {
            let rolled = seed.wrapping_add(u64::from(roll)).to_string();
            kept = changed(
                &kept,
                line,
                Change::Set {
                    key: "seed",
                    nth: 0,
                    value: &rolled,
                },
            )?;
        }
        if let Some(super::model::Source::File(named)) = &ask.workflow {
            let own = Path::new(named)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            if let (Some(own), Some(held)) = (own, read(named)) {
                std::fs::write(working.join(format!("{KEPT_FILE}{own}")), held)?;
            }
        }
    }
    std::fs::write(working.join(KEPT_RECIPE), kept)?;
    Ok(())
}

/// What take `take` of `media` keeps of what it was made from. `None` for a take that
/// kept none: one made before takes kept it, or whose working folder is gone.
pub fn kept(directory: &Path, media: &Media, take: u32) -> Option<Kept> {
    use super::Step;
    let place = super::place_of(directory, media)?.ok()?;
    let working = place.working(take);
    let mut recipe = std::fs::read_to_string(working.join(KEPT_RECIPE)).ok()?;
    // A take a recipe's file made keeps the recipe as that file had it, its files named
    // from the recipe's folder. Once the recipe is carried into the document, what the
    // take keeps reads as the steps it became.
    let carried = read_record(&place.record(take)).and_then(|record| record.carried);
    if let (Some(carried), false) = (carried, media.steps.is_empty()) {
        let folder = super::folder_of(&carried.recipe);
        if let Ok((steps, _)) = super::steps_of_text(&recipe, Some(&folder)) {
            recipe = super::steps_text(&steps);
        }
    }
    let files = Recipe::parse(&recipe)
        .ok()?
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Generate(ask) => match &ask.workflow {
                Some(super::model::Source::File(named)) => Some(named.clone()),
                _ => None,
            },
            _ => None,
        })
        .filter_map(|named| {
            let own = Path::new(&named)
                .file_name()?
                .to_string_lossy()
                .into_owned();
            let held = std::fs::read_to_string(working.join(format!("{KEPT_FILE}{own}"))).ok()?;
            Some((named, held))
        })
        .collect();
    Some(Kept { recipe, files })
}

/// The file a step of `media`'s recipe made as `{name}` on the way to take `take`, a guide
/// it drew or a picture a model made for a later step, as a document would name it: there
/// while the take's working folder is. `None` where it is not.
pub fn step_file(directory: &Path, media: &Media, take: u32, name: &str) -> Option<String> {
    let place = super::place_of(directory, media)?.ok()?;
    let inside = within(Path::new(""), name, "file").ok()?;
    place
        .working(take)
        .join(&inside)
        .is_file()
        .then(|| stored(&place.takes.join(format!("{take:03}.work")).join(&inside)))
}

/// The takes of `media`, in the order they were made. A media that names
/// no recipe has none.
pub fn takes(project: &Project, directory: &Path, media: &Media) -> Result<Vec<Take>> {
    match Standing::of(project, directory, media) {
        Some(standing) => standing.takes(),
        // Those a recipe's file made, before it is carried: there to see and to choose,
        // and held to nothing until the steps are the document's.
        None => Ok(listed(directory, media)
            .into_iter()
            .map(|(number, stored, chosen)| Take {
                number,
                stored,
                made: None,
                chosen,
                changed: Vec::new(),
                current: false,
                rolled: 0,
                kept: false,
            })
            .collect()),
    }
}

/// The takes of `media` there are files of, in the order they were made, each with its
/// number, its path as a document names it, and whether it is the one the media's line
/// names. It reads the takes' folder and nothing else, so it can be asked often: what has
/// changed since each was made is [`takes`]'s to say.
pub fn listed(directory: &Path, media: &Media) -> Vec<(u32, String, bool)> {
    let Some(Ok(place)) = super::place_of(directory, media) else {
        return Vec::new();
    };
    place
        .numbers()
        .into_iter()
        .filter(|take| place.file(*take).is_file())
        .map(|take| {
            let stored = place.stored(take);
            let chosen = Path::new(&media.path) == Path::new(&stored);
            (take, stored, chosen)
        })
        .collect()
}

/// The take numbered `number` among `media`'s, or an error that says which
/// there are.
pub fn take(project: &Project, directory: &Path, media: &Media, number: u32) -> Result<Take> {
    let all = takes(project, directory, media)?;
    let have = all
        .iter()
        .map(|take| take.number.to_string())
        .collect::<Vec<_>>();
    all.into_iter()
        .find(|take| take.number == number)
        .ok_or_else(|| {
            EngineError::msg(if have.is_empty() {
                format!(
                    "media `{}` has no takes yet; `kerf make {}` makes one",
                    media.id, media.id
                )
            } else {
                format!(
                    "media `{}` has no take {number}; it has {}",
                    media.id,
                    have.join(", ")
                )
            })
        })
}
