//! The project-scoped local transport: the endpoint a project's daemon
//! listens on, the request-scoped client, and the JSONL server.

use crate::{
    io_error, unique_id, EditOp, RenderJob, RenderRange, RequestEnvelope, RuntimeError,
    RuntimeHandle, RuntimeRequest, RuntimeResponse, StateSnapshot,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The largest JSONL request accepted by the local IPC boundary.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Conservative limit for a Unix-domain socket path.  Linux permits 108
/// bytes and macOS permits fewer bytes on some SDKs, so leave room for the
/// terminating NUL and use the same threshold on both platforms.
pub const UNIX_SOCKET_PATH_LIMIT: usize = 100;
const SOCKET_FILE: &str = ".kerfd.sock";
const CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
/// How often an idle daemon checks whether it has been orphaned.
#[cfg(unix)]
const ORPHAN_CHECK_INTERVAL: Duration = Duration::from_secs(1);
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
                    .join(format!("kerfd-{}.sock", project_directory_hash(directory))),
            };
        }

        #[cfg(windows)]
        {
            // Keep the namespace stable and independent of path length.  The
            // named pipe transport is intentionally not enabled until its
            // server/client implementation and ACL checks land.
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
    directory.canonicalize().map_err(io_error)
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

#[cfg(not(unix))]
fn current_user_id() -> Option<u32> {
    None
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
    /// from the client lifetime; dropping this value never kills `kerfd`.  The
    /// daemon exits on its own once the project directory is deleted.
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
            Err(error) if retryable && error.is_transport() => {
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
        let (start, duration) = range
            .map(|range| (Some(range.start), Some(range.duration)))
            .unwrap_or((None, None));
        match self.request(RuntimeRequest::StartRender {
            quality: quality.to_owned(),
            start,
            duration,
            proposal_id: proposal.map(str::to_owned),
            sequence: sequence.map(str::to_owned),
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
                format!("no kerfd daemon is listening at {}", self.endpoint_label()),
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
                    format!("failed to start kerfd at {}: {error}", binary.display()),
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
                        format!("kerfd did not become ready: {}", error.message),
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

pub(crate) fn expect_state(response: RuntimeResponse) -> Result<StateSnapshot, RuntimeError> {
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
/// request and then closes its connection.  The daemon outlives its clients,
/// but returns once it is orphaned: its project directory is gone, or its
/// socket was removed or replaced, so no client can reach it again.
pub fn run_ipc(directory: &Path) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let directory = canonical_project_directory(directory)?;
        let endpoint = EndpointAddress::for_canonical_directory(&directory);
        let handle = RuntimeHandle::open(&directory)?;
        let path = endpoint.as_path().ok_or_else(|| {
            RuntimeError::new("IPC_UNSUPPORTED", "Unix daemon requires a Unix endpoint")
        })?;
        let listener = bind_unix_listener(path)?;
        // Poll with a timeout instead of blocking in `accept` so the daemon
        // notices when it is orphaned.  Accepted streams would inherit
        // non-blocking mode on BSDs, so each one is reset below.
        listener.set_nonblocking(true).map_err(io_error)?;
        let socket = fs::symlink_metadata(path).map_err(io_error)?;
        let owns_socket = || {
            fs::symlink_metadata(path)
                .is_ok_and(|current| current.dev() == socket.dev() && current.ino() == socket.ino())
        };
        loop {
            if !directory.is_dir() || !owns_socket() {
                if owns_socket() {
                    let _ = fs::remove_file(path);
                }
                return Ok(());
            }
            if !wait_for_connection(&listener, ORPHAN_CHECK_INTERVAL)? {
                continue;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(io_error)?;
                    let handle = handle.clone();
                    thread::Builder::new()
                        .name("kerfd-client".into())
                        .spawn(move || serve_unix_connection(stream, handle))
                        .map_err(io_error)?;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(io_error(error)),
            }
        }
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
