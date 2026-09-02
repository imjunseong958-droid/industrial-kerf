//! Desktop-owned editing runtime.
//!
//! This is deliberately a semantic boundary.  UI and MCP clients send project
//! commands and receive snapshots; they never mutate a local copy of the
//! timeline. `cutd` owns one `RuntimeHandle` and serves those clients through
//! the project-scoped IPC transport below.

use cut_domain::{self, Project};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

const PROJECT_FILE: &str = "project.cut.json";
const HISTORY_FILE: &str = "project.cut.history.json";
const JOURNAL_FILE: &str = ".cut-runtime.journal";
const LOCK_FILE: &str = ".cut-runtime.lock";
const CHECKPOINT_FILE: &str = ".cut-runtime.checkpoint.json";
const CHECKPOINT_TEMP_FILE: &str = ".cut-runtime.checkpoint.json.tmp";
const CHECKPOINT_VERSION: u64 = 2;
const JOURNAL_VERSION: u64 = 1;
const CHECKPOINT_INTERVAL: u64 = 32;
const MAX_HISTORY_ENTRIES: usize = 256;
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
    /// Derive the endpoint from the canonical project directory.
    pub fn for_project_directory(directory: &Path) -> Result<Self, RuntimeError> {
        let canonical = canonical_project_directory(directory)?;
        Ok(Self::for_canonical_directory(&canonical))
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalHistoryEntry {
    command: cut_domain::Command,
    inverse: cut_domain::Command,
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
    command: Option<cut_domain::Command>,
    #[serde(default)]
    inverse: Option<cut_domain::Command>,
    #[serde(default)]
    operation: Option<JournalOperation>,
    #[serde(default)]
    import: Option<JournalImport>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
    #[serde(default)]
    job: Option<RenderJob>,
    #[serde(default)]
    project: Option<Project>,
    #[serde(default)]
    source_version: Option<u8>,
    checksum: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCheckpoint {
    format_version: u64,
    project: Project,
    revision: u64,
    #[serde(default)]
    past: Vec<JournalHistoryEntry>,
    #[serde(default)]
    future: Vec<JournalHistoryEntry>,
    #[serde(default)]
    journal_sequence: u64,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
    #[serde(default)]
    jobs: HashMap<String, RenderJob>,
    #[serde(default = "default_source_version")]
    source_version: u8,
    #[serde(default)]
    projection_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegacyPersistedCheckpoint {
    project: Project,
    revision: u64,
    #[serde(default)]
    operations: HashMap<String, OperationRecord>,
    #[serde(default)]
    imports: HashMap<String, ImportRecord>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
}

fn default_source_version() -> u8 {
    2
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub project: Value,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub duration: f64,
    pub sources: Vec<Value>,
    pub pending_proposal: Option<PendingProposal>,
    #[serde(default = "default_source_version")]
    pub source_version: u8,
    #[serde(default = "default_projection_status")]
    pub projection_status: String,
}

/// One machine-readable leaf change in an agent proposal.  The diff is
/// intentionally structural rather than a rendered image: it remains cheap
/// to inspect, deterministic, and useful even when no media renderer exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalChange {
    pub path: String,
    pub kind: String,
    pub before: Value,
    pub after: Value,
}

/// A finite time interval carried by a semantic proposal impact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalTimeRange {
    pub start: f64,
    pub end: f64,
}

/// A video-domain meaning derived from a staged typed command.
///
/// The raw [`ProposalChange`] list remains available for diagnostics and wire
/// compatibility. These impacts are the stable surface that native clients
/// use for review and timeline visualization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProposalImpact {
    TrackAdded {
        track_id: String,
        index: usize,
    },
    TrackRemoved {
        track_id: String,
        index: usize,
    },
    TrackReordered {
        track_id: String,
        from_index: usize,
        to_index: usize,
    },
    ClipAdded {
        clip_id: String,
        track_id: String,
        timeline: ProposalTimeRange,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<ProposalTimeRange>,
    },
    ClipRemoved {
        clip_id: String,
        track_id: String,
        timeline: ProposalTimeRange,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<ProposalTimeRange>,
    },
    ClipMoved {
        clip_id: String,
        from_track_id: String,
        from: ProposalTimeRange,
        to_track_id: String,
        to: ProposalTimeRange,
    },
    ClipTrimmed {
        clip_id: String,
        track_id: String,
        before: ProposalTimeRange,
        after: ProposalTimeRange,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_before: Option<ProposalTimeRange>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_after: Option<ProposalTimeRange>,
    },
    ClipPropertyChanged {
        clip_id: String,
        track_id: String,
        property: String,
        before: Value,
        after: Value,
    },
    OverlayAdded {
        overlay_id: String,
        index: usize,
        overlay_kind: String,
    },
    OverlayRemoved {
        overlay_id: String,
        index: usize,
        overlay_kind: String,
    },
    OverlayChanged {
        overlay_id: String,
        overlay_kind: String,
        before: Value,
        after: Value,
    },
    ProjectSettingChanged {
        property: String,
        before: Value,
        after: Value,
    },
    Unclassified {
        path: String,
        before: Value,
        after: Value,
    },
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
/// project mutation; only `Pending` exposes the proposed command and diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PendingProposal {
    Pending {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        command: Value,
        diff: Vec<ProposalChange>,
        #[serde(rename = "diffTruncated", alias = "diff_truncated")]
        diff_truncated: bool,
        #[serde(default, skip_serializing_if = "ProposalFreshness::is_current")]
        freshness: ProposalFreshness,
        /// Semantic impacts are optional for checkpoints written before this
        /// field existed. `None` is represented as an unavailable semantic
        /// projection by clients; new proposals always write `Some`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        impacts: Option<Vec<ProposalImpact>>,
    },
    Accepted {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        #[serde(rename = "appliedRevision", alias = "applied_revision")]
        applied_revision: u64,
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
    snapshot: StateSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportRecord {
    fingerprint: String,
    snapshot: StateSnapshot,
    source_id: String,
    clip_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalReceipt {
    actor: String,
    key: String,
    base_revision: u64,
    snapshot: StateSnapshot,
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
        command: Option<cut_domain::Command>,
        inverse: Option<cut_domain::Command>,
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
        project: Project,
        source_version: u8,
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
            command: None,
            inverse: None,
            operation: None,
            import: None,
            proposal: None,
            job: None,
            project: None,
            source_version: None,
            checksum: String::new(),
        };
        match event {
            JournalEvent::Mutation {
                action,
                command,
                inverse,
                operation,
                import,
                proposal,
            } => {
                record.action = action;
                record.command = command;
                record.inverse = inverse;
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
            JournalEvent::ProjectionImport {
                project,
                source_version,
                operation,
            } => {
                record.action = JournalAction::ProjectionImport;
                record.project = Some(project);
                record.source_version = Some(source_version);
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
    source_version: u8,
    past: Vec<JournalHistoryEntry>,
    future: Vec<JournalHistoryEntry>,
    jobs: HashMap<String, RenderJob>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    proposal: Option<ProposalRecord>,
}

#[derive(Debug)]
struct Runtime {
    _lock: RuntimeLock,
    project_path: PathBuf,
    journal_path: PathBuf,
    project: Project,
    revision: u64,
    past: Vec<JournalHistoryEntry>,
    future: Vec<JournalHistoryEntry>,
    jobs: HashMap<String, RenderJob>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    proposal: Option<ProposalRecord>,
    source_version: u8,
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
    GetProposalPreview {
        #[serde(rename = "proposalId")]
        proposal_id: String,
    },
    Apply {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        command: Value,
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
        metadata: Option<Value>,
        overlay: Option<Value>,
    },
    StartRender {
        quality: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration: Option<f64>,
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
    StageProposal {
        #[serde(rename = "requestKey")]
        request_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        transactions: Vec<Value>,
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
    ProposalPreview {
        before: Value,
        after: Value,
    },
    Import {
        state: StateSnapshot,
        source_id: String,
        clip_id: String,
    },
    RenderStarted {
        job: RenderJob,
        project: Value,
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

    pub fn get_state(&self) -> Result<StateSnapshot, RuntimeError> {
        match self.request(RuntimeRequest::GetState)? {
            RuntimeResponse::State(state) => Ok(state),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected state response",
            )),
        }
    }

    pub fn proposal_preview(&self, proposal_id: &str) -> Result<(Value, Value), RuntimeError> {
        match self.request(RuntimeRequest::GetProposalPreview {
            proposal_id: proposal_id.to_owned(),
        })? {
            RuntimeResponse::ProposalPreview { before, after } => Ok((before, after)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected proposal-preview response",
            )),
        }
    }

    pub fn apply_request(
        &self,
        actor: &str,
        key: &str,
        command: Value,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::Apply {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            command,
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
        metadata: Option<Value>,
        overlay: Option<Value>,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        match self.request(RuntimeRequest::Import {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            path: path.to_owned(),
            metadata,
            overlay,
        })? {
            RuntimeResponse::Import {
                state,
                source_id,
                clip_id,
            } => Ok((state, source_id, clip_id)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected import response",
            )),
        }
    }

    pub fn start_render(&self, quality: &str) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        self.start_render_range(quality, None)
    }

    pub fn start_render_range(
        &self,
        quality: &str,
        range: Option<RenderRange>,
    ) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        let (start, duration) = range
            .map(|range| (Some(range.start), Some(range.duration)))
            .unwrap_or((None, None));
        match self.request(RuntimeRequest::StartRender {
            quality: quality.to_owned(),
            start,
            duration,
        })? {
            RuntimeResponse::RenderStarted {
                job,
                project,
                output,
            } => Ok((job, project, output)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-start response",
            )),
        }
    }

    pub fn render_job(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        match self.request(RuntimeRequest::GetRender {
            job_id: job_id.to_owned(),
        })? {
            RuntimeResponse::RenderJob(job) => Ok(job),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-job response",
            )),
        }
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
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected empty response",
            )),
        }
    }

    pub fn cancel_render(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        match self.request(RuntimeRequest::CancelRender {
            job_id: job_id.to_owned(),
        })? {
            RuntimeResponse::RenderJob(job) => Ok(job),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-job response",
            )),
        }
    }

    pub fn repair_projection(&self) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::RepairProjection)?)
    }

    pub fn import_projection(
        &self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::ImportProjection {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
        })?)
    }

    pub fn stage_agent_proposal(
        &self,
        request_key: &str,
        base_revision: u64,
        transactions: Vec<Value>,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::StageProposal {
            request_key: request_key.to_owned(),
            base_revision,
            transactions,
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

/// Descriptive name for the request-scoped compatibility client.
pub type IpcRuntimeClient = RuntimeClient;

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

    pub fn proposal_preview(&self, proposal_id: &str) -> Result<(Value, Value), RuntimeError> {
        match self.request(RuntimeRequest::GetProposalPreview {
            proposal_id: proposal_id.to_owned(),
        })? {
            RuntimeResponse::ProposalPreview { before, after } => Ok((before, after)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected proposal-preview response",
            )),
        }
    }

    pub fn get_state(&self) -> Result<StateSnapshot, RuntimeError> {
        match self.request(RuntimeRequest::GetState)? {
            RuntimeResponse::State(state) => Ok(state),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected state response",
            )),
        }
    }

    pub fn apply_request(
        &self,
        actor: &str,
        key: &str,
        command: Value,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::Apply {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            command,
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
        metadata: Option<Value>,
        overlay: Option<Value>,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        match self.request(RuntimeRequest::Import {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            path: path.to_owned(),
            metadata,
            overlay,
        })? {
            RuntimeResponse::Import {
                state,
                source_id,
                clip_id,
            } => Ok((state, source_id, clip_id)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected import response",
            )),
        }
    }

    pub fn start_render(&self, quality: &str) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        self.start_render_range(quality, None)
    }

    pub fn start_render_range(
        &self,
        quality: &str,
        range: Option<RenderRange>,
    ) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        let (start, duration) = range
            .map(|range| (Some(range.start), Some(range.duration)))
            .unwrap_or((None, None));
        match self.request(RuntimeRequest::StartRender {
            quality: quality.to_owned(),
            start,
            duration,
        })? {
            RuntimeResponse::RenderStarted {
                job,
                project,
                output,
            } => Ok((job, project, output)),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-start response",
            )),
        }
    }

    pub fn render_job(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        match self.request(RuntimeRequest::GetRender {
            job_id: job_id.to_owned(),
        })? {
            RuntimeResponse::RenderJob(job) => Ok(job),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-job response",
            )),
        }
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
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected empty response",
            )),
        }
    }

    pub fn cancel_render(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        match self.request(RuntimeRequest::CancelRender {
            job_id: job_id.to_owned(),
        })? {
            RuntimeResponse::RenderJob(job) => Ok(job),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected render-job response",
            )),
        }
    }

    pub fn repair_projection(&self) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::RepairProjection)?)
    }

    pub fn import_projection(
        &self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::ImportProjection {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
        })?)
    }

    pub fn stage_agent_proposal(
        &self,
        request_key: &str,
        base_revision: u64,
        transactions: Vec<Value>,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::StageProposal {
            request_key: request_key.to_owned(),
            base_revision,
            transactions,
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
        RuntimeRequest::Apply {
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
        RuntimeRequest::StageProposal { .. } => false,
        RuntimeRequest::RejectProposal {
            idempotency_key: Some(idempotency_key),
            ..
        } => !idempotency_key.trim().is_empty(),
        RuntimeRequest::GetState
        | RuntimeRequest::GetProposalPreview { .. }
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
    if let (Some(before), Some(after)) = (object.get("beforeProject"), object.get("afterProject")) {
        return Ok(RuntimeResponse::ProposalPreview {
            before: before.clone(),
            after: after.clone(),
        });
    }
    if let Some(state) = object.get("state") {
        let state = serde_json::from_value::<StateSnapshot>(state.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        if let (Some(source_id), Some(clip_id)) = (
            object.get("sourceId").and_then(Value::as_str),
            object.get("clipId").and_then(Value::as_str),
        ) {
            return Ok(RuntimeResponse::Import {
                state,
                source_id: source_id.to_owned(),
                clip_id: clip_id.to_owned(),
            });
        }
        return Ok(RuntimeResponse::State(state));
    }
    if let Some(job) = object.get("job") {
        let job = serde_json::from_value::<RenderJob>(job.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        if let (Some(project), Some(output)) = (object.get("project"), object.get("output")) {
            let output = serde_json::from_value::<PathBuf>(output.clone())
                .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
            return Ok(RuntimeResponse::RenderStarted {
                job,
                project: project.clone(),
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
        _ => Err(RuntimeError::new(
            "RUNTIME_PROTOCOL_ERROR",
            "expected state response",
        )),
    }
}

/// Convert an internal typed result to the stable JSONL response shape.  The
/// transport is intentionally kept here so every executable uses one protocol
/// implementation and one runtime owner.
pub fn response_value(id: &str, result: Result<RuntimeResponse, RuntimeError>) -> Value {
    match result {
        Ok(RuntimeResponse::State(state)) => {
            json!({ "protocolVersion": RequestEnvelope::PROTOCOL_VERSION, "id": id, "ok": true, "state": state })
        }
        Ok(RuntimeResponse::ProposalPreview { before, after }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "beforeProject": before,
            "afterProject": after
        }),
        Ok(RuntimeResponse::Import {
            state,
            source_id,
            clip_id,
        }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "state": state,
            "sourceId": source_id,
            "clipId": clip_id
        }),
        Ok(RuntimeResponse::RenderStarted {
            job,
            project,
            output,
        }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "job": job,
            "project": project,
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

/// Explicit alias for callers that want to name the daemon boundary rather
/// than the transport.  `cutd` is the canonical binary and calls this entry
/// point; `cut-runtime` retains the alias only during migration.
pub fn run_server(directory: &Path) -> Result<(), RuntimeError> {
    run_ipc(directory)
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

// The reducer is intentionally private to the actor.  Direct operation
// helpers remain available to this crate's characterization tests while the
// public product surface is `RuntimeHandle`.
#[allow(dead_code)]
impl Runtime {
    fn open(directory: &Path) -> Result<Self, RuntimeError> {
        fs::create_dir_all(directory).map_err(io_error)?;
        let lock = RuntimeLock::acquire(directory)?;
        let project_path = directory.join(PROJECT_FILE);
        let journal_path = directory.join(JOURNAL_FILE);
        let checkpoint = load_checkpoint(directory)?;
        let checkpoint_project = checkpoint.as_ref().map(|value| value.project.clone());
        let (
            project,
            revision,
            past,
            future,
            operations,
            imports,
            proposal,
            jobs,
            source_version,
            journal_sequence,
            checkpoint_projection_digest,
        ) = if let Some(checkpoint) = checkpoint {
            cut_domain::validate(&checkpoint.project).map_err(domain_error)?;
            for entry in checkpoint.past.iter().chain(checkpoint.future.iter()) {
                validate_journal_history_entry(entry)?;
            }
            for record in checkpoint.operations.values() {
                cut_domain::parse_project(&record.snapshot.project).map_err(domain_error)?;
            }
            for record in checkpoint.imports.values() {
                cut_domain::parse_project(&record.snapshot.project).map_err(domain_error)?;
            }
            (
                checkpoint.project,
                checkpoint.revision,
                checkpoint.past,
                checkpoint.future,
                checkpoint.operations,
                checkpoint.imports,
                checkpoint.proposal,
                checkpoint.jobs,
                checkpoint.source_version,
                checkpoint.journal_sequence,
                checkpoint.projection_digest,
            )
        } else {
            let (project, source_version) = load_bootstrap_project(&project_path)?;
            (
                project,
                0,
                Vec::new(),
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                None,
                HashMap::new(),
                source_version,
                0,
                None,
            )
        };
        let mut runtime = Self {
            _lock: lock,
            project_path,
            journal_path,
            project,
            revision,
            past,
            future,
            jobs,
            operations,
            imports,
            proposal,
            source_version,
            journal_sequence,
            journal_records_since_checkpoint: 0,
            projection_status: ProjectionStatus::Missing,
            projection_digest: checkpoint_projection_digest,
        };
        bound_receipts(&mut runtime.operations, None);
        bound_receipts(&mut runtime.imports, None);

        let records = load_journal(&runtime.journal_path, runtime.journal_sequence)?;
        for record in records {
            runtime.replay_journal_record(record)?;
        }

        if checkpoint_project
            .as_ref()
            .is_some_and(|project| projection_matches(&runtime.project_path, project))
        {
            // The projection was known to be the last checkpointed value. A
            // crash may have left the journal ahead of that checkpoint, so it
            // is safe to repair this specific stale projection on startup.
            if runtime.materialize_project().is_ok() && runtime.materialize_history().is_ok() {
                runtime.projection_status = ProjectionStatus::Current;
                runtime.projection_digest = projection_digest(&runtime.project_path).ok();
            }
        } else {
            runtime.projection_status = inspect_projection(&runtime.project_path, &runtime.project);
        }

        if checkpoint_project.is_none() {
            // First launch is the one controlled bootstrap point. Import v1
            // once, write the canonical checkpoint, and publish v2 as the
            // read-only compatibility projection.
            runtime.materialize_project()?;
            runtime.materialize_history()?;
            runtime.projection_status = ProjectionStatus::Current;
            runtime.projection_digest = Some(projection_digest(&runtime.project_path)?);
            runtime.persist_checkpoint()?;
            truncate_journal(&runtime.journal_path)?;
            runtime.journal_records_since_checkpoint = 0;
        }

        if runtime.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL {
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
        let mut snapshot =
            self.snapshot_for_parts(&self.project, self.revision, &self.past, &self.future);
        snapshot.projection_status = inspect_projection(&self.project_path, &self.project)
            .as_str()
            .into();
        snapshot
    }

    fn snapshot_for_parts(
        &self,
        project: &Project,
        revision: u64,
        past: &[JournalHistoryEntry],
        future: &[JournalHistoryEntry],
    ) -> StateSnapshot {
        self.snapshot_for_parts_with_proposal(project, revision, past, future, &self.proposal)
    }

    fn snapshot_for_parts_with_proposal(
        &self,
        project: &Project,
        revision: u64,
        past: &[JournalHistoryEntry],
        future: &[JournalHistoryEntry],
        proposal: &Option<ProposalRecord>,
    ) -> StateSnapshot {
        let project_value = project_value(project);
        let sources = project_value
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        StateSnapshot {
            duration: cut_domain::project_duration(project),
            project: project_value,
            revision,
            can_undo: !past.is_empty(),
            can_redo: !future.is_empty(),
            sources,
            pending_proposal: proposal.as_ref().map(|record| {
                let mut lifecycle = record.lifecycle.clone();
                if let PendingProposal::Pending {
                    base_revision,
                    freshness,
                    ..
                } = &mut lifecycle
                {
                    *freshness = if *base_revision == revision {
                        ProposalFreshness::Current
                    } else {
                        ProposalFreshness::Stale
                    };
                }
                lifecycle
            }),
            source_version: self.source_version,
            projection_status: self.projection_status.as_str().into(),
        }
    }

    fn durable_state(&self) -> DurableState {
        DurableState {
            project: self.project.clone(),
            revision: self.revision,
            source_version: self.source_version,
            past: self.past.clone(),
            future: self.future.clone(),
            jobs: self.jobs.clone(),
            operations: self.operations.clone(),
            imports: self.imports.clone(),
            proposal: self.proposal.clone(),
        }
    }

    fn candidate_state(
        &self,
        project: Project,
        revision: u64,
        past: Vec<JournalHistoryEntry>,
        future: Vec<JournalHistoryEntry>,
    ) -> DurableState {
        let mut candidate = self.durable_state();
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
        let project_changed = candidate.project != self.project;
        if project_changed {
            self.projection_status = inspect_projection(&self.project_path, &self.project);
            if self.projection_status == ProjectionStatus::Diverged && !allow_diverged_projection {
                return Err(RuntimeError::new(
                    "PROJECTION_DIVERGED",
                    "project.cut.json differs from authoritative cutd state; repair or explicitly import the projection before editing",
                ));
            }
            // Publish the compatibility projection before the journal record.
            // If the process dies between these writes, startup compares the
            // projection with authoritative checkpoint/journal state and
            // reports divergence instead of adopting an unjournaled edit.
            self.materialize_project_value(&candidate.project)?;
            self.materialize_history_values(
                candidate.revision,
                &candidate.past,
                &candidate.future,
            )?;
        }

        let sequence = self.journal_sequence + 1;
        let record = seal_journal_record(JournalRecord::from_event(
            sequence,
            candidate.revision,
            event,
        ))?;
        append_journal_record(&self.journal_path, &record)?;

        self.project = candidate.project;
        self.revision = candidate.revision;
        self.source_version = candidate.source_version;
        self.past = candidate.past;
        self.future = candidate.future;
        self.jobs = candidate.jobs;
        self.operations = candidate.operations;
        self.imports = candidate.imports;
        self.proposal = candidate.proposal;
        self.journal_sequence = sequence;
        self.journal_records_since_checkpoint += 1;
        if project_changed {
            self.projection_status = ProjectionStatus::Current;
            self.projection_digest = projection_digest(&self.project_path).ok();
        }
        if self.journal_records_since_checkpoint >= CHECKPOINT_INTERVAL {
            // The journal is the durable commit. Checkpointing is a bounded
            // optimization; a temporary checkpoint failure leaves the typed
            // records available for recovery on the next startup.
            if self.persist_checkpoint().is_ok() {
                let _ = truncate_journal(&self.journal_path);
                self.journal_records_since_checkpoint = 0;
            }
        }
        Ok(self.snapshot())
    }

    pub fn apply(
        &mut self,
        command: Value,
        expected_revision: Option<u64>,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.check_revision(expected_revision)?;
        let typed = cut_domain::parse_command(&command).map_err(domain_error)?;
        let inverse = cut_domain::inverse_command(&self.project, &typed).map_err(domain_error)?;
        let next = cut_domain::apply_command(&self.project, &typed).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.push(JournalHistoryEntry {
            command: typed.clone(),
            inverse: inverse.clone(),
        });
        bound_history(&mut past);
        let revision = self.revision + 1;
        self.commit_candidate(
            self.candidate_state(next, revision, past, Vec::new()),
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                command: Some(typed),
                inverse: Some(inverse),
                operation: None,
                import: None,
                proposal: self.proposal.clone(),
            },
        )
    }

    pub fn apply_request(
        &mut self,
        actor: &str,
        key: &str,
        command: Value,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        // Pending-proposal policy: ordinary canonical edits may continue, but
        // a proposal stays pinned to the revision at which it was staged.
        // Acceptance then fails with REVISION_CONFLICT rather than silently
        // rebasing an agent command over unrelated human edits. The caller
        // must reject or restage the proposal explicitly.
        self.apply_request_with_proposal(actor, key, command, base_revision, self.proposal.clone())
    }

    fn apply_request_with_proposal(
        &mut self,
        actor: &str,
        key: &str,
        command: Value,
        base_revision: u64,
        proposal: Option<ProposalRecord>,
    ) -> Result<StateSnapshot, RuntimeError> {
        let operation = format!("{actor}:{key}");
        let fingerprint = format!(
            "{}:{}:{}",
            actor,
            base_revision,
            serde_json::to_string(&command).unwrap_or_default()
        );
        if let Some(record) = self.operations.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different request",
                ));
            }
            return Ok(record.snapshot.clone());
        }
        self.check_revision(Some(base_revision))?;
        let typed = cut_domain::parse_command(&command).map_err(domain_error)?;
        let inverse = cut_domain::inverse_command(&self.project, &typed).map_err(domain_error)?;
        let next = cut_domain::apply_command(&self.project, &typed).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.push(JournalHistoryEntry {
            command: typed.clone(),
            inverse: inverse.clone(),
        });
        bound_history(&mut past);
        let revision = self.revision + 1;
        let mut candidate = self.candidate_state(next, revision, past, Vec::new());
        candidate.proposal = proposal;
        let snapshot = self.snapshot_for_parts_with_proposal(
            &candidate.project,
            candidate.revision,
            &candidate.past,
            &candidate.future,
            &candidate.proposal,
        );
        if let Some(proposal) = candidate.proposal.as_mut() {
            if matches!(proposal.lifecycle, PendingProposal::Accepted { .. }) {
                proposal.receipt = Some(ProposalReceipt {
                    actor: actor.to_owned(),
                    key: key.to_owned(),
                    base_revision,
                    snapshot: snapshot.clone(),
                });
            }
        }
        candidate.operations.insert(
            operation,
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        let operation_key = format!("{actor}:{key}");
        let operation_record = candidate
            .operations
            .get(&operation_key)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "operation receipt was not staged")
            })?;
        let journal_proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                command: Some(typed),
                inverse: Some(inverse),
                operation: Some(JournalOperation {
                    key: operation_key,
                    record: operation_record,
                }),
                import: None,
                proposal: journal_proposal,
            },
        )
    }

    /// Stage the ordered commands returned by an agent without touching the
    /// canonical project or its history.  All commands are flattened into one
    /// batch and applied to a private clone so validation is all-or-nothing.
    pub fn stage_agent_proposal(
        &mut self,
        request_key: &str,
        base_revision: u64,
        transactions: &[Value],
    ) -> Result<StateSnapshot, RuntimeError> {
        if request_key.trim().is_empty() {
            return Err(RuntimeError::new(
                "AGENT_PROTOCOL_ERROR",
                "agent request requires a non-empty idempotency key",
            ));
        }
        self.check_revision(Some(base_revision))?;

        // An agent with no transactions completed without a project edit.  It
        // must not clear an existing proposal or fabricate a no-op proposal.
        if transactions.is_empty() {
            return Ok(self.snapshot());
        }

        let mut commands = Vec::new();
        for (index, transaction) in transactions.iter().enumerate() {
            let object = transaction.as_object().ok_or_else(|| {
                RuntimeError::new("AGENT_PROTOCOL_ERROR", "each transaction must be an object")
            })?;
            let key = object
                .get("idempotencyKey")
                .and_then(Value::as_str)
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| {
                    RuntimeError::new(
                        "AGENT_PROTOCOL_ERROR",
                        "each transaction requires idempotencyKey",
                    )
                })?;
            if index == 0 && key != request_key {
                return Err(RuntimeError::new(
                    "AGENT_PROTOCOL_ERROR",
                    "the first transaction idempotencyKey must match the request key",
                ));
            }
            let transaction_commands = object
                .get("commands")
                .and_then(Value::as_array)
                .filter(|commands| !commands.is_empty())
                .ok_or_else(|| {
                    RuntimeError::new(
                        "AGENT_PROTOCOL_ERROR",
                        "each transaction requires non-empty commands",
                    )
                })?;
            commands.extend(transaction_commands.iter().cloned());
        }

        let command = json!({ "type": "batch", "commands": commands });
        let fingerprint = format!(
            "{}:{}",
            base_revision,
            serde_json::to_string(&command).unwrap_or_default()
        );

        if let Some(existing) = self.proposal.as_ref() {
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

        let typed = cut_domain::parse_command(&command).map_err(domain_error)?;
        let candidate = cut_domain::apply_command(&self.project, &typed).map_err(domain_error)?;
        let before_value = project_value(&self.project);
        let candidate_value = project_value(&candidate);
        let (diff, diff_truncated) = structured_diff(&before_value, &candidate_value);
        let impacts = derive_proposal_impacts(&self.project, &candidate, &diff, diff_truncated);
        let proposal_id = format!("proposal-{}", unique_id());
        let proposal = Some(ProposalRecord {
            lifecycle: PendingProposal::Pending {
                proposal_id,
                base_revision,
                command,
                diff,
                diff_truncated,
                freshness: ProposalFreshness::Current,
                impacts: Some(impacts),
            },
            request_key: request_key.to_owned(),
            fingerprint,
            receipt: None,
        });
        let mut candidate = self.durable_state();
        candidate.proposal = proposal;
        let proposal = candidate.proposal.clone();
        self.commit_candidate(candidate, JournalEvent::Proposal { proposal })
    }

    /// Apply a previously staged proposal as exactly one canonical request.
    /// Revision and history are advanced only by `apply_request`.
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
        let record = self.proposal.clone().ok_or_else(|| {
            RuntimeError::new(
                "PROPOSAL_NOT_FOUND",
                format!("proposal not found: {proposal_id}"),
            )
        })?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision: proposal_revision,
                command,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                        applied_revision: self.revision + 1,
                    },
                    request_key: record.request_key.clone(),
                    fingerprint: record.fingerprint.clone(),
                    receipt: None,
                });
                self.apply_request_with_proposal(
                    actor,
                    key,
                    command,
                    base_revision,
                    accepted_proposal,
                )
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                Ok(receipt.snapshot.clone())
            }
            PendingProposal::Rejected {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
        let record = self.proposal.clone().ok_or_else(|| {
            RuntimeError::new(
                "PROPOSAL_NOT_FOUND",
                format!("proposal not found: {proposal_id}"),
            )
        })?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                let mut candidate = self.durable_state();
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
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
        if let Some(existing) = self.operations.get(&operation) {
            if existing.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different request",
                ));
            }
            return Ok(existing.snapshot.clone());
        }

        let record = self.proposal.clone().ok_or_else(|| {
            RuntimeError::new(
                "PROPOSAL_NOT_FOUND",
                format!("proposal not found: {proposal_id}"),
            )
        })?;
        match record.lifecycle {
            PendingProposal::Pending {
                proposal_id: current_id,
                base_revision: proposal_revision,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                let mut candidate = self.durable_state();
                candidate.proposal = rejected_proposal;
                let snapshot = self.snapshot_for_parts_with_proposal(
                    &candidate.project,
                    candidate.revision,
                    &candidate.past,
                    &candidate.future,
                    &candidate.proposal,
                );
                candidate.operations.insert(
                    operation.clone(),
                    OperationRecord {
                        fingerprint,
                        snapshot: snapshot.clone(),
                    },
                );
                if let Some(proposal) = candidate.proposal.as_mut() {
                    proposal.receipt = Some(ProposalReceipt {
                        actor: actor.to_owned(),
                        key: key.to_owned(),
                        base_revision,
                        snapshot: snapshot.clone(),
                    });
                }
                let operation_record =
                    candidate
                        .operations
                        .get(&operation)
                        .cloned()
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "RUNTIME_PROTOCOL_ERROR",
                                "operation receipt was not staged",
                            )
                        })?;
                let journal_proposal = candidate.proposal.clone();
                self.commit_candidate(
                    candidate,
                    JournalEvent::Mutation {
                        action: JournalAction::RejectProposal,
                        command: None,
                        inverse: None,
                        operation: Some(JournalOperation {
                            key: operation,
                            record: operation_record,
                        }),
                        import: None,
                        proposal: journal_proposal,
                    },
                )
            }
            PendingProposal::Accepted {
                proposal_id: current_id,
                ..
            } => {
                if current_id != proposal_id {
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                    return Err(RuntimeError::new(
                        "PROPOSAL_NOT_FOUND",
                        format!("proposal not found: {proposal_id}"),
                    ));
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
                Ok(receipt.snapshot.clone())
            }
        }
    }

    pub fn undo(&mut self, expected_revision: Option<u64>) -> Result<StateSnapshot, RuntimeError> {
        self.check_revision(expected_revision)?;
        let Some(entry) = self.past.last().cloned() else {
            return Ok(self.snapshot());
        };
        let next =
            cut_domain::apply_command(&self.project, &entry.inverse).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.pop();
        let mut future = self.future.clone();
        future.push(entry.clone());
        bound_history(&mut future);
        self.commit_candidate(
            self.candidate_state(next, self.revision + 1, past, future),
            JournalEvent::Mutation {
                action: JournalAction::Undo,
                command: Some(entry.inverse),
                inverse: Some(entry.command),
                operation: None,
                import: None,
                proposal: self.proposal.clone(),
            },
        )
    }

    pub fn undo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{}:{}:undo", actor, base_revision);
        if let Some(record) = self.operations.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different request",
                ));
            }
            return Ok(record.snapshot.clone());
        }
        self.check_revision(Some(base_revision))?;
        let Some(entry) = self.past.last().cloned() else {
            let mut candidate = self.durable_state();
            let snapshot = self.snapshot_for_parts_with_proposal(
                &candidate.project,
                candidate.revision,
                &candidate.past,
                &candidate.future,
                &candidate.proposal,
            );
            candidate.operations.insert(
                operation.clone(),
                OperationRecord {
                    fingerprint,
                    snapshot: snapshot.clone(),
                },
            );
            let operation_record =
                candidate
                    .operations
                    .get(&operation)
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::new(
                            "RUNTIME_PROTOCOL_ERROR",
                            "operation receipt was not staged",
                        )
                    })?;
            let journal_proposal = candidate.proposal.clone();
            return self.commit_candidate(
                candidate,
                JournalEvent::Mutation {
                    action: JournalAction::Noop,
                    command: None,
                    inverse: None,
                    operation: Some(JournalOperation {
                        key: operation,
                        record: operation_record,
                    }),
                    import: None,
                    proposal: journal_proposal,
                },
            );
        };
        let previous =
            cut_domain::apply_command(&self.project, &entry.inverse).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.pop();
        let mut future = self.future.clone();
        future.push(entry.clone());
        bound_history(&mut future);
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&previous, revision, &past, &future);
        let mut candidate = self.candidate_state(previous, revision, past, future);
        candidate.operations.insert(
            operation.clone(),
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        let operation_record = candidate
            .operations
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "operation receipt was not staged")
            })?;
        let journal_proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Undo,
                command: Some(entry.inverse),
                inverse: Some(entry.command),
                operation: Some(JournalOperation {
                    key: operation,
                    record: operation_record,
                }),
                import: None,
                proposal: journal_proposal,
            },
        )
    }

    pub fn redo(&mut self, expected_revision: Option<u64>) -> Result<StateSnapshot, RuntimeError> {
        self.check_revision(expected_revision)?;
        let Some(entry) = self.future.last().cloned() else {
            return Ok(self.snapshot());
        };
        let next =
            cut_domain::apply_command(&self.project, &entry.command).map_err(domain_error)?;
        let mut future = self.future.clone();
        future.pop();
        let mut past = self.past.clone();
        past.push(entry.clone());
        bound_history(&mut past);
        self.commit_candidate(
            self.candidate_state(next, self.revision + 1, past, future),
            JournalEvent::Mutation {
                action: JournalAction::Redo,
                command: Some(entry.command),
                inverse: Some(entry.inverse),
                operation: None,
                import: None,
                proposal: self.proposal.clone(),
            },
        )
    }

    pub fn redo_request(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{}:{}:redo", actor, base_revision);
        if let Some(record) = self.operations.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different request",
                ));
            }
            return Ok(record.snapshot.clone());
        }
        self.check_revision(Some(base_revision))?;
        let Some(entry) = self.future.last().cloned() else {
            let mut candidate = self.durable_state();
            let snapshot = self.snapshot_for_parts_with_proposal(
                &candidate.project,
                candidate.revision,
                &candidate.past,
                &candidate.future,
                &candidate.proposal,
            );
            candidate.operations.insert(
                operation.clone(),
                OperationRecord {
                    fingerprint,
                    snapshot: snapshot.clone(),
                },
            );
            let operation_record =
                candidate
                    .operations
                    .get(&operation)
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::new(
                            "RUNTIME_PROTOCOL_ERROR",
                            "operation receipt was not staged",
                        )
                    })?;
            let journal_proposal = candidate.proposal.clone();
            return self.commit_candidate(
                candidate,
                JournalEvent::Mutation {
                    action: JournalAction::Noop,
                    command: None,
                    inverse: None,
                    operation: Some(JournalOperation {
                        key: operation,
                        record: operation_record,
                    }),
                    import: None,
                    proposal: journal_proposal,
                },
            );
        };
        let next =
            cut_domain::apply_command(&self.project, &entry.command).map_err(domain_error)?;
        let mut future = self.future.clone();
        future.pop();
        let mut past = self.past.clone();
        past.push(entry.clone());
        bound_history(&mut past);
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &future);
        let mut candidate = self.candidate_state(next, revision, past, future);
        candidate.operations.insert(
            operation.clone(),
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        let operation_record = candidate
            .operations
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "operation receipt was not staged")
            })?;
        let journal_proposal = candidate.proposal.clone();
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Redo,
                command: Some(entry.command),
                inverse: Some(entry.inverse),
                operation: Some(JournalOperation {
                    key: operation,
                    record: operation_record,
                }),
                import: None,
                proposal: journal_proposal,
            },
        )
    }

    pub fn start_render(
        &mut self,
        quality: &str,
    ) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        self.start_render_range(quality, None)
    }

    pub fn start_render_range(
        &mut self,
        quality: &str,
        range: Option<RenderRange>,
    ) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        if !matches!(quality, "preview" | "full") {
            return Err(RuntimeError::new(
                "INVALID_RENDER_QUALITY",
                "quality must be preview or full",
            ));
        }
        if let Some(range) = range.as_ref() {
            RenderRange::new(range.start, range.duration)?;
            if range.end() > self.project.duration() + 1e-9 {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_RANGE",
                    format!(
                        "render range ends at {:.3}s but project duration is {:.3}s",
                        range.end(),
                        self.project.duration()
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
        let mut candidate = self.durable_state();
        candidate.jobs.insert(id, job.clone());
        self.commit_candidate(candidate, JournalEvent::Job { job: job.clone() })?;
        Ok((job, project_value(&self.project), output))
    }

    pub fn proposal_preview(&self, proposal_id: &str) -> Result<(Value, Value), RuntimeError> {
        let record = self.proposal.as_ref().ok_or_else(|| {
            RuntimeError::new(
                "PROPOSAL_NOT_FOUND",
                format!("proposal not found: {proposal_id}"),
            )
        })?;
        let PendingProposal::Pending {
            proposal_id: current_id,
            base_revision,
            command,
            ..
        } = &record.lifecycle
        else {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_PENDING",
                format!("proposal is not pending: {proposal_id}"),
            ));
        };
        if current_id != proposal_id {
            return Err(RuntimeError::new(
                "PROPOSAL_NOT_FOUND",
                format!("proposal not found: {proposal_id}"),
            ));
        }
        if *base_revision != self.revision {
            return Err(RuntimeError::new(
                "PROPOSAL_STALE",
                format!(
                    "proposal was created at revision {base_revision}, current revision is {}",
                    self.revision
                ),
            ));
        }
        let command = cut_domain::parse_command(command).map_err(domain_error)?;
        let after = cut_domain::apply_command(&self.project, &command).map_err(domain_error)?;
        Ok((project_value(&self.project), project_value(&after)))
    }

    pub fn import_media(
        &mut self,
        path: &str,
        metadata: Option<&Value>,
        overlay: Option<&Value>,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        let (next, source_id, clip_id, command) =
            self.build_import_candidate(path, metadata, overlay)?;
        let inverse = cut_domain::inverse_command(&self.project, &command).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.push(JournalHistoryEntry {
            command: command.clone(),
            inverse: inverse.clone(),
        });
        bound_history(&mut past);
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &[]);
        self.commit_candidate(
            self.candidate_state(next, revision, past, Vec::new()),
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                command: Some(command),
                inverse: Some(inverse),
                operation: None,
                import: None,
                proposal: self.proposal.clone(),
            },
        )?;
        Ok((snapshot, source_id, clip_id))
    }

    fn build_import_candidate(
        &self,
        path: &str,
        metadata: Option<&Value>,
        overlay: Option<&Value>,
    ) -> Result<(Project, String, String, cut_domain::Command), RuntimeError> {
        if path.trim().is_empty() {
            return Err(RuntimeError::new(
                "INVALID_IMPORT",
                "media path is required",
            ));
        }
        let source_path = validate_media_path(path)?;
        if let Some(overlay) = overlay {
            validate_overlay_shape(overlay)?;
        }
        let source_id = format!("source-{}", unique_id());
        let clip_id = format!("clip-{}", unique_id());
        let probe = Self::probe(path).ok();
        let mut source = json!({"id":source_id.clone(),"path":source_path.to_string_lossy()});
        let supplied = metadata.or(probe.as_ref());
        if let Some(info) = supplied.and_then(Value::as_object) {
            for key in [
                "duration", "width", "height", "hasAudio", "subs", "metadata",
            ] {
                if let Some(value) = info.get(key) {
                    source[key] = value.clone();
                }
            }
        }
        let duration = source
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|duration| *duration > 0.0);
        let mut next = project_value(&self.project);
        next["sources"].as_array_mut().unwrap().push(source);
        let tracks = next["tracks"].as_array_mut().unwrap();
        let mut created_track = None;
        if !tracks
            .iter()
            .any(|track| track.get("type").and_then(Value::as_str) == Some("video"))
        {
            let track =
                json!({"id": format!("video-{}", unique_id()), "type": "video", "clips": []});
            tracks.push(track.clone());
            created_track = Some(track);
        }
        let track = tracks
            .iter_mut()
            .find(|track| track.get("type").and_then(Value::as_str) == Some("video"))
            .expect("video track was created above");
        let track_id = track
            .get("id")
            .and_then(Value::as_str)
            .expect("validated video track has an id")
            .to_owned();
        track["clips"].as_array_mut().unwrap().push(json!({
            "id": clip_id.clone(),
            "sourceId": source_id.clone(),
            "timelineStart": cut_domain::project_duration(&self.project),
            "in": 0.0,
            "out": duration.unwrap_or(1.0)
        }));
        if let Some(overlay) = overlay {
            next["overlays"]
                .as_array_mut()
                .unwrap()
                .push(overlay.clone());
        }
        let next = cut_domain::parse_project(&next).map_err(domain_error)?;
        let next_value = project_value(&next);
        let source = next_value["sources"]
            .as_array()
            .and_then(|sources| {
                sources.iter().find(|source| {
                    source.get("id").and_then(Value::as_str) == Some(source_id.as_str())
                })
            })
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "import source disappeared")
            })?;
        let clip = next_value["tracks"]
            .as_array()
            .and_then(|tracks| {
                tracks.iter().find_map(|track| {
                    track["clips"].as_array().and_then(|clips| {
                        clips.iter().find(|clip| {
                            clip.get("id").and_then(Value::as_str) == Some(clip_id.as_str())
                        })
                    })
                })
            })
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "import clip disappeared")
            })?;
        let mut commands = Vec::with_capacity(4);
        if let Some(track) = created_track {
            commands.push(json!({"type": "add_track", "track": track}));
        }
        commands.push(json!({"type": "add_source", "source": source}));
        commands.push(json!({
            "type": "add_clip",
            "trackId": track_id,
            "clip": clip
        }));
        if overlay.is_some() {
            let overlay = next.overlays.last().cloned().ok_or_else(|| {
                RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "import overlay disappeared")
            })?;
            commands.push(json!({"type": "add_overlay", "overlay": overlay}));
        }
        let command = cut_domain::parse_command(&json!({
            "type": "batch",
            "commands": commands,
        }))
        .map_err(domain_error)?;
        let applied = cut_domain::apply_command(&self.project, &command).map_err(domain_error)?;
        if applied != next {
            return Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "typed import command did not reproduce candidate project",
            ));
        }
        Ok((next, source_id, clip_id, command))
    }

    pub fn import_media_request(
        &mut self,
        actor: &str,
        key: &str,
        path: &str,
        base_revision: u64,
        metadata: Option<Value>,
        overlay: Option<Value>,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!(
            "{actor}:{base_revision}:{path}:{}:{}",
            serde_json::to_string(&metadata).unwrap_or_default(),
            serde_json::to_string(&overlay).unwrap_or_default()
        );
        if let Some(record) = self.imports.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different import",
                ));
            }
            return Ok((
                record.snapshot.clone(),
                record.source_id.clone(),
                record.clip_id.clone(),
            ));
        }
        self.check_revision(Some(base_revision))?;
        // Build the import candidate without mutating the actor.  The same
        // probe/import shape is used by direct and idempotent requests so a
        // failed projection leaves every in-memory field untouched.
        let (next, source_id, clip_id, command) =
            self.build_import_candidate(path, metadata.as_ref(), overlay.as_ref())?;
        let inverse = cut_domain::inverse_command(&self.project, &command).map_err(domain_error)?;
        let mut past = self.past.clone();
        past.push(JournalHistoryEntry {
            command: command.clone(),
            inverse: inverse.clone(),
        });
        bound_history(&mut past);
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &[]);
        let mut candidate = self.candidate_state(next, revision, past, Vec::new());
        let import_key = operation.clone();
        candidate.imports.insert(
            import_key.clone(),
            ImportRecord {
                fingerprint,
                snapshot: snapshot.clone(),
                source_id: source_id.clone(),
                clip_id: clip_id.clone(),
            },
        );
        bound_receipts(&mut candidate.imports, Some(&import_key));
        let import_record = candidate.imports.get(&import_key).cloned().ok_or_else(|| {
            RuntimeError::new("RUNTIME_PROTOCOL_ERROR", "import receipt was not staged")
        })?;
        self.commit_candidate(
            candidate,
            JournalEvent::Mutation {
                action: JournalAction::Apply,
                command: Some(command),
                inverse: Some(inverse),
                operation: None,
                import: Some(JournalImport {
                    key: import_key,
                    record: import_record,
                }),
                proposal: self.proposal.clone(),
            },
        )?;
        Ok((snapshot, source_id, clip_id))
    }

    pub fn update_job(
        &mut self,
        id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
    ) -> Result<(), RuntimeError> {
        self.update_job_with_progress(id, status, error, output_url, None)
    }

    pub fn update_job_with_progress(
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
        let current = self.jobs.get(id).cloned().ok_or_else(|| {
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
        let mut candidate = self.durable_state();
        candidate.jobs.insert(id.to_owned(), job.clone());
        self.commit_candidate(candidate, JournalEvent::Job { job })?;
        Ok(())
    }

    pub fn render_job(&self, id: &str) -> Result<RenderJob, RuntimeError> {
        self.jobs.get(id).cloned().ok_or_else(|| {
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
            )?;
        }
        self.render_job(id)
    }

    fn probe(path: &str) -> Result<Value, RuntimeError> {
        let info = cut_engine::probe::info(path).map_err(engine_error)?;
        serde_json::to_value(info)
            .map_err(|error| RuntimeError::new("ENGINE_PROTOCOL_ERROR", error.to_string()))
    }

    fn execute(&mut self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.refresh_projection_status();
        match request {
            RuntimeRequest::GetState => Ok(RuntimeResponse::State(self.snapshot())),
            RuntimeRequest::GetProposalPreview { proposal_id } => {
                let (before, after) = self.proposal_preview(&proposal_id)?;
                Ok(RuntimeResponse::ProposalPreview { before, after })
            }
            RuntimeRequest::Apply {
                actor,
                idempotency_key,
                base_revision,
                command,
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                Ok(RuntimeResponse::State(self.apply_request(
                    &actor,
                    &idempotency_key,
                    command,
                    base_revision,
                )?))
            }
            RuntimeRequest::Undo {
                actor,
                idempotency_key,
                base_revision,
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                Ok(RuntimeResponse::State(self.undo_request(
                    &actor,
                    &idempotency_key,
                    base_revision,
                )?))
            }
            RuntimeRequest::Redo {
                actor,
                idempotency_key,
                base_revision,
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                Ok(RuntimeResponse::State(self.redo_request(
                    &actor,
                    &idempotency_key,
                    base_revision,
                )?))
            }
            RuntimeRequest::Import {
                actor,
                idempotency_key,
                base_revision,
                path,
                metadata,
                overlay,
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                let (state, source_id, clip_id) = self.import_media_request(
                    &actor,
                    &idempotency_key,
                    &path,
                    base_revision,
                    metadata,
                    overlay,
                )?;
                Ok(RuntimeResponse::Import {
                    state,
                    source_id,
                    clip_id,
                })
            }
            RuntimeRequest::StartRender {
                quality,
                start,
                duration,
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
                let (job, project, output) = self.start_render_range(&quality, range)?;
                Ok(RuntimeResponse::RenderStarted {
                    job,
                    project,
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
                self.update_job_with_progress(&job_id, &status, error, output_url, progress)?;
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
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                Ok(RuntimeResponse::State(self.import_projection(
                    &actor,
                    &idempotency_key,
                    base_revision,
                )?))
            }
            RuntimeRequest::StageProposal {
                request_key,
                base_revision,
                transactions,
            } => Ok(RuntimeResponse::State(self.stage_agent_proposal(
                &request_key,
                base_revision,
                &transactions,
            )?)),
            RuntimeRequest::AcceptProposal {
                proposal_id,
                actor,
                idempotency_key,
                base_revision,
            } => {
                validate_request_identity(&actor, &idempotency_key)?;
                Ok(RuntimeResponse::State(self.accept_proposal(
                    &proposal_id,
                    &actor,
                    &idempotency_key,
                    base_revision,
                )?))
            }
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
                    validate_request_identity(&actor, &idempotency_key)?;
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
        let action = record.action.clone();
        match action {
            JournalAction::Apply | JournalAction::Undo | JournalAction::Redo => {
                let command = record.command.clone();
                let inverse = record.inverse.clone();
                match record.action {
                    JournalAction::Apply => {
                        let command = command.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "apply record has no command")
                        })?;
                        let inverse = inverse.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "apply record has no inverse")
                        })?;
                        self.project = cut_domain::apply_command(&self.project, &command)
                            .map_err(domain_error)?;
                        self.past.push(JournalHistoryEntry { command, inverse });
                        bound_history(&mut self.past);
                        self.future.clear();
                        self.revision = record.revision;
                    }
                    JournalAction::Undo => {
                        let command = command.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "undo record has no command")
                        })?;
                        let inverse = inverse.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "undo record has no inverse")
                        })?;
                        self.project = cut_domain::apply_command(&self.project, &command)
                            .map_err(domain_error)?;
                        let _ = self.past.pop().ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "undo record has no past entry")
                        })?;
                        self.future.push(JournalHistoryEntry {
                            command: inverse,
                            inverse: command,
                        });
                        bound_history(&mut self.future);
                        self.revision = record.revision;
                    }
                    JournalAction::Redo => {
                        let command = command.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "redo record has no command")
                        })?;
                        let inverse = inverse.ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "redo record has no inverse")
                        })?;
                        self.project = cut_domain::apply_command(&self.project, &command)
                            .map_err(domain_error)?;
                        let _ = self.future.pop().ok_or_else(|| {
                            RuntimeError::new("INVALID_JOURNAL", "redo record has no future entry")
                        })?;
                        self.past.push(JournalHistoryEntry { command, inverse });
                        bound_history(&mut self.past);
                        self.revision = record.revision;
                    }
                    _ => unreachable!(),
                }
                self.proposal = record.proposal;
            }
            JournalAction::Noop | JournalAction::RejectProposal => {
                self.revision = record.revision;
                self.proposal = record.proposal;
            }
            JournalAction::StageProposal => {
                self.proposal = record.proposal;
            }
            JournalAction::Job => {
                let job = record
                    .job
                    .ok_or_else(|| RuntimeError::new("INVALID_JOURNAL", "job record has no job"))?;
                self.jobs.insert(job.job_id.clone(), job);
            }
            JournalAction::ProjectionImport => {
                let project = record.project.ok_or_else(|| {
                    RuntimeError::new("INVALID_JOURNAL", "projection import has no project")
                })?;
                let project_changed = project != self.project;
                self.project = project;
                cut_domain::validate(&self.project).map_err(domain_error)?;
                self.revision = record.revision;
                if project_changed {
                    self.past.clear();
                    self.future.clear();
                    self.proposal = None;
                }
                self.source_version = record.source_version.unwrap_or(2);
            }
        }
        if let Some(operation) = record.operation {
            self.operations.insert(operation.key, operation.record);
            bound_receipts(&mut self.operations, None);
        }
        if let Some(import) = record.import {
            self.imports.insert(import.key, import.record);
            bound_receipts(&mut self.imports, None);
        }
        self.journal_sequence = record.sequence;
        self.journal_records_since_checkpoint += 1;
        Ok(())
    }

    fn check_revision(&self, expected: Option<u64>) -> Result<(), RuntimeError> {
        if let Some(value) = expected {
            if value != self.revision {
                return Err(RuntimeError::conflict(value, self.revision));
            }
        }
        Ok(())
    }

    fn refresh_projection_status(&mut self) {
        self.projection_status = inspect_projection(&self.project_path, &self.project);
        if self.projection_status == ProjectionStatus::Current {
            self.projection_digest = projection_digest(&self.project_path).ok();
        }
    }

    fn persist_checkpoint(&self) -> Result<(), RuntimeError> {
        let checkpoint = PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: self.project.clone(),
            revision: self.revision,
            past: self.past.clone(),
            future: self.future.clone(),
            journal_sequence: self.journal_sequence,
            operations: self.operations.clone(),
            imports: self.imports.clone(),
            proposal: self.proposal.clone(),
            jobs: self.jobs.clone(),
            source_version: self.source_version,
            projection_digest: self.projection_digest.clone(),
        };
        write_json_atomic(
            &self.project_path.with_file_name(CHECKPOINT_FILE),
            &checkpoint,
        )
    }

    fn materialize_project(&self) -> Result<(), RuntimeError> {
        self.materialize_project_value(&self.project)
    }

    fn materialize_project_value(&self, project: &Project) -> Result<(), RuntimeError> {
        write_json_atomic(&self.project_path, project)
    }

    fn materialize_history(&self) -> Result<(), RuntimeError> {
        self.materialize_history_values(self.revision, &self.past, &self.future)
    }

    fn materialize_history_values(
        &self,
        revision: u64,
        past: &[JournalHistoryEntry],
        future: &[JournalHistoryEntry],
    ) -> Result<(), RuntimeError> {
        // Keep the old filename as a small compatibility projection. It is a
        // command journal view, never a source of truth and never a list of
        // full project snapshots.
        let history = json!({
            "formatVersion": JOURNAL_VERSION,
            "revision": revision,
            "past": past,
            "future": future,
        });
        write_json_atomic(&self.project_path.with_file_name(HISTORY_FILE), &history)
    }

    fn import_projection(
        &mut self,
        actor: &str,
        key: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
        let operation = format!("{actor}:{key}");
        let fingerprint = format!("{actor}:{base_revision}:projection_import");
        if let Some(record) = self.operations.get(&operation) {
            if record.fingerprint != fingerprint {
                return Err(RuntimeError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "idempotency key was already used for a different projection import",
                ));
            }
            return Ok(record.snapshot.clone());
        }
        self.check_revision(Some(base_revision))?;
        if !self.project_path.is_file() {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                "project.cut.json is required for an explicit projection import",
            ));
        }
        let (project, source_version) = load_bootstrap_project(&self.project_path)?;
        let project_changed = project != self.project;
        let revision = if project_changed {
            self.revision + 1
        } else {
            self.revision
        };
        let mut candidate = self.durable_state();
        candidate.project = project;
        candidate.revision = revision;
        candidate.source_version = source_version;
        if project_changed {
            candidate.past.clear();
            candidate.future.clear();
            candidate.proposal = None;
        }
        let mut snapshot = self.snapshot_for_parts_with_proposal(
            &candidate.project,
            candidate.revision,
            &candidate.past,
            &candidate.future,
            &candidate.proposal,
        );
        snapshot.source_version = source_version;
        snapshot.projection_status = ProjectionStatus::Current.as_str().into();
        candidate.operations.insert(
            operation.clone(),
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        bound_receipts(&mut candidate.operations, Some(&operation));
        let operation_record = candidate
            .operations
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new(
                    "RUNTIME_PROTOCOL_ERROR",
                    "projection import receipt was not staged",
                )
            })?;
        if !project_changed && self.projection_status != ProjectionStatus::Current {
            // The file already describes the authoritative project, but may
            // use the v1 spelling or be missing the compatibility history.
            // An explicit import is also a repair request in this case.
            self.materialize_project_value(&candidate.project)?;
            self.materialize_history_values(
                candidate.revision,
                &candidate.past,
                &candidate.future,
            )?;
            self.projection_status = ProjectionStatus::Current;
            self.projection_digest = projection_digest(&self.project_path).ok();
        }
        let imported_project = candidate.project.clone();
        self.commit_projection_import(
            candidate,
            JournalEvent::ProjectionImport {
                project: imported_project,
                source_version,
                operation: Some(JournalOperation {
                    key: operation,
                    record: operation_record,
                }),
            },
        )
    }

    fn repair_projection(&mut self) -> Result<StateSnapshot, RuntimeError> {
        self.materialize_project()?;
        self.materialize_history()?;
        self.projection_status = ProjectionStatus::Current;
        self.projection_digest = Some(projection_digest(&self.project_path)?);
        self.persist_checkpoint()?;
        truncate_journal(&self.journal_path)?;
        self.journal_records_since_checkpoint = 0;
        Ok(self.snapshot())
    }

    fn reconcile_abandoned_jobs(&mut self) -> bool {
        let mut changed = false;
        for job in self.jobs.values_mut() {
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "failed".into();
                job.error = Some("render job was abandoned when cutd restarted".into());
                changed = true;
            }
        }
        changed
    }
}

pub fn probe(path: &str) -> Result<Value, RuntimeError> {
    Runtime::probe(path)
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

fn domain_error(error: cut_domain::DomainError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
}

/// Convert a validated canonical project to the JSON representation used by
/// the protocol and compatibility projections.  This is intentionally a
/// boundary helper; the runtime actor stores only `Project`.
fn project_value(project: &Project) -> Value {
    serde_json::to_value(project).expect("validated Cut projects serialize")
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

fn load_bootstrap_project(path: &Path) -> Result<(Project, u8), RuntimeError> {
    if !path.is_file() {
        return Ok((cut_domain::empty_project(), 2));
    }
    let text = fs::read_to_string(path).map_err(io_error)?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| RuntimeError::new("INVALID_PROJECT", error.to_string()))?;
    match cut_domain::parse_versioned(&value).map_err(domain_error)? {
        cut_domain::VersionedProject::V1(legacy) => Ok((
            cut_domain::migrate_v1_to_v2(&legacy).map_err(domain_error)?,
            1,
        )),
        cut_domain::VersionedProject::V2(project) => Ok((project, 2)),
    }
}

fn validate_journal_history_entry(entry: &JournalHistoryEntry) -> Result<(), RuntimeError> {
    let _ = cut_domain::serialize_command(&entry.command).map_err(domain_error)?;
    let _ = cut_domain::serialize_command(&entry.inverse).map_err(domain_error)?;
    // Serialization above validates the command shape without pretending that
    // an inverse can be applied to an unrelated project.
    Ok(())
}

fn bound_history(entries: &mut Vec<JournalHistoryEntry>) {
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

fn inspect_projection(path: &Path, expected: &Project) -> ProjectionStatus {
    if !path.is_file() {
        return ProjectionStatus::Missing;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return ProjectionStatus::Diverged;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return ProjectionStatus::Diverged;
    };
    match cut_domain::parse_versioned(&value) {
        Ok(cut_domain::VersionedProject::V1(legacy)) => {
            match cut_domain::migrate_v1_to_v2(&legacy) {
                Ok(project) if project == *expected => ProjectionStatus::Current,
                _ => ProjectionStatus::Diverged,
            }
        }
        Ok(cut_domain::VersionedProject::V2(project)) if project == *expected => {
            ProjectionStatus::Current
        }
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

fn load_checkpoint(directory: &Path) -> Result<Option<PersistedCheckpoint>, RuntimeError> {
    let checkpoint_path = directory.join(CHECKPOINT_FILE);
    let temporary_path = directory.join(CHECKPOINT_TEMP_FILE);
    if !checkpoint_path.is_file() && temporary_path.is_file() {
        let value = read_checkpoint(&temporary_path)?;
        replace_file(&temporary_path, &checkpoint_path)?;
        sync_parent_dir(directory);
        return Ok(Some(value));
    }
    if checkpoint_path.is_file() {
        match read_checkpoint(&checkpoint_path) {
            Ok(value) => {
                // A completed checkpoint supersedes an abandoned write left
                // by a crash.  The temp path is explicit and safe to remove.
                if temporary_path.is_file() {
                    let _ = fs::remove_file(temporary_path);
                }
                return Ok(Some(value));
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
                return Ok(Some(value));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
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
    if format_version == 1 {
        let legacy: LegacyPersistedCheckpoint = serde_json::from_value(value)
            .map_err(|error| RuntimeError::new("INVALID_CHECKPOINT", error.to_string()))?;
        return Ok(PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: legacy.project,
            revision: legacy.revision,
            // v1 checkpoints retained full snapshots. Do not carry that
            // unbounded representation into the journal runtime; the current
            // project remains authoritative and older undo entries are
            // intentionally discarded at this one-time format boundary.
            past: Vec::new(),
            future: Vec::new(),
            journal_sequence: 0,
            operations: legacy.operations,
            imports: legacy.imports,
            proposal: legacy.proposal,
            jobs: HashMap::new(),
            source_version: 2,
            projection_digest: None,
        });
    }
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
    let temporary_path = atomic_temp_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary_path)
            .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
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

fn validate_overlay_shape(overlay: &Value) -> Result<(), RuntimeError> {
    let object = overlay
        .as_object()
        .ok_or_else(|| RuntimeError::new("INVALID_IMPORT", "import overlay must be an object"))?;
    for key in ["id", "type", "kind"] {
        if let Some(value) = object.get(key) {
            if value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .is_none()
            {
                return Err(RuntimeError::new(
                    "INVALID_IMPORT",
                    format!("import overlay.{key} must be a non-empty string"),
                ));
            }
        }
    }
    if let (Some(kind), Some(overlay_type)) = (
        object.get("kind").and_then(Value::as_str),
        object.get("type").and_then(Value::as_str),
    ) {
        if kind != overlay_type {
            return Err(RuntimeError::new(
                "INVALID_IMPORT",
                "import overlay.type and overlay.kind disagree",
            ));
        }
    }
    for key in ["start", "timelineStart"] {
        if let Some(value) = object.get(key) {
            if value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .is_none()
            {
                return Err(RuntimeError::new(
                    "INVALID_IMPORT",
                    format!("import overlay.{key} must be nonnegative"),
                ));
            }
        }
    }
    if let Some(value) = object.get("duration") {
        if value
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0)
            .is_none()
        {
            return Err(RuntimeError::new(
                "INVALID_IMPORT",
                "import overlay.duration must be positive",
            ));
        }
    }
    Ok(())
}

const MAX_PROPOSAL_DIFF: usize = 128;

/// Walk two JSON values in stable key/index order and collect a bounded leaf
/// diff.  Bounding happens before copying another value, so a pathological
/// agent response cannot produce an unbounded proposal payload.
fn structured_diff(from: &Value, to: &Value) -> (Vec<ProposalChange>, bool) {
    let mut changes = Vec::new();
    let mut truncated = false;
    collect_structured_diff(Some(from), Some(to), "$", &mut changes, &mut truncated);
    (changes, truncated)
}

fn collect_structured_diff(
    from: Option<&Value>,
    to: Option<&Value>,
    path: &str,
    changes: &mut Vec<ProposalChange>,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    if let (Some(left), Some(right)) = (from, to) {
        if left == right {
            return;
        }
        if let (Some(left), Some(right)) = (left.as_object(), right.as_object()) {
            let keys: BTreeSet<&str> = left
                .keys()
                .map(String::as_str)
                .chain(right.keys().map(String::as_str))
                .collect();
            for key in keys {
                collect_structured_diff(
                    left.get(key),
                    right.get(key),
                    &format!("{path}.{}", key.replace('.', "\\.")),
                    changes,
                    truncated,
                );
                if *truncated {
                    return;
                }
            }
            return;
        }
        if let (Some(left), Some(right)) = (left.as_array(), right.as_array()) {
            for index in 0..left.len().max(right.len()) {
                collect_structured_diff(
                    left.get(index),
                    right.get(index),
                    &format!("{path}[{index}]"),
                    changes,
                    truncated,
                );
                if *truncated {
                    return;
                }
            }
            return;
        }
    }

    if changes.len() >= MAX_PROPOSAL_DIFF {
        *truncated = true;
        return;
    }
    let kind = match (from, to) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        _ => "changed",
    };
    changes.push(ProposalChange {
        path: path.to_owned(),
        kind: kind.to_owned(),
        before: from.cloned().unwrap_or(Value::Null),
        after: to.cloned().unwrap_or(Value::Null),
    });
}

const MAX_PROPOSAL_IMPACTS: usize = 128;

fn derive_proposal_impacts(
    before: &Project,
    after: &Project,
    diff: &[ProposalChange],
    diff_truncated: bool,
) -> Vec<ProposalImpact> {
    let mut impacts = Vec::new();
    let before_track_indices: BTreeMap<&str, usize> = before
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| (track.id.as_str(), index))
        .collect();
    let after_track_indices: BTreeMap<&str, usize> = after
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| (track.id.as_str(), index))
        .collect();

    for (index, track) in after.tracks.iter().enumerate() {
        if !before_track_indices.contains_key(track.id.as_str())
            && !push_impact(
                &mut impacts,
                ProposalImpact::TrackAdded {
                    track_id: track.id.to_string(),
                    index,
                },
            )
        {
            return impacts;
        }
    }
    for (index, track) in before.tracks.iter().enumerate() {
        if !after_track_indices.contains_key(track.id.as_str())
            && !push_impact(
                &mut impacts,
                ProposalImpact::TrackRemoved {
                    track_id: track.id.to_string(),
                    index,
                },
            )
        {
            return impacts;
        }
    }
    for (to_index, track) in after.tracks.iter().enumerate() {
        let Some(&from_index) = before_track_indices.get(track.id.as_str()) else {
            continue;
        };
        if from_index != to_index
            && !push_impact(
                &mut impacts,
                ProposalImpact::TrackReordered {
                    track_id: track.id.to_string(),
                    from_index,
                    to_index,
                },
            )
        {
            return impacts;
        }
    }

    let before_clips = clip_locations(before);
    let after_clips = clip_locations(after);
    for (clip_id, location) in &after_clips {
        if !before_clips.contains_key(clip_id)
            && !push_impact(
                &mut impacts,
                ProposalImpact::ClipAdded {
                    clip_id: clip_id.clone(),
                    track_id: location.track_id.clone(),
                    timeline: clip_timeline_range(location.clip),
                    source: clip_source_range(location.clip),
                },
            )
        {
            return impacts;
        }
    }
    for (clip_id, location) in &before_clips {
        if !after_clips.contains_key(clip_id)
            && !push_impact(
                &mut impacts,
                ProposalImpact::ClipRemoved {
                    clip_id: clip_id.clone(),
                    track_id: location.track_id.clone(),
                    timeline: clip_timeline_range(location.clip),
                    source: clip_source_range(location.clip),
                },
            )
        {
            return impacts;
        }
    }
    for (clip_id, before_location) in &before_clips {
        let Some(after_location) = after_clips.get(clip_id) else {
            continue;
        };
        let before_timeline = clip_timeline_range(before_location.clip);
        let after_timeline = clip_timeline_range(after_location.clip);
        let before_source = clip_source_range(before_location.clip);
        let after_source = clip_source_range(after_location.clip);
        let timeline_duration_changed = (before_timeline.end - before_timeline.start)
            != (after_timeline.end - after_timeline.start);
        let trim_changed = before_source != after_source || timeline_duration_changed;
        let start_changed = before_timeline.start != after_timeline.start;
        let track_changed = before_location.track_id != after_location.track_id;
        if (track_changed || start_changed)
            && !push_impact(
                &mut impacts,
                ProposalImpact::ClipMoved {
                    clip_id: clip_id.clone(),
                    from_track_id: before_location.track_id.clone(),
                    from: before_timeline.clone(),
                    to_track_id: after_location.track_id.clone(),
                    to: after_timeline.clone(),
                },
            )
        {
            return impacts;
        }
        if trim_changed
            && !push_impact(
                &mut impacts,
                ProposalImpact::ClipTrimmed {
                    clip_id: clip_id.clone(),
                    track_id: after_location.track_id.clone(),
                    before: before_timeline.clone(),
                    after: after_timeline.clone(),
                    source_before: before_source,
                    source_after: after_source,
                },
            )
        {
            return impacts;
        }
        if !append_clip_property_impacts(
            &mut impacts,
            clip_id,
            after_location.track_id.as_str(),
            before_location.clip,
            after_location.clip,
        ) {
            return impacts;
        }
    }

    let before_overlays = overlay_locations(before);
    let after_overlays = overlay_locations(after);
    for (overlay_id, (overlay, index)) in &after_overlays {
        if !before_overlays.contains_key(overlay_id)
            && !push_impact(
                &mut impacts,
                ProposalImpact::OverlayAdded {
                    overlay_id: overlay_id.clone(),
                    index: *index,
                    overlay_kind: overlay.kind().unwrap_or("overlay").to_owned(),
                },
            )
        {
            return impacts;
        }
    }
    for (overlay_id, (overlay, index)) in &before_overlays {
        if !after_overlays.contains_key(overlay_id)
            && !push_impact(
                &mut impacts,
                ProposalImpact::OverlayRemoved {
                    overlay_id: overlay_id.clone(),
                    index: *index,
                    overlay_kind: overlay.kind().unwrap_or("overlay").to_owned(),
                },
            )
        {
            return impacts;
        }
    }
    for (overlay_id, (before_overlay, _)) in &before_overlays {
        let Some((after_overlay, _)) = after_overlays.get(overlay_id) else {
            continue;
        };
        if before_overlay == after_overlay {
            continue;
        }
        if !push_impact(
            &mut impacts,
            ProposalImpact::OverlayChanged {
                overlay_id: overlay_id.clone(),
                overlay_kind: after_overlay.kind().unwrap_or("overlay").to_owned(),
                before: serde_json::to_value(before_overlay).expect("overlay serializes"),
                after: serde_json::to_value(after_overlay).expect("overlay serializes"),
            },
        ) {
            return impacts;
        }
    }

    append_project_setting_impacts(&mut impacts, before, after);
    if impacts.is_empty() && (!diff.is_empty() || diff_truncated) {
        for change in diff {
            if !push_impact(
                &mut impacts,
                ProposalImpact::Unclassified {
                    path: change.path.clone(),
                    before: change.before.clone(),
                    after: change.after.clone(),
                },
            ) {
                break;
            }
        }
    }
    impacts
}

fn push_impact(impacts: &mut Vec<ProposalImpact>, impact: ProposalImpact) -> bool {
    if impacts.len() >= MAX_PROPOSAL_IMPACTS {
        return false;
    }
    impacts.push(impact);
    true
}

struct ClipLocation<'a> {
    track_id: String,
    clip: &'a cut_domain::Clip,
}

fn clip_locations<'a>(project: &'a Project) -> BTreeMap<String, ClipLocation<'a>> {
    project
        .tracks
        .iter()
        .flat_map(|track| {
            track.clips.iter().map(|clip| {
                (
                    clip.id.to_string(),
                    ClipLocation {
                        track_id: track.id.to_string(),
                        clip,
                    },
                )
            })
        })
        .collect()
}

fn clip_timeline_range(clip: &cut_domain::Clip) -> ProposalTimeRange {
    let start = if clip.timeline_start.is_finite() {
        clip.timeline_start.max(0.0)
    } else {
        0.0
    };
    let duration = clip
        .timeline_duration()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .unwrap_or(0.0);
    ProposalTimeRange {
        start,
        end: start + duration,
    }
}

fn clip_source_range(clip: &cut_domain::Clip) -> Option<ProposalTimeRange> {
    match (clip.source_in, clip.source_out) {
        (Some(start), Some(end)) if start.is_finite() && end.is_finite() && end > start => {
            Some(ProposalTimeRange { start, end })
        }
        _ => None,
    }
}

fn append_clip_property_impacts(
    impacts: &mut Vec<ProposalImpact>,
    clip_id: &str,
    track_id: &str,
    before: &cut_domain::Clip,
    after: &cut_domain::Clip,
) -> bool {
    let before = serde_json::to_value(before).expect("clip serializes");
    let after = serde_json::to_value(after).expect("clip serializes");
    let mut keys: BTreeSet<String> = before
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys().cloned())
        .collect();
    keys.extend(
        after
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys().cloned()),
    );
    for property in keys {
        if matches!(
            property.as_str(),
            "id" | "timelineStart" | "duration" | "in" | "out"
        ) {
            continue;
        }
        let before_value = before.get(&property).cloned().unwrap_or(Value::Null);
        let after_value = after.get(&property).cloned().unwrap_or(Value::Null);
        if before_value == after_value {
            continue;
        }
        if !push_impact(
            impacts,
            ProposalImpact::ClipPropertyChanged {
                clip_id: clip_id.to_owned(),
                track_id: track_id.to_owned(),
                property,
                before: before_value,
                after: after_value,
            },
        ) {
            return false;
        }
    }
    true
}

fn overlay_locations(project: &Project) -> BTreeMap<String, (&cut_domain::Overlay, usize)> {
    project
        .overlays
        .iter()
        .enumerate()
        .map(|(index, overlay)| {
            let id = overlay
                .id()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("index:{index}"));
            (id, (overlay, index))
        })
        .collect()
}

fn append_project_setting_impacts(
    impacts: &mut Vec<ProposalImpact>,
    before: &Project,
    after: &Project,
) {
    const COLLECTIONS: &[&str] = &[
        "sources",
        "tracks",
        "overlays",
        "gaps",
        "transitions",
        "effects",
        "markers",
    ];
    let before = project_value(before);
    let after = project_value(after);
    let mut keys: BTreeSet<String> = before
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys().cloned())
        .collect();
    keys.extend(
        after
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys().cloned()),
    );
    for property in keys {
        if COLLECTIONS.contains(&property.as_str()) {
            continue;
        }
        let before_value = before.get(&property).cloned().unwrap_or(Value::Null);
        let after_value = after.get(&property).cloned().unwrap_or(Value::Null);
        if before_value == after_value {
            continue;
        }
        if !push_impact(
            impacts,
            ProposalImpact::ProjectSettingChanged {
                property,
                before: before_value,
                after: after_value,
            },
        ) {
            return;
        }
    }
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

#[cfg(test)]
fn repository_root_from_manifest(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
fn engine_binary_candidates(root: &Path) -> [PathBuf; 2] {
    [
        root.join("target/release/cut-engine"),
        root.join("target/debug/cut-engine"),
    ]
}

fn engine_error(error: cut_engine::EngineError) -> RuntimeError {
    match error {
        cut_engine::EngineError::Cancelled => {
            RuntimeError::new("CANCELLED", "render job was cancelled")
        }
        other => RuntimeError::new("ENGINE_ERROR", other.to_string()),
    }
}

pub fn run_render(
    id: String,
    project: Value,
    output: PathBuf,
    draft: bool,
) -> Result<(), RuntimeError> {
    run_render_with_cancel_file(id, project, output, draft, None, None)
}

/// Run one render worker with an optional bounded range and job-specific
/// cancellation marker. The runtime remains the durable owner of job state;
/// the engine is invoked in-process.
pub fn run_render_with_cancel_file(
    id: String,
    project: Value,
    output: PathBuf,
    draft: bool,
    range: Option<RenderRange>,
    cancel_file: Option<PathBuf>,
) -> Result<(), RuntimeError> {
    run_render_with_cancel_file_progress(id, project, output, draft, range, cancel_file, |_| {})
}

/// Run one render worker and forward bounded engine progress to the caller.
/// The callback runs on this worker thread and must not block on UI state.
pub fn run_render_with_cancel_file_progress<F>(
    id: String,
    project: Value,
    output: PathBuf,
    draft: bool,
    range: Option<RenderRange>,
    cancel_file: Option<PathBuf>,
    mut on_progress: F,
) -> Result<(), RuntimeError>
where
    F: FnMut(f64),
{
    if let Some(range) = range.as_ref() {
        RenderRange::new(range.start, range.duration)?;
    }
    let project = cut_engine::enrich::enrich_project_v2(&project).map_err(engine_error)?;
    let opts = cut_engine::render::RenderV2Options {
        output: output.to_string_lossy().into_owned(),
        draft,
        include_embedded_audio: None,
        prefer_hardware: false,
        force_software: false,
        codec: cut_engine::VideoCodec::H264,
    };
    let cancellation = cut_engine::CancellationToken::new();
    let watcher = cancel_file.as_ref().map(|path| {
        cut_engine::render::spawn_cancel_file_watcher(path.clone(), cancellation.clone())
    });
    let result = match range {
        Some(range) => cut_engine::render::render_v2_range_with_cancellation(
            &project,
            &opts,
            range.start,
            range.duration,
            &cancellation,
            Some(&mut on_progress),
        ),
        None => cut_engine::render::render_v2_with_cancellation(
            &project,
            &opts,
            &cancellation,
            Some(&mut on_progress),
        ),
    };
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

    #[test]
    fn semantic_commands_are_atomic_and_revisioned() {
        let mut runtime = runtime();
        let state = runtime
            .apply(
                json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}),
                Some(0),
            )
            .unwrap();
        assert_eq!(state.revision, 1);
        assert!(state.can_undo);
        assert!(runtime
            .apply(json!({"type":"remove_track","trackId":"missing"}), Some(1))
            .is_err());
        assert_eq!(runtime.snapshot().revision, 1);
        runtime.undo(Some(1)).unwrap();
        assert!(runtime.snapshot().can_redo);
        runtime.redo(Some(2)).unwrap();
        assert_eq!(runtime.snapshot().revision, 3);
    }

    #[test]
    fn conflicts_are_machine_readable() {
        let mut runtime = runtime();
        let error = runtime.undo(Some(2)).unwrap_err();
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
        let command = json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}});
        let first = runtime
            .apply_request("ui", "key-1", command.clone(), 0)
            .unwrap();
        let replay = runtime.apply_request("ui", "key-1", command, 0).unwrap();
        assert_eq!(first.revision, replay.revision);
        let error = runtime
            .apply_request(
                "ui",
                "key-1",
                json!({"type":"remove_track","trackId":"titles"}),
                1,
            )
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn agent_proposal_does_not_mutate_until_accepted() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let staged = runtime
            .stage_agent_proposal(
                "agent-1",
                before.revision,
                &[json!({
                    "idempotencyKey": "agent-1",
                    "commands": [{"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}]
                })],
            )
            .unwrap();
        assert_eq!(staged.project, before.project);
        assert_eq!(staged.revision, before.revision);
        assert!(!staged.can_undo);
        assert!(matches!(
            staged.pending_proposal,
            Some(PendingProposal::Pending { .. })
        ));
    }

    #[test]
    fn rejecting_a_proposal_keeps_canonical_state_and_converges() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let staged = runtime
            .stage_agent_proposal(
                "agent-reject",
                0,
                &[json!({
                    "idempotencyKey": "agent-reject",
                    "commands": [{"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}]
                })],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending {
                proposal_id,
                freshness,
                ..
            } => {
                assert_eq!(*freshness, ProposalFreshness::Current);
                proposal_id.clone()
            }
            _ => panic!("expected pending proposal"),
        };
        let rejected = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(rejected.project, before.project);
        assert_eq!(rejected.revision, before.revision);
        assert!(!rejected.can_undo);
        assert!(matches!(
            rejected.pending_proposal,
            Some(PendingProposal::Rejected { .. })
        ));
        let replay = runtime.reject_proposal(&proposal_id).unwrap();
        assert_eq!(replay.project, before.project);
        assert_eq!(replay.revision, before.revision);
    }

    #[test]
    fn accepting_a_proposal_is_one_revision_and_one_undo_entry() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_agent_proposal(
                "agent-accept",
                0,
                &[
                    json!({
                        "idempotencyKey": "agent-accept",
                        "commands": [{"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}]
                    }),
                    json!({
                        "idempotencyKey": "agent-accept-2",
                        "commands": [{"type":"set_metadata","patch":{"intent":"approved"}}]
                    }),
                ],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
            _ => panic!("expected pending proposal"),
        };
        let accepted = runtime
            .accept_proposal(&proposal_id, "ui", "accept-1", 0)
            .unwrap();
        assert_eq!(accepted.revision, 1);
        assert!(accepted.can_undo);
        assert_eq!(accepted.project["metadata"]["intent"], "approved");
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
        runtime.undo(Some(1)).unwrap();
        assert_eq!(runtime.snapshot().revision, 2);
        assert!(!runtime.snapshot().project["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|track| track["id"] == "titles"));
    }

    #[test]
    fn accepted_proposal_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_agent_proposal(
                "agent-receipt",
                0,
                &[json!({
                    "idempotencyKey": "agent-receipt",
                    "commands": [{"type":"set_metadata","patch":{"approved":true}}]
                })],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
            _ => panic!("expected pending proposal"),
        };
        let first = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        let later = runtime
            .apply(
                json!({"type":"set_metadata","patch":{"later":true}}),
                Some(first.revision),
            )
            .unwrap();
        assert_eq!(later.revision, 2);

        let replay = runtime
            .accept_proposal(&proposal_id, "ui", "accept-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.project, first.project);
        assert_eq!(replay.project["metadata"]["approved"], true);
        assert!(replay.project["metadata"].get("later").is_none());

        let error = runtime
            .accept_proposal(&proposal_id, "ui", "accept-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn accepting_a_stale_proposal_conflicts_without_clearing_it() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_agent_proposal(
                "agent-stale",
                0,
                &[json!({
                    "idempotencyKey": "agent-stale",
                    "commands": [{"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}]
                })],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
            _ => panic!("expected pending proposal"),
        };
        runtime
            .apply(
                json!({"type":"set_metadata","patch":{"changed":true}}),
                Some(0),
            )
            .unwrap();
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
    fn invalid_agent_batch_is_all_or_nothing() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let error = runtime
            .stage_agent_proposal(
                "agent-invalid",
                0,
                &[json!({
                    "idempotencyKey": "agent-invalid",
                    "commands": [
                        {"type":"add_track","track":{"id":"titles","type":"title","clips":[]}},
                        {"type":"remove_track","trackId":"missing"}
                    ]
                })],
            )
            .unwrap_err();
        assert_eq!(error.code, "NOT_FOUND");
        let after = runtime.snapshot();
        assert_eq!(after.project, before.project);
        assert_eq!(after.revision, before.revision);
        assert!(after.pending_proposal.is_none());
    }

    #[test]
    fn revision_and_history_survive_a_runtime_restart() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-restart-{}", unique_id()));
        {
            let mut runtime = Runtime::open(&directory).unwrap();
            runtime
                .apply(
                    json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}),
                    Some(0),
                )
                .unwrap();
            assert_eq!(runtime.snapshot().revision, 1);
        }
        let mut reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, 1);
        assert!(reopened.snapshot().can_undo);
        reopened.undo(Some(1)).unwrap();
        assert_eq!(reopened.snapshot().revision, 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn source_and_overlay_commands_mutate_collections() {
        let mut runtime = runtime();
        runtime
            .apply(
                json!({"type":"add_source","source":{"id":"cam-a","path":"/media/cam-a.mp4","duration":120}}),
                Some(0),
            )
            .unwrap();
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.project["sources"].as_array().unwrap().len(), 1);
        assert_eq!(
            snapshot.project["sources"].as_array().unwrap()[0]["id"],
            "cam-a"
        );

        runtime
            .apply(
                json!({"type":"add_overlay","overlay":{"id":"subs-1","type":"subtitles","path":"/media/subs.srt"}}),
                Some(1),
            )
            .unwrap();
        assert_eq!(
            runtime.snapshot().project["overlays"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let duplicate_source = runtime.apply(
            json!({"type":"add_source","source":{"id":"cam-a","path":"/media/other.mp4"}}),
            Some(2),
        );
        assert_eq!(duplicate_source.unwrap_err().code, "INVALID_COMMAND");

        let duplicate_overlay = runtime.apply(
            json!({"type":"add_overlay","overlay":{"id":"subs-1","type":"subtitles","path":"/other.srt"}}),
            Some(2),
        );
        assert_eq!(duplicate_overlay.unwrap_err().code, "INVALID_COMMAND");

        runtime
            .apply(
                json!({"type":"remove_overlay","overlayId":"subs-1"}),
                Some(2),
            )
            .unwrap();
        assert!(runtime.snapshot().project["overlays"]
            .as_array()
            .unwrap()
            .is_empty());

        let missing = runtime.apply(
            json!({"type":"remove_overlay","overlayId":"missing"}),
            Some(3),
        );
        assert_eq!(missing.unwrap_err().code, "NOT_FOUND");
    }

    #[test]
    fn media_import_is_a_revisioned_idempotent_transaction() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-import-{}", unique_id()));
        let media = directory.join("sample.mp4");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&media, b"not decoded in this unit test").unwrap();
        {
            let mut runtime = Runtime::open(&directory).unwrap();
            let (first, source_id, clip_id) = runtime
                .import_media_request("ui", "import-1", media.to_str().unwrap(), 0, None, None)
                .unwrap();
            let (replay, replay_source, replay_clip) = runtime
                .import_media_request("ui", "import-1", media.to_str().unwrap(), 0, None, None)
                .unwrap();
            assert_eq!(first.revision, 1);
            assert_eq!(first.revision, replay.revision);
            assert_eq!((source_id, clip_id), (replay_source, replay_clip));
            let error = runtime
                .import_media_request("ui", "import-1", "other.mp4", 1, None, None)
                .unwrap_err();
            assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_projection_does_not_advance_memory_or_checkpoint() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-persist-fail-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        let before = runtime.snapshot();
        // The checkpoint remains writable, while the legacy project
        // projection is deliberately made unrenamable.  This exercises the
        // failure between candidate construction and the commit record.
        fs::remove_file(directory.join(PROJECT_FILE)).unwrap();
        fs::create_dir(directory.join(PROJECT_FILE)).unwrap();
        let error = runtime
            .apply(
                json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}),
                Some(before.revision),
            )
            .unwrap_err();
        assert_eq!(error.code, "IO_ERROR");
        assert_eq!(runtime.snapshot().revision, before.revision);
        assert_eq!(runtime.snapshot().project, before.project);
        drop(runtime);
        fs::remove_dir_all(directory.join(PROJECT_FILE)).unwrap();
        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().revision, before.revision);
        assert_eq!(reopened.snapshot().project, before.project);
        drop(reopened);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn operation_receipts_and_revision_survive_restart() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-receipt-{}", unique_id()));
        let command = json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}});
        let first = {
            let mut runtime = Runtime::open(&directory).unwrap();
            runtime
                .apply_request("ui", "request-1", command.clone(), 0)
                .unwrap()
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        let replay = reopened
            .apply_request("ui", "request-1", command, 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.project, first.project);
        let conflict = reopened
            .apply_request(
                "ui",
                "request-1",
                json!({"type":"set_metadata","patch":{"changed":true}}),
                1,
            )
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
                .stage_agent_proposal(
                    "agent-restart",
                    0,
                    &[json!({
                        "idempotencyKey":"agent-restart",
                        "commands":[{"type":"set_metadata","patch":{"survives":true}}]
                    })],
                )
                .unwrap();
            match staged.pending_proposal.unwrap() {
                PendingProposal::Pending { proposal_id, .. } => proposal_id,
                _ => panic!("expected pending proposal"),
            }
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        assert!(matches!(
            reopened.snapshot().pending_proposal,
            Some(PendingProposal::Pending {
                impacts: Some(impacts),
                ..
            }) if !impacts.is_empty()
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
                .stage_agent_proposal(
                    "agent-accepted-restart",
                    0,
                    &[json!({
                        "idempotencyKey":"agent-accepted-restart",
                        "commands":[{"type":"set_metadata","patch":{"persisted":true}}]
                    })],
                )
                .unwrap();
            let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
                PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
                _ => panic!("expected pending proposal"),
            };
            runtime
                .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
                .unwrap();
            proposal_id
        };
        let mut reopened = Runtime::open(&directory).unwrap();
        reopened
            .apply(
                json!({"type":"set_metadata","patch":{"later":true}}),
                Some(1),
            )
            .unwrap();
        let replay = reopened
            .accept_proposal(&proposal_id, "ui", "accept-persisted", 0)
            .unwrap();
        assert_eq!(replay.revision, 1);
        assert_eq!(replay.project["metadata"]["persisted"], true);
        assert!(replay.project["metadata"].get("later").is_none());
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
        let left = handle.clone();
        let right = handle.clone();
        let command = json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}});
        let first = thread::spawn(move || left.apply_request("ui", "left", command, 0));
        let command =
            json!({"type":"add_track","track":{"id":"audio-titles","type":"title","clips":[]}});
        let second = thread::spawn(move || right.apply_request("agent", "right", command, 0));
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
        let error = if let Err(error) = first {
            error
        } else {
            second.unwrap_err()
        };
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(handle.get_state().unwrap().revision, 1);
        drop(handle);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn request_envelope_round_trips_typed_operation() {
        let envelope = RequestEnvelope::new(
            "request-1",
            RuntimeRequest::Apply {
                actor: "ui".into(),
                idempotency_key: "key-1".into(),
                base_revision: 0,
                command: json!({"type":"set_metadata","patch":{"title":"Cut"}}),
            },
        );
        let encoded = serde_json::to_string(&envelope).unwrap();
        let decoded: RequestEnvelope = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.id, "request-1");
        assert!(matches!(decoded.request, RuntimeRequest::Apply { .. }));
    }

    #[test]
    fn pending_proposal_serializes_gui_field_names_exactly() {
        let pending = serde_json::to_value(PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 2,
            command: json!({"type":"set_metadata","patch":{"title":"Cut"}}),
            diff: vec![ProposalChange {
                path: "$.metadata.title".into(),
                kind: "added".into(),
                before: Value::Null,
                after: json!("Cut"),
            }],
            diff_truncated: true,
            freshness: ProposalFreshness::Current,
            impacts: None,
        })
        .unwrap();
        assert_eq!(
            pending,
            json!({
                "status": "pending",
                "proposalId": "proposal-1",
                "baseRevision": 2,
                "command": {"type":"set_metadata","patch":{"title":"Cut"}},
                "diff": [{
                    "path":"$.metadata.title",
                    "kind":"added",
                    "before": null,
                    "after":"Cut"
                }],
                "diffTruncated": true
            })
        );
        assert_eq!(
            serde_json::to_value(PendingProposal::Accepted {
                proposal_id: "proposal-1".into(),
                base_revision: 2,
                applied_revision: 3,
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
    fn semantic_proposal_impacts_cover_clip_geometry_and_properties() {
        let before = cut_domain::parse_project(&json!({
            "version": 2,
            "fps": 30.0,
            "sources": [{"id":"source-1","path":"shot.mp4","duration":12.0}],
            "tracks": [{
                "id":"video-1",
                "type":"video",
                "clips": [{
                    "id":"clip-1",
                    "sourceId":"source-1",
                    "in":0.0,
                    "out":4.0,
                    "timelineStart":1.0,
                    "duration":4.0,
                    "volume":1.0
                }]
            }],
            "overlays": [],
            "gaps": [],
            "transitions": [],
            "effects": [],
            "markers": []
        }))
        .expect("valid project");
        let after = cut_domain::parse_project(&json!({
            "version": 2,
            "fps": 30.0,
            "sources": [{"id":"source-1","path":"shot.mp4","duration":12.0}],
            "tracks": [{
                "id":"video-1",
                "type":"video",
                "clips": [{
                    "id":"clip-1",
                    "sourceId":"source-1",
                    "in":1.0,
                    "out":4.0,
                    "timelineStart":2.0,
                    "duration":3.0,
                    "volume":0.5
                }]
            }],
            "overlays": [],
            "gaps": [],
            "transitions": [],
            "effects": [],
            "markers": []
        }))
        .expect("valid project");
        let (diff, truncated) = structured_diff(&project_value(&before), &project_value(&after));
        let impacts = derive_proposal_impacts(&before, &after, &diff, truncated);

        assert!(impacts.iter().any(|impact| matches!(
            impact,
            ProposalImpact::ClipMoved {
                clip_id,
                from_track_id,
                to_track_id,
                from,
                to,
            } if clip_id == "clip-1"
                && from_track_id == "video-1"
                && to_track_id == "video-1"
                && from.start == 1.0
                && to.start == 2.0
        )));
        assert!(impacts.iter().any(|impact| matches!(
            impact,
            ProposalImpact::ClipTrimmed {
                clip_id,
                source_before: Some(source_before),
                source_after: Some(source_after),
                ..
            } if clip_id == "clip-1"
                && source_before.start == 0.0
                && source_after.start == 1.0
        )));
        assert!(impacts.iter().any(|impact| matches!(
            impact,
            ProposalImpact::ClipPropertyChanged {
                clip_id,
                property,
                before,
                after,
                ..
            } if clip_id == "clip-1"
                && property == "volume"
                && before == &json!(1.0)
                && after == &json!(0.5)
        )));
    }

    #[test]
    fn pending_proposal_without_impacts_remains_readable() {
        let legacy = json!({
            "status": "pending",
            "proposalId": "legacy",
            "baseRevision": 4,
            "command": {"type":"batch","commands":[]},
            "diff": [],
            "diffTruncated": false
        });
        let decoded: PendingProposal = serde_json::from_value(legacy).expect("legacy proposal");
        assert!(matches!(
            decoded,
            PendingProposal::Pending {
                impacts: None,
                proposal_id,
                ..
            } if proposal_id == "legacy"
        ));
    }

    #[test]
    fn engine_path_resolution_uses_workspace_root_after_crate_move() {
        let manifest = Path::new("/workspace/cut/crates/cut-runtime");
        let root = repository_root_from_manifest(manifest);
        assert_eq!(root, Path::new("/workspace/cut"));
        assert_eq!(
            engine_binary_candidates(&root),
            [
                PathBuf::from("/workspace/cut/target/release/cut-engine"),
                PathBuf::from("/workspace/cut/target/debug/cut-engine")
            ]
        );
    }

    #[test]
    fn jsonl_rejects_empty_idempotency_keys_at_runtime_boundary() {
        let directory = std::env::temp_dir().join(format!("cut-runtime-jsonl-{}", unique_id()));
        let requests = [
            json!({
                "protocolVersion":1,
                "id":"apply-empty",
                "operation":"apply",
                "actor":"ui",
                "idempotencyKey":"",
                "baseRevision":0,
                "command":{"type":"set_metadata","patch":{"x":true}}
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
    fn snapshot_duration_matches_canonical_clip_and_gap_semantics() {
        let project = cut_domain::parse_project(&json!({
            "version": 2,
            "fps": 30,
            "duration": 1.5,
            "sources": [],
            "tracks": [{
                "id": "titles",
                "type": "title",
                "clips": [
                    {"id":"a","timelineStart":2.0,"duration":4.0},
                    {"id":"b","timelineStart":8.0,"in":1.0,"out":5.0,"speed":2.0}
                ]
            }],
            "gaps": [{"timelineStart":12.0,"duration":3.0}]
        }))
        .unwrap();
        assert_eq!(cut_domain::project_duration(&project), 15.0);
    }

    #[test]
    fn import_rejects_non_files_and_malformed_overlays_but_keeps_metadata_injection() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-import-boundary-{}", unique_id()));
        let media = directory.join("sample.mp4");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&media, b"not decoded in this unit test").unwrap();
        let mut runtime = Runtime::open(&directory).unwrap();

        let missing = runtime
            .import_media_request("ui", "missing", "/definitely/not-a-file", 0, None, None)
            .unwrap_err();
        assert_eq!(missing.code, "INVALID_IMPORT");
        let directory_error = runtime
            .import_media_request(
                "ui",
                "directory",
                directory.to_str().unwrap(),
                0,
                None,
                None,
            )
            .unwrap_err();
        assert_eq!(directory_error.code, "INVALID_IMPORT");
        let overlay_error = runtime
            .import_media_request(
                "ui",
                "bad-overlay",
                media.to_str().unwrap(),
                0,
                None,
                Some(json!("not an overlay")),
            )
            .unwrap_err();
        assert_eq!(overlay_error.code, "INVALID_IMPORT");

        let (state, _, _) = runtime
            .import_media_request(
                "ui",
                "metadata",
                media.to_str().unwrap(),
                0,
                Some(json!({"duration":2.5,"width":1920,"height":1080})),
                Some(json!({"id":"overlay-1","type":"subtitles"})),
            )
            .unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.sources[0]["duration"], 2.5);
        assert_eq!(state.project["overlays"][0]["id"], "overlay-1");
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn durable_rejection_receipt_survives_later_mutation_and_rejects_reuse() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_agent_proposal(
                "agent-reject-receipt",
                0,
                &[json!({
                    "idempotencyKey":"agent-reject-receipt",
                    "commands":[{"type":"set_metadata","patch":{"proposed":true}}]
                })],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
            _ => panic!("expected pending proposal"),
        };
        let first = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        runtime
            .apply(
                json!({"type":"set_metadata","patch":{"later":true}}),
                Some(first.revision),
            )
            .unwrap();
        let replay = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-receipt", 0)
            .unwrap();
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.project, first.project);
        let error = runtime
            .reject_proposal_request(&proposal_id, "ui", "reject-other", 0)
            .unwrap_err();
        assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    }

    #[test]
    fn journal_stores_typed_commands_without_full_snapshot_history() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-shape-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        runtime
            .apply(
                json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}),
                Some(0),
            )
            .unwrap();
        let journal = fs::read_to_string(directory.join(JOURNAL_FILE)).unwrap();
        assert!(journal.contains("\"command\""));
        assert!(journal.contains("\"inverse\""));
        assert!(!journal.contains("\"before\""));
        assert!(!journal.contains("\"after\""));

        for index in 0..(MAX_HISTORY_ENTRIES + 8) {
            runtime
                .apply(json!({"type":"set_metadata","patch":{"index":index}}), None)
                .unwrap();
        }
        let history: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(HISTORY_FILE)).unwrap())
                .unwrap();
        assert_eq!(
            history["past"].as_array().unwrap().len(),
            MAX_HISTORY_ENTRIES
        );
        assert!(history["future"].as_array().unwrap().is_empty());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn checkpoint_and_torn_journal_tail_recover_authoritative_state() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-journal-recovery-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        for index in 0..(CHECKPOINT_INTERVAL + 1) {
            runtime
                .apply(json!({"type":"set_metadata","patch":{"index":index}}), None)
                .unwrap();
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
            reopened.snapshot().project["metadata"]["index"],
            CHECKPOINT_INTERVAL
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
        runtime
            .apply(
                json!({"type":"set_metadata","patch":{"saved":true}}),
                Some(0),
            )
            .unwrap();
        drop(runtime);
        let checkpoint = fs::read(directory.join(CHECKPOINT_FILE)).unwrap();
        fs::write(directory.join(CHECKPOINT_TEMP_FILE), checkpoint).unwrap();
        fs::write(directory.join(CHECKPOINT_FILE), b"{broken").unwrap();

        let runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().revision, 1);
        assert_eq!(runtime.snapshot().project["metadata"]["saved"], true);
        assert!(!directory.join(CHECKPOINT_TEMP_FILE).exists());
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn first_launch_migrates_v1_once_and_publishes_v2_projection() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-v1-bootstrap-{}", unique_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(PROJECT_FILE),
            serde_json::to_vec(&json!({
                "version": 1,
                "fps": 24,
                "sources": [{"id":"src1","path":"clip.mp4","duration":4}],
                "tracks": [{"id":"v1","clips":[{"id":"clip1","src":"src1","in":0,"out":4}]}],
                "overlays": []
            }))
            .unwrap(),
        )
        .unwrap();
        let runtime = Runtime::open(&directory).unwrap();
        let state = runtime.snapshot();
        assert_eq!(state.source_version, 1);
        assert_eq!(state.projection_status, "current");
        assert_eq!(state.project["version"], 2);
        assert!(directory.join(CHECKPOINT_FILE).is_file());
        drop(runtime);
        let projection: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(PROJECT_FILE)).unwrap())
                .unwrap();
        assert_eq!(projection["version"], 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn divergent_projection_requires_repair_or_explicit_import() {
        let directory =
            std::env::temp_dir().join(format!("cut-runtime-projection-policy-{}", unique_id()));
        let mut runtime = Runtime::open(&directory).unwrap();
        runtime
            .apply(
                json!({"type":"set_metadata","patch":{"authoritative":true}}),
                Some(0),
            )
            .unwrap();
        drop(runtime);

        let mut edited: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(PROJECT_FILE)).unwrap())
                .unwrap();
        edited["name"] = json!("human edit");
        fs::write(
            directory.join(PROJECT_FILE),
            serde_json::to_vec_pretty(&edited).unwrap(),
        )
        .unwrap();

        let mut runtime = Runtime::open(&directory).unwrap();
        assert_eq!(runtime.snapshot().projection_status, "diverged");
        let blocked = runtime.apply(
            json!({"type":"set_metadata","patch":{"blocked":true}}),
            Some(1),
        );
        assert_eq!(blocked.unwrap_err().code, "PROJECTION_DIVERGED");
        let repaired = runtime.repair_projection().unwrap();
        assert_eq!(repaired.projection_status, "current");
        assert_eq!(repaired.project["name"], "Untitled project");

        let mut imported = repaired.project.clone();
        imported["name"] = json!("explicit import");
        fs::write(
            directory.join(PROJECT_FILE),
            serde_json::to_vec_pretty(&imported).unwrap(),
        )
        .unwrap();
        let imported_state = runtime
            .import_projection("ui", "projection-import-1", repaired.revision)
            .unwrap();
        assert_eq!(imported_state.revision, repaired.revision + 1);
        assert_eq!(imported_state.project["name"], "explicit import");
        assert!(!imported_state.can_undo);
        assert_eq!(imported_state.projection_status, "current");
        drop(runtime);

        let reopened = Runtime::open(&directory).unwrap();
        assert_eq!(reopened.snapshot().project["name"], "explicit import");
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
            let (job, _, _) = runtime.start_render("preview").unwrap();
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
            let (cancelled, _, _) = runtime.start_render("preview").unwrap();
            let (queued, _, _) = runtime.start_render("preview").unwrap();
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
    fn proposal_preview_reconstructs_before_and_after_without_mutating_state() {
        let mut runtime = runtime();
        let staged = runtime
            .stage_agent_proposal(
                "preview-proposal",
                0,
                &[json!({
                    "idempotencyKey": "preview-proposal",
                    "commands": [{"type":"set_metadata","patch":{"title":"Preview"}}]
                })],
            )
            .unwrap();
        let proposal_id = match staged.pending_proposal.as_ref().unwrap() {
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
            _ => panic!("expected pending proposal"),
        };
        let before_revision = runtime.snapshot().revision;
        let (before, after) = runtime.proposal_preview(&proposal_id).unwrap();
        assert_eq!(before["metadata"].get("title"), None);
        assert_eq!(after["metadata"]["title"], "Preview");
        assert_eq!(runtime.snapshot().revision, before_revision);

        runtime
            .apply(
                json!({"type":"set_metadata","patch":{"other":true}}),
                Some(0),
            )
            .unwrap();
        assert_eq!(
            runtime.proposal_preview(&proposal_id).unwrap_err().code,
            "PROPOSAL_STALE"
        );
    }
}
