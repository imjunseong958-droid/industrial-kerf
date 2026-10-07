//! Native agent loop and provider boundary.
//!
//! The session stores one canonical Anthropic-shaped history. Provider
//! adapters translate at the boundary, so tool execution and proposal
//! handling stay independent from a vendor SDK.

use base64::Engine;
use cut_skills::{compose_system_prompt, Skill};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod context;
mod provider;
mod summary;

pub use context::ContextBudget;
pub use provider::{
    to_openai_messages, to_openai_responses_input, to_openai_responses_tool, to_openai_tool,
    AnthropicProvider, JsonHttpClient, LlmProvider, NormalizedTool, OpenAiProvider,
    OpenAiResponsesProvider, ProviderChunk, ProviderError, ProviderRequest, UreqHttpClient,
    DEFAULT_ANTHROPIC_MODEL, DEFAULT_OPENAI_MODEL, DEFAULT_RESPONSES_MODEL,
};
pub use summary::{AgentTargetSummary, ToolResultSummary};

use provider::{is_tool_use, text_from_blocks};
use summary::{summarize_targets, summarize_tool_result, MAX_AGENT_SUMMARY_TEXT_BYTES};

const DEFAULT_SYSTEM_PROMPT: &str = "You are a video editing assistant for Cut. \
Edits you make are proposals: do not claim one is applied until the user accepts it.";

pub const MAX_TOOL_RESULT_IMAGES: usize = 8;
pub const MAX_TOOL_RESULT_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_AGENT_EVENT_TEXT_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderName {
    Anthropic,
    Openai,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiApiMode {
    Chat,
    Responses,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ProviderName {
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::Openai];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.name() == text)
    }
}

impl OpenAiApiMode {
    pub const ALL: [Self; 2] = [Self::Chat, Self::Responses];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Responses => "responses",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == text)
    }
}

impl ReasoningEffort {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|effort| effort.name() == text)
    }
}

/// A structured tool available to AgentSession.
pub struct AgentTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    executor: Arc<ToolExecutor>,
}

type ToolExecutor = dyn Fn(Value, &AtomicBool) -> Result<Value, String> + Send + Sync;

impl AgentTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        executor: impl Fn(Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            executor: Arc::new(move |input, _| executor(input)),
        }
    }

    pub fn cancellable(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        executor: impl Fn(Value, &AtomicBool) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            executor: Arc::new(executor),
        }
    }

    fn execute(&self, input: Value, cancelled: &AtomicBool) -> Result<Value, String> {
        (self.executor)(input, cancelled)
    }
}

pub type ToolMap = BTreeMap<String, AgentTool>;

/// Context read fresh for every model request, such as the current project
/// document. When it changes it is added to the conversation as a new
/// message, so the system prompt and everything already sent stay the same
/// and stay cached.
pub type ContextSource = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Progress emitted by an agent session. Payloads intended for a UI are
/// bounded and contain summaries rather than provider or tool JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    TextDelta(String),
    TurnStarted {
        stop_reason: String,
    },
    ToolCallStarted {
        id: String,
        name: String,
        targets: AgentTargetSummary,
    },
    ToolCallFinished {
        id: String,
        name: String,
        ok: bool,
        summary: Box<ToolResultSummary>,
    },
    Done {
        text: String,
    },
    Aborted,
}

#[derive(Debug)]
pub enum AgentError {
    Provider(ProviderError),
    Tool(String),
    Serialization(String),
    TurnLimit(usize),
    ContextBudget(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(error) => error.fmt(formatter),
            Self::Tool(error) => write!(formatter, "tool error: {error}"),
            Self::Serialization(error) => write!(formatter, "serialization error: {error}"),
            Self::TurnLimit(limit) => write!(formatter, "agent reached the {limit}-turn limit"),
            Self::ContextBudget(error) => write!(formatter, "agent input budget: {error}"),
        }
    }
}

impl std::error::Error for AgentError {}

/// A provider refused thinking blocks made for a different view of the
/// conversation.
fn is_thinking_mismatch(error: &ProviderError) -> bool {
    let text = format!("{} {}", error.code, error.message);
    text.contains("`thinking` block") && text.contains("signature")
}

fn is_context_limit(error: &ProviderError) -> bool {
    let text = format!("{} {}", error.code, error.message).to_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "input is too long",
        "context window",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

pub struct AgentSession {
    pub model: String,
    pub provider: ProviderName,
    pub api_mode: OpenAiApiMode,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub system_prompt: String,
    pub messages: Vec<Value>,
    tools: ToolMap,
    context: Option<ContextSource>,
    llm: Box<dyn LlmProvider>,
    max_turns: usize,
    context_budget: ContextBudget,
    view: context::View,
    /// The context last added to the conversation.
    snapshot: Option<String>,
    aborted: Arc<AtomicBool>,
}

impl fmt::Debug for AgentSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentSession")
            .field("model", &self.model)
            .field("provider", &self.provider)
            .field("api_mode", &self.api_mode)
            .field("message_count", &self.messages.len())
            .finish_non_exhaustive()
    }
}

pub struct AgentSessionOptions {
    pub provider: ProviderName,
    pub api_mode: OpenAiApiMode,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub skills: Vec<Skill>,
    pub tools: ToolMap,
    pub context: Option<ContextSource>,
    pub max_turns: usize,
    pub context_budget: ContextBudget,
}

impl Default for AgentSessionOptions {
    fn default() -> Self {
        Self {
            provider: ProviderName::Anthropic,
            api_mode: OpenAiApiMode::Chat,
            reasoning_effort: None,
            api_key: None,
            base_url: None,
            model: None,
            system_prompt: None,
            skills: Vec::new(),
            tools: BTreeMap::new(),
            context: None,
            max_turns: 32,
            context_budget: ContextBudget::from_env(),
        }
    }
}

impl AgentSession {
    pub fn new(options: AgentSessionOptions) -> Self {
        let llm: Box<dyn LlmProvider> = match (options.provider, options.api_mode) {
            (ProviderName::Anthropic, _) => Box::new(
                AnthropicProvider::new(options.api_key.clone(), options.base_url.clone())
                    .with_effort(options.reasoning_effort),
            ),
            (ProviderName::Openai, OpenAiApiMode::Chat) => Box::new(OpenAiProvider::new(
                options.api_key.clone(),
                options.base_url.clone(),
            )),
            (ProviderName::Openai, OpenAiApiMode::Responses) => {
                Box::new(OpenAiResponsesProvider::new(
                    options.api_key.clone(),
                    options.base_url.clone(),
                    options.reasoning_effort,
                ))
            }
        };
        let mut session = Self::from_provider(
            llm,
            options.model,
            options.system_prompt,
            options.skills,
            options.tools,
            options.max_turns,
            options.api_mode,
            options.reasoning_effort,
        );
        session.context_budget = options.context_budget;
        session.context = options.context;
        session
    }

    /// Read `context` before every model request.
    pub fn with_context(mut self, context: ContextSource) -> Self {
        self.context = Some(context);
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_provider(
        llm: Box<dyn LlmProvider>,
        model: Option<String>,
        system_prompt: Option<String>,
        skills: Vec<Skill>,
        tools: ToolMap,
        max_turns: usize,
        api_mode: OpenAiApiMode,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Self {
        let provider = llm.name();
        let model = model.unwrap_or_else(|| llm.default_model().into());
        let system_prompt = compose_system_prompt(
            system_prompt.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT),
            &skills,
        );
        Self {
            model,
            provider,
            api_mode,
            reasoning_effort,
            system_prompt,
            messages: Vec::new(),
            tools,
            context: None,
            llm,
            max_turns: max_turns.max(1),
            context_budget: ContextBudget::default(),
            view: context::View::default(),
            snapshot: None,
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    pub fn abort_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.aborted)
    }

    pub fn normalized_tools(&self) -> Vec<NormalizedTool> {
        let mut tools = self
            .tools
            .values()
            .filter(|tool| tool.name != "session.read")
            .map(|tool| NormalizedTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect::<Vec<_>>();
        tools.push(context::read_tool());
        tools
    }

    fn execute_tool(&self, name: &str, input: Value) -> Result<Value, String> {
        if name == "session.read" {
            return context::read(&self.messages, input);
        }
        self.tools
            .get(name)
            .ok_or_else(|| format!("unknown tool: {name}"))?
            .execute(input, &self.aborted)
    }

    /// Add the current context to the conversation when it differs from
    /// the last one added. The conversation only grows.
    fn add_context(&mut self) {
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let text = context()
            .unwrap_or_else(|error| format!("The current project could not be read: {error}"));
        if text.is_empty() || self.snapshot.as_deref() == Some(text.as_str()) {
            return;
        }
        self.messages.push(json!({"role": "user", "content": [{
            "type": "text",
            "text": format!("{}\n\n{text}", context::SNAPSHOT_HEADER),
        }]}));
        self.snapshot = Some(text);
    }

    fn provider_request(&mut self, budget: &ContextBudget) -> Result<ProviderRequest, AgentError> {
        let tools = self.normalized_tools();
        let system = self.system_prompt.clone();
        let fixed = system.len()
            + context::units(
                &serde_json::to_value(&tools)
                    .map_err(|error| AgentError::Serialization(error.to_string()))?,
            )
            + tools.len() * 256
            + 1024;
        let available = budget.max_input_units.checked_sub(fixed)
            .ok_or_else(|| AgentError::ContextBudget("System prompt and tool definitions exceed the input allowance. Increase the budget for this model.".into()))?;
        self.add_context();
        let view = context::fit(&self.messages, self.view, available, budget)
            .map_err(AgentError::ContextBudget)?;
        if view.archived != self.view.archived {
            // Compacting archived the project context; show it again.
            self.snapshot = None;
            self.add_context();
        }
        self.view = context::fit(&self.messages, view, available, budget)
            .map_err(AgentError::ContextBudget)?;
        Ok(ProviderRequest {
            model: self.model.clone(),
            system,
            tools,
            messages: context::render(&self.messages, &self.view, budget),
        })
    }

    fn stream_bounded(&mut self) -> Result<Vec<ProviderChunk>, AgentError> {
        let budget = self.context_budget.clone();
        let request = self.provider_request(&budget)?;
        // No tools have run for a refused request. Retry once; never restart
        // an edit or replay a mutating tool call.
        match self.llm.stream(&request) {
            Ok(chunks) => Ok(chunks),
            Err(error) if is_context_limit(&error) => {
                let mut smaller = budget;
                smaller.max_input_units /= 2;
                self.context_budget = smaller.clone();
                let request = self.provider_request(&smaller)?;
                self.llm.stream(&request).map_err(AgentError::Provider)
            }
            Err(error) if is_thinking_mismatch(&error) => {
                // Earlier thinking no longer matches the conversation it was
                // made in; leave it out from here on.
                self.view.thinking_from = self.messages.len();
                let request = self.provider_request(&budget)?;
                self.llm.stream(&request).map_err(AgentError::Provider)
            }
            Err(error) => Err(AgentError::Provider(error)),
        }
    }

    pub fn run(&mut self, user_message: &str) -> Result<String, AgentError> {
        self.run_with_events(user_message, |_| {})
    }

    pub fn run_with_events(
        &mut self,
        user_message: &str,
        mut emit: impl FnMut(AgentEvent),
    ) -> Result<String, AgentError> {
        self.messages
            .push(json!({"role": "user", "content": user_message}));
        self.aborted.store(false, Ordering::SeqCst);
        let mut final_text = String::new();

        for _ in 0..self.max_turns {
            if self.is_aborted() {
                emit(AgentEvent::Aborted);
                emit(AgentEvent::Done {
                    text: bounded_text(&final_text, MAX_AGENT_EVENT_TEXT_BYTES),
                });
                return Ok(final_text);
            }
            let chunks = self.stream_bounded()?;
            let mut stop_reason = "end_turn".to_owned();
            let mut content = Vec::new();
            for chunk in chunks {
                match chunk {
                    ProviderChunk::TextDelta { text } => emit(AgentEvent::TextDelta(bounded_text(
                        &text,
                        MAX_AGENT_EVENT_TEXT_BYTES,
                    ))),
                    ProviderChunk::MessageEnd {
                        stop_reason: reason,
                        content: blocks,
                    } => {
                        stop_reason = reason;
                        content = blocks;
                    }
                }
            }
            let calls = content
                .iter()
                .filter(|block| is_tool_use(block))
                .collect::<Vec<_>>();
            let mut ids = std::collections::HashSet::new();
            if (stop_reason == "tool_use") != !calls.is_empty()
                || calls.iter().any(|block| {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                    id.is_empty()
                        || !ids.insert(id)
                        || block
                            .get("name")
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty)
                        || !block.get("input").is_some_and(Value::is_object)
                })
            {
                return Err(AgentError::Provider(ProviderError::new("INVALID_PROVIDER_RESPONSE","Tool calls must have unique non-empty IDs, names, object inputs, and a tool-use stop reason. No tools were executed.")));
            }
            if stop_reason == "refusal" {
                return Err(AgentError::Provider(ProviderError::new(
                    "REFUSED",
                    "The model declined this request.",
                )));
            }
            emit(AgentEvent::TurnStarted {
                stop_reason: bounded_text(&stop_reason, MAX_AGENT_SUMMARY_TEXT_BYTES),
            });
            self.messages
                .push(json!({"role": "assistant", "content": content}));
            let text = text_from_blocks(&content);
            if stop_reason != "tool_use" {
                final_text.push_str(&text);
                emit(AgentEvent::Done {
                    text: bounded_text(&final_text, MAX_AGENT_EVENT_TEXT_BYTES),
                });
                return Ok(final_text);
            }

            let mut result_blocks = Vec::new();
            for block in content.iter().filter(|block| is_tool_use(block)) {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("call_tool")
                    .to_owned();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let event_name = name.clone();
                let targets = summarize_targets(&input);
                emit(AgentEvent::ToolCallStarted {
                    id: bounded_text(&id, MAX_AGENT_SUMMARY_TEXT_BYTES),
                    name: bounded_text(&event_name, MAX_AGENT_SUMMARY_TEXT_BYTES),
                    targets,
                });
                let output = if self.is_aborted() {
                    Err("Agent aborted before this tool executed".into())
                } else {
                    self.execute_tool(&name, input)
                };
                let (ok, summary, canonical) = match output {
                    Ok(value) => {
                        let ok = value.get("isError").and_then(Value::as_bool) != Some(true);
                        let summary = summarize_tool_result(&value);
                        match normalize_tool_result(value) {
                            Ok(canonical) => (ok, summary, canonical),
                            Err(error) => (false, ToolResultSummary::from_error(&error), Value::String(json!({"error":error,"note":"Tool execution finished but its evidence could not be represented. Recheck project state before retrying a mutation."}).to_string())),
                        }
                    }
                    Err(error) => {
                        let canonical = Value::String(
                            serde_json::to_string(&json!({"error": error}))
                                .unwrap_or_else(|_| "{\"error\":\"tool failed\"}".into()),
                        );
                        (false, ToolResultSummary::from_error(&error), canonical)
                    }
                };
                emit(AgentEvent::ToolCallFinished {
                    id: bounded_text(&id, MAX_AGENT_SUMMARY_TEXT_BYTES),
                    name: bounded_text(&event_name, MAX_AGENT_SUMMARY_TEXT_BYTES),
                    ok,
                    summary: Box::new(summary),
                });
                result_blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": canonical,
                    "is_error": !ok,
                }));
            }
            self.messages
                .push(json!({"role": "user", "content": result_blocks}));
            if self.is_aborted() {
                emit(AgentEvent::Aborted);
                emit(AgentEvent::Done {
                    text: bounded_text(&final_text, MAX_AGENT_EVENT_TEXT_BYTES),
                });
                return Ok(final_text);
            }
        }
        Err(AgentError::TurnLimit(self.max_turns))
    }

    pub fn save(&self, path: &Path) -> Result<(), AgentError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| AgentError::Serialization(error.to_string()))?;
        }
        let json = serde_json::to_vec_pretty(&self.messages)
            .map_err(|error| AgentError::Serialization(error.to_string()))?;
        fs::write(path, json).map_err(|error| AgentError::Serialization(error.to_string()))
    }

    pub fn load(path: &Path, options: AgentSessionOptions) -> Result<Self, AgentError> {
        let bytes = fs::read(path).map_err(|error| AgentError::Serialization(error.to_string()))?;
        let mut session = Self::new(options);
        session.messages = serde_json::from_slice(&bytes)
            .map_err(|error| AgentError::Serialization(error.to_string()))?;
        // Thinking from the earlier session was made for a view of it this
        // session does not reproduce.
        session.view.thinking_from = session.messages.len();
        Ok(session)
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let suffix = "…";
    let limit = max_bytes.saturating_sub(suffix.len());
    let mut end = 0;
    for (index, character) in value.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    format!("{}{}", &value[..end], suffix)
}

fn normalize_tool_result(mut value: Value) -> Result<Value, String> {
    // Native observations use imageBase64 beside path/time metadata. Promote
    // those bytes to image blocks, never send encoded image data as text.
    if value.get("type").and_then(Value::as_str) != Some("multimodal_tool_result") {
        let mut images = Vec::new();
        collect_inline_images(&mut value, &mut images);
        if !images.is_empty() {
            value = json!({"type":"multimodal_tool_result","metadata":value,"images":images});
        }
    }
    if value.get("type").and_then(Value::as_str) != Some("multimodal_tool_result") {
        return Ok(Value::String(
            serde_json::to_string(&value).map_err(|error| error.to_string())?,
        ));
    }
    let images = value
        .get("images")
        .and_then(Value::as_array)
        .ok_or_else(|| "multimodal tool result images must be an array".to_owned())?;
    if images.is_empty() {
        return Err("a multimodal tool result must contain an image".into());
    }
    if images.len() > MAX_TOOL_RESULT_IMAGES {
        return Err(format!(
            "a tool result may contain at most {MAX_TOOL_RESULT_IMAGES} images"
        ));
    }
    let mut total = 0usize;
    let mut blocks = Vec::new();
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
    }
    if let Some(metadata) = value.get("metadata") {
        blocks.push(json!({
            "type": "text",
            "text": serde_json::to_string(metadata).map_err(|error| error.to_string())?,
        }));
    }
    for (index, image) in images.iter().enumerate() {
        let encoded = image
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tool result image {} has no data", index + 1))?;
        let (media_type, encoded) = if let Some((header, data)) = encoded.split_once(",") {
            if !header.starts_with("data:image/") || !header.ends_with(";base64") {
                return Err(format!(
                    "tool result image {} has invalid data URL",
                    index + 1
                ));
            }
            let media_type = header
                .strip_prefix("data:")
                .unwrap_or_default()
                .strip_suffix(";base64")
                .unwrap_or_default();
            (media_type.to_owned(), data)
        } else {
            (
                image
                    .get("mediaType")
                    .or_else(|| image.get("media_type"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                encoded,
            )
        };
        if !matches!(media_type.as_str(), "image/png" | "image/jpeg") {
            return Err(format!(
                "tool result image {} has unsupported media type",
                index + 1
            ));
        }
        let compact = encoded.split_whitespace().collect::<String>();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&compact)
            .map_err(|_| format!("tool result image {} is not valid base64", index + 1))?;
        if decoded.is_empty() || decoded.len() > MAX_TOOL_RESULT_IMAGE_BYTES {
            return Err(format!(
                "tool result image {} exceeds its byte limit",
                index + 1
            ));
        }
        total += decoded.len();
        if total > MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES {
            return Err("tool result images exceed their total byte limit".into());
        }
        blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": compact,
            }
        }));
    }
    if !blocks
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("text"))
    {
        blocks.insert(
            0,
            json!({"type": "text", "text": format!("Tool returned {} image(s).", images.len())}),
        );
    }
    Ok(Value::Array(blocks))
}

fn collect_inline_images(value: &mut Value, images: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(data)) = object.remove("imageBase64") {
                let media_type = object.get("mediaType").cloned();
                object.insert("imageIndex".into(), json!(images.len()));
                images.push(json!({
                    "mediaType": media_type.unwrap_or_else(|| json!("image/png")),
                    "data": data,
                }));
            }
            for nested in object.values_mut() {
                collect_inline_images(nested, images);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_inline_images(nested, images);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeProvider {
        chunks: Vec<ProviderChunk>,
    }

    struct FakeStreamingClient {
        events: Vec<Value>,
        bodies: Mutex<Vec<Value>>,
    }

    impl JsonHttpClient for FakeStreamingClient {
        fn post_json(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            body: &Value,
        ) -> Result<Value, ProviderError> {
            self.bodies.lock().unwrap().push(body.clone());
            self.events
                .first()
                .cloned()
                .ok_or_else(|| ProviderError::new("TEST", "no response"))
        }

        fn post_stream(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            body: &Value,
        ) -> Result<Vec<Value>, ProviderError> {
            self.bodies.lock().unwrap().push(body.clone());
            Ok(self.events.clone())
        }
    }

    fn provider_request() -> ProviderRequest {
        ProviderRequest {
            model: "fixture-model".into(),
            system: "fixture-system".into(),
            tools: vec![NormalizedTool {
                name: "timeline.query".into(),
                description: "Inspect".into(),
                input_schema: json!({"type": "object"}),
            }],
            messages: vec![json!({"role": "user", "content": "hello"})],
        }
    }

    impl LlmProvider for FakeProvider {
        fn name(&self) -> ProviderName {
            ProviderName::Anthropic
        }

        fn default_model(&self) -> &'static str {
            DEFAULT_ANTHROPIC_MODEL
        }

        fn stream(&self, _request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError> {
            Ok(self.chunks.clone())
        }
    }

    #[test]
    fn protocol_conformance_fixture_matches_provider_translations() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/protocol/conformance.json"))
                .expect("protocol fixture");
        let provider = &fixture["provider"];
        let tool: NormalizedTool =
            serde_json::from_value(provider["tool"].clone()).expect("normalized tool");
        assert_eq!(to_openai_tool(&tool), provider["expected_chat_tools"][0]);
        assert_eq!(
            to_openai_responses_tool(&tool),
            provider["expected_responses_tools"][0]
        );
        let messages = provider["messages"].as_array().expect("messages");
        assert_eq!(
            Value::Array(to_openai_messages(messages)),
            provider["expected_chat_messages"]
        );
        assert_eq!(
            Value::Array(to_openai_responses_input(messages)),
            provider["expected_responses_input"]
        );
    }

    #[test]
    fn provider_adapters_request_streams_and_normalize_sse_tool_calls() {
        let chat_client = Arc::new(FakeStreamingClient {
            events: vec![
                json!({"choices":[{"delta":{"content":"hello","tool_calls":[{"index":0,"id":"call-1","function":{"name":"timeline.query","arguments":"{\"include\":"}}]},"finish_reason":null}]}),
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"full\"}"}}]},"finish_reason":"stop"}]}),
            ],
            bodies: Mutex::new(Vec::new()),
        });
        let chat = OpenAiProvider::with_client(
            chat_client.clone(),
            Some("test-key".into()),
            Some("http://fixture/v1".into()),
        );
        let chunks = chat.stream(&provider_request()).unwrap();
        assert!(matches!(&chunks[0], ProviderChunk::TextDelta { text } if text == "hello"));
        let ProviderChunk::MessageEnd {
            stop_reason,
            content,
        } = chunks.last().unwrap()
        else {
            panic!("expected message end");
        };
        assert_eq!(stop_reason, "tool_use");
        assert_eq!(content[1]["name"], "timeline.query");
        assert_eq!(content[1]["input"]["include"], "full");
        assert_eq!(chat_client.bodies.lock().unwrap()[0]["stream"], true);

        let anthropic_client = Arc::new(FakeStreamingClient {
            events: vec![
                json!({"type":"message_start","message":{"id":"msg"}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
                json!({"type":"message_stop"}),
            ],
            bodies: Mutex::new(Vec::new()),
        });
        let anthropic = AnthropicProvider::with_client(
            anthropic_client.clone(),
            Some("test-key".into()),
            Some("http://fixture/v1".into()),
        );
        let chunks = anthropic.stream(&provider_request()).unwrap();
        assert!(matches!(&chunks[0], ProviderChunk::TextDelta { text } if text == "done"));
        assert_eq!(anthropic_client.bodies.lock().unwrap()[0]["stream"], true);
    }

    #[test]
    fn anthropic_requests_cache_their_prefix_and_keep_signed_thinking() {
        let client = Arc::new(FakeStreamingClient {
            events: vec![
                json!({"type":"message_start","message":{"id":"msg"}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Check the cut."}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQB"}}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_1","name":"look","input":{}}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            ],
            bodies: Mutex::new(Vec::new()),
        });
        let provider = AnthropicProvider::with_client(client.clone(), None, None)
            .with_effort(Some(ReasoningEffort::High));
        let chunks = provider.stream(&provider_request()).unwrap();
        let ProviderChunk::MessageEnd {
            stop_reason,
            content,
        } = chunks.last().unwrap()
        else {
            panic!("expected message end");
        };
        assert_eq!(stop_reason, "tool_use");
        assert_eq!(
            content[0],
            json!({"type":"thinking","thinking":"Check the cut.","signature":"EqQB"})
        );
        assert!(!chunks
            .iter()
            .any(|chunk| matches!(chunk, ProviderChunk::TextDelta { .. })));

        let body = &client.bodies.lock().unwrap()[0];
        assert_eq!(body["system"][0]["text"], "fixture-system");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            body["messages"][0]["content"],
            json!([{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}])
        );
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(DEFAULT_ANTHROPIC_MODEL, "claude-opus-5-5");
    }

    #[test]
    fn a_refusal_is_an_error_not_an_answer() {
        let provider = FakeProvider {
            chunks: vec![ProviderChunk::MessageEnd {
                stop_reason: "refusal".into(),
                content: vec![],
            }],
        };
        let mut session = AgentSession::from_provider(
            Box::new(provider),
            None,
            None,
            vec![],
            ToolMap::new(),
            2,
            OpenAiApiMode::Chat,
            None,
        );
        let error = session.run("edit").unwrap_err().to_string();
        assert!(error.contains("REFUSED"), "{error}");
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn a_thinking_mismatch_retries_once_without_earlier_thinking() {
        struct Strict {
            requests: Arc<Mutex<Vec<ProviderRequest>>>,
        }
        impl LlmProvider for Strict {
            fn name(&self) -> ProviderName {
                ProviderName::Anthropic
            }
            fn default_model(&self) -> &'static str {
                "test"
            }
            fn stream(
                &self,
                request: &ProviderRequest,
            ) -> Result<Vec<ProviderChunk>, ProviderError> {
                self.requests.lock().unwrap().push(request.clone());
                if request
                    .messages
                    .iter()
                    .any(|m| m.to_string().contains("\"thinking\""))
                {
                    return Err(ProviderError::new(
                        "invalid_request_error",
                        "messages.1.content.0: Invalid `signature` in `thinking` block.",
                    ));
                }
                Ok(vec![ProviderChunk::MessageEnd {
                    stop_reason: "end_turn".into(),
                    content: vec![json!({"type":"text","text":"done"})],
                }])
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut session = AgentSession::from_provider(
            Box::new(Strict {
                requests: Arc::clone(&requests),
            }),
            None,
            None,
            vec![],
            ToolMap::new(),
            2,
            OpenAiApiMode::Chat,
            None,
        );
        session.messages = vec![
            json!({"role":"user","content":"first"}),
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"old"},{"type":"text","text":"ok"}]}),
        ];
        assert_eq!(session.run("again").unwrap(), "done");
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert!(
            session.messages[1].to_string().contains("old"),
            "the transcript keeps it"
        );
    }

    #[test]
    fn context_is_added_when_it_changes_and_the_conversation_only_grows() {
        struct RecordingProvider {
            requests: Arc<Mutex<Vec<ProviderRequest>>>,
        }
        impl LlmProvider for RecordingProvider {
            fn name(&self) -> ProviderName {
                ProviderName::Anthropic
            }
            fn default_model(&self) -> &'static str {
                "test"
            }
            fn stream(
                &self,
                request: &ProviderRequest,
            ) -> Result<Vec<ProviderChunk>, ProviderError> {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.clone());
                // Two edits, then a look that changes nothing, then done.
                let (stop, content) = match requests.len() {
                    1 | 2 => (
                        "tool_use",
                        json!({"type":"tool_use","id":format!("edit_{}", requests.len()),"name":"edit","input":{}}),
                    ),
                    3 => (
                        "tool_use",
                        json!({"type":"tool_use","id":"look_3","name":"look","input":{}}),
                    ),
                    _ => ("end_turn", json!({"type":"text","text":"done"})),
                };
                Ok(vec![ProviderChunk::MessageEnd {
                    stop_reason: stop.into(),
                    content: vec![
                        json!({"type":"thinking","thinking":"","signature":"sig"}),
                        content,
                    ],
                }])
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let revision = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let mut tools = ToolMap::new();
        let edited = Arc::clone(&revision);
        tools.insert(
            "edit".into(),
            AgentTool::new("edit", "edit", json!({"type":"object"}), move |_| {
                edited.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"staged":true}))
            }),
        );
        tools.insert(
            "look".into(),
            AgentTool::new("look", "look", json!({"type":"object"}), |_| Ok(json!({}))),
        );
        let read = Arc::clone(&revision);
        let mut session = AgentSession::from_provider(
            Box::new(RecordingProvider {
                requests: Arc::clone(&requests),
            }),
            None,
            Some("fixed".into()),
            vec![],
            tools,
            8,
            OpenAiApiMode::Chat,
            None,
        )
        .with_context(Arc::new(move || {
            Ok(format!("revision {}", read.load(Ordering::SeqCst)))
        }));
        assert_eq!(session.run("cut it").unwrap(), "done");
        let requests = requests.lock().unwrap();
        assert!(requests.iter().all(|request| request.system == "fixed"));
        for pair in requests.windows(2) {
            assert_eq!(
                pair[1].messages[..pair[0].messages.len()],
                pair[0].messages[..],
                "every request extends the one before it"
            );
        }
        let snapshots = requests
            .last()
            .unwrap()
            .messages
            .iter()
            .filter_map(|message| message["content"][0]["text"].as_str())
            .filter(|text| text.starts_with(context::SNAPSHOT_HEADER))
            .map(|text| text.lines().last().unwrap().to_owned())
            .collect::<Vec<_>>();
        // A snapshot for each revision, none for the look that changed nothing.
        assert_eq!(snapshots, ["revision 1", "revision 2", "revision 3"]);
        assert_eq!(
            requests[3].messages[2]["content"][0]["signature"], "sig",
            "thinking goes back unchanged"
        );
        let names = session
            .normalized_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["edit", "look", "session.read"]);
        assert!(session.execute_tool("tools.call", json!({})).is_err());
    }

    #[test]
    fn context_retry_shrinks_only_provider_view_and_never_repeats_an_edit() {
        struct LimitedProvider {
            requests: Arc<Mutex<Vec<ProviderRequest>>>,
        }
        impl LlmProvider for LimitedProvider {
            fn name(&self) -> ProviderName {
                ProviderName::Anthropic
            }
            fn default_model(&self) -> &'static str {
                "test"
            }
            fn stream(
                &self,
                request: &ProviderRequest,
            ) -> Result<Vec<ProviderChunk>, ProviderError> {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.clone());
                match requests.len() {
                    1 => Err(ProviderError::new(
                        "context_length_exceeded",
                        "prompt is too long",
                    )),
                    2 => Ok(vec![ProviderChunk::MessageEnd {
                        stop_reason: "tool_use".into(),
                        content: vec![
                            json!({"type":"tool_use","id":"edit-1","name":"edit","input":{}}),
                        ],
                    }]),
                    _ => Ok(vec![ProviderChunk::MessageEnd {
                        stop_reason: "end_turn".into(),
                        content: vec![json!({"type":"text","text":"awaiting review"})],
                    }]),
                }
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = executions.clone();
        let mut tools = ToolMap::new();
        tools.insert(
            "edit".into(),
            AgentTool::new("edit", "stage", json!({"type":"object"}), move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"status":"awaiting_review","revision":7,"evidence":"x".repeat(100_000)}))
            }),
        );
        let mut session = AgentSession::from_provider(
            Box::new(LimitedProvider {
                requests: requests.clone(),
            }),
            None,
            None,
            vec![],
            tools,
            3,
            OpenAiApiMode::Chat,
            None,
        );
        assert_eq!(
            session
                .run("Preserve the music and trim the opening")
                .unwrap(),
            "awaiting review"
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(
            session.context_budget.max_input_units,
            ContextBudget::default().max_input_units / 2
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            requests[2].messages.last().unwrap()["content"][0]["content"]
                .as_str()
                .unwrap()
                .contains("session.read")
        );
        assert!(
            session.messages[2]["content"][0]["content"]
                .as_str()
                .unwrap()
                .len()
                > 100_000
        );
    }

    #[test]
    fn cancellation_keeps_results_of_completed_tools_and_closes_pending_calls() {
        let provider = FakeProvider {
            chunks: vec![ProviderChunk::MessageEnd {
                stop_reason: "tool_use".into(),
                content: vec![
                    json!({"type":"tool_use","id":"first","name":"first","input":{}}),
                    json!({"type":"tool_use","id":"second","name":"second","input":{}}),
                ],
            }],
        };
        let token = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));
        let stop = token.clone();
        let mut tools = ToolMap::new();
        tools.insert(
            "first".into(),
            AgentTool::new("first", "stage", json!({"type":"object"}), move |_| {
                stop.lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .store(true, Ordering::SeqCst);
                Ok(json!({"status":"awaiting_review","proposalId":"p1"}))
            }),
        );
        tools.insert(
            "second".into(),
            AgentTool::new("second", "must not run", json!({"type":"object"}), |_| {
                panic!("cancelled tool executed")
            }),
        );
        let mut session = AgentSession::from_provider(
            Box::new(provider),
            None,
            None,
            vec![],
            tools,
            2,
            OpenAiApiMode::Chat,
            None,
        );
        *token.lock().unwrap() = Some(session.abort_handle());
        session.run("make a proposal").unwrap();
        let blocks = session.messages[2]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["is_error"], false);
        assert_eq!(blocks[1]["is_error"], true);
        assert!(blocks[0]["content"].as_str().unwrap().contains("p1"));
    }

    #[test]
    fn frames_and_previews_reach_the_model_under_the_default_budget() {
        struct RecordingProvider {
            requests: Arc<Mutex<Vec<ProviderRequest>>>,
        }
        impl LlmProvider for RecordingProvider {
            fn name(&self) -> ProviderName {
                ProviderName::Anthropic
            }
            fn default_model(&self) -> &'static str {
                "test"
            }
            fn stream(
                &self,
                request: &ProviderRequest,
            ) -> Result<Vec<ProviderChunk>, ProviderError> {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.clone());
                let first = requests.len() == 1;
                Ok(vec![ProviderChunk::MessageEnd {
                    stop_reason: if first { "tool_use" } else { "end_turn" }.into(),
                    content: if first {
                        vec![json!({"type":"tool_use","id":"call_1","name":"look","input":{}})]
                    } else {
                        vec![json!({"type":"text","text":"done"})]
                    },
                }])
            }
        }
        let frame = base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3]);
        let frames = (0..MAX_TOOL_RESULT_IMAGES)
            .map(|index| json!({"at": format!("{index}s"), "imageBase64": frame}))
            .collect::<Vec<_>>();
        // A full look of frames, and an edit whose diff is too long to keep
        // whole: its text is archived, its before/after preview is not.
        for (result, images) in [
            (json!({"frames": frames}), MAX_TOOL_RESULT_IMAGES),
            (
                json!({
                    "diff": "+    clip c1 media=m1 at=00:00:00:00\n".repeat(1024),
                    "preview": {"before": {"imageBase64": frame}, "after": {"imageBase64": frame}},
                }),
                2,
            ),
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let mut tools = ToolMap::new();
            tools.insert(
                "look".into(),
                AgentTool::new("look", "look", json!({"type":"object"}), move |_| {
                    Ok(result.clone())
                }),
            );
            let mut session = AgentSession::from_provider(
                Box::new(RecordingProvider {
                    requests: Arc::clone(&requests),
                }),
                None,
                // About the size of the instructions, and a long document.
                Some("x".repeat(12 * 1024)),
                vec![],
                tools,
                4,
                OpenAiApiMode::Chat,
                None,
            )
            .with_context(Arc::new(|| Ok("y".repeat(48 * 1024))));
            assert_eq!(session.run("look at it").unwrap(), "done");
            let requests = requests.lock().unwrap();
            let result = &requests[1].messages.last().unwrap()["content"][0]["content"];
            let seen = result
                .as_array()
                .unwrap()
                .iter()
                .filter(|block| block["type"] == "image")
                .count();
            assert_eq!(seen, images, "{result}");
        }
    }

    #[test]
    fn multimodal_results_are_size_checked_and_canonicalized() {
        let encoded = base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3]);
        let value = normalize_tool_result(json!({
            "type": "multimodal_tool_result",
            "images": [{"mediaType": "image/png", "data": encoded}],
        }))
        .unwrap();
        assert_eq!(value.as_array().unwrap()[0]["type"], "text");
        assert_eq!(
            value.as_array().unwrap()[1]["source"]["media_type"],
            "image/png"
        );
    }

    #[test]
    fn native_inline_frames_become_images_with_timing_metadata() {
        let result = normalize_tool_result(json!({"revision":4,"frames":[
            {"time":1.5,"path":"frame.jpg","imageBase64":"AQID","mediaType":"image/jpeg"},
            {"time":2.0,"path":"frame.png","imageBase64":"AQID"},
        ]}))
        .unwrap();
        let blocks = result.as_array().unwrap();
        let metadata: Value = serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(metadata["frames"][0]["time"], 1.5);
        assert_eq!(metadata["frames"][0]["imageIndex"], 0);
        assert!(metadata["frames"][0].get("imageBase64").is_none());
        assert_eq!(blocks[1]["type"], "image");
        // The image keeps the type it was inlined with; PNG is the default.
        assert_eq!(blocks[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(blocks[2]["source"]["media_type"], "image/png");
    }

    #[test]
    fn malformed_tool_turn_is_rejected_before_execution_or_history_append() {
        for content in [
            vec![],
            vec![
                json!({"type":"tool_use","id":"duplicate","name":"edit","input":{}}),
                json!({"type":"tool_use","id":"duplicate","name":"edit","input":{}}),
            ],
        ] {
            let provider = FakeProvider {
                chunks: vec![ProviderChunk::MessageEnd {
                    stop_reason: "tool_use".into(),
                    content,
                }],
            };
            let mut session = AgentSession::from_provider(
                Box::new(provider),
                None,
                None,
                vec![],
                ToolMap::new(),
                2,
                OpenAiApiMode::Chat,
                None,
            );
            assert!(session
                .run("edit")
                .unwrap_err()
                .to_string()
                .contains("INVALID_PROVIDER_RESPONSE"));
            assert_eq!(session.messages.len(), 1);
        }
    }

    #[test]
    fn agent_events_expose_bounded_tool_summaries_without_raw_payloads() {
        let targets = summarize_targets(&json!({
            "command": {
                "type": "update_clip",
                "clipId": "clip-1",
                "trackId": "track-1",
            },
            "sourceId": "source-1",
        }));
        assert_eq!(targets.sources, vec!["source-1"]);
        assert_eq!(targets.clips, vec!["clip-1"]);
        assert_eq!(targets.tracks, vec!["track-1"]);

        let summary = summarize_tool_result(&json!({
            "revision": 9,
            "status": "completed",
            "clipId": "clip-1",
            "frames": [{
                "path": "/project/.cut/cache/derived/frame.png",
                "imageBase64": "should-not-be-retained",
            }],
        }));
        assert_eq!(summary.revision, Some(9));
        assert_eq!(summary.status.as_deref(), Some("completed"));
        assert_eq!(
            summary.artifact_paths,
            vec!["/project/.cut/cache/derived/frame.png"]
        );
        assert!(!format!("{summary:?}").contains("should-not-be-retained"));

        let sensitive = ToolResultSummary::from_error("authorization: bearer secret");
        assert_eq!(
            sensitive.error.as_deref(),
            Some("sensitive tool error redacted")
        );
    }
}
