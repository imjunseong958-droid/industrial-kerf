//! The model an agent runs on, as the arguments, the environment and the person's own file say:
//! for the terminal agent, and for the editor `kerf` starts.

use crate::Args;
use anyhow::{anyhow, bail, Result};
use kerf_agent::{OpenAiApiMode, ProviderName, ReasoningEffort};
use serde_json::{json, Value};
use std::env;
use std::fs;

#[derive(Debug)]
pub(crate) struct LlmConfig {
    pub(crate) provider: ProviderName,
    pub(crate) api_key: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) api_mode: OpenAiApiMode,
    pub(crate) reasoning_effort: Option<ReasoningEffort>,
    pub(crate) base_url: Option<String>,
}

pub(crate) fn resolve_llm(args: &Args) -> Result<LlmConfig> {
    let file_config = read_user_config();
    let provider_name = args
        .provider
        .as_deref()
        .or_else(|| file_config.get("provider").and_then(Value::as_str))
        .or_else(|| {
            if env::var_os("ANTHROPIC_API_KEY").is_some() {
                Some("anthropic")
            } else if env::var_os("OPENAI_API_KEY").is_some() {
                Some("openai")
            } else {
                None
            }
        })
        .unwrap_or("anthropic");
    let Some(provider) = ProviderName::parse(provider_name) else {
        bail!("unknown provider {provider_name:?}; expected anthropic or openai");
    };
    let api_mode_raw = args
        .api_mode
        .as_deref()
        .map(str::to_owned)
        .or_else(|| env::var("OPENAI_API_MODE").ok())
        .or_else(|| {
            file_config
                .get("apiMode")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "chat".into());
    let Some(api_mode) = OpenAiApiMode::parse(&api_mode_raw) else {
        bail!("unknown OpenAI API mode {api_mode_raw:?}; expected chat or responses");
    };
    let reasoning_effort = args
        .reasoning_effort
        .as_deref()
        .or_else(|| file_config.get("reasoningEffort").and_then(Value::as_str))
        .map(|value| {
            ReasoningEffort::parse(value)
                .ok_or_else(|| anyhow!("unknown reasoning effort {value:?}"))
        })
        .transpose()?;
    let config_key = file_config.get("apiKey").and_then(Value::as_str);
    let api_key = args
        .key
        .clone()
        .or_else(|| {
            if provider == ProviderName::Anthropic {
                env::var("ANTHROPIC_API_KEY").ok()
            } else {
                env::var("OPENAI_API_KEY").ok()
            }
        })
        .or_else(|| config_key.map(str::to_owned));
    let base_url = (provider == ProviderName::Openai)
        .then(|| {
            args.base_url
                .clone()
                .or_else(|| env::var("OPENAI_BASE_URL").ok())
                .or_else(|| {
                    file_config
                        .get("baseUrl")
                        .or_else(|| file_config.get("baseURL"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
        })
        .flatten();
    let model = args.model.clone().or_else(|| {
        file_config
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    Ok(LlmConfig {
        provider,
        api_key,
        model,
        api_mode,
        reasoning_effort,
        base_url,
    })
}

fn read_user_config() -> Value {
    let Some(folder) = kerf_paths::user_dir() else {
        return json!({});
    };
    fs::read_to_string(folder.join("config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}
