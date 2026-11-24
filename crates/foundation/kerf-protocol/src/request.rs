//! What a client asks of kerfd and what kerfd answers, one JSON object a line.

use crate::{
    Author, EditOp, ErrorCode, RenderCodec, RenderJob, RenderQuality, RenderStatus, RuntimeError,
    StateSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

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
        /// The script as it was written, when `script` is not that: an
        /// extension runs the operations of the proposal it extends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        /// Who proposes it; Kerf when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<Author>,
        /// The open proposal this one replaces as its next version.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revises: Option<String>,
    },
    /// Bring a stale open proposal onto the project as it is, at
    /// `baseRevision`: its operations run again there, each place they name
    /// moved with the clips that held it, and the result is staged as the
    /// proposal's next version for review. Refused, leaving the proposal as
    /// it was, where a clip a line names or a place it names was on has been
    /// cut, moved or removed since (`PROPOSAL_NOT_FORWARDED`, naming the
    /// clip), or a line would change other clips than it did.
    ForwardProposal {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        #[serde(rename = "requestKey")]
        request_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
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
    /// Add the media file at `path` and cut all of it in at the end of the
    /// program. A relative path is read against the project folder, and a
    /// file inside it is kept relative to it; a Git LFS pointer is refused.
    Import {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        path: String,
        /// Add the media without cutting it into the program.
        #[serde(default, rename = "mediaOnly")]
        media_only: bool,
    },
    /// Swap the file behind media id `media` for the one at `path` (read as
    /// `Import` reads it), keeping every clip's timing. A file that is not
    /// downloaded, does not probe, or is too short or lacks a stream the
    /// clips need is refused, naming them.
    ReplaceMedia {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
        media: String,
        path: String,
    },
    StartRender {
        quality: RenderQuality,
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
        /// See [`StartRenderOptions`](crate::StartRenderOptions).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        codec: Option<RenderCodec>,
        /// The folder or file to render to (see [`StartRenderOptions`](crate::StartRenderOptions)).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// The job's name, and the file's in a folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Every render job kept: queued, running, and the newest finished,
    /// oldest first.
    ListRenders,
    GetRender {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    UpdateJob {
        #[serde(rename = "jobId")]
        job_id: String,
        status: RenderStatus,
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
    /// Commit local edits with git, merge the upstream branch fetched
    /// beforehand (`project.kerf` through the structural merge driver), and
    /// load what the merge made as one revision that clears history. The
    /// message describes the edits when absent.
    CollabIntegrate {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Commit local edits, switch to `branch` (creating it from here when
    /// `create`), and load its project.
    CollabSwitch {
        actor: String,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        branch: String,
        #[serde(default)]
        create: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
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
                ErrorCode::ProtocolVersionUnsupported,
                "protocolVersion must be 1",
            ));
        }
        if self.id.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorCode::InvalidRequest,
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
    /// The render jobs kept, oldest first.
    RenderJobs(Vec<RenderJob>),
    /// A git operation and the state after it: what was committed, merged,
    /// or switched, as kerf-collab reports it.
    Collab {
        state: StateSnapshot,
        report: Value,
    },
    Empty,
}

pub fn validate_actor(actor: &str) -> Result<(), RuntimeError> {
    if matches!(actor, "ui" | "agent" | "mcp" | "system") {
        Ok(())
    } else {
        Err(RuntimeError::new(
            ErrorCode::InvalidRequest,
            "actor must be ui, agent, mcp, or system",
        ))
    }
}
