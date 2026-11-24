//! MCP stdio transport and project-scoped capability router.
//!
//! Every mutation goes through kerfd. The MCP process is a stateless adapter
//! for the kernel harness (`look`, `edit`, `render`, `make`; see `harness`):
//! it reads the current document for each request and stages every edit as a
//! proposal for the user, which carries the client's name, as `initialize`
//! gave it, as its author.

// The schema of `look` is one `json!`, which expands a level for each of its options, and
// has more of them than the 128 levels a crate is given.
#![recursion_limit = "256"]

mod brand;
mod camera_file;
mod cuts;
pub mod docs;
mod eye;
pub mod harness;
pub mod inbound;
mod lined_up;
mod lint;
mod making;
mod media_facts;
mod mesh_camera;
pub mod permissions;
mod presets;
pub mod qc;
mod qc_full;
mod set_scale;
mod stands;
mod text_lint;

pub use making::{MakingEvent, MakingWatcher};
pub use presets::{keep_looks, PRESET_FOLDERS};
pub use qc::{check, CheckReport, Finding, Rect, Row, RowDef, Severity, Summary};
pub use qc_full::{check_full, check_full_with, QcLimits, FULL_ROWS};
pub use text_lint::{CAPTION_GAP_FRAMES, READING_SPEED, SHORT_CAPTION_SIXTHS, TITLE_SAFE};

use kerf_agent::{AgentTool, ToolMap};
use kerf_runtime::{Author, AuthorKind, ErrorCode, RuntimeClient, RuntimeError, RuntimeLease};
use kerf_skills::{load_skills_with_bundled, Skill};
use serde::Serialize;
use serde_json::{json, Value};
use std::any::Any;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: ToolAnnotations,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct McpErrorShape {
    pub code: McpCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Which refusal a tool answers with: what an agent decides by, where the message is what it
/// reads. An agent is told a code by its name ([`McpCode::as_str`]), so a name stays as it is
/// once given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpCode {
    Cancelled,
    CannotMake,
    CapabilityUnavailable,
    EmptySequence,
    HandlerFailed,
    InvalidArguments,
    InvalidCast,
    InvalidPreset,
    InvalidProject,
    InvalidRecipe,
    InvalidSteps,
    Making,
    MediaFailed,
    MediaUnavailable,
    NotPending,
    NoAudio,
    NoChange,
    NoSequence,
    NoVideo,
    Permissions,
    PresetIo,
    ProposalBaseGone,
    ProposalNotForwarded,
    ProposalNotStale,
    ProposalPending,
    ProposalStale,
    ProposalSuperseded,
    RenderFailed,
    ResultTooLarge,
    RevisionConflict,
    RuntimeError,
    ScriptRefused,
    UnknownFile,
    UnknownJob,
    UnknownMedia,
    UnknownSequence,
    UnknownTool,
    UnreadableFile,
    Unrenderable,
}

impl McpCode {
    // Two codes with one name would be two arms the same.
    #[deny(clippy::match_same_arms)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "CANCELLED",
            Self::CannotMake => "CANNOT_MAKE",
            Self::CapabilityUnavailable => "CAPABILITY_UNAVAILABLE",
            Self::EmptySequence => "EMPTY_SEQUENCE",
            Self::HandlerFailed => "HANDLER_FAILED",
            Self::InvalidArguments => "INVALID_ARGUMENTS",
            Self::InvalidCast => "INVALID_CAST",
            Self::InvalidPreset => "INVALID_PRESET",
            Self::InvalidProject => "INVALID_PROJECT",
            Self::InvalidRecipe => "INVALID_RECIPE",
            Self::InvalidSteps => "INVALID_STEPS",
            Self::Making => "MAKING",
            Self::MediaFailed => "MEDIA_FAILED",
            Self::MediaUnavailable => "MEDIA_UNAVAILABLE",
            Self::NotPending => "NOT_PENDING",
            Self::NoAudio => "NO_AUDIO",
            Self::NoChange => "NO_CHANGE",
            Self::NoSequence => "NO_SEQUENCE",
            Self::NoVideo => "NO_VIDEO",
            Self::Permissions => "PERMISSIONS",
            Self::PresetIo => "PRESET_IO",
            Self::ProposalBaseGone => "PROPOSAL_BASE_GONE",
            Self::ProposalNotForwarded => "PROPOSAL_NOT_FORWARDED",
            Self::ProposalNotStale => "PROPOSAL_NOT_STALE",
            Self::ProposalPending => "PROPOSAL_PENDING",
            Self::ProposalStale => "PROPOSAL_STALE",
            Self::ProposalSuperseded => "PROPOSAL_SUPERSEDED",
            Self::RenderFailed => "RENDER_FAILED",
            Self::ResultTooLarge => "RESULT_TOO_LARGE",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::RuntimeError => "RUNTIME_ERROR",
            Self::ScriptRefused => "SCRIPT_REFUSED",
            Self::UnknownFile => "UNKNOWN_FILE",
            Self::UnknownJob => "UNKNOWN_JOB",
            Self::UnknownMedia => "UNKNOWN_MEDIA",
            Self::UnknownSequence => "UNKNOWN_SEQUENCE",
            Self::UnknownTool => "UNKNOWN_TOOL",
            Self::UnreadableFile => "UNREADABLE_FILE",
            Self::Unrenderable => "UNRENDERABLE",
        }
    }
}

impl fmt::Display for McpCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for McpCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
struct McpError {
    code: McpCode,
    message: String,
    details: Option<Value>,
    /// What a refused script named that is not there, as the language and the model said it
    /// ([`kerf_model::Missing`]): for the harness to act on, and not sent.
    missing: Vec<kerf_model::Missing>,
}

impl McpError {
    fn new(code: McpCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            missing: Vec::new(),
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn shape(&self) -> McpErrorShape {
        McpErrorShape {
            code: self.code,
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl From<RuntimeError> for McpError {
    fn from(error: RuntimeError) -> Self {
        // The refusals an agent acts on keep their own code.
        let code = match error.code {
            ErrorCode::RevisionConflict => McpCode::RevisionConflict,
            _ => McpCode::RuntimeError,
        };
        Self::new(code, error.message).with_details(json!({
            "runtimeCode": error.code,
            "expectedRevision": error.expected_revision,
            "actualRevision": error.actual_revision,
        }))
    }
}

/// Project-scoped capability router.
#[derive(Clone)]
pub struct KerfMcpRouter {
    dir: PathBuf,
    runtime: RuntimeClient,
    /// Holds kerfd while any copy of a long-lived router lives; see
    /// [`KerfMcpRouter::held`].
    _lease: Option<Arc<RuntimeLease>>,
    skills: Vec<Skill>,
    namespace: String,
    /// The render jobs `render` started, shared by every copy; see
    /// [`KerfMcpRouter::finish_renders`].
    renders: Arc<Mutex<Vec<RenderWorker>>>,
    /// The makings `make` started, shared by every copy; see
    /// [`KerfMcpRouter::finish_makings`].
    makings: Arc<Mutex<making::Makings>>,
    /// Permissions given in place of the person's file, as a test gives them.
    permissions: Arc<Mutex<Option<permissions::Permissions>>>,
    /// Who the proposals and notes made through this router are by,
    /// shared by every copy: Kerf for the built-in agents, the client an
    /// [`McpServer`] serves once it has said who it is.
    author: Arc<Mutex<Author>>,
    /// Who is asked before anything here begins, shared by every copy; see
    /// [`KerfMcpRouter::draw_through`].
    drawing: Arc<Mutex<Option<DrawingGate>>>,
}

/// What a router is about to begin, for the host it runs in to let or refuse
/// ([`KerfMcpRouter::draw_through`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begins<'a> {
    /// A tool call, over when it has answered.
    Call,
    /// The render of this job, on a thread that goes on after the call that started it. The
    /// file [`kerf_runtime::render_cancel_path`] names for the job stops it.
    Render(&'a str),
    /// A making of this media, on a thread that goes on likewise. Whoever watches the
    /// router's makings ([`KerfMcpRouter::watch_makings`]) is given what stops it.
    Making(&'a str),
}

/// What a host gives for the length of what it let begin: dropped when that is over.
pub type Held = Box<dyn Any + Send>;

type DrawingGate = Arc<dyn Fn(Begins<'_>) -> Result<Held, String> + Send + Sync>;

/// A render job running on a thread of this process.
struct RenderWorker {
    job_id: String,
    thread: thread::JoinHandle<()>,
}

impl fmt::Debug for KerfMcpRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KerfMcpRouter")
            .field("dir", &self.dir)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl KerfMcpRouter {
    pub fn new(directory: impl AsRef<Path>, kerfd_binary: Option<&Path>) -> Result<Self, String> {
        let dir = directory.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let runtime =
            RuntimeClient::connect_or_spawn(&dir, kerfd_binary).map_err(runtime_error_text)?;
        let skills = load_project_skills(&dir).map_err(|error| error.to_string())?;
        // The project's own looks, for checking and previewing edits.
        kerf_model::look::install_project_looks(&dir)?;
        let namespace = std::env::var("KERF_MCP_NAMESPACE")
            .unwrap_or_default()
            .trim()
            .trim_end_matches('.')
            .to_owned();
        Ok(Self {
            _lease: None,
            dir,
            runtime,
            skills,
            namespace,
            renders: Arc::default(),
            makings: Arc::default(),
            permissions: Arc::default(),
            author: Arc::new(Mutex::new(Author::kerf())),
            drawing: Arc::default(),
        })
    }

    /// Have `gate` asked before a tool call, a render or a making begins, through this router
    /// and every copy of it: what it gives is held until that is over, and when it refuses,
    /// saying why, the call is refused as cancelled with those words. A render refused once
    /// its call had begun has a job with kerfd by then, which is cancelled; nothing else is
    /// left of what was refused.
    ///
    /// A tool draws in the process its router is in, and the renders and makings it starts
    /// go on there after it has answered. A host that will exit while they may be at it (an
    /// editor, whose person closes a project when they like) has them counted here, so that
    /// it can stop them and wait for them before it does. `kerf-mcp` and `kerf` set none:
    /// they finish what they started before they end
    /// ([`finish_renders`](Self::finish_renders), [`finish_makings`](Self::finish_makings)).
    ///
    /// The gate is asked from inside the router, for a making with the router's makings
    /// held: it must not call the router back.
    pub fn draw_through(
        &self,
        gate: impl Fn(Begins<'_>) -> Result<Held, String> + Send + Sync + 'static,
    ) {
        *self.drawing.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(gate));
    }

    /// Ask the host, where there is one to ask, whether `what` may begin: what to hold until
    /// it is over, or the refusal when it may not.
    fn begin_drawing(&self, what: Begins<'_>) -> Result<Option<Held>, McpError> {
        let gate = self
            .drawing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match gate {
            None => Ok(None),
            Some(gate) => gate(what)
                .map(Some)
                .map_err(|why| McpError::new(McpCode::Cancelled, why)),
        }
    }

    /// Who the proposals and notes made through this router are by.
    pub fn author(&self) -> Author {
        self.author
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Make the proposals and notes from now on, through this router and
    /// every copy of it, by `author`.
    pub fn set_author(&self, author: Author) {
        *self.author.lock().unwrap_or_else(PoisonError::into_inner) = author;
    }

    /// What the person lets an agent do without asking, read from their own
    /// file at each asking, so a change to it holds from the next.
    pub fn permissions(&self) -> Result<permissions::Permissions, String> {
        match &*self
            .permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            Some(given) => Ok(given.clone()),
            None => permissions::Permissions::load(),
        }
    }

    /// Have every copy of this router act on `permissions` in place of the
    /// person's file, or on the file again with `None`.
    pub fn set_permissions(&self, permissions: Option<permissions::Permissions>) {
        *self
            .permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = permissions;
    }

    /// This router, holding kerfd open for as long as it or a copy of it
    /// lives: for the MCP server and agent sessions. A router used for one
    /// command holds nothing, so a run of them shares one kerfd.
    pub fn held(mut self) -> Self {
        self._lease = Some(Arc::new(self.runtime.hold()));
        self
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    /// Wait for the render jobs `render` started, cancelling them first
    /// when `cancel`. They run on this process's threads, so a process
    /// that returns from `main` with one running kills it mid-job: its job
    /// never finishes and its ffmpeg, in a process group of its own, runs
    /// on unwatched. Call this before exiting.
    pub fn finish_renders(&self, cancel: bool) {
        let workers =
            std::mem::take(&mut *self.renders.lock().unwrap_or_else(PoisonError::into_inner));
        if cancel {
            for worker in workers.iter().filter(|w| !w.thread.is_finished()) {
                // The cancel file stops the job's ffmpeg; kerfd writes it
                // and records the job cancelled, else write it here.
                if self.runtime.cancel_render(&worker.job_id).is_err() {
                    let path =
                        kerf_runtime::render_cancel_path(self.runtime.directory(), &worker.job_id);
                    if let Err(error) = fs::write(&path, b"cancelled\n") {
                        log::warn!(
                            "render {} runs on to its end: {} was not written: {error}",
                            worker.job_id,
                            path.display()
                        );
                    }
                }
            }
        }
        for worker in workers {
            // A render thread that panicked has said so already; it is only waited for here.
            let _ = worker.thread.join();
        }
    }

    /// Run `job_id`'s render on a thread [`KerfMcpRouter::finish_renders`]
    /// waits for, dropping the ones that have finished. The thread holds `drawing`, what the
    /// host gave for the render, until it ends.
    fn spawn_render(
        &self,
        job_id: String,
        drawing: Option<Held>,
        render: impl FnOnce() + Send + 'static,
    ) {
        let mut workers = self.renders.lock().unwrap_or_else(PoisonError::into_inner);
        workers.retain(|worker| !worker.thread.is_finished());
        workers.push(RenderWorker {
            job_id,
            thread: thread::spawn(move || {
                let _drawing = drawing;
                render();
            }),
        });
    }

    /// How many render jobs this process is still running.
    pub fn running_renders(&self) -> usize {
        self.renders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|worker| !worker.thread.is_finished())
            .count()
    }

    pub fn runtime(&self) -> &RuntimeClient {
        &self.runtime
    }

    /// Return the skills available to this project-scoped adapter.
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Generate the deterministic project-local agent documentation bundle.
    pub fn generate_agent_docs(&self) -> Result<docs::GeneratedDocs, docs::DocsError> {
        docs::generate_agent_docs(self)
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        harness::TOOL_NAMES
            .iter()
            .map(|name| self.definition(name))
            .collect()
    }

    /// The fixed system prompt for an agent on this project.
    pub fn agent_system_prompt(&self) -> String {
        harness::system_prompt(&self.skills)
    }

    /// The current annotated document and proposal state, read before
    /// every model request.
    pub fn agent_context(&self) -> kerf_agent::ContextSource {
        let runtime = self.runtime.clone();
        std::sync::Arc::new(move || harness::context(&runtime))
    }

    /// The tools for the built-in agent, named without the MCP namespace.
    pub fn agent_tools(&self) -> ToolMap {
        let mut tools = ToolMap::new();
        for name in harness::TOOL_NAMES {
            let definition = self.definition(name);
            let router = self.clone();
            tools.insert(
                (*name).to_owned(),
                AgentTool::new(
                    *name,
                    definition.description,
                    definition.input_schema,
                    move |input| {
                        router
                            .call_base(name, input)
                            .map_err(|error| error.to_string())
                    },
                ),
            );
        }
        tools
    }

    /// Run a tool by the name `tools/list` presented. The document and
    /// frames come back whole: the document is the agent's context and
    /// frames travel as images.
    pub fn call(&self, presented_name: &str, arguments: Value) -> Value {
        // Looks another process added since, as an applied preset's; one
        // that does not read leaves the catalog as it was.
        let _ = kerf_model::look::refresh_project_looks(&self.dir);
        let name = if self.namespace.is_empty() {
            presented_name
        } else {
            presented_name
                .strip_prefix(&format!("{}.", self.namespace))
                .unwrap_or(presented_name)
        };
        match self.call_base(name, arguments) {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(error) => json!({"ok": false, "error": error.shape()}),
        }
    }

    fn definition(&self, base_name: &str) -> ToolDefinition {
        let name = if self.namespace.is_empty() {
            base_name.to_owned()
        } else {
            format!("{}.{}", self.namespace, base_name)
        };
        let (description, input_schema, annotations) = if base_name == "make" {
            making::metadata()
        } else {
            harness::metadata(base_name).expect(
                "callers pass only harness::TOOL_NAMES, and harness::tools::metadata has an arm \
                 for each of them but make, which is answered above",
            )
        };
        ToolDefinition {
            name,
            description: description.into(),
            input_schema,
            annotations,
        }
    }

    fn call_base(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let _drawing = self.begin_drawing(Begins::Call)?;
        match name {
            "look" => self.look(arguments),
            "edit" => self.edit(arguments),
            "render" => self.render(arguments),
            "make" => self.make(arguments),
            _ => Err(McpError::new(
                McpCode::UnknownTool,
                format!("unknown tool: {name}"),
            )),
        }
    }
}

const DOCUMENT_URI: &str = "kerf://document";
const REFERENCE_URI: &str = "kerf://reference";

/// Serve the project in `directory` to the MCP client on this process's stdin and stdout,
/// until it closes stdin. It is what the `kerf-mcp` program does, and what the editor does
/// when it is run as one.
pub fn serve_stdio(directory: &Path, kerfd: Option<&Path>) -> Result<(), String> {
    let router = KerfMcpRouter::new(directory, kerfd)?.held();
    let served = McpServer::new(router.clone())
        .serve(
            std::io::BufReader::new(std::io::stdin().lock()),
            std::io::BufWriter::new(std::io::stdout().lock()),
        )
        .map_err(|error| error.to_string());
    // The client closed stdin: nobody is left to read a render's job, so
    // stop the ones still running rather than orphan their ffmpeg.
    router.finish_renders(true);
    // The same of a making, whose model's program then takes back what it asked a server for.
    router.finish_makings(true);
    served
}

/// MCP JSON-RPC server over newline-delimited stdin/stdout.
pub struct McpServer {
    router: KerfMcpRouter,
    name: String,
    version: String,
    instructions: String,
}

impl McpServer {
    /// Serve `router` to one MCP client. Until the client names itself in
    /// `initialize`, what it does is by an unnamed MCP client.
    pub fn new(router: KerfMcpRouter) -> Self {
        router.set_author(client_author(None));
        Self {
            name: "kerf-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            instructions: format!(
                "{}\n\nRead the current document with `look` and no arguments, or the \
                 resource {DOCUMENT_URI}.",
                router.agent_system_prompt()
            ),
            router,
        }
    }

    pub fn handle(&self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id.is_none() && method.starts_with("notifications/") {
            return None;
        }
        let Some(id) = id else {
            return Some(json_rpc_error(
                Value::Null,
                -32600,
                "request id is required",
            ));
        };
        match method {
            "initialize" => {
                let client = request
                    .get("params")
                    .and_then(|params| params.get("clientInfo"))
                    .and_then(|info| info.get("name"))
                    .and_then(Value::as_str);
                self.router.set_author(client_author(client));
                Some(json_rpc_result(
                    id,
                    json!({
                        "protocolVersion": request
                            .get("params")
                            .and_then(|params| params.get("protocolVersion"))
                            .and_then(Value::as_str)
                            .unwrap_or(docs::MCP_PROTOCOL_VERSION),
                        "capabilities": {"tools": {"listChanged": false}, "resources": {}},
                        "serverInfo": {"name": self.name, "version": self.version},
                        "instructions": self.instructions,
                    }),
                ))
            }
            "ping" => Some(json_rpc_result(id, json!({}))),
            "tools/list" => Some(json_rpc_result(
                id,
                json!({"tools": self.router.list_tools()}),
            )),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let mut structured = self.router.call(name, arguments);
                let is_error = structured.get("ok").and_then(Value::as_bool) == Some(false);
                // Images travel as image content, the index of each left
                // where its data was; each comes after a line that says
                // which it is, a preview's before ahead of its after.
                let images = match structured.get_mut("result") {
                    Some(result) => kerf_agent::take_inline_images(result, "image"),
                    None => kerf_agent::take_inline_images(&mut structured, "image"),
                };
                let text = serde_json::to_string(&structured).unwrap_or_else(|_| {
                    "{\"ok\":false,\"error\":{\"code\":\"SERIALIZATION_FAILED\",\"message\":\"result could not be serialized\"}}".into()
                });
                let mut content = vec![json!({"type": "text", "text": text})];
                for image in images {
                    content.push(json!({"type": "text", "text": image.label}));
                    content.push(json!({
                        "type": "image",
                        "data": image.data,
                        "mimeType": image.media_type,
                    }));
                }
                Some(json_rpc_result(
                    id,
                    json!({
                        "content": content,
                        "structuredContent": structured,
                        "isError": is_error,
                    }),
                ))
            }
            "resources/list" => Some(json_rpc_result(
                id,
                json!({"resources": [
                    {"uri": DOCUMENT_URI, "name": "project.kerf", "mimeType": "text/plain",
                     "description": "The current project document, annotated with each clip's end and source out."},
                    {"uri": REFERENCE_URI, "name": "Kerf language reference", "mimeType": "text/markdown",
                     "description": "Edit script grammar and every property, effect, transition, and generator."},
                ]}),
            )),
            "resources/read" => {
                let uri = request
                    .get("params")
                    .and_then(|params| params.get("uri"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let text = match uri {
                    DOCUMENT_URI => match self.router.runtime().get_document(true) {
                        Ok((_, document)) => document,
                        Err(error) => {
                            return Some(json_rpc_error(
                                id,
                                -32603,
                                format!("{}: {}", error.code, error.message),
                            ))
                        }
                    },
                    REFERENCE_URI => kerf_lang::reference(kerf_model::Catalog::builtin()),
                    _ => {
                        return Some(json_rpc_error(
                            id,
                            -32002,
                            format!("unknown resource {uri}"),
                        ))
                    }
                };
                let mime = if uri == DOCUMENT_URI {
                    "text/plain"
                } else {
                    "text/markdown"
                };
                Some(json_rpc_result(
                    id,
                    json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}),
                ))
            }
            _ => Some(json_rpc_error(id, -32601, "method not found")),
        }
    }

    pub fn serve<R: BufRead, W: Write>(&self, input: R, mut output: W) -> io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(request) => self.handle(&request),
                Err(error) => Some(json_rpc_error(
                    Value::Null,
                    -32700,
                    format!("invalid JSON: {error}"),
                )),
            };
            if let Some(response) = response {
                serde_json::to_writer(&mut output, &response)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

/// Bundled skills, then the user's (`~/.kerf/skills`), then the project's.
pub fn load_project_skills(dir: &Path) -> io::Result<Vec<Skill>> {
    let mut directories = Vec::new();
    if let Some(folder) = kerf_paths::user_dir() {
        directories.push(folder.join("skills"));
    }
    directories.push(dir.join(".kerf").join("skills"));
    load_skills_with_bundled(&directories)
}

/// What an MCP client's proposals and notes are by: `{kind: mcp, name,
/// id, role: MCP}`. `name` is the client's own `clientInfo.name`, kept as
/// `id`; the agents the editor shows by name (Claude Code, Cursor, Codex,
/// Claude) read as they spell themselves, any other client as it named
/// itself, and one that gave no name as "MCP client".
pub fn client_author(name: Option<&str>) -> Author {
    let given = name
        .and_then(|name| name.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|name| bounded_chars(name, MAX_CLIENT_NAME_CHARS));
    let key: String = given
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let known = KNOWN_CLIENTS
        .iter()
        .find(|(prefix, _)| key.starts_with(prefix))
        .map(|(_, name)| (*name).to_owned());
    let mut author = Author::new(
        AuthorKind::Mcp,
        known
            .or_else(|| given.clone())
            .unwrap_or_else(|| "MCP client".to_owned()),
    );
    author.id = given;
    author.role = Some("MCP".to_owned());
    author
}

/// Clients by the start of their `clientInfo.name`, letters and digits
/// only, lowercased: `claude-code`, `cursor-vscode`, `codex-mcp-client`,
/// `claude-ai` (the desktop app).
const KNOWN_CLIENTS: &[(&str, &str)] = &[
    ("claudecode", "Claude Code"),
    ("claudeai", "Claude"),
    ("cursor", "Cursor"),
    ("codex", "Codex"),
];
const MAX_CLIENT_NAME_CHARS: usize = 64;

/// `text`, cut to at most `max` characters.
fn bounded_chars(text: &str, max: usize) -> String {
    text.chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn runtime_error_text(error: RuntimeError) -> String {
    format!("{}: {}", error.code, error.message)
}

fn engine_error(error: kerf_engine::EngineError) -> McpError {
    match error {
        kerf_engine::EngineError::CapabilityUnavailable {
            capability,
            message,
        } => McpError::new(McpCode::CapabilityUnavailable, message)
            .with_details(json!({"capability": capability})),
        kerf_engine::EngineError::Cancelled => {
            McpError::new(McpCode::Cancelled, "operation cancelled")
        }
        error => McpError::new(McpCode::HandlerFailed, error.to_string()),
    }
}

fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn json_rpc_error(id: Value, code: i32, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_conformance_fixture_lists_the_harness_tools() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/protocol/conformance.json"
        ))
        .expect("protocol fixture");
        assert_eq!(
            fixture["mcp"]["protocol_version"],
            docs::MCP_PROTOCOL_VERSION
        );
        let expected = fixture["mcp"]["tools"]
            .as_array()
            .expect("tool names")
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .expect("string tool names");
        // The shared fixture lists the tools every client relies on, in
        // order.
        assert_eq!(harness::TOOL_NAMES.to_vec(), expected);
    }
}
