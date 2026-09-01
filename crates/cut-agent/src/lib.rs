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
use std::time::Duration;

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";
pub const DEFAULT_RESPONSES_MODEL: &str = "gpt-5.6-luna";

const DEFAULT_SYSTEM_PROMPT: &str = "You are a video editing assistant for Cut.
The project file is project.cut.json (v2).
Inspect the timeline before editing. Use structured project_edit commands for mutations, batch related changes, and render a draft before a full render. Agent edits are staged as proposals for user review; do not claim a staged edit is applied until it is accepted.
Use skills.read to load a playbook body when a skill is relevant.
Do not call a tool named edit.";

pub const MAX_TOOL_RESULT_IMAGES: usize = 8;
pub const MAX_TOOL_RESULT_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;

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

pub fn default_model(provider: ProviderName, api_mode: OpenAiApiMode) -> &'static str {
    match (provider, api_mode) {
        (ProviderName::Anthropic, _) => DEFAULT_ANTHROPIC_MODEL,
        (ProviderName::Openai, OpenAiApiMode::Chat) => DEFAULT_OPENAI_MODEL,
        (ProviderName::Openai, OpenAiApiMode::Responses) => DEFAULT_RESPONSES_MODEL,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "input_schema")]
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    pub model: String,
    pub system: String,
    pub tools: Vec<NormalizedTool>,
    pub messages: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderChunk {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "message_end")]
    MessageEnd {
        #[serde(rename = "stopReason")]
        stop_reason: String,
        content: Vec<Value>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    pub code: String,
    pub message: String,
}

impl ProviderError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProviderError {}

/// Provider interface used by AgentSession and deterministic conformance
/// fixtures. Implementations return normalized chunks in response order.
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> ProviderName;
    fn default_model(&self) -> &'static str;
    fn stream(&self, request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError>;
}

/// Injectable JSON HTTP boundary. Tests can replace it without a network.
pub trait JsonHttpClient: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<Value, ProviderError>;

    /// Post a streaming request and return decoded SSE data events. A test
    /// client that only implements JSON can use the default one-value
    /// fallback; compatible HTTP servers that support SSE override this.
    fn post_stream(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<Vec<Value>, ProviderError> {
        self.post_json(url, headers, body).map(|value| vec![value])
    }
}

/// Default synchronous HTTP client for provider adapters.
#[derive(Clone, Debug)]
pub struct UreqHttpClient {
    timeout: Duration,
}

impl UreqHttpClient {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for UreqHttpClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(120))
    }
}

impl JsonHttpClient for UreqHttpClient {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<Value, ProviderError> {
        let body = self.send(url, headers, body)?;
        serde_json::from_str(&body).map_err(|error| {
            ProviderError::new(
                "PROVIDER_PROTOCOL_ERROR",
                format!("provider returned invalid JSON: {error}"),
            )
        })
    }

    fn post_stream(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<Vec<Value>, ProviderError> {
        let body = self.send(url, headers, body)?;
        parse_stream_events(&body)
    }
}

impl UreqHttpClient {
    fn send(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<String, ProviderError> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build();
        let agent: ureq::Agent = config.into();
        let mut request = agent.post(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let mut response = request.send_json(body).map_err(|error| {
            ProviderError::new(
                "PROVIDER_HTTP_ERROR",
                format!("provider request failed: {error}"),
            )
        })?;
        response.body_mut().read_to_string().map_err(|error| {
            ProviderError::new(
                "PROVIDER_HTTP_ERROR",
                format!("provider response read failed: {error}"),
            )
        })
    }
}

fn parse_stream_events(body: &str) -> Result<Vec<Value>, ProviderError> {
    if let Ok(value) = serde_json::from_str::<Value>(body.trim()) {
        return Ok(vec![value]);
    }
    let mut events = Vec::new();
    for line in body.lines() {
        let data = line.trim().strip_prefix("data:").map(str::trim);
        let Some(data) = data else { continue };
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        let value = serde_json::from_str(data).map_err(|error| {
            ProviderError::new(
                "PROVIDER_PROTOCOL_ERROR",
                format!("provider returned invalid stream JSON: {error}"),
            )
        })?;
        events.push(value);
    }
    if events.is_empty() {
        return Err(ProviderError::new(
            "PROVIDER_PROTOCOL_ERROR",
            "provider returned neither JSON nor SSE data events",
        ));
    }
    Ok(events)
}

/// Anthropic messages adapter.
pub struct AnthropicProvider {
    client: Arc<dyn JsonHttpClient>,
    api_key: Option<String>,
    base_url: String,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .finish()
    }
}

impl AnthropicProvider {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        Self::with_client(Arc::new(UreqHttpClient::default()), api_key, base_url)
    }

    pub fn with_client(
        client: Arc<dyn JsonHttpClient>,
        api_key: Option<String>,
        base_url: Option<String>,
    ) -> Self {
        Self {
            client,
            api_key,
            base_url: base_url
                .unwrap_or_else(|| "https://api.anthropic.com/v1".into())
                .trim_end_matches('/')
                .to_owned(),
        }
    }
}

impl LlmProvider for AnthropicProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Anthropic
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_ANTHROPIC_MODEL
    }

    fn stream(&self, request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![
            ("anthropic-version".into(), "2023-06-01".into()),
            ("content-type".into(), "application/json".into()),
        ];
        if let Some(key) = &self.api_key {
            headers.push(("x-api-key".into(), key.clone()));
        }
        let body = json!({
            "model": request.model,
            "max_tokens": 8192,
            "system": request.system,
            "tools": request.tools.iter().map(anthropic_tool).collect::<Vec<_>>(),
            "messages": request.messages,
            "stream": true,
        });
        let events =
            self.client
                .post_stream(&format!("{}/messages", self.base_url), &headers, &body)?;
        if events.len() != 1 || events[0].get("content").is_none() {
            return Ok(anthropic_stream_chunks(&events));
        }
        let response = &events[0];
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = text_from_blocks(&content);
        let mut chunks = Vec::new();
        if !text.is_empty() {
            chunks.push(ProviderChunk::TextDelta { text });
        }
        chunks.push(ProviderChunk::MessageEnd {
            stop_reason: response
                .get("stop_reason")
                .and_then(Value::as_str)
                .unwrap_or("end_turn")
                .to_owned(),
            content,
        });
        Ok(chunks)
    }
}

fn anthropic_stream_chunks(events: &[Value]) -> Vec<ProviderChunk> {
    let mut blocks = Vec::new();
    let mut tool_arguments = BTreeMap::<usize, String>::new();
    let mut stop_reason = "end_turn".to_owned();
    let mut chunks = Vec::new();
    for event in events {
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                set_block(
                    &mut blocks,
                    index,
                    event
                        .get("content_block")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "text", "text": ""})),
                );
            }
            Some("content_block_delta") => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").cloned().unwrap_or_else(|| json!({}));
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !text.is_empty() {
                            ensure_text_block(&mut blocks, index);
                            if let Some(block_text) = blocks[index].get_mut("text") {
                                let current = block_text.as_str().unwrap_or_default().to_owned();
                                *block_text = Value::String(format!("{current}{text}"));
                            }
                            chunks.push(ProviderChunk::TextDelta { text: text.into() });
                        }
                    }
                    Some("input_json_delta") => {
                        tool_arguments.entry(index).or_default().push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        );
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(value) = event
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    stop_reason = value.to_owned();
                }
            }
            _ => {}
        }
    }
    for (index, arguments) in tool_arguments {
        ensure_tool_block(&mut blocks, index);
        blocks[index]["input"] = serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}));
    }
    chunks.push(ProviderChunk::MessageEnd {
        stop_reason: normalize_stop_reason(Some(&stop_reason)),
        content: blocks,
    });
    chunks
}

fn set_block(blocks: &mut Vec<Value>, index: usize, block: Value) {
    while blocks.len() <= index {
        blocks.push(json!({}));
    }
    blocks[index] = block;
}

fn ensure_text_block(blocks: &mut Vec<Value>, index: usize) {
    if blocks
        .get(index)
        .and_then(|block| block.get("type"))
        .and_then(Value::as_str)
        != Some("text")
    {
        set_block(blocks, index, json!({"type": "text", "text": ""}));
    }
}

fn ensure_tool_block(blocks: &mut Vec<Value>, index: usize) {
    if blocks
        .get(index)
        .and_then(|block| block.get("type"))
        .and_then(Value::as_str)
        != Some("tool_use")
    {
        set_block(
            blocks,
            index,
            json!({"type": "tool_use", "id": "call_tool", "name": "", "input": {}}),
        );
    }
}

/// OpenAI Chat Completions adapter, including compatible endpoints.
pub struct OpenAiProvider {
    client: Arc<dyn JsonHttpClient>,
    api_key: Option<String>,
    base_url: String,
}

impl fmt::Debug for OpenAiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiProvider")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .finish()
    }
}

impl OpenAiProvider {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        Self::with_client(Arc::new(UreqHttpClient::default()), api_key, base_url)
    }

    pub fn with_client(
        client: Arc<dyn JsonHttpClient>,
        api_key: Option<String>,
        base_url: Option<String>,
    ) -> Self {
        Self {
            client,
            api_key,
            base_url: base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .to_owned(),
        }
    }
}

impl LlmProvider for OpenAiProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Openai
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_OPENAI_MODEL
    }

    fn stream(&self, request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![("content-type".into(), "application/json".into())];
        if let Some(key) = &self.api_key {
            headers.push(("authorization".into(), format!("Bearer {key}")));
        }
        let mut messages = vec![json!({"role": "system", "content": request.system})];
        messages.extend(to_openai_messages(&request.messages));
        let body = json!({
            "model": request.model,
            "messages": messages,
            "tools": request.tools.iter().map(to_openai_tool).collect::<Vec<_>>(),
            "tool_choice": "auto",
            "stream": true,
        });
        let events = self.client.post_stream(
            &format!("{}/chat/completions", self.base_url),
            &headers,
            &body,
        )?;
        if events.len() != 1
            || events[0]
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .is_none()
        {
            return Ok(openai_chat_stream_chunks(&events));
        }
        let response = &events[0];
        let message = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let content = assistant_to_blocks(&message);
        let text = text_from_blocks(&content);
        let mut chunks = Vec::new();
        if !text.is_empty() {
            chunks.push(ProviderChunk::TextDelta { text });
        }
        let finish = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str);
        chunks.push(ProviderChunk::MessageEnd {
            stop_reason: normalize_stop_reason(finish),
            content,
        });
        Ok(chunks)
    }
}

fn openai_chat_stream_chunks(events: &[Value]) -> Vec<ProviderChunk> {
    let mut text = String::new();
    let mut tool_calls = BTreeMap::<usize, (String, String, String)>::new();
    let mut finish_reason = None;
    let mut chunks = Vec::new();
    for event in events {
        let Some(choice) = event
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            continue;
        };
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        if let Some(value) = delta.get("content").and_then(Value::as_str) {
            if !value.is_empty() {
                text.push_str(value);
                chunks.push(ProviderChunk::TextDelta { text: value.into() });
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let entry = tool_calls.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    entry.0 = id.to_owned();
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        entry.1.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        entry.2.push_str(arguments);
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            finish_reason = Some(reason.to_owned());
        }
    }
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    for (_, (id, name, arguments)) in tool_calls {
        content.push(json!({
            "type": "tool_use",
            "id": if id.is_empty() { format!("call_{name}") } else { id },
            "name": name,
            "input": serde_json::from_str::<Value>(&arguments).unwrap_or_else(|_| json!({})),
        }));
    }
    chunks.push(ProviderChunk::MessageEnd {
        stop_reason: normalize_stop_reason(finish_reason.as_deref()),
        content,
    });
    chunks
}

/// OpenAI Responses API adapter.
pub struct OpenAiResponsesProvider {
    client: Arc<dyn JsonHttpClient>,
    api_key: Option<String>,
    base_url: String,
    reasoning_effort: Option<ReasoningEffort>,
}

impl fmt::Debug for OpenAiResponsesProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesProvider")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .field("reasoning_effort", &self.reasoning_effort)
            .finish()
    }
}

impl OpenAiResponsesProvider {
    pub fn new(
        api_key: Option<String>,
        base_url: Option<String>,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Self {
        Self::with_client(
            Arc::new(UreqHttpClient::default()),
            api_key,
            base_url,
            reasoning_effort,
        )
    }

    pub fn with_client(
        client: Arc<dyn JsonHttpClient>,
        api_key: Option<String>,
        base_url: Option<String>,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Self {
        Self {
            client,
            api_key,
            base_url: base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .to_owned(),
            reasoning_effort,
        }
    }
}

impl LlmProvider for OpenAiResponsesProvider {
    fn name(&self) -> ProviderName {
        ProviderName::Openai
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_RESPONSES_MODEL
    }

    fn stream(&self, request: &ProviderRequest) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![("content-type".into(), "application/json".into())];
        if let Some(key) = &self.api_key {
            headers.push(("authorization".into(), format!("Bearer {key}")));
        }
        let mut body = json!({
            "model": request.model,
            "instructions": request.system,
            "input": to_openai_responses_input(&request.messages),
            "tools": request.tools.iter().map(to_openai_responses_tool).collect::<Vec<_>>(),
            "tool_choice": "auto",
            "stream": true,
        });
        if let Some(effort) = self.reasoning_effort {
            body["reasoning"] = json!({"effort": reasoning_effort_name(effort)});
        }
        let events =
            self.client
                .post_stream(&format!("{}/responses", self.base_url), &headers, &body)?;
        if events.len() != 1 || events[0].get("output").is_none() {
            return Ok(openai_responses_stream_chunks(&events));
        }
        let response = &events[0];
        let content = response_output_to_blocks(response);
        let text = text_from_blocks(&content);
        let mut chunks = Vec::new();
        if !text.is_empty() {
            chunks.push(ProviderChunk::TextDelta { text });
        }
        chunks.push(ProviderChunk::MessageEnd {
            stop_reason: if content.iter().any(is_tool_use) {
                "tool_use".into()
            } else {
                "end_turn".into()
            },
            content,
        });
        Ok(chunks)
    }
}

fn openai_responses_stream_chunks(events: &[Value]) -> Vec<ProviderChunk> {
    let mut text = String::new();
    let mut calls = BTreeMap::<String, (String, String, String)>::new();
    let mut final_response = None;
    let mut chunks = Vec::new();
    for event in events {
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        text.push_str(delta);
                        chunks.push(ProviderChunk::TextDelta { text: delta.into() });
                    }
                }
            }
            Some("response.output_item.added") | Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        let id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("call_tool")
                            .to_owned();
                        let entry = calls.entry(id).or_default();
                        entry.0 = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        entry.1 = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                            entry.2 = arguments.to_owned();
                        }
                    }
                }
            }
            Some("response.function_call_arguments.delta") => {
                let id = event
                    .get("call_id")
                    .or_else(|| event.get("item_id"))
                    .or_else(|| event.get("output_index"))
                    .map(value_key)
                    .unwrap_or_else(|| "0".into());
                let entry = calls.entry(id).or_default();
                if let Some(name) = event.get("name").and_then(Value::as_str) {
                    entry.1 = name.to_owned();
                }
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    entry.2.push_str(delta);
                }
            }
            Some("response.completed") => {
                final_response = event.get("response").cloned();
            }
            Some("error") => {
                chunks.push(ProviderChunk::MessageEnd {
                    stop_reason: "error".into(),
                    content: vec![json!({
                        "type": "text",
                        "text": event.get("message").and_then(Value::as_str).unwrap_or("Responses API stream error")
                    })],
                });
                return chunks;
            }
            _ => {}
        }
    }
    let content = final_response
        .as_ref()
        .map(response_output_to_blocks)
        .unwrap_or_else(|| {
            let mut content = Vec::new();
            if !text.is_empty() {
                content.push(json!({"type": "text", "text": text}));
            }
            for (call_id, (id, name, arguments)) in calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": if call_id.is_empty() { id } else { call_id },
                    "name": name,
                    "input": serde_json::from_str::<Value>(&arguments).unwrap_or_else(|_| json!({})),
                }));
            }
            content
        });
    if text.is_empty() {
        if let Some(value) = content.iter().find_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
        }) {
            chunks.push(ProviderChunk::TextDelta { text: value.into() });
        }
    }
    chunks.push(ProviderChunk::MessageEnd {
        stop_reason: if content.iter().any(is_tool_use) {
            "tool_use".into()
        } else {
            "end_turn".into()
        },
        content,
    });
    chunks
}

fn value_key(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .unwrap_or_else(|| "0".into())
}

fn anthropic_tool(tool: &NormalizedTool) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    })
}

pub fn to_openai_tool(tool: &NormalizedTool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

pub fn to_openai_responses_tool(tool: &NormalizedTool) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    })
}

/// Translate the canonical Anthropic-shaped history to Chat Completions.
pub fn to_openai_messages(messages: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let blocks = content_blocks(message);
        if role == "user" {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                output.push(json!({"role": "user", "content": text}));
                continue;
            }
            let text = text_from_blocks(&blocks);
            if !text.trim().is_empty() {
                output.push(json!({"role": "user", "content": text}));
            }
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (tool_text, image_urls) = tool_result_visuals(&block);
                output.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": tool_text,
                }));
                if !image_urls.is_empty() {
                    let mut content = vec![json!({
                        "type": "text",
                        "text": visual_link_text(id),
                    })];
                    content.extend(
                        image_urls
                            .into_iter()
                            .map(|url| json!({"type": "image_url", "image_url": {"url": url}})),
                    );
                    output.push(json!({"role": "user", "content": content}));
                }
            }
            continue;
        }
        let text = text_from_blocks(&blocks);
        let mut assistant = json!({
            "role": "assistant",
            "content": if text.is_empty() { Value::Null } else { Value::String(text) },
        });
        let tool_calls = blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            .map(|block| {
                json!({
                    "id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                    "type": "function",
                    "function": {
                        "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                        "arguments": serde_json::to_string(block.get("input").unwrap_or(&Value::Null)).unwrap_or_else(|_| "{}".into()),
                    }
                })
            })
            .collect::<Vec<_>>();
        if !tool_calls.is_empty() {
            assistant["tool_calls"] = Value::Array(tool_calls);
        }
        output.push(assistant);
    }
    output
}

/// Translate history to Responses API input items.
pub fn to_openai_responses_input(messages: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let blocks = content_blocks(message);
        if role == "user" {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                output.push(json!({"role": "user", "content": text}));
                continue;
            }
            let text = text_from_blocks(&blocks);
            if !text.trim().is_empty() {
                output.push(json!({"role": "user", "content": text}));
            }
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (tool_text, image_urls) = tool_result_visuals(&block);
                output.push(json!({
                    "type": "function_call_output",
                    "call_id": id,
                    "output": tool_text,
                }));
                if !image_urls.is_empty() {
                    let mut content = vec![json!({
                        "type": "input_text",
                        "text": visual_link_text(id),
                    })];
                    content.extend(
                        image_urls
                            .into_iter()
                            .map(|url| json!({"type": "input_image", "image_url": url})),
                    );
                    output.push(json!({"role": "user", "content": content}));
                }
            }
            continue;
        }
        let text = text_from_blocks(&blocks);
        if !text.trim().is_empty() {
            output.push(json!({"role": "assistant", "content": text}));
        }
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            output.push(json!({
                "type": "function_call",
                "call_id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                "arguments": serde_json::to_string(block.get("input").unwrap_or(&Value::Null)).unwrap_or_else(|_| "{}".into()),
            }));
        }
    }
    output
}

fn assistant_to_blocks(message: &Value) -> Vec<Value> {
    let mut blocks = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let function = call.get("function").cloned().unwrap_or_else(|| json!({}));
            let input = function
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_else(|| json!({}));
            blocks.push(json!({
                "type": "tool_use",
                "id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
                "input": input,
            }));
        }
    }
    blocks
}

fn response_output_to_blocks(response: &Value) -> Vec<Value> {
    let mut blocks = Vec::new();
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                blocks.push(json!({"type": "text", "text": text}));
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                let input = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_else(|| json!({}));
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("call_tool");
                blocks.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "input": input,
                }));
            }
            _ => {}
        }
    }
    if blocks.is_empty() {
        if let Some(text) = response.get("output_text").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(json!({"type": "text", "text": text}));
            }
        }
    }
    blocks
}

fn content_blocks(message: &Value) -> Vec<Value> {
    match message.get("content") {
        Some(Value::Array(blocks)) => blocks.clone(),
        Some(Value::String(text)) => vec![json!({"type": "text", "text": text})],
        _ => Vec::new(),
    }
}

fn text_from_blocks(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .map(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect()
}

fn image_block_to_data_url(block: &Value) -> Option<String> {
    let source = block.get("source")?;
    if source.get("type").and_then(Value::as_str) == Some("url") {
        return source.get("url").and_then(Value::as_str).map(str::to_owned);
    }
    if source.get("type").and_then(Value::as_str) != Some("base64") {
        return None;
    }
    let data = source.get("data").and_then(Value::as_str)?;
    if data.starts_with("data:image/png;base64,") || data.starts_with("data:image/jpeg;base64,") {
        return Some(data.to_owned());
    }
    let media_type = source.get("media_type").and_then(Value::as_str)?;
    matches!(media_type, "image/png" | "image/jpeg")
        .then(|| format!("data:{media_type};base64,{data}"))
}

fn tool_result_visuals(block: &Value) -> (String, Vec<String>) {
    match block.get("content") {
        Some(Value::String(text)) => (text.clone(), Vec::new()),
        Some(Value::Array(parts)) => {
            let mut text = Vec::new();
            let mut images = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => text.push(
                        part.get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                    Some("image") => {
                        if let Some(url) = image_block_to_data_url(part) {
                            images.push(url);
                        }
                    }
                    _ => {}
                }
            }
            (
                if text.is_empty() {
                    if images.is_empty() {
                        serde_json::to_string(parts).unwrap_or_else(|_| "null".into())
                    } else {
                        "Tool result includes visual output.".into()
                    }
                } else {
                    text.join("\n")
                },
                images,
            )
        }
        value => (
            serde_json::to_string(value.unwrap_or(&Value::Null)).unwrap_or_else(|_| "null".into()),
            Vec::new(),
        ),
    }
}

fn visual_link_text(id: &str) -> String {
    format!("Visual output for tool call {id} (see the tool result immediately above).")
}

fn normalize_stop_reason(value: Option<&str>) -> String {
    matches!(value, Some("tool_use" | "tool_calls" | "function_call"))
        .then_some("tool_use")
        .unwrap_or("end_turn")
        .into()
}

fn reasoning_effort_name(value: ReasoningEffort) -> &'static str {
    match value {
        ReasoningEffort::None => "none",
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max => "max",
    }
}

fn is_tool_use(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("tool_use")
}

/// A structured tool available to AgentSession.
pub struct AgentTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    executor: Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>,
}

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
            executor: Arc::new(executor),
        }
    }

    fn execute(&self, input: Value) -> Result<Value, String> {
        (self.executor)(input)
    }
}

pub type ToolMap = BTreeMap<String, AgentTool>;

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    TextDelta(String),
    TurnStarted {
        stop_reason: String,
    },
    ToolCallStarted {
        id: String,
        name: String,
        input: Value,
    },
    ToolCallFinished {
        id: String,
        name: String,
        ok: bool,
        detail: String,
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
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(error) => error.fmt(formatter),
            Self::Tool(error) => write!(formatter, "tool error: {error}"),
            Self::Serialization(error) => write!(formatter, "serialization error: {error}"),
            Self::TurnLimit(limit) => write!(formatter, "agent reached the {limit}-turn limit"),
        }
    }
}

impl std::error::Error for AgentError {}

pub struct AgentSession {
    pub model: String,
    pub provider: ProviderName,
    pub api_mode: OpenAiApiMode,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub system_prompt: String,
    pub messages: Vec<Value>,
    tools: ToolMap,
    llm: Box<dyn LlmProvider>,
    max_turns: usize,
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
    pub max_turns: usize,
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
            max_turns: 32,
        }
    }
}

impl AgentSession {
    pub fn new(options: AgentSessionOptions) -> Self {
        let llm: Box<dyn LlmProvider> = match (options.provider, options.api_mode) {
            (ProviderName::Anthropic, _) => Box::new(AnthropicProvider::new(
                options.api_key.clone(),
                options.base_url.clone(),
            )),
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
        Self::from_provider(
            llm,
            options.model,
            options.system_prompt,
            options.skills,
            options.tools,
            options.max_turns,
            options.api_mode,
            options.reasoning_effort,
        )
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
            llm,
            max_turns: max_turns.max(1),
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
        self.tools
            .values()
            .map(|tool| NormalizedTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect()
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
                    text: final_text.clone(),
                });
                return Ok(final_text);
            }
            let chunks = self
                .llm
                .stream(&ProviderRequest {
                    model: self.model.clone(),
                    system: self.system_prompt.clone(),
                    tools: self.normalized_tools(),
                    messages: self.messages.clone(),
                })
                .map_err(AgentError::Provider)?;
            let mut stop_reason = "end_turn".to_owned();
            let mut content = Vec::new();
            for chunk in chunks {
                match chunk {
                    ProviderChunk::TextDelta { text } => emit(AgentEvent::TextDelta(text)),
                    ProviderChunk::MessageEnd {
                        stop_reason: reason,
                        content: blocks,
                    } => {
                        stop_reason = reason;
                        content = blocks;
                    }
                }
            }
            emit(AgentEvent::TurnStarted {
                stop_reason: stop_reason.clone(),
            });
            self.messages
                .push(json!({"role": "assistant", "content": content}));
            let text = text_from_blocks(&content);
            if stop_reason != "tool_use" {
                final_text.push_str(&text);
                emit(AgentEvent::Done {
                    text: final_text.clone(),
                });
                return Ok(final_text);
            }

            let mut result_blocks = Vec::new();
            for block in content.iter().filter(|block| is_tool_use(block)) {
                if self.is_aborted() {
                    emit(AgentEvent::Aborted);
                    emit(AgentEvent::Done {
                        text: final_text.clone(),
                    });
                    return Ok(final_text);
                }
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
                emit(AgentEvent::ToolCallStarted {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
                let output = match self.tools.get(&name) {
                    Some(tool) => tool.execute(input),
                    None => Err(format!("unknown tool: {name}")),
                };
                let (ok, detail, canonical) = match output {
                    Ok(value) => {
                        let canonical =
                            normalize_tool_result(value).map_err(AgentError::Serialization)?;
                        let detail =
                            serde_json::to_string(&canonical).unwrap_or_else(|_| "null".into());
                        (true, detail, canonical)
                    }
                    Err(error) => {
                        let canonical = Value::String(
                            serde_json::to_string(&json!({"error": error}))
                                .unwrap_or_else(|_| "{\"error\":\"tool failed\"}".into()),
                        );
                        (false, error, canonical)
                    }
                };
                emit(AgentEvent::ToolCallFinished {
                    id: id.clone(),
                    name,
                    ok,
                    detail,
                });
                result_blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": canonical,
                }));
            }
            self.messages
                .push(json!({"role": "user", "content": result_blocks}));
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
        Ok(session)
    }
}

fn normalize_tool_result(value: Value) -> Result<Value, String> {
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

/// Helper for tools that need to stage an agent proposal through cutd.
pub fn stage_proposal(
    runtime: &cut_runtime::RuntimeClient,
    request_key: &str,
    base_revision: u64,
    transactions: Vec<Value>,
) -> Result<cut_runtime::StateSnapshot, AgentError> {
    runtime
        .stage_agent_proposal(request_key, base_revision, transactions)
        .map_err(|error| AgentError::Tool(format!("{}: {}", error.code, error.message)))
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
    fn translates_tool_history_for_both_openai_shapes() {
        let messages = vec![
            json!({"role": "user", "content": "trim it"}),
            json!({"role": "assistant", "content": [
                {"type": "text", "text": "I will"},
                {"type": "tool_use", "id": "call-1", "name": "timeline", "input": {"track": "video-1"}}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call-1", "content": "{\"ok\":true}"}
            ]}),
        ];
        let chat = to_openai_messages(&messages);
        assert_eq!(chat[1]["tool_calls"][0]["function"]["name"], "timeline");
        assert_eq!(chat[2]["role"], "tool");
        let responses = to_openai_responses_input(&messages);
        assert_eq!(responses[2]["type"], "function_call");
        assert_eq!(responses[3]["type"], "function_call_output");
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
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"full\"}"}}]},"finish_reason":"tool_calls"}]}),
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
    fn agent_runs_tool_turn_and_serializes_history() {
        let provider = FakeProvider {
            chunks: vec![ProviderChunk::MessageEnd {
                stop_reason: "end_turn".into(),
                content: vec![json!({"type": "text", "text": "done"})],
            }],
        };
        let mut tools = ToolMap::new();
        tools.insert(
            "timeline".into(),
            AgentTool::new("timeline", "inspect", json!({"type": "object"}), |_| {
                Ok(json!({}))
            }),
        );
        let mut session = AgentSession::from_provider(
            Box::new(provider),
            None,
            None,
            Vec::new(),
            tools,
            2,
            OpenAiApiMode::Chat,
            None,
        );
        assert_eq!(session.run("hello").unwrap(), "done");
        assert_eq!(session.messages.len(), 2);
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
}
