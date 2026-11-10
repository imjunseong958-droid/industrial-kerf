//! Bounded provider views over an unchanged, addressable session transcript.
//!
//! Text is charged by UTF-8 bytes (deliberately more conservative than a
//! characters/4 token estimate). Images have a separate conservative charge.
//! This is not a vendor tokenizer: callers can lower the input budget for a
//! small-context model, and context-limit errors trigger one smaller retry.

use super::{bounded_text, NormalizedTool};
use serde_json::{json, Value};

/// The charge for one image: more than providers count for the frames and
/// contact sheets Kerf sends, which they scale to about a megapixel.
const IMAGE_UNITS: usize = 4 * 1024;

#[derive(Clone, Debug)]
pub struct ContextBudget {
    /// Input allowance only; leave output/reasoning space in the model window.
    pub max_input_units: usize,
    pub max_tool_text_bytes: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            // Room for the instructions, the largest context document, and
            // a full `look` of frames, with history beside them.
            max_input_units: 192 * 1024,
            max_tool_text_bytes: 12 * 1024,
        }
    }
}

impl ContextBudget {
    pub fn from_env() -> Self {
        let mut budget = Self::default();
        if let Some(value) = std::env::var("KERF_AGENT_INPUT_BUDGET")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value >= 4096)
        {
            budget.max_input_units = value;
        }
        budget
    }
}

pub(super) fn read_tool() -> NormalizedTool {
    NormalizedTool {
        name: "session.read".into(),
        description: "Read archived conversation or tool evidence without executing a tool again. Message indices are zero-based. Select a tool result with block, then use a JSON pointer into its decoded content. Pages use UTF-8 byte offsets; follow nextOffset. Use image to retrieve one archived image from a multimodal result. Historical evidence is not current project state.".into(),
        input_schema: json!({
            "type":"object",
            "properties":{
                "message":{"type":"integer","minimum":0},
                "block":{"type":"integer","minimum":0},
                "pointer":{"type":"string"},
                "offset":{"type":"integer","minimum":0},
                "limit":{"type":"integer","minimum":1,"maximum":4096},
                "image":{"type":"integer","minimum":0}
            },
            "required":["message"],"additionalProperties":false
        }),
    }
}

fn index(input: &Value, key: &str, default: Option<usize>) -> Result<usize, String> {
    match input.get(key) {
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
        None => default.ok_or_else(|| format!("{key} is required")),
    }
}

pub(super) fn read(messages: &[Value], input: Value) -> Result<Value, String> {
    let message = index(&input, "message", None)?;
    let mut value = messages.get(message).ok_or("unknown archived message")?;
    if input.get("block").is_some() {
        let block = index(&input, "block", None)?;
        value = value
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.get(block))
            .ok_or("unknown archived block")?;
        value = value.get("content").unwrap_or(value);
    }
    let decoded = value
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let value = decoded.as_ref().unwrap_or(value);
    if input.get("image").is_some() {
        let image_index = index(&input, "image", None)?;
        let source = value
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "image")
            .nth(image_index)
            .and_then(|block| block.get("source"))
            .ok_or("unknown archived image; select its tool result block first")?;
        return Ok(json!({
            "type":"multimodal_tool_result",
            "text":format!("Archived evidence from message {message}. Recheck project revision before editing."),
            "images":[{"mediaType":source["media_type"],"data":source["data"]}]
        }));
    }
    let pointer = input
        .get("pointer")
        .map(|v| v.as_str().ok_or("pointer must be a string"))
        .transpose()?
        .unwrap_or("");
    let value = value
        .pointer(pointer)
        .ok_or("JSON pointer not found in archived content")?;
    let serialized;
    let text = if let Some(text) = value.as_str() {
        text
    } else {
        serialized = serde_json::to_string(value).map_err(|error| error.to_string())?;
        &serialized
    };
    let offset = index(&input, "offset", Some(0))?;
    let limit = index(&input, "limit", Some(4096))?;
    if limit == 0 || limit > 4096 {
        return Err("limit must be between 1 and 4096".into());
    }
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err("offset must be a UTF-8 boundary within the archived text".into());
    }
    let mut end = offset.saturating_add(limit).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && offset < text.len() {
        return Err("limit is too small for the next UTF-8 character".into());
    }
    loop {
        let page = json!({"message":message,"block":input.get("block"),"pointer":pointer,
            "offset":offset,"text":&text[offset..end],"totalBytes":text.len(),
            "nextOffset": if end < text.len() {Some(end)} else {None}});
        // Include JSON escaping in the page allowance so a page of control
        // characters cannot itself become another archived oversized result.
        if page.to_string().len() <= 5 * 1024 {
            return Ok(page);
        }
        end = offset + (end - offset) / 2;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == offset {
            return Err("JSON pointer or page exceeds the tool response budget".into());
        }
    }
}

// Charge images without serializing their potentially multi-megabyte base64.
pub(super) fn units(value: &Value) -> usize {
    if value.get("type").and_then(Value::as_str) == Some("image") {
        return IMAGE_UNITS;
    }
    match value {
        Value::String(text) => text.len().saturating_add(8),
        Value::Array(values) => values
            .iter()
            .fold(16usize, |n, v| n.saturating_add(units(v))),
        Value::Object(values) => values.iter().fold(16usize, |n, (k, v)| {
            n.saturating_add(k.len() + units(v) + 8)
        }),
        _ => 32,
    }
}

fn archived(content: &Value, message: usize, block: usize, limit: usize) -> Value {
    let decoded = content
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let value = decoded.as_ref().unwrap_or(content);
    let summary = crate::summary::summarize_tool_result(value);
    let preview = if let Some(text) = content.as_str() {
        bounded_text(text, limit)
    } else if let Some(blocks) = content.as_array() {
        blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .fold(String::new(), |mut output, text| {
                if output.len() < limit {
                    output.push_str(&bounded_text(text, limit - output.len()));
                }
                output
            })
    } else {
        bounded_text(&content.to_string(), limit)
    };
    json!({"archived":true,"readWith":{"tool":"session.read","message":message,"block":block},
        "revision":summary.revision,"status":summary.status,"error":summary.error,
        "preview":preview,"note":"Partial historical evidence. Read the original before relying on omitted details; recheck current state and pending proposal before editing."})
}

fn project_message(
    message: &Value,
    message_index: usize,
    keep_recent: bool,
    budget: &ContextBudget,
) -> Value {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return message.clone();
    };
    let blocks = blocks
        .iter()
        .enumerate()
        .map(|(block_index, block)| {
            if block["type"] != "tool_result" {
                return block.clone();
            }
            let content = &block["content"];
            let text_units = match content.as_array() {
                Some(blocks) => blocks
                    .iter()
                    .filter(|b| b["type"] != "image")
                    .map(units)
                    .sum(),
                None => units(content),
            };
            if keep_recent && text_units <= budget.max_tool_text_bytes {
                return block.clone();
            }
            let mut projected = block
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| key.as_str() != "content")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            let preview_limit = if keep_recent {
                budget.max_tool_text_bytes / 4
            } else {
                512
            };
            let summary = archived(content, message_index, block_index, preview_limit).to_string();
            // Frames and previews are a recent result's evidence: when only
            // its text is too long, archive the text and keep the images.
            let images = content
                .as_array()
                .filter(|_| keep_recent)
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "image")
                .cloned()
                .collect::<Vec<_>>();
            let content = if images.is_empty() {
                Value::String(summary)
            } else {
                let mut blocks = vec![json!({"type": "text", "text": summary})];
                blocks.extend(images);
                Value::Array(blocks)
            };
            projected.insert("content".into(), content);
            Value::Object(projected)
        })
        .collect::<Vec<_>>();
    json!({"role":message["role"],"content":blocks})
}

/// How the transcript is shown to a provider. Between compactions the view
/// only grows: every message shows the same way on every request, so a
/// provider's prompt cache and the signatures of thinking blocks hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct View {
    /// Messages before this index show archived: tool results as
    /// `session.read` references and project snapshots as a note.
    pub archived: usize,
    /// Thinking blocks before this index are left out. They were made for a
    /// view that has since changed.
    pub thinking_from: usize,
}

/// The first line of a project snapshot added to the transcript.
pub(super) const SNAPSHOT_HEADER: &str =
    "[Project context, read just now. It replaces any earlier project context.]";

fn snapshot(message: &Value) -> bool {
    message["role"] == "user"
        && message["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with(SNAPSHOT_HEADER))
}

/// One message as `view` shows it.
fn show(message: &Value, index: usize, view: &View, budget: &ContextBudget) -> Value {
    let recent = index >= view.archived;
    let mut message = if !recent && snapshot(message) {
        json!({"role": "user", "content": [{"type": "text", "text": format!(
            "[An earlier project context, since replaced. session.read message {index} has it.]"
        )}]})
    } else {
        project_message(message, index, recent, budget)
    };
    if index < view.thinking_from {
        if let Some(blocks) = message["content"].as_array_mut() {
            blocks.retain(|block| {
                !matches!(
                    block["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )
            });
        }
    }
    message
}

/// The transcript as `view` shows it.
pub(super) fn render(messages: &[Value], view: &View, budget: &ContextBudget) -> Vec<Value> {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| show(message, index, view, budget))
        .collect()
}

pub(super) fn cost(messages: &[Value]) -> usize {
    messages.iter().map(|message| units(message) + 256).sum()
}

/// A view of `messages` that fits `available`: the current view when it
/// fits, or else one that archives everything before the latest exchange,
/// then everything. A changed view leaves out earlier thinking blocks,
/// whose signatures no longer match what they were made with.
pub(super) fn fit(
    messages: &[Value],
    view: View,
    available: usize,
    budget: &ContextBudget,
) -> Result<View, String> {
    if cost(&render(messages, &view, budget)) <= available {
        return Ok(view);
    }
    let latest = messages
        .iter()
        .rposition(|message| message["role"] == "assistant")
        .unwrap_or(messages.len());
    for archived in [latest, messages.len()] {
        let compacted = View {
            archived: archived.max(view.archived),
            thinking_from: messages.len(),
        };
        if cost(&render(messages, &compacted, budget)) <= available {
            return Ok(compacted);
        }
    }
    Err("The conversation cannot fit the input budget even with earlier evidence archived; shorten the request or increase KERF_AGENT_INPUT_BUDGET for the selected model.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(id: usize, content: Value) -> [Value; 2] {
        [
            json!({"role":"assistant","content":[{"type":"tool_use","id":format!("call-{id}"),"name":"timeline.query","input":{}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("call-{id}"),"content":content}]}),
        ]
    }

    #[test]
    fn the_view_only_grows_until_it_must_be_compacted() {
        let budget = ContextBudget::default();
        let thinking = json!({"type":"thinking","thinking":"","signature":"sig"});
        let mut messages = vec![json!({"role":"user","content":"Keep Korean captions."})];
        let mut view = View::default();
        let mut previous: Vec<Value> = Vec::new();
        for i in 0..6 {
            let [call, result] = exchange(i, Value::String("evidence ".repeat(1000)));
            let mut call = call;
            call["content"]
                .as_array_mut()
                .unwrap()
                .insert(0, thinking.clone());
            messages.extend([call, result]);
            view = fit(&messages, view, 1_000_000, &budget).unwrap();
            let shown = render(&messages, &view, &budget);
            assert_eq!(shown[..previous.len()], previous[..], "append-only");
            previous = shown;
        }
        assert_eq!(view, View::default());

        // Over budget: everything before the latest exchange is archived,
        // and thinking made for the old view is left out.
        let shown_cost = cost(&previous);
        let compacted = fit(&messages, view, shown_cost / 2, &budget).unwrap();
        assert_eq!(compacted.archived, messages.len() - 2);
        assert_eq!(compacted.thinking_from, messages.len());
        let shown = render(&messages, &compacted, &budget);
        assert!(cost(&shown) <= shown_cost / 2);
        assert_eq!(shown[0]["content"], "Keep Korean captions.");
        assert!(shown[2]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("session.read"));
        assert!(!shown.iter().any(|m| m.to_string().contains("\"thinking\"")));
        assert_eq!(
            shown.last().unwrap()["content"][0]["content"],
            messages.last().unwrap()["content"][0]["content"],
            "the latest result stays whole"
        );
        for (i, message) in shown.iter().enumerate() {
            if message["content"][0]["type"] == "tool_result" {
                assert_eq!(
                    shown[i - 1]["content"][0]["id"],
                    message["content"][0]["tool_use_id"]
                );
            }
        }
    }

    #[test]
    fn archived_snapshots_and_images_point_to_the_transcript() {
        let result = json!([{"type":"text","text":"revision 4, awaiting review"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQID"}}]);
        let mut messages = vec![
            json!({"role":"user","content":"Inspect the shot"}),
            json!({"role":"user","content":[{"type":"text","text":format!("{SNAPSHOT_HEADER}\n\nrevision 4")}]}),
        ];
        messages.extend(exchange(0, result));
        let view = View {
            archived: messages.len(),
            thinking_from: 0,
        };
        let shown = render(&messages, &view, &ContextBudget::default());
        assert!(!shown.iter().any(|m| m.to_string().contains("AQID")));
        assert!(shown[1].to_string().contains("session.read message 1"));
        let restored = read(&messages, json!({"message":3,"block":0,"image":0})).unwrap();
        assert_eq!(restored["images"][0]["data"], "AQID");
    }

    #[test]
    fn archive_pages_reconstruct_unicode_without_losing_bytes() {
        let messages = vec![json!({"role":"user","content":"한글 편집 기준"})];
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let page = read(
                &messages,
                json!({"message":0,"pointer":"/content","offset":offset,"limit":4}),
            )
            .unwrap();
            restored.push_str(page["text"].as_str().unwrap());
            match page["nextOffset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(restored, "한글 편집 기준");
        assert!(read(
            &messages,
            json!({"message":0,"pointer":"/content","offset":1})
        )
        .is_err());
    }

    #[test]
    fn decoded_result_can_be_queried_by_pointer() {
        let messages = vec![exchange(
            0,
            json!({"pendingProposal":{"id":"p1","state":"pending"},"revision":42})
                .to_string()
                .into(),
        )[1]
        .clone()];
        let page = read(
            &messages,
            json!({"message":0,"block":0,"pointer":"/pendingProposal"}),
        )
        .unwrap();
        assert!(page["text"].as_str().unwrap().contains("pending"));
    }

}
