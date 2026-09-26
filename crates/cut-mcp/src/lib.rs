//! MCP stdio transport and project-scoped capability router.
//!
//! Every mutation goes through cutd. The MCP process is a stateless adapter:
//! it reads an authoritative snapshot for each request and sends canonical
//! commands back with an idempotency key and revision.

mod composition;
mod composition_bridge;
mod composition_catalog;
mod composition_files;
mod composition_manifest;
mod composition_render;
mod contracts;
pub mod docs;
pub mod inbound;
mod results;

use base64::Engine;
use cut_agent::{AgentTool, ToolMap};
use cut_domain::{Clip, Command, OutlineNode, Project};
use cut_engine::derivatives::{DerivativeCache, SourceIdentity};
use cut_runtime::{RuntimeClient, RuntimeError, StateSnapshot};
use cut_skills::{
    find_skill, load_skills_with_bundled, skill_list_tool_schema, skill_read_tool_schema, Skill,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
const MAX_INLINE_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CAPTURE_TIMES: usize = 64;
const MAX_OBSERVATION_TIMES: usize = 32;
const MAX_OBSERVATION_EVIDENCE: usize = 5;
/// Keep one observation request bounded so an agent can refine its query.
pub const MAX_OBSERVATION_RANGE_SECONDS: f64 = 120.0;

const AGENT_CANONICAL_ONLY_TOOLS: &[&str] = &[
    "media.fetch",
    "media.import",
    "timeline.undo",
    "timeline.redo",
];

pub const CORE_TOOL_NAMES: &[&str] = &[
    "results.read",
    "tools.search",
    "tools.describe",
    "timeline.query",
    "timeline.check",
    "composition.check",
    "composition.catalog",
    "composition.preview",
    "composition.render",
    "timeline.capture",
    "timeline.apply",
    "media.probe",
    "media.asset",
    "workflow.roughcut",
    "render.preview",
    "render.start",
    "render.result",
    "project.outline",
    "skills.list",
    "skills.read",
];

pub const FULL_TOOL_NAMES: &[&str] = &[
    "results.read",
    "tools.search",
    "tools.describe",
    "project.inspect",
    "project.outline",
    "timeline.query",
    "timeline.check",
    "composition.check",
    "composition.catalog",
    "composition.preview",
    "composition.render",
    "timeline.capture",
    "timeline.apply",
    "project.validate",
    "timeline.diff",
    "proposal.capture",
    "render.preview",
    "render.start",
    "render.status",
    "render.cancel",
    "render.result",
    "media.probe",
    "media.asset",
    "workflow.roughcut",
    "skills.list",
    "skills.read",
];

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeOptions {
    noise_db: Option<f64>,
    min_duration: Option<f64>,
    threshold: Option<f64>,
    frames: Option<usize>,
    times: Option<Vec<f64>>,
    time: Option<f64>,
    max_width: Option<u32>,
    columns: Option<usize>,
    buckets: Option<usize>,
    include_base64: Option<bool>,
    include_image: Option<bool>,
    image_width: Option<u32>,
    image_height: Option<u32>,
    start: Option<f64>,
    end: Option<f64>,
    duration: Option<f64>,
    language: Option<String>,
    backend: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    model_version: Option<String>,
    backend_version: Option<String>,
    evidence: Option<Vec<ObservationEvidence>>,
    phase: Option<ObservationPhase>,
}

/// A cached media modality that can be requested by one bounded observation.
///
/// Each variant delegates to the existing `media.probe` derivative path; the
/// enum only gives agents a stable, composable request contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ObservationEvidence {
    Frames,
    Filmstrip,
    Waveform,
    Transcript,
    Listen,
}

/// Whether an observation is narrowing candidates or examining a selected
/// target. The phase is trace metadata and never changes media semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ObservationPhase {
    Coarse,
    Targeted,
}

impl ObservationEvidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Filmstrip => "filmstrip",
            Self::Waveform => "waveform",
            Self::Transcript => "transcript",
            Self::Listen => "listen",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationArtifact {
    kind: ObservationEvidence,
    data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedAssetRecord {
    asset_id: String,
    path: String,
    kind: String,
    request_fingerprint: String,
    content_fingerprint: String,
    backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedAssetManifest {
    schema_version: u8,
    assets: Vec<GeneratedAssetRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoughcutSelection {
    source_id: String,
    start: f64,
    end: f64,
    #[serde(default)]
    timeline_start: Option<f64>,
    #[serde(default)]
    track_id: Option<String>,
    #[serde(default)]
    clip_id: Option<String>,
    #[serde(default)]
    speed: Option<f64>,
    rationale: String,
    #[serde(default)]
    observation_ref: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureOptions {
    start: Option<f64>,
    end: Option<f64>,
    duration: Option<f64>,
    times: Option<Vec<f64>>,
    count: Option<usize>,
    max_width: Option<u32>,
    columns: Option<usize>,
    include_base64: Option<bool>,
    draft: Option<bool>,
    format: Option<String>,
}

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

    /// Stable profile label used by generated agent documentation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Full => "full",
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

    /// Return the skills available to this project-scoped adapter.
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Return the profile used to advertise this adapter's tools.
    pub const fn profile(&self) -> McpProfile {
        self.profile
    }

    /// Generate the deterministic project-local agent documentation bundle.
    pub fn generate_agent_docs(&self) -> Result<docs::GeneratedDocs, docs::DocsError> {
        docs::generate_agent_docs(self)
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
            let agent_name = self
                .parse_name(&definition.name)
                .unwrap_or_else(|| definition.name.clone());
            if AGENT_CANONICAL_ONLY_TOOLS.contains(&agent_name.as_str()) {
                continue;
            }
            let router = self.clone_for_agent();
            // Namespaces belong to the external MCP adapter, not the shared
            // built-in agent's bootstrap/discovery contract.
            let name = agent_name.clone();
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
        let base_name = self.parse_name(presented_name).unwrap_or_else(|| {
            presented_name
                .strip_prefix(&format!("{}.", self.namespace))
                .unwrap_or(presented_name)
                .to_owned()
        });
        let result = if AGENT_CANONICAL_ONLY_TOOLS.contains(&base_name.as_str()) {
            Err(McpError::new(
                "AGENT_CANONICAL_BYPASS",
                agent_canonical_bypass_error(&base_name),
            ))
        } else if base_name == "timeline.apply" {
            self.stage_agent_edit(arguments)
        } else {
            self.call_base(&base_name, arguments)
        };
        match result {
            Ok(result) => {
                let mut result = results::bound(&self.dir, result);
                if result.get("readWith").is_some() {
                    result["readWith"]["tool"] = json!(self.definition("results.read").name);
                }
                json!({"ok": true, "result": result})
            }
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
            "results.read" => results::read(&self.dir, &arguments)
                .map_err(|error| McpError::new("RESULT_UNAVAILABLE", error)),
            "tools.search" => self.tools_search(&arguments),
            "tools.describe" => self.tools_describe(&arguments),
            "project.inspect" => self.project_inspect(arguments),
            "project.outline" => self.project_outline(arguments),
            "project.validate" => self.project_validate(arguments),
            "timeline.query" => self.timeline_query(arguments),
            "timeline.check" => self.timeline_check(arguments),
            "composition.check" => composition::check(&self.dir, &arguments),
            "composition.catalog" => composition_catalog::catalog(&self.dir, &arguments),
            "composition.preview" => composition_render::execute(
                &self.dir,
                &arguments,
                composition_render::Mode::Preview,
                None,
            ),
            "composition.render" => composition_render::execute(
                &self.dir,
                &arguments,
                composition_render::Mode::Render,
                None,
            ),
            "timeline.capture" => self.timeline_capture(arguments),
            "timeline.apply" => self.timeline_apply(arguments),
            "timeline.undo" => self.history_move(arguments, false),
            "timeline.redo" => self.history_move(arguments, true),
            "timeline.diff" => self.timeline_diff(arguments),
            "proposal.capture" => self.proposal_capture(arguments),
            "media.probe" => self.media_probe(arguments),
            "media.asset" => self.media_asset(arguments),
            "media.fetch" => self.media_fetch(arguments),
            "media.import" => self.media_import(arguments),
            "workflow.roughcut" => self.workflow_roughcut(arguments),
            "render.preview" => self.render_start(true, &arguments),
            "render.start" => self.render_start(
                arguments
                    .get("draft")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                &arguments,
            ),
            "render.status" => self.render_status(&arguments),
            "render.cancel" => self.render_cancel(&arguments),
            "render.result" => self.render_result(&arguments),
            "skills.read" => self.skills_read(&arguments),
            "skills.list" => self.skills_list(&arguments),
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
        match arguments.get("include").and_then(Value::as_str) {
            Some("full") => result["project"] = snapshot.project,
            Some("outline") => {
                result["outline"] = serde_json::to_value(cut_domain::project_outline(&project))
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
            }
            _ => {}
        }
        result["sourceVersion"] = json!(snapshot.source_version);
        result["normalizedVersion"] = json!(2);
        Ok(result)
    }

    fn project_outline(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let mut outline = cut_domain::project_outline(&project);
        let kind = arguments.get("kind").and_then(Value::as_str);
        let track_id = arguments.get("trackId").and_then(Value::as_str);
        let node_path = arguments.get("nodePath").and_then(Value::as_str);
        let mut selected_child_total = None;
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        outline.root.children.retain(|node| {
            kind.is_none_or(|value| node.kind.as_str() == value)
                && track_id.is_none_or(|value| node.entity_id.as_deref() == Some(value))
        });
        if let Some(path) = node_path {
            if let Some(node) = outline
                .root
                .children
                .iter()
                .find_map(|node| find_outline_node(node, path).cloned())
            {
                outline.root.children = vec![node];
            } else {
                outline.root.children.clear();
            }
            let child_offset = arguments
                .get("childOffset")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let child_limit = arguments
                .get("childLimit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 500) as usize;
            if let Some(node) = outline
                .root
                .children
                .first_mut()
                .and_then(|root| find_outline_node_mut(root, path))
            {
                selected_child_total = Some(node.children.len());
                node.children = node
                    .children
                    .drain(..)
                    .skip(child_offset)
                    .take(child_limit)
                    .collect();
            }
        }
        let filtered_total = outline.root.children.len();
        outline.root.children = outline
            .root
            .children
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        let max_nodes = arguments
            .get("maxNodes")
            .and_then(Value::as_u64)
            .unwrap_or(500)
            .clamp(2, 5000) as usize;
        let before_nodes = outline
            .root
            .children
            .iter()
            .map(count_outline_nodes)
            .sum::<usize>();
        let mut budget = max_nodes;
        let mut children = Vec::new();
        for mut child in outline.root.children.drain(..) {
            if budget == 0 {
                break;
            }
            truncate_outline_nodes(&mut child, &mut budget);
            children.push(child);
        }
        outline.root.children = children;
        let returned_nodes = outline
            .root
            .children
            .iter()
            .map(count_outline_nodes)
            .sum::<usize>();
        let mut result = serde_json::to_value(outline)
            .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
        result["action"] = json!("project.outline");
        result["revision"] = json!(snapshot.revision);
        result["sourceVersion"] = json!(snapshot.source_version);
        result["projectionStatus"] = json!(snapshot.projection_status);
        result["offset"] = json!(offset);
        result["limit"] = json!(limit);
        result["total"] = json!(filtered_total);
        result["nodeCount"] = json!(returned_nodes);
        result["totalNodeCount"] = json!(before_nodes);
        result["truncated"] = json!(before_nodes > max_nodes);
        if let Some(path) = node_path {
            let requested = result["root"]["children"]
                .as_array()
                .and_then(|v| v.first())
                .cloned();
            if let Some(node) = requested {
                let total = selected_child_total
                    .unwrap_or_else(|| node["children"].as_array().map_or(0, Vec::len));
                let page = node["children"].as_array().cloned().unwrap_or_default();
                result["nodePath"] = json!(path);
                result["childTotal"] = json!(total);
                result["childCount"] = json!(page.len());
                let child_offset = arguments
                    .get("childOffset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                result["childNextOffset"] = json!((child_offset.saturating_add(page.len())
                    < total)
                    .then_some(child_offset.saturating_add(page.len())));
            }
        }
        result["nextOffset"] = json!((offset
            .saturating_add(result["root"]["children"].as_array().map_or(0, Vec::len))
            < filtered_total)
            .then_some(
                offset.saturating_add(result["root"]["children"].as_array().map_or(0, Vec::len))
            ));
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
        let range_start = arguments.get("start").and_then(Value::as_f64);
        let range_end = arguments.get("end").and_then(Value::as_f64);
        for key in ["start", "end"] {
            if arguments.get(key).is_some_and(|value| {
                value
                    .as_f64()
                    .is_none_or(|time| !time.is_finite() || time < 0.0)
            }) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("{key} must be a finite non-negative time"),
                ));
            }
        }
        if matches!((range_start,range_end),(Some(start),Some(end)) if end <= start) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "end must be greater than start",
            ));
        }
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let mut seen = 0usize;
        let mut emitted = 0usize;
        let tracks = project
            .tracks
            .iter()
            .filter(|track| track_filter.is_none_or(|id| id == track.id.as_str()))
            .map(|track| {
                let clips = track
                    .clips
                    .iter()
                    .filter(|clip| clip_filter.is_none_or(|id| id == clip.id.as_str()))
                    .filter(|clip| {
                        let start = clip.timeline_start;
                        let end = start + clip.timeline_duration().unwrap_or_default();
                        range_start.is_none_or(|value| end > value)
                            && range_end.is_none_or(|value| start < value)
                    })
                    .filter_map(|clip| {
                        let index = seen;
                        seen += 1;
                        if index < offset || emitted >= limit {
                            return None;
                        }
                        emitted += 1;
                        Some(json!({
                            "id": clip.id,
                            "sourceId": clip.source_id,
                            "timelineStart": clip.timeline_start,
                            "duration": clip.timeline_duration(),
                            "in": clip.source_in,
                            "out": clip.source_out,
                        }))
                    })
                    .collect::<Vec<_>>();
                json!({"id": track.id, "type": track.kind.as_str(), "clips": clips})
            })
            .filter(|track| {
                track["clips"]
                    .as_array()
                    .is_some_and(|clips| !clips.is_empty())
            })
            .collect::<Vec<_>>();
        let total = seen;
        let mut result = json!({"revision": snapshot.revision, "tracks": tracks, "offset": offset, "limit": limit, "total": total, "nextOffset": (offset.saturating_add(emitted) < total).then_some(offset.saturating_add(emitted))});
        if arguments.get("include").and_then(Value::as_str) == Some("full") {
            result["project"] = snapshot.project;
        }
        Ok(result)
    }

    fn timeline_check(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let (project, candidate_proposal) = if let Some(proposal_id) = arguments
            .get("proposalId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
        {
            let (_, after) = self
                .runtime
                .proposal_preview(proposal_id)
                .map_err(McpError::from)?;
            (after, Some(proposal_id.to_owned()))
        } else {
            (snapshot.project.clone(), None)
        };
        let report = cut_engine::check::check_project(&project, Some(&self.dir));
        let mut result = serde_json::to_value(report)
            .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
        result["revision"] = json!(snapshot.revision);
        result["duration"] = json!(project_duration_from_value(&project));
        result["candidate"] = json!(candidate_proposal.is_some());
        if let Some(proposal_id) = candidate_proposal {
            result["proposalId"] = json!(proposal_id);
        }
        result["valid"] = json!(result["findings"]
            .as_array()
            .map(|findings| {
                !findings
                    .iter()
                    .any(|finding| finding["severity"] == "error")
            })
            .unwrap_or(false));
        Ok(result)
    }

    fn timeline_capture(&self, arguments: Value) -> Result<Value, McpError> {
        let snapshot = self.state()?;
        let options = capture_options(&arguments)?;
        self.capture_project(
            &snapshot.project,
            snapshot.revision,
            &options,
            "timeline-capture",
            "timeline.capture",
        )
    }

    fn proposal_capture(&self, arguments: Value) -> Result<Value, McpError> {
        let proposal_id = arguments
            .get("proposalId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "proposalId is required"))?;
        let snapshot = self.state()?;
        let (before, after) = self
            .runtime
            .proposal_preview(proposal_id)
            .map_err(McpError::from)?;
        let mut options = capture_options(&arguments)?;
        options.draft = Some(true);
        if options.start.is_none() && options.end.is_none() && options.duration.is_none() {
            let end = project_duration_from_value(&before)
                .min(project_duration_from_value(&after))
                .min(cut_runtime::MAX_RENDER_RANGE_SECONDS);
            if end <= 0.0 {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "proposal has no positive duration to capture",
                ));
            }
            options.start = Some(0.0);
            options.duration = Some(end);
        }
        let before_capture = self.capture_project(
            &before,
            snapshot.revision,
            &options,
            "proposal-before",
            "proposal.capture",
        )?;
        let after_capture = self.capture_project(
            &after,
            snapshot.revision,
            &options,
            "proposal-after",
            "proposal.capture",
        )?;
        Ok(json!({
            "action": "proposal.capture",
            "proposalId": proposal_id,
            "revision": snapshot.revision,
            "before": before_capture,
            "after": after_capture,
        }))
    }

    fn capture_project(
        &self,
        project_value: &Value,
        revision: u64,
        options: &CaptureOptions,
        cache_kind: &str,
        action: &str,
    ) -> Result<Value, McpError> {
        let project = cut_domain::parse_project(project_value)
            .map_err(|error| McpError::new("RUNTIME_ERROR", error.to_string()))?;
        let report = cut_engine::check::check_project(project_value, Some(&self.dir));
        if !report.is_valid() {
            let details = serde_json::to_value(report)
                .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
            return Err(McpError::new(
                "TIMELINE_INVALID",
                "timeline capture requires a project without error findings",
            )
            .with_details(details));
        }
        let format = options.format.as_deref().unwrap_or("filmstrip");
        if !matches!(format, "filmstrip" | "contact_sheet" | "frame" | "frames") {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "timeline.capture format must be filmstrip, contact_sheet, frame, or frames",
            ));
        }
        let total_duration = cut_engine::compile::timeline_duration_v2(project_value)
            .map_err(|error| engine_error(error.into()))?;
        let start = options.start.unwrap_or(0.0);
        if !start.is_finite() || start < 0.0 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "timeline.capture start must be finite and nonnegative",
            ));
        }
        let end = if let Some(end) = options.end {
            end
        } else if let Some(duration) = options.duration {
            if !duration.is_finite() || duration <= 0.0 {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "timeline.capture duration must be finite and positive",
                ));
            }
            start + duration
        } else {
            total_duration
        };
        if !end.is_finite() || end <= start || end > total_duration + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("timeline.capture range must satisfy 0 <= start < end <= {total_duration}"),
            ));
        }
        let times = capture_times(options, start, end, format)?;
        let max_width = options.max_width.unwrap_or(1280);
        if max_width == 0 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "timeline.capture maxWidth must be positive",
            ));
        }
        let columns = options
            .columns
            .unwrap_or_else(|| times.len().min(4))
            .clamp(1, 16);
        let source_identities = project
            .sources
            .iter()
            .map(|source| {
                SourceIdentity::from_path(&resolve_project_source_path(&self.dir, &source.path))
                    .map_err(engine_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cache = DerivativeCache::new(&self.dir);
        let cache_options = json!({
            "format": format,
            "start": start,
            "end": end,
            "times": times,
            "maxWidth": max_width,
            "columns": columns,
            "draft": options.draft.unwrap_or(false),
        });
        let entry = cache
            .project_entry(
                project_value,
                &source_identities,
                cache_kind,
                &cache_options,
            )
            .map_err(engine_error)?;
        let frame_names = (0..times.len())
            .map(|index| format!("frame-{index:03}.png"))
            .collect::<Vec<_>>();
        let mut required = vec!["capture.mp4", "capture.png", "capture.json"];
        required.extend(frame_names.iter().map(String::as_str));
        let cached = entry.is_complete(&required);
        let payload = if cached {
            entry
                .read_json::<CapturePayload>("capture.json")
                .map_err(engine_error)?
        } else {
            let output = entry.path("capture.mp4");
            let render_options = cut_engine::render::RenderV2Options {
                output: output.to_string_lossy().into_owned(),
                draft: options.draft.unwrap_or(false),
                force_software: true,
                ..cut_engine::render::RenderV2Options::default()
            };
            cut_engine::render::render_v2_range(
                project_value,
                &render_options,
                start,
                end - start,
                None::<fn(f64)>,
            )
            .map_err(engine_error)?;
            let relative_times = times.iter().map(|time| time - start).collect::<Vec<_>>();
            let frames = cut_engine::probe::extract_exact_frames(
                &output.to_string_lossy(),
                &relative_times,
                &entry.directory,
                max_width,
            )
            .map_err(engine_error)?;
            let frame_paths = frames
                .iter()
                .map(|frame| frame.path.clone())
                .collect::<Vec<_>>();
            cut_engine::probe::create_contact_sheet(
                &frame_paths,
                &entry.path("capture.png"),
                columns,
            )
            .map_err(engine_error)?;
            let frame_values = frames
                .into_iter()
                .zip(times.iter().copied())
                .map(|(frame, time)| {
                    json!({
                        "time": time,
                        "path": frame.path,
                        "mediaType": frame.media_type,
                    })
                })
                .collect::<Vec<_>>();
            let payload = CapturePayload {
                start,
                end,
                duration: end - start,
                path: entry.path("capture.png").to_string_lossy().into_owned(),
                media_type: "image/png".into(),
                frames: frame_values,
            };
            entry
                .write_json("capture.json", &payload)
                .map_err(engine_error)?;
            entry.mark_complete(&required).map_err(engine_error)?;
            payload
        };
        let mut result = serde_json::to_value(&payload)
            .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
        result["action"] = json!(action);
        result["format"] = json!(format);
        result["key"] = json!(entry.key);
        result["cached"] = json!(cached);
        result["revision"] = json!(revision);
        if options.include_base64.unwrap_or(false) {
            add_inline_base64(&mut result, "path", "imageBase64")?;
            if let Some(frames) = result.get_mut("frames").and_then(Value::as_array_mut) {
                for frame in frames {
                    add_inline_base64(frame, "path", "imageBase64")?;
                }
            }
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
        let source_ref = arguments
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.probe src is required"))?;
        let source_ref = source_ref.to_owned();
        let project = self
            .state()
            .ok()
            .and_then(|snapshot| cut_domain::parse_project(&snapshot.project).ok());
        let src = resolve_media_src(&self.dir, &source_ref, project.as_ref())?;
        let opts = probe_options(&arguments)?;
        match action {
            "observe" => self.media_observe(&source_ref, &src, &opts),
            "info" => {
                let info = cut_engine::info(&src).map_err(engine_error)?;
                Ok(json!({"action": action, "info": info}))
            }
            "silence" => {
                let noise = opts.noise_db.unwrap_or(-35.0);
                let duration = opts.min_duration.unwrap_or(0.35);
                if !noise.is_finite() || !duration.is_finite() || duration <= 0.0 {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "silence noiseDb must be finite and minDuration must be positive",
                    ));
                }
                let ranges = cut_engine::probe::silence_detect(&src, noise, duration)
                    .map_err(engine_error)?;
                Ok(json!({"action": action, "ranges": ranges}))
            }
            "scenes" => {
                let threshold = opts.threshold.unwrap_or(0.35);
                if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "scene threshold must be finite and between 0 and 1",
                    ));
                }
                let ranges = cut_engine::detect_scenes(&src, threshold).map_err(engine_error)?;
                Ok(json!({"action": action, "ranges": ranges}))
            }
            "frames" | "frame" => {
                let meta = cut_engine::info(&src).map_err(engine_error)?;
                let duration = meta.duration.unwrap_or(60.0).max(0.01);
                let times = if action == "frame" {
                    opts.times.clone().or_else(|| opts.time.map(|time| vec![time])).ok_or_else(|| {
                        McpError::new("INVALID_ARGUMENTS", "media.probe frame requires opts.time or opts.times")
                    })?
                } else if let Some(times) = opts.times.clone() {
                    times
                } else {
                    let count = opts.frames.unwrap_or(8).clamp(1, 32);
                    (0..count)
                        .map(|index| duration * (index as f64 + 0.5) / count as f64)
                        .collect::<Vec<_>>()
                };
                let max_width = opts.max_width.unwrap_or(1280);
                let result = cut_engine::cached_exact_frames(
                    &src,
                    &DerivativeCache::new(&self.dir),
                    &times,
                    max_width,
                )
                .map_err(engine_error)?;
                let include_base64 = opts.include_base64.unwrap_or(false);
                let mut serialized = serde_json::to_value(&result)
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
                serialized["action"] = json!(action);
                if include_base64 {
                    if let Some(frames) = serialized.get_mut("frames").and_then(Value::as_array_mut) {
                        for frame in frames {
                            add_inline_base64(frame, "path", "imageBase64")?;
                        }
                    }
                }
                Ok(serialized)
            }
            "filmstrip" | "contact_sheet" => {
                let meta = cut_engine::info(&src).map_err(engine_error)?;
                let duration = meta.duration.unwrap_or(60.0).max(0.01);
                let times = if let Some(times) = opts.times.clone() {
                    times
                } else {
                    let count = opts.frames.unwrap_or(8).clamp(1, 32);
                    (0..count)
                        .map(|index| duration * (index as f64 + 0.5) / count as f64)
                        .collect::<Vec<_>>()
                };
                let result = cut_engine::cached_filmstrip(
                    &src,
                    &DerivativeCache::new(&self.dir),
                    &times,
                    opts.max_width.unwrap_or(1280),
                    opts.columns.unwrap_or_else(|| times.len().min(4)),
                )
                .map_err(engine_error)?;
                let mut serialized = serde_json::to_value(&result)
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
                serialized["action"] = json!(action);
                if opts.include_base64.unwrap_or(false) {
                    add_inline_base64(&mut serialized, "path", "imageBase64")?;
                }
                Ok(serialized)
            }
            "waveform" => {
                let result = cut_engine::cached_waveform(
                    &src,
                    &DerivativeCache::new(&self.dir),
                    opts.buckets.unwrap_or(256),
                    opts.include_image.unwrap_or(false),
                    opts.image_width.unwrap_or(1200),
                    opts.image_height.unwrap_or(240),
                )
                .map_err(engine_error)?;
                let mut serialized = serde_json::to_value(&result)
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
                serialized["action"] = json!(action);
                if opts.include_base64.unwrap_or(false)
                    && serialized.get("imagePath").is_some_and(|value| !value.is_null())
                {
                    add_inline_base64(&mut serialized, "imagePath", "imageBase64")?;
                }
                Ok(serialized)
            }
            "transcribe" | "listen" => {
                let metadata = cut_engine::info(&src).map_err(engine_error)?;
                let listen = action == "listen";
                let range = transcription_range(&opts, &metadata, listen)?;
                let transcription_options = transcription_options(&self.dir, &opts, range)?;
                let cancellation = cut_engine::CancellationToken::new();
                let result = if listen {
                    cut_engine::listen_cached(
                        &src,
                        &DerivativeCache::new(&self.dir),
                        &transcription_options,
                        &cancellation,
                        |_| {},
                    )
                } else {
                    cut_engine::transcribe_cached(
                        &src,
                        &DerivativeCache::new(&self.dir),
                        &transcription_options,
                        &cancellation,
                        |_| {},
                    )
                }
                .map_err(engine_error)?;
                let mut serialized = serde_json::to_value(&result)
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
                serialized["action"] = json!(action);
                Ok(serialized)
            }
            _ => Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.probe action must be info, silence, scenes, frames, frame, filmstrip, contact_sheet, waveform, transcribe, listen, or observe",
            )),
        }
    }

    /// Collect existing media derivatives for one bounded source range.
    ///
    /// This is an evidence envelope for agent refinement: it does not create
    /// project state and each requested artifact is still generated by the
    /// same cached probe/transcription implementation as its standalone
    /// `media.probe` action.
    fn media_observe(
        &self,
        source_ref: &str,
        src: &str,
        options: &ProbeOptions,
    ) -> Result<Value, McpError> {
        let metadata = cut_engine::info(src).map_err(engine_error)?;
        let observation_revision = self.state()?.revision;
        let range = observation_range(options, &metadata)?;
        let evidence = observation_evidence(options)?;
        let has_visual = evidence.iter().any(|kind| {
            matches!(
                kind,
                ObservationEvidence::Frames | ObservationEvidence::Filmstrip
            )
        });
        let times = if has_visual {
            observation_times(options, range)?
        } else {
            Vec::new()
        };
        let cache = DerivativeCache::new(&self.dir);
        let mut artifacts = Vec::with_capacity(evidence.len());
        for kind in evidence.iter().copied() {
            let mut data = match kind {
                ObservationEvidence::Frames => serde_json::to_value(
                    cut_engine::cached_exact_frames(
                        src,
                        &cache,
                        &times,
                        options.max_width.unwrap_or(1280),
                    )
                    .map_err(engine_error)?,
                )
                .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?,
                ObservationEvidence::Filmstrip => serde_json::to_value(
                    cut_engine::cached_filmstrip(
                        src,
                        &cache,
                        &times,
                        options.max_width.unwrap_or(1280),
                        options.columns.unwrap_or_else(|| times.len().min(4)),
                    )
                    .map_err(engine_error)?,
                )
                .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?,
                ObservationEvidence::Waveform => {
                    let mut result = serde_json::to_value(
                        cut_engine::cached_waveform_range(
                            src,
                            &cache,
                            options.buckets.unwrap_or(256),
                            options.include_image.unwrap_or(false),
                            options.image_width.unwrap_or(1200),
                            options.image_height.unwrap_or(240),
                            range.start,
                            range.end,
                        )
                        .map_err(engine_error)?,
                    )
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
                    result["scope"] = json!("range");
                    result
                }
                ObservationEvidence::Transcript | ObservationEvidence::Listen => {
                    let listen = matches!(kind, ObservationEvidence::Listen);
                    let transcription_options = transcription_options(&self.dir, options, range)?;
                    let cancellation = cut_engine::CancellationToken::new();
                    let result = if listen {
                        cut_engine::listen_cached(
                            src,
                            &cache,
                            &transcription_options,
                            &cancellation,
                            |_| {},
                        )
                    } else {
                        cut_engine::transcribe_cached(
                            src,
                            &cache,
                            &transcription_options,
                            &cancellation,
                            |_| {},
                        )
                    }
                    .map_err(engine_error)?;
                    serde_json::to_value(result)
                        .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?
                }
            };
            data["action"] = json!(kind.as_str());
            if options.include_base64.unwrap_or(false) {
                match kind {
                    ObservationEvidence::Frames => {
                        if let Some(frames) = data.get_mut("frames").and_then(Value::as_array_mut) {
                            for frame in frames {
                                add_inline_base64(frame, "path", "imageBase64")?;
                            }
                        }
                    }
                    ObservationEvidence::Filmstrip => {
                        add_inline_base64(&mut data, "path", "imageBase64")?;
                    }
                    ObservationEvidence::Waveform => {
                        if data.get("imagePath").is_some_and(|value| !value.is_null()) {
                            add_inline_base64(&mut data, "imagePath", "imageBase64")?;
                        }
                    }
                    ObservationEvidence::Transcript | ObservationEvidence::Listen => {}
                }
            }
            artifacts.push(ObservationArtifact { kind, data });
        }
        let source_identity = SourceIdentity::from_path(Path::new(src)).map_err(engine_error)?;
        let artifact_keys = artifacts
            .iter()
            .filter_map(|artifact| artifact.data.get("key").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let observation_id = cut_engine::derivatives::cache_key_for_values(
            "observation",
            &serde_json::to_value(&source_identity)
                .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?,
            &json!({
                "source": source_ref,
                "start": range.start,
                "end": range.end,
                "evidence": evidence,
                "phase": options.phase,
                "artifactKeys": artifact_keys,
            }),
        );
        let observation_ref = json!({
            "id": format!("observation-{}", &observation_id[..16]),
            "sourceRef": source_ref,
            "sourcePath": src,
            "sourceIdentity": source_identity,
            "range": range,
            "evidence": evidence,
            "artifactKeys": artifact_keys,
            "phase": options.phase,
            "revision": observation_revision,
        });
        let envelope = json!({
            "action": "media.observe",
            "phase": options.phase,
            "observationRef": observation_ref,
            "source": {
                "ref": source_ref,
                "path": src,
            },
            "media": metadata,
            "range": range,
            "times": times,
            "evidence": artifacts,
        });
        Ok(envelope)
    }

    /// Register an already-produced local artifact with a content-addressed
    /// provenance manifest. This deliberately does not invoke a provider or
    /// manufacture media; optional project insertion is staged through the
    /// runtime proposal boundary and waits for acceptance.
    fn media_asset(&self, arguments: Value) -> Result<Value, McpError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.asset action is required"))?;
        if action != "register" {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.asset action must be register",
            ));
        }
        let input = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "media.asset path is required"))?;
        let path = resolve_import_path(&self.dir, input)?;
        let kind = arguments
            .get("kind")
            .and_then(Value::as_str)
            .filter(|kind| !kind.trim().is_empty())
            .unwrap_or("video")
            .to_owned();
        if !kind.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        }) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.asset kind must contain only letters, numbers, '-', '_', or '.'",
            ));
        }
        let content_fingerprint = file_fingerprint(&path)?;
        let request_fingerprint = arguments
            .get("requestFingerprint")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                cut_engine::derivatives::cache_key_for_values(
                    "asset-request",
                    &json!({"kind": kind}),
                    &json!({
                        "prompt": arguments.get("prompt"),
                        "provider": arguments.get("provider"),
                        "model": arguments.get("model"),
                        "provenance": arguments.get("provenance"),
                    }),
                )
            });
        let asset_id = format!("asset-{}", &content_fingerprint[..16]);
        let record = GeneratedAssetRecord {
            asset_id,
            path: path.to_string_lossy().into_owned(),
            kind,
            request_fingerprint: request_fingerprint.clone(),
            content_fingerprint,
            backend: "local-artifact".into(),
            provider: arguments
                .get("provider")
                .and_then(Value::as_str)
                .map(str::to_owned),
            model: arguments
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            provenance: arguments.get("provenance").cloned(),
        };
        let manifest_path = self.dir.join(".cut/cache/assets.json");
        let (cached, manifest_record) = upsert_asset_manifest(&manifest_path, &record)?;

        let mut result = json!({
            "action": "media.asset",
            "asset": manifest_record.clone(),
            "cached": cached,
            "backend": "local-artifact",
            "inserted": false,
        });
        if arguments.get("import").and_then(Value::as_bool) == Some(true) {
            let replacement_clip_id = arguments
                .get("replaceClipId")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty());
            let import_key = arguments
                .get("idempotencyKey")
                .and_then(Value::as_str)
                .filter(|key| !key.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    stable_operation_key(
                        "asset-import",
                        &json!({
                            "requestFingerprint": record.request_fingerprint,
                            "contentFingerprint": record.content_fingerprint,
                            "path": path.to_string_lossy(),
                            "replaceClipId": replacement_clip_id,
                        }),
                    )
                });
            let snapshot = self.state()?;
            let metadata = arguments.get("metadata").cloned().or_else(|| {
                Some(json!({
                    "metadata": {
                        "generatedAssetId": manifest_record.asset_id,
                        "assetProvenance": {
                            "requestFingerprint": manifest_record.request_fingerprint,
                            "contentFingerprint": manifest_record.content_fingerprint,
                            "backend": manifest_record.backend,
                        }
                    }
                }))
            });
            let staging_fingerprint = file_fingerprint(&path)?;
            if staging_fingerprint != record.content_fingerprint {
                return Err(McpError::new(
                    "ASSET_CHANGED",
                    "asset changed after registration and before proposal staging",
                )
                .with_details(json!({
                    "expectedContentFingerprint": record.content_fingerprint,
                    "actualContentFingerprint": staging_fingerprint,
                    "path": path,
                })));
            }
            let staged = self
                .runtime
                .stage_import_proposal(
                    &import_key,
                    snapshot.revision,
                    &path.to_string_lossy(),
                    metadata,
                    arguments.get("overlay").cloned(),
                    replacement_clip_id,
                    None,
                )
                .map_err(McpError::from)?;
            let pending = staged
                .pending_proposal
                .as_ref()
                .map(|proposal| serde_json::to_value(proposal).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            let (source_id, clip_id) = staged_import_ids(&pending);
            result["staged"] = json!(true);
            result["inserted"] = json!(false);
            result["import"] = json!({
                "path": path,
                "sourceId": source_id,
                "clipId": clip_id,
                "revision": staged.revision,
                "staged": true,
            });
            result["pendingProposal"] = pending.clone();
            if replacement_clip_id.is_some() {
                result["replacementProposal"] = pending;
            }
        } else if arguments.get("replaceClipId").is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.asset replaceClipId requires import:true; review the replacement proposal before accepting it",
            ));
        }
        Ok(result)
    }

    /// Execute source ranges selected by the agent after its observation loop.
    ///
    /// Semantic selection stays at the agent boundary: an omitted selection
    /// returns `planning_needed`, while each supplied range carries a rationale
    /// and becomes one typed clip command in a single reviewable proposal.
    fn workflow_roughcut(&self, arguments: Value) -> Result<Value, McpError> {
        let goal = arguments
            .get("goal")
            .and_then(Value::as_str)
            .filter(|goal| !goal.trim().is_empty())
            .unwrap_or("Create a concise rough cut")
            .trim()
            .to_owned();
        let requested_duration = arguments
            .get("duration")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if requested_duration != 0.0
            && (!requested_duration.is_finite()
                || requested_duration <= 0.0
                || requested_duration > MAX_OBSERVATION_RANGE_SECONDS)
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!(
                    "workflow.roughcut duration must be finite, positive, and at most {MAX_OBSERVATION_RANGE_SECONDS:.0}s"
                ),
            ));
        }
        let max_retries = arguments
            .get("maxRetries")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .min(2) as usize;
        let Some(raw_selections) = arguments.get("selections").and_then(Value::as_array) else {
            return Ok(json!({
                "action": "workflow.roughcut",
                "status": "planning_needed",
                "goal": goal,
                "message": "The agent must observe media and provide one or more selected source ranges with rationale before roughcut execution.",
                "retryPolicy": {"maxRetries": max_retries, "attempts": 0},
            }));
        };
        if raw_selections.is_empty() {
            return Ok(json!({
                "action": "workflow.roughcut",
                "status": "planning_needed",
                "goal": goal,
                "message": "The agent must provide at least one selected source range with rationale.",
                "retryPolicy": {"maxRetries": max_retries, "attempts": 0},
            }));
        }
        if raw_selections.len() > 16 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "workflow.roughcut supports at most 16 selected ranges",
            ));
        }
        let selections = raw_selections
            .iter()
            .cloned()
            .map(serde_json::from_value::<RoughcutSelection>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection is invalid: {error}"),
                )
            })?;
        let selected_duration = roughcut_selected_duration(&selections)?;
        if requested_duration > 0.0 && selected_duration > requested_duration + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!(
                    "workflow.roughcut selected timeline duration {selected_duration:.3}s exceeds requested output duration {requested_duration:.3}s"
                ),
            )
            .with_details(json!({
                "requestedDuration": requested_duration,
                "selectedDuration": selected_duration,
            })));
        }
        let snapshot = self.state()?;
        let project = parse_project(&snapshot)?;
        let default_track = project
            .tracks
            .iter()
            .find(|track| track.kind == cut_domain::TrackType::Video)
            .ok_or_else(|| {
                McpError::new(
                    "INVALID_ARGUMENTS",
                    "workflow.roughcut requires a video track for selected ranges",
                )
            })?;
        let request_key = arguments
            .get("idempotencyKey")
            .and_then(Value::as_str)
            .filter(|key| !key.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                stable_operation_key(
                    "roughcut",
                    &json!({"goal": goal, "selections": raw_selections}),
                )
            });
        let mut commands = Vec::with_capacity(selections.len());
        let mut targeted = Vec::with_capacity(selections.len());
        let mut observation_refs = Vec::with_capacity(selections.len());
        let mut selection_results = Vec::with_capacity(selections.len());
        let mut timeline_ranges = Vec::with_capacity(selections.len());
        let mut next_timeline = project_duration(&project);
        for (index, selection) in selections.iter().enumerate() {
            if selection.source_id.trim().is_empty() || selection.rationale.trim().is_empty() {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection {index} requires sourceId and rationale"),
                ));
            }
            if !selection.start.is_finite()
                || !selection.end.is_finite()
                || selection.start < 0.0
                || selection.end <= selection.start
                || selection.end - selection.start > MAX_OBSERVATION_RANGE_SECONDS
            {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!(
                        "workflow.roughcut selection {index} has an invalid bounded source range"
                    ),
                ));
            }
            let source = project
                .sources
                .iter()
                .find(|source| source.id.as_str() == selection.source_id)
                .ok_or_else(|| {
                    McpError::new(
                        "INVALID_ARGUMENTS",
                        format!("workflow.roughcut selection {index} references an unknown source"),
                    )
                })?;
            let coarse_observation = selection
                .observation_ref
                .as_ref()
                .map(|value| {
                    let observation = serde_json::from_value::<
                        cut_runtime::ObservationReference,
                    >(value.clone())
                    .map_err(|error| {
                        McpError::new(
                            "INVALID_ARGUMENTS",
                            format!(
                                "workflow.roughcut selection {index} coarse observation is invalid: {error}"
                            ),
                        )
                    })?;
                    validate_observation_reference(&self.dir, &project, snapshot.revision, &observation)?;
                    if observation.source_ref != selection.source_id {
                        return Err(McpError::new(
                            "STALE_OBSERVATION",
                            format!(
                                "workflow.roughcut selection {index} coarse observation source does not match sourceId"
                            ),
                        ));
                    }
                    Ok(observation)
                })
                .transpose()?;
            if source
                .duration
                .is_some_and(|duration| selection.end > duration + 1e-9)
            {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection {index} exceeds source duration"),
                ));
            }
            let speed = selection.speed.unwrap_or(1.0);
            if !speed.is_finite() || speed <= 0.0 {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection {index} speed must be positive"),
                ));
            }
            let track = if let Some(track_id) = selection.track_id.as_deref() {
                project
                    .tracks
                    .iter()
                    .find(|track| track.id.as_str() == track_id)
                    .filter(|track| track.kind == cut_domain::TrackType::Video)
                    .ok_or_else(|| {
                        McpError::new(
                            "INVALID_ARGUMENTS",
                            format!("workflow.roughcut selection {index} requires a video track"),
                        )
                    })?
            } else {
                default_track
            };
            let timeline_start = selection.timeline_start.unwrap_or(next_timeline);
            if !timeline_start.is_finite() || timeline_start < 0.0 {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection {index} timelineStart is invalid"),
                ));
            }
            let timeline_duration = (selection.end - selection.start) / speed;
            let timeline_end = timeline_start + timeline_duration;
            if !timeline_duration.is_finite()
                || timeline_duration <= 0.0
                || !timeline_end.is_finite()
            {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    format!("workflow.roughcut selection {index} timeline range is invalid"),
                ));
            }
            timeline_ranges.push((timeline_start, timeline_end));
            if let Some(clip_id) = selection.clip_id.as_deref() {
                let clip = project
                    .tracks
                    .iter()
                    .flat_map(|track| track.clips.iter())
                    .find(|clip| clip.id.as_str() == clip_id)
                    .ok_or_else(|| {
                        McpError::new(
                            "INVALID_ARGUMENTS",
                            format!(
                                "workflow.roughcut selection {index} references an unknown clip"
                            ),
                        )
                    })?;
                if clip
                    .source_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() != selection.source_id)
                {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        format!("workflow.roughcut selection {index} clip/source mismatch"),
                    ));
                }
                commands.push(json!({
                    "type": "update_clip",
                    "clipId": clip_id,
                    "patch": {
                        "sourceId": selection.source_id,
                        "in": selection.start,
                        "out": selection.end,
                        "timelineStart": timeline_start,
                        "speed": speed,
                    }
                }));
            } else {
                let clip_id = stable_operation_key(
                    "roughcut-clip",
                    &json!({"requestKey": request_key, "index": index}),
                );
                commands.push(json!({
                    "type": "add_clip",
                    "trackId": track.id,
                    "clip": {
                        "id": clip_id,
                        "sourceId": selection.source_id,
                        "in": selection.start,
                        "out": selection.end,
                        "timelineStart": timeline_start,
                        "speed": speed,
                    }
                }));
            }
            let mut observation = self.media_probe(json!({
                "action": "observe",
                "src": source.id,
                "opts": {
                    "start": selection.start,
                    "end": selection.end,
                    "frames": 6,
                    "evidence": ["frames"],
                    "phase": "targeted"
                }
            }))?;
            if let Some(reference) = observation.get_mut("observationRef") {
                reference["selectionIndex"] = json!(index);
                reference["rationale"] = json!(selection.rationale.trim());
                reference["trackId"] = json!(track.id.as_str());
                reference["timelineRange"] = json!({
                    "start": timeline_start,
                    "end": timeline_end,
                });
                if let Some(coarse_observation) = coarse_observation.as_ref() {
                    reference["coarseObservation"] = serde_json::to_value(coarse_observation)
                        .map_err(|error| {
                            McpError::new("SERIALIZATION_FAILED", error.to_string())
                        })?;
                }
            }
            let reference = observation.get("observationRef").cloned().ok_or_else(|| {
                McpError::new(
                    "HANDLER_FAILED",
                    "targeted observation omitted its reference",
                )
            })?;
            observation_refs.push(reference);
            targeted.push(observation);
            selection_results.push(json!({
                "index": index,
                "sourceId": selection.source_id,
                "start": selection.start,
                "end": selection.end,
                "timelineStart": timeline_start,
                "timelineEnd": timeline_end,
                "rationale": selection.rationale.trim(),
                "trackId": track.id.as_str(),
                "coarseObservationRef": coarse_observation
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?,
            }));
            next_timeline = timeline_end;
        }
        let observation = observation_refs.first().cloned();
        let command = json!({"type": "batch", "commands": commands});
        let validation = self.timeline_check(json!({}))?;
        let mut result = json!({
            "action": "workflow.roughcut",
            "status": "observed",
            "goal": goal,
            "selections": selection_results,
            "observationRefs": observation_refs.clone(),
            "targeted": targeted,
            "selectedDuration": selected_duration,
            "validation": validation,
            "retryPolicy": {"maxRetries": max_retries, "attempts": 0},
            "revision": snapshot.revision,
        });
        let timeline_valid = validation
            .get("valid")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !timeline_valid {
            result["status"] = json!("failed");
            result["failure"] = json!({
                "code": "TIMELINE_INVALID",
                "message": "rough-cut workflow stopped because the current timeline has error findings",
                "retryable": false,
                "attempts": 0,
                "maxRetries": max_retries,
            });
            return Ok(result);
        }
        let mut proposal_arguments = json!({
            "expectedRevision": snapshot.revision,
            "idempotencyKey": request_key,
            "command": command,
            "observationRefs": observation_refs,
        });
        if let Some(observation) = observation {
            proposal_arguments["observationRef"] = observation;
        }
        let proposal = self.stage_agent_edit(proposal_arguments)?;
        let pending = proposal
            .get("pendingProposal")
            .cloned()
            .unwrap_or(Value::Null);
        let proposal_id = pending
            .get("proposalId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                McpError::new("HANDLER_FAILED", "rough-cut proposal id was not returned")
            })?;
        let candidate_validation = self.timeline_check(json!({"proposalId": proposal_id}))?;
        result["validation"] = candidate_validation.clone();
        if !candidate_validation
            .get("valid")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let _ = self.runtime.reject_proposal(proposal_id);
            result["status"] = json!("failed");
            result["failure"] = json!({
                "code": "TIMELINE_INVALID",
                "message": "rough-cut candidate failed timeline validation and was not made reviewable",
                "retryable": false,
                "attempts": 1,
                "maxRetries": max_retries,
            });
            result["proposal"] = pending;
            return Ok(result);
        }
        let (capture_start, capture_end, capture_coverage) =
            proposal_capture_window(&timeline_ranges);
        let capture = match self.proposal_capture(json!({
            "proposalId": proposal_id,
            "start": capture_start,
            "duration": capture_end - capture_start,
            "count": 4,
            "draft": true,
        })) {
            Ok(capture) => capture,
            Err(error) => {
                let cleanup = self.runtime.reject_proposal(proposal_id);
                result["status"] = json!("failed");
                result["failure"] = json!({
                    "code": error.code,
                    "message": error.message,
                    "retryable": false,
                    "attempts": 1,
                    "maxRetries": max_retries,
                    "proposalCleanup": match cleanup {
                        Ok(_) => json!({"status": "rejected"}),
                        Err(cleanup_error) => json!({
                            "status": "cleanup_failed",
                            "code": cleanup_error.code,
                            "message": cleanup_error.message,
                        }),
                    },
                });
                result["proposal"] = pending;
                return Ok(result);
            }
        };
        result["status"] = json!("awaiting_review");
        result["captureCoverage"] = capture_coverage;
        result["proposal"] = pending;
        result["capture"] = capture;
        result["retryPolicy"] = json!({"maxRetries": max_retries, "attempts": 1});
        Ok(result)
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

    fn render_start(&self, draft: bool, arguments: &Value) -> Result<Value, McpError> {
        let range = render_range(arguments)?;
        let (job, project, output) = self
            .runtime
            .start_render_range(if draft { "preview" } else { "full" }, range)
            .map_err(McpError::from)?;
        let runtime = self.runtime.clone();
        let job_id = job.job_id.clone();
        let output_for_thread = output.clone();
        let range_for_thread = job.range.clone();
        let cancel_file = cut_runtime::render_cancel_path(runtime.directory(), &job_id);
        let cancel_file_for_cleanup = cancel_file.clone();
        thread::spawn(move || {
            let _ = runtime.update_job(&job_id, "running", None, None);
            let progress_runtime = runtime.clone();
            let progress_job_id = job_id.clone();
            let mut last_progress = -1.0_f64;
            let result = cut_runtime::run_render_with_cancel_file_progress(
                job_id.clone(),
                project,
                output_for_thread.clone(),
                draft,
                range_for_thread,
                Some(cancel_file),
                move |progress| {
                    if progress < 1.0 && progress - last_progress < 0.01 {
                        return;
                    }
                    last_progress = progress;
                    let _ = progress_runtime.update_job_with_progress(
                        &progress_job_id,
                        "running",
                        None,
                        None,
                        Some(progress),
                    );
                },
            );
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
                    if runtime
                        .render_job(&job_id)
                        .map(|job| job.status != "cancelled")
                        .unwrap_or(false)
                    {
                        let _ = runtime.update_job(&job_id, "failed", Some(error.message), None);
                    }
                }
            }
            let _ = fs::remove_file(cancel_file_for_cleanup);
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
            "progress": job.progress,
            "range": job.range,
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
            "progress": job.progress,
            "range": job.range,
        }))
    }

    fn tools_search(&self, arguments: &Value) -> Result<Value, McpError> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let mut all = self
            .profile
            .names()
            .iter()
            .filter(|name| {
                arguments.get("agentOnly").and_then(Value::as_bool) != Some(true)
                    || !AGENT_CANONICAL_ONLY_TOOLS.contains(name)
            })
            .filter_map(|name| {
                let (description, _, _) = tool_metadata(name);
                (query.is_empty()
                    || name.to_ascii_lowercase().contains(&query)
                    || description.to_ascii_lowercase().contains(&query))
                .then(|| json!({"name": self.definition(name).name, "description": description}))
            })
            .collect::<Vec<_>>();
        if !query.is_empty() {
            for kind in contracts::names() {
                let fields = contracts::schema(kind)
                    .unwrap_or_default()
                    .to_string()
                    .to_ascii_lowercase();
                if kind.contains(&query)
                    || kind.replace('_', " ").contains(&query)
                    || fields.contains(&query)
                {
                    all.push(json!({"name":self.definition("timeline.apply").name,"commandType":kind,
                        "description":format!("Canonical {kind} edit. Read tools.describe with this commandType for fields and an example.")}));
                }
            }
        }
        let items = all
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_offset = (offset.saturating_add(items.len()) < all.len())
            .then_some(offset.saturating_add(items.len()));
        Ok(
            json!({"items": items, "offset": offset, "limit": limit, "total": all.len(), "nextOffset": next_offset}),
        )
    }

    fn tools_describe(&self, arguments: &Value) -> Result<Value, McpError> {
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "tools.describe name is required"))?;
        let base = self
            .parse_name(name)
            .or_else(|| {
                self.profile
                    .names()
                    .contains(&name)
                    .then(|| name.to_owned())
            })
            .ok_or_else(|| McpError::new("UNKNOWN_TOOL", format!("unknown tool: {name}")))?;
        let mut definition = self.definition(&base);
        let mut example = example_for_tool(&base);
        if base == "timeline.apply" {
            if let Some(kind) = arguments.get("commandType") {
                let kind = kind.as_str().ok_or_else(|| {
                    McpError::new("INVALID_ARGUMENTS", "commandType must be a string")
                })?;
                let schema = contracts::schema(kind).ok_or_else(|| {
                    McpError::new("INVALID_ARGUMENTS", format!("unknown commandType: {kind}"))
                })?;
                definition.input_schema["properties"]["command"] = schema.clone();
                definition.input_schema["properties"]["commands"]["items"] = json!({"type":"object","description":"Each item follows the command contract shown above."});
                example = json!({"command": contracts::example(kind)});
            }
        } else if arguments.get("commandType").is_some() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "commandType applies only to timeline.apply",
            ));
        }
        let mut result = json!({"tool": definition, "example": example});
        if base == "timeline.apply" {
            result["commandTypes"] = json!(contracts::names());
        }
        Ok(result)
    }

    fn skills_list(&self, arguments: &Value) -> Result<Value, McpError> {
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let skills = self.skills.iter().skip(offset).take(limit).map(|skill| json!({"name": skill.name, "description": skill.description.chars().take(200).collect::<String>()})).collect::<Vec<_>>();
        Ok(
            json!({"skills": skills, "offset": offset, "limit": limit, "total": self.skills.len(), "nextOffset": (offset.saturating_add(skills.len()) < self.skills.len()).then_some(offset.saturating_add(skills.len()))}),
        )
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
        if AGENT_CANONICAL_ONLY_TOOLS.contains(&name) {
            return Err(agent_canonical_bypass_error(name));
        }
        let router = CutMcpRouter {
            dir: self.dir.clone(),
            runtime: self.runtime.clone(),
            skills: self.skills.clone(),
            profile: self.profile,
            namespace: String::new(),
        };
        if name == "tools.search" {
            let mut arguments = arguments;
            arguments["agentOnly"] = json!(true);
            return router
                .tools_search(&arguments)
                .map_err(|error| error.to_string());
        }
        if name == "tools.describe"
            && arguments
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| AGENT_CANONICAL_ONLY_TOOLS.contains(&name))
        {
            return Err(agent_canonical_bypass_error(
                arguments["name"].as_str().unwrap_or_default(),
            ));
        }
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

fn agent_canonical_bypass_error(name: &str) -> String {
    match name {
        "media.import" => {
            "AGENT_CANONICAL_BYPASS: media.import is human-only; use media.asset with import:true for a staged local-artifact proposal".into()
        }
        "media.fetch" => {
            "AGENT_CANONICAL_BYPASS: media.fetch is human-only; use media.probe for project media, then media.asset for an existing local artifact".into()
        }
        "timeline.undo" | "timeline.redo" => {
            format!(
                "AGENT_CANONICAL_BYPASS: {name} is human-only; use timeline.apply for an explicit staged edit"
            )
        }
        _ => format!("AGENT_CANONICAL_BYPASS: {name} is unavailable to the agent"),
    }
}

impl CutMcpRouter {
    /// Stage an embedded or external agent edit for explicit human review.
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
        let observation_values = arguments
            .get("observationRefs")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| {
                arguments
                    .get("observationRef")
                    .cloned()
                    .map(|value| vec![value])
            })
            .unwrap_or_default();
        let mut observations = Vec::with_capacity(observation_values.len());
        for value in observation_values {
            let observation = serde_json::from_value::<cut_runtime::ObservationReference>(value)
                .map_err(|error| {
                    McpError::new(
                        "INVALID_ARGUMENTS",
                        format!("invalid observationRef: {error}"),
                    )
                })?;
            validate_observation_reference(&self.dir, &project, base_revision, &observation)?;
            observations.push(observation);
        }
        let observation = observations.first().cloned().map(|_| {
            arguments
                .get("observationRef")
                .cloned()
                .unwrap_or_else(|| serde_json::to_value(&observations[0]).unwrap_or(Value::Null))
        });
        let mut transaction = json!({
            "idempotencyKey": request_key,
            "commands": [command],
        });
        if let Some(observation) = observation {
            transaction["observation"] = observation;
        }
        if observations.len() > 1 {
            transaction["observations"] = serde_json::to_value(&observations)
                .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
        }
        let next = self
            .runtime
            .stage_agent_proposal(&request_key, base_revision, vec![transaction])
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
            version: env!("CARGO_PKG_VERSION").into(),
            instructions: "Read the structural project with project.outline or timeline.query, inspect deterministic findings with timeline.check or composition.check, discover local HTML templates with composition.catalog, use composition.preview/composition.render only through the configured deterministic external bridge, use media.probe action observe for one bounded cached evidence envelope (frames, filmstrip, waveform, transcript, or listen), capture exact bounded previews with timeline.capture, use proposal.capture for before/after contact sheets, apply canonical v2 commands with timeline.apply, render canonical timelines with render.preview/render.start, and load playbooks with skills.read. Shell access is unavailable.".into(),
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

fn count_outline_nodes(node: &OutlineNode) -> usize {
    1 + node.children.iter().map(count_outline_nodes).sum::<usize>()
}

fn find_outline_node<'a>(node: &'a OutlineNode, path: &str) -> Option<&'a OutlineNode> {
    if node.path == path {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_outline_node(child, path))
}

fn find_outline_node_mut<'a>(node: &'a mut OutlineNode, path: &str) -> Option<&'a mut OutlineNode> {
    if node.path == path {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_outline_node_mut(child, path))
}

fn truncate_outline_nodes(node: &mut OutlineNode, budget: &mut usize) {
    if *budget == 0 {
        node.children.clear();
        return;
    }
    *budget -= 1;
    let mut kept = Vec::new();
    for mut child in node.children.drain(..) {
        if *budget == 0 {
            break;
        }
        truncate_outline_nodes(&mut child, budget);
        kept.push(child);
    }
    node.children = kept;
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
        "results.read" => (
            "Read a page or JSON pointer from an archived tool result without repeating the operation. Historical results may be evicted from the bounded cache.",
            json!({"type":"object","properties":{"resultId":{"type":"string","minLength":64,"maxLength":64},"pointer":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":4096}},"required":["resultId"],"additionalProperties":false}),
            read_only,
        ),
        "tools.search" => (
            "Search available tools by name or description; returns compact metadata only.",
            json!({"type":"object","properties":{"query":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}),
            read_only,
        ),
        "tools.describe" => (
            "Read the exact input schema and a representative example for one tool.",
            json!({"type":"object","properties":{"name":{"type":"string","minLength":1},"commandType":{"type":"string"}},"required":["name"],"additionalProperties":false}),
            read_only,
        ),
        "project.inspect" => (
            "Inspect the current project summary, full v2 project, or structural outline.",
            json!({
                "type": "object",
                "properties": {"include": {"enum": ["summary", "full", "outline"]}},
                "additionalProperties": false
            }),
            read_only,
        ),
        "project.outline" => (
            "Read the structural sequence, track, clip, caption, effect, and marker outline.",
            json!({
                "type": "object",
                "properties": {"kind":{"type":"string","enum":["sequence","track","clip","caption","effect","marker"]},"trackId":{"type":"string"},"nodePath":{"type":"string"},"childOffset":{"type":"integer","minimum":0},"childLimit":{"type":"integer","minimum":1,"maximum":500},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":500},"maxNodes":{"type":"integer","minimum":2,"maximum":5000}},
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
                    "clipId": {"type": "string"},
                    "start": {"type": "number", "minimum": 0},
                    "end": {"type": "number", "minimum": 0},
                    "offset": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500}
                },
                "additionalProperties": false
            }),
            read_only,
        ),
        "timeline.check" => (
            "Check source availability, timeline ranges, visible video coverage, compiler support, and live/offline preview parity.",
            json!({
                "type": "object",
                "properties": {
                    "proposalId": {"type": "string", "minLength": 1}
                },
                "additionalProperties": false
            }),
            read_only,
        ),
        "composition.check" => (
            "Statically inspect a local HTML motion composition sidecar without executing HTML, scripts, a browser, CLI, or network requests. Returns bounded metadata, typed variable bindings, optional manifest/instance validation, timed elements, assets, nested references and findings; never changes the canonical project or writes a result cache. The entry defaults to index.html and must be a project-relative regular file without symlinks or parent components. An optional schemaVersion 1 manifest declares template identity, explicit width/height/fps/duration output, and typed text/color/number/boolean/enum/image/video/audio slots. An optional instance sidecar contains sparse overrides and must match the manifest template id/version. HTML data-var-* bindings must refer to declared slots. Root: data-composition-id, data-composition with id, or id=composition; width/height/fps/duration use data-* attributes (plain attributes also accepted). valid means no errors; deterministic means the entry scan completed with no known determinism violations or uninspected executable dependencies, not a runtime guarantee. Dependencies are only checked for local availability, never read recursively. Error findings cover invalid declarations, manifest/override mismatches, and unsafe behavior; warnings cover uninspected dependencies. Requires Unix secure file traversal.",
            composition::input_schema(),
            read_only,
        ),
        "composition.catalog" => (
            "Discover bounded local HTML compositions and their manifest/meta/storyboard metadata without executing scripts or changing canonical state.",
            composition_catalog::input_schema(),
            read_only,
        ),
        "composition.preview" => (
            "Render one deterministic composition frame through the configured external JSON bridge and cache the verified PNG artifact.",
            composition_render::input_schema(composition_render::Mode::Preview),
            read_only,
        ),
        "composition.render" => (
            "Render a deterministic composition through the configured external JSON bridge and cache the verified MP4 artifact.",
            composition_render::input_schema(composition_render::Mode::Render),
            read_only,
        ),
        "timeline.capture" => (
            "Capture an exact, bounded offline-compiler preview of the current timeline.",
            json!({
                "type": "object",
                "properties": {
                    "start": {"type": "number", "minimum": 0},
                    "end": {"type": "number", "exclusiveMinimum": 0},
                    "duration": {"type": "number", "exclusiveMinimum": 0},
                    "times": {"type": "array", "minItems": 1, "maxItems": MAX_CAPTURE_TIMES, "items": {"type": "number", "minimum": 0}},
                    "count": {"type": "integer", "minimum": 1, "maximum": 32},
                    "maxWidth": {"type": "integer", "minimum": 1},
                    "columns": {"type": "integer", "minimum": 1, "maximum": 16},
                    "format": {"enum": ["filmstrip", "contact_sheet", "frame", "frames"]},
                    "includeBase64": {"type": "boolean"},
                    "draft": {"type": "boolean"},
                    "opts": {"type": "object"}
                },
                "additionalProperties": false
            }),
            read_only,
        ),
        "timeline.apply" => {
            let schema = json!({
                "type": "object",
                "properties": {
                    "expectedRevision": {"type": "integer", "minimum": 0},
                    "idempotencyKey": {"type": "string"},
                    "command": {"type":"object","description":"Read tools.describe with commandType for the exact canonical command contract."},
                    "commands": {"type":"array","minItems":1,"maxItems":256,"items":{"type":"object"}},
                    "op": {"type": "object"},
                    "ops": {"type": "array", "minItems": 1,"maxItems":256, "items": {"type": "object"}},
                    "observationRef": {"type": "object"}
                },
                "additionalProperties": false
            });
            ("Stage one atomic canonical v2 command or legacy operation for human review through cutd. Canonical state changes only on acceptance.", schema, mutation)
        },
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
        "proposal.capture" => (
            "Capture before and after contact sheets for the pending proposal using the offline compiler.",
            json!({
                "type": "object",
                "properties": {
                    "proposalId": {"type": "string", "minLength": 1},
                    "start": {"type": "number", "minimum": 0},
                    "end": {"type": "number", "exclusiveMinimum": 0},
                    "duration": {"type": "number", "exclusiveMinimum": 0},
                    "times": {"type": "array", "minItems": 1, "maxItems": MAX_CAPTURE_TIMES, "items": {"type": "number", "minimum": 0}},
                    "count": {"type": "integer", "minimum": 1, "maximum": 32},
                    "maxWidth": {"type": "integer", "minimum": 1},
                    "columns": {"type": "integer", "minimum": 1, "maximum": 16},
                    "includeBase64": {"type": "boolean"},
                    "format": {"enum": ["filmstrip", "contact_sheet", "frame", "frames"]}
                },
                "required": ["proposalId"],
                "additionalProperties": false
            }),
            read_only,
        ),
        "media.probe" => (
            "Probe media metadata, silence, scenes, exact frames, filmstrips, waveform, transcription, selected-range listening, or one bounded multi-modal observation envelope.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"enum": ["info", "silence", "scenes", "frames", "frame", "filmstrip", "contact_sheet", "waveform", "transcribe", "listen", "observe"]},
                    "src": {"type": "string", "minLength": 1},
                    "opts": {
                        "type": "object",
                        "properties": {
                            "noiseDb": {"type": "number"},
                            "minDuration": {"type": "number", "exclusiveMinimum": 0},
                            "threshold": {"type": "number", "minimum": 0, "maximum": 1},
                            "frames": {"type": "integer", "minimum": 1, "maximum": 32},
                            "times": {"type": "array", "minItems": 1, "maxItems": 128, "items": {"type": "number", "minimum": 0}},
                            "time": {"type": "number", "minimum": 0},
                            "maxWidth": {"type": "integer", "minimum": 1},
                            "columns": {"type": "integer", "minimum": 1, "maximum": 16},
                            "buckets": {"type": "integer", "minimum": 1, "maximum": 4096},
                            "includeBase64": {"type": "boolean"},
                            "includeImage": {"type": "boolean"},
                            "imageWidth": {"type": "integer", "minimum": 1},
                            "imageHeight": {"type": "integer", "minimum": 1},
                            "start": {"type": "number", "minimum": 0},
                            "end": {"type": "number", "exclusiveMinimum": 0},
                            "duration": {"type": "number", "exclusiveMinimum": 0},
                            "language": {"type": "string", "minLength": 1},
                            "backend": {"enum": ["local", "whisper.cpp", "whisper_cpp", "provider"]},
                            "provider": {"type": "string", "minLength": 1},
                            "model": {"type": "string", "minLength": 1},
                            "modelVersion": {"type": "string", "minLength": 1},
                            "backendVersion": {"type": "string", "minLength": 1},
                            "evidence": {"type": "array", "minItems": 1, "maxItems": 5, "items": {"enum": ["frames", "filmstrip", "waveform", "transcript", "listen"]}},
                            "phase": {"enum": ["coarse", "targeted"]}
                        },
                        "additionalProperties": false
                    }
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
        "media.asset" => (
            "Register an existing local generated artifact with request and content provenance; optionally import it through cutd.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"enum": ["register"]},
                    "path": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "minLength": 1},
                    "requestFingerprint": {"type": "string", "minLength": 1},
                    "prompt": {"type": "string"},
                    "provider": {"type": "string"},
                    "model": {"type": "string"},
                    "provenance": {"type": "object"},
                    "import": {"type": "boolean"},
                    "replaceClipId": {"type": "string", "minLength": 1},
                    "idempotencyKey": {"type": "string", "minLength": 1},
                    "metadata": {"type": "object"},
                    "overlay": {"type": "object"}
                },
                "required": ["action", "path"],
                "additionalProperties": false
            }),
            mutation,
        ),
        "workflow.roughcut" => (
            "Execute the agent's observed source-range selections as one typed rough-cut proposal; call media.probe first and provide rationale for every range.",
            json!({
                "type": "object",
                "properties": {
                    "goal": {"type": "string", "minLength": 1},
                    "idempotencyKey": {"type": "string", "minLength": 1},
                    "sourceId": {"type": "string", "minLength": 1},
                    "source": {"type": "string", "minLength": 1},
                    "duration": {"type": "number", "exclusiveMinimum": 0, "maximum": MAX_OBSERVATION_RANGE_SECONDS},
                    "maxRetries": {"type": "integer", "minimum": 0, "maximum": 2},
                    "selections": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 16,
                        "items": {
                            "type": "object",
                            "properties": {
                                "sourceId": {"type": "string", "minLength": 1},
                                "start": {"type": "number", "minimum": 0},
                                "end": {"type": "number", "exclusiveMinimum": 0},
                                "timelineStart": {"type": "number", "minimum": 0},
                                "trackId": {"type": "string", "minLength": 1},
                                "clipId": {"type": "string", "minLength": 1},
                                "speed": {"type": "number", "exclusiveMinimum": 0},
                                "rationale": {"type": "string", "minLength": 1},
                                "observationRef": {"type": "object"}
                            },
                            "required": ["sourceId", "start", "end", "rationale"],
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
            mutation,
        ),
        "render.preview" | "render.start" => (
            "Queue a durable draft or full render job, optionally bounded to a project-time range.",
            json!({"type": "object", "properties": {"draft": {"type": "boolean"}, "start": {"type": "number", "minimum": 0}, "duration": {"type": "number", "exclusiveMinimum": 0}}, "additionalProperties": false}),
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
        "skills.list" => {
            let schema = skill_list_tool_schema();
            let mut input = schema.get("inputSchema").cloned().unwrap_or_else(|| json!({"type":"object"}));
            input["properties"] = json!({"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}});
            ("List installed playbooks as compact names and descriptions.", input, read_only)
        }
        _ => (
            "Unknown capability.",
            json!({"type": "object", "additionalProperties": false}),
            read_only,
        ),
    }
}

fn example_for_tool(name: &str) -> Value {
    match name {
        "composition.check" => json!({"entry":"index.html"}),
        "composition.catalog" => json!({}),
        "composition.preview" => {
            json!({"entry":"index.html","manifest":"composition.manifest.json","frame":0})
        }
        "composition.render" => {
            json!({"entry":"index.html","manifest":"composition.manifest.json"})
        }
        "timeline.query" => json!({"trackId":"video-1","limit":50}),
        "timeline.apply" => {
            json!({"commands":[{"type":"update_clip","clipId":"clip-1","patch":{}}]})
        }
        "skills.read" => json!({"name":"shorts"}),
        "skills.list" => json!({}),
        _ => json!({}),
    }
}

fn canonical_command(project: &Project, arguments: &Value) -> Result<Command, McpError> {
    let mut pending = Vec::new();
    for key in ["command", "op"] {
        if let Some(value) = arguments.get(key) {
            pending.push((value, 0usize));
        }
    }
    for key in ["commands", "ops"] {
        if let Some(values) = arguments.get(key).and_then(Value::as_array) {
            if values.len() > 256 {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "A batch may contain at most 256 command entries",
                ));
            }
            pending.extend(values.iter().map(|value| (value, 0)));
        }
    }
    let mut count = 0usize;
    while let Some((value, depth)) = pending.pop() {
        count += 1;
        if count > 256 || depth > 16 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "A batch may contain at most 256 command entries and 16 nested levels",
            ));
        }
        if value.get("type").and_then(Value::as_str) == Some("batch") {
            if let Some(values) = value.get("commands").and_then(Value::as_array) {
                if values.len() > 256 {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "A batch may contain at most 256 command entries",
                    ));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
        }
    }
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

fn probe_options(arguments: &Value) -> Result<ProbeOptions, McpError> {
    let value = arguments.get("opts").cloned().unwrap_or_else(|| json!({}));
    serde_json::from_value(value).map_err(|error| {
        McpError::new(
            "INVALID_ARGUMENTS",
            format!("invalid media.probe opts: {error}"),
        )
    })
}

fn observation_evidence(options: &ProbeOptions) -> Result<Vec<ObservationEvidence>, McpError> {
    let evidence = options
        .evidence
        .clone()
        .unwrap_or_else(|| vec![ObservationEvidence::Filmstrip]);
    if evidence.is_empty() || evidence.len() > MAX_OBSERVATION_EVIDENCE {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            format!(
                "media.probe observe evidence must contain 1..={MAX_OBSERVATION_EVIDENCE} items"
            ),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for kind in &evidence {
        if !seen.insert(*kind) {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("media.probe observe evidence contains duplicate {:?}", kind),
            ));
        }
    }
    Ok(evidence)
}

fn observation_range(
    options: &ProbeOptions,
    metadata: &cut_engine::ProbeInfo,
) -> Result<cut_engine::TimeRange, McpError> {
    let start = options.start.ok_or_else(|| {
        McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe observe requires opts.start",
        )
    })?;
    if options.end.is_some() && options.duration.is_some() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe observe accepts opts.end or opts.duration, not both",
        ));
    }
    let end = if let Some(end) = options.end {
        end
    } else if let Some(duration) = options.duration {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.probe observe duration must be finite and positive",
            ));
        }
        start + duration
    } else {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe observe requires opts.end or opts.duration",
        ));
    };
    let range = cut_engine::TimeRange::new(start, end)
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
    if range.duration() > MAX_OBSERVATION_RANGE_SECONDS + 1e-9 {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            format!(
                "media.probe observe range exceeds the {MAX_OBSERVATION_RANGE_SECONDS:.0}s limit"
            ),
        ));
    }
    if let Some(duration) = metadata.duration {
        if !duration.is_finite() || duration <= 0.0 || range.end > duration + 1e-6 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("media.probe observe range must be inside source duration {duration:.3}s"),
            ));
        }
    }
    Ok(range)
}

fn observation_times(
    options: &ProbeOptions,
    range: cut_engine::TimeRange,
) -> Result<Vec<f64>, McpError> {
    if let Some(times) = options.times.as_ref() {
        if times.is_empty() || times.len() > MAX_OBSERVATION_TIMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("media.probe observe times must contain 1..={MAX_OBSERVATION_TIMES} items"),
            ));
        }
        if times
            .iter()
            .any(|time| !time.is_finite() || *time < range.start || *time > range.end)
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.probe observe times must be finite and inside the selected range",
            ));
        }
        return Ok(times.clone());
    }
    let count = options.frames.unwrap_or(8).clamp(1, MAX_OBSERVATION_TIMES);
    let duration = range.duration();
    Ok((0..count)
        .map(|index| range.start + duration * (index as f64 + 0.5) / count as f64)
        .collect())
}

fn transcription_range(
    options: &ProbeOptions,
    metadata: &cut_engine::ProbeInfo,
    listen: bool,
) -> Result<cut_engine::TimeRange, McpError> {
    if listen && options.start.is_none() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe listen requires opts.start",
        ));
    }
    if listen && options.end.is_none() && options.duration.is_none() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe listen requires opts.end or opts.duration",
        ));
    }
    if options.end.is_some() && options.duration.is_some() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe transcription accepts opts.end or opts.duration, not both",
        ));
    }
    let start = options.start.unwrap_or(0.0);
    let end = if let Some(end) = options.end {
        end
    } else if let Some(duration) = options.duration {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "media.probe transcription duration must be finite and positive",
            ));
        }
        start + duration
    } else if let Some(duration) = metadata.duration {
        duration
    } else {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "media.probe transcribe requires opts.end or a source duration",
        ));
    };
    let range = cut_engine::TimeRange::new(start, end)
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
    let max = if listen {
        cut_engine::MAX_LISTEN_RANGE_SECONDS
    } else {
        cut_engine::MAX_TRANSCRIPTION_RANGE_SECONDS
    };
    if range.duration() > max + 1e-9 {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            format!(
                "media.probe {0} range exceeds the {max:.0}s limit",
                if listen { "listen" } else { "transcribe" }
            ),
        ));
    }
    if let Some(duration) = metadata.duration {
        if !duration.is_finite() || duration <= 0.0 || range.end > duration + 1e-6 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!(
                    "media.probe transcription range must be inside source duration {duration:.3}s"
                ),
            ));
        }
    }
    Ok(range)
}

fn transcription_options(
    directory: &Path,
    options: &ProbeOptions,
    range: cut_engine::TimeRange,
) -> Result<cut_engine::TranscriptionOptions, McpError> {
    let backend = match options.backend.as_deref().unwrap_or("local") {
        "local" | "whisper.cpp" | "whisper_cpp" => {
            cut_engine::TranscriptionBackend::LocalWhisperCpp
        }
        "provider" => cut_engine::TranscriptionBackend::ExplicitProvider {
            name: options
                .provider
                .clone()
                .unwrap_or_else(|| "provider".into()),
        },
        other => {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("unknown transcription backend {other:?}; expected local or provider"),
            ));
        }
    };
    let mut result = cut_engine::TranscriptionOptions::new(range);
    result.backend = backend;
    result.language = options.language.clone();
    result.model = options.model.as_deref().map(|model| {
        let path = Path::new(model);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            directory.join(path)
        }
    });
    result.model_version = options.model_version.clone();
    result.backend_version = options.backend_version.clone();
    Ok(result)
}

fn capture_options(arguments: &Value) -> Result<CaptureOptions, McpError> {
    let mut object = arguments.as_object().cloned().unwrap_or_default();
    if let Some(opts) = arguments.get("opts").and_then(Value::as_object) {
        object.extend(opts.iter().map(|(key, value)| (key.clone(), value.clone())));
    }
    serde_json::from_value(Value::Object(object)).map_err(|error| {
        McpError::new(
            "INVALID_ARGUMENTS",
            format!("invalid timeline.capture options: {error}"),
        )
    })
}

fn capture_times(
    options: &CaptureOptions,
    start: f64,
    end: f64,
    format: &str,
) -> Result<Vec<f64>, McpError> {
    if let Some(times) = options.times.as_ref() {
        if times.is_empty() || times.len() > MAX_CAPTURE_TIMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("timeline.capture times must contain 1..={MAX_CAPTURE_TIMES} items"),
            ));
        }
        if times
            .iter()
            .any(|time| !time.is_finite() || *time < start || *time > end)
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "timeline.capture times must be finite and inside the selected range",
            ));
        }
        return Ok(times.clone());
    }
    let default_count = if matches!(format, "frame") { 1 } else { 8 };
    let count = options.count.unwrap_or(default_count).clamp(1, 32);
    let length = end - start;
    Ok((0..count)
        .map(|index| start + length * (index as f64 + 0.5) / count as f64)
        .collect())
}

fn project_duration_from_value(project: &Value) -> f64 {
    cut_engine::compile::timeline_duration_v2(project).unwrap_or(0.0)
}

fn roughcut_selected_duration(selections: &[RoughcutSelection]) -> Result<f64, McpError> {
    let mut total = 0.0;
    for (index, selection) in selections.iter().enumerate() {
        let speed = selection.speed.unwrap_or(1.0);
        let duration = (selection.end - selection.start) / speed;
        if !selection.start.is_finite()
            || !selection.end.is_finite()
            || selection.end <= selection.start
            || !speed.is_finite()
            || speed <= 0.0
            || !duration.is_finite()
            || duration <= 0.0
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("workflow.roughcut selection {index} has an invalid duration"),
            ));
        }
        total += duration;
        if !total.is_finite() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "workflow.roughcut selected duration is not finite",
            ));
        }
    }
    Ok(total)
}

fn proposal_capture_window(ranges: &[(f64, f64)]) -> (f64, f64, Value) {
    let min_start = ranges
        .iter()
        .map(|(start, _)| *start)
        .fold(f64::INFINITY, f64::min);
    let max_end = ranges
        .iter()
        .map(|(_, end)| *end)
        .fold(f64::NEG_INFINITY, f64::max);
    let full_span = max_end - min_start;
    let (start, end, mode) = if full_span <= MAX_OBSERVATION_RANGE_SECONDS {
        (min_start, max_end, "full")
    } else {
        let &(start, end) = ranges
            .iter()
            .max_by(|left, right| {
                (left.1 - left.0)
                    .partial_cmp(&(right.1 - right.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("workflow has at least one selection");
        (
            start,
            (start + MAX_OBSERVATION_RANGE_SECONDS).min(end),
            "representative",
        )
    };
    let coverage = ranges
        .iter()
        .enumerate()
        .map(|(index, (range_start, range_end))| {
            json!({
                "selectionIndex": index,
                "timelineRange": {"start": range_start, "end": range_end},
                "fullyCovered": *range_start >= start && *range_end <= end,
            })
        })
        .collect::<Vec<_>>();
    (
        start,
        end,
        json!({
            "mode": mode,
            "capturedRange": {"start": start, "end": end},
            "requestedRanges": coverage,
            "fullyCovered": mode == "full",
        }),
    )
}

fn render_range(arguments: &Value) -> Result<Option<cut_runtime::RenderRange>, McpError> {
    let start = arguments.get("start").and_then(Value::as_f64);
    let duration = arguments.get("duration").and_then(Value::as_f64);
    match (start, duration) {
        (None, None) => Ok(None),
        (Some(start), Some(duration)) => cut_runtime::RenderRange::new(start, duration)
            .map(Some)
            .map_err(McpError::from),
        _ => Err(McpError::new(
            "INVALID_ARGUMENTS",
            "render start and duration must be provided together",
        )),
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

fn add_inline_base64(object: &mut Value, path_key: &str, base64_key: &str) -> Result<(), McpError> {
    let path = object
        .get(path_key)
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::new("HANDLER_FAILED", "artifact path is missing"))?
        .to_owned();
    let metadata = fs::metadata(&path).map_err(io_error)?;
    if metadata.len() > MAX_INLINE_ARTIFACT_BYTES as u64 {
        return Err(McpError::new(
            "RESULT_TOO_LARGE",
            format!("inline artifact exceeds {MAX_INLINE_ARTIFACT_BYTES} bytes"),
        ));
    }
    let data = fs::read(path).map_err(io_error)?;
    object[base64_key] = json!(base64::engine::general_purpose::STANDARD.encode(data));
    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<String, McpError> {
    let mut file = fs::File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut fingerprint = String::with_capacity(64);
    for byte in digest.finalize() {
        use std::fmt::Write as _;
        let _ = write!(fingerprint, "{byte:02x}");
    }
    Ok(fingerprint)
}

fn write_asset_manifest(path: &Path, manifest: &GeneratedAssetManifest) -> Result<(), McpError> {
    let parent = path
        .parent()
        .ok_or_else(|| McpError::new("HANDLER_FAILED", "asset manifest has no parent directory"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| McpError::new("SERIALIZATION_FAILED", error.to_string()))?;
    let temp = parent.join(format!("assets.json.tmp-{}", std::process::id()));
    let mut file = fs::File::create(&temp).map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::rename(temp, path).map_err(io_error)?;
    Ok(())
}

fn upsert_asset_manifest(
    path: &Path,
    record: &GeneratedAssetRecord,
) -> Result<(bool, GeneratedAssetRecord), McpError> {
    let parent = path
        .parent()
        .ok_or_else(|| McpError::new("HANDLER_FAILED", "asset manifest has no parent directory"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let lock_path = parent.join("assets.json.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(io_error)?;
    lock.lock_exclusive().map_err(io_error)?;

    let mut manifest = if path.is_file() {
        let bytes = fs::read(path).map_err(io_error)?;
        serde_json::from_slice::<GeneratedAssetManifest>(&bytes).map_err(|error| {
            McpError::new(
                "HANDLER_FAILED",
                format!("generated asset manifest is invalid: {error}"),
            )
        })?
    } else {
        GeneratedAssetManifest {
            schema_version: 1,
            assets: Vec::new(),
        }
    };
    if manifest.schema_version != 1 {
        return Err(McpError::new(
            "HANDLER_FAILED",
            "generated asset manifest version is unsupported",
        ));
    }
    if let Some(existing) = manifest.assets.iter().find(|existing| {
        existing.request_fingerprint == record.request_fingerprint
            && existing.content_fingerprint == record.content_fingerprint
    }) {
        return Ok((true, existing.clone()));
    }
    if let Some(existing) = manifest
        .assets
        .iter()
        .find(|existing| existing.request_fingerprint == record.request_fingerprint)
    {
        return Err(McpError::new(
            "ASSET_REQUEST_CONFLICT",
            "request fingerprint is already bound to a different content fingerprint",
        )
        .with_details(json!({
            "requestFingerprint": record.request_fingerprint,
            "existingAssetId": existing.asset_id,
            "existingContentFingerprint": existing.content_fingerprint,
            "requestedContentFingerprint": record.content_fingerprint,
            "existingPath": existing.path,
            "requestedPath": record.path,
        })));
    }
    manifest.assets.push(record.clone());
    manifest
        .assets
        .sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
    write_asset_manifest(path, &manifest)?;
    Ok((false, record.clone()))
}

fn stable_operation_key(kind: &str, identity: &Value) -> String {
    let key = cut_engine::derivatives::cache_key_for_values(
        "mcp-operation",
        &json!({"kind": kind}),
        identity,
    );
    format!("{kind}-{}", &key[..24])
}

fn staged_import_ids(pending: &Value) -> (Value, Value) {
    let commands = pending
        .get("command")
        .and_then(|command| command.get("commands"))
        .and_then(Value::as_array);
    let source_id = commands
        .into_iter()
        .flatten()
        .find(|command| command.get("type").and_then(Value::as_str) == Some("add_source"))
        .and_then(|command| command.get("source"))
        .and_then(|source| source.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    let clip_id = commands
        .into_iter()
        .flatten()
        .find(|command| command.get("type").and_then(Value::as_str) == Some("add_clip"))
        .and_then(|command| command.get("clip"))
        .and_then(|clip| clip.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    (source_id, clip_id)
}

fn resolve_media_src(
    dir: &Path,
    input: &str,
    project: Option<&Project>,
) -> Result<String, McpError> {
    if let Some(source) = project.and_then(|project| {
        project
            .sources
            .iter()
            .find(|source| source.id.as_str() == input)
    }) {
        let path = resolve_import_path(dir, &source.path)?;
        return Ok(path.to_string_lossy().into_owned());
    }
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

fn validate_observation_reference(
    dir: &Path,
    project: &Project,
    base_revision: u64,
    observation: &cut_runtime::ObservationReference,
) -> Result<(), McpError> {
    observation
        .validate()
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error))?;
    if let Some(observation_revision) = observation.revision {
        if observation_revision != base_revision {
            return Err(McpError::new(
                "STALE_OBSERVATION",
                format!(
                    "observation belongs to revision {observation_revision}, not {base_revision}"
                ),
            ));
        }
    }
    let observed_path = Path::new(&observation.source_path)
        .canonicalize()
        .map_err(|error| {
            McpError::new(
                "STALE_OBSERVATION",
                format!("observation source is unavailable: {error}"),
            )
        })?;
    let source = project
        .sources
        .iter()
        .find(|source| {
            let source_path = resolve_project_source_path(dir, &source.path)
                .canonicalize()
                .ok();
            source.id.as_str() == observation.source_ref
                || source_path.as_ref() == Some(&observed_path)
        })
        .ok_or_else(|| {
            McpError::new(
                "STALE_OBSERVATION",
                "observation source is not present in the current project",
            )
        })?;
    let observation_revision = observation.revision.ok_or_else(|| {
        McpError::new(
            "STALE_OBSERVATION",
            "project-bound observation must include its project revision",
        )
    })?;
    if observation_revision != base_revision {
        return Err(McpError::new(
            "STALE_OBSERVATION",
            format!("observation belongs to revision {observation_revision}, not {base_revision}"),
        ));
    }
    let source_path = resolve_import_path(dir, &source.path).map_err(|error| {
        McpError::new(
            "STALE_OBSERVATION",
            format!("current project source is unavailable: {}", error.message),
        )
    })?;
    if source_path != observed_path {
        return Err(McpError::new(
            "STALE_OBSERVATION",
            "observation source path does not match the project source",
        ));
    }
    let identity = SourceIdentity::from_path(&observed_path).map_err(engine_error)?;
    if identity.path != observation.source_identity.path
        || identity.size != observation.source_identity.size
        || identity.modified_ns != observation.source_identity.modified_ns
    {
        return Err(McpError::new(
            "STALE_OBSERVATION",
            "observation source identity no longer matches the media file",
        ));
    }
    if source
        .duration
        .is_some_and(|duration| observation.range.end > duration + 1e-9)
    {
        return Err(McpError::new(
            "STALE_OBSERVATION",
            "observation range is outside the project source duration",
        ));
    }
    Ok(())
}

fn is_contained(root: &Path, candidate: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    candidate.starts_with(root)
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
    use cut_agent::{
        AgentSession, LlmProvider, OpenAiApiMode, ProviderChunk, ProviderError, ProviderName,
        ProviderRequest,
    };
    use std::io::Cursor;
    use std::process::{Command as ProcessCommand, Stdio};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tempfile::TempDir;

    struct SequencedAgentProvider {
        responses: Mutex<Vec<Vec<ProviderChunk>>>,
    }

    impl LlmProvider for SequencedAgentProvider {
        fn name(&self) -> ProviderName {
            ProviderName::Anthropic
        }

        fn default_model(&self) -> &'static str {
            cut_agent::DEFAULT_ANTHROPIC_MODEL
        }

        fn stream(&self, _request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError> {
            self.responses
                .lock()
                .expect("provider responses")
                .drain(..1)
                .next()
                .ok_or_else(|| ProviderError {
                    code: "TEST".into(),
                    message: "no response".into(),
                })
        }
    }

    #[test]
    fn external_catalog_hides_bypasses_and_dispatch_rejects_them() {
        for profile in [McpProfile::Core, McpProfile::Full] {
            let root = TempDir::new().expect("temp dir");
            let mut router = CutMcpRouter::new(root.path(), profile, None).expect("router");
            for namespace in ["", "cut"] {
                router.namespace = namespace.into();
                let listed = router.list_tools();
                let search = router.call(
                    &router.definition("tools.search").name,
                    json!({"limit":100}),
                );
                assert_eq!(search["ok"], true);
                for name in AGENT_CANONICAL_ONLY_TOOLS {
                    let presented = router.definition(name).name;
                    assert!(!listed.iter().any(|tool| tool.name == presented));
                    assert!(!search["result"]["items"]
                        .as_array()
                        .expect("search items")
                        .iter()
                        .any(|tool| tool["name"] == presented));
                    for attempted_name in [*name, presented.as_str()] {
                        let result = router.call(attempted_name, json!({}));
                        assert_eq!(result["ok"], false);
                        assert_eq!(result["error"]["code"], "AGENT_CANONICAL_BYPASS");
                        let description = router.call(
                            &router.definition("tools.describe").name,
                            json!({"name":attempted_name}),
                        );
                        assert_eq!(description["ok"], false);
                        assert_eq!(description["error"]["code"], "UNKNOWN_TOOL");
                    }
                }
            }
        }
    }

    #[test]
    fn external_timeline_apply_stages_without_changing_canonical_state() {
        let cutd_binary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cutd");
        assert!(cutd_binary.is_file(), "cutd test binary is required");
        for profile in [McpProfile::Core, McpProfile::Full] {
            let root = TempDir::new().expect("temp dir");
            let mut router =
                CutMcpRouter::new(root.path(), profile, Some(&cutd_binary)).expect("router");
            router.namespace = "cut".into();
            let before = router.runtime().get_state().expect("initial state");
            let result = router.call(
                &router.definition("timeline.apply").name,
                json!({
                    "expectedRevision": before.revision,
                    "idempotencyKey": "external-edit",
                    "command": {"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}
                }),
            );
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(result["result"]["staged"], true);
            assert_eq!(result["result"]["pendingProposal"]["status"], "pending");
            assert_eq!(result["result"]["revision"], before.revision);
            assert_eq!(result["result"]["project"], before.project);
            let after = router.runtime().get_state().expect("state after staging");
            assert_eq!(after.revision, before.revision);
            assert_eq!(after.project, before.project);
            let pending = serde_json::to_value(after.pending_proposal).expect("proposal json");
            assert_eq!(pending["status"], "pending");
            assert_eq!(pending, result["result"]["pendingProposal"]);
            assert!(pending["command"].to_string().contains("titles"));
        }
    }

    #[test]
    fn agent_tools_hide_canonical_bypasses_and_dispatch_rejects_them() {
        let root = TempDir::new().expect("temp dir");
        let router = CutMcpRouter::new(root.path(), McpProfile::Core, None).expect("router");
        let tools = router.agent_tools();
        for name in AGENT_CANONICAL_ONLY_TOOLS {
            assert!(
                !tools.contains_key(*name),
                "agent tool listing exposed {name}"
            );
            let error = router
                .clone_for_agent()
                .call_base(name, json!({}))
                .expect_err("canonical bypass must be rejected");
            assert!(error.starts_with("AGENT_CANONICAL_BYPASS:"));
        }
        assert!(tools.contains_key("timeline.apply"));
        assert!(tools.contains_key("workflow.roughcut"));
    }

    #[cfg(unix)]
    #[test]
    fn composition_check_profiles_discovery_and_dispatch_are_read_only() {
        let root = TempDir::new().expect("temp dir");
        let project_path = root.path().join("project.cut.json");
        let project_bytes = serde_json::to_vec(
            &cut_domain::serialize_project(&cut_domain::empty_project()).expect("project"),
        )
        .expect("project bytes");
        fs::write(&project_path, &project_bytes).expect("project fixture");
        let html = "<main data-composition-id='intro' data-width='1920' data-height='1080' data-fps='30' data-duration='5'></main>";
        fs::write(root.path().join("index.html"), html).expect("composition fixture");
        for profile in [McpProfile::Core, McpProfile::Full] {
            // No daemon or binary is available: successful dispatch also proves
            // this sidecar read never needs an authoritative state request.
            let mut router = CutMcpRouter {
                dir: root.path().to_path_buf(),
                runtime: RuntimeClient::new(root.path()).expect("client"),
                skills: Vec::new(),
                profile,
                namespace: String::new(),
            };
            for namespace in ["", "cut"] {
                router.namespace = namespace.into();
                let name = router.definition("composition.check").name;
                let listed = router.list_tools();
                let definition = listed
                    .iter()
                    .find(|tool| tool.name == name)
                    .expect("listed");
                assert_eq!(definition.annotations.read_only_hint, Some(true));
                assert_eq!(definition.annotations.idempotent_hint, Some(true));
                assert_eq!(definition.input_schema["additionalProperties"], false);
                assert_eq!(
                    definition.input_schema["properties"]["entry"]["default"],
                    "index.html"
                );
                let search = router.call(
                    &router.definition("tools.search").name,
                    json!({"query":"composition"}),
                );
                assert_eq!(search["ok"], true, "{search}");
                assert!(search["result"]["items"]
                    .as_array()
                    .expect("items")
                    .iter()
                    .any(|item| item["name"] == name));
                let described = router.call(
                    &router.definition("tools.describe").name,
                    json!({"name":name}),
                );
                assert_eq!(described["ok"], true, "{described}");
                assert_eq!(described["result"]["tool"]["name"], name);
                assert_eq!(
                    described["result"]["example"],
                    json!({"entry":"index.html"})
                );
                let response = router.call(&name, described["result"]["example"].clone());
                assert_eq!(response["ok"], true, "{response}");
                assert_eq!(response["result"]["valid"], true);
                assert_eq!(response["result"]["deterministic"], true);
                assert_eq!(response, router.call(&name, json!({})));
                assert!(router.agent_tools().contains_key("composition.check"));
                let embedded = router
                    .clone_for_agent()
                    .call_base("composition.check", json!({}))
                    .expect("embedded dispatch");
                assert_eq!(embedded, response["result"]);
                let traversal = router.call(&name, json!({"entry":"../index.html"}));
                assert_eq!(traversal["ok"], false);
                assert_eq!(traversal["error"]["code"], "INVALID_ARGUMENTS");
            }
        }
        assert_eq!(
            fs::read(project_path).expect("project after check"),
            project_bytes
        );
        assert_eq!(
            fs::read_to_string(root.path().join("index.html")).expect("entry after check"),
            html
        );
        assert!(!root.path().join(".cut").exists());
        assert_eq!(fs::read_dir(root.path()).expect("directory").count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn composition_catalog_and_render_tools_are_registered_and_routed() {
        let root = TempDir::new().expect("temp dir");
        fs::write(
            root.path().join("index.html"),
            "<main data-composition-id='catalog' data-width='320' data-height='180' data-fps='30' data-duration='1'></main>",
        )
        .expect("composition fixture");
        let expected = [
            "composition.check",
            "composition.catalog",
            "composition.preview",
            "composition.render",
        ];

        for profile in [McpProfile::Core, McpProfile::Full] {
            let mut router = CutMcpRouter {
                dir: root.path().to_path_buf(),
                runtime: RuntimeClient::new(root.path()).expect("client"),
                skills: Vec::new(),
                profile,
                namespace: String::new(),
            };
            for namespace in ["", "cut"] {
                router.namespace = namespace.into();
                let listed = router.list_tools();
                let agent_tools = router.agent_tools();
                for base_name in expected {
                    assert!(profile.names().contains(&base_name));
                    let name = router.definition(base_name).name;
                    let definition = listed
                        .iter()
                        .find(|tool| tool.name == name)
                        .expect("registered composition tool");
                    assert_eq!(definition.annotations.read_only_hint, Some(true));
                    assert_eq!(definition.annotations.idempotent_hint, Some(true));
                    assert_eq!(definition.input_schema["additionalProperties"], false);
                    assert!(agent_tools.contains_key(base_name));

                    let search = router.call(
                        &router.definition("tools.search").name,
                        json!({"query":base_name}),
                    );
                    assert_eq!(search["ok"], true, "{search}");
                    assert!(search["result"]["items"]
                        .as_array()
                        .expect("search items")
                        .iter()
                        .any(|item| item["name"] == name));

                    let described = router.call(
                        &router.definition("tools.describe").name,
                        json!({"name":name}),
                    );
                    assert_eq!(described["ok"], true, "{described}");
                    assert_eq!(described["result"]["tool"]["name"], name);
                    assert_eq!(described["result"]["example"], example_for_tool(base_name));
                }

                let catalog_name = router.definition("composition.catalog").name;
                let response = router.call(&catalog_name, json!({}));
                assert_eq!(response["ok"], true, "{response}");
                assert_eq!(response["result"]["items"][0]["entry"], "index.html");
                assert_eq!(response["result"]["canonicalStateChanged"], false);
            }
        }
        assert!(!root.path().join(".cut").exists());
    }

    #[cfg(unix)]
    #[test]
    fn composition_check_large_dispatch_never_archives_or_creates_runtime_state() {
        let root = TempDir::new().expect("temp dir");
        let html = format!(
            "<main data-composition-id='intro' data-width='1920' data-height='1080' data-fps='30' data-duration='5'>{}</main>",
            "<div data-start='0' data-duration='1' data-track-index='0'></div>".repeat(500)
        );
        fs::write(root.path().join("index.html"), &html).expect("large fixture");
        let router = CutMcpRouter {
            dir: root.path().to_path_buf(),
            runtime: RuntimeClient::new(root.path()).expect("client"),
            skills: Vec::new(),
            profile: McpProfile::Core,
            namespace: String::new(),
        };
        let response = router.call("composition.check", json!({}));
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(response["result"]["valid"], true);
        assert_eq!(response["result"]["counts"]["elements"], 500);
        assert_eq!(response["result"]["truncated"], true);
        assert!(response["result"].get("readWith").is_none());
        assert!(response["result"].to_string().len() <= results::MAX_INLINE_BYTES);
        assert_eq!(
            fs::read_to_string(root.path().join("index.html")).expect("unchanged entry"),
            html
        );
        assert!(!root.path().join(".cut").exists());
        assert_eq!(fs::read_dir(root.path()).expect("directory").count(), 1);
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
    fn agent_router_runs_observe_planning_and_multi_range_proposal_flow() {
        let root = TempDir::new().expect("temp dir");
        let source_path = root.path().join("source.mp4");
        let status = ProcessCommand::new(cut_engine::ffmpeg::resolve_ffmpeg_bin())
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64:r=30:d=2",
                "-an",
                "-c:v",
                "mpeg4",
                source_path.to_str().expect("source path"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "ffmpeg fixture generation failed");
        let mut project =
            cut_domain::serialize_project(&cut_domain::empty_project()).expect("empty project");
        project["sources"] = json!([{
            "id": "source-1",
            "path": source_path.to_string_lossy(),
            "duration": 2.0
        }]);
        project["tracks"] = json!([{"id": "video-1", "type": "video", "clips": []}]);
        project["tracks"][0]["clips"] = json!([
            {"id":"clip-existing-1","sourceId":"source-1","in":0.0,"out":0.5,"timelineStart":0.0},
            {"id":"clip-existing-2","sourceId":"source-1","in":0.8,"out":1.4,"timelineStart":0.8}
        ]);
        std::fs::write(
            root.path().join("project.cut.json"),
            serde_json::to_vec_pretty(&project).expect("project json"),
        )
        .expect("project");
        let cutd_binary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/cutd");
        assert!(cutd_binary.is_file(), "cutd test binary is required");
        let router =
            CutMcpRouter::new(root.path(), McpProfile::Core, Some(&cutd_binary)).expect("router");
        let provider = SequencedAgentProvider {
            responses: Mutex::new(vec![
                vec![ProviderChunk::MessageEnd {
                    stop_reason: "tool_use".into(),
                    content: vec![json!({
                        "type": "tool_use",
                        "id": "outline-1",
                        "name": "project.outline",
                        "input": {"include": "summary"}
                    })],
                }],
                vec![ProviderChunk::MessageEnd {
                    stop_reason: "tool_use".into(),
                    content: vec![json!({
                        "type": "tool_use",
                        "id": "observe-1",
                        "name": "media.probe",
                        "input": {
                            "action": "observe",
                            "src": "source-1",
                            "opts": {"start": 0.0, "duration": 1.0, "frames": 2, "phase": "coarse", "evidence": ["frames"]}
                        }
                    })],
                }],
                vec![ProviderChunk::MessageEnd {
                    stop_reason: "tool_use".into(),
                    content: vec![json!({
                        "type": "tool_use",
                        "id": "plan-1",
                        "name": "workflow.roughcut",
                        "input": {"goal": "assemble selected moments"}
                    })],
                }],
                vec![ProviderChunk::MessageEnd {
                    stop_reason: "tool_use".into(),
                    content: vec![json!({
                        "type": "tool_use",
                        "id": "roughcut-1",
                        "name": "workflow.roughcut",
                        "input": {
                            "goal": "assemble selected moments",
                            "selections": [
                                {"sourceId":"source-1","start":0.0,"end":0.4,"timelineStart":0.0,"clipId":"clip-existing-1","rationale":"opening beat"},
                                {"sourceId":"source-1","start":0.8,"end":1.2,"timelineStart":0.8,"clipId":"clip-existing-2","rationale":"closing beat"}
                            ]
                        }
                    })],
                }],
                vec![ProviderChunk::MessageEnd {
                    stop_reason: "end_turn".into(),
                    content: vec![json!({"type": "text", "text": "awaiting review"})],
                }],
            ]),
        };
        let mut session = AgentSession::from_provider(
            Box::new(provider),
            None,
            None,
            Vec::new(),
            router.agent_tools(),
            6,
            OpenAiApiMode::Chat,
            None,
        );
        assert_eq!(
            session.run("make a rough cut").expect("agent run"),
            "awaiting review"
        );
        let state = router.runtime().get_state().expect("runtime state");
        let pending = state.pending_proposal.expect("pending proposal");
        let pending = serde_json::to_value(pending).expect("proposal json");
        assert_eq!(pending["status"], "pending");
        assert_eq!(
            pending["command"]["commands"][0]["commands"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(session
            .messages
            .iter()
            .any(|message| message.to_string().contains("planning_needed")));
        assert!(session
            .messages
            .iter()
            .any(|message| message.to_string().contains("candidate")));
    }

    #[test]
    fn legacy_trim_lowers_to_typed_update() {
        let project = cut_domain::empty_project();
        let mut project_value = cut_domain::serialize_project(&project).unwrap();
        project_value["sources"] = json!([{
            "id": "source-1", "path": "/tmp/source.mp4", "duration": 4
        }]);
        project_value["tracks"] = json!([{"id": "video-1", "type": "video", "clips": []}]);
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

    #[test]
    fn observation_range_is_explicit_and_bounded() {
        let metadata = cut_engine::ProbeInfo {
            duration: Some(30.0),
            ..cut_engine::ProbeInfo::default()
        };
        let options = ProbeOptions {
            start: Some(4.0),
            duration: Some(8.0),
            ..ProbeOptions::default()
        };
        let range = observation_range(&options, &metadata).expect("range");
        assert_eq!(range.start, 4.0);
        assert_eq!(range.end, 12.0);

        let too_long = ProbeOptions {
            start: Some(0.0),
            duration: Some(MAX_OBSERVATION_RANGE_SECONDS + 1.0),
            ..ProbeOptions::default()
        };
        assert!(observation_range(&too_long, &metadata).is_err());
    }

    #[test]
    fn observation_times_are_selected_inside_the_shared_range() {
        let range = cut_engine::TimeRange::new(4.0, 12.0).expect("range");
        let options = ProbeOptions {
            frames: Some(4),
            ..ProbeOptions::default()
        };
        let times = observation_times(&options, range).expect("times");
        assert_eq!(times, vec![5.0, 7.0, 9.0, 11.0]);

        let outside = ProbeOptions {
            times: Some(vec![3.0]),
            ..ProbeOptions::default()
        };
        assert!(observation_times(&outside, range).is_err());
    }

    #[test]
    fn media_probe_accepts_a_project_source_id() {
        let root = TempDir::new().expect("temp dir");
        let source_path = root.path().join("source.mp4");
        std::fs::write(&source_path, b"placeholder").expect("source");
        let mut value =
            cut_domain::serialize_project(&cut_domain::empty_project()).expect("project");
        value["sources"] = json!([{
            "id": "source-1",
            "path": source_path.to_string_lossy()
        }]);
        let project = cut_domain::parse_project(&value).expect("project");
        let resolved =
            resolve_media_src(root.path(), "source-1", Some(&project)).expect("source id");
        assert_eq!(
            resolved,
            source_path
                .canonicalize()
                .expect("canonical source")
                .to_string_lossy()
        );
    }

    #[test]
    fn generated_asset_registration_is_cached_and_keeps_provenance_outside_project() {
        let root = TempDir::new().expect("temp dir");
        let artifact = root.path().join("generated.mp4");
        std::fs::write(&artifact, b"local generated artifact").expect("artifact");
        let router = CutMcpRouter::new(root.path(), McpProfile::Core, None).expect("router");
        let request = json!({
            "action": "register",
            "path": "generated.mp4",
            "kind": "video",
            "requestFingerprint": "request-product-1",
            "provider": "local",
            "model": "fixture",
            "provenance": {"source": "test"}
        });
        let first = router.call("media.asset", request.clone());
        assert_eq!(first["ok"], true);
        assert_eq!(first["result"]["cached"], false);
        assert_eq!(first["result"]["asset"]["backend"], "local-artifact");
        assert!(first["result"]["asset"]["contentFingerprint"].is_string());
        assert!(first["result"]["asset"]["path"].is_string());
        assert!(first["result"]["project"].is_null());
        let second = router.call("media.asset", request);
        assert_eq!(second["ok"], true);
        assert_eq!(second["result"]["cached"], true);
        assert!(root.path().join(".cut/cache/assets.json").is_file());
        assert!(!root
            .path()
            .join("project.cut.json")
            .to_string_lossy()
            .contains("generated.mp4"));
    }

    #[test]
    fn generated_asset_manifest_keeps_concurrent_provenance_records() {
        let root = TempDir::new().expect("temp dir");
        let artifact = root.path().join("generated.mp4");
        std::fs::write(&artifact, b"local generated artifact").expect("artifact");
        let manifest_path = Arc::new(root.path().join(".cut/cache/assets.json"));
        let artifact_path = artifact.to_string_lossy().into_owned();
        let workers = (0..8)
            .map(|index| {
                let manifest_path = Arc::clone(&manifest_path);
                let artifact_path = artifact_path.clone();
                thread::spawn(move || {
                    let record = GeneratedAssetRecord {
                        asset_id: format!("asset-{index}"),
                        path: artifact_path,
                        kind: "video".into(),
                        request_fingerprint: format!("request-{index}"),
                        content_fingerprint: format!("content-{index}"),
                        backend: "fixture".into(),
                        provider: None,
                        model: None,
                        provenance: Some(json!({"worker": index})),
                    };
                    upsert_asset_manifest(&manifest_path, &record).expect("manifest update");
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("manifest worker");
        }
        let bytes = std::fs::read(&*manifest_path).expect("manifest");
        let manifest: GeneratedAssetManifest = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(manifest.assets.len(), 8);
        assert!(manifest
            .assets
            .iter()
            .all(|asset| asset.provenance.is_some()));
    }

    #[test]
    fn generated_asset_request_conflict_keeps_the_immutable_first_record() {
        let root = TempDir::new().expect("temp dir");
        let first_path = root.path().join("first.mp4");
        let second_path = root.path().join("second.mp4");
        std::fs::write(&first_path, b"first artifact").expect("first artifact");
        std::fs::write(&second_path, b"second artifact").expect("second artifact");
        let manifest_path = root.path().join(".cut/cache/assets.json");
        let first = GeneratedAssetRecord {
            asset_id: "asset-first".into(),
            path: first_path.to_string_lossy().into_owned(),
            kind: "video".into(),
            request_fingerprint: "same-request".into(),
            content_fingerprint: "content-first".into(),
            backend: "fixture".into(),
            provider: None,
            model: None,
            provenance: Some(json!({"version": 1})),
        };
        let second = GeneratedAssetRecord {
            asset_id: "asset-second".into(),
            path: second_path.to_string_lossy().into_owned(),
            content_fingerprint: "content-second".into(),
            ..first.clone()
        };
        upsert_asset_manifest(&manifest_path, &first).expect("first manifest update");
        let error = upsert_asset_manifest(&manifest_path, &second).expect_err("conflict");
        assert_eq!(error.code, "ASSET_REQUEST_CONFLICT");
        let bytes = std::fs::read(&manifest_path).expect("manifest");
        let manifest: GeneratedAssetManifest = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(manifest.assets[0].asset_id, "asset-first");
        assert_eq!(manifest.assets[0].path, first.path);
    }

    #[test]
    fn workflow_duration_caps_speed_adjusted_selected_output() {
        let root = TempDir::new().expect("temp dir");
        let router = CutMcpRouter::new(root.path(), McpProfile::Core, None).expect("router");
        let result = router.call(
            "workflow.roughcut",
            json!({
                "goal": "assemble a ten second cut",
                "duration": 10.0,
                "selections": [
                    {"sourceId":"source-1","start":0.0,"end":20.0,"speed":2.0,"rationale":"first beat"},
                    {"sourceId":"source-2","start":0.0,"end":20.0,"speed":1.0,"rationale":"second beat"}
                ]
            }),
        );
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "INVALID_ARGUMENTS");
        assert_eq!(result["error"]["details"]["selectedDuration"], 30.0);
    }

    #[test]
    fn proposal_capture_window_follows_selected_timeline_ranges() {
        let (start, end, coverage) = proposal_capture_window(&[(200.0, 206.0), (210.0, 218.0)]);
        assert_eq!((start, end), (200.0, 218.0));
        assert_eq!(coverage["mode"], "full");
        assert_eq!(coverage["fullyCovered"], true);

        let (start, end, coverage) = proposal_capture_window(&[(0.0, 2.0), (300.0, 304.0)]);
        assert_eq!((start, end), (300.0, 304.0));
        assert_eq!(coverage["mode"], "representative");
        assert_eq!(coverage["fullyCovered"], false);
        assert_eq!(coverage["requestedRanges"][0]["fullyCovered"], false);
    }

    #[test]
    fn listen_requires_an_explicit_bounded_range() {
        let metadata = cut_engine::ProbeInfo {
            duration: Some(30.0),
            has_audio: Some(true),
            ..cut_engine::ProbeInfo::default()
        };
        let options = ProbeOptions::default();
        assert!(transcription_range(&options, &metadata, true).is_err());
        let options = ProbeOptions {
            start: Some(4.0),
            duration: Some(8.0),
            ..ProbeOptions::default()
        };
        let range = transcription_range(&options, &metadata, true).expect("range");
        assert_eq!(range.start, 4.0);
        assert_eq!(range.end, 12.0);
    }

    #[test]
    fn provider_backend_is_only_selected_when_explicit() {
        let range = cut_engine::TimeRange::new(0.0, 1.0).expect("range");
        let local =
            transcription_options(Path::new("/tmp/project"), &ProbeOptions::default(), range)
                .expect("options");
        assert!(matches!(
            local.backend,
            cut_engine::TranscriptionBackend::LocalWhisperCpp
        ));
        let provider = ProbeOptions {
            backend: Some("provider".into()),
            provider: Some("openai".into()),
            ..ProbeOptions::default()
        };
        let provider =
            transcription_options(Path::new("/tmp/project"), &provider, range).expect("options");
        assert!(matches!(
            provider.backend,
            cut_engine::TranscriptionBackend::ExplicitProvider { name } if name == "openai"
        ));
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
