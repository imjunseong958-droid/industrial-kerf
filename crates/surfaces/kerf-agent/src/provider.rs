//! Provider adapters. The session keeps one Anthropic-shaped history; each
//! adapter translates it to its API and normalizes the streamed reply.
//!
//! Each API's requests and replies are types of its own module (`anthropic`, `chat`,
//! `responses`): a request is written from them, and each thing a server says is read into
//! them once, as it comes. What stays JSON is what has no one shape: the transcript the
//! session keeps, a tool's arguments and schema, and what a tool answered.

use crate::{ProviderName, ReasoningEffort};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

mod anthropic;
mod chat;
mod responses;
mod said;
mod transcript;

use transcript::Block;

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";
pub const DEFAULT_RESPONSES_MODEL: &str = "gpt-5.6-luna";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedTool {
    pub name: String,
    pub description: String,
    /// A JSON Schema, each tool's own (`AgentTool::input_schema`), so it stays JSON.
    #[serde(rename = "input_schema")]
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    pub model: String,
    pub system: String,
    pub tools: Vec<NormalizedTool>,
    /// The session's transcript, in the Anthropic Messages API's shape. It stays JSON:
    /// the session keeps it on disk and `session.read` points into it.
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
        /// The reply's blocks, as the transcript keeps them (see [`ProviderRequest::messages`]).
        content: Vec<Value>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    pub code: String,
    pub message: String,
}

impl ProviderError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
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

    /// As [`LlmProvider::stream`], telling `on_text` each piece of the reply's
    /// text as it arrives, so a person reads it while the model writes. Once
    /// `stop` says so the rest of the reply is not read. One that cannot
    /// stream tells the whole text once the reply is in.
    fn stream_live(
        &self,
        request: &ProviderRequest,
        on_text: &mut dyn FnMut(&str),
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        // The reply is read whole before any of it is told, so there is no rest to stop.
        let _ = stop;
        let chunks = self.stream(request)?;
        for chunk in &chunks {
            if let ProviderChunk::TextDelta { text } = chunk {
                on_text(text);
            }
        }
        Ok(chunks)
    }
}

/// Told of each thing a server says as it is read, with the text it adds to the reply, if
/// any; answers whether to read on.
type OnSaid<'a> = dyn FnMut(Option<&str>) -> bool + 'a;

/// Run one exchange, telling `on_text` its text as each event brings it, or
/// all of it at the end where no event did (a reply that came whole).
fn relay_live(
    exchange: impl FnOnce(&mut OnSaid) -> Result<Vec<ProviderChunk>, ProviderError>,
    on_text: &mut dyn FnMut(&str),
    stop: &dyn Fn() -> bool,
) -> Result<Vec<ProviderChunk>, ProviderError> {
    let mut live = false;
    let chunks = exchange(&mut |text| {
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            live = true;
            on_text(text);
        }
        !stop()
    })?;
    if !live {
        for chunk in &chunks {
            if let ProviderChunk::TextDelta { text } = chunk {
                on_text(text);
            }
        }
    }
    Ok(chunks)
}

/// One thing an API's server says, read once: an event of a streamed reply, or the reply
/// whole.
trait Said: Sized {
    /// What `said` says; something that is not an object says nothing that is read.
    fn read(said: &Value) -> Self;

    /// The text it adds to the reply as it streams.
    fn text(&self) -> Option<&str>;
}

/// Send `request` to `url` and read each thing the server says as an `S`, once, as it comes:
/// `on_said` is told the text each adds and answers whether to read on.
fn exchange<S: Said>(
    client: &dyn JsonHttpClient,
    url: &str,
    headers: &[(String, String)],
    request: &impl Serialize,
    on_said: &mut OnSaid,
) -> Result<Vec<S>, ProviderError> {
    let body = json(request);
    let mut read = Vec::new();
    let said = client.post_stream_each(url, headers, &body, &mut |said| {
        let said = S::read(said);
        let more = on_said(said.text());
        read.push(said);
        more
    })?;
    // A client that reads the whole reply before handing it over gives back what it did
    // not hand over too; the reply is all of it, as it always was.
    let handed = read.len();
    read.extend(said.iter().skip(handed).map(S::read));
    Ok(read)
}

/// Injectable JSON HTTP boundary. Tests can replace it without a network.
///
/// It carries JSON, not the APIs' types: one client serves all three, and each adapter
/// reads what it hands over into its own types, once.
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

    /// As [`JsonHttpClient::post_stream`], handing `on_event` each event as
    /// it is read; once it answers `false` the rest is not read. The default
    /// reads them all first.
    fn post_stream_each(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        on_event: &mut dyn FnMut(&Value) -> bool,
    ) -> Result<Vec<Value>, ProviderError> {
        let events = self.post_stream(url, headers, body)?;
        for event in &events {
            if !on_event(event) {
                break;
            }
        }
        Ok(events)
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
    /// Ten minutes: a response is read whole, and a model that thinks
    /// before a long answer can take several.
    fn default() -> Self {
        Self::new(Duration::from_secs(600))
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
        self.post_stream_each(url, headers, body, &mut |_| true)
    }

    fn post_stream_each(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        on_event: &mut dyn FnMut(&Value) -> bool,
    ) -> Result<Vec<Value>, ProviderError> {
        let mut response = self.request(url, headers, body)?;
        let reader = std::io::BufReader::new(response.body_mut().as_reader());
        let mut events = Vec::new();
        // A server that answered whole, not as events.
        let mut whole = String::new();
        for line in std::io::BufRead::lines(reader) {
            let line = line.map_err(|error| {
                ProviderError::new(
                    "PROVIDER_HTTP_ERROR",
                    format!("provider response read failed: {error}"),
                )
            })?;
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
                if events.is_empty() {
                    whole.push_str(&line);
                    whole.push('\n');
                }
                continue;
            };
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(data).map_err(|error| {
                ProviderError::new(
                    "PROVIDER_PROTOCOL_ERROR",
                    format!("provider returned invalid stream JSON: {error}"),
                )
            })?;
            let more = on_event(&event);
            events.push(event);
            if !more {
                // Dropping the response closes the connection: the model
                // stops writing what no one will read.
                return Ok(events);
            }
        }
        if events.is_empty() {
            events = parse_stream_events(&whole)?;
            for event in &events {
                on_event(event);
            }
        }
        Ok(events)
    }
}

impl UreqHttpClient {
    fn request(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<ureq::http::Response<ureq::Body>, ProviderError> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build();
        let agent: ureq::Agent = config.into();
        let mut request = agent.post(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        request.send_json(body).map_err(|error| {
            ProviderError::new(
                "PROVIDER_HTTP_ERROR",
                format!("provider request failed: {error}"),
            )
        })
    }

    fn send(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<String, ProviderError> {
        let mut response = self.request(url, headers, body)?;
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

/// Anthropic messages adapter. The system prompt and the conversation so
/// far are marked for prompt caching; thinking blocks are kept with their
/// signatures so they can be sent back unchanged.
pub struct AnthropicProvider {
    client: Arc<dyn JsonHttpClient>,
    api_key: Option<String>,
    base_url: String,
    effort: Option<ReasoningEffort>,
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
            effort: None,
        }
    }

    /// How hard the model thinks. `none` and `minimal` ask for `low`, the
    /// least Anthropic models take; unset leaves the model's default.
    pub fn with_effort(mut self, effort: Option<ReasoningEffort>) -> Self {
        self.effort = effort;
        self
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
        self.exchange(request, &mut |_| true)
    }

    fn stream_live(
        &self,
        request: &ProviderRequest,
        on_text: &mut dyn FnMut(&str),
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        relay_live(|on_said| self.exchange(request, on_said), on_text, stop)
    }
}

impl AnthropicProvider {
    fn exchange(
        &self,
        request: &ProviderRequest,
        on_said: &mut OnSaid,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![
            ("anthropic-version".into(), "2023-06-01".into()),
            ("content-type".into(), "application/json".into()),
        ];
        if let Some(key) = &self.api_key {
            headers.push(("x-api-key".into(), key.clone()));
        }
        let said = exchange(
            self.client.as_ref(),
            &format!("{}/messages", self.base_url),
            &headers,
            &anthropic::Request::new(request, self.effort),
            on_said,
        )?;
        Ok(anthropic::reply(said))
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
        self.exchange(request, &mut |_| true)
    }

    fn stream_live(
        &self,
        request: &ProviderRequest,
        on_text: &mut dyn FnMut(&str),
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        relay_live(|on_said| self.exchange(request, on_said), on_text, stop)
    }
}

impl OpenAiProvider {
    fn exchange(
        &self,
        request: &ProviderRequest,
        on_said: &mut OnSaid,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![("content-type".into(), "application/json".into())];
        if let Some(key) = &self.api_key {
            headers.push(("authorization".into(), format!("Bearer {key}")));
        }
        let said = exchange(
            self.client.as_ref(),
            &format!("{}/chat/completions", self.base_url),
            &headers,
            &chat::Request::new(request),
            on_said,
        )?;
        Ok(chat::reply(said))
    }
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
        self.exchange(request, &mut |_| true)
    }

    fn stream_live(
        &self,
        request: &ProviderRequest,
        on_text: &mut dyn FnMut(&str),
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        relay_live(|on_said| self.exchange(request, on_said), on_text, stop)
    }
}

impl OpenAiResponsesProvider {
    fn exchange(
        &self,
        request: &ProviderRequest,
        on_said: &mut OnSaid,
    ) -> Result<Vec<ProviderChunk>, ProviderError> {
        let mut headers = vec![("content-type".into(), "application/json".into())];
        if let Some(key) = &self.api_key {
            headers.push(("authorization".into(), format!("Bearer {key}")));
        }
        let said = exchange(
            self.client.as_ref(),
            &format!("{}/responses", self.base_url),
            &headers,
            &responses::Request::new(request, self.reasoning_effort),
            on_said,
        )?;
        Ok(responses::reply(said))
    }
}

/// `tool` as a Chat Completions tool, in JSON.
pub fn to_openai_tool(tool: &NormalizedTool) -> Value {
    json(chat::Tool::from(tool))
}

/// `tool` as a Responses API tool, in JSON.
pub fn to_openai_responses_tool(tool: &NormalizedTool) -> Value {
    json(responses::Tool::from(tool))
}

/// Translate the canonical Anthropic-shaped history to Chat Completions.
pub fn to_openai_messages(messages: &[Value]) -> Vec<Value> {
    chat::messages_of(messages).into_iter().map(json).collect()
}

/// Translate history to Responses API input items.
pub fn to_openai_responses_input(messages: &[Value]) -> Vec<Value> {
    responses::input_of(messages)
        .into_iter()
        .map(json)
        .collect()
}

/// One of this module's types in JSON, as it is sent or kept. Their keys are all text, so
/// they always write as JSON.
fn json(said: impl Serialize) -> Value {
    serde_json::to_value(said).expect("Kerf's provider types write as JSON")
}

/// The text of `blocks`' text blocks, run together.
pub(crate) fn text_from_blocks(blocks: &[Value]) -> String {
    transcript::text_of(&blocks.iter().map(Block::read).collect::<Vec<_>>())
}

pub(crate) fn is_tool_use(block: &Value) -> bool {
    Block::read(block).is_tool_use()
}

/// A tool call's arguments as JSON text, as OpenAI's APIs take them.
fn arguments(input: &Value) -> String {
    serde_json::to_string(input).unwrap_or_else(|_| "{}".into())
}

/// A tool call's arguments from their JSON text; ones that are not JSON are none.
fn input(arguments: Option<&str>) -> Value {
    arguments
        .and_then(|arguments| serde_json::from_str(arguments).ok())
        .unwrap_or_else(|| Value::Object(Map::new()))
}

fn visual_link_text(id: &str) -> String {
    format!("Visual output for tool call {id} (see the tool result immediately above).")
}

fn normalize_stop_reason(value: Option<&str>) -> String {
    match value {
        Some("tool_use" | "tool_calls" | "function_call") => "tool_use",
        Some("refusal") => "refusal",
        _ => "end_turn",
    }
    .into()
}
