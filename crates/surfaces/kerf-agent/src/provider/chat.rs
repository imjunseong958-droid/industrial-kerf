//! OpenAI's Chat Completions API, and servers that speak it: the request Kerf sends, and
//! what a reply says, streamed as chunks or whole.

use super::said::{as_said, each_object_said, object, object_said, some_object_said};
use super::transcript::{self, text_of, visuals, Block, Content, Role};
use super::{normalize_stop_reason, NormalizedTool, ProviderChunk, ProviderRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A request for one reply, streamed.
#[derive(Serialize)]
pub(super) struct Request<'a> {
    model: &'a str,
    messages: Vec<Message>,
    tools: Vec<Tool<'a>>,
    tool_choice: ToolChoice,
    stream: bool,
}

impl<'a> Request<'a> {
    pub fn new(request: &'a ProviderRequest) -> Self {
        let mut messages = vec![Message::System {
            content: request.system.clone(),
        }];
        messages.extend(messages_of(&request.messages));
        Self {
            model: &request.model,
            messages,
            tools: request.tools.iter().map(Tool::from).collect(),
            tool_choice: ToolChoice::Auto,
            stream: true,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ToolChoice {
    Auto,
}

/// A tool the model may call.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum Tool<'a> {
    Function { function: Function<'a> },
}

#[derive(Serialize)]
pub(super) struct Function<'a> {
    name: &'a str,
    description: &'a str,
    /// A JSON Schema, each tool's own, as the tool gives it.
    parameters: &'a Value,
}

impl<'a> From<&'a NormalizedTool> for Tool<'a> {
    fn from(tool: &'a NormalizedTool) -> Self {
        Self::Function {
            function: Function {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.input_schema,
            },
        }
    }
}

/// A message of the conversation, by who says it.
#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub(super) enum Message {
    System {
        content: String,
    },
    User {
        content: UserContent,
    },
    /// What the model said: its text (`null` when it said none) and its tool calls.
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// A tool's result, as text.
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum UserContent {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Part {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
pub(super) struct ImageUrl {
    url: String,
}

#[derive(Serialize)]
pub(super) struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: CallKind,
    function: CallFunction,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum CallKind {
    Function,
}

#[derive(Serialize)]
struct CallFunction {
    name: String,
    /// The call's arguments as JSON text.
    arguments: String,
}

/// The transcript as Chat Completions messages: a user's text and tool results (each a
/// `tool` message, its pictures in a user message after it), and the model's text and tool
/// calls.
pub(super) fn messages_of(transcript: &[Value]) -> Vec<Message> {
    let mut output = Vec::new();
    for message in transcript.iter().map(transcript::Message::read) {
        if message.role == Role::User {
            let blocks = match message.content {
                Content::Text(text) => {
                    output.push(Message::User {
                        content: UserContent::Text(text),
                    });
                    continue;
                }
                content => content.into_blocks(),
            };
            let text = text_of(&blocks);
            if !text.trim().is_empty() {
                output.push(Message::User {
                    content: UserContent::Text(text),
                });
            }
            for block in blocks {
                let Block::ToolResult {
                    tool_use_id: id,
                    content,
                    ..
                } = block
                else {
                    continue;
                };
                let (text, images) = visuals(&content);
                let shown = (!images.is_empty()).then(|| {
                    let mut parts = vec![Part::Text {
                        text: super::visual_link_text(&id),
                    }];
                    parts.extend(images.into_iter().map(|url| Part::ImageUrl {
                        image_url: ImageUrl { url },
                    }));
                    Message::User {
                        content: UserContent::Parts(parts),
                    }
                });
                output.push(Message::Tool {
                    tool_call_id: id,
                    content: text,
                });
                output.extend(shown);
            }
            continue;
        }
        let blocks = message.content.into_blocks();
        let text = text_of(&blocks);
        let tool_calls = blocks
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolUse {
                    id, name, input, ..
                } => Some(ToolCall {
                    id,
                    kind: CallKind::Function,
                    function: CallFunction {
                        name,
                        arguments: super::arguments(&input),
                    },
                }),
                _ => None,
            })
            .collect();
        output.push(Message::Assistant {
            content: (!text.is_empty()).then_some(text),
            tool_calls,
        });
    }
    output
}

/// One thing a server said: a chunk of a streamed reply, or the reply whole.
#[derive(Default, Deserialize)]
pub(super) struct Said {
    /// The first choice, the one asked for: the first of `choices`, when there is one.
    #[serde(default, deserialize_with = "first_choice")]
    choices: Choice,
}

impl super::Said for Said {
    fn read(said: &Value) -> Self {
        object(said)
    }

    fn text(&self) -> Option<&str> {
        self.choices.delta.content.as_deref()
    }
}

#[derive(Default, Deserialize)]
struct Choice {
    /// The reply, from a server that answered whole: said at all marks it so.
    #[serde(default, deserialize_with = "some_object_said")]
    message: Option<Reply>,
    /// A piece of the reply, from a server that streams it.
    #[serde(default, deserialize_with = "object_said")]
    delta: Reply,
    #[serde(default, deserialize_with = "as_said")]
    finish_reason: Option<String>,
}

/// What the model said, or a piece of it: its text, and its tool calls.
#[derive(Default, Deserialize)]
struct Reply {
    #[serde(default, deserialize_with = "as_said")]
    content: Option<String>,
    #[serde(default, deserialize_with = "each_object_said")]
    tool_calls: Vec<CallSaid>,
}

/// A tool call, or a piece of one.
#[derive(Default, Deserialize)]
struct CallSaid {
    /// Which call a piece is of; a whole reply's calls say none.
    #[serde(default, deserialize_with = "as_said")]
    index: usize,
    #[serde(default, deserialize_with = "as_said")]
    id: Option<String>,
    #[serde(default, deserialize_with = "object_said")]
    function: FunctionSaid,
}

#[derive(Default, Deserialize)]
struct FunctionSaid {
    #[serde(default, deserialize_with = "as_said")]
    name: Option<String>,
    /// The call's arguments as JSON text, or a piece of that text.
    #[serde(default, deserialize_with = "as_said")]
    arguments: Option<String>,
}

fn first_choice<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Choice, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(choices) => choices.first().map(object).unwrap_or_default(),
        _ => Choice::default(),
    })
}

/// The reply `said` makes: whole, when the server said one thing and it was the reply, or
/// else built from its stream's chunks.
pub(super) fn reply(mut said: Vec<Said>) -> Vec<ProviderChunk> {
    if said.len() == 1
        && let Some(message) = said[0].choices.message.take()
    {
        return whole(message, said[0].choices.finish_reason.take());
    }
    streamed(said)
}

/// A reply that came whole: its text told at once, then its end.
fn whole(message: Reply, finish_reason: Option<String>) -> Vec<ProviderChunk> {
    let mut content = Vec::new();
    if let Some(text) = message.content.filter(|text| !text.is_empty()) {
        content.push(Block::text(text));
    }
    content.extend(message.tool_calls.into_iter().map(|call| {
        Block::tool_use(
            call.id.unwrap_or_default(),
            call.function.name.unwrap_or_default(),
            super::input(call.function.arguments.as_deref()),
        )
    }));
    let text = text_of(&content);
    let mut chunks = Vec::new();
    if !text.is_empty() {
        chunks.push(ProviderChunk::TextDelta { text });
    }
    chunks.push(end(finish_reason.as_deref(), content));
    chunks
}

/// A streamed reply: its text told as it came, then its end with the text and the tool
/// calls its pieces made, in the order of their index.
fn streamed(said: Vec<Said>) -> Vec<ProviderChunk> {
    let mut text = String::new();
    // Each call's id, name and arguments, by its index.
    let mut calls = BTreeMap::<usize, (String, String, String)>::new();
    let mut finish_reason = None;
    let mut chunks = Vec::new();
    for said in said {
        let choice = said.choices;
        if let Some(piece) = choice.delta.content.filter(|piece| !piece.is_empty()) {
            text.push_str(&piece);
            chunks.push(ProviderChunk::TextDelta { text: piece });
        }
        for call in choice.delta.tool_calls {
            let (id, name, arguments) = calls.entry(call.index).or_default();
            if let Some(said) = call.id {
                *id = said;
            }
            name.push_str(call.function.name.as_deref().unwrap_or_default());
            arguments.push_str(call.function.arguments.as_deref().unwrap_or_default());
        }
        if let Some(reason) = choice.finish_reason {
            finish_reason = Some(reason);
        }
    }
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(Block::text(text));
    }
    for (_, (id, name, arguments)) in calls {
        let id = if id.is_empty() {
            format!("call_{name}")
        } else {
            id
        };
        content.push(Block::tool_use(id, name, super::input(Some(&arguments))));
    }
    chunks.push(end(finish_reason.as_deref(), content));
    chunks
}

/// The reply's end: why it ended, where `stop` with tool calls means to call them.
fn end(finish_reason: Option<&str>, content: Vec<Block>) -> ProviderChunk {
    let stop_reason = if matches!(finish_reason, Some("stop" | "STOP"))
        && content.iter().any(Block::is_tool_use)
    {
        "tool_use".into()
    } else {
        normalize_stop_reason(finish_reason)
    };
    ProviderChunk::MessageEnd {
        stop_reason,
        content: content.into_iter().map(Value::from).collect(),
    }
}
