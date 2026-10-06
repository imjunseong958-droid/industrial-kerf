//! Desktop-owned editing runtime.
//!
//! This is deliberately a semantic boundary.  UI and MCP clients send project
//! commands and receive snapshots; they never mutate a local copy of the
//! timeline. `cutd` owns one `RuntimeHandle` and serves those clients through
//! the project-scoped IPC transport below.
//!
//! The canonical project is a `cut_model::Project` stored as the `.cut`
//! document `project.cut` (ADR 0006). Edits arrive as scripts or typed
//! operations (see `kernel`) and are journaled as scripts. Snapshots carry
//! the document and a timeline view of the model; proposals carry the script
//! and the document diff. Renders and proposal previews hand renderers the
//! document with media paths resolved.

mod kernel;
mod timeline;

pub use kernel::{resolve_media_paths, EditOp, TrimSide};

use cut_engine::derivatives::SourceIdentity;
use cut_model::Project;
use fs2::FileExt;
use kernel::{Edit, Edited};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

const PROJECT_FILE: &str = "project.cut";
/// The v1/v2 project file, read once to import a project and never written.
const LEGACY_PROJECT_FILE: &str = "project.cut.json";
const JOURNAL_FILE: &str = ".cut-runtime.journal";
const LOCK_FILE: &str = ".cut-runtime.lock";
const CHECKPOINT_FILE: &str = ".cut-runtime.checkpoint.json";
const CHECKPOINT_TEMP_FILE: &str = ".cut-runtime.checkpoint.json.tmp";
/// Where state written by an older cutd is moved, untouched, on first open.
const PREVIOUS_STATE_PREFIX: &str = ".cut-runtime.previous";
const CHECKPOINT_VERSION: u64 = 4;
const JOURNAL_VERSION: u64 = 3;
const CHECKPOINT_INTERVAL: u64 = 32;
/// History holds project snapshots, so it is bounded more tightly than the
/// command history it replaced.
const MAX_HISTORY_ENTRIES: usize = 100;
/// `StateSnapshot::source_version` of the canonical `.cut` document; 1 and 2
/// were the `project.cut.json` formats.
const SOURCE_VERSION: u8 = 3;
const MAX_RECEIPTS: usize = 1024;
/// Maximum duration of one bounded rendered-preview request.
pub const MAX_RENDER_RANGE_SECONDS: f64 = 120.0;
const RENDER_CANCEL_FILE_PREFIX: &str = ".cut-render-";

/// The largest JSONL request accepted by the local IPC boundary.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Conservative limit for a Unix-domain socket path.  Linux permits 108
/// bytes and macOS permits fewer bytes on some SDKs, so leave room for the
/// terminating NUL and use the same threshold on both platforms.
pub const UNIX_SOCKET_PATH_LIMIT: usize = 100;
const SOCKET_FILE: &str = ".cutd.sock";
const CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
static TEST_RESPONSE_DROPPED: AtomicBool = AtomicBool::new(false);

/// Address for the project-scoped local runtime endpoint.
///
/// Unix is the implemented transport in this unit.  The named-pipe variant
/// is part of the public address model so callers cannot accidentally claim
/// Windows support without implementing its transport and credential checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointAddress {
    Unix { path: PathBuf },
    WindowsNamedPipe { name: String },
}

impl EndpointAddress {
    fn for_canonical_directory(directory: &Path) -> Self {
        #[cfg(unix)]
        {
            let direct = directory.join(SOCKET_FILE);
            if unix_path_len(&direct) < UNIX_SOCKET_PATH_LIMIT {
                return Self::Unix { path: direct };
            }
            return Self::Unix {
                path: user_runtime_directory()
                    .join(format!("cutd-{}.sock", project_directory_hash(directory))),
            };
        }

        #[cfg(windows)]
        {
            // Keep the namespace stable and independent of path length.  The
            // named pipe transport is intentionally not enabled until its
            // server/client implementation and ACL checks land.
            return Self::WindowsNamedPipe {
                name: format!(r"\\.\pipe\cutd-{}", project_directory_hash(directory)),
            };
        }

        #[allow(unreachable_code)]
        Self::WindowsNamedPipe {
            name: format!("cutd-{}", project_directory_hash(directory)),
        }
    }

    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::Unix { path } => Some(path),
            Self::WindowsNamedPipe { .. } => None,
        }
    }
}

fn canonical_project_directory(directory: &Path) -> Result<PathBuf, RuntimeError> {
    fs::create_dir_all(directory).map_err(io_error)?;
    directory.canonicalize().map_err(io_error)
}

/// Return the project-local cancellation marker for one durable render job.
///
/// The marker is an execution signal, not project data. It lets a worker in a
/// separate client process deliver cancellation to the exact engine process
/// that owns the job's FFmpeg child.
pub fn render_cancel_path(directory: &Path, job_id: &str) -> PathBuf {
    directory.join(format!("{RENDER_CANCEL_FILE_PREFIX}{job_id}.cancel"))
}

#[cfg(unix)]
fn unix_path_len(path: &Path) -> usize {
    path.as_os_str().as_encoded_bytes().len()
}

fn project_directory_hash(directory: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(directory.as_os_str().to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn user_runtime_directory() -> PathBuf {
    let directory = std::env::var_os("CUT_RUNTIME_DIR")
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            {
                // macOS TMPDIR is already a private per-user runtime root;
                // adding another component can exceed sockaddr_un's limit.
                std::env::temp_dir()
            }
            #[cfg(not(target_os = "macos"))]
            {
                let uid = current_user_id()
                    .map(|uid| uid.to_string())
                    .unwrap_or_else(|| "user".into());
                std::env::temp_dir().join(format!("cut-runtime-{uid}"))
            }
        });
    let directory = if directory.is_absolute() {
        directory
    } else {
        std::env::current_dir()
            .map(|current| current.join(&directory))
            .unwrap_or(directory)
    };
    let _ = fs::create_dir_all(&directory);
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700));
    }
    directory
}

#[cfg(all(unix, not(target_os = "macos")))]
fn current_user_id() -> Option<u32> {
    // `geteuid` has no Rust wrapper on the supported Unix targets and does not
    // dereference pointers or depend on process-owned state.
    Some(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn current_user_id() -> Option<u32> {
    None
}

/// A project directory has exactly one canonical runtime owner.  Keeping the
/// lock open for the lifetime of the runtime prevents two desktop windows (or
/// two daemons) from concurrently advancing and overwriting the same history.
#[derive(Debug)]
struct RuntimeLock {
    _file: File,
}

impl RuntimeLock {
    fn acquire(directory: &Path) -> Result<Self, RuntimeError> {
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
                    format!("another cut runtime already owns {}", directory.display()),
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
struct HistoryEntry {
    script: String,
    #[serde(with = "document_text")]
    project: Arc<Project>,
}

/// Projects in history and checkpoints are stored as `.cut` text.
mod document_text {
    use super::kernel;
    use cut_model::Project;
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
struct Change {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document: Option<String>,
    digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalAction {
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
struct JournalOperation {
    key: String,
    record: OperationRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalImport {
    key: String,
    record: ImportRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalRecord {
    format_version: u64,
    sequence: u64,
    revision: u64,
    action: JournalAction,
    #[serde(default)]
    change: Option<Change>,
    #[serde(default)]
    operation: Option<JournalOperation>,
    #[serde(default)]
    import: Option<JournalImport>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
    #[serde(default)]
    job: Option<RenderJob>,
    checksum: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCheckpoint {
    format_version: u64,
    #[serde(with = "document_text")]
    project: Arc<Project>,
    revision: u64,
    #[serde(default)]
    past: Vec<HistoryEntry>,
    #[serde(default)]
    future: Vec<HistoryEntry>,
    #[serde(default)]
    journal_sequence: u64,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
    #[serde(default)]
    jobs: HashMap<String, RenderJob>,
    #[serde(default)]
    import_warnings: Vec<String>,
    #[serde(default)]
    projection_digest: Option<String>,
}

fn default_source_version() -> u8 {
    SOURCE_VERSION
}

fn default_projection_status() -> String {
    "current".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionStatus {
    Current,
    Missing,
    Diverged,
}

impl ProjectionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Missing => "missing",
            Self::Diverged => "diverged",
        }
    }
}

/// What clients see of the runtime. `document` is the canonical `.cut`
/// text and `timeline` a JSON view of its first sequence for editors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub document: String,
    pub timeline: Value,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub duration: f64,
    pub pending_proposal: Option<PendingProposal>,
    /// 3 for the canonical `.cut` document.
    #[serde(default = "default_source_version")]
    pub source_version: u8,
    #[serde(default = "default_projection_status")]
    pub projection_status: String,
    /// What the one-time import of `project.cut.json` changed or left
    /// behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_warnings: Vec<String>,
}

/// A finite time interval of an observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalTimeRange {
    pub start: f64,
    pub end: f64,
}

/// A bounded media observation that can be attached to an agent proposal.
///
/// Observation artifacts remain in the regenerable cache. The proposal stores
/// only this compact reference so reviewers can trace why a change was made
/// without putting media bytes or transcript payloads in canonical project
/// state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationReference {
    pub id: String,
    pub source_ref: String,
    pub source_path: String,
    pub source_identity: ObservationSourceIdentity,
    pub range: ProposalTimeRange,
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_keys: Vec<String>,
    /// Revision at which the observation was taken, when tied to a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_range: Option<ProposalTimeRange>,
    /// Index of the selected range within one rough-cut request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_index: Option<usize>,
    /// Agent-provided reason for selecting this range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Optional coarse observation that led to this targeted observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coarse_observation: Option<Box<ObservationReference>>,
}

/// File identity carried by an [`ObservationReference`] for cache
/// invalidation and provenance display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSourceIdentity {
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ns: Option<u64>,
}

impl ObservationReference {
    /// Validate the bounded metadata accepted into a durable proposal.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.source_ref.trim().is_empty()
            || self.source_path.trim().is_empty()
        {
            return Err("observation reference requires source and id".into());
        }
        if !self.range.start.is_finite()
            || !self.range.end.is_finite()
            || self.range.start < 0.0
            || self.range.end <= self.range.start
            || self.range.end - self.range.start > 120.0 + 1e-9
        {
            return Err("observation reference range is invalid or exceeds 120 seconds".into());
        }
        if self.evidence.is_empty() || self.evidence.len() > 5 {
            return Err("observation reference evidence must contain 1..=5 items".into());
        }
        if self.selection_index.is_some_and(|index| index >= 16) {
            return Err("observation reference selection index is out of bounds".into());
        }
        if self
            .rationale
            .as_deref()
            .is_some_and(|rationale| rationale.trim().is_empty())
        {
            return Err("observation reference rationale must not be empty".into());
        }
        if let Some(timeline_range) = &self.timeline_range {
            if !timeline_range.start.is_finite()
                || !timeline_range.end.is_finite()
                || timeline_range.start < 0.0
                || timeline_range.end <= timeline_range.start
            {
                return Err("observation reference timeline range is invalid".into());
            }
        }
        if let Some(coarse_observation) = &self.coarse_observation {
            coarse_observation.validate()?;
        }
        Ok(())
    }
}

/// Whether a pending proposal still targets the canonical project revision.
///
/// Older checkpoints omit this field and therefore deserialize as `Current`.
/// Runtime snapshots always recompute it from the proposal base revision so
/// every adapter receives the same explicit stale state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalFreshness {
    #[default]
    Current,
    Stale,
}

impl ProposalFreshness {
    fn is_current(value: &Self) -> bool {
        *value == Self::Current
    }
}

/// Proposal lifecycle owned by the canonical runtime.  Terminal states stay
/// in memory so repeating an accept/reject request converges without another
/// project mutation; only `Pending` exposes the proposed script and diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PendingProposal {
    Pending {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        /// The proposed edit as an operation script.
        script: String,
        /// Unified diff of the canonical document before and after.
        #[serde(rename = "documentDiff", alias = "document_diff")]
        document_diff: String,
        #[serde(default, skip_serializing_if = "ProposalFreshness::is_current")]
        freshness: ProposalFreshness,
        /// Optional evidence reference produced by the agent workflow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observation: Option<ObservationReference>,
        /// All evidence references produced for a multi-range agent proposal.
        /// The singular field remains for compatibility with older clients.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observations: Option<Vec<ObservationReference>>,
    },
    Accepted {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        #[serde(rename = "appliedRevision", alias = "applied_revision")]
        applied_revision: u64,
        /// Preserve the evidence reference after a proposal is accepted so
        /// durable receipts retain the reason for the canonical edit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observation: Option<ObservationReference>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observations: Option<Vec<ObservationReference>>,
    },
    Rejected {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
    },
}

/// A bounded project-time range rendered for a preview job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRange {
    pub start: f64,
    pub duration: f64,
}

impl RenderRange {
    /// Validate a positive, bounded project-time range.
    pub fn new(start: f64, duration: f64) -> Result<Self, RuntimeError> {
        if !start.is_finite()
            || start < 0.0
            || !duration.is_finite()
            || duration <= 0.0
            || duration > MAX_RENDER_RANGE_SECONDS
            || !(start + duration).is_finite()
        {
            return Err(RuntimeError::new(
                "INVALID_RENDER_RANGE",
                format!(
                    "render range requires start >= 0, 0 < duration <= {MAX_RENDER_RANGE_SECONDS}s"
                ),
            ));
        }
        Ok(Self { start, duration })
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

fn default_render_progress() -> f64 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderJob {
    pub job_id: String,
    pub status: String,
    pub output_url: Option<String>,
    pub output_path: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub range: Option<RenderRange>,
    #[serde(default = "default_render_progress")]
    pub progress: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
    pub expected_revision: Option<u64>,
    pub actual_revision: Option<u64>,
}

impl RuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            expected_revision: None,
            actual_revision: None,
        }
    }

    pub fn conflict(expected: u64, actual: u64) -> Self {
        Self {
            code: "REVISION_CONFLICT".into(),
            message: format!("revision conflict: expected {expected}, actual {actual}"),
            expected_revision: Some(expected),
            actual_revision: Some(actual),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperationRecord {
    fingerprint: String,
    snapshot: Arc<StateSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportRecord {
    fingerprint: String,
    snapshot: Arc<StateSnapshot>,
    media_id: String,
    clip_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalReceipt {
    actor: String,
    key: String,
    base_revision: u64,
    snapshot: Arc<StateSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalRecord {
    lifecycle: PendingProposal,
    request_key: String,
    fingerprint: String,
    #[serde(default)]
    receipt: Option<ProposalReceipt>,
}

#[allow(clippy::large_enum_variant)]
enum JournalEvent {
    Mutation {
        action: JournalAction,
        change: Option<Change>,
        operation: Option<JournalOperation>,
        import: Option<JournalImport>,
        proposal: Option<ProposalRecord>,
    },
    Proposal {
        proposal: Option<ProposalRecord>,
    },
    Job {
        job: RenderJob,
    },
    ProjectionImport {
        change: Change,
        operation: Option<JournalOperation>,
    },
}

impl JournalRecord {
    fn from_event(sequence: u64, revision: u64, event: JournalEvent) -> Self {
        let mut record = Self {
            format_version: JOURNAL_VERSION,
            sequence,
            revision,
            action: JournalAction::Noop,
            change: None,
            operation: None,
            import: None,
            proposal: None,
            job: None,
            checksum: String::new(),
        };
        match event {
            JournalEvent::Mutation {
                action,
                change,
                operation,
                import,
                proposal,
            } => {
                record.action = action;
                record.change = change;
                record.operation = operation;
                record.import = import;
                record.proposal = proposal;
            }
            JournalEvent::Proposal { proposal } => {
                record.action = JournalAction::StageProposal;
                record.proposal = proposal;
            }
            JournalEvent::Job { job } => {
                record.action = JournalAction::Job;
                record.job = Some(job);
            }
            JournalEvent::ProjectionImport { change, operation } => {
                record.action = JournalAction::ProjectionImport;
                record.change = Some(change);
                record.operation = operation;
            }
        }
        record
    }
}

#[derive(Debug, Clone)]
struct DurableState {
    project: Project,
    revision: u64,
    past: Vec<HistoryEntry>,
    future: Vec<HistoryEntry>,
    jobs: HashMap<String, RenderJob>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    proposal: Option<ProposalRecord>,
}

impl DurableState {
    /// Keep `snapshot` as the receipt of `operation` and return the journal
    /// entry that carries it.
    fn record_operation(
        &mut self,
        operation: String,
        fingerprint: String,
        snapshot: StateSnapshot,
    ) -> JournalOperation {
        let record = OperationRecord {
            fingerprint,
            snapshot: Arc::new(snapshot),
        };
        self.operations.insert(operation.clone(), record.clone());
        JournalOperation {
            key: operation,
            record,
        }
    }
}

#[derive(Debug)]
struct Runtime {
    _lock: RuntimeLock,
    project_path: PathBuf,
    journal_path: PathBuf,
    /// What a commit replaces as one unit.
    state: DurableState,
    import_warnings: Vec<String>,
    journal_sequence: u64,
    journal_records_since_checkpoint: u64,
    projection_status: ProjectionStatus,
    projection_digest: Option<String>,
}

/// Versioned operations accepted by the runtime mailbox.  JSON/IPC payloads
/// are parsed into this enum at the protocol boundary before they reach the
/// reducer, so the actor never branches on unvalidated field maps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RuntimeRequest {
    GetState,
    /// The canonical `.cut` document, optionally annotated with end times
    /// and source outs.
    GetDocument {
        #[serde(default)]
        annotated: bool,
    },
    /// The documents before and after a pending proposal, with media paths
    /// resolved for a renderer.
    GetProposalDocuments {
        #[serde(rename = "proposalId")]
        proposal_id: String,
    },
    /// Apply an edit script atomically at `baseRevision`.
    ApplyScript {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        script: String,
    },
    /// Apply typed operations atomically at `baseRevision`.
    ApplyOps {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        ops: Vec<EditOp>,
    },
    /// Stage an agent's edit script as a pending proposal.
    StageScriptProposal {
        #[serde(rename = "requestKey")]
        request_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        script: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        observations: Vec<Value>,
    },
    Undo {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
    },
    Redo {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
    },
    Import {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        path: String,
    },
    StartRender {
        quality: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration: Option<f64>,
        /// Render what this pending proposal would make of the project.
        #[serde(
            rename = "proposalId",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        proposal_id: Option<String>,
    },
    GetRender {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    UpdateJob {
        #[serde(rename = "jobId")]
        job_id: String,
        status: String,
        error: Option<String>,
        #[serde(rename = "outputUrl")]
        output_url: Option<String>,
        #[serde(default)]
        progress: Option<f64>,
    },
    CancelRender {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    RepairProjection,
    ImportProjection {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
    },
    AcceptProposal {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
    },
    RejectProposal {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        #[serde(
            rename = "idempotencyKey",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        idempotency_key: Option<String>,
        #[serde(
            rename = "baseRevision",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        base_revision: Option<u64>,
    },
}

/// Transport envelope used by `cutd`/`cut-runtime` JSONL clients.  It keeps
/// the request identifier and protocol version outside the domain operation.
#[derive(Debug, Clone)]
pub struct RequestEnvelope {
    pub protocol_version: u64,
    pub id: String,
    pub request: RuntimeRequest,
}

impl RequestEnvelope {
    pub const PROTOCOL_VERSION: u64 = 1;

    pub fn new(id: impl Into<String>, request: RuntimeRequest) -> Self {
        Self {
            protocol_version: Self::PROTOCOL_VERSION,
            id: id.into(),
            request,
        }
    }

    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.protocol_version != Self::PROTOCOL_VERSION {
            return Err(RuntimeError::new(
                "PROTOCOL_VERSION_UNSUPPORTED",
                "protocolVersion must be 1",
            ));
        }
        if self.id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "request id must be a non-empty string",
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for RequestEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("request must be an object"))?;
        let protocol_version = object
            .remove("protocolVersion")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| serde::de::Error::custom("protocolVersion must be an integer"))?;
        let id = object
            .remove("id")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("id must be a string"))?;
        let request = serde_json::from_value(Value::Object(std::mem::take(object)))
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            protocol_version,
            id,
            request,
        })
    }
}

impl Serialize for RequestEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut object = serde_json::to_value(&self.request)
            .map_err(serde::ser::Error::custom)?
            .as_object()
            .cloned()
            .ok_or_else(|| serde::ser::Error::custom("request must serialize as an object"))?;
        object.insert(
            "protocolVersion".into(),
            Value::Number(self.protocol_version.into()),
        );
        object.insert("id".into(), Value::String(self.id.clone()));
        Value::Object(object).serialize(serializer)
    }
}

/// Typed result of one serialized mailbox operation.
#[derive(Debug, Clone)]
pub enum RuntimeResponse {
    State(StateSnapshot),
    Document {
        revision: u64,
        document: String,
    },
    ProposalDocuments {
        before: String,
        after: String,
    },
    /// A media import and the media and clip it added.
    Import {
        state: StateSnapshot,
        media_id: String,
        clip_id: String,
    },
    /// A queued render and the document to render, with media paths
    /// resolved.
    RenderStarted {
        job: RenderJob,
        document: String,
        output: PathBuf,
    },
    RenderJob(RenderJob),
    Empty,
}

struct MailboxMessage {
    request: RuntimeRequest,
    reply: Sender<Result<RuntimeResponse, RuntimeError>>,
}

/// A cloneable handle to the one runtime actor that owns project state,
/// history, jobs, and persistence.  Calls block only until this operation is
/// applied by the actor; concurrent callers are serialized by the mailbox.
#[derive(Clone)]
pub struct RuntimeHandle {
    sender: Sender<MailboxMessage>,
}

impl std::fmt::Debug for RuntimeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeHandle")
            .finish_non_exhaustive()
    }
}

impl RuntimeHandle {
    pub fn open(directory: &Path) -> Result<Self, RuntimeError> {
        let directory = directory.to_path_buf();
        let (sender, receiver) = mpsc::channel::<MailboxMessage>();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("cut-runtime".into())
            .spawn(move || runtime_actor(directory, receiver, ready_sender))
            .map_err(io_error)?;
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self { sender }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(RuntimeError::new(
                "RUNTIME_UNAVAILABLE",
                "runtime actor exited during startup",
            )),
        }
    }

    pub fn request(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send(MailboxMessage { request, reply })
            .map_err(|_| RuntimeError::new("RUNTIME_UNAVAILABLE", "runtime actor is stopped"))?;
        response.recv().map_err(|_| {
            RuntimeError::new("RUNTIME_UNAVAILABLE", "runtime actor dropped the response")
        })?
    }
}

/// Request-scoped client for the project daemon.
///
/// The client owns no project state and never owns the daemon process.  Every
/// request opens a fresh local connection, which keeps a disconnected client
/// from affecting other clients or jobs.  When a binary is configured, the
/// first request that cannot connect starts it and waits for the endpoint to
/// become ready.
#[derive(Clone)]
pub struct RuntimeClient {
    inner: Arc<RuntimeClientInner>,
}

struct RuntimeClientInner {
    directory: PathBuf,
    endpoint: EndpointAddress,
    binary: Option<PathBuf>,
    request_timeout: Duration,
    startup_timeout: Duration,
}

impl std::fmt::Debug for RuntimeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeClient")
            .field("directory", &self.inner.directory)
            .field("endpoint", &self.inner.endpoint)
            .finish_non_exhaustive()
    }
}

impl RuntimeClient {
    /// Create a client that connects only to an already-running daemon.
    pub fn new(directory: &Path) -> Result<Self, RuntimeError> {
        Self::with_binary(directory, None)
    }

    /// Create a client that connects first and starts `binary` only when no
    /// daemon is listening.  The spawned process is intentionally detached
    /// from the client lifetime; dropping this value never kills `cutd`.
    pub fn connect_or_spawn(directory: &Path, binary: Option<&Path>) -> Result<Self, RuntimeError> {
        Self::with_binary(directory, binary)
    }

    fn with_binary(directory: &Path, binary: Option<&Path>) -> Result<Self, RuntimeError> {
        let directory = canonical_project_directory(directory)?;
        let endpoint = EndpointAddress::for_canonical_directory(&directory);
        Ok(Self {
            inner: Arc::new(RuntimeClientInner {
                directory,
                endpoint,
                binary: binary.map(Path::to_path_buf),
                request_timeout: CLIENT_REQUEST_TIMEOUT,
                startup_timeout: CLIENT_STARTUP_TIMEOUT,
            }),
        })
    }

    /// Return the canonical project directory used by this client.
    pub fn directory(&self) -> &Path {
        &self.inner.directory
    }

    /// Return the derived project-scoped endpoint.
    pub fn endpoint(&self) -> &EndpointAddress {
        &self.inner.endpoint
    }

    /// Send one versioned request over a fresh connection.
    pub fn request(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let retryable = request_has_stable_idempotency_key(&request);
        match self.request_once(&request) {
            Ok(response) => Ok(response),
            Err(error) if retryable && is_transport_error(&error) => {
                // The first request may have committed before its response
                // was lost.  Re-read authoritative state before retrying so
                // the caller observes a current revision and the retry can
                // safely resolve through the durable receipt.
                self.request_once(&RuntimeRequest::GetState)
                    .map_err(|_| error.clone())?;
                self.request_once(&request)
            }
            Err(error) => Err(error),
        }
    }

    /// The documents before and after a pending proposal, with media paths
    /// resolved for a renderer.
    pub fn proposal_documents(&self, proposal_id: &str) -> Result<(String, String), RuntimeError> {
        match self.request(RuntimeRequest::GetProposalDocuments {
            proposal_id: proposal_id.to_owned(),
        })? {
            RuntimeResponse::ProposalDocuments { before, after } => Ok((before, after)),
            _ => Err(unexpected_response("proposal-documents")),
        }
    }

    pub fn get_state(&self) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::GetState)?)
    }

    /// The canonical `.cut` document and the revision it describes.
    pub fn get_document(&self, annotated: bool) -> Result<(u64, String), RuntimeError> {
        match self.request(RuntimeRequest::GetDocument { annotated })? {
            RuntimeResponse::Document { revision, document } => Ok((revision, document)),
            _ => Err(unexpected_response("document")),
        }
    }

    pub fn apply_script_request(
        &self,
        actor: &str,
        key: &str,
        script: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::ApplyScript {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            script: script.to_owned(),
        })?)
    }

    pub fn apply_ops_request(
        &self,
        actor: &str,
        key: &str,
        ops: Vec<EditOp>,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::ApplyOps {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            ops,
        })?)
    }

    pub fn stage_script_proposal(
        &self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        observations: Vec<Value>,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::StageScriptProposal {
            request_key: request_key.to_owned(),
            base_revision,
            script: script.to_owned(),
            observations,
        })?)
    }

    pub fn undo_request(
        &self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::Undo {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
        })?)
    }

    pub fn redo_request(
        &self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::Redo {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
        })?)
    }

    pub fn import_media_request(
        &self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        match self.request(RuntimeRequest::Import {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            path: path.to_owned(),
        })? {
            RuntimeResponse::Import {
                state,
                media_id,
                clip_id,
            } => Ok((state, media_id, clip_id)),
            _ => Err(unexpected_response("import")),
        }
    }

    pub fn start_render(
        &self,
        quality: &str,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_range(quality, None, None)
    }

    /// Queue a render job of the project, or of what a pending `proposal`
    /// would make of it. The returned document is what to render.
    pub fn start_render_range(
        &self,
        quality: &str,
        range: Option<RenderRange>,
        proposal: Option<&str>,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        let (start, duration) = range
            .map(|range| (Some(range.start), Some(range.duration)))
            .unwrap_or((None, None));
        match self.request(RuntimeRequest::StartRender {
            quality: quality.to_owned(),
            start,
            duration,
            proposal_id: proposal.map(str::to_owned),
        })? {
            RuntimeResponse::RenderStarted {
                job,
                document,
                output,
            } => Ok((job, document, output)),
            _ => Err(unexpected_response("render-start")),
        }
    }

    pub fn render_job(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        expect_job(self.request(RuntimeRequest::GetRender {
            job_id: job_id.to_owned(),
        })?)
    }

    pub fn update_job(
        &self,
        job_id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
    ) -> Result<(), RuntimeError> {
        self.update_job_with_progress(job_id, status, error, output_url, None)
    }

    pub fn update_job_with_progress(
        &self,
        job_id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
        progress: Option<f64>,
    ) -> Result<(), RuntimeError> {
        match self.request(RuntimeRequest::UpdateJob {
            job_id: job_id.to_owned(),
            status: status.to_owned(),
            error,
            output_url,
            progress,
        })? {
            RuntimeResponse::Empty => Ok(()),
            _ => Err(unexpected_response("empty")),
        }
    }

    pub fn cancel_render(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        expect_job(self.request(RuntimeRequest::CancelRender {
            job_id: job_id.to_owned(),
        })?)
    }

    pub fn accept_proposal(
        &self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::AcceptProposal {
            proposal_id: proposal_id.to_owned(),
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
        })?)
    }

    pub fn reject_proposal(&self, proposal_id: &str) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::RejectProposal {
            proposal_id: proposal_id.to_owned(),
            actor: None,
            idempotency_key: None,
            base_revision: None,
        })?)
    }

    pub fn reject_proposal_request(
        &self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::RejectProposal {
            proposal_id: proposal_id.to_owned(),
            actor: Some(actor.to_owned()),
            idempotency_key: Some(key.to_owned()),
            base_revision: Some(base_revision),
        })?)
    }

    #[cfg(unix)]
    fn request_once(&self, request: &RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let envelope = RequestEnvelope::new(format!("request-{}", unique_id()), request.clone());
        let mut payload = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
        payload.push(b'\n');
        if payload.len() > MAX_REQUEST_BYTES {
            return Err(RuntimeError::new(
                "REQUEST_TOO_LARGE",
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ));
        }
        let mut stream = self.connect_or_start()?;
        stream
            .set_read_timeout(Some(self.inner.request_timeout))
            .map_err(ipc_error)?;
        stream
            .set_write_timeout(Some(self.inner.request_timeout))
            .map_err(ipc_error)?;
        stream.write_all(&payload).map_err(ipc_error)?;
        stream.flush().map_err(ipc_error)?;
        let line = read_bounded_line(&mut stream)?;
        let value: Value = serde_json::from_slice(&line)
            .map_err(|error| RuntimeError::new("INVALID_JSON", error.to_string()))?;
        decode_response(&value, &envelope.id)
    }

    #[cfg(unix)]
    fn connect_or_start(&self) -> Result<UnixStream, RuntimeError> {
        match self.connect_once() {
            Ok(stream) => return Ok(stream),
            Err(error) if error.code == "IPC_PERMISSION_DENIED" => return Err(error),
            Err(_) => {}
        }
        let binary = self.inner.binary.as_ref().ok_or_else(|| {
            RuntimeError::new(
                "RUNTIME_UNAVAILABLE",
                format!("no cutd daemon is listening at {}", self.endpoint_label()),
            )
        })?;
        Command::new(binary)
            .arg("--dir")
            .arg(&self.inner.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                RuntimeError::new(
                    "RUNTIME_UNAVAILABLE",
                    format!("failed to start cutd at {}: {error}", binary.display()),
                )
            })?;
        let deadline = std::time::Instant::now() + self.inner.startup_timeout;
        loop {
            match self.connect_once() {
                Ok(stream) => return Ok(stream),
                Err(error) if std::time::Instant::now() < deadline => {
                    // A competing client may have started the daemon, or the
                    // child may still be loading its checkpoint.
                    if error.code == "IPC_PERMISSION_DENIED" {
                        return Err(error);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    return Err(RuntimeError::new(
                        "RUNTIME_UNAVAILABLE",
                        format!("cutd did not become ready: {}", error.message),
                    ));
                }
            }
        }
    }

    #[cfg(not(unix))]
    fn request_once(&self, _request: &RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        Err(RuntimeError::new(
            "IPC_UNSUPPORTED",
            "Windows named-pipe transport is defined but not implemented yet",
        ))
    }

    #[cfg(unix)]
    fn connect_once(&self) -> Result<UnixStream, RuntimeError> {
        let path = self
            .inner
            .endpoint
            .as_path()
            .ok_or_else(|| RuntimeError::new("IPC_UNSUPPORTED", "endpoint is not Unix IPC"))?;
        UnixStream::connect(path).map_err(|error| {
            let code = match error.kind() {
                std::io::ErrorKind::PermissionDenied => "IPC_PERMISSION_DENIED",
                _ => "IPC_CONNECT_ERROR",
            };
            RuntimeError::new(
                code,
                format!("cannot connect to {}: {error}", path.display()),
            )
        })
    }

    fn endpoint_label(&self) -> String {
        match &self.inner.endpoint {
            EndpointAddress::Unix { path } => path.display().to_string(),
            EndpointAddress::WindowsNamedPipe { name } => name.clone(),
        }
    }
}

fn request_has_stable_idempotency_key(request: &RuntimeRequest) -> bool {
    match request {
        RuntimeRequest::ApplyScript {
            idempotency_key, ..
        }
        | RuntimeRequest::ApplyOps {
            idempotency_key, ..
        }
        | RuntimeRequest::Undo {
            idempotency_key, ..
        }
        | RuntimeRequest::Redo {
            idempotency_key, ..
        }
        | RuntimeRequest::Import {
            idempotency_key, ..
        }
        | RuntimeRequest::ImportProjection {
            idempotency_key, ..
        }
        | RuntimeRequest::AcceptProposal {
            idempotency_key, ..
        } => !idempotency_key.trim().is_empty(),
        // Staging has a request key, but it is not backed by the durable
        // operation-receipt table. A lost response is therefore ambiguous
        // and must not be replayed automatically.
        RuntimeRequest::StageScriptProposal { .. } => false,
        RuntimeRequest::RejectProposal {
            idempotency_key: Some(idempotency_key),
            ..
        } => !idempotency_key.trim().is_empty(),
        RuntimeRequest::GetState
        | RuntimeRequest::GetDocument { .. }
        | RuntimeRequest::GetProposalDocuments { .. }
        | RuntimeRequest::StartRender { .. }
        | RuntimeRequest::GetRender { .. }
        | RuntimeRequest::UpdateJob { .. }
        | RuntimeRequest::CancelRender { .. }
        | RuntimeRequest::RepairProjection
        | RuntimeRequest::RejectProposal {
            idempotency_key: None,
            ..
        } => false,
    }
}

fn is_transport_error(error: &RuntimeError) -> bool {
    matches!(
        error.code.as_str(),
        "IPC_CONNECT_ERROR"
            | "IPC_PERMISSION_DENIED"
            | "IPC_TIMEOUT"
            | "IPC_ERROR"
            | "RUNTIME_UNAVAILABLE"
    )
}

fn decode_response(value: &Value, expected_id: &str) -> Result<RuntimeResponse, RuntimeError> {
    let object = value.as_object().ok_or_else(|| {
        RuntimeError::new(
            "RUNTIME_PROTOCOL_ERROR",
            "runtime response must be an object",
        )
    })?;
    if object.get("protocolVersion").and_then(Value::as_u64)
        != Some(RequestEnvelope::PROTOCOL_VERSION)
    {
        return Err(RuntimeError::new(
            "PROTOCOL_VERSION_UNSUPPORTED",
            "runtime response protocolVersion must be 1",
        ));
    }
    if object.get("id").and_then(Value::as_str) != Some(expected_id) {
        return Err(RuntimeError::new(
            "RUNTIME_PROTOCOL_ERROR",
            "runtime response id does not match request",
        ));
    }
    if !object.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(object
            .get("error")
            .cloned()
            .and_then(|error| serde_json::from_value::<RuntimeError>(error).ok())
            .unwrap_or_else(|| RuntimeError::new("RUNTIME_ERROR", "runtime request failed")));
    }
    if let (Some(revision), Some(document)) = (
        object.get("revision").and_then(Value::as_u64),
        object.get("document").and_then(Value::as_str),
    ) {
        return Ok(RuntimeResponse::Document {
            revision,
            document: document.to_owned(),
        });
    }
    if let (Some(before), Some(after)) = (
        object.get("beforeDocument").and_then(Value::as_str),
        object.get("afterDocument").and_then(Value::as_str),
    ) {
        return Ok(RuntimeResponse::ProposalDocuments {
            before: before.to_owned(),
            after: after.to_owned(),
        });
    }
    if let Some(state) = object.get("state") {
        let state = serde_json::from_value::<StateSnapshot>(state.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        if let (Some(media_id), Some(clip_id)) = (
            object.get("mediaId").and_then(Value::as_str),
            object.get("clipId").and_then(Value::as_str),
        ) {
            return Ok(RuntimeResponse::Import {
                state,
                media_id: media_id.to_owned(),
                clip_id: clip_id.to_owned(),
            });
        }
        return Ok(RuntimeResponse::State(state));
    }
    if let Some(job) = object.get("job") {
        let job = serde_json::from_value::<RenderJob>(job.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        if let (Some(document), Some(output)) = (
            object.get("document").and_then(Value::as_str),
            object.get("output"),
        ) {
            let output = serde_json::from_value::<PathBuf>(output.clone())
                .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
            return Ok(RuntimeResponse::RenderStarted {
                job,
                document: document.to_owned(),
                output,
            });
        }
        return Ok(RuntimeResponse::RenderJob(job));
    }
    Ok(RuntimeResponse::Empty)
}

fn ipc_error(error: std::io::Error) -> RuntimeError {
    let code = match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "IPC_TIMEOUT",
        _ => "IPC_ERROR",
    };
    RuntimeError::new(code, error.to_string())
}

fn read_bounded_line<R: Read>(reader: &mut R) -> Result<Vec<u8>, RuntimeError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk).map_err(ipc_error)?;
        if count == 0 {
            break;
        }
        if let Some(index) = chunk[..count].iter().position(|byte| *byte == b'\n') {
            bytes.extend_from_slice(&chunk[..index]);
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(RuntimeError::new(
                "REQUEST_TOO_LARGE",
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ));
        }
    }
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(RuntimeError::new(
            "REQUEST_TOO_LARGE",
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    if bytes.is_empty() {
        return Err(RuntimeError::new(
            "RUNTIME_UNAVAILABLE",
            "runtime connection closed before a response",
        ));
    }
    Ok(bytes)
}

fn read_bounded_bufread_line<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, RuntimeError> {
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().map_err(io_error)?;
        if buffer.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Ok(Some(bytes))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.unwrap_or(buffer.len());
        if bytes.len() + length > MAX_REQUEST_BYTES {
            return Err(RuntimeError::new(
                "REQUEST_TOO_LARGE",
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ));
        }
        bytes.extend_from_slice(&buffer[..length]);
        reader.consume(newline.map_or(length, |index| index + 1));
        if newline.is_some() {
            return Ok(Some(bytes));
        }
    }
}

fn expect_state(response: RuntimeResponse) -> Result<StateSnapshot, RuntimeError> {
    match response {
        RuntimeResponse::State(state) => Ok(state),
        _ => Err(unexpected_response("state")),
    }
}

fn expect_job(response: RuntimeResponse) -> Result<RenderJob, RuntimeError> {
    match response {
        RuntimeResponse::RenderJob(job) => Ok(job),
        _ => Err(unexpected_response("render-job")),
    }
}

fn unexpected_response(expected: &str) -> RuntimeError {
    RuntimeError::new(
        "RUNTIME_PROTOCOL_ERROR",
        format!("expected {expected} response"),
    )
}

/// Convert an internal typed result to the stable JSONL response shape.  The
/// transport is intentionally kept here so every executable uses one protocol
/// implementation and one runtime owner.
pub fn response_value(id: &str, result: Result<RuntimeResponse, RuntimeError>) -> Value {
    match result {
        Ok(RuntimeResponse::State(state)) => {
            json!({ "protocolVersion": RequestEnvelope::PROTOCOL_VERSION, "id": id, "ok": true, "state": state })
        }
        Ok(RuntimeResponse::Document { revision, document }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "revision": revision,
            "document": document
        }),
        Ok(RuntimeResponse::ProposalDocuments { before, after }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "beforeDocument": before,
            "afterDocument": after
        }),
        Ok(RuntimeResponse::Import {
            state,
            media_id,
            clip_id,
        }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "state": state,
            "mediaId": media_id,
            "clipId": clip_id
        }),
        Ok(RuntimeResponse::RenderStarted {
            job,
            document,
            output,
        }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "job": job,
            "document": document,
            "output": output
        }),
        Ok(RuntimeResponse::RenderJob(job)) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "job": job
        }),
        Ok(RuntimeResponse::Empty) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true
        }),
        Err(error) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": false,
            "error": error
        }),
    }
}

/// Run the canonical project-scoped local IPC daemon.
///
/// One `RuntimeHandle` is created for the canonical project directory and is
/// shared by short-lived connection workers.  Each worker handles one JSONL
/// request and then closes its connection; the daemon and its mailbox remain
/// alive until the process itself is terminated.
pub fn run_ipc(directory: &Path) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        let directory = canonical_project_directory(directory)?;
        let endpoint = EndpointAddress::for_canonical_directory(&directory);
        let handle = RuntimeHandle::open(&directory)?;
        let path = endpoint.as_path().ok_or_else(|| {
            RuntimeError::new("IPC_UNSUPPORTED", "Unix daemon requires a Unix endpoint")
        })?;
        let listener = bind_unix_listener(path)?;
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let handle = handle.clone();
                    thread::Builder::new()
                        .name("cutd-client".into())
                        .spawn(move || serve_unix_connection(stream, handle))
                        .map_err(io_error)?;
                }
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(RuntimeError::new(
            "IPC_UNSUPPORTED",
            "Windows named-pipe transport is defined but not implemented yet",
        ))
    }
}

#[cfg(unix)]
fn bind_unix_listener(path: &Path) -> Result<UnixListener, RuntimeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    match UnixListener::bind(path) {
        Ok(listener) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error)?;
            Ok(listener)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let stale_socket = fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_socket())
                .unwrap_or(false);
            if !stale_socket {
                return Err(io_error(error));
            }
            match UnixStream::connect(path) {
                Ok(_) => Err(RuntimeError::new(
                    "RUNTIME_ALREADY_RUNNING",
                    format!("another cutd is listening at {}", path.display()),
                )),
                Err(connect_error)
                    if matches!(
                        connect_error.kind(),
                        std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    fs::remove_file(path).map_err(io_error)?;
                    let listener = UnixListener::bind(path).map_err(io_error)?;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                        .map_err(io_error)?;
                    Ok(listener)
                }
                Err(connect_error) => Err(io_error(connect_error)),
            }
        }
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(unix)]
fn serve_unix_connection(mut stream: UnixStream, handle: RuntimeHandle) {
    if let Err(error) = verify_unix_peer(&stream) {
        let _ = write_ipc_response(&mut stream, "", Err(error));
        return;
    }
    let _ = stream.set_read_timeout(Some(SERVER_REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SERVER_REQUEST_TIMEOUT));
    let line = match read_bounded_line(&mut stream) {
        Ok(line) => line,
        Err(error) => {
            let _ = write_ipc_response(&mut stream, "", Err(error));
            return;
        }
    };
    let (id, result) = process_request_line(&line, &handle);
    if should_drop_test_response(&line) {
        return;
    }
    let _ = write_ipc_response(&mut stream, &id, result);
}

#[cfg(unix)]
fn should_drop_test_response(line: &[u8]) -> bool {
    let Some(key) = std::env::var_os("CUTD_TEST_DROP_RESPONSE_KEY") else {
        return false;
    };
    let matches = serde_json::from_slice::<Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("idempotencyKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|candidate| candidate == key.to_string_lossy());
    matches && !TEST_RESPONSE_DROPPED.swap(true, Ordering::SeqCst)
}

#[cfg(unix)]
fn write_ipc_response(
    stream: &mut UnixStream,
    id: &str,
    result: Result<RuntimeResponse, RuntimeError>,
) -> Result<(), RuntimeError> {
    let value = response_value(id, result);
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    stream.write_all(&bytes).map_err(ipc_error)?;
    stream.write_all(b"\n").map_err(ipc_error)?;
    stream.flush().map_err(ipc_error)
}

#[cfg(unix)]
fn verify_unix_peer(stream: &UnixStream) -> Result<(), RuntimeError> {
    #[cfg(target_os = "linux")]
    {
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // The buffer and length are valid for this synchronous libc call, and
        // the socket descriptor is borrowed from `stream`.
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(ipc_error(std::io::Error::last_os_error()));
        }
        // `geteuid` is a side-effect-free libc query with no pointer argument.
        if credentials.uid != unsafe { libc::geteuid() } {
            return Err(RuntimeError::new(
                "IPC_UNAUTHORIZED_PEER",
                "cutd rejected a connection from a different user",
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut effective_uid = 0;
        let mut effective_gid = 0;
        // `getpeereid` writes only to the two local uid/gid values and uses the
        // borrowed socket descriptor for the duration of this call.
        let result =
            unsafe { libc::getpeereid(stream.as_raw_fd(), &mut effective_uid, &mut effective_gid) };
        if result != 0 {
            return Err(ipc_error(std::io::Error::last_os_error()));
        }
        // `geteuid` is a side-effect-free libc query with no pointer argument.
        if effective_uid != unsafe { libc::geteuid() } {
            return Err(RuntimeError::new(
                "IPC_UNAUTHORIZED_PEER",
                "cutd rejected a connection from a different user",
            ));
        }
    }

    Ok(())
}

fn process_request_line(
    line: &[u8],
    handle: &RuntimeHandle,
) -> (String, Result<RuntimeResponse, RuntimeError>) {
    match serde_json::from_slice::<Value>(line) {
        Err(error) => (
            String::new(),
            Err(RuntimeError::new("INVALID_JSON", error.to_string())),
        ),
        Ok(value) => {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            match serde_json::from_value::<RequestEnvelope>(value) {
                Ok(envelope) => {
                    let result = envelope
                        .validate()
                        .and_then(|()| handle.request(envelope.request));
                    (id, result)
                }
                Err(error) => {
                    let code = if error.to_string().contains("unknown variant") {
                        "UNKNOWN_OPERATION"
                    } else {
                        "INVALID_REQUEST"
                    };
                    (id, Err(RuntimeError::new(code, error.to_string())))
                }
            }
        }
    }
}

/// Run the persistent JSONL daemon over stdin/stdout.  A single actor owns
/// the runtime; malformed requests are rejected at this transport boundary.
pub fn run_stdio<R: BufRead, W: Write>(
    directory: &Path,
    mut input: R,
    mut output: W,
) -> Result<(), RuntimeError> {
    let handle = RuntimeHandle::open(directory)?;
    while let Some(line) = read_bounded_bufread_line(&mut input)? {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (id, result) = process_request_line(&line, &handle);
        writeln!(output, "{}", response_value(&id, result)).map_err(io_error)?;
        output.flush().map_err(io_error)?;
    }
    Ok(())
}

fn runtime_actor(
    directory: PathBuf,
    receiver: Receiver<MailboxMessage>,
    ready_sender: mpsc::SyncSender<Result<(), RuntimeError>>,
) {
    let mut runtime = match Runtime::open(&directory) {
        Ok(runtime) => {
            let _ = ready_sender.send(Ok(()));
            runtime
        }
        Err(error) => {
            let _ = ready_sender.send(Err(error));
            return;
        }
    };
    for message in receiver {
        let result = runtime.execute(message.request);
        let _ = message.reply.send(result);
    }
}

// The reducer is private to the actor; the public surface is
// `RuntimeHandle`.
impl Runtime {
    fn open(directory: &Path) -> Result<Self, RuntimeError> {
        fs::create_dir_all(directory).map_err(io_error)?;
        let lock = RuntimeLock::acquire(directory)?;
        let project_path = directory.join(PROJECT_FILE);
        let journal_path = directory.join(JOURNAL_FILE);
        let mut notes = Vec::new();
        let checkpoint = match load_checkpoint(directory)? {
            Loaded::Current(checkpoint) => Some(checkpoint),
            Loaded::Previous => {
                // State from an older cutd. Its project.cut (or, before the
                // kernel, project.cut.json) is the last published project;
                // keep its runtime files untouched and reopen the project.
                preserve_previous_state(directory)?;
                notes.push(format!(
                    "previous cutd state was preserved as {PREVIOUS_STATE_PREFIX}.*"
                ));
                None
            }
            Loaded::None => None,
        };
        let checkpoint_project = checkpoint.as_ref().map(|value| value.project.clone());
        let mut runtime = if let Some(checkpoint) = checkpoint {
            Self {
                _lock: lock,
                project_path,
                journal_path,
                state: DurableState {
                    project: Arc::unwrap_or_clone(checkpoint.project),
                    revision: checkpoint.revision,
                    past: checkpoint.past,
                    future: checkpoint.future,
                    jobs: checkpoint.jobs,
                    operations: checkpoint.operations,
                    imports: checkpoint.imports,
                    proposal: checkpoint.proposal,
                },
                import_warnings: checkpoint.import_warnings,
                journal_sequence: checkpoint.journal_sequence,
                journal_records_since_checkpoint: 0,
                projection_status: ProjectionStatus::Missing,
                projection_digest: checkpoint.projection_digest,
            }
        } else {
            let (project, mut warnings) = load_bootstrap_project(directory)?;
            notes.append(&mut warnings);
            Self {
                _lock: lock,
                project_path,
                journal_path,
                state: DurableState {
                    project,
                    revision: 0,
                    past: Vec::new(),
                    future: Vec::new(),
                    jobs: HashMap::new(),
                    operations: HashMap::new(),
                    imports: HashMap::new(),
                    proposal: None,
                },
                import_warnings: notes,
                journal_sequence: 0,
                journal_records_since_checkpoint: 0,
                projection_status: ProjectionStatus::Missing,
                projection_digest: None,
            }
        };
        bound_receipts(&mut runtime.state.operations, None);
        bound_receipts(&mut runtime.state.imports, None);

        let records = load_journal(&runtime.journal_path, runtime.journal_sequence)?;
        // A journal may contain several committed mutations after the last
        // checkpoint. If publication failed for the newest mutation, the
        // projection can still equal the last successfully published state
        // from that journal tail. Remember every state reached while replaying
        // so that only a recognized authoritative state is auto-repaired;
        // arbitrary external edits remain divergent and are preserved.
        let mut recognized_projection = checkpoint_project
            .as_ref()
            .is_some_and(|project| projection_matches(&runtime.project_path, project));
        for record in records {
            runtime.replay_journal_record(record)?;
            if !recognized_projection
                && checkpoint_project.is_some()
                && projection_matches(&runtime.project_path, &runtime.state.project)
            {
                recognized_projection = true;
            }
        }

        if recognized_projection {
            // The projection was known to equal an authoritative state before
            // or within the replayed journal tail. A crash may have left the
            // journal ahead of that state, so it is safe to repair this stale
            // projection on startup.
            if runtime.materialize_project().is_ok() {
                runtime.projection_status = ProjectionStatus::Current;
                runtime.projection_digest = projection_digest(&runtime.project_path).ok();
            } else {
                runtime.projection_status =
                    inspect_projection(&runtime.project_path, &runtime.state.project);
                runtime.projection_digest =
                    if runtime.projection_status == ProjectionStatus::Current {
                        projection_digest(&runtime.project_path).ok()
                    } else {
                        None
                    };
            }
        } else {
            runtime.projection_status =
                inspect_projection(&runtime.project_path, &runtime.state.project);
        }

        if checkpoint_project.is_none() {
            // First launch is the one controlled bootstrap point: import
            // project.cut.json once if there is no project.cut, then write the
            // canonical document and checkpoint.
            runtime.materialize_project()?;
            runtime.projection_status = ProjectionStatus::Current;
            runtime.projection_digest = Some(projection_digest(&runtime.project_path)?);
            runtime.persist_checkpoint()?;
            truncate_journal(&runtime.journal_path)?;
            runtime.journal_records_since_checkpoint = 0;
        }

        if runtime.projection_status == ProjectionStatus::Current
            && runtime.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL
        {
            // A previous process may have persisted a checkpoint but failed
            // to truncate the journal. Compact again on startup so a healthy
            // project does not accumulate stale records indefinitely.
            if runtime.persist_checkpoint().is_ok() {
                let _ = truncate_journal(&runtime.journal_path);
                runtime.journal_records_since_checkpoint = 0;
            }
        }

        if runtime.reconcile_abandoned_jobs() {
            runtime.persist_checkpoint()?;
            truncate_journal(&runtime.journal_path)?;
            runtime.journal_records_since_checkpoint = 0;
        }
        Ok(runtime)
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot_of(&self.state)
    }

    /// What clients see of `state`.
    fn snapshot_of(&self, state: &DurableState) -> StateSnapshot {
        StateSnapshot {
            duration: kernel::program_duration(&state.project).seconds(),
            document: kernel::print(&state.project),
            timeline: timeline::timeline_json(&state.project),
            revision: state.revision,
            can_undo: !state.past.is_empty(),
            can_redo: !state.future.is_empty(),
            pending_proposal: state.proposal.as_ref().map(|record| {
                let mut lifecycle = record.lifecycle.clone();
                if let PendingProposal::Pending {
                    base_revision,
                    freshness,
                    ..
                } = &mut lifecycle
                {
                    *freshness = if *base_revision == state.revision {
                        ProposalFreshness::Current
                    } else {
                        ProposalFreshness::Stale
                    };
                }
                lifecycle
            }),
            source_version: SOURCE_VERSION,
            projection_status: self.projection_status.as_str().into(),
            import_warnings: self.import_warnings.clone(),
        }
    }

    fn candidate_state(
        &self,
        project: Project,
        revision: u64,
        past: Vec<HistoryEntry>,
        future: Vec<HistoryEntry>,
    ) -> DurableState {
        let mut candidate = self.state.clone();
        candidate.project = project;
        candidate.revision = revision;
        candidate.past = past;
        candidate.future = future;
        candidate
    }

    fn commit_candidate(
        &mut self,
        candidate: DurableState,
        event: JournalEvent,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.commit_candidate_with_policy(candidate, event, false)
    }

    fn commit_projection_import(
        &mut self,
        candidate: DurableState,
        event: JournalEvent,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.commit_candidate_with_policy(candidate, event, true)
    }

    fn commit_candidate_with_policy(
        &mut self,
        mut candidate: DurableState,
        event: JournalEvent,
        allow_diverged_projection: bool,
    ) -> Result<StateSnapshot, RuntimeError> {
        let preserve_operation = match &event {
            JournalEvent::Mutation { operation, .. }
            | JournalEvent::ProjectionImport { operation, .. } => {
                operation.as_ref().map(|value| value.key.as_str())
            }
            _ => None,
        };
        let preserve_import = match &event {
            JournalEvent::Mutation { import, .. } => {
                import.as_ref().map(|value| value.key.as_str())
            }
            _ => None,
        };
        bound_receipts(&mut candidate.operations, preserve_operation);
        bound_receipts(&mut candidate.imports, preserve_import);
        let project_changed = candidate.project != self.state.project;
        if project_changed {
            self.projection_status = inspect_projection(&self.project_path, &self.state.project);
            if self.projection_status == ProjectionStatus::Diverged && !allow_diverged_projection {
                return Err(RuntimeError::new(
                    "PROJECTION_DIVERGED",
                    "project.cut differs from authoritative cutd state; repair or explicitly import the projection before editing",
                ));
            }
        }

        let sequence = self.journal_sequence + 1;
        let record = seal_journal_record(JournalRecord::from_event(
            sequence,
            candidate.revision,
            event,
        ))?;
        // The journal is the authoritative durable commit. Publish the
        // document only after the record is safely appended; startup can then
        // replay a committed record and repair a stale document without
        // mistaking it for an external edit.
        append_journal_record(&self.journal_path, &record)?;

        self.state = candidate;
        self.journal_sequence = sequence;
        self.journal_records_since_checkpoint += 1;
        if project_changed {
            if self.materialize_project().is_ok() {
                self.projection_status = ProjectionStatus::Current;
                self.projection_digest = projection_digest(&self.project_path).ok();
            } else {
                // The canonical state is already committed in the journal;
                // keep it in memory and require projection repair instead of
                // reporting an ambiguous mutation failure to the caller.
                self.projection_status = ProjectionStatus::Diverged;
                self.projection_digest = None;
            }
        }
        if self.projection_status == ProjectionStatus::Current
            && self.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL
        {
            // The journal is the durable commit. Checkpointing is a bounded
            // optimization; a temporary checkpoint failure leaves the
            // records available for recovery on the next startup.
            if self.persist_checkpoint().is_ok() {
                let _ = truncate_journal(&self.journal_path);
                self.journal_records_since_checkpoint = 0;
            }
        }
        Ok(self.snapshot())
    }

    /// The recorded result when `operation` already ran as the request
    /// `fingerprint` describes, or a conflict when its key was used for a
    /// different request.
    fn replayed(
        &self,
        operation: &str,
        fingerprint: &str,
    ) -> Option<Result<StateSnapshot, RuntimeError>> {
        let record = self.state.operations.get(operation)?;
        Some(if record.fingerprint == fingerprint {
            Ok((*record.snapshot).clone())
        } else {
            Err(RuntimeError::new(
                "IDEMPOTENCY_CONFLICT",
                "idempotency key was already used for a different request",
            ))
        })
    }

    /// The state after an edit, with a history entry, and how to journal
    /// it.
    fn edit_candidate(&self, edited: Edited) -> (DurableState, Change) {
        let Edited { project, script } = edited;
        let change = Change {
            script: Some(script.clone()),
            document: None,
            digest: digest(&project),
        };
        let mut past = self.state.past.clone();
        past.push(HistoryEntry {
            script,
            project: Arc::new(self.state.project.clone()),
        });
        bound_history(&mut past);
        (
            self.candidate_state(project, self.state.revision + 1, past, Vec::new()),
            change,
        )
    }

    pub fn apply_script_request(
        &mut self,
        actor: &str,
        key: &str,
        script: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.apply_edit_request(
            actor,
            key,
            Edit::Script {
                script: script.to_owned(),
            },
            base_revision,
            self.state.proposal.clone(),
        )
    }

    pub fn apply_ops_request(
        &mut self,
        actor: &str,
        key: &str,
        ops: Vec<EditOp>,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.apply_edit_request(
            actor,
            key,
            Edit::Ops { ops },
            base_revision,
            self.state.proposal.clone(),
        )
    }

    fn apply_edit_request(
        &mut self,
        actor: &str,
        key: &str,
        edit: Edit,
        base_revision: u64,
        proposal: Option<ProposalRecord>,
    ) -> Result<StateSnapshot, RuntimeError> {
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{}", edit.fingerprint());
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        let (mut candidate, change) = self.edit_candidate(edit.run(&self.state.project)?);
        candidate.proposal = proposal;
        let snapshot = self.snapshot_of(&candidate);
        if let Some(proposal) = candidate.proposal.as_mut() {
            if matches!(proposal.lifecycle, PendingProposal::Accepted { .. })
                && proposal.receipt.is_none()
            {
                proposal.receipt = Some(ProposalReceipt {
                    actor: actor.to_owned(),
                    key: key.to_owned(),
                    base_revision,
                    snapshot: Arc::new(snapshot.clone()),
                });
            }
        }
        let operation = candidate.record_operation(operation, fingerprint, snapshot);
        let proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                change: Some(change),
                operation: Some(operation),
                import: None,
                proposal,
            },
        )
    }

    /// Stage an agent's edit script as a pending proposal.
    pub fn stage_script_proposal(
        &mut self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        observations: Vec<Value>,
    ) -> Result<StateSnapshot, RuntimeError> {
        if request_key.trim().is_empty() {
            return Err(RuntimeError::new(
                "AGENT_PROTOCOL_ERROR",
                "agent request requires a non-empty idempotency key",
            ));
        }
        self.check_revision(Some(base_revision))?;
        self.stage_edit_proposal(
            request_key,
            base_revision,
            Edit::Script {
                script: script.to_owned(),
            },
            observations,
        )
    }

    fn stage_edit_proposal(
        &mut self,
        request_key: &str,
        base_revision: u64,
        edit: Edit,
        observations: Vec<Value>,
    ) -> Result<StateSnapshot, RuntimeError> {
        let observations = observations
            .into_iter()
            .map(|value| {
                let observation =
                    serde_json::from_value::<ObservationReference>(value).map_err(|error| {
                        RuntimeError::new(
                            "AGENT_PROTOCOL_ERROR",
                            format!("invalid observation reference: {error}"),
                        )
                    })?;
                self.validate_observation_reference(&observation, base_revision)?;
                Ok(observation)
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let observation = observations.first().cloned();
        let fingerprint = format!(
            "{}:{}:{}",
            base_revision,
            edit.fingerprint(),
            serde_json::to_string(&observations).unwrap_or_default()
        );

        if let Some(existing) = self.state.proposal.as_ref() {
            if existing.request_key == request_key {
                if existing.fingerprint != fingerprint {
                    return Err(RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "agent request key was already used for a different proposal",
                    ));
                }
                return Ok(self.snapshot());
            }
            if matches!(existing.lifecycle, PendingProposal::Pending { .. }) {
                return Err(RuntimeError::new(
                    "PROPOSAL_PENDING",
                    "another agent proposal is awaiting approval",
                ));
            }
        }

        let edited = edit.run(&self.state.project)?;
        let document_diff = cut_lang::diff_text(
            &kernel::print(&self.state.project),
            &kernel::print(&edited.project),
        );
        let proposal = Some(ProposalRecord {
            lifecycle: PendingProposal::Pending {
                proposal_id: format!("proposal-{}", unique_id()),
                base_revision,
                script: edited.script,
                document_diff,
                freshness: ProposalFreshness::Current,
                observation,
                observations: (observations.len() > 1).then_some(observations),
            },
            request_key: request_key.to_owned(),
            fingerprint,
            receipt: None,
        });
        let mut candidate = self.state.clone();
        candidate.proposal = proposal;
        let proposal = candidate.proposal.clone();
        self.commit_candidate(candidate, JournalEvent::Proposal { proposal })
    }

    fn validate_observation_reference(
        &self,
        observation: &ObservationReference,
        base_revision: u64,
    ) -> Result<(), RuntimeError> {
        observation
            .validate()
            .map_err(|error| RuntimeError::new("AGENT_PROTOCOL_ERROR", error))?;
        let observed_path =
            Path::new(&observation.source_path)
                .canonicalize()
                .map_err(|error| {
                    RuntimeError::new(
                        "STALE_OBSERVATION",
                        format!("observation source is unavailable: {error}"),
                    )
                })?;
        let identity = SourceIdentity::from_path(&observed_path).map_err(|error| {
            RuntimeError::new(
                "STALE_OBSERVATION",
                format!("observation source identity is unavailable: {error}"),
            )
        })?;
        if identity.path != observation.source_identity.path
            || identity.size != observation.source_identity.size
            || identity.modified_ns != observation.source_identity.modified_ns
        {
            return Err(RuntimeError::new(
                "STALE_OBSERVATION",
                "observation source identity no longer matches the media file",
            ));
        }
        let project_media = self.state.project.media.iter().find(|media| {
            media.id == observation.source_ref.as_str()
                || self
                    .project_source_path(&media.path)
                    .canonicalize()
                    .ok()
                    .as_ref()
                    == Some(&observed_path)
        });
        if let Some(media) = project_media {
            let observation_revision = observation.revision.ok_or_else(|| {
                RuntimeError::new(
                    "STALE_OBSERVATION",
                    "project-bound observation must include its project revision",
                )
            })?;
            if observation_revision != base_revision {
                return Err(RuntimeError::new(
                    "STALE_OBSERVATION",
                    format!(
                        "observation belongs to revision {observation_revision}, not {base_revision}"
                    ),
                ));
            }
            let source_path = self
                .project_source_path(&media.path)
                .canonicalize()
                .map_err(|error| {
                    RuntimeError::new(
                        "STALE_OBSERVATION",
                        format!("current project source is unavailable: {error}"),
                    )
                })?;
            if source_path != observed_path {
                return Err(RuntimeError::new(
                    "STALE_OBSERVATION",
                    "observation source path does not match the project source",
                ));
            }
            if observation.range.end > media.duration.seconds() + 1e-9 {
                return Err(RuntimeError::new(
                    "STALE_OBSERVATION",
                    "observation range is outside the project source duration",
                ));
            }
        } else if observation.revision.is_some() {
            return Err(RuntimeError::new(
                "STALE_OBSERVATION",
                "raw observation must not claim a project revision",
            ));
        }
        Ok(())
    }

    fn project_source_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        }
    }

    /// Apply a previously staged proposal as exactly one canonical request.
    pub fn accept_proposal(
        &mut self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }
        let record = self
            .state
            .proposal
            .clone()
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision: proposal_revision,
                script,
                observation,
                observations,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                if let Some(observations) = observations.as_ref() {
                    for observation in observations {
                        self.validate_observation_reference(observation, base_revision)?;
                    }
                } else if let Some(observation) = observation.as_ref() {
                    self.validate_observation_reference(observation, base_revision)?;
                }
                self.check_revision(Some(base_revision))?;
                if base_revision != proposal_revision {
                    return Err(RuntimeError::new(
                        "PROPOSAL_BASE_MISMATCH",
                        format!(
                            "proposal was created at revision {proposal_revision}, not {base_revision}"
                        ),
                    ));
                }
                let accepted_proposal = Some(ProposalRecord {
                    lifecycle: PendingProposal::Accepted {
                        proposal_id: current_id.clone(),
                        base_revision: proposal_revision,
                        applied_revision: self.state.revision + 1,
                        observation,
                        observations,
                    },
                    request_key: record.request_key.clone(),
                    fingerprint: record.fingerprint.clone(),
                    receipt: None,
                });
                self.apply_edit_request(
                    actor,
                    key,
                    Edit::Script { script },
                    base_revision,
                    accepted_proposal,
                )
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal has no durable acceptance receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal acceptance was already recorded for a different request",
                    ));
                }
                // Terminal lifecycle states are durable receipts.  Returning
                // the recorded snapshot makes retries converge even after a
                // later mutation advances the live project.
                Ok((*receipt.snapshot).clone())
            }
            PendingProposal::Rejected {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                Err(RuntimeError::new(
                    "PROPOSAL_REJECTED",
                    "proposal has already been rejected",
                ))
            }
        }
    }

    /// Reject a proposal without changing canonical project state, revision,
    /// or undo/redo history.  Rejection is represented as a terminal receipt
    /// so a repeated request is a harmless no-op.
    pub fn reject_proposal(&mut self, proposal_id: &str) -> Result<StateSnapshot, RuntimeError> {
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }
        let record = self
            .state
            .proposal
            .clone()
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                let rejected_proposal = Some(ProposalRecord {
                    lifecycle: PendingProposal::Rejected {
                        proposal_id: current_id.clone(),
                        base_revision,
                    },
                    request_key: record.request_key.clone(),
                    fingerprint: record.fingerprint.clone(),
                    receipt: None,
                });
                let mut candidate = self.state.clone();
                candidate.proposal = rejected_proposal;
                let proposal = candidate.proposal.clone();
                self.commit_candidate(candidate, JournalEvent::Proposal { proposal })
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            }
            | PendingProposal::Rejected {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                Ok(self.snapshot())
            }
        }
    }

    /// Reject a proposal as a revisioned, idempotent request.  The legacy
    /// `reject_proposal` method above intentionally remains available to the
    /// desktop adapter, but clients that supply an actor/key/base revision
    /// receive the same durable receipt guarantees as apply/undo/redo.
    pub fn reject_proposal_request(
        &mut self,
        proposal_id: &str,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        if proposal_id.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_REQUEST",
                "proposalId is required",
            ));
        }

        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:reject:{proposal_id}");
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }

        let record = self
            .state
            .proposal
            .clone()
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision: proposal_revision,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                self.check_revision(Some(base_revision))?;
                if base_revision != proposal_revision {
                    return Err(RuntimeError::new(
                        "PROPOSAL_BASE_MISMATCH",
                        format!(
                            "proposal was created at revision {proposal_revision}, not {base_revision}"
                        ),
                    ));
                }

                let rejected_proposal = Some(ProposalRecord {
                    lifecycle: PendingProposal::Rejected {
                        proposal_id: current_id,
                        base_revision: proposal_revision,
                    },
                    request_key: record.request_key,
                    fingerprint: record.fingerprint,
                    receipt: None,
                });
                let mut candidate = self.state.clone();
                candidate.proposal = rejected_proposal;
                let snapshot = self.snapshot_of(&candidate);
                if let Some(proposal) = candidate.proposal.as_mut() {
                    proposal.receipt = Some(ProposalReceipt {
                        actor: actor.to_owned(),
                        key: key.to_owned(),
                        base_revision,
                        snapshot: Arc::new(snapshot.clone()),
                    });
                }
                let operation = candidate.record_operation(operation, fingerprint, snapshot);
                let proposal = candidate.proposal.clone();
                self.commit_candidate(
                    candidate,
                    JournalEvent::Mutation {
                        action: JournalAction::RejectProposal,
                        change: None,
                        operation: Some(operation),
                        import: None,
                        proposal,
                    },
                )
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                Err(RuntimeError::new(
                    "PROPOSAL_ALREADY_ACCEPTED",
                    "proposal has already been accepted",
                ))
            }
            PendingProposal::Rejected {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(proposal_not_found(proposal_id));
                }
                let receipt = record.receipt.as_ref().ok_or_else(|| {
                    RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal has no durable rejection receipt",
                    )
                })?;
                if receipt.actor != actor
                    || receipt.key != key
                    || receipt.base_revision != base_revision
                {
                    return Err(RuntimeError::new(
                        "IDEMPOTENCY_CONFLICT",
                        "proposal rejection was already recorded for a different request",
                    ));
                }
                Ok((*receipt.snapshot).clone())
            }
        }
    }

    pub fn undo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.history_request(Direction::Undo, actor, key, base_revision)
    }

    pub fn redo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        self.history_request(Direction::Redo, actor, key, base_revision)
    }

    fn history_request(
        &mut self,
        direction: Direction,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{}", direction.name());
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        self.history_step(direction, operation, fingerprint)
    }

    /// Restore the newest snapshot in the direction's stack. With nothing to
    /// restore, the request still records a no-op receipt.
    fn history_step(
        &mut self,
        direction: Direction,
        operation: String,
        fingerprint: String,
    ) -> Result<StateSnapshot, RuntimeError> {
        let (mut candidate, action, change) = if self.history(direction).is_empty() {
            (self.state.clone(), JournalAction::Noop, None)
        } else {
            let (project, past, future, change) = self.stepped(direction);
            let candidate = self.candidate_state(project, self.state.revision + 1, past, future);
            (candidate, direction.action(), Some(change))
        };
        let snapshot = self.snapshot_of(&candidate);
        let operation = candidate.record_operation(operation, fingerprint, snapshot);
        let proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action,
                change,
                operation: Some(operation),
                import: None,
                proposal,
            },
        )
    }

    /// The history an undo or redo takes its entry from.
    fn history(&self, direction: Direction) -> &[HistoryEntry] {
        match direction {
            Direction::Undo => &self.state.past,
            Direction::Redo => &self.state.future,
        }
    }

    /// The project, past, and future after one undo or redo. The caller has
    /// checked that the stack to take from is not empty.
    fn stepped(
        &self,
        direction: Direction,
    ) -> (Project, Vec<HistoryEntry>, Vec<HistoryEntry>, Change) {
        let (mut from, mut to) = match direction {
            Direction::Undo => (self.state.past.clone(), self.state.future.clone()),
            Direction::Redo => (self.state.future.clone(), self.state.past.clone()),
        };
        let entry = from.pop().expect("history step has an entry");
        let project = kernel::restore(&entry.project, &self.state.project);
        to.push(HistoryEntry {
            script: entry.script,
            project: Arc::new(self.state.project.clone()),
        });
        bound_history(&mut to);
        let change = Change {
            script: None,
            document: None,
            digest: digest(&project),
        };
        match direction {
            Direction::Undo => (project, from, to, change),
            Direction::Redo => (project, to, from, change),
        }
    }

    /// Queue a render of the first sequence of the project, or of what a
    /// pending `proposal` would make of it, and return the document to
    /// render, with media paths resolved.
    pub fn start_render_range(
        &mut self,
        quality: &str,
        range: Option<RenderRange>,
        proposal: Option<&str>,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        if !matches!(quality, "preview" | "full") {
            return Err(RuntimeError::new(
                "INVALID_RENDER_QUALITY",
                "quality must be preview or full",
            ));
        }
        let proposed = proposal.map(|id| self.proposal_result(id)).transpose()?;
        let project = proposed.as_ref().unwrap_or(&self.state.project);
        let document = self.renderable(project);
        let duration = kernel::program_duration(project).seconds();
        if let Some(range) = range.as_ref() {
            RenderRange::new(range.start, range.duration)?;
            if range.end() > duration + 1e-9 {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_RANGE",
                    format!(
                        "render range ends at {:.3}s but project duration is {duration:.3}s",
                        range.end(),
                    ),
                ));
            }
        }
        let id = format!("render-{}", unique_id());
        let output = self
            .project_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{id}.mp4"));
        let cancel_path =
            render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), &id);
        match fs::remove_file(&cancel_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let job = RenderJob {
            job_id: id.clone(),
            status: "queued".into(),
            output_url: None,
            output_path: Some(output.to_string_lossy().into_owned()),
            error: None,
            range,
            progress: 0.0,
        };
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id, job.clone());
        self.commit_candidate(candidate, JournalEvent::Job { job: job.clone() })?;
        Ok((job, document, output))
    }

    /// The documents before and after a pending proposal, for a renderer.
    pub fn proposal_documents(&self, proposal_id: &str) -> Result<(String, String), RuntimeError> {
        let after = self.proposal_result(proposal_id)?;
        Ok((
            self.renderable(&self.state.project),
            self.renderable(&after),
        ))
    }

    /// The project a pending, current proposal would produce.
    fn proposal_result(&self, proposal_id: &str) -> Result<Project, RuntimeError> {
        let record = self
            .state
            .proposal
            .as_ref()
            .ok_or_else(|| proposal_not_found(proposal_id))?;
        let PendingProposal::Pending {
            proposal_id: current_id,
            base_revision,
            script,
            ..
        } = &record.lifecycle
        else {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_PENDING",
                format!("proposal is not pending: {proposal_id}"),
            ));
        };
        if current_id != proposal_id {
            return Err(proposal_not_found(proposal_id));
        }
        if *base_revision != self.state.revision {
            return Err(RuntimeError::new(
                "PROPOSAL_STALE",
                format!(
                    "proposal was created at revision {base_revision}, current revision is {}",
                    self.state.revision
                ),
            ));
        }
        let edit = Edit::Script {
            script: script.clone(),
        };
        Ok(edit.run(&self.state.project)?.project)
    }

    /// The document for a renderer: media paths resolved against the
    /// project directory.
    fn renderable(&self, project: &Project) -> String {
        kernel::renderable(
            project,
            self.project_path.parent().unwrap_or(Path::new(".")),
        )
    }

    pub fn import_media_request(
        &mut self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:{path}");
        if let Some(record) = self.state.imports.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different import",
                ));
            }
            return Ok((
                (*record.snapshot).clone(),
                record.media_id.clone(),
                record.clip_id.clone(),
            ));
        }
        self.check_revision(Some(base_revision))?;
        // Build the import candidate without mutating the actor, so a failed
        // probe or edit leaves every in-memory field untouched.
        let path = validate_media_path(path)?.to_string_lossy().into_owned();
        let probe = cut_engine::probe::info(&path).ok();
        let (ops, media_id, clip_id) =
            kernel::import_media(&self.state.project, &path, probe.as_ref())?;
        let (mut candidate, change) =
            self.edit_candidate(kernel::apply_ops(&self.state.project, &ops)?);
        let (media_id, clip_id) = (media_id.to_string(), clip_id.to_string());
        let snapshot = self.snapshot_of(&candidate);
        let record = ImportRecord {
            fingerprint,
            snapshot: Arc::new(snapshot.clone()),
            media_id: media_id.clone(),
            clip_id: clip_id.clone(),
        };
        candidate.imports.insert(operation.clone(), record.clone());
        let proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                change: Some(change),
                operation: None,
                import: Some(JournalImport {
                    key: operation,
                    record,
                }),
                proposal,
            },
        )?;
        Ok((snapshot, media_id, clip_id))
    }

    pub fn update_job(
        &mut self,
        id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
        progress: Option<f64>,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            status,
            "queued" | "running" | "completed" | "failed" | "cancelled"
        ) {
            return Err(RuntimeError::new(
                "INVALID_RENDER_STATUS",
                format!("unsupported render status: {status}"),
            ));
        }
        let current = self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })?;
        if matches!(
            current.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) && current.status != status
        {
            return Err(RuntimeError::new(
                "RENDER_STATE_CONFLICT",
                format!("render job {id} is already {}", current.status),
            ));
        }
        let mut job = current;
        job.status = status.into();
        job.error = error;
        job.output_url = output_url;
        if let Some(progress) = progress {
            if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_PROGRESS",
                    "render progress must be finite and between 0 and 1",
                ));
            }
            job.progress = progress;
        }
        if status == "completed" {
            job.progress = 1.0;
        }
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id.to_owned(), job.clone());
        self.commit_candidate(candidate, JournalEvent::Job { job })?;
        Ok(())
    }

    pub fn render_job(&self, id: &str) -> Result<RenderJob, RuntimeError> {
        self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })
    }

    pub fn cancel_render(&mut self, id: &str) -> Result<RenderJob, RuntimeError> {
        let current = self.render_job(id)?;
        if matches!(current.status.as_str(), "queued" | "running") {
            let cancel_path =
                render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), id);
            fs::write(&cancel_path, b"cancelled\n").map_err(io_error)?;
            self.update_job(
                id,
                "cancelled",
                Some("render job was cancelled".into()),
                None,
                None,
            )?;
        }
        self.render_job(id)
    }

    fn execute(&mut self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.refresh_projection_status();
        match request {
            RuntimeRequest::GetState => Ok(RuntimeResponse::State(self.snapshot())),
            RuntimeRequest::GetDocument { annotated } => Ok(RuntimeResponse::Document {
                revision: self.state.revision,
                document: self.document(annotated),
            }),
            RuntimeRequest::ApplyScript {
                actor,
                idempotency_key,
                base_revision,
                script,
            } => Ok(RuntimeResponse::State(self.apply_script_request(
                &actor,
                &idempotency_key,
                &script,
                base_revision,
            )?)),
            RuntimeRequest::ApplyOps {
                actor,
                idempotency_key,
                base_revision,
                ops,
            } => Ok(RuntimeResponse::State(self.apply_ops_request(
                &actor,
                &idempotency_key,
                ops,
                base_revision,
            )?)),
            RuntimeRequest::StageScriptProposal {
                request_key,
                base_revision,
                script,
                observations,
            } => Ok(RuntimeResponse::State(self.stage_script_proposal(
                &request_key,
                base_revision,
                &script,
                observations,
            )?)),
            RuntimeRequest::GetProposalDocuments { proposal_id } => {
                let (before, after) = self.proposal_documents(&proposal_id)?;
                Ok(RuntimeResponse::ProposalDocuments { before, after })
            }
            RuntimeRequest::Undo {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.undo_request(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::Redo {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.redo_request(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::Import {
                actor,
                idempotency_key,
                base_revision,
                path,
            } => {
                let (state, media_id, clip_id) =
                    self.import_media_request(&actor, &idempotency_key, &path, base_revision)?;
                Ok(RuntimeResponse::Import {
                    state,
                    media_id,
                    clip_id,
                })
            }
            RuntimeRequest::StartRender {
                quality,
                start,
                duration,
                proposal_id,
            } => {
                let range = match (start, duration) {
                    (None, None) => None,
                    (Some(start), Some(duration)) => Some(RenderRange::new(start, duration)?),
                    _ => {
                        return Err(RuntimeError::new(
                            "INVALID_RENDER_RANGE",
                            "render start and duration must be provided together",
                        ))
                    }
                };
                let (job, document, output) =
                    self.start_render_range(&quality, range, proposal_id.as_deref())?;
                Ok(RuntimeResponse::RenderStarted {
                    job,
                    document,
                    output,
                })
            }
            RuntimeRequest::GetRender { job_id } => {
                Ok(RuntimeResponse::RenderJob(self.render_job(&job_id)?))
            }
            RuntimeRequest::UpdateJob {
                job_id,
                status,
                error,
                output_url,
                progress,
            } => {
                self.update_job(&job_id, &status, error, output_url, progress)?;
                Ok(RuntimeResponse::Empty)
            }
            RuntimeRequest::CancelRender { job_id } => {
                Ok(RuntimeResponse::RenderJob(self.cancel_render(&job_id)?))
            }
            RuntimeRequest::RepairProjection => {
                Ok(RuntimeResponse::State(self.repair_projection()?))
            }
            RuntimeRequest::ImportProjection {
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.import_projection(
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::AcceptProposal {
                proposal_id,
                actor,
                idempotency_key,
                base_revision,
            } => Ok(RuntimeResponse::State(self.accept_proposal(
                &proposal_id,
                &actor,
                &idempotency_key,
                base_revision,
            )?)),
            RuntimeRequest::RejectProposal {
                proposal_id,
                actor,
                idempotency_key,
                base_revision,
            } => match (actor, idempotency_key, base_revision) {
                (None, None, None) => {
                    Ok(RuntimeResponse::State(self.reject_proposal(&proposal_id)?))
                }
                (Some(actor), Some(idempotency_key), Some(base_revision)) => {
                    Ok(RuntimeResponse::State(self.reject_proposal_request(
                        &proposal_id,
                        &actor,
                        &idempotency_key,
                        base_revision,
                    )?))
                }
                _ => Err(RuntimeError::new(
                    "INVALID_REQUEST",
                    "proposal rejection requires actor, idempotencyKey, and baseRevision together",
                )),
            },
        }
    }

    fn replay_journal_record(&mut self, record: JournalRecord) -> Result<(), RuntimeError> {
        if record.sequence != self.journal_sequence + 1 {
            return Err(RuntimeError::new(
                "JOURNAL_SEQUENCE_ERROR",
                format!(
                    "expected journal sequence {}, got {}",
                    self.journal_sequence + 1,
                    record.sequence
                ),
            ));
        }
        let missing = |what: &str| {
            RuntimeError::new("INVALID_JOURNAL", format!("{what} record has no change"))
        };
        match record.action {
            JournalAction::Apply => {
                let change = record.change.as_ref().ok_or_else(|| missing("apply"))?;
                let script = change.script.clone().ok_or_else(|| missing("apply"))?;
                let edited = Edit::Script { script }.run(&self.state.project)?;
                verify_change(change, &edited.project, record.sequence)?;
                let before = std::mem::replace(&mut self.state.project, edited.project);
                self.state.past.push(HistoryEntry {
                    script: edited.script,
                    project: Arc::new(before),
                });
                bound_history(&mut self.state.past);
                self.state.future.clear();
                self.state.revision = record.revision;
                self.state.proposal = record.proposal;
            }
            JournalAction::Undo | JournalAction::Redo => {
                let direction = if matches!(record.action, JournalAction::Undo) {
                    Direction::Undo
                } else {
                    Direction::Redo
                };
                if self.history(direction).is_empty() {
                    return Err(RuntimeError::new(
                        "INVALID_JOURNAL",
                        format!("{} record has no history entry", direction.name()),
                    ));
                }
                let change = record
                    .change
                    .as_ref()
                    .ok_or_else(|| missing(direction.name()))?;
                let (project, past, future, _) = self.stepped(direction);
                verify_change(change, &project, record.sequence)?;
                self.state.project = project;
                self.state.past = past;
                self.state.future = future;
                self.state.revision = record.revision;
                self.state.proposal = record.proposal;
            }
            JournalAction::Noop | JournalAction::RejectProposal => {
                self.state.revision = record.revision;
                self.state.proposal = record.proposal;
            }
            JournalAction::StageProposal => {
                self.state.proposal = record.proposal;
            }
            JournalAction::Job => {
                let job = record
                    .job
                    .ok_or_else(|| RuntimeError::new("INVALID_JOURNAL", "job record has no job"))?;
                self.state.jobs.insert(job.job_id.clone(), job);
            }
            JournalAction::ProjectionImport => {
                let change = record
                    .change
                    .as_ref()
                    .ok_or_else(|| missing("projection import"))?;
                let document = change.document.as_deref().ok_or_else(|| {
                    RuntimeError::new("INVALID_JOURNAL", "projection import has no document")
                })?;
                let project = kernel::parse(document)?;
                verify_change(change, &project, record.sequence)?;
                let project_changed = project != self.state.project;
                self.state.project = project;
                self.state.revision = record.revision;
                if project_changed {
                    self.state.past.clear();
                    self.state.future.clear();
                    self.state.proposal = None;
                }
            }
        }
        if let Some(operation) = record.operation {
            self.state
                .operations
                .insert(operation.key, operation.record);
            bound_receipts(&mut self.state.operations, None);
        }
        if let Some(import) = record.import {
            self.state.imports.insert(import.key, import.record);
            bound_receipts(&mut self.state.imports, None);
        }
        self.journal_sequence = record.sequence;
        self.journal_records_since_checkpoint += 1;
        Ok(())
    }

    fn check_revision(&self, expected: Option<u64>) -> Result<(), RuntimeError> {
        if let Some(value) = expected {
            if value != self.state.revision {
                return Err(RuntimeError::conflict(value, self.state.revision));
            }
        }
        Ok(())
    }

    fn refresh_projection_status(&mut self) {
        // An unchanged file is the document this runtime last wrote.
        let digest = projection_digest(&self.project_path).ok();
        if digest.is_some() && digest == self.projection_digest {
            self.projection_status = ProjectionStatus::Current;
            return;
        }
        self.projection_status = inspect_projection(&self.project_path, &self.state.project);
        if self.projection_status == ProjectionStatus::Current {
            self.projection_digest = digest;
        }
    }

    fn persist_checkpoint(&self) -> Result<(), RuntimeError> {
        let checkpoint = PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: Arc::new(self.state.project.clone()),
            revision: self.state.revision,
            past: self.state.past.clone(),
            future: self.state.future.clone(),
            journal_sequence: self.journal_sequence,
            operations: self.state.operations.clone(),
            imports: self.state.imports.clone(),
            proposal: self.state.proposal.clone(),
            jobs: self.state.jobs.clone(),
            import_warnings: self.import_warnings.clone(),
            projection_digest: self.projection_digest.clone(),
        };
        write_json_atomic(
            &self.project_path.with_file_name(CHECKPOINT_FILE),
            &checkpoint,
        )
    }

    /// Write the canonical document.
    fn materialize_project(&self) -> Result<(), RuntimeError> {
        write_bytes_atomic(
            &self.project_path,
            kernel::print(&self.state.project).as_bytes(),
        )
    }

    /// Make the edited `project.cut` canonical. Text edits made outside cutd
    /// become one revision that clears history.
    fn import_projection(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:projection_import");
        if let Some(replayed) = self.replayed(&operation, &fingerprint) {
            return replayed;
        }
        self.check_revision(Some(base_revision))?;
        if !self.project_path.is_file() {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                "project.cut is required for an explicit projection import",
            ));
        }
        let text = fs::read_to_string(&self.project_path).map_err(io_error)?;
        let project = kernel::parse(&text)?;
        let project_changed = project != self.state.project;
        let revision = if project_changed {
            self.state.revision + 1
        } else {
            self.state.revision
        };
        let mut candidate = self.state.clone();
        candidate.project = project;
        candidate.revision = revision;
        if project_changed {
            candidate.past.clear();
            candidate.future.clear();
            candidate.proposal = None;
        }
        let mut snapshot = self.snapshot_of(&candidate);
        snapshot.projection_status = ProjectionStatus::Current.as_str().into();
        let operation = candidate.record_operation(operation, fingerprint, snapshot);
        if !project_changed && self.projection_status != ProjectionStatus::Current {
            // The file already describes the authoritative project but is not
            // its canonical text (comments, spacing). An explicit import is
            // also a repair request in this case.
            write_bytes_atomic(
                &self.project_path,
                kernel::print(&candidate.project).as_bytes(),
            )?;
            self.projection_status = ProjectionStatus::Current;
            self.projection_digest = projection_digest(&self.project_path).ok();
        }
        let change = Change {
            script: None,
            document: Some(kernel::print(&candidate.project)),
            digest: digest(&candidate.project),
        };
        self.commit_projection_import(
            candidate,
            JournalEvent::ProjectionImport {
                change,
                operation: Some(operation),
            },
        )
    }

    fn repair_projection(&mut self) -> Result<StateSnapshot, RuntimeError> {
        self.materialize_project()?;
        self.projection_status = ProjectionStatus::Current;
        self.projection_digest = Some(projection_digest(&self.project_path)?);
        self.persist_checkpoint()?;
        truncate_journal(&self.journal_path)?;
        self.journal_records_since_checkpoint = 0;
        Ok(self.snapshot())
    }

    fn document(&self, annotated: bool) -> String {
        if annotated {
            kernel::print_annotated(&self.state.project)
        } else {
            kernel::print(&self.state.project)
        }
    }

    fn reconcile_abandoned_jobs(&mut self) -> bool {
        let mut changed = false;
        for job in self.state.jobs.values_mut() {
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "failed".into();
                job.error = Some("render job was abandoned when cutd restarted".into());
                changed = true;
            }
        }
        changed
    }
}

pub fn validate_actor(actor: &str) -> Result<(), RuntimeError> {
    if matches!(actor, "ui" | "agent" | "mcp" | "system") {
        Ok(())
    } else {
        Err(RuntimeError::new(
            "INVALID_REQUEST",
            "actor must be ui, agent, mcp, or system",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }

    fn action(self) -> JournalAction {
        match self {
            Self::Undo => JournalAction::Undo,
            Self::Redo => JournalAction::Redo,
        }
    }
}

/// How to journal a change that leaves `project`.
/// The digest a journaled change records of the project it produced.
fn digest(project: &Project) -> String {
    sha256_hex(kernel::print(project).as_bytes())
}

/// Fail a replay that does not reproduce the journaled document.
fn verify_change(change: &Change, project: &Project, sequence: u64) -> Result<(), RuntimeError> {
    if digest(project) == change.digest {
        Ok(())
    } else {
        Err(RuntimeError::new(
            "JOURNAL_REPLAY_MISMATCH",
            format!("journal record {sequence} did not reproduce its project"),
        ))
    }
}

fn validate_request_identity(actor: &str, key: &str) -> Result<(), RuntimeError> {
    validate_actor(actor)?;
    if key.trim().is_empty() {
        return Err(RuntimeError::new(
            "INVALID_REQUEST",
            "idempotencyKey is required",
        ));
    }
    Ok(())
}

/// The project of a directory without runtime state: `project.cut`, else a
/// one-time import of `project.cut.json` (left untouched) with its
/// warnings, else an empty project.
fn load_bootstrap_project(directory: &Path) -> Result<(Project, Vec<String>), RuntimeError> {
    let document = directory.join(PROJECT_FILE);
    if document.is_file() {
        let text = fs::read_to_string(&document).map_err(io_error)?;
        return Ok((kernel::parse(&text)?, Vec::new()));
    }
    let legacy = directory.join(LEGACY_PROJECT_FILE);
    if !legacy.is_file() {
        return Ok((kernel::empty_project(), Vec::new()));
    }
    let text = fs::read_to_string(&legacy).map_err(io_error)?;
    let imported = cut_interchange::v2::import_json(&text).map_err(|errors| {
        RuntimeError::new(
            "INVALID_PROJECT",
            format!(
                "{LEGACY_PROJECT_FILE} could not be imported: {}",
                errors.join("; ")
            ),
        )
    })?;
    let mut warnings = vec![format!(
        "imported {LEGACY_PROJECT_FILE} into {PROJECT_FILE}; {LEGACY_PROJECT_FILE} is no longer read or written"
    )];
    warnings.extend(imported.warnings);
    Ok((imported.project, warnings))
}

/// Move an older cutd's checkpoint and journal aside, unchanged.
fn preserve_previous_state(directory: &Path) -> Result<(), RuntimeError> {
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

fn bound_history(entries: &mut Vec<HistoryEntry>) {
    if entries.len() > MAX_HISTORY_ENTRIES {
        let excess = entries.len() - MAX_HISTORY_ENTRIES;
        entries.drain(..excess);
    }
}

fn bound_receipts<T>(receipts: &mut HashMap<String, T>, preserve: Option<&str>) {
    while receipts.len() > MAX_RECEIPTS {
        let Some(key) = receipts
            .keys()
            .find(|key| preserve != Some(key.as_str()))
            .cloned()
        else {
            break;
        };
        receipts.remove(&key);
    }
}

fn seal_journal_record(mut record: JournalRecord) -> Result<JournalRecord, RuntimeError> {
    record.checksum.clear();
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    record.checksum = sha256_hex(&bytes);
    Ok(record)
}

fn verify_journal_record(record: &JournalRecord) -> Result<(), RuntimeError> {
    if record.format_version != JOURNAL_VERSION {
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn append_journal_record(path: &Path, record: &JournalRecord) -> Result<(), RuntimeError> {
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

fn load_journal(path: &Path, after_sequence: u64) -> Result<Vec<JournalRecord>, RuntimeError> {
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

fn truncate_journal(path: &Path) -> Result<(), RuntimeError> {
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

/// Whether `project.cut` holds `expected`. Formatting and comments do not
/// matter; the parsed project does.
fn inspect_projection(path: &Path, expected: &Project) -> ProjectionStatus {
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

fn projection_matches(path: &Path, expected: &Project) -> bool {
    inspect_projection(path, expected) == ProjectionStatus::Current
}

fn projection_digest(path: &Path) -> Result<String, RuntimeError> {
    let bytes = fs::read(path).map_err(io_error)?;
    Ok(sha256_hex(&bytes))
}

/// What a directory's checkpoint holds.
enum Loaded {
    None,
    Current(Box<PersistedCheckpoint>),
    /// A checkpoint written by an older cutd.
    Previous,
}

fn load_checkpoint(directory: &Path) -> Result<Loaded, RuntimeError> {
    let checkpoint_path = directory.join(CHECKPOINT_FILE);
    let temporary_path = directory.join(CHECKPOINT_TEMP_FILE);
    for path in [&checkpoint_path, &temporary_path] {
        if path.is_file()
            && checkpoint_format(path).is_some_and(|format| format < CHECKPOINT_VERSION)
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
    if format_version != CHECKPOINT_VERSION {
        return Err(RuntimeError::new(
            "CHECKPOINT_VERSION_UNSUPPORTED",
            format!("checkpoint format {format_version} is not supported"),
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| RuntimeError::new("INVALID_CHECKPOINT", error.to_string()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), RuntimeError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
    write_bytes_atomic(path, &bytes)
}

/// Write through a synced temporary file and an atomic rename.
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
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

fn proposal_not_found(proposal_id: &str) -> RuntimeError {
    RuntimeError::new(
        "PROPOSAL_NOT_FOUND",
        format!("proposal not found: {proposal_id}"),
    )
}

fn io_error(error: std::io::Error) -> RuntimeError {
    RuntimeError::new("IO_ERROR", error.to_string())
}

fn validate_media_path(path: &str) -> Result<PathBuf, RuntimeError> {
    if path.trim().is_empty() {
        return Err(RuntimeError::new(
            "INVALID_IMPORT",
            "media path is required",
        ));
    }
    let source_path = PathBuf::from(path);
    let metadata = fs::metadata(&source_path).map_err(|error| {
        RuntimeError::new(
            "INVALID_IMPORT",
            format!(
                "media path is not a readable file: {} ({error})",
                source_path.display()
            ),
        )
    })?;
    if !metadata.is_file() {
        return Err(RuntimeError::new(
            "INVALID_IMPORT",
            format!("media path is not a file: {}", source_path.display()),
        ));
    }
    Ok(source_path)
}

fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}-{:x}", SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

fn engine_error(error: cut_engine::EngineError) -> RuntimeError {
    match error {
        cut_engine::EngineError::Cancelled => {
            RuntimeError::new("CANCELLED", "render job was cancelled")
        }
        other => RuntimeError::new("ENGINE_ERROR", other.to_string()),
    }
}

/// Read a `.cut` document, such as one a render job or a snapshot carries.
pub fn parse_document(text: &str) -> Result<Project, RuntimeError> {
    kernel::parse(text)
}

/// Run one render worker in-process. `document` is the `.cut` text a render
/// job carries; its first sequence is rendered, optionally only `range`.
/// Creating `cancel_file` cancels the render. The runtime remains the durable
/// owner of job state. `on_progress` runs on this worker thread and must not
/// block on UI state.
pub fn run_render<F>(
    id: String,
    document: String,
    output: PathBuf,
    draft: bool,
    range: Option<RenderRange>,
    cancel_file: Option<PathBuf>,
    mut on_progress: F,
) -> Result<(), RuntimeError>
where
    F: FnMut(f64),
{
    let project = kernel::parse(&document)?;
    let window = match range.as_ref() {
        Some(range) => {
            RenderRange::new(range.start, range.duration)?;
            Some(kernel::time_range(&project, range.start, range.end())?)
        }
        None => None,
    };
    let opts = cut_engine::render::RenderOptions {
        output: output.to_string_lossy().into_owned(),
        draft,
        ..cut_engine::render::RenderOptions::default()
    };
    let cancellation = cut_engine::CancellationToken::new();
    let watcher = cancel_file.as_ref().map(|path| {
        cut_engine::render::spawn_cancel_file_watcher(path.clone(), cancellation.clone())
    });
    let result = cut_engine::render::render_program_with_cancellation(
        &project,
        None,
        window,
        &opts,
        &cancellation,
        Some(&mut on_progress),
    );
    cancellation.cancel();
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
    match result {
        Ok(()) => {
            if output.is_file() {
                Ok(())
            } else {
                Err(RuntimeError::new(
                    "ENGINE_ERROR",
                    format!(
                        "render job {id} completed without producing {}",
                        output.display()
                    ),
                ))
            }
        }
        Err(cut_engine::EngineError::Cancelled) => Err(RuntimeError::new(
            "CANCELLED",
            format!("render job {id} was cancelled"),
        )),
        Err(error) => Err(engine_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> Runtime {
        let directory = std::env::temp_dir().join(format!("cut-runtime-test-{}", unique_id()));
        Runtime::open(&directory).unwrap()
    }

    /// Apply `script` at `revision` under a fresh request key.
    fn edit(
        runtime: &mut Runtime,
        script: &str,
        revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        runtime.apply_script_request("ui", &unique_id(), script, revision)
    }

    fn undo(runtime: &mut Runtime, revision: u64) -> Result<StateSnapshot, RuntimeError> {
        runtime.undo_request("ui", &unique_id(), revision)
    }

    /// Resize the first sequence with a typed operation.
    fn resize(runtime: &mut Runtime, width: u32) -> StateSnapshot {
        let ops = vec![EditOp::SetSequence {
            width,
            height: 1080,
            sequence: None,
        }];
        let revision = runtime.state.revision;
        runtime
            .apply_ops_request("ui", &unique_id(), ops, revision)
            .unwrap()
    }

    fn has_marker(state: &StateSnapshot, id: &str) -> bool {
        state.document.contains(&format!("marker {id} "))
    }

    fn pending_id(state: &StateSnapshot) -> String {
        match state.pending_proposal.as_ref() {
            Some(PendingProposal::Pending { proposal_id, .. }) => proposal_id.clone(),
            other => panic!("expected a pending proposal, got {other:?}"),
        }
    }

    #[test]
    fn edits_are_atomic_and_revisioned() {
        let mut runtime = runtime();
        let state = edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
        assert_eq!(state.revision, 1);
        assert!(state.can_undo);
        let error = edit(&mut runtime, "marker add at=0f\ntrack remove missing", 1).unwrap_err();
        assert_eq!(error.code, "EDIT_REJECTED");
        assert_eq!(runtime.snapshot().revision, 1);
        assert!(!has_marker(&runtime.snapshot(), "mk1"));
        undo(&mut runtime, 1).unwrap();
        assert!(runtime.snapshot().can_redo);
        runtime.redo_request("ui", "redo", 2).unwrap();
        assert_eq!(runtime.snapshot().revision, 3);
    }

    #[test]
    fn conflicts_are_machine_readable() {
        let mut runtime = runtime();
        let error = undo(&mut runtime, 2).unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(error.actual_revision, Some(0));
    }

    #[test]
    fn one_runtime_owns_a_project_directory() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-lock-{}", unique_id()));
        let runtime = Runtime::open(&directory).unwrap();
        let error = Runtime::open(&directory).unwrap_err();
        assert_eq!(error.code, "RUNTIME_ALREADY_RUNNING");
        drop(runtime);
        let reopened = Runtime::open(&directory).unwrap();
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn idempotency_replays_only_the_same_request() {
        let mut runtime = runtime();
        let first = runtime
            .apply_script_request("ui", "key-1", "track add T1 subtitle", 0)
            .unwrap();
        let replay = runtime
            .apply_script_request("ui", "key-1", "track add T1 subtitle", 0)
            .unwrap();
        assert_eq!(first.revision, replay.revision);
        let error = runtime
            .apply_script_request("ui", "key-1", "track remove T1", 1)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn observation_reference_survives_proposal_restart() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-observation-{}", unique_id()));
        fs::create_dir_all(&directory).expect("observation directory");
        let source = directory.join("raw-source.mp4");
        fs::write(&source, b"raw source").expect("raw source");
        let identity = SourceIdentity::from_path(&source).expect("source identity");
        let observation_identity = ObservationSourceIdentity {
            path: identity.path,
            size: identity.size,
            modified_ns: identity.modified_ns,
        };
        let coarse_observation = ObservationReference {
            id: "observation-coarse".into(),
            source_ref: "raw-source".into(),
            source_path: source.to_string_lossy().into_owned(),
            source_identity: observation_identity.clone(),
            range: ProposalTimeRange {
                start: 0.0,
                end: 2.0,
            },
            evidence: vec!["filmstrip".into()],
            artifact_keys: vec!["filmstrip-key".into()],
            revision: None,
            phase: Some("coarse".into()),
            clip_id: None,
            track_id: None,
            timeline_range: None,
            selection_index: None,
            rationale: None,
            coarse_observation: None,
        };
        let observation = ObservationReference {
            id: "observation-test".into(),
            source_ref: "raw-source".into(),
            source_path: source.to_string_lossy().into_owned(),
            source_identity: observation_identity,
            range: ProposalTimeRange {
                start: 4.0,
                end: 8.0,
            },
            evidence: vec!["frames".into()],
            artifact_keys: vec!["frames-key".into()],
            revision: None,
            phase: Some("targeted".into()),
            clip_id: Some("clip-1".into()),
            track_id: Some("video-1".into()),
            timeline_range: Some(ProposalTimeRange {
                start: 0.0,
                end: 4.0,
            }),
            selection_index: Some(2),
            rationale: Some("raw source evidence is retained".into()),
            coarse_observation: Some(Box::new(coarse_observation)),
        };
        {
            let mut runtime = Runtime::open(&directory).expect("runtime");
            let staged = runtime
                .stage_script_proposal(
                    "agent-observation",
                    0,
                    "track add T1 subtitle",
                    vec![serde_json::to_value(&observation).unwrap()],
                )
                .expect("proposal");
            assert!(matches!(
                staged.pending_proposal,
                Some(PendingProposal::Pending {
                    observation: Some(_),
                    ..
                })
            ));
        }
        let mut reopened = Runtime::open(&directory).expect("reopen runtime");
        let (proposal_id, observation) = match reopened.snapshot().pending_proposal {
            Some(PendingProposal::Pending {
                proposal_id,
                observation,
                ..
            }) => (proposal_id, observation.expect("observation")),
            other => panic!("expected pending proposal, got {other:?}"),
        };
        assert_eq!(observation.id, "observation-test");
        assert_eq!(observation.range.end, 8.0);
        let accepted = reopened
            .accept_proposal(&proposal_id, "ui", "accept-observation-test", 0)
            .expect("accept proposal");
        match accepted.pending_proposal {
            Some(PendingProposal::Accepted {
                observation: Some(observation),
                ..
            }) => {
                assert_eq!(
                    observation.rationale.as_deref(),
                    Some("raw source evidence is retained")
                );
                assert_eq!(observation.selection_index, Some(2));
                assert_eq!(
                    observation
                        .coarse_observation
                        .as_deref()
                        .map(|value| value.id.as_str()),
                    Some("observation-coarse")
                );
            }
            other => panic!("expected accepted observation, got {other:?}"),
        }
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn accepting_a_project_observation_rechecks_file_identity() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-observation-stale-{}", unique_id()));
        fs::create_dir_all(&directory).expect("observation directory");
        let source = directory.join("source.mp4");
        fs::write(&source, b"source").expect("source");
        fs::write(
            directory.join(PROJECT_FILE),
            format!(
                "cut 1\nproject name=\"Observed\"\n\n\
                 media source-1 path=\"{}\" rate=24 dur=00:00:08:00 video=1920x1080\n\n\
                 sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n",
                source.display()
            ),
        )
        .expect("project");
        let identity = SourceIdentity::from_path(&source).expect("source identity");
        let observation = ObservationReference {
            id: "observation-stale".into(),
            source_ref: "source-1".into(),
            source_path: source.to_string_lossy().into_owned(),
            source_identity: ObservationSourceIdentity {
                path: identity.path,
                size: identity.size,
                modified_ns: identity.modified_ns,
            },
            range: ProposalTimeRange {
                start: 1.0,
                end: 3.0,
            },
            evidence: vec!["frames".into()],
            artifact_keys: Vec::new(),
            revision: Some(0),
            phase: Some("targeted".into()),
            clip_id: None,
            track_id: None,
            timeline_range: None,
            selection_index: Some(0),
            rationale: Some("the subject is clearly visible".into()),
            coarse_observation: None,
        };
        let mut runtime = Runtime::open(&directory).expect("runtime");
        let staged = runtime
            .stage_script_proposal(
                "observation-stale",
                0,
                "track add T1 subtitle",
                vec![serde_json::to_value(&observation).unwrap()],
            )
            .expect("proposal");
        let proposal_id = pending_id(&staged);
        let before = runtime.snapshot();
        fs::write(&source, b"changed source with a new size").expect("replace source");
        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-observation-stale", 0)
            .expect_err("changed source must reject acceptance");
        assert_eq!(error.code, "STALE_OBSERVATION");
        assert_eq!(runtime.snapshot().document, before.document);
        assert!(matches!(
            runtime.snapshot().pending_proposal,
            Some(PendingProposal::Pending { .. })
        ));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn rejecting_a_proposal_keeps_canonical_state_and_converges() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let staged = runtime
            .stage_script_proposal("agent-reject", 0, "track add T1 subtitle", Vec::new())
            .unwrap();
        assert!(matches!(
            staged.pending_proposal,
            Some(PendingProposal::Pending {
                freshness: ProposalFreshness::Current,
                ..
            })
        ));
        let proposal_id = pending_id(&staged);
        let rejected = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(rejected.document, before.document);
        assert_eq!(rejected.revision, before.revision);
        assert!(!rejected.can_undo);
        assert!(matches!(
            rejected.pending_proposal,
            Some(PendingProposal::Rejected { .. })
        ));
        let replay = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(replay.document, before.document);
        assert_eq!(replay.revision, before.revision);
    }

    #[test]
    fn accepting_a_proposal_is_one_revision_and_one_undo_entry() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "agent-accept",
                0,
                "track add T1 subtitle\nmarker add at=0f as=intent",
                Vec::new(),
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let accepted = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(accepted.revision, 1);
        assert!(accepted.can_undo);
        assert!(has_marker(&accepted, "intent"));
        assert!(matches!(
            accepted.pending_proposal,
            Some(PendingProposal::Accepted {
                applied_revision: 1,
                ..
            })
        ));
        let replay = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(replay.revision, 1);
        undo(&mut runtime, 1).unwrap();
        assert_eq!(runtime.snapshot().revision, 2);
        assert!(!runtime.document(false).contains("track T1"));
    }

    #[test]
    fn accepted_proposal_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "agent-receipt",
                0,
                "marker add at=0f as=approved",
                Vec::new(),
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let first = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        let later = edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
        assert_eq!(later.revision, 2);

        let replay = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        assert!(has_marker(&replay, "approved"));
        assert!(!has_marker(&replay, "later"));

        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn accepting_a_stale_proposal_conflicts_without_clearing_it() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal("agent-stale", 0, "track add T1 subtitle", Vec::new())
            .unwrap();
        let proposal_id = pending_id(&staged);
        edit(&mut runtime, "marker add at=0f as=changed", 0).unwrap();
        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-stale", 0)
            .unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert!(matches!(
            runtime.snapshot().pending_proposal,
            Some(PendingProposal::Pending {
                freshness: ProposalFreshness::Stale,
                ..
            })
        ));
    }

    #[test]
    fn invalid_proposal_scripts_stage_nothing() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let error = runtime
            .stage_script_proposal(
                "agent-invalid",
                0,
                "track add T1 subtitle\ntrack remove missing",
                Vec::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, "EDIT_REJECTED");
        let after = runtime.snapshot();
        assert_eq!(after.document, before.document);
        assert_eq!(after.revision, before.revision);
        assert!(after.pending_proposal.is_none());
    }

    #[test]
    fn revision_and_history_survive_a_runtime_restart() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-restart-{}", unique_id()));
        {
            let mut runtime = Runtime::open(&directory).unwrap();
            edit(&mut runtime, "track add T1 subtitle", 0).unwrap();
            assert_eq!(runtime.snapshot().revision, 1);
        }
        let mut reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, 1);
        assert!(reopened.snapshot().can_undo);
        undo(&mut reopened, 1).unwrap();
        assert_eq!(reopened.snapshot().revision, 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn media_import_is_a_revisioned_idempotent_transaction() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-import-{}", unique_id()));
        let media = directory.join("sample.mp4");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&media, b"not decoded in this unit test").unwrap();
        {
            let mut runtime = Runtime::open(&directory).unwrap();
            let (first, media_id, clip_id) = runtime
                .import_media_request("ui", "import-1", media.to_str().unwrap(), 0)
                .unwrap();
            let (replay, replay_media, replay_clip) = runtime
                .import_media_request("ui", "import-1", media.to_str().unwrap(), 0)
                .unwrap();
            assert_eq!(first.revision, 1);
            assert_eq!(first.revision, replay.revision);
            assert_eq!((&media_id, &clip_id), (&replay_media, &replay_clip));
            // Unprobed media is a second of picture at the sequence size,
            // cut in on a new picture track.
            assert!(
                first.document.contains(&format!(
                    "media {media_id} path=\"{}\" rate=30 dur=00:00:01:00 video=1920x1080",
                    media.display()
                )),
                "{}",
                first.document
            );
            assert!(
                first.document.contains("track V1 video"),
                "{}",
                first.document
            );
            assert!(
                first
                    .document
                    .contains(&format!("clip {clip_id} media={media_id}")),
                "{}",
                first.document
            );
            let error = runtime
                .import_media_request("ui", "import-1", "other.mp4", 1)
                .unwrap_err();
            assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn imported_media_with_sound_gets_a_linked_clip() {
        let project = kernel::empty_project();
        let probe = cut_engine::ProbeInfo {
            duration: Some(2.0),
            width: Some(1280),
            height: Some(720),
            fps: Some(24.0),
            has_audio: Some(true),
            video_codec: None,
            audio_codec: None,
        };
        let (ops, media, clip) =
            kernel::import_media(&project, "/media/a.mov", Some(&probe)).unwrap();
        let edited = kernel::apply_ops(&project, &ops).unwrap();
        let document = kernel::print(&edited.project);
        assert!(
            document.contains(&format!(
                "media {media} path=\"/media/a.mov\" rate=24 dur=00:00:02:00 video=1280x720 audio=2ch@48000"
            )),
            "{document}"
        );
        assert!(document.contains("track A1 audio"), "{document}");
        assert_eq!(document.matches("link=").count(), 2, "{document}");
        assert!(edited.project.clip(clip.as_str()).is_some());
    }

    #[test]
    fn import_rejects_paths_that_are_not_files() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-import-boundary-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let mut runtime = Runtime::open(&directory).unwrap();
        let missing = runtime
            .import_media_request("ui", "missing", "/definitely/not-a-file", 0)
            .unwrap_err();
        assert_eq!(missing.code, "INVALID_IMPORT");
        let directory_error = runtime
            .import_media_request("ui", "directory", directory.to_str().unwrap(), 0)
            .unwrap_err();
        assert_eq!(directory_error.code, "INVALID_IMPORT");
        assert_eq!(runtime.snapshot().revision, 0);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_journal_recovers_after_projection_failure() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-persist-fail-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before = runtime.snapshot();
        let before_document = runtime.document(false);
        // The projection is deliberately made unrenamable. The journal commit
        // must still advance the authoritative runtime state and survive a
        // restart after the old projection is restored.
        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
        let committed = edit(&mut runtime, "track add T1 subtitle", before.revision).unwrap();
        assert_eq!(committed.revision, before.revision + 1);
        assert_eq!(committed.projection_status, "diverged");
        assert!(directory.join(JOURNAL_FILE).is_file());
        drop(runtime);
        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        fs::write(directory.join(PROJECT_FILE), &before_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, committed.revision);
        assert!(reopened.document(false).contains("track T1 subtitle"));
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn journal_tail_recovery_keeps_external_projection_edits_divergent() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-tail-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let first = edit(&mut runtime, "marker add at=0f as=first", 0).unwrap();
        let first_document = runtime.document(false);

        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
        let second = edit(&mut runtime, "marker add at=0f as=second", first.revision).unwrap();
        assert_eq!(second.projection_status, "diverged");
        drop(runtime);

        // The projection is the last successfully published journal state,
        // while the second mutation remains durable in the journal.
        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        fs::write(directory.join(PROJECT_FILE), &first_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, second.revision);
        assert!(has_marker(&reopened.snapshot(), "second"));
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);

        // An edit that does not match any recognized journal state must remain
        // visible on disk and keep subsequent mutations blocked.
        let edited = fs::read_to_string(directory.join(PROJECT_FILE))
            .unwrap()
            .replace(
                "project name=\"Untitled project\"",
                "project name=\"external edit\"",
            );
        fs::write(directory.join(PROJECT_FILE), edited).unwrap();
        let mut diverged = Runtime::open(&directory).unwrap();
        assert_eq!(diverged.snapshot().projection_status, "diverged");
        assert_eq!(
            edit(
                &mut diverged,
                "marker add at=0f as=blocked",
                second.revision
            )
            .unwrap_err()
            .code,
            "PROJECTION_DIVERGED"
        );
        let on_disk = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
        assert!(on_disk.contains("project name=\"external edit\""));
        drop(diverged);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn diverged_projection_keeps_checkpoint_threshold_journal_for_recovery() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-threshold-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before = runtime.snapshot();
        let before_document = runtime.document(false);
        runtime.journal_records_since_checkpoint = CHECKPOINT_INTERVAL - 1;
        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();

        let committed = edit(
            &mut runtime,
            "marker add at=0f as=threshold",
            before.revision,
        )
        .unwrap();
        assert_eq!(committed.projection_status, "diverged");
        let checkpoint: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap())
                .unwrap();
        assert_eq!(checkpoint["revision"], before.revision);
        assert!(fs::metadata(directory.join(JOURNAL_FILE)).unwrap().len() > 0);
        drop(runtime);

        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        fs::write(directory.join(PROJECT_FILE), &before_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, committed.revision);
        assert!(has_marker(&reopened.snapshot(), "threshold"));
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_startup_projection_repair_reports_project_status() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-repair-status-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before_document = runtime.document(false);
        let committed = resize(&mut runtime, 1280);
        drop(runtime);

        // Put back the last checkpointed document so journal replay
        // recognizes it as an internal stale projection and repairs it.
        fs::write(directory.join(PROJECT_FILE), before_document).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, committed.revision);
        assert_eq!(reopened.snapshot().timeline["width"], 1280);
        assert_eq!(reopened.snapshot().projection_status, "current");
        assert!(fs::read_to_string(directory.join(PROJECT_FILE))
            .unwrap()
            .contains("size=1280x1080"));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn operation_receipts_and_revision_survive_restart() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-receipt-{}", unique_id()));
        let first = {
            let mut runtime = Runtime::open(&directory).unwrap();
            runtime
                .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
                .unwrap()
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        let replay = reopened
            .apply_script_request("ui", "request-1", "track add T1 subtitle", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        let conflict = reopened
            .apply_script_request("ui", "request-1", "marker add at=0f", 1)
            .unwrap_err();
        assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pending_proposal_lifecycle_survives_restart() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-proposal-restart-{}", unique_id()));
        let proposal_id = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let staged = runtime
                .stage_script_proposal(
                    "agent-restart",
                    0,
                    "marker add at=0f as=survives",
                    Vec::new(),
                )
                .unwrap();
            pending_id(&staged)
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        assert!(matches!(
            reopened.snapshot().pending_proposal,
            Some(PendingProposal::Pending { script, document_diff, .. })
                if script == "marker add as=survives at=00:00:00:00\n"
                    && document_diff.contains("+  marker survives at=00:00:00:00")
        ));
        let accepted = reopened
            .accept_proposal(&proposal_id, "ui", "accept-restart", 0)
            .unwrap();
        assert_eq!(accepted.revision, 1);
        assert!(matches!(
            accepted.pending_proposal,
            Some(PendingProposal::Accepted { .. })
        ));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn accepted_proposal_receipt_survives_restart() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-accepted-restart-{}", unique_id()));
        let proposal_id = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let staged = runtime
                .stage_script_proposal(
                    "agent-accepted-restart",
                    0,
                    "marker add at=0f as=persisted",
                    Vec::new(),
                )
                .unwrap();
            let proposal_id = pending_id(&staged);
            runtime
                .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
                .unwrap();
            proposal_id
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        edit(&mut reopened, "marker add at=0f as=later", 1).unwrap();
        let replay = reopened
            .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
            .unwrap();
        assert_eq!(replay.revision, 1);
        assert!(has_marker(&replay, "persisted"));
        assert!(!has_marker(&replay, "later"));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_lock_file_is_reusable_after_process_exit() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-stale-lock-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(LOCK_FILE), b"dead pid").unwrap();
        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().revision, 0);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn mailbox_serializes_two_clients_against_one_revision() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-mailbox-{}", unique_id()));
        let handle = RuntimeHandle::open(&directory).unwrap();
        let apply = |actor: &str, key: &str, script: &str| {
            let handle = handle.clone();
            let request = RuntimeRequest::ApplyScript {
                actor: actor.into(),
                idempotency_key: key.into(),
                base_revision: 0,
                script: script.into(),
            };
            thread::spawn(move || handle.request(request).and_then(expect_state))
        };
        let first = apply("ui", "left", "track add T1 video");
        let second = apply("agent", "right", "track add T2 video");
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
        let error = if let Err(error) = first {
            error
        } else {
            second.unwrap_err()
        };
        assert_eq!(error.code, "REVISION_CONFLICT");
        let state = expect_state(handle.request(RuntimeRequest::GetState).unwrap()).unwrap();
        assert_eq!(state.revision, 1);
        drop(handle);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pending_proposal_serializes_gui_field_names_exactly() {
        let pending = serde_json::to_value(PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 2,
            script: "trim c1 out -12f\n".into(),
            document_diff: "-a\n+b\n".into(),
            freshness: ProposalFreshness::Current,
            observation: None,
            observations: None,
        })
        .unwrap();
        assert_eq!(
            pending,
            json!({
                "status": "pending",
                "proposalId": "proposal-1",
                "baseRevision": 2,
                "script": "trim c1 out -12f\n",
                "documentDiff": "-a\n+b\n"
            })
        );
        assert_eq!(
            serde_json::to_value(PendingProposal::Accepted {
                proposal_id: "proposal-1".into(),
                base_revision: 2,
                applied_revision: 3,
                observation: None,
                observations: None,
            })
            .unwrap(),
            json!({
                "status":"accepted",
                "proposalId":"proposal-1",
                "baseRevision":2,
                "appliedRevision":3
            })
        );
    }

    #[test]
    fn jsonl_rejects_empty_idempotency_keys_at_runtime_boundary() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-jsonl-{}", unique_id()));
        let requests = [
            json!({
                "protocolVersion":1,
                "id":"script-empty",
                "operation":"apply_script",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0,
                "script":"marker add at=0f"
            }),
            json!({
                "protocolVersion":1,
                "id":"undo-empty",
                "operation":"undo",
                "actor":"ui",
                "idempotencyKey":"   ",
                "baseRevision":0
            }),
            json!({
                "protocolVersion":1,
                "id":"redo-empty",
                "operation":"redo",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0
            }),
            json!({
                "protocolVersion":1,
                "id":"import-empty",
                "operation":"import",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0,
                "path":"/definitely/not-a-file"
            }),
            json!({
                "protocolVersion":1,
                "id":"accept-empty",
                "operation":"accept_proposal",
                "proposalId":"proposal-missing",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0
            }),
            json!({
                "protocolVersion":1,
                "id":"reject-empty",
                "operation":"reject_proposal",
                "proposalId":"proposal-missing",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0
            }),
        ];
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = Vec::new();
        run_stdio(
            &directory,
            std::io::Cursor::new(format!("{input}\n")),
            &mut output,
        )
        .unwrap();
        let responses = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), requests.len());
        for (response, request) in responses.iter().zip(requests.iter()) {
            assert_eq!(response["id"], request["id"]);
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"]["code"], "INVALID_REQUEST");
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_rejection_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "agent-reject-receipt",
                0,
                "marker add at=0f as=proposed",
                Vec::new(),
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let first = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        edit(&mut runtime, "marker add at=0f as=later", first.revision).unwrap();
        let replay = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.document, first.document);
        let error = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn journal_stores_scripts_without_full_snapshot_history() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-shape-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "track add titles video", 0).unwrap();
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        let record: JournalRecord = serde_json::from_str(journal.trim()).unwrap();
        let change = record.change.unwrap();
        assert_eq!(change.script.as_deref(), Some("track add titles video\n"));
        assert!(change.document.is_none());

        for index in 0..(MAX_HISTORY_ENTRIES + 8) {
            resize(&mut runtime, 1000 + 2 * index as u32);
        }
        assert_eq!(runtime.state.past.len(), MAX_HISTORY_ENTRIES);
        assert!(runtime.state.future.is_empty());
        drop(runtime);
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.state.past.len(), MAX_HISTORY_ENTRIES);
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn checkpoint_and_torn_journal_tail_recover_authoritative_state() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-recovery-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        for index in 0..(CHECKPOINT_INTERVAL + 1) {
            resize(&mut runtime, 1000 + 2 * index as u32);
        }
        let checkpoint: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(CHECKPOINT_FILE)).unwrap())
                .unwrap();
        assert_eq!(checkpoint["journal_sequence"], CHECKPOINT_INTERVAL);
        assert_eq!(
            checkpoint["past"].as_array().unwrap().len(),
            CHECKPOINT_INTERVAL as usize
        );
        drop(runtime);

        let mut journal = OpenOptions::new()
            .append(true)
            .open(directory.join(JOURNAL_FILE))
            .unwrap();
        journal.write_all(br#"{"#).unwrap();
        journal.sync_all().unwrap();

        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, CHECKPOINT_INTERVAL + 1);
        assert_eq!(
            reopened.snapshot().timeline["width"],
            1000 + 2 * CHECKPOINT_INTERVAL
        );
        drop(reopened);
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        assert!(journal
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn interrupted_checkpoint_write_recovers_from_complete_temp_file() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-checkpoint-recovery-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "marker add at=0f as=saved", 0).unwrap();
        drop(runtime);
        let checkpoint = fs::read(directory.join(CHECKPOINT_FILE)).unwrap();
        fs::write(directory.join(CHECKPOINT_TEMP_FILE), checkpoint).unwrap();
        fs::write(directory.join(CHECKPOINT_FILE), b"{broken").unwrap();

        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().revision, 1);
        assert!(has_marker(&runtime.snapshot(), "saved"));
        assert!(!directory.join(CHECKPOINT_TEMP_FILE).exists());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn first_launch_imports_project_json_once_and_leaves_it_untouched() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-v1-bootstrap-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let legacy = serde_json::to_vec(&json!({
            "version": 1,
            "fps": 24,
            "sources": [{"id":"src1","path":"clip.mp4","duration":4}],
            "tracks": [{"id":"v1","clips":[{"id":"clip1","src":"src1","in":0,"out":4}]}],
            "overlays": []
        }))
        .unwrap();
        fs::write(directory.join(LEGACY_PROJECT_FILE), &legacy).unwrap();
        let runtime = Runtime::open(&directory).unwrap();
        let state = runtime.snapshot();
        assert_eq!(state.source_version, SOURCE_VERSION);
        assert_eq!(state.projection_status, "current");
        assert!(state.import_warnings[0].contains("imported project.cut.json"));
        assert!(directory.join(CHECKPOINT_FILE).is_file());
        drop(runtime);
        let document = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
        assert!(document.starts_with("cut 1\n"), "{document}");
        assert!(document.contains("clip clip1 media=src1"), "{document}");
        assert_eq!(
            fs::read(directory.join(LEGACY_PROJECT_FILE)).unwrap(),
            legacy
        );

        // project.cut is canonical from now on: a later project.cut.json is
        // ignored.
        fs::write(directory.join(LEGACY_PROJECT_FILE), b"{}").unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().projection_status, "current");
        assert!(reopened.document(false).contains("clip clip1"));
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn state_from_an_older_runtime_is_preserved_and_the_project_reopened() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-old-state-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let document = kernel::print(&Project {
            name: "Earlier".into(),
            ..kernel::empty_project()
        });
        fs::write(directory.join(PROJECT_FILE), &document).unwrap();
        let old_checkpoint = br#"{"format_version":3,"project":"","revision":7}"#;
        fs::write(directory.join(CHECKPOINT_FILE), old_checkpoint).unwrap();
        fs::write(directory.join(JOURNAL_FILE), b"{\"format_version\":2}\n").unwrap();
        let runtime = Runtime::open(&directory).unwrap();
        let state = runtime.snapshot();
        assert_eq!(state.revision, 0);
        assert_eq!(state.document, document);
        assert!(state
            .import_warnings
            .iter()
            .any(|warning| warning.contains(PREVIOUS_STATE_PREFIX)));
        assert_eq!(
            fs::read(directory.join(format!("{PREVIOUS_STATE_PREFIX}.checkpoint.json"))).unwrap(),
            old_checkpoint
        );
        assert!(directory
            .join(format!("{PREVIOUS_STATE_PREFIX}.journal"))
            .is_file());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn divergent_projection_requires_repair_or_explicit_import() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-projection-policy-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        edit(&mut runtime, "marker add at=0f as=authoritative", 0).unwrap();
        drop(runtime);

        let original = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            original.replace("Untitled project", "human edit"),
        )
        .unwrap();

        let mut runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().projection_status, "diverged");
        let blocked = edit(&mut runtime, "marker add at=0f as=blocked", 1);
        assert_eq!(blocked.unwrap_err().code, "PROJECTION_DIVERGED");
        let repaired = runtime.repair_projection().unwrap();
        assert_eq!(repaired.projection_status, "current");
        assert_eq!(repaired.timeline["name"], "Untitled project");

        // A text edit of the canonical document, made outside cutd.
        fs::write(
            directory.join(PROJECT_FILE),
            original.replace("Untitled project", "explicit import")
                + "# a comment the import does not keep\n",
        )
        .unwrap();
        let imported_state = runtime
            .import_projection("ui", "projection-import-1", repaired.revision)
            .unwrap();
        assert_eq!(imported_state.revision, repaired.revision + 1);
        assert_eq!(imported_state.timeline["name"], "explicit import");
        assert!(!imported_state.can_undo);
        assert_eq!(imported_state.projection_status, "current");
        drop(runtime);

        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().timeline["name"], "explicit import");
        assert_eq!(reopened.snapshot().projection_status, "current");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn abandoned_render_jobs_are_reconciled_after_restart() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-job-recovery-{}", unique_id()));
        let job_id = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let (job, _, _) = runtime.start_render_range("preview", None, None).unwrap();
            assert_eq!(job.status, "queued");
            job.job_id
        };
        let runtime = Runtime::open(&directory).unwrap();
        let job = runtime.render_job(&job_id).unwrap();
        assert_eq!(job.status, "failed");
        assert_eq!(
            job.error.as_deref(),
            Some("render job was abandoned when cutd restarted")
        );
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn render_cancellation_is_job_scoped_and_survives_restart() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-job-cancel-{}", unique_id()));
        let (cancelled_id, queued_id) = {
            let mut runtime = Runtime::open(&directory).unwrap();
            let (cancelled, _, _) = runtime.start_render_range("preview", None, None).unwrap();
            let (queued, _, _) = runtime.start_render_range("preview", None, None).unwrap();
            let cancel_path = render_cancel_path(&directory, &cancelled.job_id);
            let queued_path = render_cancel_path(&directory, &queued.job_id);
            let cancelled_job = runtime.cancel_render(&cancelled.job_id).unwrap();
            assert_eq!(cancelled_job.status, "cancelled");
            assert!(cancel_path.is_file());
            assert!(!queued_path.is_file());
            assert_eq!(runtime.render_job(&queued.job_id).unwrap().status, "queued");
            (cancelled.job_id, queued.job_id)
        };

        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(
            runtime.render_job(&cancelled_id).unwrap().status,
            "cancelled"
        );
        assert_eq!(runtime.render_job(&queued_id).unwrap().status, "failed");
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn render_range_is_finite_and_bounded() {
        assert!(RenderRange::new(0.0, MAX_RENDER_RANGE_SECONDS).is_ok());
        assert_eq!(
            RenderRange::new(0.0, MAX_RENDER_RANGE_SECONDS + 0.1)
                .unwrap_err()
                .code,
            "INVALID_RENDER_RANGE"
        );
        assert!(RenderRange::new(f64::NAN, 1.0).is_err());
    }

    #[test]
    fn proposal_documents_reconstruct_before_and_after_without_mutating_state() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_script_proposal(
                "preview-proposal",
                0,
                "marker add at=0f as=title",
                Vec::new(),
            )
            .unwrap();
        let proposal_id = pending_id(&staged);
        let before_revision = runtime.snapshot().revision;
        let (before, after) = runtime.proposal_documents(&proposal_id).unwrap();
        assert!(!before.contains("marker title"));
        assert!(after.contains("marker title"));
        assert_eq!(runtime.snapshot().revision, before_revision);

        edit(&mut runtime, "marker add at=0f as=other", 0).unwrap();
        assert_eq!(
            runtime.proposal_documents(&proposal_id).unwrap_err().code,
            "PROPOSAL_STALE"
        );
    }

    /// A project with media, a picture clip, and its linked sound.
    fn kernel_runtime(label: &str) -> (Runtime, PathBuf) {
        let directory = std::env::temp_dir().join(format!("cut-runtime-{label}-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            "cut 1\nproject name=\"Kernel\"\n\n\
             media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\n\
             sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1\n",
        )
        .unwrap();
        (Runtime::open(&directory).unwrap(), directory)
    }

    #[test]
    fn renders_and_proposals_hand_out_documents_with_resolved_media() {
        let (mut runtime, directory) = kernel_runtime("render-documents");
        let media = directory.join("a.mov").to_string_lossy().into_owned();
        let snapshot = runtime.snapshot();
        assert!(snapshot.document.contains("path=\"a.mov\""));
        assert_eq!(
            snapshot.timeline["tracks"][0]["clips"][0]["durationFrames"],
            96
        );
        assert_eq!(snapshot.duration, 4.0);

        let (_, document, _) = runtime
            .start_render_range("preview", Some(RenderRange::new(1.0, 2.0).unwrap()), None)
            .unwrap();
        let rendered = parse_document(&document).unwrap();
        assert_eq!(rendered.media[0].path, media);
        assert_eq!(
            runtime
                .start_render_range("preview", Some(RenderRange::new(3.0, 2.0).unwrap()), None)
                .unwrap_err()
                .code,
            "INVALID_RENDER_RANGE"
        );

        let staged = runtime
            .stage_script_proposal(
                "trim",
                runtime.snapshot().revision,
                "trim c1 out -24f",
                Vec::new(),
            )
            .unwrap();
        let Some(PendingProposal::Pending { proposal_id, .. }) = staged.pending_proposal else {
            panic!("expected a pending proposal");
        };
        let (before, after) = runtime.proposal_documents(&proposal_id).unwrap();
        let (before, after) = (
            parse_document(&before).unwrap(),
            parse_document(&after).unwrap(),
        );
        assert_eq!(after.media[0].path, media);
        assert_eq!(kernel::program_duration(&before).seconds(), 4.0);
        assert_eq!(kernel::program_duration(&after).seconds(), 3.0);

        // A render of the proposal renders what it would make, not the project.
        let (_, document, _) = runtime
            .start_render_range(
                "preview",
                Some(RenderRange::new(1.0, 2.0).unwrap()),
                Some(&proposal_id),
            )
            .unwrap();
        let proposed = parse_document(&document).unwrap();
        assert_eq!(proposed.media[0].path, media);
        assert_eq!(kernel::program_duration(&proposed).seconds(), 3.0);
        assert_eq!(
            runtime
                .start_render_range(
                    "preview",
                    Some(RenderRange::new(2.0, 2.0).unwrap()),
                    Some(&proposal_id),
                )
                .unwrap_err()
                .code,
            "INVALID_RENDER_RANGE"
        );
        assert_eq!(
            runtime
                .start_render_range("preview", None, Some("proposal-0"))
                .unwrap_err()
                .code,
            "PROPOSAL_NOT_FOUND"
        );
    }

    #[test]
    fn typed_ops_apply_atomically_and_replay_by_key() {
        let (mut runtime, directory) = kernel_runtime("ops");
        let ops: Vec<EditOp> = serde_json::from_value(json!([
            {"op": "split", "clip": "c1", "at": 48, "tail": "c2"},
            {"op": "set", "clip": "c2", "props": {"opacity": 0.25}},
            {"op": "add-marker", "at": 12, "name": "look", "color": "green"}
        ]))
        .unwrap();
        let first = runtime
            .apply_ops_request("ui", "ops-1", ops.clone(), 0)
            .unwrap();
        assert_eq!(first.revision, 1);
        let replay = runtime.apply_ops_request("ui", "ops-1", ops, 0).unwrap();
        assert_eq!(replay.revision, 1);
        assert!(runtime.document(false).contains("set opacity=0.25"));
        assert_eq!(
            runtime.state.past[0].script,
            "split c1 at=00:00:02:00 as=c2\nset c2 opacity=0.25\nmarker add at=00:00:00:12 name=\"look\" color=green\n"
        );
        // Any failing operation leaves the project unchanged.
        let failing: Vec<EditOp> = serde_json::from_value(json!([
            {"op": "trim", "clip": "c1", "edge": "out", "frames": -6},
            {"op": "slip", "clip": "missing", "frames": 1}
        ]))
        .unwrap();
        let before = runtime.document(false);
        let error = runtime
            .apply_ops_request("ui", "ops-2", failing, 1)
            .unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT");
        assert_eq!(runtime.document(false), before);
        assert_eq!(runtime.snapshot().revision, 1);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn editor_ops_add_named_tracks_and_resize_the_sequence() {
        let (mut runtime, directory) = kernel_runtime("editor-ops");
        let ops: Vec<EditOp> = serde_json::from_value(json!([
            {"op": "add-track", "kind": "audio", "name": "Music"},
            {"op": "add-track", "kind": "video", "id": "titles"},
            {"op": "set-sequence", "width": 1080, "height": 1920}
        ]))
        .unwrap();
        runtime.apply_ops_request("ui", "ops", ops, 0).unwrap();
        let document = runtime.document(false);
        assert!(
            document.contains("track A2 audio name=\"Music\""),
            "{document}"
        );
        assert!(document.contains("track titles video"), "{document}");
        assert!(document.contains("size=1080x1920"), "{document}");
        let unknown: Vec<EditOp> =
            serde_json::from_value(json!([{"op": "add-track", "kind": "title"}])).unwrap();
        let error = runtime
            .apply_ops_request("ui", "bad", unknown, 1)
            .unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT");
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn script_proposals_show_a_document_diff_and_apply_on_acceptance() {
        let (mut runtime, directory) = kernel_runtime("script-proposal");
        let staged = runtime
            .stage_script_proposal("agent-1", 0, "trim c1 out -1s ripple", Vec::new())
            .unwrap();
        assert_eq!(staged.revision, 0);
        let pending = serde_json::to_value(staged.pending_proposal.unwrap()).unwrap();
        assert_eq!(pending["status"], "pending");
        assert_eq!(pending["script"], "trim c1 out -24f ripple\n");
        let diff = pending["documentDiff"].as_str().unwrap();
        assert!(
            diff.contains(
                "-    clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1"
            ),
            "{diff}"
        );
        assert!(
            diff.contains(
                "+    clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:03:00 link=L1"
            ),
            "{diff}"
        );
        let proposal_id = pending["proposalId"].as_str().unwrap().to_owned();

        drop(runtime);
        let mut runtime = Runtime::open(&directory).unwrap();
        let accepted = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(accepted.revision, 1);
        assert!(runtime.document(false).contains("dur=00:00:03:00 link=L1"));
        assert_eq!(runtime.state.past.len(), 1);
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn history_snapshots_survive_checkpoints() {
        let (mut runtime, directory) = kernel_runtime("history-checkpoint");
        for index in 0..(CHECKPOINT_INTERVAL + 2) {
            runtime
                .apply_script_request(
                    "ui",
                    &format!("marker-{index}"),
                    &format!("marker add at={index}f"),
                    index,
                )
                .unwrap();
        }
        let expected = runtime.document(false);
        drop(runtime);
        let mut reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.document(false), expected);
        let revision = reopened.snapshot().revision;
        for step in 0..(CHECKPOINT_INTERVAL + 2) {
            undo(&mut reopened, revision + step).unwrap();
        }
        let undone = reopened.document(false);
        assert!(!undone.contains("marker mk"), "{undone}");
        // Undone markers keep their ids retired.
        assert!(undone.contains("retired=mk1,"), "{undone}");
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn replay_refuses_a_record_that_does_not_reproduce_its_project() {
        let (mut runtime, directory) = kernel_runtime("replay-digest");
        runtime
            .apply_script_request("ui", "trim", "trim c1 out -12f", 0)
            .unwrap();
        drop(runtime);
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        let mut record: JournalRecord = serde_json::from_str(journal.trim()).unwrap();
        record.change.as_mut().unwrap().script = Some("trim c1 out -6f\n".into());
        let record = seal_journal_record(record).unwrap();
        fs::write(
            directory.join(JOURNAL_FILE),
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let error = Runtime::open(&directory).unwrap_err();
        assert_eq!(error.code, "JOURNAL_REPLAY_MISMATCH");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn get_document_returns_the_canonical_and_annotated_text() {
        let (mut runtime, directory) = kernel_runtime("document");
        match runtime
            .execute(RuntimeRequest::GetDocument { annotated: true })
            .unwrap()
        {
            RuntimeResponse::Document { revision, document } => {
                assert_eq!(revision, 0);
                assert!(document.contains("# ends 00:00:04:00"), "{document}");
            }
            other => panic!("expected a document, got {other:?}"),
        }
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }
}
