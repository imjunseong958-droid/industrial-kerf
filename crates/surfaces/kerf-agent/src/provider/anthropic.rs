//! The Anthropic Messages API: the request Kerf sends, and what a reply says, streamed as
//! events or whole.

use super::said::{as_said, object, object_said};
use super::transcript::{text_of, Block};
use super::{normalize_stop_reason, NormalizedTool, ProviderChunk, ProviderRequest};
use crate::ReasoningEffort;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The most a response may write, thinking included.
const MAX_TOKENS: u32 = 32_000;

/// A request for one reply, streamed.
#[derive(Serialize)]
pub(super) struct Request<'a> {
    model: &'a str,
    max_tokens: u32,
    tools: Vec<Tool<'a>>,
    /// The transcript, which is already this API's shape and goes back as it was said:
    /// thinking with its signature, and whatever else the API put on a block. So it stays
    /// JSON, and only its last block is marked ([`with_cache_breakpoint`]).
    messages: Vec<Value>,
    stream: bool,
    /// The instructions, marked for caching; none when there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<[System<'a>; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<OutputConfig>,
}

impl<'a> Request<'a> {
    /// `request`, with the model thinking as hard as `effort` says: `none` and `minimal` ask
    /// for `low`, the least Anthropic models take, and none leaves the model's default.
    pub fn new(request: &'a ProviderRequest, effort: Option<ReasoningEffort>) -> Self {
        Self {
            model: &request.model,
            max_tokens: MAX_TOKENS,
            tools: request.tools.iter().map(Tool::from).collect(),
            messages: with_cache_breakpoint(&request.messages),
            stream: true,
            system: (!request.system.is_empty()).then(|| {
                [System::Text {
                    text: &request.system,
                    cache_control: CacheControl::Ephemeral,
                }]
            }),
            output_config: effort.map(|effort| OutputConfig {
                effort: match effort {
                    ReasoningEffort::None | ReasoningEffort::Minimal => ReasoningEffort::Low,
                    other => other,
                },
            }),
        }
    }
}

#[derive(Serialize)]
struct Tool<'a> {
    name: &'a str,
    description: &'a str,
    /// A JSON Schema, each tool's own, as the tool gives it.
    input_schema: &'a Value,
}

impl<'a> From<&'a NormalizedTool> for Tool<'a> {
    fn from(tool: &'a NormalizedTool) -> Self {
        Self {
            name: &tool.name,
            description: &tool.description,
            input_schema: &tool.input_schema,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum System<'a> {
    Text {
        text: &'a str,
        cache_control: CacheControl,
    },
}

/// Where the cache ends: everything before it is read from the cache next time.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CacheControl {
    Ephemeral,
}

#[derive(Serialize)]
struct OutputConfig {
    effort: ReasoningEffort,
}

/// `messages` with a cache breakpoint on the last block, so the next request reads
/// everything before it from the cache. A last message of text becomes one text block.
fn with_cache_breakpoint(messages: &[Value]) -> Vec<Value> {
    let mut messages = messages.to_vec();
    let Some(Value::Object(last)) = messages.last_mut() else {
        return messages;
    };
    if let Some(Value::String(text)) = last.get("content") {
        let block = Block::text(text.as_str()).into();
        last.insert("content".into(), Value::Array(vec![block]));
    }
    if let Some(Value::Array(blocks)) = last.get_mut("content")
        && let Some(Value::Object(block)) = blocks.last_mut()
    {
        block.insert("cache_control".into(), super::json(CacheControl::Ephemeral));
    }
    messages
}

/// One thing the API said: an event of a streamed reply, or, from a server that answers
/// whole, the reply itself, which says its `content`.
#[derive(Default, Deserialize)]
pub(super) struct Said {
    /// The reply's blocks, when it came whole: said at all marks it so.
    #[serde(default, deserialize_with = "some_blocks")]
    content: Option<Vec<Block>>,
    /// Why the whole reply ended.
    #[serde(default, deserialize_with = "as_said")]
    stop_reason: Option<String>,
    #[serde(flatten, deserialize_with = "as_said")]
    event: Event,
}

impl super::Said for Said {
    fn read(said: &Value) -> Self {
        object(said)
    }

    fn text(&self) -> Option<&str> {
        match &self.event {
            Event::ContentBlockDelta {
                delta: Delta::Text { text },
                ..
            } => Some(text),
            _ => None,
        }
    }
}

/// The events of a streamed reply that are read; any other is [`Event::Other`].
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    /// A block begins at `index`, as the API says it; one that says none is empty text.
    ContentBlockStart {
        #[serde(default, deserialize_with = "as_said")]
        index: usize,
        #[serde(default = "empty_text", deserialize_with = "block")]
        content_block: Block,
    },
    /// The block at `index` grows.
    ContentBlockDelta {
        #[serde(default, deserialize_with = "as_said")]
        index: usize,
        #[serde(default, deserialize_with = "object_said")]
        delta: Delta,
    },
    /// The reply ends, and why.
    MessageDelta {
        #[serde(default, deserialize_with = "object_said")]
        delta: MessageDelta,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// How a block grows.
#[derive(Default, Deserialize)]
#[serde(tag = "type")]
enum Delta {
    #[serde(rename = "text_delta")]
    Text {
        #[serde(default, deserialize_with = "as_said")]
        text: String,
    },
    #[serde(rename = "thinking_delta")]
    Thinking {
        #[serde(default, deserialize_with = "as_said")]
        thinking: String,
    },
    #[serde(rename = "signature_delta")]
    Signature {
        #[serde(default, deserialize_with = "as_said")]
        signature: Option<String>,
    },
    /// A piece of a tool call's arguments, which are JSON once all have come.
    #[serde(rename = "input_json_delta")]
    InputJson {
        #[serde(default, deserialize_with = "as_said")]
        partial_json: String,
    },
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Default, Deserialize)]
struct MessageDelta {
    #[serde(default, deserialize_with = "as_said")]
    stop_reason: Option<String>,
}

fn empty_text() -> Block {
    Block::text("")
}

/// A block as [`Block::read`] reads it.
fn block<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Block, D::Error> {
    Ok(Block::read(&Value::deserialize(deserializer)?))
}

/// A whole reply's blocks, each as [`Block::read`] reads it; what is not a list has none.
fn some_blocks<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<Block>>, D::Error> {
    Ok(Some(match Value::deserialize(deserializer)? {
        Value::Array(blocks) => blocks.iter().map(Block::read).collect(),
        _ => Vec::new(),
    }))
}

/// The reply `said` makes: whole, when the API said one thing and it was the reply, or else
/// built from its stream's events.
pub(super) fn reply(mut said: Vec<Said>) -> Vec<ProviderChunk> {
    if said.len() == 1
        && let Some(content) = said[0].content.take()
    {
        let stop_reason = said[0].stop_reason.take();
        return whole(content, stop_reason.unwrap_or_else(|| "end_turn".into()));
    }
    streamed(said)
}

/// A reply that came whole: its text told at once, then its end.
fn whole(content: Vec<Block>, stop_reason: String) -> Vec<ProviderChunk> {
    let text = text_of(&content);
    let mut chunks = Vec::new();
    if !text.is_empty() {
        chunks.push(ProviderChunk::TextDelta { text });
    }
    chunks.push(ProviderChunk::MessageEnd {
        stop_reason,
        content: content.into_iter().map(Value::from).collect(),
    });
    chunks
}

/// A streamed reply: its blocks as their events build them, each piece of text told as it
/// came, then its end.
fn streamed(said: Vec<Said>) -> Vec<ProviderChunk> {
    let mut blocks = Vec::new();
    let mut arguments = BTreeMap::<usize, String>::new();
    let mut stop_reason = None;
    let mut chunks = Vec::new();
    for said in said {
        match said.event {
            Event::ContentBlockStart {
                index,
                content_block,
            } => place(&mut blocks, index, content_block),
            Event::ContentBlockDelta { index, delta } => match delta {
                Delta::Text { text } if !text.is_empty() => {
                    match blocks.get_mut(index) {
                        Some(Block::Text { text: said, .. }) => said.push_str(&text),
                        _ => place(&mut blocks, index, Block::text(text.as_str())),
                    }
                    chunks.push(ProviderChunk::TextDelta { text });
                }
                Delta::Thinking { thinking } => {
                    if let Some(Block::Thinking { thinking: said, .. }) = blocks.get_mut(index) {
                        said.push_str(&thinking);
                    }
                }
                Delta::Signature {
                    signature: Some(signature),
                } => {
                    if let Some(Block::Thinking {
                        signature: said, ..
                    }) = blocks.get_mut(index)
                    {
                        *said = Some(signature);
                    }
                }
                Delta::InputJson { partial_json } => {
                    arguments.entry(index).or_default().push_str(&partial_json);
                }
                _ => {}
            },
            Event::MessageDelta { delta } => {
                if let Some(reason) = delta.stop_reason {
                    stop_reason = Some(reason);
                }
            }
            Event::Other => {}
        }
    }
    // A tool call's arguments, once all have come; ones that are not JSON are none.
    for (index, arguments) in arguments {
        let input = super::input(Some(&arguments));
        match blocks.get_mut(index) {
            Some(Block::ToolUse { input: said, .. }) => *said = input,
            _ => place(&mut blocks, index, Block::tool_use("call_tool", "", input)),
        }
    }
    chunks.push(ProviderChunk::MessageEnd {
        stop_reason: normalize_stop_reason(stop_reason.as_deref()),
        content: blocks.into_iter().map(Value::from).collect(),
    });
    chunks
}

/// Put `block` at `index`, with empty objects before it where no block began.
fn place(blocks: &mut Vec<Block>, index: usize, block: Block) {
    while blocks.len() <= index {
        blocks.push(Block::Other(Value::Object(Map::new())));
    }
    blocks[index] = block;
}
