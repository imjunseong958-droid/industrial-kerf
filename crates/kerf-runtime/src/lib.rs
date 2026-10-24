//! Desktop-owned editing runtime.
//!
//! This is deliberately a semantic boundary.  UI and MCP clients send project
//! commands and receive snapshots; they never mutate a local copy of the
//! timeline. `kerfd` owns one `RuntimeHandle` and serves those clients through
//! the project-scoped IPC transport (`ipc`). The actor that owns the state is
//! in `runtime`; its journal and checkpoints are in `store`.
//!
//! The canonical project is a `kerf_model::Project` stored as the `.kerf`
//! document `project.kerf`. Edits arrive as scripts or typed
//! operations (see `kernel`) and are journaled as scripts. Snapshots carry
//! the document and a timeline view of the model; proposals carry the script
//! and the document diff. Renders and proposal previews hand renderers the
//! document with media paths resolved.

mod ipc;
mod kernel;
mod render;
mod runtime;
mod store;
mod timeline;

pub use ipc::{
    response_value, run_ipc, run_stdio, EndpointAddress, RuntimeClient, MAX_REQUEST_BYTES,
    UNIX_SOCKET_PATH_LIMIT,
};
pub use kernel::{resolve_media_paths, use_proxies, EditOp, TrimSide};
pub use render::{render_cancel_path, run_render_job};
pub use timeline::fx_catalog_json;

use kerf_model::Project;
use runtime::runtime_actor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

/// `StateSnapshot::source_version` of the canonical `.kerf` document; 1 and 2
/// were earlier project formats.
const SOURCE_VERSION: u8 = 3;

/// Maximum duration of one bounded rendered-preview request.
pub const MAX_RENDER_RANGE_SECONDS: f64 = 120.0;

fn default_source_version() -> u8 {
    SOURCE_VERSION
}

fn default_projection_status() -> String {
    "current".into()
}

/// What clients see of the runtime. `document` is the canonical `.kerf`
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
    /// 3 for the canonical `.kerf` document.
    #[serde(default = "default_source_version")]
    pub source_version: u8,
    #[serde(default = "default_projection_status")]
    pub projection_status: String,
    /// What opening the project changed or left behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_warnings: Vec<String>,
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
    /// The sequence rendered; the first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
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

    /// Whether the request did not reach kerfd, or its answer was lost.
    pub fn is_transport(&self) -> bool {
        matches!(
            self.code.as_str(),
            "IPC_UNSUPPORTED"
                | "IPC_CONNECT_ERROR"
                | "IPC_PERMISSION_DENIED"
                | "IPC_TIMEOUT"
                | "IPC_ERROR"
                | "RUNTIME_UNAVAILABLE"
        )
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

/// Versioned operations accepted by the runtime mailbox.  JSON/IPC payloads
/// are parsed into this enum at the protocol boundary before they reach the
/// reducer, so the actor never branches on unvalidated field maps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum RuntimeRequest {
    GetState,
    /// The canonical `.kerf` document, optionally annotated with end times
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
        /// The sequence to render; the first when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<String>,
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

/// Transport envelope used by `kerfd`/`kerf-runtime` JSONL clients.  It keeps
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
            .name("kerf-runtime".into())
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

fn io_error(error: std::io::Error) -> RuntimeError {
    RuntimeError::new("IO_ERROR", error.to_string())
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

/// Read a `.kerf` document, such as one a render job or a snapshot carries.
pub fn parse_document(text: &str) -> Result<Project, RuntimeError> {
    kernel::parse(text)
}

/// The timeline view of `document` as a proposal's `script` would leave it,
/// in the same shape as a snapshot's `timeline`, so an editor can show a
/// proposal in place before it is accepted.
pub fn proposal_timeline(document: &str, script: &str) -> Result<Value, RuntimeError> {
    let project = kernel::parse(document)?;
    let edited = kernel::Edit::Script {
        script: script.to_string(),
    }
    .run(&project)?;
    Ok(timeline::timeline_json(&edited.project))
}
