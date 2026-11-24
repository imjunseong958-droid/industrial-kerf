//! What a client asks of kerfd and what kerfd answers, one JSON object a line.

use crate::{
    Author, EditOp, ErrorCode, RenderCodec, RenderJob, RenderQuality, RenderStatus, RuntimeError,
    StateSnapshot,
};
use serde::de::{self, value::MapDeserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
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

    /// A request read from the JSON of its line. One whose envelope reads and whose
    /// `operation` is none this kerfd has, as a newer client may ask, is refused as
    /// `UNKNOWN_OPERATION`, told by that name alone before the rest is read; anything else
    /// that does not read is `INVALID_REQUEST`. Either is said in serde's words.
    pub fn from_value(value: Value) -> Result<Self, RuntimeError> {
        let (protocol_version, id, operation) = Self::outside(value)
            .map_err(|message| RuntimeError::new(ErrorCode::InvalidRequest, message))?;
        let code = match operation.get("operation").and_then(Value::as_str) {
            Some(name) if !RuntimeRequest::is_operation(name) => ErrorCode::UnknownOperation,
            _ => ErrorCode::InvalidRequest,
        };
        let request = serde_json::from_value(Value::Object(operation))
            .map_err(|error| RuntimeError::new(code, error.to_string()))?;
        Ok(Self {
            protocol_version,
            id,
            request,
        })
    }

    /// The protocol version and the id of a request's JSON, and the rest of it, which is the
    /// operation asked for; or what is wrong with the first two.
    fn outside(mut value: Value) -> Result<(u64, String, Map<String, Value>), &'static str> {
        let object = value.as_object_mut().ok_or("request must be an object")?;
        let protocol_version = object
            .remove("protocolVersion")
            .and_then(|value| value.as_u64())
            .ok_or("protocolVersion must be an integer")?;
        let id = object
            .remove("id")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or("id must be a string")?;
        Ok((protocol_version, id, std::mem::take(object)))
    }
}

impl RuntimeRequest {
    /// Whether `name` is the `operation` of a request this build has. Serde is asked for a
    /// request with that name and nothing else, and says that it has no such operation through
    /// [`de::Error::unknown_variant`] rather than in words; what else that request lacks is no
    /// matter here.
    pub fn is_operation(name: &str) -> bool {
        let named = MapDeserializer::<_, Named>::new(std::iter::once(("operation", name)));
        !matches!(Self::deserialize(named), Err(Named::Unknown))
    }
}

/// What serde made of a request asked for by its operation's name alone
/// ([`RuntimeRequest::is_operation`]).
#[derive(Debug)]
enum Named {
    /// No operation has that name.
    Unknown,
    /// Anything else: one does, and the request lacks the rest.
    Other,
}

impl fmt::Display for Named {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unknown => "no operation has that name",
            Self::Other => "an operation has that name",
        })
    }
}

impl std::error::Error for Named {}

impl de::Error for Named {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self::Other
    }

    fn unknown_variant(_: &str, _: &'static [&'static str]) -> Self {
        Self::Unknown
    }
}

impl<'de> Deserialize<'de> for RequestEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (protocol_version, id, operation) =
            Self::outside(Value::deserialize(deserializer)?).map_err(de::Error::custom)?;
        let request =
            serde_json::from_value(Value::Object(operation)).map_err(de::Error::custom)?;
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
