//! MCP stdio transport and project-scoped capability router.
//!
//! Every mutation goes through cutd. The MCP process is a stateless adapter:
//! it reads an authoritative snapshot for each request and sends canonical
//! commands back with an idempotency key and revision.

pub mod inbound;

use base64::Engine;
use cut_agent::{AgentTool, ToolMap};
use cut_domain::{Clip, Command, Project};
use cut_runtime::{RuntimeClient, RuntimeError, StateSnapshot};
use cut_skills::{find_skill, load_skills, skill_read_tool_schema, Skill};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

pub const CORE_TOOL_NAMES: &[&str] = &[
    "timeline.query",
    "timeline.apply",
    "timeline.undo",
    "timeline.redo",
    "media.probe",
    "media.fetch",
    "media.import",
    "render.preview",
    "render.start",
    "render.result",
    "skills.read",
];

pub const FULL_TOOL_NAMES: &[&str] = &[
    "project.inspect",
    "timeline.query",
    "timeline.apply",
    "timeline.undo",
    "timeline.redo",
    "project.validate",
    "timeline.diff",
    "render.preview",
    "render.start",
    "render.status",
    "render.cancel",
    "render.result",
    "media.fetch",
    "media.import",
    "media.probe",
    "vision.track_objects",
    "skills.read",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpProfile {
    Core,
    Full,
}

impl McpProfile {
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Self::Core => CORE_TOOL_NAMES,
            Self::Full => FULL_TOOL_NAMES,
        }
    }
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
pub struct CutMcpRouter {
    dir: PathBuf,
    runtime: RuntimeClient,
    skills: Vec<Skill>,
    profile: McpProfile,
    namespace: String,
}

impl fmt::Debug for CutMcpRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CutMcpRouter")
            .field("dir", &self.dir)
            .field("profile", &self.profile)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl CutMcpRouter {
    pub fn new(
        directory: impl AsRef<Path>,
        profile: McpProfile,
        cutd_binary: Option<&Path>,
    ) -> Result<Self, String> {
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
            profile,
            namespace,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    pub fn runtime(&self) -> &RuntimeClient {
        &self.runtime
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.profile
            .names()
            .iter()
            .map(|name| self.definition(name))
            .collect()
    }

    pub fn agent_tools(&self) -> ToolMap {
        let mut tools = ToolMap::new();
        for definition in self.list_tools() {
            let router = self.clone_for_agent();
            let name = definition.name.clone();
            let agent_name = name.clone();
            let description = definition.description.clone();
            let input_schema = definition.input_schema.clone();
            tools.insert(
                name.clone(),
                AgentTool::new(name, description, input_schema, move |input| {
                    router.call_base(&agent_name, input)
                }),
            );
        }
        tools
    }

    pub fn call(&self, presented_name: &str, arguments: Value) -> Value {
        let base_name = self
            .parse_name(presented_name)
            .unwrap_or_else(|| presented_name.to_owned());
        match self.call_base(&base_name, arguments) {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(error) => json!({"ok": false, "error": error.shape()}),
        }
    }

    fn clone_for_agent(&self) -> AgentRouter {
        AgentRouter {
            dir: self.dir.clone(),
            runtime: self.runtime.clone(),
            skills: self.skills.clone(),
            profile: self.profile,
        }
    }

    fn parse_name(&self, name: &str) -> Option<String> {
        if self.namespace.is_empty() {
            return self
                .profile
                .names()
                .contains(&name)
                .then(|| name.to_owned());
        }
        name.strip_prefix(&format!("{}.", self.namespace))
            .map(str::to_owned)
            .filter(|name| self.profile.names().contains(&name.as_str()))
    }

    fn definition(&self, base_name: &str) -> ToolDefinition {
        let name = if self.namespace.is_empty() {
            base_name.to_owned()
        } else {
            format!("{}.{}", self.namespace, base_name)
        };
        let (description, input_schema, annotations) = tool_metadata(base_name);
        ToolDefinition {
            name,
            description: description.into(),
            input_schema,
            annotations,
        }
    }

    fn call_base(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        if !self.profile.names().contains(&name) {
            return Err(McpError::new(
                "UNKNOWN_TOOL",
                format!("unknown tool: {name}"),
            ));
        }
        self.call_inner(name, arguments)
    }

    fn call_inner(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        match name {
            "project.inspect" => self.project_inspect(arguments),
            "project.validate" => self.project_validate(arguments),
            "timeline.query" => self.timeline_query(arguments),
            "timeline.apply" => self.timeline_apply(arguments),
            "timeline.undo" => self.history_move(arguments, false),
            "timeline.redo" => self.history_move(arguments, true),
            "timeline.diff" => self.timeline_diff(arguments),
            "media.probe" => self.media_probe(arguments),
            "media.fetch" => self.media_fetch(arguments),
            "media.import" => self.media_import(arguments),
            "render.preview" => self.render_start(true),
            "render.start" => self.render_start(
                arguments
                    .get("draft")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            "render.status" => self.render_status(&arguments),
            "render.cancel" => self.render_cancel(&arguments),
            "render.result" => self.render_result(&arguments),
            "vision.track_objects" => Err(McpError::new(
                "CAPABILITY_UNAVAILABLE",
                "vision.track_objects requires a configured YOLO runtime",
            )),
            "skills.read" => self.skills_read(&arguments),
            _ => Err(McpError::new(
                "UNKNOWN_TOOL",
                format!("unknown tool: {name}"),
            )),
        }
    }

    fn project_inspect(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let mut result = summary(&project, snapshot.revision);
        if arguments.get("include").and_then(Value::as_str) == Some("full") {
            result["project"] = snapshot.project;
        }
        result["sourceVersion"] = json!(snapshot.source_version);
        result["normalizedVersion"] = json!(2);
        Ok(result)
    }

    fn project_validate(&self, _arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        match cut_domain::parse_project(&snapshot.project) {
            Ok(_) => Ok(json!({
                "valid": true,
                "version": snapshot.source_version,
                "normalizedVersion": 2,
                "projectionStatus": snapshot.projection_status,
            })),
            Err(error) => Ok(json!({"valid": false, "error": error.to_string()})),
        }
    }

    fn timeline_query(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let track_filter = arguments.get("trackId").and_then(Value::as_str);
        let clip_filter = arguments.get("clipId").and_then(Value::as_str);
        let tracks = project
            .tracks
            .iter()
            .filter(|track| track_filter.is_none_or(|id| id == track.id.as_str()))
            .map(|track| {
                let clips = track
                    .clips
                    .iter()
                    .filter(|clip| clip_filter.is_none_or(|id| id == clip.id.as_str()))
                    .map(|clip| {
                        json!({
                            "id": clip.id,
                            "sourceId": clip.source_id,
                            "timelineStart": clip.timeline_start,
                            "duration": clip.timeline_duration(),
                            "in": clip.source_in,
                            "out": clip.source_out,
                        })
                    })
                    .collect::<Vec<_>>();
                json!({"id": track.id, "type": track.kind.as_str(), "clips": clips})
            })
            .collect::<Vec<_>>();
        let mut result = json!({"revision": snapshot.revision, "tracks": tracks});
        if arguments.get("include").and_then(Value::as_str) == Some("full") {
            result["project"] = snapshot.project;
        }
        Ok(result)
    }

    fn timeline_apply(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let command = canonical_command(&project, &arguments)?;
        let expected = arguments
            .get("expectedRevision")
            .and_then(Value::as_u64)
            .unwrap_or(snapshot.revision);
        let key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| operation_key("mcp-apply"));
        let next = self
            .runtime
            .apply_request(
                "mcp",
                &key,
                cut_domain::serialize_command(&command)
                    .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?,
                expected,
            )
            .map_err(McpError::from)?;
        Ok(state_result(&next))
    }

    fn history_move(&self, arguments: Value, redo: bool) -> Result<Value, McpError> {
        let steps = arguments.get("steps").and_then(Value::as_u64).unwrap_or(1);
        if !(1..=100).contains(&steps) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "steps must be an integer between 1 and 100",
            ));
        }
        let mut current = self.state()?;
        for index in 0..steps {
            let revision = if index == 0 {
                arguments
                    .get("expectedRevision")
                    .and_then(Value::as_u64)
                    .unwrap_or(current.revision)
            } else {
                current.revision
            };
            let key = if index == 0 {
                arguments
                    .get("idempotencyKey")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| operation_key(if redo { "mcp-redo" } else { "mcp-undo" }))
            } else {
                operation_key(if redo { "mcp-redo" } else { "mcp-undo" })
            };
            current = if redo {
                self.runtime
                    .redo_request("mcp", &key, revision)
                    .map_err(McpError::from)?
            } else {
                self.runtime
                    .undo_request("mcp", &key, revision)
                    .map_err(McpError::from)?
            };
        }
        Ok(state_result(&current))
    }

    fn timeline_diff(&self, arguments: Value) -> Result<Value, McpError> {
        let current = self.state()?;
        let from = arguments
            .get("from")
            .cloned()
            .unwrap_or_else(|| current.project.clone());
        let to = arguments
            .get("to")
            .cloned()
            .unwrap_or_else(|| current.project.clone());
        let changes = if from == to {
            Vec::new()
        } else {
            vec![json!({"path": "$", "kind": "replace", "before": from, "after": to})]
        };
        Ok(json!({
            "equal": changes.is_empty(),
            "changes": changes,
            "revision": current.revision,
        }))
    }

    fn media_probe(&self, arguments: Value) -> Result<Value, McpError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.probe action is required"))?;
        let src = arguments
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.probe src is required"))?;
        let project = self
            .state()
            .ok()
            .and_then(|snapshot| cut_domain::parse_project(&snapshot.project).ok());
        let src = resolve_media_src(&self.dir, src, project.as_ref())?;
        let opts = arguments.get("opts").cloned().unwrap_or_else(|| json!({}));
        match action {
            "info" => {
                let info = cut_engine::info(&src).map_err(engine_error)?;
                Ok(json!({"action": action, "info": info}))
            }
            "silence" => {
                let noise = opts.get("noiseDb").and_then(Value::as_f64).unwrap_or(-35.0);
                let duration = opts
                    .get("minDuration")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.35);
                let ranges = cut_engine::probe::silence_detect(&src, noise, duration)
                    .map_err(engine_error)?;
                Ok(json!({"action": action, "ranges": ranges}))
            }
            "scenes" => {
                let threshold = opts
                    .get("threshold")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.35);
                let ranges = cut_engine::detect_scenes(&src, threshold).map_err(engine_error)?;
                Ok(json!({"action": action, "ranges": ranges}))
            }
            "frames" => {
                let count = opts
                    .get("frames")
                    .and_then(Value::as_u64)
                    .unwrap_or(8)
                    .clamp(1, 32) as usize;
                let meta = cut_engine::info(&src).map_err(engine_error)?;
                let duration = meta.duration.unwrap_or(60.0).max(0.01);
                let times = (0..count)
                    .map(|index| duration * (index as f64 + 0.5) / count as f64)
                    .collect::<Vec<_>>();
                let frames_dir = self.dir.join(".cut").join("frames");
                let frames =
                    cut_engine::extract_frames(&src, &times, &frames_dir.to_string_lossy(), 1280)
                        .map_err(engine_error)?;
                let mut serialized = Vec::new();
                for frame in frames {
                    let data = fs::read(&frame.path).map_err(io_error)?;
                    serialized.push(json!({
                        "time": frame.time,
                        "path": frame.path,
                        "imageBase64": base64::engine::general_purpose::STANDARD.encode(data),
                        "mediaType": "image/png",
                    }));
                }
                Ok(json!({"action": action, "frames": serialized}))
            }
            _ => Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.probe action must be info, silence, scenes, or frames",
            )),
        }
    }

    fn media_fetch(&self, arguments: Value) -> Result<Value, McpError> {
        let url = arguments
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.fetch url is required"))?;
        let options = cut_fetcher::FetchOptions {
            cookies: arguments
                .get("cookies")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            ..cut_fetcher::FetchOptions::default()
        };
        let result = cut_fetcher::fetch_video(url, &self.dir, &options)
            .map_err(|error| McpError::new("HANDLER_FAILED", error.to_string()))?;
        let metadata = cut_engine::info(&result.path.to_string_lossy()).map_err(engine_error)?;
        let snapshot = self.state()?;
        let key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("mcp-fetch");
        let metadata = json!({
            "duration": metadata.duration,
            "width": metadata.width,
            "height": metadata.height,
            "fps": metadata.fps,
            "hasAudio": metadata.has_audio,
            "videoCodec": metadata.video_codec,
            "audioCodec": metadata.audio_codec,
            "subs": result.subs_path,
        });
        let (state, source_id, clip_id) = self
            .runtime
            .import_media_request(
                "mcp",
                key,
                &result.path.to_string_lossy(),
                snapshot.revision,
                Some(metadata),
                None,
            )
            .map_err(McpError::from)?;
        Ok(json!({
            "path": result.path,
            "subsPath": result.subs_path,
            "title": result.title,
            "sourceId": source_id,
            "clipId": clip_id,
            "revision": state.revision,
        }))
    }

    fn media_import(&self, arguments: Value) -> Result<Value, McpError> {
        let input = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.import path is required"))?;
        let path = resolve_import_path(&self.dir, input)?;
        let snapshot = self.state()?;
        if let Ok(project) = cut_domain::parse_project(&snapshot.project) {
            if let Some(source) = project.sources.iter().find(|source| {
                Path::new(&source.path).canonicalize().ok().as_deref()
                    == path.canonicalize().ok().as_deref()
            }) {
                let clip_id = project
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter())
                    .find(|clip| clip.source_id.as_ref().is_some_and(|id| id == &source.id))
                    .map(|clip| clip.id.to_string())
                    .unwrap_or_default();
                return Ok(json!({
                    "path": path,
                    "sourceId": source.id,
                    "clipId": clip_id,
                    "revision": snapshot.revision,
                }));
            }
        }
        let key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| operation_key("mcp-import"));
        let metadata = arguments.get("metadata").cloned();
        let overlay = arguments.get("overlay").cloned();
        let (state, source_id, clip_id) = self
            .runtime
            .import_media_request(
                "mcp",
                &key,
                &path.to_string_lossy(),
                snapshot.revision,
                metadata,
                overlay,
            )
            .map_err(McpError::from)?;
        Ok(json!({
            "path": path,
            "sourceId": source_id,
            "clipId": clip_id,
            "revision": state.revision,
        }))
    }

    fn render_start(&self, draft: bool) -> Result<Value, McpError> {
        let (job, project, output) = self
            .runtime
            .start_render(if draft { "preview" } else { "full" })
            .map_err(McpError::from)?;
        let runtime = self.runtime.clone();
        let job_id = job.job_id.clone();
        let output_for_thread = output.clone();
        thread::spawn(move || {
            let result =
                cut_runtime::run_render(job_id.clone(), project, output_for_thread.clone(), draft);
            match result {
                Ok(()) => {
                    let _ = runtime.update_job(
                        &job_id,
                        "completed",
                        None,
                        Some(output_for_thread.to_string_lossy().into_owned()),
                    );
                }
                Err(error) => {
                    let _ = runtime.update_job(&job_id, "failed", Some(error.message), None);
                }
            }
        });
        Ok(json!({
            "jobId": job.job_id,
            "output": output,
            "revision": self.state()?.revision,
            "status": "queued",
        }))
    }

    fn render_status(&self, arguments: &Value) -> Result<Value, McpError> {
        let job_id = job_id(arguments)?;
        let job = self.runtime.render_job(job_id).map_err(McpError::from)?;
        Ok(json!({
            "jobId": job_id,
            "status": job.status,
            "output": job.output_path,
            "error": job.error,
        }))
    }

    fn render_cancel(&self, arguments: &Value) -> Result<Value, McpError> {
        let job_id = job_id(arguments)?;
        let job = self.runtime.cancel_render(job_id).map_err(McpError::from)?;
        Ok(json!({"jobId": job_id, "cancelled": job.status == "cancelled", "status": job.status}))
    }

    fn render_result(&self, arguments: &Value) -> Result<Value, McpError> {
        let job_id = job_id(arguments)?;
        let job = self.runtime.render_job(job_id).map_err(McpError::from)?;
        Ok(json!({
            "jobId": job_id,
            "status": job.status,
            "output": job.output_path,
            "error": job.error,
        }))
    }

    fn skills_read(&self, arguments: &Value) -> Result<Value, McpError> {
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "skills.read name is required"))?;
        let skill = find_skill(&self.skills, name)
            .ok_or_else(|| McpError::new("HANDLER_FAILED", format!("unknown skill {name:?}")))?;
        Ok(json!({
            "name": skill.name,
            "description": skill.description,
            "body": skill.body,
        }))
    }

    fn state(&self) -> Result<StateSnapshot, McpError> {
        self.runtime.get_state().map_err(McpError::from)
    }
}

#[derive(Clone)]
struct AgentRouter {
    dir: PathBuf,
    runtime: RuntimeClient,
    skills: Vec<Skill>,
    profile: McpProfile,
}

impl AgentRouter {
    fn call_base(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let router = CutMcpRouter {
            dir: self.dir.clone(),
            runtime: self.runtime.clone(),
            skills: self.skills.clone(),
            profile: self.profile,
            namespace: String::new(),
        };
        if name == "timeline.apply" {
            return router
                .stage_agent_edit(arguments)
                .map_err(|error| error.to_string());
        }
        router
            .call_base(name, arguments)
            .map_err(|error| error.to_string())
    }
}

impl CutMcpRouter {
    /// Stage an agent edit for explicit user review. Direct MCP calls keep the
    /// normal `timeline.apply` mutation semantics; only the agent-facing
    /// catalog uses this proposal boundary.
    fn stage_agent_edit(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let command = canonical_command(&project, &arguments)?;
        let base_revision = arguments
            .get("expectedRevision")
            .and_then(Value::as_u64)
            .unwrap_or(snapshot.revision);
        let request_key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| operation_key("agent-proposal"));
        let command = cut_domain::serialize_command(&command)
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let next = self
            .runtime
            .stage_agent_proposal(
                &request_key,
                base_revision,
                vec![json!({
                    "idempotencyKey": request_key,
                    "commands": [command],
                })],
            )
            .map_err(McpError::from)?;
        Ok(json!({
            "project": next.project,
            "revision": next.revision,
            "canUndo": next.can_undo,
            "canRedo": next.can_redo,
            "pendingProposal": next.pending_proposal,
            "staged": true,
        }))
    }
}

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
            router,
            name: "cut-mcp".into(),
            version: "0.1.0".into(),
            instructions: "Read the timeline with timeline.query, apply canonical v2 commands with timeline.apply, use media.probe for media context, render with render.preview/render.start, and load playbooks with skills.read. Shell access is unavailable.".into(),
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
                        .unwrap_or("2024-11-05"),
                    "capabilities": {"tools": {"listChanged": false}},
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
                let structured = self.router.call(name, arguments);
                let is_error = structured.get("ok").and_then(Value::as_bool) == Some(false);
                let text = serde_json::to_string(&structured).unwrap_or_else(|_| {
                    "{\"ok\":false,\"error\":{\"code\":\"SERIALIZATION_FAILED\",\"message\":\"result could not be serialized\"}}".into()
                });
                Some(json_rpc_result(
                    id,
                    json!({
                        "content": [{"type": "text", "text": text}],
                        "structuredContent": structured,
                        "isError": is_error,
                    }),
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

fn tool_metadata(name: &str) -> (&'static str, Value, ToolAnnotations) {
    let read_only = ToolAnnotations {
        read_only_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    };
    let mutation = ToolAnnotations {
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    };
    match name {
        "project.inspect" => (
            "Inspect the current project summary or full v2 project.",
            json!({
                "type": "object",
                "properties": {"include": {"enum": ["summary", "full"]}},
                "additionalProperties": false
            }),
            read_only,
        ),
        "project.validate" => (
            "Validate the authoritative project projection.",
            json!({"type": "object", "properties": {"strict": {"type": "boolean"}}, "additionalProperties": false}),
            read_only,
        ),
        "timeline.query" => (
            "Inspect track and clip ids, source ranges, and timeline placement.",
            json!({
                "type": "object",
                "properties": {
                    "include": {"enum": ["summary", "full"]},
                    "trackId": {"type": "string"},
                    "clipId": {"type": "string"}
                },
                "additionalProperties": false
            }),
            read_only,
        ),
        "timeline.apply" => (
            "Apply one atomic canonical v2 command or legacy operation through cutd.",
            json!({
                "type": "object",
                "properties": {
                    "expectedRevision": {"type": "integer", "minimum": 0},
                    "idempotencyKey": {"type": "string"},
                    "command": {"type": "object"},
                    "commands": {"type": "array", "minItems": 1, "items": {"type": "object"}},
                    "op": {"type": "object"},
                    "ops": {"type": "array", "minItems": 1, "items": {"type": "object"}}
                },
                "additionalProperties": false
            }),
            mutation,
        ),
        "timeline.undo" | "timeline.redo" => (
            "Move the durable project history by one or more steps.",
            json!({
                "type": "object",
                "properties": {
                    "steps": {"type": "integer", "minimum": 1, "maximum": 100},
                    "expectedRevision": {"type": "integer", "minimum": 0},
                    "idempotencyKey": {"type": "string"}
                },
                "additionalProperties": false
            }),
            mutation,
        ),
        "timeline.diff" => (
            "Compare two project values or compare the current project with itself.",
            json!({"type": "object", "properties": {"from": {"type": "object"}, "to": {"type": "object"}}, "additionalProperties": false}),
            read_only,
        ),
        "media.probe" => (
            "Probe media metadata, silence, scenes, or sampled frames.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"enum": ["info", "silence", "scenes", "frames"]},
                    "src": {"type": "string", "minLength": 1},
                    "opts": {"type": "object"}
                },
                "required": ["action", "src"],
                "additionalProperties": false
            }),
            read_only,
        ),
        "media.fetch" => (
            "Download URL media with yt-dlp and import it through cutd.",
            json!({
                "type": "object",
                "properties": {"url": {"type": "string", "minLength": 1}, "cookies": {"type": "string"}, "idempotencyKey": {"type": "string"}},
                "required": ["url"],
                "additionalProperties": false
            }),
            mutation,
        ),
        "media.import" => (
            "Import one local media file through the canonical runtime.",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string", "minLength": 1}, "idempotencyKey": {"type": "string"}, "metadata": {"type": "object"}, "overlay": {"type": "object"}},
                "required": ["path"],
                "additionalProperties": false
            }),
            mutation,
        ),
        "render.preview" | "render.start" => (
            "Queue a durable draft or full render job.",
            json!({"type": "object", "properties": {"draft": {"type": "boolean"}}, "additionalProperties": false}),
            mutation,
        ),
        "render.status" | "render.cancel" | "render.result" => (
            "Inspect, cancel, or retrieve a durable render job.",
            json!({"type": "object", "properties": {"jobId": {"type": "string", "minLength": 1}}, "required": ["jobId"], "additionalProperties": false}),
            if name == "render.status" || name == "render.result" {
                read_only
            } else {
                mutation
            },
        ),
        "vision.track_objects" => (
            "Track configured cam, chat, or content regions when a YOLO runtime is installed.",
            json!({"type": "object", "properties": {"source": {"type": "string"}}, "required": ["source"], "additionalProperties": false}),
            read_only,
        ),
        "skills.read" => {
            let schema = skill_read_tool_schema();
            (
                "Read the full body of one loaded playbook by exact name.",
                schema
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"})),
                read_only,
            )
        }
        _ => (
            "Unknown capability.",
            json!({"type": "object", "additionalProperties": false}),
            read_only,
        ),
    }
}

fn canonical_command(project: &Project, arguments: &Value) -> Result<Command, McpError> {
    if let Some(command) = arguments.get("command") {
        return cut_domain::parse_command(command)
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()));
    }
    if let Some(commands) = arguments.get("commands").and_then(Value::as_array) {
        let commands = commands
            .iter()
            .map(cut_domain::parse_command)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        if commands.is_empty() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "commands must not be empty",
            ));
        }
        return Ok(Command::Batch { commands });
    }
    if let Some(op) = arguments.get("op") {
        return legacy_command(project, op);
    }
    if let Some(ops) = arguments.get("ops").and_then(Value::as_array) {
        let commands = ops
            .iter()
            .map(|op| legacy_command(project, op))
            .collect::<Result<Vec<_>, _>>()?;
        if commands.is_empty() {
            return Err(McpError::new("INVALID_ARGUMENTS", "ops must not be empty"));
        }
        return Ok(Command::Batch { commands });
    }
    Err(McpError::new(
        "INVALID_ARGUMENTS",
        "timeline.apply requires command, commands, op, or ops",
    ))
}

fn legacy_command(project: &Project, value: &Value) -> Result<Command, McpError> {
    let object = value
        .as_object()
        .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "legacy operation must be an object"))?;
    let operation = object
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "legacy operation requires op"))?;
    let clip_id = || {
        object
            .get("clipId")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "operation requires clipId"))
    };
    let update = |patch: Map<String, Value>| {
        Ok(Command::UpdateClip {
            clip_id: cut_domain::ClipId::new(clip_id()?)
                .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?,
            patch: patch
                .into_iter()
                .map(|(key, value)| (key, json_to_domain(value)))
                .collect(),
        })
    };
    match operation {
        "trim" => {
            let mut patch = Map::new();
            if let Some(value) = object.get("newIn").and_then(Value::as_f64) {
                patch.insert("in".into(), json!(value));
            }
            if let Some(value) = object.get("newOut").and_then(Value::as_f64) {
                patch.insert("out".into(), json!(value));
            }
            if patch.is_empty() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "trim requires newIn or newOut",
                ));
            }
            update(patch)
        }
        "set_speed" => update(single_patch(object, "speed")?),
        "set_volume" => update(single_patch(object, "volume")?),
        "set_crop" => update(single_patch(object, "crop")?),
        "remove_clip" => Ok(Command::RemoveClip {
            clip_id: cut_domain::ClipId::new(clip_id()?)
                .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?,
            track_id: None,
        }),
        "move_clip" => {
            let index = object
                .get("toIndex")
                .and_then(Value::as_u64)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "move_clip requires toIndex"))?
                as usize;
            Ok(Command::MoveClip {
                clip_id: cut_domain::ClipId::new(clip_id()?)
                    .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?,
                track_id: object
                    .get("trackId")
                    .and_then(Value::as_str)
                    .map(cut_domain::TrackId::new)
                    .transpose()
                    .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?,
                to_track_id: None,
                index: None,
                to_index: Some(index),
            })
        }
        "split" => {
            let id = clip_id()?;
            let at = object
                .get("at")
                .and_then(Value::as_f64)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "split requires at"))?;
            let (track_id, clip) = find_clip(project, id)?;
            let clip_in = clip.source_in.unwrap_or(0.0);
            let clip_out = clip
                .source_out
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "split requires clip out"))?;
            if !(at > clip_in && at < clip_out) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "split must be inside clip bounds",
                ));
            }
            let right_id = format!("{id}-b");
            let right = json!({
                "id": right_id,
                "sourceId": clip.source_id,
                "in": at,
                "out": clip_out,
                "timelineStart": clip.timeline_start + (at - clip_in) / clip.speed.unwrap_or(1.0),
                "speed": clip.speed,
                "volume": clip.volume,
            });
            let left = json!({"type": "update_clip", "clipId": id, "patch": {"out": at}});
            let right = json!({"type": "add_clip", "trackId": track_id, "clip": right});
            canonical_command(project, &json!({"commands": [left, right]}))
        }
        "add_source" => {
            let source = object
                .get("source")
                .cloned()
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_source requires source"))?;
            cut_domain::parse_command(&json!({"type": "add_source", "source": source}))
                .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))
        }
        "add_clip" => {
            let raw = object
                .get("clip")
                .and_then(Value::as_object)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_clip requires clip"))?;
            let source_id = raw
                .get("src")
                .or_else(|| raw.get("sourceId"))
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_clip requires src"))?;
            let track_id = object
                .get("trackId")
                .and_then(Value::as_str)
                .or_else(|| project.tracks.first().map(|track| track.id.as_str()))
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_clip requires trackId"))?;
            let clip = json!({
                "id": raw.get("id").and_then(Value::as_str).unwrap_or("clip-new"),
                "sourceId": source_id,
                "in": raw.get("in").and_then(Value::as_f64).unwrap_or(0.0),
                "out": raw.get("out").and_then(Value::as_f64).unwrap_or(0.0),
                "timelineStart": raw.get("timelineStart").and_then(Value::as_f64).unwrap_or_else(|| project_duration(project)),
                "speed": raw.get("speed"),
                "volume": raw.get("volume"),
            });
            cut_domain::parse_command(&json!({
                "type": "add_clip",
                "trackId": track_id,
                "clip": clip,
                "index": object.get("index"),
            }))
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))
        }
        "add_subtitles" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_subtitles requires path"))?;
            let id = next_overlay_id(project, "subs");
            let mut overlay = json!({"id": id, "type": "subtitles", "path": path});
            if let Some(style) = object.get("style") {
                overlay["style"] = style.clone();
            }
            cut_domain::parse_command(&json!({"type": "add_overlay", "overlay": overlay}))
                .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))
        }
        "add_captions" => {
            let cues = object
                .get("cues")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "add_captions requires cues"))?;
            let id = next_overlay_id(project, "captions");
            let mut overlay = json!({"id": id, "type": "captions", "cues": cues});
            if let Some(style) = object.get("style") {
                overlay["style"] = style.clone();
            }
            cut_domain::parse_command(&json!({"type": "add_overlay", "overlay": overlay}))
                .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))
        }
        "remove_overlay" => {
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "remove_overlay requires id"))?;
            Ok(Command::RemoveOverlay {
                overlay_id: id.to_owned(),
            })
        }
        _ => Err(McpError::new(
            "INVALID_ARGUMENTS",
            format!("unsupported legacy operation: {operation}"),
        )),
    }
}

fn single_patch(object: &Map<String, Value>, key: &str) -> Result<Map<String, Value>, McpError> {
    let value = object
        .get(key)
        .cloned()
        .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", format!("operation requires {key}")))?;
    let mut patch = Map::new();
    patch.insert(key.into(), value);
    Ok(patch)
}

fn json_to_domain(value: Value) -> cut_domain::JsonValue {
    match value {
        Value::Null => cut_domain::JsonValue::Null,
        Value::Bool(value) => cut_domain::JsonValue::Bool(value),
        Value::Number(value) => cut_domain::JsonValue::Number(value),
        Value::String(value) => cut_domain::JsonValue::String(value),
        Value::Array(values) => {
            cut_domain::JsonValue::Array(values.into_iter().map(json_to_domain).collect())
        }
        Value::Object(values) => cut_domain::JsonValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, json_to_domain(value)))
                .collect(),
        ),
    }
}

fn find_clip(project: &Project, id: &str) -> Result<(String, Clip), McpError> {
    let mut found = None;
    for track in &project.tracks {
        if let Some(clip) = track.clips.iter().find(|clip| clip.id.as_str() == id) {
            if found.is_some() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("clip id is ambiguous: {id}"),
                ));
            }
            found = Some((track.id.to_string(), clip.clone()));
        }
    }
    found.ok_or_else(|| McpError::new("INVALID_ARGUMENTS", format!("clip not found: {id}")))
}

fn next_overlay_id(project: &Project, prefix: &str) -> String {
    let mut index = 1;
    loop {
        let candidate = format!("{prefix}{index}");
        if !project
            .overlays
            .iter()
            .any(|overlay| overlay.id() == Some(candidate.as_str()))
        {
            return candidate;
        }
        index += 1;
    }
}

fn project_duration(project: &Project) -> f64 {
    let mut duration = project.duration();
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(length) = clip.timeline_duration() {
                duration = duration.max(clip.timeline_start + length);
            }
        }
    }
    duration
}

fn parse_project(snapshot: &StateSnapshot) -> Result<Project, McpError> {
    cut_domain::parse_project(&snapshot.project)
        .map_err(|error| McpError::new("RUNTIME_ERROR", error.to_string()))
}

fn summary(project: &Project, revision: u64) -> Value {
    json!({
        "version": 2,
        "revision": revision,
        "sourceCount": project.sources.len(),
        "trackCount": project.tracks.len(),
        "clipCount": project.tracks.iter().map(|track| track.clips.len()).sum::<usize>(),
        "duration": project_duration(project),
    })
}

fn state_result(snapshot: &StateSnapshot) -> Value {
    json!({
        "project": snapshot.project,
        "revision": snapshot.revision,
        "canUndo": snapshot.can_undo,
        "canRedo": snapshot.can_redo,
    })
}

fn job_id(arguments: &Value) -> Result<&str, McpError> {
    arguments
        .get("jobId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "jobId is required"))
}

fn resolve_media_src(
    dir: &Path,
    input: &str,
    project: Option<&Project>,
) -> Result<String, McpError> {
    let candidate = resolve_import_path(dir, input)?;
    if is_contained(dir, &candidate) {
        return Ok(candidate.to_string_lossy().into_owned());
    }
    if project.is_some_and(|project| {
        project.sources.iter().any(|source| {
            Path::new(&source.path)
                .canonicalize()
                .ok()
                .is_some_and(|path| path == candidate)
        })
    }) {
        return Ok(candidate.to_string_lossy().into_owned());
    }
    Err(McpError::new(
        "INVALID_ARGUMENTS",
        format!("media path is outside the project directory: {input}"),
    ))
}

fn resolve_import_path(dir: &Path, input: &str) -> Result<PathBuf, McpError> {
    if input.trim().is_empty() || input.as_bytes().contains(&0) {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media path must be non-empty",
        ));
    }
    let path = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        dir.join(input)
    };
    let path = path.canonicalize().map_err(|error| {
        McpError::new(
            "INVALID_ARGUMENTS",
            format!("media path is unavailable: {error}"),
        )
    })?;
    if !path.is_file() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media path must be a file",
        ));
    }
    Ok(path)
}

fn is_contained(root: &Path, candidate: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    candidate.starts_with(root)
}

fn load_project_skills(dir: &Path) -> io::Result<Vec<Skill>> {
    let mut directories = vec![std::env::current_dir()?.join("skills")];
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".cut").join("skills"));
    }
    directories.push(dir.join(".cut").join("skills"));
    load_skills(&directories)
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

fn engine_error(error: impl fmt::Display) -> McpError {
    McpError::new("HANDLER_FAILED", error.to_string())
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
    fn core_profile_keeps_stable_mcp_order() {
        assert_eq!(McpProfile::Core.names()[0], "timeline.query");
        assert_eq!(McpProfile::Core.names().last(), Some(&"skills.read"));
    }

    #[test]
    fn protocol_conformance_fixture_matches_core_profile() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/protocol/conformance.json"))
                .expect("protocol fixture");
        let expected = fixture["mcp"]["core_tools"]
            .as_array()
            .expect("core tool names")
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .expect("string tool names");
        assert_eq!(McpProfile::Core.names(), expected.as_slice());
    }

    #[test]
    fn stdio_server_handles_initialize_and_tools_list_without_runtime_io() {
        let root = TempDir::new().expect("temp dir");
        let router = CutMcpRouter::new(root.path(), McpProfile::Core, None).expect("router");
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
            CORE_TOOL_NAMES.len()
        );
    }

    #[test]
    fn legacy_trim_lowers_to_typed_update() {
        let project = cut_domain::empty_project();
        let mut project_value = cut_domain::serialize_project(&project).unwrap();
        project_value["sources"] = json!([{
            "id": "source-1", "path": "/tmp/source.mp4", "duration": 4
        }]);
        project_value["tracks"][0]["clips"] = json!([{
            "id": "clip-1", "sourceId": "source-1", "in": 0, "out": 4, "timelineStart": 0
        }]);
        let project = cut_domain::parse_project(&project_value).unwrap();
        let command = legacy_command(
            &project,
            &json!({"op": "trim", "clipId": "clip-1", "newOut": 2}),
        )
        .unwrap();
        assert!(matches!(command, Command::UpdateClip { .. }));
    }
}
