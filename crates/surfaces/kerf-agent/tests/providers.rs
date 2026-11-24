//! What each provider adapter reads a reply as, from a client that answers as its API does:
//! fields and events it does not read change nothing, and the blocks of an Anthropic reply
//! keep what the API put on them, since they go back to it in the transcript.

use kerf_agent::{
    AnthropicProvider, JsonHttpClient, LlmProvider, OpenAiProvider, OpenAiResponsesProvider,
    ProviderChunk, ProviderError, ProviderRequest,
};
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Answers every request with `0`, as the server said it.
struct Replay(Vec<Value>);

impl JsonHttpClient for Replay {
    fn post_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: &Value,
    ) -> Result<Value, ProviderError> {
        Ok(self.0[0].clone())
    }

    fn post_stream(
        &self,
        _: &str,
        _: &[(String, String)],
        _: &Value,
    ) -> Result<Vec<Value>, ProviderError> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Copy, Debug)]
enum Api {
    Anthropic,
    Chat,
    Responses,
}

fn read(api: Api, said: Vec<Value>) -> Vec<ProviderChunk> {
    let client = Arc::new(Replay(said));
    let provider: Box<dyn LlmProvider> = match api {
        Api::Anthropic => Box::new(AnthropicProvider::with_client(client, None, None)),
        Api::Chat => Box::new(OpenAiProvider::with_client(client, None, None)),
        Api::Responses => Box::new(OpenAiResponsesProvider::with_client(
            client, None, None, None,
        )),
    };
    let request = ProviderRequest {
        model: "model".into(),
        system: String::new(),
        tools: Vec::new(),
        messages: vec![json!({"role": "user", "content": "cut it"})],
    };
    provider.stream(&request).expect("a reply")
}

/// A streamed reply with text and a tool call, as each API says one.
fn stream(api: Api) -> Vec<Value> {
    match api {
        Api::Anthropic => vec![
            json!({"type": "message_start", "message": {"content": []}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Plan."}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "c2ln"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Cut"}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "ting."}}),
            json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "edit", "input": {}}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"at\":"}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "3}"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}),
        ],
        Api::Chat => vec![
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "Cut"}}]}),
            json!({"choices": [{"index": 0, "delta": {"content": "ting."}}]}),
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "edit", "arguments": "{\"at\":"}}]}}]}),
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "3}"}}]}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        ],
        Api::Responses => vec![
            json!({"type": "response.output_text.delta", "output_index": 0, "delta": "Cut"}),
            json!({"type": "response.output_text.delta", "output_index": 0, "delta": "ting."}),
            json!({"type": "response.output_item.added", "output_index": 1, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "edit", "arguments": ""}}),
            json!({"type": "response.function_call_arguments.delta", "call_id": "call_1", "delta": "{\"at\":"}),
            json!({"type": "response.function_call_arguments.delta", "call_id": "call_1", "delta": "3}"}),
        ],
    }
}

/// `said` with `extra` on it and on each object in it, but for a block's own fields and a
/// tool's arguments.
fn with_extra(said: &mut Value, extra: &Map<String, Value>) {
    match said {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if key != "content_block" && key != "input" {
                    with_extra(value, extra);
                }
            }
            object.extend(extra.clone());
        }
        Value::Array(items) => items.iter_mut().for_each(|item| with_extra(item, extra)),
        _ => {}
    }
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-z ]{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(2, 8, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::from),
            prop::collection::btree_map("[a-z]{1,4}", inner, 0..3)
                .prop_map(|fields| Value::Object(fields.into_iter().collect())),
        ]
    })
}

/// Fields no API here has: their names begin `x_`.
fn extra() -> impl Strategy<Value = Map<String, Value>> {
    prop::collection::btree_map("x_[a-z]{1,6}", json_value(), 0..4)
        .prop_map(|fields| fields.into_iter().collect())
}

proptest! {
    #[test]
    fn fields_and_events_no_api_here_has_change_no_reply(
        api in prop_oneof![Just(Api::Anthropic), Just(Api::Chat), Just(Api::Responses)],
        extra in extra(),
        unknown in prop::collection::vec((0usize..12, "x_[a-z]{1,6}", extra()), 0..3),
    ) {
        let plain = read(api, stream(api));
        let mut said = stream(api);
        for event in &mut said {
            with_extra(event, &extra);
        }
        for (at, kind, fields) in unknown {
            let mut event = Value::Object(fields);
            event["type"] = Value::from(kind);
            said.insert(at.min(said.len()), event);
        }
        prop_assert_eq!(read(api, said), plain);
    }

    #[test]
    fn an_anthropic_replys_blocks_keep_what_the_api_put_on_them(
        extras in prop::collection::vec(extra(), 4),
        streamed in any::<bool>(),
    ) {
        let blocks = [
            json!({"type": "thinking", "thinking": "Plan.", "signature": "c2ln"}),
            json!({"type": "text", "text": "Cutting."}),
            json!({"type": "tool_use", "id": "toolu_1", "name": "edit", "input": {"at": 3}}),
            json!({"type": "redacted_thinking", "data": "ZGF0YQ=="}),
        ];
        let blocks: Vec<Value> = blocks
            .into_iter()
            .zip(extras)
            .map(|(mut block, extra)| {
                block.as_object_mut().unwrap().extend(extra);
                block
            })
            .collect();
        let said = if streamed {
            // Each block begins as the API begins one, with its fields, and grows.
            let mut said = Vec::new();
            for (index, block) in blocks.iter().enumerate() {
                let mut begun = block.clone();
                match block["type"].as_str() {
                    Some("thinking") => {
                        begun["thinking"] = json!("");
                        begun["signature"] = json!("");
                        said.push(json!({"type": "content_block_start", "index": index, "content_block": begun}));
                        said.push(json!({"type": "content_block_delta", "index": index, "delta": {"type": "thinking_delta", "thinking": "Plan."}}));
                        said.push(json!({"type": "content_block_delta", "index": index, "delta": {"type": "signature_delta", "signature": "c2ln"}}));
                    }
                    Some("text") => {
                        begun["text"] = json!("");
                        said.push(json!({"type": "content_block_start", "index": index, "content_block": begun}));
                        said.push(json!({"type": "content_block_delta", "index": index, "delta": {"type": "text_delta", "text": "Cutting."}}));
                    }
                    Some("tool_use") => {
                        begun["input"] = json!({});
                        said.push(json!({"type": "content_block_start", "index": index, "content_block": begun}));
                        said.push(json!({"type": "content_block_delta", "index": index, "delta": {"type": "input_json_delta", "partial_json": "{\"at\": 3}"}}));
                    }
                    _ => said.push(json!({"type": "content_block_start", "index": index, "content_block": begun})),
                }
            }
            said.push(json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}));
            said
        } else {
            vec![json!({"type": "message", "content": blocks, "stop_reason": "tool_use"})]
        };
        let Some(ProviderChunk::MessageEnd { stop_reason, content }) = read(Api::Anthropic, said).pop() else {
            panic!("a reply ends");
        };
        prop_assert_eq!(stop_reason, "tool_use");
        prop_assert_eq!(content, blocks);
    }
}

#[test]
fn a_field_said_otherwise_than_its_api_says_it_costs_only_itself() {
    let chunks = read(
        Api::Anthropic,
        vec![
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Cut."}, "usage": "none"}),
            json!({"type": "message_delta", "delta": {"stop_reason": 5}}),
        ],
    );
    assert_eq!(
        chunks,
        [
            ProviderChunk::TextDelta {
                text: "Cut.".into()
            },
            ProviderChunk::MessageEnd {
                stop_reason: "end_turn".into(),
                content: vec![json!({"type": "text", "text": "Cut."})],
            },
        ]
    );
    let chunks = read(
        Api::Chat,
        vec![
            json!({"choices": [{"delta": {"content": "Cut.", "tool_calls": "none"}, "finish_reason": 5}]}),
        ],
    );
    assert_eq!(
        chunks[0],
        ProviderChunk::TextDelta {
            text: "Cut.".into()
        }
    );
    let chunks = read(
        Api::Responses,
        vec![
            json!({"type": "response.output_item.added", "item": {"type": "function_call", "call_id": "call_1", "name": 5, "arguments": "{}"}}),
        ],
    );
    assert_eq!(
        chunks,
        [ProviderChunk::MessageEnd {
            stop_reason: "tool_use".into(),
            content: vec![json!({"type": "tool_use", "id": "call_1", "name": "", "input": {}})],
        }]
    );
}
