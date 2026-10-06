//! MCP stdio transport and project-scoped capability router.
//!
//! Every mutation goes through cutd. The MCP process is a stateless adapter
//! for the kernel harness (`look`, `edit`, `render`; see `harness`): it reads
//! the current document for each request and stages every edit as a
//! proposal for the user.

pub mod docs;
pub mod harness;
pub mod inbound;

use base64::Engine;
use cut_agent::{AgentTool, ToolMap};
use cut_runtime::{RenderRange, RuntimeClient, RuntimeError};
use cut_skills::{load_skills_with_bundled, Skill};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
const MAX_INLINE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapturePayload {
    start: f64,
    end: f64,
    duration: f64,
    path: String,
    media_type: String,
    frames: Vec<Value>,
}

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
        let code = if error.code == "REVISION_CONFLICT" {
            "REVISION_CONFLICT"
        } else {
            "RUNTIME_ERROR"
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
pub struct CutMcpRouter {
    dir: PathBuf,
    runtime: RuntimeClient,
    skills: Vec<Skill>,
    namespace: String,
}

impl fmt::Debug for CutMcpRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CutMcpRouter")
            .field("dir", &self.dir)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl CutMcpRouter {
    pub fn new(directory: impl AsRef<Path>, cutd_binary: Option<&Path>) -> Result<Self, String> {
        let dir = directory.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let runtime =
            RuntimeClient::connect_or_spawn(&dir, cutd_binary).map_err(runtime_error_text)?;
        let skills = load_project_skills(&dir).map_err(|error| error.to_string())?;
        let namespace = std::env::var("CUT_MCP_NAMESPACE")
            .unwrap_or_default()
            .trim()
            .trim_end_matches('.')
            .to_owned();
        Ok(Self {
            dir,
            runtime,
            skills,
            namespace,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.dir
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
    pub fn agent_context(&self) -> cut_agent::ContextSource {
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
            _ => Err(McpError::new(
                "UNKNOWN_TOOL",
                format!("unknown tool: {name}"),
            )),
        }
    }

    /// Start a render job of the project, or of a pending proposal, in the
    /// background and return it queued.
    fn render_start(
        &self,
        draft: bool,
        range: Option<RenderRange>,
        proposal: Option<&str>,
    ) -> Result<Value, McpError> {
        let (job, document, output) = self
            .runtime
            .start_render_range(if draft { "preview" } else { "full" }, range, proposal)
            .map_err(McpError::from)?;
        let runtime = self.runtime.clone();
        let worker_job = job.clone();
        let worker_output = output.clone();
        thread::spawn(move || {
            let mut last_progress = -1.0_f64;
            let _ = cut_runtime::run_render_job(
                &runtime,
                &worker_job,
                &document,
                &worker_output,
                draft,
                |progress| {
                    if progress < 1.0 && progress - last_progress < 0.01 {
                        return;
                    }
                    last_progress = progress;
                    let _ = runtime.update_job_with_progress(
                        &worker_job.job_id,
                        "running",
                        None,
                        None,
                        Some(progress),
                    );
                },
            );
        });
        Ok(json!({
            "jobId": job.job_id,
            "output": output,
            "revision": self.runtime.get_state()?.revision,
            "status": "queued",
        }))
    }
}

const DOCUMENT_URI: &str = "cut://document";
const REFERENCE_URI: &str = "cut://reference";

/// MCP JSON-RPC server over newline-delimited stdin/stdout.
pub struct McpServer {
    router: CutMcpRouter,
    name: String,
    version: String,
    instructions: String,
}

impl McpServer {
    pub fn new(router: CutMcpRouter) -> Self {
        Self {
            name: "cut-mcp".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            instructions: format!(
                "{}\n\nRead the current document with `look` and no arguments, or the \
                 resource {DOCUMENT_URI}.",
                router.agent_system_prompt()
            ),
            router,
        }
    }

    pub fn router(&self) -> &CutMcpRouter {
        &self.router
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
            "initialize" => Some(json_rpc_result(
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
            )),
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
                    {"uri": DOCUMENT_URI, "name": "project.cut", "mimeType": "text/plain",
                     "description": "The current project document, annotated with each clip's end and source out."},
                    {"uri": REFERENCE_URI, "name": "Cut language reference", "mimeType": "text/markdown",
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
                    REFERENCE_URI => cut_lang::reference(cut_model::Catalog::builtin()),
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
fn resolve_project_source_path(dir: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    }
}

/// Inline the image at `object.path` as `imageBase64`, with its `mediaType`.
fn inline_image(object: &mut Value) -> Result<(), McpError> {
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::new("HANDLER_FAILED", "artifact path is missing"))?
        .to_owned();
    let media_type = match Path::new(&path).extension().and_then(|ext| ext.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => {
            return Err(McpError::new(
                "HANDLER_FAILED",
                format!("artifact is not an image: {path}"),
            ))
        }
    };
    let metadata = fs::metadata(&path).map_err(io_error)?;
    if metadata.len() > MAX_INLINE_ARTIFACT_BYTES as u64 {
        return Err(McpError::new(
            "RESULT_TOO_LARGE",
            format!("inline artifact exceeds {MAX_INLINE_ARTIFACT_BYTES} bytes"),
        ));
    }
    let data = fs::read(path).map_err(io_error)?;
    object["imageBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(data));
    object["mediaType"] = json!(media_type);
    Ok(())
}

fn load_project_skills(dir: &Path) -> io::Result<Vec<Skill>> {
    let mut directories = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".cut").join("skills"));
    }
    directories.push(dir.join(".cut").join("skills"));
    load_skills_with_bundled(&directories)
}

fn operation_key(kind: &str) -> String {
    let sequence = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{kind}-{}-{sequence}-{timestamp}", std::process::id())
}

fn runtime_error_text(error: RuntimeError) -> String {
    format!("{}: {}", error.code, error.message)
}

fn engine_error(error: cut_engine::EngineError) -> McpError {
    match error {
        cut_engine::EngineError::CapabilityUnavailable {
            capability,
            message,
        } => McpError::new("CAPABILITY_UNAVAILABLE", message)
            .with_details(json!({"capability": capability})),
        cut_engine::EngineError::Cancelled => McpError::new("CANCELLED", "operation cancelled"),
        error => McpError::new("HANDLER_FAILED", error.to_string()),
    }
}

fn io_error(error: io::Error) -> McpError {
    McpError::new("HANDLER_FAILED", error.to_string())
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
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/protocol/conformance.json"))
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
        assert_eq!(harness::TOOL_NAMES, expected.as_slice());
    }

    #[test]
    fn stdio_server_handles_initialize_and_tools_list_without_runtime_io() {
        let root = TempDir::new().expect("temp dir");
        let router = CutMcpRouter::new(root.path(), None).expect("router");
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
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], "cut-mcp");
        assert_eq!(
            lines[1]["result"]["tools"].as_array().unwrap().len(),
            harness::TOOL_NAMES.len()
        );
    }

    #[test]
    fn unknown_and_namespaced_names_resolve_to_harness_tools() {
        let root = TempDir::new().expect("temp dir");
        let mut router = CutMcpRouter::new(root.path(), None).expect("router");
        let unknown = router.call("timeline.apply", json!({}));
        assert_eq!(unknown["error"]["code"], "UNKNOWN_TOOL");
        router.namespace = "cut".into();
        let names = router
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["cut.look", "cut.edit", "cut.render"]);
        assert_eq!(
            router.agent_tools().keys().collect::<Vec<_>>(),
            ["edit", "look", "render"]
        );
    }

    #[test]
    fn capability_unavailable_is_a_structured_mcp_error() {
        let error = engine_error(cut_engine::EngineError::capability_unavailable(
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
