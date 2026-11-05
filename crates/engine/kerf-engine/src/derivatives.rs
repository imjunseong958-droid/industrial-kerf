//! Project-local, content-addressed storage for regenerable media artifacts.
//!
//! The cache records source identity and normalized options in a small manifest
//! so an interrupted generation cannot be mistaken for a valid result. Cache
//! entries are never canonical project state, so any of them may be removed:
//! an entry made from a source goes once that source changes, and an entry
//! of a kind made per document revision once newer ones push it out (see
//! [`DerivativeCache::keep_newest`]).

use crate::error::{EngineError, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Relative location of regenerable media artifacts inside a project.
pub const DERIVED_CACHE_RELATIVE_PATH: &str = ".kerf/cache/derived";
const MANIFEST_VERSION: u8 = 1;

/// File identity used to invalidate derivatives when a source is replaced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIdentity {
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ns: Option<u64>,
}

impl SourceIdentity {
    /// Read a stable path/stat identity for a local source.
    pub fn from_path(path: &Path) -> Result<Self> {
        let canonical = path.canonicalize()?;
        let metadata = fs::metadata(&canonical)?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| u64::try_from(duration.as_nanos()).ok());
        Ok(Self {
            path: canonical.to_string_lossy().into_owned(),
            size: metadata.len(),
            modified_ns,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DerivedManifest {
    schema_version: u8,
    kind: String,
    key: String,
    identity: Value,
    options: Value,
    artifacts: Vec<String>,
}

/// One content-addressed cache entry.
#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub key: String,
    pub directory: PathBuf,
    kind: String,
    identity: Value,
    options: Value,
}

impl CacheEntry {
    /// Return a path below this entry. Callers should use fixed artifact names.
    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    /// Whether the manifest and every requested artifact are complete.
    pub fn is_complete(&self, artifacts: &[&str]) -> bool {
        if artifacts.iter().any(|name| !self.path(name).is_file()) {
            return false;
        }
        let manifest_path = self.path("manifest.json");
        let Ok(bytes) = fs::read(manifest_path) else {
            return false;
        };
        let Ok(manifest) = serde_json::from_slice::<DerivedManifest>(&bytes) else {
            return false;
        };
        manifest.schema_version == MANIFEST_VERSION
            && manifest.kind == self.kind
            && manifest.key == self.key
            && manifest.identity == self.identity
            && manifest.options == self.options
            && artifacts
                .iter()
                .all(|name| manifest.artifacts.iter().any(|artifact| artifact == name))
    }

    /// Publish a completed entry by atomically replacing its manifest.
    pub fn mark_complete(&self, artifacts: &[&str]) -> Result<()> {
        let artifacts = artifacts.iter().map(|name| (*name).to_owned()).collect();
        let manifest = DerivedManifest {
            schema_version: MANIFEST_VERSION,
            kind: self.kind.clone(),
            key: self.key.clone(),
            identity: self.identity.clone(),
            options: self.options.clone(),
            artifacts,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        let temp_path = self.path(&format!("manifest.json.tmp-{}", std::process::id()));
        fs::write(&temp_path, bytes)?;
        fs::rename(temp_path, self.path("manifest.json"))?;
        self.remove_stale_versions();
        Ok(())
    }

    /// Remove the entries of this kind made from an earlier version of this
    /// entry's source: one that was replaced or touched, which no request
    /// asks for again.
    fn remove_stale_versions(&self) {
        let Ok(identity) = serde_json::from_value::<SourceIdentity>(self.identity.clone()) else {
            return;
        };
        // An entry finished after its source changed is itself the stale
        // one, and must not remove the entry made from the source as it is.
        if SourceIdentity::from_path(Path::new(&identity.path))
            .ok()
            .as_ref()
            != Some(&identity)
        {
            return;
        }
        let Some(root) = self.directory.parent() else {
            return;
        };
        for (directory, manifest) in manifests(root, &self.kind) {
            let stale = serde_json::from_value::<SourceIdentity>(manifest.identity)
                .is_ok_and(|other| other.path == identity.path && other != identity);
            if stale && directory != self.directory {
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    /// Read a JSON artifact written by a completed generator.
    pub fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        let bytes = fs::read(self.path(name))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Write a JSON artifact before publishing the entry manifest.
    pub fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(value)?;
        fs::write(self.path(name), bytes)?;
        Ok(())
    }
}

/// Accessor for the project-local derivative cache.
#[derive(Clone, Debug)]
pub struct DerivativeCache {
    root: PathBuf,
}

impl DerivativeCache {
    /// Create a cache accessor. The directory is created on first entry use.
    pub fn new(project_dir: impl AsRef<Path>) -> Self {
        Self {
            root: project_dir.as_ref().join(DERIVED_CACHE_RELATIVE_PATH),
        }
    }

    /// Build an entry keyed by a source's current identity and normalized options.
    pub fn source_entry(&self, source: &Path, kind: &str, options: &Value) -> Result<CacheEntry> {
        let identity = SourceIdentity::from_path(source)?;
        self.entry(kind, serde_json::to_value(identity)?, options)
    }

    /// Build an entry keyed by a project identity and normalized options.
    pub fn project_entry(
        &self,
        project: &Value,
        source_identities: &[SourceIdentity],
        kind: &str,
        options: &Value,
    ) -> Result<CacheEntry> {
        let identity = json_identity(project, source_identities)?;
        self.entry(kind, identity, options)
    }

    /// The complete entries of `kind` made from `source` as it is now, with
    /// whatever options they were made with.
    pub fn source_entries(&self, source: &Path, kind: &str) -> Result<Vec<CacheEntry>> {
        let identity = serde_json::to_value(SourceIdentity::from_path(source)?)?;
        let prefix = format!("{kind}-");
        let directories = match fs::read_dir(&self.root) {
            Ok(directories) => directories,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut entries = Vec::new();
        for directory in directories {
            let directory = directory?.path();
            let named = directory
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix));
            let Some(manifest) = named
                .then(|| fs::read(directory.join("manifest.json")).ok())
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<DerivedManifest>(&bytes).ok())
            else {
                continue;
            };
            if manifest.schema_version == MANIFEST_VERSION
                && manifest.kind == kind
                && manifest.identity == identity
            {
                entries.push(CacheEntry {
                    key: manifest.key,
                    directory,
                    kind: manifest.kind,
                    identity: manifest.identity,
                    options: manifest.options,
                });
            }
        }
        Ok(entries)
    }

    /// Keep `current` and, of the other entries of its kind, the `keep`
    /// used most recently; for kinds made per document revision, which
    /// would otherwise gather an entry for every edit. Entries made from a
    /// file of a removed entry go with it.
    pub fn keep_newest(&self, current: &CacheEntry, keep: usize) -> Result<()> {
        // Marks `current` used, so another process keeps it too.
        fs::File::options()
            .write(true)
            .open(current.path("manifest.json"))?
            .set_modified(SystemTime::now())?;
        let mut others = kind_directories(&self.root, &current.kind)
            .into_iter()
            .filter(|directory| *directory != current.directory)
            .map(|directory| (last_used(&directory), directory))
            .collect::<Vec<_>>();
        if others.len() <= keep {
            return Ok(());
        }
        others.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, directory) in others.drain(keep..) {
            let _ = fs::remove_dir_all(directory);
        }
        self.remove_orphans();
        Ok(())
    }

    /// Remove the entries made from a file of an entry that is gone, as a
    /// transcript of a rendered program's sound.
    fn remove_orphans(&self) {
        let Ok(root) = self.root.canonicalize() else {
            return;
        };
        let Ok(directories) = fs::read_dir(&root) else {
            return;
        };
        for directory in directories.filter_map(|entry| Some(entry.ok()?.path())) {
            let Some(manifest) = read_manifest(&directory) else {
                continue;
            };
            let orphaned =
                serde_json::from_value::<SourceIdentity>(manifest.identity).is_ok_and(|source| {
                    let source = Path::new(&source.path);
                    source.starts_with(&root) && !source.exists()
                });
            if orphaned {
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    fn entry(&self, kind: &str, identity: Value, options: &Value) -> Result<CacheEntry> {
        if kind.is_empty()
            || !kind
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(EngineError::msg(
                "derivative cache kind must be a safe identifier",
            ));
        }
        let identity = normalize_json(&identity);
        let options = normalize_json(options);
        let key = cache_key_for_values(kind, &identity, &options);
        let directory = self.root.join(format!("{kind}-{key}"));
        fs::create_dir_all(&directory)?;
        Ok(CacheEntry {
            key,
            directory,
            kind: kind.to_owned(),
            identity,
            options,
        })
    }
}

/// The manifest of the entry in `directory`, when it has a readable one.
fn read_manifest(directory: &Path) -> Option<DerivedManifest> {
    let bytes = fs::read(directory.join("manifest.json")).ok()?;
    serde_json::from_slice::<DerivedManifest>(&bytes)
        .ok()
        .filter(|manifest| manifest.schema_version == MANIFEST_VERSION)
}

/// The directory of each entry of `kind` below `root`, complete or not.
fn kind_directories(root: &Path, kind: &str) -> Vec<PathBuf> {
    let Ok(directories) = fs::read_dir(root) else {
        return Vec::new();
    };
    let prefix = format!("{kind}-");
    directories
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|directory| {
            // `<kind>-<key>`, so one kind's prefix never takes another's.
            directory
                .file_name()
                .and_then(|name| name.to_str()?.strip_prefix(&prefix))
                .is_some_and(|key| key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()))
        })
        .collect()
}

/// Each complete entry of `kind` below `root`, with its manifest.
fn manifests(root: &Path, kind: &str) -> Vec<(PathBuf, DerivedManifest)> {
    kind_directories(root, kind)
        .into_iter()
        .filter_map(|directory| {
            let manifest = read_manifest(&directory)?;
            (manifest.kind == kind).then_some((directory, manifest))
        })
        .collect()
}

/// When the entry in `directory` was last made or used: its manifest's
/// time, else (one never completed) the directory's own.
fn last_used(directory: &Path) -> SystemTime {
    fs::metadata(directory.join("manifest.json"))
        .or_else(|_| fs::metadata(directory))
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

/// Return the deterministic SHA-256 key used for one derivative request.
pub fn cache_key_for_values(kind: &str, identity: &Value, options: &Value) -> String {
    let payload = json!({
        "kind": kind,
        "identity": normalize_json(identity),
        "options": normalize_json(options),
    });
    let bytes = serde_json::to_vec(&payload).expect("JSON values are serializable");
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn json_identity(project: &Value, source_identities: &[SourceIdentity]) -> Result<Value> {
    let source_identities = source_identities
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!({
        "project": project,
        "sources": source_identities,
    }))
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        Value::Object(values) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in values {
                sorted.insert(key.clone(), normalize_json(value));
            }
            let object: Map<String, Value> = sorted.into_iter().collect();
            Value::Object(object)
        }
        value => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn normalized_options_have_stable_keys() {
        let first = cache_key_for_values(
            "waveform",
            &json!({"path":"a","size":1}),
            &json!({"b": 2, "a": 1}),
        );
        let second = cache_key_for_values(
            "waveform",
            &json!({"size":1,"path":"a"}),
            &json!({"a": 1, "b": 2}),
        );
        assert_eq!(first, second);
    }

    #[test]
    fn replacement_identity_creates_a_new_entry() {
        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.bin");
        fs::write(&source, b"one").expect("write source");
        let cache = DerivativeCache::new(root.path());
        let first = cache
            .source_entry(&source, "frames", &json!({"times":[0.0]}))
            .expect("first entry");
        fs::write(&source, b"replacement").expect("replace source");
        let second = cache
            .source_entry(&source, "frames", &json!({"times":[0.0]}))
            .expect("second entry");
        assert_ne!(first.key, second.key);
    }

    #[test]
    fn source_entries_find_complete_entries_of_the_source() {
        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.bin");
        let other = root.path().join("other.bin");
        fs::write(&source, b"source").expect("write source");
        fs::write(&other, b"other").expect("write other");
        let cache = DerivativeCache::new(root.path());
        for (path, options) in [
            (&source, json!({"n": 1})),
            (&source, json!({"n": 2})),
            (&other, json!({"n": 1})),
        ] {
            let entry = cache.source_entry(path, "frames", &options).expect("entry");
            entry.mark_complete(&[]).expect("publish");
        }
        cache
            .source_entry(&source, "frames", &json!({"n": 3}))
            .expect("incomplete entry");
        cache
            .source_entry(&source, "waveform", &json!({"n": 1}))
            .expect("entry")
            .mark_complete(&[])
            .expect("publish");
        let entries = cache.source_entries(&source, "frames").expect("entries");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.is_complete(&[])));
    }

    #[test]
    fn a_changed_source_s_entries_go_once_its_new_one_completes() {
        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.bin");
        fs::write(&source, b"one").expect("write source");
        let cache = DerivativeCache::new(root.path());
        let old = cache
            .source_entry(&source, "proxy", &json!({}))
            .expect("entry");
        old.mark_complete(&[]).expect("publish");
        let other = cache
            .source_entry(&source, "frames", &json!({}))
            .expect("entry");
        other.mark_complete(&[]).expect("publish");
        fs::write(&source, b"replacement").expect("replace source");
        let new = cache
            .source_entry(&source, "proxy", &json!({}))
            .expect("entry");
        assert!(old.directory.is_dir(), "kept until the new one is made");
        new.mark_complete(&[]).expect("publish");
        assert!(!old.directory.exists());
        assert!(new.is_complete(&[]));
        assert!(other.directory.is_dir(), "another kind is its own");
    }

    #[test]
    fn an_entry_finished_after_its_source_changed_leaves_the_new_one() {
        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.bin");
        fs::write(&source, b"one").expect("write source");
        let cache = DerivativeCache::new(root.path());
        let late = cache
            .source_entry(&source, "proxy", &json!({}))
            .expect("entry");
        fs::write(&source, b"replacement").expect("replace source");
        let new = cache
            .source_entry(&source, "proxy", &json!({}))
            .expect("entry");
        new.mark_complete(&[]).expect("publish");
        late.mark_complete(&[]).expect("publish");
        assert!(new.is_complete(&[]), "made from the source as it is");
    }

    #[test]
    fn keep_newest_keeps_the_entries_used_last_and_what_was_made_from_them() {
        let root = tempdir().expect("temp directory");
        let cache = DerivativeCache::new(root.path());
        let revision = |n: u64| {
            let entry = cache
                .project_entry(&json!({"revision": n}), &[], "program-sound", &json!({}))
                .expect("entry");
            fs::write(entry.path("sound.mp4"), b"sound").expect("write artifact");
            entry.mark_complete(&["sound.mp4"]).expect("publish");
            entry
        };
        let set_used = |entry: &CacheEntry, seconds: u64| {
            fs::File::options()
                .write(true)
                .open(entry.path("manifest.json"))
                .and_then(|file| {
                    file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                })
                .expect("set time");
        };
        let entries: Vec<_> = (0..4).map(revision).collect();
        for (n, entry) in entries.iter().enumerate() {
            set_used(entry, 1_000 + n as u64);
        }
        // Made from the oldest revision's sound, as its transcript is.
        let transcript = cache
            .source_entry(&entries[0].path("sound.mp4"), "transcription", &json!({}))
            .expect("entry");
        transcript.mark_complete(&[]).expect("publish");
        // Another kind sharing the prefix is not one of them.
        let longer = cache
            .project_entry(&json!({}), &[], "program-sound-x", &json!({}))
            .expect("entry");
        // The second oldest was used again.
        set_used(&entries[1], 2_000);

        cache.keep_newest(&entries[0], 2).expect("evict");
        let kept: Vec<bool> = entries
            .iter()
            .map(|entry| entry.directory.is_dir())
            .collect();
        assert_eq!(kept, [true, true, false, true]);
        assert!(transcript.directory.is_dir(), "its sound is kept");

        cache.keep_newest(&entries[3], 1).expect("evict");
        let kept: Vec<bool> = entries
            .iter()
            .map(|entry| entry.directory.is_dir())
            .collect();
        assert_eq!(
            kept,
            [true, false, false, true],
            "the current one counts as used now"
        );
        cache.keep_newest(&entries[3], 0).expect("evict");
        assert!(!entries[0].directory.exists());
        assert!(!transcript.directory.exists(), "its sound is gone");
        assert!(longer.directory.is_dir());
        assert!(entries[3].is_complete(&["sound.mp4"]));
    }

    #[test]
    fn incomplete_entry_is_not_a_cache_hit() {
        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.bin");
        fs::write(&source, b"source").expect("write source");
        let cache = DerivativeCache::new(root.path());
        let entry = cache
            .source_entry(&source, "frames", &json!({"times":[0.0]}))
            .expect("entry");
        fs::write(entry.path("frame-000.png"), b"image").expect("write artifact");
        assert!(!entry.is_complete(&["frame-000.png"]));
        entry.mark_complete(&["frame-000.png"]).expect("publish");
        assert!(entry.is_complete(&["frame-000.png"]));
    }
}
