//! The session's transcript, in the Anthropic Messages API's shape, as the adapters read it:
//! the messages they translate for OpenAI's APIs, and the blocks a reply is made of.
//!
//! The transcript itself stays JSON (`AgentSession::messages`): the session keeps it on disk,
//! `session.read` points into it, and it goes back to Anthropic as it was said. So it is read
//! here where it is read, not held as these types.

use super::said::{as_said, object, object_said};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Who said a message: `user`, or anything else, which reads as the model, as the
/// translations have always taken it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Role {
    User,
    #[default]
    Assistant,
}

/// A message of the transcript: who said it, and what.
#[derive(Default, Deserialize)]
pub(super) struct Message {
    #[serde(default, deserialize_with = "role")]
    pub role: Role,
    #[serde(default, deserialize_with = "content")]
    pub content: Content,
}

impl Message {
    /// A message of the transcript; one that is not an object says nothing, from the model.
    pub fn read(said: &Value) -> Self {
        object(said)
    }
}

/// What a message says: text, blocks, or neither (nothing, or anything else).
#[derive(Default)]
pub(super) enum Content {
    Text(String),
    Blocks(Vec<Block>),
    #[default]
    Neither,
}

impl Content {
    /// Its blocks: text is one text block, and neither is none.
    pub fn into_blocks(self) -> Vec<Block> {
        match self {
            Self::Text(text) => vec![Block::text(text)],
            Self::Blocks(blocks) => blocks,
            Self::Neither => Vec::new(),
        }
    }
}

/// A block of a message: of a reply (its text, thinking and tool calls), or of what went back
/// to the model (a tool's result). A reply's blocks go back to Anthropic in the transcript,
/// so whatever else the API put on them stays as said in `rest`, and a block of another kind
/// stays whole ([`Block::Other`]). The fields Kerf reads each read on their own
/// ([`as_said`]): one said otherwise reads, and is written again, as its default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Block {
    Text {
        #[serde(default, deserialize_with = "as_said")]
        text: String,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    Thinking {
        #[serde(default, deserialize_with = "as_said")]
        thinking: String,
        /// Said once the thinking is whole; the API checks it when the block comes back.
        #[serde(
            default,
            deserialize_with = "as_said",
            skip_serializing_if = "Option::is_none"
        )]
        signature: Option<String>,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    ToolUse {
        #[serde(default, deserialize_with = "as_said")]
        id: String,
        #[serde(default, deserialize_with = "as_said")]
        name: String,
        /// The call's arguments. Their shape is the tool's own, so they stay JSON; the
        /// session hands them to the tool as they are.
        #[serde(default)]
        input: Value,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    ToolResult {
        #[serde(default, deserialize_with = "as_said")]
        tool_use_id: String,
        /// What the tool answered, as the session put it: its JSON as text, or blocks of
        /// text and pictures. It stays JSON because an answer that says neither goes to
        /// OpenAI as its JSON text ([`visuals`]).
        #[serde(default)]
        content: Value,
        #[serde(flatten)]
        rest: Map<String, Value>,
    },
    #[serde(untagged)]
    Other(Value),
}

impl Block {
    /// A block as said. One whose kind is not said as text stays whole, and so does one
    /// that is not an object: serde would read a list field by field, and a number in
    /// `type` as the kind of that place in the list of kinds.
    pub fn read(said: &Value) -> Self {
        match said {
            Value::Object(fields) if fields.get("type").is_some_and(Value::is_string) => {
                Self::deserialize(said).unwrap_or_else(|_| Self::Other(said.clone()))
            }
            _ => Self::Other(said.clone()),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            rest: Map::new(),
        }
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
            rest: Map::new(),
        }
    }

    pub fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse { .. })
    }
}

impl From<Block> for Value {
    fn from(block: Block) -> Self {
        super::json(block)
    }
}

/// The text of `blocks`' text blocks, run together.
pub(super) fn text_of(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// A tool's result for a model that reads it as text: its text, and the pictures in it as
/// URLs (a `data:` URL for an encoded PNG or JPEG). A result of blocks with no text says it
/// has pictures, or, with none, is its blocks' JSON; any other result is its JSON.
pub(super) fn visuals(content: &Value) -> (String, Vec<String>) {
    let parts = match content {
        Value::String(text) => return (text.clone(), Vec::new()),
        Value::Array(parts) => parts,
        other => return (json_text(other), Vec::new()),
    };
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for part in parts.iter().map(object) {
        match part {
            Part::Text { text } => texts.push(text),
            Part::Image { source } => images.extend(source.url()),
            Part::Other => {}
        }
    }
    let text = if !texts.is_empty() {
        texts.join("\n")
    } else if !images.is_empty() {
        "Tool result includes visual output.".into()
    } else {
        json_text(content)
    };
    (text, images)
}

fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// A part of a tool's result, of the kinds a model that reads text is shown.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Part {
    Text {
        #[serde(default, deserialize_with = "as_said")]
        text: String,
    },
    Image {
        #[serde(default, deserialize_with = "object_said")]
        source: ImageSource,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// Where a picture is: encoded in the message, or at a URL.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ImageSource {
    Base64 {
        #[serde(default, deserialize_with = "as_said")]
        media_type: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        data: Option<String>,
    },
    Url {
        #[serde(default, deserialize_with = "as_said")]
        url: Option<String>,
    },
    #[default]
    #[serde(other)]
    Other,
}

impl ImageSource {
    /// The picture as a URL OpenAI takes: its own, or a `data:` URL of a PNG or JPEG.
    fn url(self) -> Option<String> {
        let (media_type, data) = match self {
            Self::Url { url } => return url,
            Self::Base64 { media_type, data } => (media_type, data?),
            Self::Other => return None,
        };
        if data.starts_with("data:image/png;base64,") || data.starts_with("data:image/jpeg;base64,")
        {
            return Some(data);
        }
        let media_type = media_type?;
        matches!(media_type.as_str(), "image/png" | "image/jpeg")
            .then(|| format!("data:{media_type};base64,{data}"))
    }
}

/// A message's role: `user`, or anything else, which is the model's.
fn role<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Role, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(role) if role == "user" => Role::User,
        _ => Role::Assistant,
    })
}

/// A message's content: text, each block of a list as [`Block::read`] reads it, or neither.
fn content<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Content, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => Content::Text(text),
        Value::Array(blocks) => Content::Blocks(blocks.iter().map(Block::read).collect()),
        _ => Content::Neither,
    })
}
