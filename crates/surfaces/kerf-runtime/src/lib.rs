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
//!
//! What a client asks and is answered with (requests, snapshots, typed edits, errors) is
//! kerf-protocol's, given out again from here; this crate is what carries it out.

pub mod author;
pub mod crash;
mod ipc;
mod kernel;
pub mod logging;
#[cfg(windows)]
mod named_pipe;
mod render;
mod runtime;
mod store;
mod timeline;

pub use ipc::{
    response_value, run_ipc, run_ipc_with, run_stdio, EndpointAddress, IpcOptions, RuntimeClient,
    RuntimeLease, MAX_REQUEST_BYTES, UNIX_SOCKET_PATH_LIMIT,
};
pub use kerf_protocol::{
    validate_actor, Author, AuthorKind, EditKey, EditOp, EditParam, ErrorCode, GeneratorSpec,
    Interp, MaskOutline, PendingProposal, ProjectionStatus, ProposalEntry, ProposalFreshness,
    ProposalStatus, ProposalVersion, RateSpec, RenderCodec, RenderJob, RenderQuality, RenderRange,
    RenderStatus, RequestEnvelope, RuntimeError, RuntimeRequest, RuntimeResponse,
    StartRenderOptions, StateSnapshot, TimelineView, TrimSide, ViewClip, ViewEffect, ViewFxDef,
    ViewKey, ViewMarker, ViewMask, ViewOutline, ViewOutlineKey, ViewParam, ViewParamDef,
    ViewSequence, ViewSource, ViewTake, ViewTrack, ViewTrackKind, ViewTransition, MAX_PROPOSALS,
    MAX_RENDER_RANGE_SECONDS,
};
pub use kernel::{resolve_media_paths, use_proxies, CachedFacts, RATE_NEEDS_EMPTY};
pub use render::{render_cancel_path, run_render_job};
pub use timeline::{clip_properties, fx_catalog_all};

use kerf_model::Project;
use runtime::runtime_actor;
use serde_json::Value;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

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
                ErrorCode::RuntimeUnavailable,
                "runtime actor exited during startup",
            )),
        }
    }

    pub fn request(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send(MailboxMessage { request, reply })
            .map_err(|_| {
                RuntimeError::new(ErrorCode::RuntimeUnavailable, "runtime actor is stopped")
            })?;
        response.recv().map_err(|_| {
            RuntimeError::new(
                ErrorCode::RuntimeUnavailable,
                "runtime actor dropped the response",
            )
        })?
    }
}

fn io_error(error: std::io::Error) -> RuntimeError {
    RuntimeError::new(ErrorCode::IoError, error.to_string())
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
/// so an editor can show a proposal in place before it is accepted. It has
/// no takes and no cast or styles shown.
pub fn proposal_timeline_view(document: &str, script: &str) -> Result<TimelineView, RuntimeError> {
    let project = kernel::parse(document)?;
    let edited = kernel::Edit::Script {
        script: script.to_string(),
    }
    .run(&project, &kerf_lang::NoFacts)?;
    Ok(timeline::timeline_view(&edited.project))
}

/// [`proposal_timeline_view`] as JSON, in the same shape as a snapshot's
/// `timeline`.
pub fn proposal_timeline(document: &str, script: &str) -> Result<Value, RuntimeError> {
    proposal_timeline_view(document, script).map(|view| timeline::to_json(&view))
}
