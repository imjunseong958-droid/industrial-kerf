//! Desktop-owned editing runtime.
//!
//! This is deliberately a semantic boundary.  The web client sends project
//! commands and receives snapshots; it never mutates a local copy of the
//! timeline.  Browser/MCP clients use their own host adapter and do not pass
//! through this runtime.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const PROJECT_FILE: &str = "project.cut.json";
const HISTORY_FILE: &str = "project.cut.history.json";
const LOCK_FILE: &str = ".cut-runtime.lock";
const CHECKPOINT_FILE: &str = ".cut-runtime.checkpoint.json";
const CHECKPOINT_TEMP_FILE: &str = ".cut-runtime.checkpoint.json.tmp";
const CHECKPOINT_VERSION: u64 = 1;

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
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::AlreadyExists
            ) {
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

#[derive(Debug, Serialize, serde::Deserialize)]
struct PersistedHistory {
    revision: u64,
    past: Vec<Value>,
    future: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCheckpoint {
    format_version: u64,
    project: Value,
    revision: u64,
    past: Vec<Value>,
    future: Vec<Value>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    #[serde(default)]
    proposal: Option<ProposalRecord>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderJob {
    pub job_id: String,
    pub status: String,
    pub output_url: Option<String>,
    pub output_path: Option<String>,
    pub error: Option<String>,
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

#[derive(Debug, Clone)]
struct JobRecord {
    job: RenderJob,
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

#[derive(Debug, Clone)]
struct DurableState {
    project: Value,
    revision: u64,
    past: Vec<Value>,
    future: Vec<Value>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    proposal: Option<ProposalRecord>,
}

#[derive(Debug)]
struct Runtime {
    _lock: RuntimeLock,
    project_path: PathBuf,
    project: Value,
    revision: u64,
    past: Vec<Value>,
    future: Vec<Value>,
    jobs: HashMap<String, JobRecord>,
    operations: HashMap<String, OperationRecord>,
    imports: HashMap<String, ImportRecord>,
    proposal: Option<ProposalRecord>,
}

/// Versioned operations accepted by the runtime mailbox.  JSON/IPC payloads
/// are parsed into this enum at the protocol boundary before they reach the
/// reducer, so the actor never branches on unvalidated field maps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RuntimeRequest {
    GetState,
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
        match self.request(RuntimeRequest::StartRender {
            quality: quality.to_owned(),
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
        match self.request(RuntimeRequest::UpdateJob {
            job_id: job_id.to_owned(),
            status: status.to_owned(),
            error,
            output_url,
        })? {
            RuntimeResponse::Empty => Ok(()),
            _ => Err(RuntimeError::new(
                "RUNTIME_PROTOCOL_ERROR",
                "expected empty response",
            )),
        }
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

/// Run the persistent JSONL daemon over stdin/stdout.  A single actor owns
/// the runtime; malformed requests are rejected at this transport boundary.
pub fn run_stdio<R: BufRead, W: Write>(
    directory: &Path,
    input: R,
    mut output: W,
) -> Result<(), RuntimeError> {
    let handle = RuntimeHandle::open(directory)?;
    for line in input.lines() {
        let line = line.map_err(io_error)?;
        if line.trim().is_empty() {
            continue;
        }
        let (id, result) = match serde_json::from_str::<Value>(&line) {
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
        };
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
        let history_path = directory.join(HISTORY_FILE);
        let checkpoint = load_checkpoint(directory)?;
        let (project, revision, past, future, operations, imports, proposal) =
            if let Some(checkpoint) = checkpoint {
                validate_project(&checkpoint.project)?;
                for snapshot in checkpoint.past.iter().chain(checkpoint.future.iter()) {
                    validate_project(snapshot)?;
                }
                for record in checkpoint.operations.values() {
                    validate_project(&record.snapshot.project)?;
                }
                for record in checkpoint.imports.values() {
                    validate_project(&record.snapshot.project)?;
                }
                (
                    checkpoint.project,
                    checkpoint.revision,
                    checkpoint.past,
                    checkpoint.future,
                    checkpoint.operations,
                    checkpoint.imports,
                    checkpoint.proposal,
                )
            } else {
                let project = if project_path.is_file() {
                    let text = fs::read_to_string(&project_path).map_err(io_error)?;
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|error| RuntimeError::new("INVALID_PROJECT", error.to_string()))?;
                    validate_project(&value)?;
                    value
                } else {
                    empty_project()
                };
                let persisted = if history_path.is_file() {
                    let text = fs::read_to_string(&history_path).map_err(io_error)?;
                    let history: PersistedHistory = serde_json::from_str(&text)
                        .map_err(|error| RuntimeError::new("INVALID_HISTORY", error.to_string()))?;
                    for snapshot in history.past.iter().chain(history.future.iter()) {
                        validate_project(snapshot)?;
                    }
                    Some(history)
                } else {
                    None
                };
                let (revision, past, future) = persisted
                    .map(|history| (history.revision, history.past, history.future))
                    .unwrap_or((0, Vec::new(), Vec::new()));
                (
                    project,
                    revision,
                    past,
                    future,
                    HashMap::new(),
                    HashMap::new(),
                    None,
                )
            };
        let runtime = Self {
            _lock: lock,
            project_path,
            project,
            revision,
            past,
            future,
            jobs: HashMap::new(),
            operations,
            imports,
            proposal,
        };
        // The checkpoint is the commit record.  Legacy projects are imported
        // once into it before any mutation, so a later projection write cannot
        // leave restart recovery ambiguous.
        if !directory.join(CHECKPOINT_FILE).is_file() {
            runtime.persist_checkpoint()?;
        } else {
            // Repair the human-readable projections opportunistically.  They
            // are exports; the checkpoint remains authoritative on failure.
            let _ = runtime.materialize_project();
            let _ = runtime.materialize_history();
        }
        Ok(runtime)
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.snapshot_for_parts(&self.project, self.revision, &self.past, &self.future)
    }

    fn snapshot_for_parts(
        &self,
        project: &Value,
        revision: u64,
        past: &[Value],
        future: &[Value],
    ) -> StateSnapshot {
        self.snapshot_for_parts_with_proposal(project, revision, past, future, &self.proposal)
    }

    fn snapshot_for_parts_with_proposal(
        &self,
        project: &Value,
        revision: u64,
        past: &[Value],
        future: &[Value],
        proposal: &Option<ProposalRecord>,
    ) -> StateSnapshot {
        let sources = project
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        StateSnapshot {
            duration: project_duration(project),
            project: project.clone(),
            revision,
            can_undo: !past.is_empty(),
            can_redo: !future.is_empty(),
            sources,
            pending_proposal: proposal.as_ref().map(|record| record.lifecycle.clone()),
        }
    }

    fn durable_state(&self) -> DurableState {
        DurableState {
            project: self.project.clone(),
            revision: self.revision,
            past: self.past.clone(),
            future: self.future.clone(),
            operations: self.operations.clone(),
            imports: self.imports.clone(),
            proposal: self.proposal.clone(),
        }
    }

    fn candidate_state(
        &self,
        project: Value,
        revision: u64,
        past: Vec<Value>,
        future: Vec<Value>,
    ) -> DurableState {
        let mut candidate = self.durable_state();
        candidate.project = project;
        candidate.revision = revision;
        candidate.past = past;
        candidate.future = future;
        candidate
    }

    fn commit_candidate(&mut self, candidate: DurableState) -> Result<StateSnapshot, RuntimeError> {
        // Persist first.  No in-memory field is changed until all durable
        // projections and the checkpoint commit succeed.
        self.persist_candidate(&candidate)?;
        self.project = candidate.project;
        self.revision = candidate.revision;
        self.past = candidate.past;
        self.future = candidate.future;
        self.operations = candidate.operations;
        self.imports = candidate.imports;
        self.proposal = candidate.proposal;
        Ok(self.snapshot())
    }

    pub fn apply(
        &mut self,
        command: Value,
        expected_revision: Option<u64>,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.check_revision(expected_revision)?;
        let mut next = self.project.clone();
        apply_command(&mut next, &command)?;
        validate_project(&next)?;
        let mut past = self.past.clone();
        past.push(self.project.clone());
        let revision = self.revision + 1;
        self.commit_candidate(self.candidate_state(next, revision, past, Vec::new()))
    }

    pub fn apply_request(
        &mut self,
        actor: &str,
        key: &str,
        command: Value,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        validate_request_identity(actor, key)?;
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
        let mut next = self.project.clone();
        apply_command(&mut next, &command)?;
        validate_project(&next)?;
        let mut past = self.past.clone();
        past.push(self.project.clone());
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
        self.commit_candidate(candidate)
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

        let mut candidate = self.project.clone();
        apply_command(&mut candidate, &command)?;
        validate_project(&candidate)?;
        let (diff, diff_truncated) = structured_diff(&self.project, &candidate);
        let proposal_id = format!("proposal-{}", unique_id());
        let proposal = Some(ProposalRecord {
            lifecycle: PendingProposal::Pending {
                proposal_id,
                base_revision,
                command,
                diff,
                diff_truncated,
            },
            request_key: request_key.to_owned(),
            fingerprint,
            receipt: None,
        });
        let mut candidate = self.durable_state();
        candidate.proposal = proposal;
        self.persist_candidate(&candidate)?;
        let proposal = candidate.proposal;
        self.proposal = proposal;
        Ok(self.snapshot())
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
                self.persist_candidate(&candidate)?;
                self.proposal = candidate.proposal;
                Ok(self.snapshot())
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
                    operation,
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
                self.commit_candidate(candidate)
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
        let Some(previous) = self.past.last().cloned() else {
            return Ok(self.snapshot());
        };
        let mut past = self.past.clone();
        past.pop();
        let mut future = self.future.clone();
        future.push(self.project.clone());
        self.commit_candidate(self.candidate_state(previous, self.revision + 1, past, future))
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
        let Some(previous) = self.past.last().cloned() else {
            let mut candidate = self.durable_state();
            let snapshot = self.snapshot_for_parts_with_proposal(
                &candidate.project,
                candidate.revision,
                &candidate.past,
                &candidate.future,
                &candidate.proposal,
            );
            candidate.operations.insert(
                operation,
                OperationRecord {
                    fingerprint,
                    snapshot: snapshot.clone(),
                },
            );
            return self.commit_candidate(candidate);
        };
        let mut past = self.past.clone();
        past.pop();
        let mut future = self.future.clone();
        future.push(self.project.clone());
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&previous, revision, &past, &future);
        let mut candidate = self.candidate_state(previous, revision, past, future);
        candidate.operations.insert(
            operation,
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        self.commit_candidate(candidate)
    }

    pub fn redo(&mut self, expected_revision: Option<u64>) -> Result<StateSnapshot, RuntimeError> {
        self.check_revision(expected_revision)?;
        let Some(next) = self.future.last().cloned() else {
            return Ok(self.snapshot());
        };
        let mut future = self.future.clone();
        future.pop();
        let mut past = self.past.clone();
        past.push(self.project.clone());
        self.commit_candidate(self.candidate_state(next, self.revision + 1, past, future))
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
        let Some(next) = self.future.last().cloned() else {
            let mut candidate = self.durable_state();
            let snapshot = self.snapshot_for_parts_with_proposal(
                &candidate.project,
                candidate.revision,
                &candidate.past,
                &candidate.future,
                &candidate.proposal,
            );
            candidate.operations.insert(
                operation,
                OperationRecord {
                    fingerprint,
                    snapshot: snapshot.clone(),
                },
            );
            return self.commit_candidate(candidate);
        };
        let mut future = self.future.clone();
        future.pop();
        let mut past = self.past.clone();
        past.push(self.project.clone());
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &future);
        let mut candidate = self.candidate_state(next, revision, past, future);
        candidate.operations.insert(
            operation,
            OperationRecord {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        self.commit_candidate(candidate)
    }

    pub fn start_render(
        &mut self,
        quality: &str,
    ) -> Result<(RenderJob, Value, PathBuf), RuntimeError> {
        if !matches!(quality, "preview" | "full") {
            return Err(RuntimeError::new(
                "INVALID_RENDER_QUALITY",
                "quality must be preview or full",
            ));
        }
        let id = format!("render-{}", unique_id());
        let output = self
            .project_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{id}.mp4"));
        let job = RenderJob {
            job_id: id.clone(),
            status: "queued".into(),
            output_url: None,
            output_path: Some(output.to_string_lossy().into_owned()),
            error: None,
        };
        self.jobs.insert(id, JobRecord { job: job.clone() });
        Ok((job, self.project.clone(), output))
    }

    pub fn import_media(
        &mut self,
        path: &str,
        metadata: Option<&Value>,
        overlay: Option<&Value>,
    ) -> Result<(StateSnapshot, String, String), RuntimeError> {
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
        if let Some(info) = supplied {
            if let Some(object) = info.as_object() {
                for key in [
                    "duration", "width", "height", "hasAudio", "subs", "metadata",
                ] {
                    if let Some(value) = object.get(key) {
                        source[key] = value.clone();
                    }
                }
            }
        }
        let duration = source
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|duration| *duration > 0.0);
        let clip = json!({
            "id": clip_id.clone(),
            "sourceId": source_id,
            "timelineStart": project_duration(&self.project),
            "in": 0.0,
            "out": duration.unwrap_or(1.0)
        });
        let mut next = self.project.clone();
        next.get_mut("sources")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(source);
        let tracks = next
            .get_mut("tracks")
            .and_then(Value::as_array_mut)
            .unwrap();
        if !tracks
            .iter()
            .any(|track| track.get("type").and_then(Value::as_str) == Some("video"))
        {
            tracks.push(json!({
                "id": format!("video-{}", unique_id()),
                "type": "video",
                "clips": []
            }));
        }
        let track = tracks
            .iter_mut()
            .find(|track| track.get("type").and_then(Value::as_str) == Some("video"))
            .expect("video track was created above");
        track
            .get_mut("clips")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(clip);
        if let Some(overlay) = overlay {
            next.get_mut("overlays")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(overlay.clone());
        }
        validate_project(&next)?;
        let mut past = self.past.clone();
        past.push(self.project.clone());
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &[]);
        self.commit_candidate(self.candidate_state(next, revision, past, Vec::new()))?;
        Ok((snapshot, source_id, clip_id))
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
        let source_path = validate_media_path(path)?;
        if let Some(overlay) = overlay.as_ref() {
            validate_overlay_shape(overlay)?;
        }
        // Build the import candidate without mutating the actor.  The same
        // probe/import shape is used by direct and idempotent requests so a
        // failed projection leaves every in-memory field untouched.
        let source_id = format!("source-{}", unique_id());
        let clip_id = format!("clip-{}", unique_id());
        let probe = Self::probe(path).ok();
        let mut source = json!({"id":source_id.clone(),"path":source_path.to_string_lossy()});
        let supplied = metadata.as_ref().or(probe.as_ref());
        if let Some(info) = supplied {
            if let Some(object) = info.as_object() {
                for key in [
                    "duration", "width", "height", "hasAudio", "subs", "metadata",
                ] {
                    if let Some(value) = object.get(key) {
                        source[key] = value.clone();
                    }
                }
            }
        }
        let duration = source
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|duration| *duration > 0.0);
        let clip = json!({
            "id": clip_id.clone(),
            "sourceId": source_id,
            "timelineStart": project_duration(&self.project),
            "in": 0.0,
            "out": duration.unwrap_or(1.0)
        });
        let mut next = self.project.clone();
        next.get_mut("sources")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(source);
        let tracks = next
            .get_mut("tracks")
            .and_then(Value::as_array_mut)
            .unwrap();
        if !tracks
            .iter()
            .any(|track| track.get("type").and_then(Value::as_str) == Some("video"))
        {
            tracks.push(json!({
                "id": format!("video-{}", unique_id()),
                "type": "video",
                "clips": []
            }));
        }
        let track = tracks
            .iter_mut()
            .find(|track| track.get("type").and_then(Value::as_str) == Some("video"))
            .expect("video track was created above");
        track
            .get_mut("clips")
            .and_then(Value::as_array_mut)
            .unwrap()
            .push(clip);
        if let Some(overlay) = overlay.as_ref() {
            next.get_mut("overlays")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(overlay.clone());
        }
        validate_project(&next)?;
        let mut past = self.past.clone();
        past.push(self.project.clone());
        let revision = self.revision + 1;
        let snapshot = self.snapshot_for_parts(&next, revision, &past, &[]);
        let mut candidate = self.candidate_state(next, revision, past, Vec::new());
        candidate.imports.insert(
            operation,
            ImportRecord {
                fingerprint,
                snapshot: snapshot.clone(),
                source_id: source_id.clone(),
                clip_id: clip_id.clone(),
            },
        );
        self.commit_candidate(candidate)?;
        Ok((snapshot, source_id, clip_id))
    }

    pub fn update_job(
        &mut self,
        id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
    ) {
        if let Some(record) = self.jobs.get_mut(id) {
            record.job.status = status.into();
            record.job.error = error;
            record.job.output_url = output_url;
        }
    }

    pub fn render_job(&self, id: &str) -> Result<RenderJob, RuntimeError> {
        self.jobs
            .get(id)
            .map(|record| record.job.clone())
            .ok_or_else(|| {
                RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
            })
    }

    fn probe(path: &str) -> Result<Value, RuntimeError> {
        let binary = engine_binary()?;
        let output = Command::new(binary)
            .args(["probe", "info", path])
            .output()
            .map_err(io_error)?;
        if !output.status.success() {
            return Err(RuntimeError::new(
                "ENGINE_ERROR",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| RuntimeError::new("ENGINE_PROTOCOL_ERROR", error.to_string()))
    }

    fn execute(&mut self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        match request {
            RuntimeRequest::GetState => Ok(RuntimeResponse::State(self.snapshot())),
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
            RuntimeRequest::StartRender { quality } => {
                let (job, project, output) = self.start_render(&quality)?;
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
            } => {
                self.update_job(&job_id, &status, error, output_url);
                Ok(RuntimeResponse::Empty)
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

    fn check_revision(&self, expected: Option<u64>) -> Result<(), RuntimeError> {
        if let Some(value) = expected {
            if value != self.revision {
                return Err(RuntimeError::conflict(value, self.revision));
            }
        }
        Ok(())
    }

    fn persist_candidate(&self, candidate: &DurableState) -> Result<(), RuntimeError> {
        // Materialized files remain useful to existing tooling, but the
        // checkpoint is the authoritative commit record.  Write projections
        // first; if either fails, the previous checkpoint still wins on
        // restart and the actor has not advanced.
        self.materialize_project_value(&candidate.project)?;
        self.materialize_history_values(candidate.revision, &candidate.past, &candidate.future)?;
        let checkpoint = PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: candidate.project.clone(),
            revision: candidate.revision,
            past: candidate.past.clone(),
            future: candidate.future.clone(),
            operations: candidate.operations.clone(),
            imports: candidate.imports.clone(),
            proposal: candidate.proposal.clone(),
        };
        write_json_atomic(
            &self.project_path.with_file_name(CHECKPOINT_FILE),
            &checkpoint,
        )
    }

    fn persist_checkpoint(&self) -> Result<(), RuntimeError> {
        let checkpoint = PersistedCheckpoint {
            format_version: CHECKPOINT_VERSION,
            project: self.project.clone(),
            revision: self.revision,
            past: self.past.clone(),
            future: self.future.clone(),
            operations: self.operations.clone(),
            imports: self.imports.clone(),
            proposal: self.proposal.clone(),
        };
        write_json_atomic(
            &self.project_path.with_file_name(CHECKPOINT_FILE),
            &checkpoint,
        )
    }

    fn materialize_project(&self) -> Result<(), RuntimeError> {
        self.materialize_project_value(&self.project)
    }

    fn materialize_project_value(&self, project: &Value) -> Result<(), RuntimeError> {
        write_json_atomic(&self.project_path, project)
    }

    fn materialize_history(&self) -> Result<(), RuntimeError> {
        self.materialize_history_values(self.revision, &self.past, &self.future)
    }

    fn materialize_history_values(
        &self,
        revision: u64,
        past: &[Value],
        future: &[Value],
    ) -> Result<(), RuntimeError> {
        let history = PersistedHistory {
            revision,
            past: past.to_vec(),
            future: future.to_vec(),
        };
        write_json_atomic(&self.project_path.with_file_name(HISTORY_FILE), &history)
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

fn load_checkpoint(directory: &Path) -> Result<Option<PersistedCheckpoint>, RuntimeError> {
    let checkpoint_path = directory.join(CHECKPOINT_FILE);
    let temporary_path = directory.join(CHECKPOINT_TEMP_FILE);
    if !checkpoint_path.is_file() && temporary_path.is_file() {
        let value = read_checkpoint(&temporary_path)?;
        fs::rename(&temporary_path, &checkpoint_path).map_err(io_error)?;
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
                fs::rename(&temporary_path, &checkpoint_path).map_err(io_error)?;
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
    let checkpoint: PersistedCheckpoint = serde_json::from_str(&text)
        .map_err(|error| RuntimeError::new("INVALID_CHECKPOINT", error.to_string()))?;
    if checkpoint.format_version != CHECKPOINT_VERSION {
        return Err(RuntimeError::new(
            "CHECKPOINT_VERSION_UNSUPPORTED",
            format!(
                "checkpoint format {} is not supported",
                checkpoint.format_version
            ),
        ));
    }
    Ok(checkpoint)
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
        fs::rename(&temporary_path, path).map_err(io_error)?;
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

fn empty_project() -> Value {
    json!({"version":2,"name":"Untitled project","fps":30,"sources":[],"tracks":[{"id":"video-1","type":"video","clips":[]},{"id":"audio-1","type":"audio","clips":[]}],"overlays":[],"gaps":[],"transitions":[],"effects":[],"markers":[]})
}

fn project_duration(project: &Value) -> f64 {
    let mut duration = project
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0);
    if let Some(tracks) = project.get("tracks").and_then(Value::as_array) {
        for track in tracks {
            if let Some(clips) = track.get("clips").and_then(Value::as_array) {
                for clip in clips {
                    let start = clip
                        .get("timelineStart")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or(0.0);
                    let clip_duration = clip
                        .get("duration")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value > 0.0)
                        .or_else(|| {
                            let input = clip.get("in").and_then(Value::as_f64)?;
                            let output = clip.get("out").and_then(Value::as_f64)?;
                            let speed = clip
                                .get("speed")
                                .and_then(Value::as_f64)
                                .filter(|value| value.is_finite() && *value > 0.0)
                                .unwrap_or(1.0);
                            if input.is_finite()
                                && output.is_finite()
                                && input >= 0.0
                                && output > input
                            {
                                Some((output - input) / speed)
                            } else {
                                None
                            }
                        });
                    if let Some(clip_duration) = clip_duration {
                        duration = duration.max(start + clip_duration);
                    }
                }
            }
        }
    }
    if let Some(gaps) = project.get("gaps").and_then(Value::as_array) {
        for gap in gaps {
            let start = gap
                .get("start")
                .or_else(|| gap.get("timelineStart"))
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(0.0);
            let gap_duration = gap
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value > 0.0);
            if let Some(gap_duration) = gap_duration {
                duration = duration.max(start + gap_duration);
            }
        }
    }
    duration
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

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, RuntimeError> {
    value
        .as_object()
        .ok_or_else(|| RuntimeError::new("INVALID_PROJECT", format!("{label} must be an object")))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, RuntimeError> {
    value.get(key).and_then(Value::as_array).ok_or_else(|| {
        RuntimeError::new("INVALID_PROJECT", format!("project.{key} must be an array"))
    })
}

fn id<'a>(value: &'a Value, label: &str) -> Result<&'a str, RuntimeError> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RuntimeError::new(
                "INVALID_PROJECT",
                format!("{label}.id must be a non-empty string"),
            )
        })
}

/// Validate the canonical v2 envelope at the runtime boundary. Extension
/// fields remain available, but aliases and malformed references do not.
pub fn validate_project(project: &Value) -> Result<(), RuntimeError> {
    let root = object(project, "project")?;
    if root.get("version").and_then(Value::as_u64) != Some(2) {
        return Err(RuntimeError::new(
            "INVALID_PROJECT",
            "project.version must be 2",
        ));
    }
    if root
        .get("fps")
        .and_then(Value::as_f64)
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .is_none()
    {
        return Err(RuntimeError::new(
            "INVALID_PROJECT",
            "project.fps must be positive",
        ));
    }
    let sources = array(project, "sources")?;
    let tracks = array(project, "tracks")?;
    let mut source_ids = std::collections::HashSet::new();
    for source in sources {
        id(source, "source")?;
        if !source_ids.insert(id(source, "source")?.to_owned()) {
            return Err(RuntimeError::new("INVALID_PROJECT", "duplicate source id"));
        }
        if source.get("path").and_then(Value::as_str).is_none() {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                "source.path is required",
            ));
        }
        if let Some(duration) = source.get("duration").and_then(Value::as_f64) {
            if !duration.is_finite() || duration < 0.0 {
                return Err(RuntimeError::new(
                    "INVALID_PROJECT",
                    "source.duration must be nonnegative",
                ));
            }
        }
    }
    let mut track_ids = std::collections::HashSet::new();
    let mut marker_ids = std::collections::HashSet::new();
    let mut clip_ids = std::collections::HashSet::new();
    for track in tracks {
        let track_id = id(track, "track")?;
        if !track_ids.insert(track_id.to_owned()) {
            return Err(RuntimeError::new("INVALID_PROJECT", "duplicate track id"));
        }
        let track_type = track
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::new("INVALID_PROJECT", "track.type is required"))?;
        if !matches!(track_type, "video" | "audio" | "title" | "adjustment") {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                format!("unsupported track.type: {track_type}"),
            ));
        }
        let clips = track
            .get("clips")
            .and_then(Value::as_array)
            .ok_or_else(|| RuntimeError::new("INVALID_PROJECT", "track.clips must be an array"))?;
        for clip in clips {
            let clip_id = id(clip, "clip")?;
            if !clip_ids.insert(clip_id.to_owned()) {
                return Err(RuntimeError::new("INVALID_PROJECT", "duplicate clip id"));
            }
            let source_id = clip.get("sourceId").and_then(Value::as_str);
            if matches!(track_type, "video" | "audio") && source_id.is_none() {
                return Err(RuntimeError::new(
                    "INVALID_PROJECT",
                    format!("{track_type} clip requires sourceId"),
                ));
            }
            if let Some(source_id) = source_id {
                if !source_ids.contains(source_id) {
                    return Err(RuntimeError::new(
                        "INVALID_PROJECT",
                        format!("clip references unknown source: {source_id}"),
                    ));
                }
            }
            let timeline_start = clip
                .get("timelineStart")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0);
            if timeline_start.is_none() {
                return Err(RuntimeError::new(
                    "INVALID_PROJECT",
                    "clip.timelineStart must be nonnegative",
                ));
            }
            let clip_in = clip.get("in").and_then(Value::as_f64);
            let clip_out = clip.get("out").and_then(Value::as_f64);
            if clip_in.is_some() != clip_out.is_some() {
                return Err(RuntimeError::new(
                    "INVALID_PROJECT",
                    "clip source interval requires both in and out",
                ));
            }
            if let (Some(start), Some(end)) = (clip_in, clip_out) {
                if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
                    return Err(RuntimeError::new(
                        "INVALID_PROJECT",
                        "clip out must be greater than in",
                    ));
                }
                if let Some(source_id) = source_id {
                    let source_duration = sources
                        .iter()
                        .find(|source| source.get("id").and_then(Value::as_str) == Some(source_id))
                        .and_then(|source| source.get("duration"))
                        .and_then(Value::as_f64);
                    if let Some(duration) = source_duration {
                        if end > duration {
                            return Err(RuntimeError::new(
                                "INVALID_PROJECT",
                                "clip out exceeds source duration",
                            ));
                        }
                    }
                }
            }
            if let Some(duration) = clip.get("duration").and_then(Value::as_f64) {
                if !duration.is_finite() || duration <= 0.0 {
                    return Err(RuntimeError::new(
                        "INVALID_PROJECT",
                        "clip.duration must be positive",
                    ));
                }
            }
        }
    }
    for key in ["overlays", "gaps", "transitions", "effects", "markers"] {
        array(project, key)?;
    }
    for marker in array(project, "markers")? {
        let marker_id = id(marker, "marker")?;
        if !marker_ids.insert(marker_id.to_owned()) {
            return Err(RuntimeError::new("INVALID_PROJECT", "duplicate marker id"));
        }
        if marker
            .get("time")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .is_none()
        {
            return Err(RuntimeError::new(
                "INVALID_PROJECT",
                "marker.time must be nonnegative",
            ));
        }
    }
    Ok(())
}

fn apply_command(project: &mut Value, command: &Value) -> Result<(), RuntimeError> {
    let command_object = object(command, "command")
        .map_err(|_| RuntimeError::new("INVALID_COMMAND", "command must be an object"))?;
    let kind = command_object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::new("INVALID_COMMAND", "command.type is required"))?;
    match kind {
        "batch" => {
            let commands = command_object
                .get("commands")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "batch.commands must be an array")
                })?;
            if commands.is_empty() {
                return Err(RuntimeError::new(
                    "INVALID_COMMAND",
                    "batch.commands must not be empty",
                ));
            }
            for child in commands {
                apply_command(project, child)?;
            }
        }
        "add_track" => {
            let track = command_object.get("track").ok_or_else(|| {
                RuntimeError::new("INVALID_COMMAND", "add_track.track is required")
            })?;
            let tracks = project
                .get_mut("tracks")
                .and_then(Value::as_array_mut)
                .unwrap();
            let track_id = id(track, "track")?.to_owned();
            if tracks
                .iter()
                .any(|value| id(value, "track").ok() == Some(track_id.as_str()))
            {
                return Err(RuntimeError::new("INVALID_COMMAND", "track already exists"));
            }
            tracks.push(track.clone());
        }
        "remove_track" => {
            let track_id = command_object
                .get("trackId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "remove_track.trackId is required")
                })?;
            let tracks = project
                .get_mut("tracks")
                .and_then(Value::as_array_mut)
                .unwrap();
            let index = tracks
                .iter()
                .position(|value| id(value, "track").ok() == Some(track_id))
                .ok_or_else(|| {
                    RuntimeError::new("NOT_FOUND", format!("track not found: {track_id}"))
                })?;
            tracks.remove(index);
        }
        "add_clip" => {
            let track_id = command_object
                .get("trackId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "add_clip.trackId is required")
                })?;
            let clip = command_object
                .get("clip")
                .ok_or_else(|| RuntimeError::new("INVALID_COMMAND", "add_clip.clip is required"))?;
            let tracks = project
                .get_mut("tracks")
                .and_then(Value::as_array_mut)
                .unwrap();
            let track = tracks
                .iter_mut()
                .find(|value| id(value, "track").ok() == Some(track_id))
                .ok_or_else(|| {
                    RuntimeError::new("NOT_FOUND", format!("track not found: {track_id}"))
                })?;
            let clips = track
                .get_mut("clips")
                .and_then(Value::as_array_mut)
                .unwrap();
            if clips
                .iter()
                .any(|value| id(value, "clip").ok() == id(clip, "clip").ok())
            {
                return Err(RuntimeError::new("INVALID_COMMAND", "clip already exists"));
            }
            let index = command_object
                .get("index")
                .and_then(Value::as_u64)
                .map(|v| v as usize)
                .unwrap_or(clips.len());
            if index > clips.len() {
                return Err(RuntimeError::new(
                    "INVALID_COMMAND",
                    "add_clip.index is out of range",
                ));
            }
            clips.insert(index, clip.clone());
        }
        "remove_clip" => {
            let clip_id = command_object
                .get("clipId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "remove_clip.clipId is required")
                })?;
            let track_filter = command_object.get("trackId").and_then(Value::as_str);
            let (track, index) = find_clip_mut(project, clip_id, track_filter)?;
            track
                .get_mut("clips")
                .and_then(Value::as_array_mut)
                .unwrap()
                .remove(index);
        }
        "update_clip" => {
            let clip_id = command_object
                .get("clipId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "update_clip.clipId is required")
                })?;
            let patch = command_object
                .get("patch")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "update_clip.patch must be an object")
                })?;
            let (track, index) = find_clip_mut(project, clip_id, None)?;
            let clip = track
                .get_mut("clips")
                .and_then(Value::as_array_mut)
                .unwrap()
                .get_mut(index)
                .unwrap();
            let target = clip.as_object_mut().unwrap();
            for (key, value) in patch {
                if key == "id" && value.as_str() != Some(clip_id) {
                    return Err(RuntimeError::new(
                        "INVALID_COMMAND",
                        "update_clip cannot change clip id",
                    ));
                }
                target.insert(key.clone(), value.clone());
            }
        }
        "set_clip" => {
            let clip_id = command_object
                .get("clipId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "set_clip.clipId is required")
                })?;
            let clip = command_object
                .get("clip")
                .ok_or_else(|| RuntimeError::new("INVALID_COMMAND", "set_clip.clip is required"))?;
            if id(clip, "clip")? != clip_id {
                return Err(RuntimeError::new("INVALID_COMMAND", "set_clip id mismatch"));
            }
            let (track, index) = find_clip_mut(project, clip_id, None)?;
            track
                .get_mut("clips")
                .and_then(Value::as_array_mut)
                .unwrap()[index] = clip.clone();
        }
        "move_clip" => {
            let clip_id = command_object
                .get("clipId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "move_clip.clipId is required")
                })?;
            let destination = command_object
                .get("toIndex")
                .or_else(|| command_object.get("index"))
                .and_then(Value::as_u64)
                .ok_or_else(|| RuntimeError::new("INVALID_COMMAND", "move_clip requires toIndex"))?
                as usize;
            let target_track_id = command_object
                .get("toTrackId")
                .or_else(|| command_object.get("trackId"))
                .and_then(Value::as_str);
            let (source_track_id, clip) = {
                let (source_track, source_index) = find_clip_mut(project, clip_id, None)?;
                let source_track_id = id(source_track, "track")?.to_owned();
                let clip = source_track
                    .get_mut("clips")
                    .and_then(Value::as_array_mut)
                    .unwrap()
                    .remove(source_index);
                (source_track_id, clip)
            };
            let target_id = target_track_id.unwrap_or(source_track_id.as_str());
            let tracks = project
                .get_mut("tracks")
                .and_then(Value::as_array_mut)
                .unwrap();
            let target = tracks
                .iter_mut()
                .find(|value| id(value, "track").ok() == Some(target_id))
                .ok_or_else(|| {
                    RuntimeError::new("NOT_FOUND", format!("track not found: {target_id}"))
                })?;
            let clips = target
                .get_mut("clips")
                .and_then(Value::as_array_mut)
                .unwrap();
            if destination > clips.len() {
                return Err(RuntimeError::new(
                    "INVALID_COMMAND",
                    "move_clip index is out of range",
                ));
            }
            clips.insert(destination, clip);
        }
        "add_marker" | "remove_marker" | "set_metadata" => {
            apply_simple_collection_command(project, kind, command_object)?;
        }
        "add_source" => {
            let source = command_object.get("source").ok_or_else(|| {
                RuntimeError::new("INVALID_COMMAND", "add_source.source is required")
            })?;
            let source_id = id(source, "source")?.to_owned();
            let sources = project
                .get_mut("sources")
                .and_then(Value::as_array_mut)
                .unwrap();
            if sources
                .iter()
                .any(|value| id(value, "source").ok() == Some(source_id.as_str()))
            {
                return Err(RuntimeError::new(
                    "INVALID_COMMAND",
                    "source already exists",
                ));
            }
            sources.push(source.clone());
        }
        "add_overlay" => {
            let overlay = command_object.get("overlay").ok_or_else(|| {
                RuntimeError::new("INVALID_COMMAND", "add_overlay.overlay is required")
            })?;
            let overlay_id = id(overlay, "overlay")?.to_owned();
            let overlays = project
                .get_mut("overlays")
                .and_then(Value::as_array_mut)
                .unwrap();
            if overlays
                .iter()
                .any(|value| id(value, "overlay").ok() == Some(overlay_id.as_str()))
            {
                return Err(RuntimeError::new(
                    "INVALID_COMMAND",
                    "overlay already exists",
                ));
            }
            overlays.push(overlay.clone());
        }
        "remove_overlay" => {
            let overlay_id = command_object
                .get("overlayId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "remove_overlay.overlayId is required")
                })?;
            let overlays = project
                .get_mut("overlays")
                .and_then(Value::as_array_mut)
                .unwrap();
            let index = overlays
                .iter()
                .position(|overlay| id(overlay, "overlay").ok() == Some(overlay_id))
                .ok_or_else(|| {
                    RuntimeError::new("NOT_FOUND", format!("overlay not found: {overlay_id}"))
                })?;
            overlays.remove(index);
        }
        _ => {
            return Err(RuntimeError::new(
                "INVALID_COMMAND",
                format!("unsupported semantic command: {kind}"),
            ))
        }
    }
    Ok(())
}

fn find_clip_mut<'a>(
    project: &'a mut Value,
    clip_id: &str,
    track_filter: Option<&str>,
) -> Result<(&'a mut Value, usize), RuntimeError> {
    let tracks = project
        .get_mut("tracks")
        .and_then(Value::as_array_mut)
        .unwrap();
    for track in tracks {
        if track_filter.is_some() && id(track, "track").ok() != track_filter {
            continue;
        }
        if let Some(index) = track
            .get("clips")
            .and_then(Value::as_array)
            .and_then(|clips| {
                clips
                    .iter()
                    .position(|clip| id(clip, "clip").ok() == Some(clip_id))
            })
        {
            return Ok((track, index));
        }
    }
    Err(RuntimeError::new(
        "NOT_FOUND",
        format!("clip not found: {clip_id}"),
    ))
}

fn apply_simple_collection_command(
    project: &mut Value,
    kind: &str,
    command: &Map<String, Value>,
) -> Result<(), RuntimeError> {
    match kind {
        "set_metadata" => {
            let patch = command
                .get("patch")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "set_metadata.patch must be an object")
                })?;
            if !project
                .get("metadata")
                .map(Value::is_object)
                .unwrap_or(false)
            {
                project["metadata"] = json!({});
            }
            let metadata = project
                .get_mut("metadata")
                .and_then(Value::as_object_mut)
                .unwrap();
            metadata.extend(patch.clone());
        }
        "add_marker" => {
            let marker = command.get("marker").ok_or_else(|| {
                RuntimeError::new("INVALID_COMMAND", "add_marker.marker is required")
            })?;
            project
                .get_mut("markers")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(marker.clone());
        }
        "remove_marker" => {
            let marker_id = command
                .get("markerId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RuntimeError::new("INVALID_COMMAND", "remove_marker.markerId is required")
                })?;
            let markers = project
                .get_mut("markers")
                .and_then(Value::as_array_mut)
                .unwrap();
            let index = markers
                .iter()
                .position(|marker| id(marker, "marker").ok() == Some(marker_id))
                .ok_or_else(|| {
                    RuntimeError::new("NOT_FOUND", format!("marker not found: {marker_id}"))
                })?;
            markers.remove(index);
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn repository_root_from_manifest(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn engine_binary_candidates(root: &Path) -> [PathBuf; 2] {
    [
        root.join("target/release/cut-engine"),
        root.join("target/debug/cut-engine"),
    ]
}

fn engine_binary() -> Result<PathBuf, RuntimeError> {
    if let Some(path) = std::env::var_os("CUT_ENGINE_BIN") {
        return Ok(PathBuf::from(path));
    }
    let root = repository_root_from_manifest(Path::new(env!("CARGO_MANIFEST_DIR")));
    for candidate in engine_binary_candidates(&root) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(RuntimeError::new(
        "ENGINE_UNAVAILABLE",
        "cut-engine is not available; set CUT_ENGINE_BIN",
    ))
}

pub fn run_render(
    id: String,
    project: Value,
    output: PathBuf,
    draft: bool,
) -> Result<(), RuntimeError> {
    let binary = engine_binary()?;
    let mut child = Command::new(binary)
        .args(["render-v2", "--run", "--output"])
        .arg(&output)
        .args(if draft { vec!["--draft"] } else { Vec::new() })
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    if let Some(mut stdin) = child.stdin.take() {
        serde_json::to_writer(&mut stdin, &project)
            .map_err(|error| RuntimeError::new("ENGINE_PROTOCOL_ERROR", error.to_string()))?;
    }
    let status = child.wait().map_err(io_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(RuntimeError::new(
            "ENGINE_ERROR",
            format!("render job {id} failed: cut-engine exited with {status}"),
        ))
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
            PendingProposal::Pending { proposal_id, .. } => proposal_id.clone(),
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
            Some(PendingProposal::Pending { .. })
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
        let project = json!({
            "duration": 1.5,
            "tracks": [{
                "clips": [
                    {"timelineStart":2.0,"duration":4.0},
                    {"timelineStart":8.0,"in":1.0,"out":5.0,"speed":2.0}
                ]
            }],
            "gaps": [{"timelineStart":12.0,"duration":3.0}]
        });
        assert_eq!(project_duration(&project), 15.0);
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
}
