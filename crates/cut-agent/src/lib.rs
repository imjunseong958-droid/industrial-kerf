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

mod context;
pub use context::ContextBudget;

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";
pub const DEFAULT_RESPONSES_MODEL: &str = "gpt-5.6-luna";

const DEFAULT_SYSTEM_PROMPT: &str = "You are a video editing assistant for Cut. \
Edits you make are proposals: do not claim one is applied until the user accepts it.";

pub const MAX_TOOL_RESULT_IMAGES: usize = 8;
pub const MAX_TOOL_RESULT_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;

const MAX_AGENT_EVENT_TEXT_BYTES: usize = 8 * 1024;
const MAX_AGENT_SUMMARY_TEXT_BYTES: usize = 512;
const MAX_AGENT_SUMMARY_ITEMS: usize = 8;
const MAX_AGENT_SUMMARY_NODES: usize = 256;

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
    fn get_json(&self, _url: &str, _headers: &[(String, String)]) -> Result<Value, ProviderError> {
        Err(ProviderError::new(
            "PROVIDER_HTTP_ERROR",
            "model discovery is not supported by this HTTP client",
        ))
    }

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
    fn get_json(&self, url: &str, headers: &[(String, String)]) -> Result<Value, ProviderError> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build();
        let agent: ureq::Agent = config.into();
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let mut response = request.call().map_err(|error| {
            ProviderError::new(
                "PROVIDER_HTTP_ERROR",
                format!("provider model check failed: {error}"),
            )
        })?;
        response.body_mut().read_json().map_err(|error| {
            ProviderError::new(
                "PROVIDER_PROTOCOL_ERROR",
                format!("provider returned an invalid model list: {error}"),
            )
        })
    }

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

/// Verify that provider credentials can see the selected model without
/// consuming generation tokens.
pub fn check_model_access(
    provider: ProviderName,
    api_key: &str,
    base_url: Option<&str>,
    model: &str,
) -> Result<(), ProviderError> {
    let base_url = base_url
        .unwrap_or(match provider {
            ProviderName::Anthropic => "https://api.anthropic.com/v1",
            ProviderName::Openai => "https://api.openai.com/v1",
        })
        .trim_end_matches('/');
    let headers = match provider {
        ProviderName::Anthropic => vec![
            ("anthropic-version".into(), "2023-06-01".into()),
            ("x-api-key".into(), api_key.into()),
        ],
        ProviderName::Openai => vec![("authorization".into(), format!("Bearer {api_key}"))],
    };
    let response = UreqHttpClient::new(Duration::from_secs(15))
        .get_json(&format!("{base_url}/models"), &headers)?;
    if model_list_contains(&response, model) {
        Ok(())
    } else {
        Err(ProviderError::new(
            "MODEL_UNAVAILABLE",
            format!("model {model} is not available for these credentials"),
        ))
    }
}

fn model_list_contains(response: &Value, model: &str) -> bool {
    response
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models
                .iter()
                .any(|candidate| candidate.get("id").and_then(Value::as_str) == Some(model))
        })
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
            stop_reason: normalize_openai_stop_reason(finish, &content),
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
        stop_reason: normalize_openai_stop_reason(finish_reason.as_deref(), &content),
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

fn normalize_openai_stop_reason(value: Option<&str>, content: &[Value]) -> String {
    if matches!(value, Some("stop" | "STOP")) && content.iter().any(is_tool_use) {
        "tool_use".into()
    } else {
        normalize_stop_reason(value)
    }
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
/// document. It follows the fixed system prompt, so the fixed part stays
/// cacheable while the context tracks every edit.
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

/// Bounded entity references extracted from a tool call for human-facing
/// progress displays. Raw tool input is intentionally not part of an event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTargetSummary {
    pub sources: Vec<String>,
    pub clips: Vec<String>,
    pub tracks: Vec<String>,
    pub overlays: Vec<String>,
    pub jobs: Vec<String>,
}

/// Bounded, human-facing metadata extracted from a tool result. The canonical
/// result remains in the agent conversation; this summary is the only result
/// shape exposed through desktop event transport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolResultSummary {
    pub targets: AgentTargetSummary,
    pub artifact_paths: Vec<String>,
    pub observation: Option<ObservationSummary>,
    pub revision: Option<u64>,
    pub status: Option<String>,
    pub count: Option<usize>,
    pub error: Option<String>,
    pub truncated: bool,
}

/// Compact observation metadata used in the live agent activity stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservationSummary {
    pub id: String,
    pub phase: Option<String>,
    pub range: Option<String>,
    pub evidence: Vec<String>,
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

    /// The fixed system prompt followed by the current context.
    pub fn system(&self) -> String {
        match self.context.as_ref().map(|context| context()) {
            None => self.system_prompt.clone(),
            Some(Ok(context)) if context.is_empty() => self.system_prompt.clone(),
            Some(Ok(context)) => format!("{}\n\n{context}", self.system_prompt),
            Some(Err(error)) => format!(
                "{}\n\nThe current project could not be read: {error}",
                self.system_prompt
            ),
        }
    }

    fn provider_request(&self, budget: &ContextBudget) -> Result<ProviderRequest, AgentError> {
        let tools = self.normalized_tools();
        let system = self.system();
        let fixed = system.len()
            + context::units(
                &serde_json::to_value(&tools)
                    .map_err(|error| AgentError::Serialization(error.to_string()))?,
            )
            + tools.len() * 256
            + 1024;
        let available = budget.max_input_units.checked_sub(fixed)
            .ok_or_else(|| AgentError::ContextBudget("System prompt, project context, and tool definitions exceed the input allowance. Increase the budget for this model.".into()))?;
        let messages = context::project(&self.messages, available, budget)
            .map_err(AgentError::ContextBudget)?;
        Ok(ProviderRequest {
            model: self.model.clone(),
            system,
            tools,
            messages,
        })
    }

    fn stream_bounded(&mut self) -> Result<Vec<ProviderChunk>, AgentError> {
        let request = self.provider_request(&self.context_budget)?;
        match self.llm.stream(&request) {
            Ok(chunks) => Ok(chunks),
            Err(error) if is_context_limit(&error) => {
                // No tools have run for this request. Retry once with a smaller
                // view; never restart an edit or replay a mutating tool call.
                let mut smaller = self.context_budget.clone();
                smaller.max_input_units /= 2;
                smaller.recent_tool_messages = 0;
                let request = self.provider_request(&smaller)?;
                self.context_budget = smaller;
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
        Ok(session)
    }
}

#[derive(Clone, Copy)]
enum TargetKind {
    Source,
    Clip,
    Track,
    Overlay,
    Job,
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

fn summarize_targets(value: &Value) -> AgentTargetSummary {
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
                    "overlayId" => add_target(
                        &mut summary.overlays,
                        value.as_str(),
                        MAX_AGENT_SUMMARY_ITEMS,
                    ),
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
                        Some(TargetKind::Overlay) => add_target(
                            &mut summary.overlays,
                            value.as_str(),
                            MAX_AGENT_SUMMARY_ITEMS,
                        ),
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
        "overlay" | "overlays" => Some(TargetKind::Overlay),
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

fn summarize_tool_result(value: &Value) -> ToolResultSummary {
    let mut summary = ToolResultSummary {
        targets: summarize_targets(value),
        observation: find_observation(value),
        ..ToolResultSummary::default()
    };
    let mut nodes = 0;
    collect_result_fields(value, &mut summary, &mut nodes);
    summary
}

fn find_observation(value: &Value) -> Option<ObservationSummary> {
    match value {
        Value::Array(values) => values.iter().find_map(find_observation),
        Value::Object(object) => {
            if let Some(reference) = object.get("observationRef") {
                let id = reference.get("id").and_then(Value::as_str)?.to_owned();
                let range = reference.get("range").and_then(|range| {
                    Some(format!(
                        "{:.2}–{:.2}s",
                        range.get("start")?.as_f64()?,
                        range.get("end")?.as_f64()?
                    ))
                });
                let evidence = reference
                    .get("evidence")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .take(MAX_AGENT_SUMMARY_ITEMS)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                return Some(ObservationSummary {
                    id: bounded_text(&id, MAX_AGENT_SUMMARY_TEXT_BYTES),
                    phase: reference
                        .get("phase")
                        .and_then(Value::as_str)
                        .map(|phase| bounded_text(phase, MAX_AGENT_SUMMARY_TEXT_BYTES)),
                    range,
                    evidence,
                });
            }
            object.values().find_map(find_observation)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
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
    fn from_error(error: &str) -> Self {
        Self {
            error: Some(compact_error(error)),
            ..Self::default()
        }
    }
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
    fn context_is_read_fresh_after_the_fixed_prompt_for_every_request() {
        struct RecordingProvider {
            systems: Arc<Mutex<Vec<String>>>,
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
                let mut systems = self.systems.lock().unwrap();
                systems.push(request.system.clone());
                let content = if systems.len() == 1 {
                    vec![json!({"type":"tool_use","id":"call_1","name":"edit","input":{}})]
                } else {
                    vec![json!({"type":"text","text":"done"})]
                };
                Ok(vec![ProviderChunk::MessageEnd {
                    stop_reason: if systems.len() == 1 {
                        "tool_use"
                    } else {
                        "end_turn"
                    }
                    .into(),
                    content,
                }])
            }
        }
        let systems = Arc::new(Mutex::new(Vec::new()));
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
        let read = Arc::clone(&revision);
        let mut session = AgentSession::from_provider(
            Box::new(RecordingProvider {
                systems: Arc::clone(&systems),
            }),
            None,
            Some("fixed".into()),
            vec![],
            tools,
            4,
            OpenAiApiMode::Chat,
            None,
        )
        .with_context(Arc::new(move || {
            Ok(format!("revision {}", read.load(Ordering::SeqCst)))
        }));
        assert_eq!(session.run("cut it").unwrap(), "done");
        assert_eq!(
            *systems.lock().unwrap(),
            vec!["fixed\n\nrevision 1", "fixed\n\nrevision 2"]
        );
        assert_eq!(session.system_prompt, "fixed");
        let names = session
            .normalized_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["edit", "session.read"]);
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

        let observation = summarize_tool_result(&json!({
            "action": "media.observe",
            "observationRef": {
                "id": "observation-123",
                "phase": "targeted",
                "range": {"start": 4.0, "end": 8.0},
                "evidence": ["frames"]
            }
        }))
        .observation
        .expect("observation summary");
        assert_eq!(observation.phase.as_deref(), Some("targeted"));
        assert_eq!(observation.range.as_deref(), Some("4.00–8.00s"));
        assert!(!format!("{summary:?}").contains("should-not-be-retained"));

        let sensitive = ToolResultSummary::from_error("authorization: bearer secret");
        assert_eq!(
            sensitive.error.as_deref(),
            Some("sensitive tool error redacted")
        );
    }

    #[test]
    fn model_list_requires_an_exact_visible_model_id() {
        let response = json!({
            "data": [
                {"id": "gpt-test"},
                {"id": "provider/model-v2"}
            ]
        });
        assert!(model_list_contains(&response, "provider/model-v2"));
        assert!(!model_list_contains(&response, "model-v2"));
        assert!(!model_list_contains(&json!({}), "gpt-test"));
    }
}
