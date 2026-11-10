//! MCP stdio transport and project-scoped capability router.
//!
//! Every mutation goes through kerfd. The MCP process is a stateless adapter
//! for the kernel harness (`look`, `edit`, `render`, `note`; see `harness`):
//! it reads the current document for each request, stages every edit as a
//! proposal for the user, and posts notes to the project's review log. Both
//! carry the client's name, as `initialize` gave it, as their author.

pub mod docs;
pub mod harness;
pub mod inbound;
mod lint;
mod media_facts;
mod presets;
pub mod qc;
mod qc_full;
mod text_lint;

pub use presets::{keep_looks, PRESET_FOLDERS};
pub use qc::{check, CheckReport, Finding, Rect, Row, RowDef, Severity, Summary};
pub use qc_full::{check_full, check_full_with, QcLimits, FULL_ROWS};
pub use text_lint::{CAPTION_GAP_FRAMES, READING_SPEED, SHORT_CAPTION_SIXTHS, TITLE_SAFE};

use kerf_agent::{AgentTool, ToolMap};
use kerf_runtime::{Author, RuntimeClient, RuntimeError, RuntimeLease};
use kerf_skills::{load_skills_with_bundled, Skill};
use serde::Serialize;
use serde_json::{json, Value};
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
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Debug)]
struct McpError {
    code: String,
    message: String,
    details: Option<Value>,
}

impl McpError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn shape(&self) -> McpErrorShape {
        McpErrorShape {
            code: self.code.clone(),
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
        let code = match error.code.as_str() {
            "REVISION_CONFLICT" => "REVISION_CONFLICT",
            "BLOCKING_NOTES" => "BLOCKING_NOTES",
            _ => "RUNTIME_ERROR",
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
    /// Who the proposals and notes made through this router are by,
    /// shared by every copy: Kerf for the built-in agents, the client an
    /// [`McpServer`] serves once it has said who it is.
    author: Arc<Mutex<Author>>,
}

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
            author: Arc::new(Mutex::new(Author::kerf())),
        })
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
                    let _ = fs::write(path, b"cancelled\n");
                }
            }
        }
        for worker in workers {
            let _ = worker.thread.join();
        }
    }

    /// Run `job_id`'s render on a thread [`KerfMcpRouter::finish_renders`]
    /// waits for, dropping the ones that have finished.
    fn spawn_render(&self, job_id: String, render: impl FnOnce() + Send + 'static) {
        let mut workers = self.renders.lock().unwrap_or_else(PoisonError::into_inner);
        workers.retain(|worker| !worker.thread.is_finished());
        workers.push(RenderWorker {
            job_id,
            thread: thread::spawn(render),
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
        let (description, input_schema, annotations) =
            harness::metadata(base_name).expect("every harness tool has metadata");
        ToolDefinition {
            name,
            description: description.into(),
            input_schema,
            annotations,
        }
    }

    fn call_base(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        match name {
            "look" => self.look(arguments),
            "edit" => self.edit(arguments),
            "render" => self.render(arguments),
            "note" => self.note(arguments),
            _ => Err(McpError::new(
                "UNKNOWN_TOOL",
                format!("unknown tool: {name}"),
            )),
        }
    }
}

const DOCUMENT_URI: &str = "kerf://document";
const REFERENCE_URI: &str = "kerf://reference";

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
                let mut images = Vec::new();
                take_images(&mut structured, &mut images);
                let text = serde_json::to_string(&structured).unwrap_or_else(|_| {
                    "{\"ok\":false,\"error\":{\"code\":\"SERIALIZATION_FAILED\",\"message\":\"result could not be serialized\"}}".into()
                });
                let mut content = vec![json!({"type": "text", "text": text})];
                content.extend(images);
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

/// Move inline `imageBase64` data into MCP image content, leaving the index
/// of each image where its data was.
fn take_images(value: &mut Value, images: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(data)) = object.remove("imageBase64") {
                let mime_type = object.get("mediaType").cloned();
                object.insert("image".into(), json!(images.len()));
                images.push(json!({
                    "type": "image",
                    "data": data,
                    "mimeType": mime_type.unwrap_or_else(|| json!("image/png")),
                }));
            }
            for nested in object.values_mut() {
                take_images(nested, images);
            }
        }
        Value::Array(values) => {
            for nested in values {
                take_images(nested, images);
            }
        }
        _ => {}
    }
}
/// Bundled skills, then the user's (`~/.kerf/skills`), then the project's.
pub fn load_project_skills(dir: &Path) -> io::Result<Vec<Skill>> {
    let mut directories = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".kerf").join("skills"));
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
        "mcp",
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
        } => McpError::new("CAPABILITY_UNAVAILABLE", message)
            .with_details(json!({"capability": capability})),
        kerf_engine::EngineError::Cancelled => McpError::new("CANCELLED", "operation cancelled"),
        error => McpError::new("HANDLER_FAILED", error.to_string()),
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
    use std::io::Cursor;
    use tempfile::TempDir;

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

    #[test]
    fn stdio_server_handles_initialize_and_tools_list_without_runtime_io() {
        let root = TempDir::new().expect("temp dir");
        let router = KerfMcpRouter::new(root.path(), None).expect("router");
        let server = McpServer::new(router);
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n"
        );
        let mut output = Vec::new();
        server
            .serve(Cursor::new(input), &mut output)
            .expect("serve");
        let lines = String::from_utf8(output)
            .expect("utf8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json"))
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], "kerf-mcp");
        assert_eq!(
            lines[1]["result"]["tools"].as_array().unwrap().len(),
            harness::TOOL_NAMES.len()
        );
    }

    #[test]
    fn unknown_and_namespaced_names_resolve_to_harness_tools() {
        let root = TempDir::new().expect("temp dir");
        let mut router = KerfMcpRouter::new(root.path(), None).expect("router");
        let unknown = router.call("timeline.apply", json!({}));
        assert_eq!(unknown["error"]["code"], "UNKNOWN_TOOL");
        router.namespace = "kerf".into();
        let names = router
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["kerf.look", "kerf.edit", "kerf.render", "kerf.note"]
        );
        assert_eq!(
            router.agent_tools().keys().collect::<Vec<_>>(),
            ["edit", "look", "note", "render"]
        );
    }

    #[test]
    fn initialize_names_the_author_of_what_the_client_does() {
        let root = TempDir::new().expect("temp dir");
        let router = KerfMcpRouter::new(root.path(), None).expect("router");
        assert_eq!(router.author(), Author::kerf(), "built-in agents are Kerf");
        let server = McpServer::new(router.clone());
        assert_eq!(router.author().name, "MCP client");
        server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05",
                       "clientInfo": {"name": "claude-code", "version": "2.1.0"}}}));
        let author = router.author();
        assert_eq!(author.kind, "mcp");
        assert_eq!(author.name, "Claude Code");
        assert_eq!(author.id.as_deref(), Some("claude-code"));
    }

    #[test]
    fn capability_unavailable_is_a_structured_mcp_error() {
        let error = engine_error(kerf_engine::EngineError::capability_unavailable(
            "transcription",
            "whisper.cpp is not installed",
        ));
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert_eq!(
            error.details.as_ref().unwrap()["capability"],
            "transcription"
        );
    }
}
