//! Project-local, content-addressed storage for regenerable media artifacts.
//!
//! The cache records source identity and normalized options in a small manifest
//! so an interrupted generation cannot be mistaken for a valid result. Cache
//! entries are never canonical project state.

use crate::error::{EngineError, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Relative location of regenerable media artifacts inside a project.
pub const DERIVED_CACHE_RELATIVE_PATH: &str = ".cut/cache/derived";
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
        Ok(())
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
