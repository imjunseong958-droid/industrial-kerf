//! Bounded, human-facing summaries of tool calls and results for progress
//! displays. Raw tool input and output never reach an event.

use crate::bounded_text;
use serde_json::Value;

pub(crate) const MAX_AGENT_SUMMARY_TEXT_BYTES: usize = 512;
const MAX_AGENT_SUMMARY_ITEMS: usize = 8;
const MAX_AGENT_SUMMARY_NODES: usize = 256;

/// Bounded entity references extracted from a tool call for human-facing
/// progress displays. Raw tool input is intentionally not part of an event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTargetSummary {
    pub sources: Vec<String>,
    pub clips: Vec<String>,
    pub tracks: Vec<String>,
    pub jobs: Vec<String>,
}

/// Bounded, human-facing metadata extracted from a tool result. The canonical
/// result remains in the agent conversation; this summary is the only result
/// shape exposed through desktop event transport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolResultSummary {
    pub targets: AgentTargetSummary,
    pub artifact_paths: Vec<String>,
    pub revision: Option<u64>,
    pub status: Option<String>,
    pub count: Option<usize>,
    pub error: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
enum TargetKind {
    Source,
    Clip,
    Track,
    Job,
}

pub(crate) fn summarize_targets(value: &Value) -> AgentTargetSummary {
    let mut summary = AgentTargetSummary::default();
    let mut nodes = 0;
    collect_targets(value, None, &mut summary, &mut nodes);
    summary
}

fn collect_targets(
    value: &Value,
    context: Option<TargetKind>,
    summary: &mut AgentTargetSummary,
    nodes: &mut usize,
) {
    if *nodes >= MAX_AGENT_SUMMARY_NODES {
        return;
    }
    *nodes += 1;
    match value {
        Value::Array(values) => {
            for value in values {
                collect_targets(value, context, summary, nodes);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                let nested_context = target_kind(key).or(context);
                match key.as_str() {
                    "sourceId" => add_target(
                        &mut summary.sources,
                        value.as_str(),
                        MAX_AGENT_SUMMARY_ITEMS,
                    ),
                    "clipId" => {
                        add_target(&mut summary.clips, value.as_str(), MAX_AGENT_SUMMARY_ITEMS)
                    }
                    "trackId" => {
                        add_target(&mut summary.tracks, value.as_str(), MAX_AGENT_SUMMARY_ITEMS)
                    }
                    "jobId" => {
                        add_target(&mut summary.jobs, value.as_str(), MAX_AGENT_SUMMARY_ITEMS)
                    }
                    "src" | "source" => add_target(
                        &mut summary.sources,
                        value.as_str(),
                        MAX_AGENT_SUMMARY_ITEMS,
                    ),
                    "id" => match context {
                        Some(TargetKind::Source) => add_target(
                            &mut summary.sources,
                            value.as_str(),
                            MAX_AGENT_SUMMARY_ITEMS,
                        ),
                        Some(TargetKind::Clip) => {
                            add_target(&mut summary.clips, value.as_str(), MAX_AGENT_SUMMARY_ITEMS)
                        }
                        Some(TargetKind::Track) => {
                            add_target(&mut summary.tracks, value.as_str(), MAX_AGENT_SUMMARY_ITEMS)
                        }
                        Some(TargetKind::Job) | None => {}
                    },
                    _ => {}
                }
                if !value.is_string() {
                    collect_targets(value, nested_context, summary, nodes);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn target_kind(key: &str) -> Option<TargetKind> {
    match key {
        "source" | "sources" => Some(TargetKind::Source),
        "clip" | "clips" => Some(TargetKind::Clip),
        "track" | "tracks" => Some(TargetKind::Track),
        "job" | "jobs" => Some(TargetKind::Job),
        _ => None,
    }
}

fn add_target(targets: &mut Vec<String>, value: Option<&str>, limit: usize) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    let value = bounded_text(value, MAX_AGENT_SUMMARY_TEXT_BYTES);
    if targets.iter().any(|target| target == &value) || targets.len() >= limit {
        return;
    }
    targets.push(value);
}

pub(crate) fn summarize_tool_result(value: &Value) -> ToolResultSummary {
    let mut summary = ToolResultSummary {
        targets: summarize_targets(value),
        ..ToolResultSummary::default()
    };
    let mut nodes = 0;
    collect_result_fields(value, &mut summary, &mut nodes);
    summary
}

fn collect_result_fields(value: &Value, summary: &mut ToolResultSummary, nodes: &mut usize) {
    if *nodes >= MAX_AGENT_SUMMARY_NODES {
        summary.truncated = true;
        return;
    }
    *nodes += 1;
    match value {
        Value::Array(values) => {
            for value in values {
                collect_result_fields(value, summary, nodes);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                match key.as_str() {
                    "revision" if summary.revision.is_none() => summary.revision = value.as_u64(),
                    "status" if summary.status.is_none() => {
                        summary.status = value
                            .as_str()
                            .map(|value| bounded_text(value, MAX_AGENT_SUMMARY_TEXT_BYTES))
                    }
                    "error" if summary.error.is_none() => {
                        summary.error = value.as_str().map(compact_error)
                    }
                    "path" | "output" => {
                        let Some(path) = value.as_str() else {
                            continue;
                        };
                        if path.starts_with("data:") || path.trim().is_empty() {
                            continue;
                        }
                        if !add_artifact_path(&mut summary.artifact_paths, path) {
                            summary.truncated = true;
                        }
                    }
                    "count" if summary.count.is_none() => {
                        summary.count = value.as_u64().and_then(|value| usize::try_from(value).ok())
                    }
                    "frames" | "ranges" | "tracks" | "clips" if summary.count.is_none() => {
                        summary.count = value.as_array().map(|values| values.len());
                    }
                    _ => {}
                }
                if !value.is_string() {
                    collect_result_fields(value, summary, nodes);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn add_artifact_path(paths: &mut Vec<String>, path: &str) -> bool {
    let path = bounded_text(path, MAX_AGENT_SUMMARY_TEXT_BYTES);
    if paths.iter().any(|existing| existing == &path) {
        return true;
    }
    if paths.len() >= MAX_AGENT_SUMMARY_ITEMS {
        return false;
    }
    paths.push(path);
    true
}

fn compact_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "api_key",
        "apikey",
        "access_token",
        "authorization",
        "bearer ",
        "cookie",
        "password",
        "secret",
        "token=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        "sensitive tool error redacted".into()
    } else {
        bounded_text(error, MAX_AGENT_SUMMARY_TEXT_BYTES)
    }
}

impl ToolResultSummary {
    pub(crate) fn from_error(error: &str) -> Self {
        Self {
            error: Some(compact_error(error)),
            ..Self::default()
        }
    }
}
