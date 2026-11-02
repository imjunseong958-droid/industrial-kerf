//! Durable runtime state on disk: the directory lock, the checksummed
//! journal, checkpoints, and atomic file replacement.

use crate::kernel;
use crate::{io_error, PendingProposal, RenderJob, RuntimeError, StateSnapshot};
use fs2::FileExt;
use kerf_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) const PROJECT_FILE: &str = "project.kerf";
pub(crate) const JOURNAL_FILE: &str = ".kerf-runtime.journal";
pub(crate) const LOCK_FILE: &str = ".kerf-runtime.lock";
pub(crate) const CHECKPOINT_FILE: &str = ".kerf-runtime.checkpoint.json";
pub(crate) const CHECKPOINT_TEMP_FILE: &str = ".kerf-runtime.checkpoint.json.tmp";

/// Where state written by an older kerfd is moved, untouched, on first open.
pub(crate) const PREVIOUS_STATE_PREFIX: &str = ".kerf-runtime.previous";
/// Format 5 keeps receipts without the state they left. Format 4, which
/// kept it, still reads.
pub(crate) const CHECKPOINT_VERSION: u64 = 5;
const OLDEST_CHECKPOINT_VERSION: u64 = 4;
/// Likewise, journal records of format 3 still read.
pub(crate) const JOURNAL_VERSION: u64 = 4;
const OLDEST_JOURNAL_VERSION: u64 = 3;

/// A project directory has exactly one canonical runtime owner.  Keeping the
/// lock open for the lifetime of the runtime prevents two desktop windows (or
/// two daemons) from concurrently advancing and overwriting the same history.
#[derive(Debug)]
pub(crate) struct RuntimeLock {
    pub(crate) _file: File,
}

impl RuntimeLock {
    pub(crate) fn acquire(directory: &Path) -> Result<Self, RuntimeError> {
        let path = directory.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error)?;
        file.try_lock_exclusive().map_err(|error| {
            // fs2 reports lock contention with a platform-specific OS error;
            // Windows uses ERROR_LOCK_VIOLATION rather than WouldBlock.
            let is_contention = matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::AlreadyExists
            ) || error.raw_os_error()
                == fs2::lock_contended_error().raw_os_error();
            if is_contention {
                RuntimeError::new(
                    "RUNTIME_ALREADY_RUNNING",
                    format!("another kerf runtime already owns {}", directory.display()),
                )
            } else {
                io_error(error)
            }
        })?;
        Ok(Self { _file: file })
    }
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

/// One undoable edit: how it reads, and the project on the other side of
/// it (before the edit in `past`, after it in `future`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub(crate) script: String,
    #[serde(with = "document_text")]
    pub(crate) project: Arc<Project>,
}

/// Projects in history and checkpoints are stored as `.kerf` text.
mod document_text {
    use super::kernel;
    use kerf_model::Project;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::sync::Arc;

    pub fn serialize<S: Serializer>(
        project: &Arc<Project>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&kernel::print(project))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Project>, D::Error> {
        let text = String::deserialize(deserializer)?;
        kernel::parse(&text)
            .map(Arc::new)
            .map_err(|error| serde::de::Error::custom(error.message))
    }
}

/// How a journaled mutation changed the project. Edits replay from their
/// script; projection imports carry the imported document. The digest of
/// the resulting document guards every replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Change {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) document: Option<String>,
    pub(crate) digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalAction {
    Apply,
    Undo,
    Redo,
    Noop,
    StageProposal,
    RejectProposal,
    Job,
    ProjectionImport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournalOperation {
    pub(crate) key: String,
    pub(crate) record: OperationRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournalImport {
    pub(crate) key: String,
    pub(crate) record: ImportRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JournalRecord {
    pub(crate) format_version: u64,
    pub(crate) sequence: u64,
    pub(crate) revision: u64,
    pub(crate) action: JournalAction,
    #[serde(default)]
    pub(crate) change: Option<Change>,
    #[serde(default)]
    pub(crate) operation: Option<JournalOperation>,
    #[serde(default)]
    pub(crate) import: Option<JournalImport>,
    #[serde(default)]
    pub(crate) proposal: Option<ProposalRecord>,
    #[serde(default)]
    pub(crate) job: Option<RenderJob>,
    pub(crate) checksum: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PersistedCheckpoint {
    pub(crate) format_version: u64,
    #[serde(with = "document_text")]
    pub(crate) project: Arc<Project>,
    pub(crate) revision: u64,
    #[serde(default)]
    pub(crate) past: Vec<HistoryEntry>,
    #[serde(default)]
    pub(crate) future: Vec<HistoryEntry>,
    #[serde(default)]
    pub(crate) journal_sequence: u64,
    pub(crate) operations: HashMap<String, OperationRecord>,
    pub(crate) imports: HashMap<String, ImportRecord>,
    #[serde(default)]
    pub(crate) proposal: Option<ProposalRecord>,
    #[serde(default)]
    pub(crate) jobs: HashMap<String, RenderJob>,
    #[serde(default)]
    pub(crate) import_warnings: Vec<String>,
    #[serde(default)]
    pub(crate) projection_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionStatus {
    Current,
    Missing,
    Diverged,
}

impl ProjectionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Missing => "missing",
            Self::Diverged => "diverged",
        }
    }
}

/// The receipt of a keyed request: what it asked, and the revision it
/// left. A retry is answered with the state as it is now, so a receipt
/// stays small whatever the size of the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperationRecord {
    pub(crate) fingerprint: String,
    /// The whole state the request left, which receipts kept before format
    /// 5. It is read so their journal records still verify, then dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<Arc<StateSnapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
}

impl OperationRecord {
    pub(crate) fn new(fingerprint: String, revision: u64) -> Self {
        Self {
            fingerprint,
            snapshot: None,
            revision: Some(revision),
        }
    }

    /// The revision the request left.
    pub(crate) fn revision(&self) -> u64 {
        receipt_revision(self.revision, self.snapshot.as_deref())
    }

    /// The receipt without the snapshot an older format kept.
    pub(crate) fn settled(mut self) -> Self {
        self.revision = Some(self.revision());
        self.snapshot = None;
        self
    }
}

/// The receipt of a keyed import, with the media and clip it added.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImportRecord {
    pub(crate) fingerprint: String,
    /// As in [`OperationRecord`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<Arc<StateSnapshot>>,
    pub(crate) media_id: String,
    pub(crate) clip_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
}

impl ImportRecord {
    pub(crate) fn new(
        fingerprint: String,
        revision: u64,
        media_id: String,
        clip_id: String,
    ) -> Self {
        Self {
            fingerprint,
            snapshot: None,
            media_id,
            clip_id,
            revision: Some(revision),
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        receipt_revision(self.revision, self.snapshot.as_deref())
    }

    pub(crate) fn settled(mut self) -> Self {
        self.revision = Some(self.revision());
        self.snapshot = None;
        self
    }
}

fn receipt_revision(revision: Option<u64>, snapshot: Option<&StateSnapshot>) -> u64 {
    revision
        .or_else(|| snapshot.map(|snapshot| snapshot.revision))
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProposalReceipt {
    pub(crate) actor: String,
    pub(crate) key: String,
    pub(crate) base_revision: u64,
    pub(crate) snapshot: Arc<StateSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProposalRecord {
    pub(crate) lifecycle: PendingProposal,
    pub(crate) request_key: String,
    pub(crate) fingerprint: String,
    #[serde(default)]
    pub(crate) receipt: Option<ProposalReceipt>,
}

/// Move an older kerfd's checkpoint and journal aside, unchanged.
pub(crate) fn preserve_previous_state(directory: &Path) -> Result<(), RuntimeError> {
    for (from, suffix) in [
        (CHECKPOINT_FILE, "checkpoint.json"),
        (CHECKPOINT_TEMP_FILE, "checkpoint.json.tmp"),
        (JOURNAL_FILE, "journal"),
    ] {
        let from = directory.join(from);
        if from.exists() {
            let to = directory.join(format!("{PREVIOUS_STATE_PREFIX}.{suffix}"));
            fs::rename(&from, &to).map_err(io_error)?;
        }
    }
    sync_parent_dir(directory);
    Ok(())
}

pub(crate) fn seal_journal_record(
    mut record: JournalRecord,
) -> Result<JournalRecord, RuntimeError> {
    record.checksum.clear();
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    record.checksum = sha256_hex(&bytes);
    Ok(record)
}

fn verify_journal_record(record: &JournalRecord) -> Result<(), RuntimeError> {
    if !(OLDEST_JOURNAL_VERSION..=JOURNAL_VERSION).contains(&record.format_version) {
        return Err(RuntimeError::new(
            "JOURNAL_VERSION_UNSUPPORTED",
            format!(
                "journal record format {} is not supported",
                record.format_version
            ),
        ));
    }
    let mut unsigned = record.clone();
    let checksum = unsigned.checksum.clone();
    unsigned.checksum.clear();
    let bytes = serde_json::to_vec(&unsigned)
        .map_err(|error| RuntimeError::new("INVALID_JOURNAL", error.to_string()))?;
    if checksum != sha256_hex(&bytes) {
        return Err(RuntimeError::new(
            "JOURNAL_CHECKSUM_MISMATCH",
            format!(
                "journal record {} failed checksum verification",
                record.sequence
            ),
        ));
    }
    Ok(())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn append_journal_record(
    path: &Path,
    record: &JournalRecord,
) -> Result<(), RuntimeError> {
    let bytes = serde_json::to_vec(record)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.write_all(b"\n").map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

pub(crate) fn load_journal(
    path: &Path,
    after_sequence: u64,
) -> Result<Vec<JournalRecord>, RuntimeError> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        let newline = bytes[offset..].iter().position(|byte| *byte == b'\n');
        let (end, terminated) = match newline {
            Some(index) => (offset + index, true),
            None => (bytes.len(), false),
        };
        let line = &bytes[start..end];
        offset = if terminated { end + 1 } else { end };
        if line.iter().all(u8::is_ascii_whitespace) {
            if !terminated {
                file.set_len(start as u64).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
            }
            break;
        }
        let parsed = serde_json::from_slice::<JournalRecord>(line);
        let record = match parsed {
            Ok(record) => record,
            Err(error) if offset == bytes.len() && !terminated => {
                // A process can die after writing only part of the final JSON
                // line. The valid prefix remains authoritative; discard only
                // this torn tail and keep all earlier records.
                file.set_len(start as u64).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
                let _ = error;
                break;
            }
            Err(error) => {
                return Err(RuntimeError::new(
                    "INVALID_JOURNAL",
                    format!("journal record at byte {start} is invalid: {error}"),
                ));
            }
        };
        if let Err(error) = verify_journal_record(&record) {
            if offset == bytes.len() && !terminated {
                file.set_len(start as u64).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
                break;
            }
            return Err(error);
        }
        if record.sequence <= after_sequence {
            continue;
        }
        records.push(record);
    }
    Ok(records)
}

pub(crate) fn truncate_journal(path: &Path) -> Result<(), RuntimeError> {
    if !path.exists() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

/// Whether `project.kerf` holds `expected`. Formatting and comments do not
/// matter; the parsed project does.
pub(crate) fn inspect_projection(path: &Path, expected: &Project) -> ProjectionStatus {
    if !path.is_file() {
        return ProjectionStatus::Missing;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return ProjectionStatus::Diverged;
    };
    match kernel::parse(&text) {
        Ok(project) if project == *expected => ProjectionStatus::Current,
        _ => ProjectionStatus::Diverged,
    }
}

pub(crate) fn projection_matches(path: &Path, expected: &Project) -> bool {
    inspect_projection(path, expected) == ProjectionStatus::Current
}

pub(crate) fn projection_digest(path: &Path) -> Result<String, RuntimeError> {
    let bytes = fs::read(path).map_err(io_error)?;
    Ok(sha256_hex(&bytes))
}

/// What a directory's checkpoint holds.
pub(crate) enum Loaded {
    None,
    Current(Box<PersistedCheckpoint>),
    /// A checkpoint written by an older kerfd.
    Previous,
}

pub(crate) fn load_checkpoint(directory: &Path) -> Result<Loaded, RuntimeError> {
    let checkpoint_path = directory.join(CHECKPOINT_FILE);
    let temporary_path = directory.join(CHECKPOINT_TEMP_FILE);
    for path in [&checkpoint_path, &temporary_path] {
        if path.is_file()
            && checkpoint_format(path).is_some_and(|format| format < OLDEST_CHECKPOINT_VERSION)
        {
            return Ok(Loaded::Previous);
        }
    }
    if !checkpoint_path.is_file() && temporary_path.is_file() {
        let value = read_checkpoint(&temporary_path)?;
        replace_file(&temporary_path, &checkpoint_path)?;
        sync_parent_dir(directory);
        return Ok(Loaded::Current(Box::new(value)));
    }
    if checkpoint_path.is_file() {
        match read_checkpoint(&checkpoint_path) {
            Ok(value) => {
                // A completed checkpoint supersedes an abandoned write left
                // by a crash.  The temp path is explicit and safe to remove.
                if temporary_path.is_file() {
                    let _ = fs::remove_file(temporary_path);
                }
                return Ok(Loaded::Current(Box::new(value)));
            }
            Err(canonical_error) if temporary_path.is_file() => {
                // A manually interrupted/corrupted canonical file can still
                // be recovered from the last complete atomic temp write.
                let value = read_checkpoint(&temporary_path).map_err(|temporary_error| {
                    RuntimeError::new(
                        "INVALID_CHECKPOINT",
                        format!(
                            "canonical checkpoint failed: {}; temp checkpoint failed: {}",
                            canonical_error.message, temporary_error.message
                        ),
                    )
                })?;
                replace_file(&temporary_path, &checkpoint_path)?;
                sync_parent_dir(directory);
                return Ok(Loaded::Current(Box::new(value)));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Loaded::None)
}

/// The format version a checkpoint file declares, if it is readable.
fn checkpoint_format(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("format_version")
        .or_else(|| value.get("formatVersion"))
        .and_then(Value::as_u64)
}

fn read_checkpoint(path: &Path) -> Result<PersistedCheckpoint, RuntimeError> {
    let text = fs::read_to_string(path).map_err(io_error)?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| RuntimeError::new("INVALID_CHECKPOINT", error.to_string()))?;
    let format_version = value
        .get("format_version")
        .or_else(|| value.get("formatVersion"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            RuntimeError::new(
                "INVALID_CHECKPOINT",
                "checkpoint format_version is required",
            )
        })?;
    if !(OLDEST_CHECKPOINT_VERSION..=CHECKPOINT_VERSION).contains(&format_version) {
        return Err(RuntimeError::new(
            "CHECKPOINT_VERSION_UNSUPPORTED",
            format!("checkpoint format {format_version} is not supported"),
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::new("INVALID_CHECKPOINT", error.to_string()))
}

pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), RuntimeError> {
    // Compact: a checkpoint is written on the actor's thread, and nobody
    // reads it by eye.
    let bytes = serde_json::to_vec(value)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    write_bytes_atomic(path, &bytes)
}

/// Write through a synced temporary file and an atomic rename.
pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
    let temporary_path = atomic_temp_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary_path)
            .map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        replace_file(&temporary_path, path)?;
        if let Some(parent) = path.parent() {
            sync_parent_dir(parent);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn atomic_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    path.with_file_name(format!("{file_name}.tmp"))
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, destination: &Path) -> Result<(), RuntimeError> {
    fs::rename(temporary_path, destination).map_err(io_error)
}

#[cfg(windows)]
fn replace_file(temporary_path: &Path, destination: &Path) -> Result<(), RuntimeError> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // MoveFileExW with REPLACE_EXISTING and WRITE_THROUGH is the Windows
    // equivalent of the same-volume atomic replacement used by rename(2).
    // The pointers remain valid for the duration of the synchronous call.
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;

#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

fn sync_parent_dir(directory: &Path) {
    // Directory fsync is available on Unix and harmlessly unsupported on a
    // few platforms.  The file itself is always synced before its rename.
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}
