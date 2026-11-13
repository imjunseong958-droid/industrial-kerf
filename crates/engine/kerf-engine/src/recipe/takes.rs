//! The takes of a media a recipe makes, and what each was made from.
//!
//! A model does not give the same shot twice, so a making never writes
//! over the one before it. Each is a take, kept in a folder beside the
//! recipe, with a record of what it was made from and what its steps made
//! on the way:
//!
//! ```text
//! shots/open.kerfrecipe
//! shots/open.takes/001.mp4         the first take
//! shots/open.takes/001.take.json   what it was made from
//! shots/open.takes/001.work/       the guide its steps drew, the request its model read
//! shots/open.takes/002.mp4
//! ```
//!
//! The media's line names the take it is (`path="shots/open.takes/002.mp4"`),
//! so choosing a take is an edit like any other: it is undone, proposed, and
//! merged as one.
//!
//! A take's record holds a hash of each thing it was made from: the recipe,
//! each file the recipe reads, each sequence it has Kerf render, and each
//! signal it hands a model, as the model is handed it. A media
//! wants making again when one of those is no longer what its take was made
//! from, and says which. Nothing else about a project is in the record, so
//! trimming, moving, grading, or titling a made shot never asks for it
//! again.

use super::{signal::Signal, stored, within, Recipe};
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
    /// The takes of the recipe named `recipe` in the project kept in
    /// `directory`, for a media whose file is at `media`.
    pub fn of(directory: &Path, recipe: &str, media: &str) -> Result<Self> {
        // Every version of a recipe, `open.kerfrecipe` and `open@2.kerfrecipe`, keeps its takes
        // in one folder, so a take is never lost to the recipe being written again.
        let recipe = within(Path::new(""), recipe, "recipe")?;
        let (base, _) = super::versioned(&recipe);
        let takes = recipe.with_file_name(format!("{base}.takes"));
        Ok(Self {
            directory: directory.to_path_buf(),
            takes,
            extension: Path::new(media)
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned()),
        })
    }

    fn folder(&self) -> PathBuf {
        self.directory.join(&self.takes)
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
}

/// Write the record of `take`, made just now from `from`.
pub(crate) fn record(place: &Place, take: u32, from: BTreeMap<String, String>) -> Result<()> {
    let record = Record {
        version: RECORD_VERSION,
        made: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
        from,
    };
    std::fs::write(place.record(take), serde_json::to_vec_pretty(&record)?)?;
    Ok(())
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
/// changed in place to another of the same length is not seen.
fn sequence_hash(project: &Project, directory: &Path, id: &str) -> String {
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
        part.sequences.push(sequence.clone());
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

/// What a take of `recipe`, whose text is `text` and whose folder in the
/// project is `folder`, is made from as things stand: each thing by name,
/// with a hash of it.
pub(crate) fn from(
    project: &Project,
    directory: &Path,
    folder: &Path,
    text: &str,
    recipe: &Recipe,
) -> Result<BTreeMap<String, String>> {
    let mut from = BTreeMap::new();
    let mut hasher = Sha256::new();
    hasher.update(Recipe::essence(text));
    from.insert(RECIPE.to_owned(), hex_of(hasher));
    for file in recipe.reads() {
        let inside = within(folder, file, "file the recipe needs")?;
        from.insert(
            format!("file {}", stored(&inside)),
            file_hash(&directory.join(&inside)),
        );
    }
    for sequence in recipe.sequences() {
        from.insert(
            format!("sequence {sequence}"),
            sequence_hash(project, directory, sequence),
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
            other => other.strip_prefix("file ").unwrap_or(other).to_owned(),
        })
        .collect()
}

/// Why a media that names a recipe wants making.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unmade {
    /// Its file is not there.
    Missing,
    /// What its file was made from has changed since: each thing that has.
    Stale(Vec<String>),
    /// Its recipe cannot run as it is written, with why.
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
            Self::Unreadable(why) => format!("its recipe cannot run: {why}"),
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

/// Whether `media` of `project`, kept in `directory`, wants making from the
/// recipe it names, and why. `None` when it is made from things as they
/// stand, when it names no recipe, and when its recipe's file cannot be
/// read at all, since then nothing could make it.
pub fn unmade(project: &Project, directory: &Path, media: &Media) -> Option<Unmade> {
    let name = media.recipe.as_deref()?;
    let recipe_file = directory.join(within(Path::new(""), name, "recipe").ok()?);
    let text = std::fs::read_to_string(&recipe_file).ok()?;
    let recipe = match Recipe::parse(&text) {
        Ok(recipe) => recipe,
        Err(error) => return Some(Unmade::Unreadable(error.to_string())),
    };
    let file = directory.join(&media.path);
    if !file.is_file() {
        return Some(Unmade::Missing);
    }
    let place = Place::of(directory, name, &media.path).ok()?;
    let Some(record) = record_of(&place, media) else {
        // A file no take made is held to its recipe by their times.
        let written = std::fs::metadata(&recipe_file).ok()?.modified().ok()?;
        let made = std::fs::metadata(&file).ok()?.modified().ok()?;
        return (made < written).then(|| Unmade::Stale(vec!["the recipe".to_owned()]));
    };
    let folder = recipe_file
        .parent()
        .and_then(|folder| folder.strip_prefix(directory).ok())
        .unwrap_or(Path::new(""));
    match from(project, directory, folder, &text, &recipe) {
        Ok(now) => {
            let what = changed(&record.from, &now);
            (!what.is_empty()).then_some(Unmade::Stale(what))
        }
        Err(error) => Some(Unmade::Unreadable(error.to_string())),
    }
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
    /// The version of the media's recipe it was made from, as a document names it, where one
    /// of the versions there holds what its record says: the `recipe=` a media that is this
    /// take again names, so that it is not asked for again.
    pub recipe: Option<String>,
}

/// The takes of `media`, in the order they were made. A media that names
/// no recipe has none.
pub fn takes(project: &Project, directory: &Path, media: &Media) -> Result<Vec<Take>> {
    let Some(name) = media.recipe.as_deref() else {
        return Ok(Vec::new());
    };
    let place = Place::of(directory, name, &media.path)?;
    let folder = Path::new(name).parent().unwrap_or(Path::new(""));
    // What a take would be made from now, where the recipe can be read.
    let now = std::fs::read_to_string(directory.join(name))
        .ok()
        .and_then(|text| {
            let recipe = Recipe::parse(&text).ok()?;
            from(project, directory, folder, &text, &recipe).ok()
        });
    // Each version of the recipe, by the hash a take's record keeps of it.
    let versions: HashMap<String, String> = within(Path::new(""), name, "recipe")
        .map(|inside| super::versions(directory, &inside))
        .unwrap_or_default()
        .into_iter()
        .map(|(path, _, text)| {
            let mut hasher = Sha256::new();
            hasher.update(Recipe::essence(&text));
            (hex_of(hasher), stored(&path))
        })
        .collect();
    Ok(place
        .numbers()
        .into_iter()
        .filter(|take| place.file(*take).is_file())
        .map(|take| {
            let record = read_record(&place.record(take));
            let stored = place.stored(take);
            Take {
                number: take,
                chosen: Path::new(&media.path) == Path::new(&stored),
                made: record.as_ref().map(|record| record.made),
                changed: match (&record, &now) {
                    (Some(record), Some(now)) => changed(&record.from, now),
                    _ => Vec::new(),
                },
                recipe: record
                    .as_ref()
                    .and_then(|record| record.from.get(RECIPE))
                    .and_then(|hash| versions.get(hash).cloned()),
                stored,
            }
        })
        .collect())
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
