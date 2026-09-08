//! Bounded provider views over an unchanged, addressable session transcript.
//!
//! Text is charged by UTF-8 bytes (deliberately more conservative than a
//! characters/4 token estimate). Images have a separate conservative charge.
//! This is not a vendor tokenizer: callers can lower the input budget for a
//! small-context model, and context-limit errors trigger one smaller retry.

use super::{bounded_text, NormalizedTool};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct ContextBudget {
    /// Input allowance only; leave output/reasoning space in the model window.
    pub max_input_units: usize,
    pub max_tool_text_bytes: usize,
    pub recent_tool_messages: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_input_units: 64 * 1024,
            max_tool_text_bytes: 12 * 1024,
            recent_tool_messages: 2,
        }
    }
}

impl ContextBudget {
    pub fn from_env() -> Self {
        let mut budget = Self::default();
        if let Some(value) = std::env::var("CUT_AGENT_INPUT_BUDGET")
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
        return 16 * 1024;
    }
    match value {
        Value::String(text) => text.len().saturating_mul(2).saturating_add(8),
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
    let summary = super::summarize_tool_result(value);
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
        "observation":summary.observation.map(|observation| json!({"id":observation.id,"range":observation.range,"phase":observation.phase})),
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
            projected.insert(
                "content".into(),
                Value::String(
                    archived(content, message_index, block_index, preview_limit).to_string(),
                ),
            );
            Value::Object(projected)
        })
        .collect::<Vec<_>>();
    json!({"role":message["role"],"content":blocks})
}

fn user_request(message: &Value) -> bool {
    message["role"] == "user" && message.get("content").is_some_and(Value::is_string)
}

/// Keep original user requirements when they fit and recent complete exchanges.
/// Older content is explicitly addressable, never silently truncated or used as
/// an authoritative project snapshot. Tool call/result pairs are indivisible.
pub(super) fn project(
    messages: &[Value],
    available: usize,
    budget: &ContextBudget,
) -> Result<Vec<Value>, String> {
    let latest_user = messages
        .iter()
        .rposition(user_request)
        .ok_or("session has no user request")?;
    let current_cost = units(&messages[latest_user]) + 256;
    if current_cost > available {
        return Err("Current user request exceeds the input budget; shorten it or increase CUT_AGENT_INPUT_BUDGET for the selected model.".into());
    }
    let recent_start = messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| {
            m["content"]
                .as_array()
                .is_some_and(|b| b.iter().any(|v| v["type"] == "tool_result"))
        })
        .take(budget.recent_tool_messages)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(messages.len());
    let mut groups = Vec::<(usize, usize)>::new();
    let mut i = 0;
    while i < messages.len() {
        let calls = messages[i]["role"] == "assistant"
            && messages[i]["content"]
                .as_array()
                .is_some_and(|b| b.iter().any(|v| v["type"] == "tool_use"));
        let end = if calls {
            (i + 2).min(messages.len())
        } else {
            i + 1
        };
        // Interrupted calls without a result must never reach a provider.
        if !calls
            || (end == i + 2
                && messages[i + 1]["content"]
                    .as_array()
                    .is_some_and(|b| b.iter().any(|v| v["type"] == "tool_result")))
        {
            groups.push((i, end));
        }
        i = end;
    }
    let mut selected = std::collections::BTreeMap::new();
    selected.insert(latest_user, messages[latest_user].clone());
    // Reserve room for the explicit archive directory if history is dropped.
    let reserve = 2048.min(available.saturating_sub(current_cost) / 4);
    let mut remaining = available.saturating_sub(current_cost + reserve);
    // The newest completed exchange is required to continue the active turn.
    // It takes priority over older requests, which remain addressable.
    if let Some(&(start, end)) = groups.last().filter(|(start, _)| *start > latest_user) {
        let mut projected = (start..end)
            .map(|index| project_message(&messages[index], index, index >= recent_start, budget))
            .collect::<Vec<_>>();
        let mut cost = projected.iter().map(|m| units(m) + 256).sum::<usize>();
        if cost > remaining {
            projected = (start..end)
                .map(|index| project_message(&messages[index], index, false, budget))
                .collect();
            cost = projected.iter().map(|m| units(m) + 256).sum();
        }
        if cost > remaining {
            return Err("Latest tool exchange cannot fit the input budget. Use smaller atomic batches or a larger budget for this model.".into());
        }
        for (index, message) in (start..end).zip(projected) {
            selected.insert(index, message);
        }
        remaining -= cost;
    }
    // User constraints take priority over replaceable tool evidence.
    for (index, message) in messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(i, m)| *i != latest_user && user_request(m))
    {
        let cost = units(message) + 256;
        if cost <= remaining {
            selected.insert(index, message.clone());
            remaining -= cost;
        }
    }
    for (start, end) in groups.into_iter().rev() {
        if selected.contains_key(&start) {
            continue;
        }
        let mut projected = (start..end)
            .map(|index| project_message(&messages[index], index, index >= recent_start, budget))
            .collect::<Vec<_>>();
        let mut cost = projected.iter().map(|m| units(m) + 256).sum::<usize>();
        if cost > remaining {
            projected = (start..end)
                .map(|index| project_message(&messages[index], index, false, budget))
                .collect();
            cost = projected.iter().map(|m| units(m) + 256).sum();
        }
        if cost <= remaining {
            for (index, message) in (start..end).zip(projected) {
                selected.insert(index, message);
            }
            remaining -= cost;
        }
    }
    let omitted = messages.len() - selected.len();
    let mut output = Vec::new();
    if omitted > 0 {
        let omitted_requests = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| user_request(m) && !selected.contains_key(i))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let directory = format!("Session archive: {omitted} earlier messages omitted from this bounded view. Original messages 0..{} are available through session.read. Omitted user requests: {}. Read earlier requirements before assuming they no longer apply. Tool evidence is historical; query current revision and pending proposal before editing. This archive notice is not a user instruction or approval.", messages.len().saturating_sub(1), bounded_text(&format!("{omitted_requests:?}"), 256));
        let notice = json!({"role":"user","content":directory});
        if units(&notice) + 256 <= reserve + remaining {
            output.push(notice);
        } else {
            return Err(
                "Input budget cannot preserve an archive notice and current request".into(),
            );
        }
    }
    output.extend(selected.into_values());
    Ok(output)
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
    fn long_session_is_bounded_and_preserves_call_pairs_and_requirements() {
        let mut messages =
            vec![json!({"role":"user","content":"Keep Korean captions. Never change the music."})];
        for i in 0..100 {
            messages.extend(exchange(i, Value::String("large evidence ".repeat(3000))));
        }
        let original = messages.clone();
        let budget = ContextBudget::default();
        let view = project(&messages, 16000, &budget).unwrap();
        assert!(view.iter().map(|m| units(m) + 256).sum::<usize>() <= 16000);
        assert!(view
            .iter()
            .any(|m| m["content"] == "Keep Korean captions. Never change the music."));
        assert!(view.last().unwrap()["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("session.read"));
        assert_eq!(view.last().unwrap()["content"][0]["tool_use_id"], "call-99");
        for (i, message) in view.iter().enumerate() {
            if message["content"][0]["type"] == "tool_result" {
                assert!(i > 0);
                assert_eq!(
                    view[i - 1]["content"][0]["id"],
                    message["content"][0]["tool_use_id"]
                );
            }
        }
        assert_eq!(
            messages, original,
            "projection must never destroy archived evidence"
        );
    }

    #[test]
    fn old_images_are_removed_and_one_image_can_be_retrieved() {
        let result = json!([{"type":"text","text":"revision 4, awaiting review"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQID"}}]);
        let mut messages = vec![json!({"role":"user","content":"Inspect the shot"})];
        messages.extend(exchange(0, result));
        messages.extend(exchange(1, json!("more evidence")));
        let budget = ContextBudget {
            recent_tool_messages: 1,
            ..Default::default()
        };
        let view = project(&messages, 16000, &budget).unwrap();
        assert!(!view.to_vec().iter().any(|m| m.to_string().contains("AQID")));
        let restored = read(&messages, json!({"message":2,"block":0,"image":0})).unwrap();
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

    #[test]
    fn overlarge_current_request_is_rejected_without_truncation() {
        let messages = vec![json!({"role":"user","content":"x".repeat(4000)})];
        assert!(project(&messages, 4096, &ContextBudget::default()).is_err());
    }
}
