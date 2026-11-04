//! The project-scoped local transport: the endpoint a project's daemon
//! listens on, the request-scoped client, and the JSONL server.

use crate::{
    io_error, kernel, unique_id, Author, Comment, CommentPatch, EditOp, NewComment, RenderJob,
    RenderRange, RequestEnvelope, ReviewLog, RuntimeError, RuntimeHandle, RuntimeRequest,
    RuntimeResponse, StartRenderOptions, StateSnapshot,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(windows)]
use crate::named_pipe::{PipeListener, PipeStream};

/// A connection to the project's daemon.
#[cfg(unix)]
type LocalStream = UnixStream;
#[cfg(windows)]
type LocalStream = PipeStream;

/// The largest JSONL request accepted by the local IPC boundary.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Conservative limit for a Unix-domain socket path.  Linux permits 108
/// bytes and macOS permits fewer bytes on some SDKs, so leave room for the
/// terminating NUL and use the same threshold on both platforms.
pub const UNIX_SOCKET_PATH_LIMIT: usize = 100;
#[cfg(unix)]
const SOCKET_FILE: &str = ".kerfd.sock";
const CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
/// How often an idle daemon checks whether it has been orphaned.
#[cfg(any(unix, windows))]
const ORPHAN_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// The operation a client sends to hold the daemon open. It never reaches
/// the runtime: the connection is the lease.
#[cfg(any(unix, windows))]
const HOLD_OPERATION: &str = "hold";
/// How long kerfd stays after its last lease closes, so a client that
/// restarts straight away finds it still running.
const LEASE_GRACE: Duration = Duration::from_secs(5);
/// How long kerfd stays after its last request when no client leases it,
/// so a run of one-shot commands does not start it every time.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long a new kerfd waits for one on its way out to let go of the
/// project.
#[cfg(any(unix, windows))]
const STARTUP_LOCK_WAIT: Duration = Duration::from_secs(3);
/// How often a lease whose daemon has gone tries to attach again.
#[cfg(any(unix, windows))]
const LEASE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// How long a lease leaves a kerfd too old to hold before asking again.
#[cfg(any(unix, windows))]
const LEASE_UNSUPPORTED_RETRY: Duration = Duration::from_secs(30);
/// How long a held lease reads before checking whether it was dropped.
#[cfg(any(unix, windows))]
const LEASE_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How long a lease waits for kerfd to answer it.
#[cfg(any(unix, windows))]
const LEASE_ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(any(unix, windows))]
static TEST_RESPONSE_DROPPED: AtomicBool = AtomicBool::new(false);

/// Address for the project-scoped local runtime endpoint.
///
/// A Unix socket on Unix and a named pipe on Windows, each reachable only
/// by the user the daemon runs as.
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
                    .join(format!("kerfd-{}.sock", project_directory_hash(directory))),
            };
        }

        #[cfg(windows)]
        {
            // Keep the namespace stable and independent of path length.  The
            // pipe's ACL, not its name, keeps other users out.
            return Self::WindowsNamedPipe {
                name: format!(r"\\.\pipe\kerfd-{}", project_directory_hash(directory)),
            };
        }

        #[allow(unreachable_code)]
        Self::WindowsNamedPipe {
            name: format!("kerfd-{}", project_directory_hash(directory)),
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
    let directory = directory.canonicalize().map_err(io_error)?;
    #[cfg(windows)]
    let directory = without_verbatim_prefix(directory);
    Ok(directory)
}

/// `C:\project` for the `\\?\C:\project` Windows canonicalizes to. A
/// verbatim path takes `/` literally, so a project-relative `media/a.mov`
/// joined to it names nothing. Paths too long without the prefix keep it.
#[cfg(windows)]
fn without_verbatim_prefix(directory: PathBuf) -> PathBuf {
    let stripped = directory
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|rest| {
            let bytes = rest.as_bytes();
            rest.len() < 248
                && bytes.first().is_some_and(u8::is_ascii_alphabetic)
                && bytes.get(1) == Some(&b':')
        })
        .map(PathBuf::from);
    stripped.unwrap_or(directory)
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

#[cfg(unix)]
fn user_runtime_directory() -> PathBuf {
    let directory = std::env::var_os("KERF_RUNTIME_DIR")
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
                std::env::temp_dir().join(format!("kerf-runtime-{uid}"))
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

#[derive(Clone)]
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
    /// from the client lifetime; dropping this value never kills `kerfd`.  The
    /// daemon exits on its own once nothing holds it (see [`run_ipc`]); the
    /// next request starts it again.
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

    /// This client, waiting up to `timeout` for each answer instead of 10
    /// seconds, for requests that do slow work inside kerfd, such as a git
    /// merge.
    pub fn with_request_timeout(self, timeout: Duration) -> Self {
        let mut inner = Arc::unwrap_or_clone(self.inner);
        inner.request_timeout = timeout;
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The longest this client waits for an answer.
    pub fn request_timeout(&self) -> Duration {
        self.inner.request_timeout
    }

    /// Return the canonical project directory used by this client.
    pub fn directory(&self) -> &Path {
        &self.inner.directory
    }

    /// Return the derived project-scoped endpoint.
    pub fn endpoint(&self) -> &EndpointAddress {
        &self.inner.endpoint
    }

    /// Hold the project's daemon open until the returned lease is dropped.
    ///
    /// The lease never starts kerfd, so it cannot make a deleted project
    /// folder again: it attaches to a daemon a request has started, and
    /// attaches again within a second once a later request starts another.
    /// The OS closes its connection when this process ends, however it
    /// ends, so a lease cannot outlive its client.
    pub fn hold(&self) -> RuntimeLease {
        let state = Arc::new(LeaseState::default());
        #[cfg(any(unix, windows))]
        let thread = {
            let client = self.clone();
            let state = state.clone();
            // Without a thread the lease holds nothing, and kerfd falls
            // back on its idle timeout.
            thread::Builder::new()
                .name("kerf-lease".into())
                .spawn(move || client.keep_lease(&state))
                .ok()
        };
        #[cfg(not(any(unix, windows)))]
        let thread = None;
        RuntimeLease { state, thread }
    }

    /// Send one versioned request over a fresh connection.
    pub fn request(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        let retryable = request_has_stable_idempotency_key(&request);
        match self.request_attempt(&request) {
            Ok(response) => Ok(response),
            // Reading again changes nothing. A kerfd that exited as this
            // connected has gone by now, so the retry starts another; one
            // that could not be started is not tried twice.
            Err(Failed {
                connected: true,
                error,
            }) if request_is_read_only(&request)
                && matches!(error.code.as_str(), "IPC_ERROR" | "RUNTIME_UNAVAILABLE") =>
            {
                self.request_once(&request)
            }
            Err(Failed { error, .. }) if retryable && error.is_transport() => {
                // The first request may have committed before its response
                // was lost.  Re-read authoritative state before retrying so
                // the caller observes a current revision and the retry can
                // safely resolve through the durable receipt.
                self.request_once(&RuntimeRequest::GetState)
                    .map_err(|_| error.clone())?;
                self.request_once(&request)
            }
            Err(Failed { error, .. }) => Err(error),
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

    /// The canonical `.kerf` document and the revision it describes.
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

    /// Stage `script` as a proposal that keeps `source` as the script it
    /// was written as, when that is not `script` itself.
    pub fn stage_script_proposal(
        &self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        source: Option<&str>,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::StageScriptProposal {
            request_key: request_key.to_owned(),
            base_revision,
            script: script.to_owned(),
            source: source.map(str::to_owned),
            author: None,
            revises: None,
        })?)
    }

    /// Stage `script` as a proposal by `author`, beside the ones already
    /// open; with `revises`, as the next version of that open proposal.
    pub fn stage_script_proposal_by(
        &self,
        request_key: &str,
        base_revision: u64,
        script: &str,
        source: Option<&str>,
        author: &Author,
        revises: Option<&str>,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::StageScriptProposal {
            request_key: request_key.to_owned(),
            base_revision,
            script: script.to_owned(),
            source: source.map(str::to_owned),
            author: Some(author.clone()),
            revises: revises.map(str::to_owned),
        })?)
    }

    /// The review log's comments.
    pub fn comments(&self) -> Result<ReviewLog, RuntimeError> {
        expect_comments(self.request(RuntimeRequest::ListComments)?).map(|(log, _)| log)
    }

    /// Add a comment as `actor` (`ui`, `agent`, `mcp` or `system`): the
    /// comment as written, and the log after it.
    pub fn add_comment(
        &self,
        actor: &str,
        comment: NewComment,
    ) -> Result<(Comment, ReviewLog), RuntimeError> {
        let (log, comment) = expect_comments(self.request(RuntimeRequest::AddComment {
            actor: actor.to_owned(),
            comment,
        })?)?;
        let comment = comment.ok_or_else(|| unexpected_response("comment"))?;
        Ok((comment, log))
    }

    /// Change comment `id` as `patch` says: the comment after it (none once
    /// deleted), and the log.
    pub fn set_comment(
        &self,
        actor: &str,
        id: &str,
        patch: CommentPatch,
    ) -> Result<(Option<Comment>, ReviewLog), RuntimeError> {
        let (log, comment) = expect_comments(self.request(RuntimeRequest::SetComment {
            actor: actor.to_owned(),
            id: id.to_owned(),
            patch,
        })?)?;
        Ok((comment, log))
    }

    /// Load a `project.kerf` changed outside kerfd as one revision.
    pub fn import_projection_request(
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

    /// Write the authoritative project over a `project.kerf` changed
    /// outside kerfd.
    pub fn repair_projection(&self) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::RepairProjection)?)
    }

    /// Commit local edits and merge the fetched upstream branch; the state
    /// after it and what it did.
    pub fn collab_integrate(
        &self,
        actor: &str,
        key: &str,
        message: Option<&str>,
    ) -> Result<(StateSnapshot, Value), RuntimeError> {
        expect_collab(self.request(RuntimeRequest::CollabIntegrate {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            message: message.map(str::to_owned),
        })?)
    }

    /// Commit local edits and switch to `branch`, creating it when `create`.
    pub fn collab_switch(
        &self,
        actor: &str,
        key: &str,
        branch: &str,
        create: bool,
        message: Option<&str>,
    ) -> Result<(StateSnapshot, Value), RuntimeError> {
        expect_collab(self.request(RuntimeRequest::CollabSwitch {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            branch: branch.to_owned(),
            create,
            message: message.map(str::to_owned),
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

    /// Swap the file behind `media` for the one at `path`, keeping every
    /// clip's timing.
    pub fn replace_media_request(
        &self,
        actor: &str,
        key: &str,
        media: &str,
        path: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        expect_state(self.request(RuntimeRequest::ReplaceMedia {
            actor: actor.to_owned(),
            idempotency_key: key.to_owned(),
            base_revision,
            media: media.to_owned(),
            path: path.to_owned(),
        })?)
    }

    pub fn start_render(
        &self,
        quality: &str,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_range(quality, None, None, None)
    }

    /// Queue a render job of the project, or of what a pending `proposal`
    /// would make of it. The returned document is what to render.
    pub fn start_render_range(
        &self,
        quality: &str,
        range: Option<RenderRange>,
        proposal: Option<&str>,
        sequence: Option<&str>,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_with(&StartRenderOptions {
            quality: quality.to_owned(),
            range,
            proposal: proposal.map(str::to_owned),
            sequence: sequence.map(str::to_owned),
            ..StartRenderOptions::default()
        })
    }

    /// Queue a render job as `options` describe it: its quality and codec
    /// (a ProRes 422 HQ master, a review copy), where its file goes, and its
    /// name. Returns the job, the document to render, and the file to write;
    /// [`crate::run_render_job`] renders it.
    pub fn start_render_with(
        &self,
        options: &StartRenderOptions,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        let (start, duration) = options
            .range
            .as_ref()
            .map(|range| (Some(range.start), Some(range.duration)))
            .unwrap_or((None, None));
        match self.request(RuntimeRequest::StartRender {
            quality: options.quality.clone(),
            start,
            duration,
            proposal_id: options.proposal.clone(),
            sequence: options.sequence.clone(),
            codec: options.codec.clone(),
            output: options.output.clone(),
            name: options.name.clone(),
        })? {
            RuntimeResponse::RenderStarted {
                job,
                document,
                output,
            } => Ok((job, document, output)),
            _ => Err(unexpected_response("render-start")),
        }
    }

    /// Every render job kept: queued, running, and the newest finished,
    /// oldest first.
    pub fn list_renders(&self) -> Result<Vec<RenderJob>, RuntimeError> {
        match self.request(RuntimeRequest::ListRenders)? {
            RuntimeResponse::RenderJobs(jobs) => Ok(jobs),
            _ => Err(unexpected_response("render-jobs")),
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

    fn request_once(&self, request: &RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.request_attempt(request).map_err(|failed| failed.error)
    }

    #[cfg(any(unix, windows))]
    fn request_attempt(&self, request: &RuntimeRequest) -> Result<RuntimeResponse, Failed> {
        let envelope = RequestEnvelope::new(format!("request-{}", unique_id()), request.clone());
        let mut payload = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))
            .map_err(Failed::unconnected)?;
        payload.push(b'\n');
        if payload.len() > MAX_REQUEST_BYTES {
            return Err(Failed::unconnected(RuntimeError::new(
                "REQUEST_TOO_LARGE",
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            )));
        }
        let mut stream = self.connect_or_start().map_err(Failed::unconnected)?;
        self.exchange(&mut stream, &payload, &envelope.id)
            .map_err(|error| Failed {
                connected: true,
                error,
            })
    }

    /// Send `payload` on `stream` and read the response to request `id`.
    #[cfg(any(unix, windows))]
    fn exchange(
        &self,
        stream: &mut LocalStream,
        payload: &[u8],
        id: &str,
    ) -> Result<RuntimeResponse, RuntimeError> {
        stream
            .set_read_timeout(Some(self.inner.request_timeout))
            .map_err(ipc_error)?;
        stream
            .set_write_timeout(Some(self.inner.request_timeout))
            .map_err(ipc_error)?;
        stream.write_all(payload).map_err(ipc_error)?;
        stream.flush().map_err(ipc_error)?;
        let line = read_bounded_line(stream)?;
        let value: Value = serde_json::from_slice(&line)
            .map_err(|error| RuntimeError::new("INVALID_JSON", error.to_string()))?;
        decode_response(&value, id)
    }

    #[cfg(any(unix, windows))]
    fn connect_or_start(&self) -> Result<LocalStream, RuntimeError> {
        match self.connect_once() {
            Ok(stream) => return Ok(stream),
            Err(error) if error.code == "IPC_PERMISSION_DENIED" => return Err(error),
            Err(_) => {}
        }
        let binary = self.inner.binary.as_ref().ok_or_else(|| {
            RuntimeError::new(
                "RUNTIME_UNAVAILABLE",
                format!("no kerfd daemon is listening at {}", self.endpoint_label()),
            )
        })?;
        let mut child = self.spawn_daemon(binary)?;
        let mut respawned = false;
        let deadline = std::time::Instant::now() + self.inner.startup_timeout;
        let result = loop {
            match self.connect_once() {
                Ok(stream) => break Ok(stream),
                Err(error) if std::time::Instant::now() < deadline => {
                    // A competing client may have started the daemon, or the
                    // child may still be loading its checkpoint.
                    if error.code == "IPC_PERMISSION_DENIED" {
                        break Err(error);
                    }
                    // A kerfd on its way out can hold the project longer
                    // than the child waits for it; start one more once.
                    if !respawned && matches!(child.try_wait(), Ok(Some(_))) {
                        respawned = true;
                        child = match self.spawn_daemon(binary) {
                            Ok(next) => next,
                            Err(error) => break Err(error),
                        };
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    break Err(RuntimeError::new(
                        "RUNTIME_UNAVAILABLE",
                        format!("kerfd did not become ready: {}", error.message),
                    ));
                }
            }
        };
        // kerfd exits once it is idle, while this process may live on; reap
        // it then so it does not linger as a zombie.
        let _ = thread::Builder::new()
            .name("kerfd-reaper".into())
            .spawn(move || child.wait());
        result
    }

    #[cfg(any(unix, windows))]
    fn spawn_daemon(&self, binary: &Path) -> Result<std::process::Child, RuntimeError> {
        let mut command = Command::new(binary);
        command
            .arg("--dir")
            .arg(&self.inner.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // kerfd is a console program; started from the editor it would
            // otherwise open a console window of its own.
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        // A binary just written can stay busy for a moment: a process another
        // thread is forking holds it open for writing until its exec.
        let mut busy = 0;
        loop {
            match command.spawn() {
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && busy < 25 =>
                {
                    busy += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                result => {
                    return result.map_err(|error| {
                        RuntimeError::new(
                            "RUNTIME_UNAVAILABLE",
                            format!("failed to start kerfd at {}: {error}", binary.display()),
                        )
                    });
                }
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn request_attempt(&self, _request: &RuntimeRequest) -> Result<RuntimeResponse, Failed> {
        Err(Failed::unconnected(RuntimeError::new(
            "IPC_UNSUPPORTED",
            "no local transport on this platform",
        )))
    }

    #[cfg(any(unix, windows))]
    fn connect_once(&self) -> Result<LocalStream, RuntimeError> {
        self.connect_within(self.inner.request_timeout)
    }

    /// Connect to a running daemon, waiting up to `timeout` for a free
    /// pipe instance on Windows.
    #[cfg(unix)]
    fn connect_within(&self, _timeout: Duration) -> Result<UnixStream, RuntimeError> {
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

    #[cfg(windows)]
    fn connect_within(&self, timeout: Duration) -> Result<PipeStream, RuntimeError> {
        let EndpointAddress::WindowsNamedPipe { name } = &self.inner.endpoint else {
            return Err(RuntimeError::new(
                "IPC_UNSUPPORTED",
                "endpoint is not a named pipe",
            ));
        };
        crate::named_pipe::connect(name, timeout).map_err(|error| {
            let code = match error.kind() {
                std::io::ErrorKind::PermissionDenied => "IPC_PERMISSION_DENIED",
                _ => "IPC_CONNECT_ERROR",
            };
            RuntimeError::new(code, format!("cannot connect to {name}: {error}"))
        })
    }

    #[cfg(any(unix, windows))]
    fn endpoint_label(&self) -> String {
        match &self.inner.endpoint {
            EndpointAddress::Unix { path } => path.display().to_string(),
            EndpointAddress::WindowsNamedPipe { name } => name.clone(),
        }
    }

    /// Keep a lease on whichever daemon is running until it is dropped.
    #[cfg(any(unix, windows))]
    fn keep_lease(&self, state: &LeaseState) {
        let stop = &state.stop;
        while !stop.load(Ordering::Acquire) {
            let retry = match self.attach_lease() {
                Ok(mut stream) => {
                    state.held.store(true, Ordering::Release);
                    wait_while_held(&mut stream, stop);
                    state.held.store(false, Ordering::Release);
                    LEASE_RETRY_INTERVAL
                }
                // An older kerfd lives until its folder goes, so there is
                // nothing to hold; ask again in case a newer one replaces it.
                Err(error) if error.code == "UNKNOWN_OPERATION" => LEASE_UNSUPPORTED_RETRY,
                Err(_) => LEASE_RETRY_INTERVAL,
            };
            let deadline = Instant::now() + retry;
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(LEASE_POLL_INTERVAL.min(deadline - Instant::now()));
            }
        }
    }

    /// Open a lease on the running daemon, never starting one.
    #[cfg(any(unix, windows))]
    fn attach_lease(&self) -> Result<LocalStream, RuntimeError> {
        let id = format!("lease-{}", unique_id());
        let mut line = serde_json::to_vec(&json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "operation": HOLD_OPERATION,
        }))
        .map_err(|error| RuntimeError::new("SERIALIZE_ERROR", error.to_string()))?;
        line.push(b'\n');
        let mut stream = self.connect_within(LEASE_ANSWER_TIMEOUT)?;
        stream
            .set_read_timeout(Some(LEASE_ANSWER_TIMEOUT))
            .map_err(ipc_error)?;
        stream
            .set_write_timeout(Some(LEASE_ANSWER_TIMEOUT))
            .map_err(ipc_error)?;
        stream.write_all(&line).map_err(ipc_error)?;
        stream.flush().map_err(ipc_error)?;
        let answer = read_bounded_line(&mut stream)?;
        let value: Value = serde_json::from_slice(&answer)
            .map_err(|error| RuntimeError::new("INVALID_JSON", error.to_string()))?;
        decode_response(&value, &id)?;
        // Short reads, so a dropped lease stops promptly.
        stream
            .set_read_timeout(Some(LEASE_POLL_INTERVAL))
            .map_err(ipc_error)?;
        Ok(stream)
    }
}

/// Holds a project's daemon open while it lives; see [`RuntimeClient::hold`].
/// Dropping it lets the daemon go once nothing else needs it.
pub struct RuntimeLease {
    state: Arc<LeaseState>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Default)]
struct LeaseState {
    stop: AtomicBool,
    held: AtomicBool,
}

impl RuntimeLease {
    /// Whether a running daemon holds this lease right now.
    pub fn is_held(&self) -> bool {
        self.state.held.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for RuntimeLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeLease")
            .field("held", &self.is_held())
            .finish()
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Wait while the daemon holds the lease open, until it goes or `stop` is
/// set.
#[cfg(any(unix, windows))]
fn wait_while_held<R: Read>(stream: &mut R, stop: &AtomicBool) {
    let mut buffer = [0_u8; 64];
    while !stop.load(Ordering::Acquire) {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
    }
}

/// A request that failed, and whether it reached a daemon first.
struct Failed {
    connected: bool,
    error: RuntimeError,
}

impl Failed {
    fn unconnected(error: RuntimeError) -> Self {
        Self {
            connected: false,
            error,
        }
    }
}

fn request_is_read_only(request: &RuntimeRequest) -> bool {
    matches!(
        request,
        RuntimeRequest::GetState
            | RuntimeRequest::GetDocument { .. }
            | RuntimeRequest::GetProposalDocuments { .. }
            | RuntimeRequest::GetRender { .. }
            | RuntimeRequest::ListRenders
            | RuntimeRequest::ListComments
    )
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
        | RuntimeRequest::ReplaceMedia {
            idempotency_key, ..
        }
        | RuntimeRequest::ImportProjection {
            idempotency_key, ..
        }
        // Git converges: committing and merging again after a lost
        // response changes nothing more.
        | RuntimeRequest::CollabIntegrate {
            idempotency_key, ..
        }
        | RuntimeRequest::CollabSwitch {
            idempotency_key, ..
        }
        | RuntimeRequest::AcceptProposal {
            idempotency_key, ..
        } => !idempotency_key.trim().is_empty(),
        // Staging has a request key, but it is not backed by the durable
        // operation-receipt table. A lost response is therefore ambiguous
        // and must not be replayed automatically.
        RuntimeRequest::StageScriptProposal { .. } => false,
        // An add with its own id returns the comment it already made, and a
        // change sets fields to values, so either converges when repeated.
        RuntimeRequest::AddComment { comment, .. } => comment.id.is_some(),
        RuntimeRequest::SetComment { .. } => true,
        RuntimeRequest::RejectProposal {
            idempotency_key: Some(idempotency_key),
            ..
        } => !idempotency_key.trim().is_empty(),
        RuntimeRequest::GetState
        | RuntimeRequest::ListComments
        | RuntimeRequest::GetDocument { .. }
        | RuntimeRequest::GetProposalDocuments { .. }
        | RuntimeRequest::StartRender { .. }
        | RuntimeRequest::GetRender { .. }
        | RuntimeRequest::ListRenders
        | RuntimeRequest::UpdateJob { .. }
        | RuntimeRequest::CancelRender { .. }
        | RuntimeRequest::RepairProjection
        | RuntimeRequest::RejectProposal {
            idempotency_key: None,
            ..
        } => false,
    }
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
    if let Some(comments) = object.get("comments") {
        let log = ReviewLog {
            items: serde_json::from_value(comments.clone())
                .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?,
            review_revision: object
                .get("reviewRevision")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        };
        let comment = match object.get("comment") {
            Some(Value::Null) | None => None,
            Some(comment) => {
                Some(serde_json::from_value(comment.clone()).map_err(|error| {
                    RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string())
                })?)
            }
        };
        return Ok(RuntimeResponse::Comments { log, comment });
    }
    if let Some(state) = object.get("state") {
        let state = serde_json::from_value::<StateSnapshot>(state.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        if let Some(report) = object.get("collab") {
            return Ok(RuntimeResponse::Collab {
                state,
                report: report.clone(),
            });
        }
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
    if let Some(jobs) = object.get("jobs") {
        let jobs = serde_json::from_value::<Vec<RenderJob>>(jobs.clone())
            .map_err(|error| RuntimeError::new("RUNTIME_PROTOCOL_ERROR", error.to_string()))?;
        return Ok(RuntimeResponse::RenderJobs(jobs));
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

pub(crate) fn expect_state(response: RuntimeResponse) -> Result<StateSnapshot, RuntimeError> {
    match response {
        RuntimeResponse::State(state) => Ok(state),
        _ => Err(unexpected_response("state")),
    }
}

fn expect_collab(response: RuntimeResponse) -> Result<(StateSnapshot, Value), RuntimeError> {
    match response {
        RuntimeResponse::Collab { state, report } => Ok((state, report)),
        _ => Err(unexpected_response("collab")),
    }
}

fn expect_comments(
    response: RuntimeResponse,
) -> Result<(ReviewLog, Option<Comment>), RuntimeError> {
    match response {
        RuntimeResponse::Comments { log, comment } => Ok((log, comment)),
        _ => Err(unexpected_response("comments")),
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
        Ok(RuntimeResponse::RenderJobs(jobs)) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "jobs": jobs
        }),
        Ok(RuntimeResponse::Comments { log, comment }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "comments": log.items,
            "reviewRevision": log.review_revision,
            "comment": comment
        }),
        Ok(RuntimeResponse::Collab { state, report }) => json!({
            "protocolVersion": RequestEnvelope::PROTOCOL_VERSION,
            "id": id,
            "ok": true,
            "state": state,
            "collab": report
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

/// When an idle daemon exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcOptions {
    /// How long the daemon stays once its last lease has closed.
    pub lease_grace: Duration,
    /// How long it stays after its last request otherwise, for clients that
    /// never lease; `None` keeps it until a lease releases it.
    pub idle_timeout: Option<Duration>,
}

impl Default for IpcOptions {
    fn default() -> Self {
        Self {
            lease_grace: LEASE_GRACE,
            idle_timeout: Some(IDLE_TIMEOUT),
        }
    }
}

impl IpcOptions {
    /// The defaults, with the idle timeout taken from `KERFD_IDLE_TIMEOUT`
    /// in seconds, where `0` turns it off. A value that does not read as
    /// seconds keeps the default.
    pub fn from_env() -> Self {
        Self::with_idle_timeout(std::env::var("KERFD_IDLE_TIMEOUT").ok().as_deref())
    }

    fn with_idle_timeout(seconds: Option<&str>) -> Self {
        let mut options = Self::default();
        if let Some(seconds) = seconds.and_then(|value| value.trim().parse::<u64>().ok()) {
            options.idle_timeout = (seconds > 0).then(|| Duration::from_secs(seconds));
        }
        options
    }
}

/// Run the canonical project-scoped local IPC daemon.
///
/// One `RuntimeHandle` is created for the canonical project directory and is
/// shared by short-lived connection workers.  Each worker handles one JSONL
/// request and then closes its connection, except a `hold`, which it keeps
/// open as a lease until the client closes it.
///
/// The daemon runs while a request is in flight, a lease is open, or it
/// prepares media in the background. With none of those it exits five
/// seconds after the last lease closed, or, once a client that held no
/// lease has made a request since, `KERFD_IDLE_TIMEOUT` seconds after its
/// last request (ten minutes unless set; `0` waits for a lease to release
/// it instead). It also returns once
/// it is orphaned: its project directory is gone, or its socket was removed
/// or replaced, so no client can reach it again. A later request starts it
/// again.
pub fn run_ipc(directory: &Path) -> Result<(), RuntimeError> {
    run_ipc_with(directory, IpcOptions::from_env())
}

/// [`run_ipc`] with the idle times given rather than read from the
/// environment.
pub fn run_ipc_with(directory: &Path, options: IpcOptions) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let directory = canonical_project_directory(directory)?;
        let endpoint = EndpointAddress::for_canonical_directory(&directory);
        // However this ends, media work still running for the project is
        // stopped: its FFmpeg leads a process group of its own and would
        // outlive kerfd.  Declared before the handle so it drops after it:
        // the project lock goes first, and a kerfd that replaced this one
        // need not wait out the media work to open the project.
        let _media: StopMediaOnExit;
        let handle = open_runtime(&directory)?;
        _media = StopMediaOnExit(&directory);
        let path = endpoint.as_path().ok_or_else(|| {
            RuntimeError::new("IPC_UNSUPPORTED", "Unix daemon requires a Unix endpoint")
        })?;
        let listener = bind_unix_listener(path)?;
        // Poll with a timeout instead of blocking in `accept` so the daemon
        // notices when it is orphaned or idle.  Accepted streams would
        // inherit non-blocking mode on BSDs, so each one is reset below.
        listener.set_nonblocking(true).map_err(io_error)?;
        let socket = fs::symlink_metadata(path).map_err(io_error)?;
        let owns_socket = || {
            fs::symlink_metadata(path)
                .is_ok_and(|current| current.dev() == socket.dev() && current.ino() == socket.ino())
        };
        let activity = Arc::new(Activity::new());
        loop {
            if !directory.is_dir() || !owns_socket() {
                if owns_socket() {
                    let _ = fs::remove_file(path);
                }
                return Ok(());
            }
            let wait = match activity.idle_wait(&directory, &options) {
                Some(wait) => wait,
                // Idle long enough, unless a client is already waiting.
                None if !wait_for_connection(&listener, Duration::ZERO)? => {
                    if owns_socket() {
                        let _ = fs::remove_file(path);
                    }
                    // A client may have connected just before the socket
                    // went; serve it rather than drop it.
                    serve_pending(&listener, &handle, &activity);
                    // Let go of the project so a client that finds no
                    // socket can start another kerfd at once, then wait
                    // out any media work those requests began: ending the
                    // process would leave its FFmpeg running.
                    drop(handle);
                    while kernel::background_work(&directory).0 && directory.is_dir() {
                        thread::sleep(ORPHAN_CHECK_INTERVAL / 10);
                    }
                    return Ok(());
                }
                None => Duration::ZERO,
            };
            if !wait_for_connection(&listener, wait)? {
                continue;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(io_error)?;
                    spawn_unix_worker(stream, &handle, &activity, false).map_err(io_error)?;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                            | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                // Out of descriptors or buffers for now: the connection
                // waits, and the next look may find room for it.
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM)
                    ) =>
                {
                    thread::sleep(ORPHAN_CHECK_INTERVAL / 10);
                }
                Err(error) => return Err(io_error(error)),
            }
        }
    }

    #[cfg(windows)]
    {
        let directory = canonical_project_directory(directory)?;
        let endpoint = EndpointAddress::for_canonical_directory(&directory);
        // However this ends, media work still running for the project is
        // stopped: its FFmpeg leads a process group of its own and would
        // outlive kerfd.  Declared before the handle so it drops after it:
        // the project lock goes first, and a kerfd that replaced this one
        // need not wait out the media work to open the project.
        let _media: StopMediaOnExit;
        let handle = open_runtime(&directory)?;
        _media = StopMediaOnExit(&directory);
        let EndpointAddress::WindowsNamedPipe { name } = &endpoint else {
            return Err(RuntimeError::new(
                "IPC_UNSUPPORTED",
                "Windows daemon requires a named-pipe endpoint",
            ));
        };
        // The pipe goes away with this process, so there is no stale
        // endpoint to clear, and no other process can take its name while
        // it lives.
        let mut listener = PipeListener::bind(name).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AddrInUse {
                RuntimeError::new(
                    "RUNTIME_ALREADY_RUNNING",
                    format!("another process already listens at {name}"),
                )
            } else {
                io_error(error)
            }
        })?;
        let activity = Arc::new(Activity::new());
        loop {
            if !directory.is_dir() {
                return Ok(());
            }
            let wait = match activity.idle_wait(&directory, &options) {
                Some(wait) => wait,
                // Idle long enough, unless a client is already waiting.
                // The pipe closes with the listener, and a client that
                // connects after that starts a new kerfd. One that opens
                // the pipe in the instant between is cut off; a read that
                // fails that way is tried again (see `request`).
                None => match listener.accept(Duration::ZERO).map_err(io_error)? {
                    Some(stream) => {
                        spawn_pipe_worker(stream, &handle, &activity).map_err(io_error)?;
                        continue;
                    }
                    None => return Ok(()),
                },
            };
            let Some(stream) = listener.accept(wait).map_err(io_error)? else {
                continue;
            };
            spawn_pipe_worker(stream, &handle, &activity).map_err(io_error)?;
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (directory, options);
        Err(RuntimeError::new(
            "IPC_UNSUPPORTED",
            "no local transport on this platform",
        ))
    }
}

/// Open the project's runtime, waiting briefly for a kerfd that is exiting
/// to let go of it: a client that finds its socket gone starts this one
/// straight away.
#[cfg(any(unix, windows))]
fn open_runtime(directory: &Path) -> Result<RuntimeHandle, RuntimeError> {
    let deadline = Instant::now() + STARTUP_LOCK_WAIT;
    loop {
        match RuntimeHandle::open(directory) {
            Err(error) if error.code == "RUNTIME_ALREADY_RUNNING" && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

/// How long an exiting kerfd waits for the media work it stops.
#[cfg(any(unix, windows))]
const MEDIA_STOP_WAIT: Duration = Duration::from_secs(2);

/// Stops a project's media work when dropped.
#[cfg(any(unix, windows))]
struct StopMediaOnExit<'a>(&'a Path);

#[cfg(any(unix, windows))]
impl Drop for StopMediaOnExit<'_> {
    fn drop(&mut self) {
        kernel::stop_media(self.0, MEDIA_STOP_WAIT);
    }
}

/// What keeps a daemon running: its open connections, each a request in
/// flight or a lease, and when the last one closed.
#[cfg(any(unix, windows))]
struct Activity {
    state: Mutex<ActivityState>,
}

#[cfg(any(unix, windows))]
struct ActivityState {
    open: usize,
    leases: usize,
    last: Instant,
    /// Whether a lease released the daemon, and no client that never held
    /// one has come since.
    lease_released: bool,
}

#[cfg(any(unix, windows))]
impl Activity {
    fn new() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                open: 0,
                leases: 0,
                last: Instant::now(),
                lease_released: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ActivityState> {
        // Every update is one statement, so a panic cannot leave it torn.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Count a connection from the moment it is accepted, before its
    /// worker runs, so the daemon never looks idle in between.
    fn open(self: &Arc<Self>) -> Connection {
        let mut state = self.lock();
        state.open += 1;
        Connection {
            activity: self.clone(),
            lease: false,
            // A leasing client's own requests can close after its lease
            // when it exits; they do not make it a one-shot client.
            under_lease: state.leases > 0,
        }
    }

    /// How long to wait for a connection before looking again, or `None`
    /// once the daemon has been idle long enough to exit.
    fn idle_wait(&self, directory: &Path, options: &IpcOptions) -> Option<Duration> {
        let state = self.lock();
        if state.open > 0 {
            return Some(ORPHAN_CHECK_INTERVAL);
        }
        // Work starts inside a request, before its connection closes, so
        // looking after the count misses none.
        let (working, finished) = kernel::background_work(directory);
        if working {
            return Some(ORPHAN_CHECK_INTERVAL);
        }
        let since = finished.map_or(state.last, |finished| finished.max(state.last));
        let timeout = if state.lease_released {
            Some(options.lease_grace)
        } else {
            options.idle_timeout
        };
        let Some(deadline) = timeout.and_then(|timeout| since.checked_add(timeout)) else {
            return Some(ORPHAN_CHECK_INTERVAL);
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        (!remaining.is_zero()).then(|| remaining.min(ORPHAN_CHECK_INTERVAL))
    }
}

/// One accepted connection; it stops counting when dropped.
#[cfg(any(unix, windows))]
struct Connection {
    activity: Arc<Activity>,
    lease: bool,
    under_lease: bool,
}

#[cfg(any(unix, windows))]
impl Connection {
    /// Hold the daemon open until this connection closes.
    fn hold(&mut self) {
        self.lease = true;
        self.activity.lock().leases += 1;
    }
}

#[cfg(any(unix, windows))]
impl Drop for Connection {
    fn drop(&mut self) {
        let mut state = self.activity.lock();
        state.open = state.open.saturating_sub(1);
        state.last = Instant::now();
        if self.lease {
            state.leases = state.leases.saturating_sub(1);
            state.lease_released = true;
        } else if !self.under_lease {
            state.lease_released = false;
        }
    }
}

/// Wait up to `timeout` for a pending connection on `listener`.
#[cfg(unix)]
fn wait_for_connection(listener: &UnixListener, timeout: Duration) -> Result<bool, RuntimeError> {
    let mut descriptor = libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX);
    // The single `pollfd` is valid for this synchronous call, and the socket
    // descriptor is borrowed from `listener`.
    let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
    if ready < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(io_error(error));
    }
    Ok(ready > 0)
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
                    format!("another kerfd is listening at {}", path.display()),
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

/// Serve `stream` on a thread of its own. `closing` says the daemon is
/// exiting, so a lease is refused rather than held.
#[cfg(unix)]
fn spawn_unix_worker(
    stream: UnixStream,
    handle: &RuntimeHandle,
    activity: &Arc<Activity>,
    closing: bool,
) -> std::io::Result<thread::JoinHandle<()>> {
    let connection = activity.open();
    let handle = handle.clone();
    thread::Builder::new()
        .name("kerfd-client".into())
        .spawn(move || serve_unix_connection(stream, handle, connection, closing))
}

/// Serve the clients that connected before the socket was removed, and
/// wait for them.
#[cfg(unix)]
fn serve_pending(listener: &UnixListener, handle: &RuntimeHandle, activity: &Arc<Activity>) {
    let mut workers = Vec::new();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if let Ok(worker) = spawn_unix_worker(stream, handle, activity, true) {
                    workers.push(worker);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
}

#[cfg(unix)]
fn serve_unix_connection(
    mut stream: UnixStream,
    handle: RuntimeHandle,
    mut connection: Connection,
    closing: bool,
) {
    if let Err(error) = verify_unix_peer(&stream) {
        let _ = write_ipc_response(&mut stream, "", Err(error));
        return;
    }
    // A client writes its request as soon as it connects, and an exiting
    // daemon holds the project while it waits.
    let read_timeout = if closing {
        LEASE_ANSWER_TIMEOUT
    } else {
        SERVER_REQUEST_TIMEOUT
    };
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(SERVER_REQUEST_TIMEOUT));
    if serve_request(&mut stream, &handle, closing) {
        connection.hold();
        // A lease has no deadline: it ends when its client goes.
        let _ = stream.set_read_timeout(None);
        read_until_closed(&mut stream);
    }
}

#[cfg(windows)]
fn spawn_pipe_worker(
    stream: PipeStream,
    handle: &RuntimeHandle,
    activity: &Arc<Activity>,
) -> std::io::Result<thread::JoinHandle<()>> {
    let connection = activity.open();
    let handle = handle.clone();
    thread::Builder::new()
        .name("kerfd-client".into())
        .spawn(move || serve_pipe_connection(stream, handle, connection))
}

/// Serve one client of the pipe. Only the user kerfd runs as can open it,
/// which the pipe's ACL enforces, so there is no peer to check here.
#[cfg(windows)]
fn serve_pipe_connection(
    mut stream: PipeStream,
    handle: RuntimeHandle,
    mut connection: Connection,
) {
    let _ = stream.set_read_timeout(Some(SERVER_REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SERVER_REQUEST_TIMEOUT));
    if serve_request(&mut stream, &handle, false) {
        connection.hold();
        // A lease has no deadline: it ends when its client goes, which
        // breaks the pipe.
        let _ = stream.set_read_timeout(None);
        read_until_closed(&mut stream);
        return;
    }
    stream.wait_until_read();
}

/// Read one JSONL request from `stream` and write its response. Returns
/// whether it was a lease the caller should now hold; one sent while
/// `closing` gets no answer, and its client tries again later.
#[cfg(any(unix, windows))]
fn serve_request<S: Read + Write>(stream: &mut S, handle: &RuntimeHandle, closing: bool) -> bool {
    let line = match read_bounded_line(stream) {
        Ok(line) => line,
        Err(error) => {
            let _ = write_ipc_response(stream, "", Err(error));
            return false;
        }
    };
    if let Some(id) = hold_id(&line) {
        return !closing && write_ipc_response(stream, &id, Ok(RuntimeResponse::Empty)).is_ok();
    }
    let (id, result) = process_request_line(&line, handle);
    if should_drop_test_response(&line) {
        return false;
    }
    let _ = write_ipc_response(stream, &id, result);
    false
}

/// The id of `line` when it asks for a lease.
#[cfg(any(unix, windows))]
fn hold_id(line: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let object = value.as_object()?;
    let holds = object.get("operation").and_then(Value::as_str) == Some(HOLD_OPERATION)
        && object.get("protocolVersion").and_then(Value::as_u64)
            == Some(RequestEnvelope::PROTOCOL_VERSION);
    let id = object.get("id").and_then(Value::as_str)?;
    (holds && !id.trim().is_empty()).then(|| id.to_owned())
}

/// Read and discard until the other end closes.
#[cfg(any(unix, windows))]
fn read_until_closed<R: Read>(stream: &mut R) {
    let mut buffer = [0_u8; 64];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

#[cfg(any(unix, windows))]
fn should_drop_test_response(line: &[u8]) -> bool {
    let Some(key) = std::env::var_os("KERFD_TEST_DROP_RESPONSE_KEY") else {
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

#[cfg(any(unix, windows))]
fn write_ipc_response<W: Write>(
    stream: &mut W,
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
                "kerfd rejected a connection from a different user",
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
                "kerfd rejected a connection from a different user",
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

#[cfg(all(test, any(unix, windows)))]
mod tests {
    //! The idle lifetime, with kerfd running on a thread of the test and
    //! times short enough to wait out. Deadlines are generous; what must
    //! not happen is checked well past the time it would.

    use super::*;
    use std::time::SystemTime;

    /// Long enough for any exit that should happen to have happened.
    const EXIT_DEADLINE: Duration = Duration::from_secs(15);

    struct Daemon {
        client: RuntimeClient,
        thread: Option<thread::JoinHandle<Result<(), RuntimeError>>>,
    }

    impl Daemon {
        fn start(label: &str, options: IpcOptions) -> Self {
            let stamp = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!("kerfd-idle-{label}-{stamp}"));
            let client = RuntimeClient::new(&directory).unwrap();
            let canonical = client.directory().to_path_buf();
            let thread = Some(thread::spawn(move || run_ipc_with(&canonical, options)));
            let mut daemon = Self { client, thread };
            daemon.wait_until_ready();
            daemon
        }

        fn wait_until_ready(&mut self) {
            let deadline = Instant::now() + EXIT_DEADLINE;
            while let Err(error) = self.client.get_state() {
                assert!(Instant::now() < deadline, "kerfd did not start: {error:?}");
                if !self.running() {
                    let ended = self.thread.take().unwrap().join();
                    panic!("kerfd stopped while starting: {ended:?}");
                }
                thread::sleep(Duration::from_millis(20));
            }
        }

        fn running(&self) -> bool {
            self.thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
        }

        /// Wait for the daemon to exit on its own, and check it exited
        /// cleanly.
        fn wait_for_exit(mut self) -> RuntimeClient {
            let deadline = Instant::now() + EXIT_DEADLINE;
            while self.running() {
                assert!(Instant::now() < deadline, "kerfd did not exit when idle");
                thread::sleep(Duration::from_millis(20));
            }
            let thread = self.thread.take().unwrap();
            thread.join().unwrap().unwrap();
            self.client.clone()
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            // Removing the folder ends a daemon a failed test left running.
            if self.thread.is_some() {
                let _ = fs::remove_dir_all(self.client.directory());
            }
        }
    }

    fn options(lease_grace: Duration, idle_timeout: Option<Duration>) -> IpcOptions {
        IpcOptions {
            lease_grace,
            idle_timeout,
        }
    }

    fn wait_until_held(lease: &RuntimeLease) {
        let deadline = Instant::now() + EXIT_DEADLINE;
        while !lease.is_held() {
            assert!(Instant::now() < deadline, "the lease never attached");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn remove_project(client: &RuntimeClient) {
        let _ = fs::remove_dir_all(client.directory());
    }

    #[test]
    fn it_exits_soon_after_its_last_lease_closes() {
        // No idle timeout, so only the lease can let it go.
        let daemon = Daemon::start("lease", options(Duration::from_millis(200), None));
        let first = daemon.client.hold();
        let second = daemon.client.hold();
        wait_until_held(&first);
        wait_until_held(&second);
        drop(first);
        thread::sleep(Duration::from_secs(2));
        assert!(daemon.running(), "kerfd exited while a lease was held");
        let released = Instant::now();
        drop(second);
        let client = daemon.wait_for_exit();
        assert!(released.elapsed() >= Duration::from_millis(200));
        #[cfg(unix)]
        assert!(!client.endpoint().as_path().unwrap().exists());
        remove_project(&client);
    }

    #[test]
    fn a_held_lease_outlasts_the_idle_timeout() {
        // The idle timeout leaves the lease ample time to attach, and the
        // wait is well past it.
        let daemon = Daemon::start(
            "outlast",
            options(Duration::from_millis(200), Some(Duration::from_secs(2))),
        );
        let lease = daemon.client.hold();
        wait_until_held(&lease);
        thread::sleep(Duration::from_secs(5));
        assert!(daemon.running(), "kerfd exited while a lease was held");
        assert!(lease.is_held());
        drop(lease);
        remove_project(&daemon.wait_for_exit());
    }

    #[test]
    fn a_client_that_goes_without_a_word_releases_its_lease() {
        let daemon = Daemon::start("vanish", options(Duration::from_millis(200), None));
        // The bare connection, as a client killed mid-lease leaves it for
        // the OS to close.
        let stream = daemon.client.attach_lease().unwrap();
        thread::sleep(Duration::from_secs(2));
        assert!(daemon.running(), "kerfd exited while a lease was held");
        drop(stream);
        remove_project(&daemon.wait_for_exit());
    }

    #[test]
    fn background_work_keeps_it_running_until_the_work_ends() {
        let daemon = Daemon::start(
            "work",
            options(Duration::from_millis(200), Some(Duration::from_secs(2))),
        );
        let lease = daemon.client.hold();
        wait_until_held(&lease);
        let work = kernel::BackgroundWork::start(daemon.client.directory());
        drop(lease);
        thread::sleep(Duration::from_secs(5));
        assert!(daemon.running(), "kerfd exited under its background work");
        let finished = Instant::now();
        drop(work);
        let client = daemon.wait_for_exit();
        assert!(finished.elapsed() >= Duration::from_millis(200));
        remove_project(&client);
    }

    #[test]
    fn each_request_starts_the_idle_timeout_again() {
        let idle = Duration::from_millis(800);
        let daemon = Daemon::start("requests", options(Duration::from_millis(100), Some(idle)));
        let mut last = Instant::now();
        let busy_until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < busy_until {
            // Taken before the request, so never after kerfd's own.
            last = Instant::now();
            daemon.client.get_state().unwrap();
            thread::sleep(Duration::from_millis(200));
        }
        assert!(daemon.running());
        let client = daemon.wait_for_exit();
        assert!(last.elapsed() >= idle, "it exited before its idle timeout");
        remove_project(&client);
    }

    #[test]
    fn a_request_after_the_last_lease_waits_out_the_idle_timeout() {
        let idle = Duration::from_secs(3);
        let daemon = Daemon::start("one-shot", options(Duration::from_secs(1), Some(idle)));
        let lease = daemon.client.hold();
        wait_until_held(&lease);
        drop(lease);
        // Within the grace, but after kerfd has seen the lease close: a
        // request it accepts while the lease is open belongs to the
        // leasing client.
        thread::sleep(Duration::from_millis(200));
        let last = Instant::now();
        daemon.client.get_state().unwrap();
        let client = daemon.wait_for_exit();
        assert!(
            last.elapsed() >= idle,
            "a one-shot request got the lease grace"
        );
        remove_project(&client);
    }

    #[test]
    fn without_an_idle_timeout_only_a_lease_lets_it_go() {
        let daemon = Daemon::start("forever", options(Duration::from_millis(100), None));
        daemon.client.get_state().unwrap();
        thread::sleep(Duration::from_secs(2));
        assert!(daemon.running(), "kerfd exited with its idle timeout off");
        let lease = daemon.client.hold();
        wait_until_held(&lease);
        drop(lease);
        remove_project(&daemon.wait_for_exit());
    }

    #[cfg(unix)]
    #[test]
    fn it_leaves_a_socket_that_replaced_its_own() {
        let daemon = Daemon::start("replaced", options(Duration::from_millis(100), None));
        let path = daemon.client.endpoint().as_path().unwrap().to_path_buf();
        fs::remove_file(&path).unwrap();
        let other = UnixListener::bind(&path).unwrap();
        let client = daemon.wait_for_exit();
        assert!(path.exists(), "kerfd removed a socket it does not own");
        drop(other);
        let _ = fs::remove_file(&path);
        remove_project(&client);
    }

    #[test]
    fn the_idle_timeout_reads_as_seconds_with_zero_for_never() {
        let default = IpcOptions::default();
        assert_eq!(IpcOptions::with_idle_timeout(None), default);
        assert_eq!(
            IpcOptions::with_idle_timeout(Some(" 30 ")).idle_timeout,
            Some(Duration::from_secs(30))
        );
        assert_eq!(IpcOptions::with_idle_timeout(Some("0")).idle_timeout, None);
        assert_eq!(IpcOptions::with_idle_timeout(Some("soon")), default);
        assert_eq!(
            IpcOptions::with_idle_timeout(Some("0")).lease_grace,
            default.lease_grace
        );
    }

    /// The editor's typed ops over the socket, read from the JSON the
    /// bridge forwards: a title inserted as a generator, and a frame rate
    /// that changes only while the sequence is empty.
    #[test]
    fn kerfd_takes_generator_inserts_and_rate_changes_from_the_editor() {
        let daemon = Daemon::start("editor-ops", options(Duration::from_millis(200), None));
        let lease = daemon.client.hold();
        wait_until_held(&lease);
        let client = &daemon.client;
        let send = |key: &str, ops: serde_json::Value| {
            let ops: Vec<EditOp> = serde_json::from_value(ops).unwrap();
            let revision = client.get_state().unwrap().revision;
            client.apply_ops_request("ui", key, ops, revision)
        };
        // A new project's sequence is empty, so its rate can change.
        let state = send(
            "rate-1",
            serde_json::json!([
                {"op": "set-sequence", "rate": {"num": 24, "den": 1}},
                {"op": "add-track", "kind": "video"},
                {"op": "add-track", "kind": "video"}
            ]),
        )
        .unwrap();
        assert_eq!(state.timeline["fps"], 24.0);
        let state = send(
            "title-1",
            serde_json::json!([{"op": "insert",
                "generator": {"fx": "title", "params": {"text": "Opening"}},
                "at": 0, "frames": 48, "tracks": ["V2"]}]),
        )
        .unwrap();
        let clips = &state.timeline["tracks"][1]["clips"];
        assert_eq!(clips[0]["kind"], "generator", "{clips}");
        assert_eq!(clips[0]["fx"], "title");
        assert_eq!(clips[0]["name"], "Opening");
        assert_eq!(clips[0]["durationFrames"], 48);
        // Now it has a clip, the rate stays, and kerfd says why.
        let error = send(
            "rate-2",
            serde_json::json!([{"op": "set-sequence", "rate": {"num": 30, "den": 1}}]),
        )
        .unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT");
        assert_eq!(error.message, crate::kernel::RATE_NEEDS_EMPTY);
        let error = send(
            "title-2",
            serde_json::json!([{"op": "insert", "generator": {"fx": "sparkle"},
                "at": 0, "frames": 24, "tracks": ["V2"]}]),
        )
        .unwrap_err();
        assert_eq!(error.message, "no generator `sparkle`");
        // The name, size, and start still go through set-sequence.
        let state = send(
            "format-1",
            serde_json::json!([{"op": "set-sequence", "name": "Short", "width": 1080,
                "height": 1920, "start": "01:00:00:00"}]),
        )
        .unwrap();
        assert_eq!(state.timeline["width"], 1080);
        assert_eq!(state.timeline["start"], 3600.0);
        drop(lease);
        let client = daemon.wait_for_exit();
        let written =
            fs::read_to_string(client.directory().join(crate::store::PROJECT_FILE)).unwrap();
        assert!(written.contains("gen=title@1"), "{written}");
        assert!(written.contains("text=\"Opening\""), "{written}");
        assert!(
            written.contains("name=\"Short\" rate=24 size=1080x1920"),
            "{written}"
        );
        remove_project(&client);
    }

    #[test]
    fn a_hold_is_told_apart_from_a_request() {
        assert_eq!(
            hold_id(br#"{"protocolVersion":1,"id":"l1","operation":"hold"}"#).as_deref(),
            Some("l1")
        );
        assert_eq!(
            hold_id(br#"{"protocolVersion":1,"id":"","operation":"hold"}"#),
            None
        );
        assert_eq!(
            hold_id(br#"{"protocolVersion":2,"id":"l1","operation":"hold"}"#),
            None
        );
        assert_eq!(
            hold_id(br#"{"protocolVersion":1,"id":"r1","operation":"get_state"}"#),
            None
        );
    }
}
