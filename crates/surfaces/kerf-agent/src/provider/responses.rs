//! OpenAI's Responses API: the request Kerf sends, and what a reply says, streamed as
//! events or whole.

use super::said::{as_said, each_object, each_object_said, object, object_said, some_object_said};
use super::transcript::{self, text_of, visuals, Block, Content, Role};
use super::{NormalizedTool, ProviderChunk, ProviderRequest};
use crate::ReasoningEffort;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A request for one reply, streamed.
#[derive(Serialize)]
pub(super) struct Request<'a> {
    model: &'a str,
    instructions: &'a str,
    input: Vec<Item>,
    tools: Vec<Tool<'a>>,
    tool_choice: ToolChoice,
    stream: bool,
    /// How hard the model thinks; none leaves the model's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning>,
}

impl<'a> Request<'a> {
    pub fn new(request: &'a ProviderRequest, effort: Option<ReasoningEffort>) -> Self {
        Self {
            model: &request.model,
            instructions: &request.system,
            input: input_of(&request.messages),
            tools: request.tools.iter().map(Tool::from).collect(),
            tool_choice: ToolChoice::Auto,
            stream: true,
            reasoning: effort.map(|effort| Reasoning { effort }),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ToolChoice {
    Auto,
}

#[derive(Serialize)]
struct Reasoning {
    effort: ReasoningEffort,
}

/// A tool the model may call.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum Tool<'a> {
    Function {
        name: &'a str,
        description: &'a str,
        /// A JSON Schema, each tool's own, as the tool gives it.
        parameters: &'a Value,
    },
}

impl<'a> From<&'a NormalizedTool> for Tool<'a> {
    fn from(tool: &'a NormalizedTool) -> Self {
        Self::Function {
            name: &tool.name,
            description: &tool.description,
            parameters: &tool.input_schema,
        }
    }
}

/// An item of the input: a message, or a tool call or its output.
#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Item {
    Message { role: Role, content: ItemContent },
    Call(Call),
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ItemContent {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Part {
    InputText { text: String },
    InputImage { image_url: String },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Call {
    /// A call the model made, its arguments as JSON text.
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// What the tool answered, as text.
    FunctionCallOutput { call_id: String, output: String },
}

/// The transcript as Responses input: a user's text and tool outputs (each picture in a
/// user message after its output), and the model's text and calls.
pub(super) fn input_of(transcript: &[Value]) -> Vec<Item> {
    let mut output = Vec::new();
    let said = |role, text| Item::Message {
        role,
        content: ItemContent::Text(text),
    };
    for message in transcript.iter().map(transcript::Message::read) {
        if message.role == Role::User {
            let blocks = match message.content {
                Content::Text(text) => {
                    output.push(said(Role::User, text));
                    continue;
                }
                content => content.into_blocks(),
            };
            let text = text_of(&blocks);
            if !text.trim().is_empty() {
                output.push(said(Role::User, text));
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
                    let mut parts = vec![Part::InputText {
                        text: super::visual_link_text(&id),
                    }];
                    parts.extend(
                        images
                            .into_iter()
                            .map(|image_url| Part::InputImage { image_url }),
                    );
                    Item::Message {
                        role: Role::User,
                        content: ItemContent::Parts(parts),
                    }
                });
                output.push(Item::Call(Call::FunctionCallOutput {
                    call_id: id,
                    output: text,
                }));
                output.extend(shown);
            }
            continue;
        }
        let blocks = message.content.into_blocks();
        let text = text_of(&blocks);
        if !text.trim().is_empty() {
            output.push(said(Role::Assistant, text));
        }
        for block in blocks {
            if let Block::ToolUse {
                id, name, input, ..
            } = block
            {
                output.push(Item::Call(Call::FunctionCall {
                    call_id: id,
                    name,
                    arguments: super::arguments(&input),
                }));
            }
        }
    }
    output
}

/// One thing the API said: an event of a streamed reply, or, from a server that answers
/// whole, the response itself, which says its `output`.
#[derive(Default, Deserialize)]
pub(super) struct Said {
    /// The response's output, when it came whole: said at all marks it so.
    #[serde(default, deserialize_with = "some_items")]
    output: Option<Vec<OutputItem>>,
    #[serde(default, deserialize_with = "as_said")]
    output_text: Option<String>,
    #[serde(flatten, deserialize_with = "as_said")]
    event: Event,
}

impl super::Said for Said {
    fn read(said: &Value) -> Self {
        object(said)
    }

    fn text(&self) -> Option<&str> {
        match &self.event {
            Event::TextDelta { delta } => delta.as_deref(),
            _ => None,
        }
    }
}

/// The events of a streamed reply that are read; any other is [`Event::Other`].
#[derive(Default, Deserialize)]
#[serde(tag = "type")]
enum Event {
    #[serde(rename = "response.output_text.delta")]
    TextDelta {
        #[serde(default, deserialize_with = "as_said")]
        delta: Option<String>,
    },
    /// An item of the output begins, or is whole.
    #[serde(
        rename = "response.output_item.added",
        alias = "response.output_item.done"
    )]
    Item {
        #[serde(default, deserialize_with = "object_said")]
        item: OutputItem,
    },
    /// A piece of a call's arguments, said of the call by its `call_id`, its item's id, or
    /// its place in the output, whichever it says at all first.
    #[serde(rename = "response.function_call_arguments.delta")]
    ArgumentsDelta {
        #[serde(default, deserialize_with = "as_key")]
        call_id: Option<String>,
        #[serde(default, deserialize_with = "as_key")]
        item_id: Option<String>,
        #[serde(default, deserialize_with = "as_key")]
        output_index: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        name: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        delta: Option<String>,
    },
    /// The response, whole, at the end of its stream.
    #[serde(rename = "response.completed")]
    Completed {
        #[serde(default, deserialize_with = "some_object_said")]
        response: Option<Response>,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(default, deserialize_with = "as_said")]
        message: Option<String>,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// A response whole: its output, and the text of it as one, which some servers say instead.
#[derive(Default, Deserialize)]
struct Response {
    #[serde(default, deserialize_with = "each_object_said")]
    output: Vec<OutputItem>,
    #[serde(default, deserialize_with = "as_said")]
    output_text: Option<String>,
}

/// An item of the output, of the kinds that are read.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputItem {
    Message {
        #[serde(default, deserialize_with = "each_object_said")]
        content: Vec<OutputPart>,
    },
    FunctionCall {
        #[serde(default, deserialize_with = "call_id")]
        call_id: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        id: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        name: String,
        /// The call's arguments as JSON text.
        #[serde(default, deserialize_with = "as_said")]
        arguments: Option<String>,
    },
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputPart {
    OutputText {
        #[serde(default, deserialize_with = "as_said")]
        text: Option<String>,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// What a call is known by: its `call_id`, or else its item's `id`, or else `call_tool`.
fn call_key(call_id: Option<String>, id: Option<String>) -> String {
    call_id.or(id).unwrap_or_else(|| "call_tool".into())
}

impl Response {
    /// The response's text and calls as blocks; with none, its `output_text`.
    fn into_blocks(self) -> Vec<Block> {
        let mut blocks = Vec::new();
        for item in self.output {
            match item {
                OutputItem::Message { content } => {
                    blocks.extend(content.into_iter().filter_map(|part| match part {
                        OutputPart::OutputText { text } => text.map(Block::text),
                        OutputPart::Other => None,
                    }));
                }
                OutputItem::FunctionCall {
                    call_id,
                    id,
                    name,
                    arguments,
                } => blocks.push(Block::tool_use(
                    call_key(call_id, id),
                    name,
                    super::input(arguments.as_deref()),
                )),
                OutputItem::Other => {}
            }
        }
        if blocks.is_empty()
            && let Some(text) = self.output_text.filter(|text| !text.is_empty())
        {
            blocks.push(Block::text(text));
        }
        blocks
    }
}

/// A key, said as text or a whole number; said as anything else, `0`.
fn as_key<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Some(match Value::deserialize(deserializer)? {
        Value::String(key) => key,
        Value::Number(number) if number.is_u64() => number.to_string(),
        _ => "0".into(),
    }))
}

/// A call's `call_id`; said as anything but text, `call_tool`, as a call that says no id is.
fn call_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Some(match Value::deserialize(deserializer)? {
        Value::String(id) => id,
        _ => "call_tool".into(),
    }))
}

/// A whole response's output items, each read as [`object`] reads it.
fn some_items<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<OutputItem>>, D::Error> {
    Ok(Some(each_object(&Value::deserialize(deserializer)?)))
}

/// The reply `said` makes: whole, when the API said one thing and it was the response, or
/// else built from its stream's events.
pub(super) fn reply(mut said: Vec<Said>) -> Vec<ProviderChunk> {
    if said.len() == 1
        && let Some(output) = said[0].output.take()
    {
        let content = Response {
            output,
            output_text: said[0].output_text.take(),
        }
        .into_blocks();
        let text = text_of(&content);
        let mut chunks = Vec::new();
        if !text.is_empty() {
            chunks.push(ProviderChunk::TextDelta { text });
        }
        chunks.push(end(content));
        return chunks;
    }
    streamed(said)
}

/// A streamed reply: its text told as it came, then its end with the completed response's
/// text and calls, or, when it never completed, the text and the calls its events made.
fn streamed(said: Vec<Said>) -> Vec<ProviderChunk> {
    let mut text = String::new();
    // Each call's item id, name and arguments, by what it is known by.
    let mut calls = BTreeMap::<String, (String, String, String)>::new();
    let mut completed = None;
    let mut chunks = Vec::new();
    for said in said {
        match said.event {
            Event::TextDelta { delta: Some(delta) } if !delta.is_empty() => {
                text.push_str(&delta);
                chunks.push(ProviderChunk::TextDelta { text: delta });
            }
            Event::Item {
                item:
                    OutputItem::FunctionCall {
                        call_id,
                        id,
                        name,
                        arguments,
                    },
            } => {
                let call = calls.entry(call_key(call_id, id.clone())).or_default();
                call.0 = id.unwrap_or_default();
                call.1 = name;
                if let Some(arguments) = arguments {
                    call.2 = arguments;
                }
            }
            Event::ArgumentsDelta {
                call_id,
                item_id,
                output_index,
                name,
                delta,
            } => {
                let key = call_id.or(item_id).or(output_index);
                let call = calls.entry(key.unwrap_or_else(|| "0".into())).or_default();
                if let Some(name) = name {
                    call.1 = name;
                }
                call.2.push_str(delta.as_deref().unwrap_or_default());
            }
            Event::Completed { response } => completed = response,
            Event::Error { message } => {
                let message = message.unwrap_or_else(|| "Responses API stream error".into());
                chunks.push(ProviderChunk::MessageEnd {
                    stop_reason: "error".into(),
                    content: vec![Block::text(message).into()],
                });
                return chunks;
            }
            _ => {}
        }
    }
    let content = match completed {
        Some(response) => response.into_blocks(),
        None => {
            let mut content = Vec::new();
            if !text.is_empty() {
                content.push(Block::text(text.as_str()));
            }
            for (key, (id, name, arguments)) in calls {
                let id = if key.is_empty() { id } else { key };
                content.push(Block::tool_use(id, name, super::input(Some(&arguments))));
            }
            content
        }
    };
    // A reply whose text came only with the completed response tells its first text then.
    if text.is_empty()
        && let Some(Block::Text { text, .. }) = content
            .iter()
            .find(|block| matches!(block, Block::Text { .. }))
    {
        chunks.push(ProviderChunk::TextDelta { text: text.clone() });
    }
    chunks.push(end(content));
    chunks
}

/// The reply's end: to call tools when it asks for any, and the turn's end otherwise.
fn end(content: Vec<Block>) -> ProviderChunk {
    ProviderChunk::MessageEnd {
        stop_reason: if content.iter().any(Block::is_tool_use) {
            "tool_use".into()
        } else {
            "end_turn".into()
        },
        content: content.into_iter().map(Value::from).collect(),
    }
}
