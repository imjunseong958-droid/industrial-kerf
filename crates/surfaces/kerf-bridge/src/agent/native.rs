//! Kerf's own agent loop on a model API: the provider, key and model it runs with, the
//! tools it has (Kerf's, the person's MCP servers' and `suggest`), and what it is told of
//! the editor.

use crate::event::{AgentUpdate, NextStep};
use crate::wake::Outbox;
use crate::{cloud_session, emit, media, secrets, AgentSettings, ChatProvider, Event, Shared};
use kerf_agent::{
    AgentSession, AgentSessionOptions, AgentTool, OpenAiApiMode, ProviderName, ReasoningEffort,
};
use kerf_mcp::inbound;
use kerf_mcp::KerfMcpRouter;
use serde_json::json;
use std::sync::Arc;

pub(super) fn native_agent(
    shared: &Arc<Shared>,
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &Outbox,
) -> Result<(AgentSession, KerfMcpRouter), String> {
    // Kerf Cloud speaks the Anthropic Messages protocol with the account token.
    let cloud = if settings.cloud {
        let store = kerf_cloud::Store::default_path()
            .map(|path| cloud_session::load(&path, &secrets::SystemKeychain::new()))
            .unwrap_or_default();
        Some(kerf_cloud::model_credentials(&store).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let provider = if cloud.is_some() {
        ProviderName::Anthropic
    } else if let Some(provider) = settings.provider.and_then(ChatProvider::api) {
        provider
    } else {
        match std::env::var("KERF_PROVIDER").ok().as_deref() {
            Some(name) => {
                ProviderName::parse(name).ok_or("KERF_PROVIDER must be openai or anthropic")?
            }
            None if settings
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()) =>
            {
                // A key entered in the UI without an explicit provider defaults to
                // OpenAI, matching the existing environment fallback.
                ProviderName::Openai
            }
            None if std::env::var_os("OPENAI_API_KEY").is_some() => ProviderName::Openai,
            _ => ProviderName::Anthropic,
        }
    };
    let key_name = if provider == ProviderName::Openai {
        "OPENAI_API_KEY"
    } else {
        "ANTHROPIC_API_KEY"
    };
    let api_key = cloud
        .as_ref()
        .map(|(_, token)| token.clone())
        .or_else(|| settings.api_key.clone())
        .filter(|key| !key.trim().is_empty())
        .or_else(|| std::env::var(key_name).ok())
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Add an API key in Settings ▸ Chat or set {key_name}"))?;
    let api_mode = match settings.api_mode {
        Some(mode) => mode,
        None => match std::env::var("OPENAI_API_MODE") {
            Ok(name) => {
                OpenAiApiMode::parse(&name).ok_or("OPENAI_API_MODE must be chat or responses")?
            }
            Err(_) => OpenAiApiMode::Chat,
        },
    };
    let reasoning_effort = std::env::var("OPENAI_REASONING_EFFORT")
        .ok()
        .map(|value| ReasoningEffort::parse(&value).ok_or("Invalid OPENAI_REASONING_EFFORT"))
        .transpose()?;
    // The editor's own lease covers it too, but an agent session holds
    // kerfd for as long as it lives wherever it runs.
    let router = KerfMcpRouter::new(directory, Some(binary))?.held();
    crate::tools::begin_on_drawers(&router, shared);
    // What it makes on its own, where its person lets it, shows as their own making does.
    router.watch_makings(media::agent_makings(shared.clone(), tx.clone()));
    let mut tools = router.agent_tools();
    inbound::add_agent_tools(&mut tools, directory, settings.mcp_enabled, |line| {
        // Connection errors may contain command arguments or server responses.
        // Never forward raw names, paths, or error details to chat activity.
        emit(
            tx,
            Event::Agent(AgentUpdate::Status {
                text: mcp_connection_status(&line).to_owned(),
            }),
        );
    })
    .map_err(|_| {
        "Could not load MCP configuration. Check ~/.kerf/mcp.json and project .kerf/mcp.json."
            .to_owned()
    })?;
    let next = tx.clone();
    tools.insert(
        "suggest".to_owned(),
        AgentTool::new(
            "suggest",
            SUGGEST_DOC,
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["steps"],
                "properties": {"steps": {
                    "type": "array", "minItems": 1, "maxItems": MAX_NEXT_STEPS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["label", "request"],
                        "properties": {
                            "label": {"type": "string", "maxLength": 40, "description": "The button: a few words, such as \"Stronger glow\"."},
                            "request": {"type": "string", "description": "What the person asks by pressing it, in their words."}
                        }
                    }
                }}
            }),
            move |input| {
                let steps: Vec<NextStep> = input["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|step| {
                        let label = step["label"].as_str()?.trim();
                        let request = step["request"].as_str()?.trim();
                        (!label.is_empty() && !request.is_empty()).then(|| NextStep {
                            label: label.chars().take(40).collect(),
                            request: request.to_owned(),
                        })
                    })
                    .take(MAX_NEXT_STEPS)
                    .collect();
                if steps.is_empty() {
                    return Err("give one to three steps, each with a label and a request".into());
                }
                let shown = steps.len();
                emit(&next, Event::AgentNext { steps });
                Ok(json!({"shown": shown}))
            },
        ),
    );
    let session = AgentSession::new(AgentSessionOptions {
        provider,
        api_mode,
        reasoning_effort,
        api_key: Some(api_key),
        model: settings
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .or_else(|| std::env::var("KERF_MODEL").ok()),
        base_url: cloud
            .map(|(url, _)| url)
            .or_else(|| settings.base_url.clone())
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                std::env::var(if provider == ProviderName::Openai {
                    "OPENAI_BASE_URL"
                } else {
                    "ANTHROPIC_BASE_URL"
                })
                .ok()
            }),
        system_prompt: Some(format!(
            "{}\n\n{EDITOR_GUIDE}",
            router.agent_system_prompt()
        )),
        tools,
        context: Some(router.agent_context()),
        ..Default::default()
    });
    Ok((session, router))
}

/// The most next steps the editor shows at once.
const MAX_NEXT_STEPS: usize = 3;

const SUGGEST_DOC: &str = "Offer the person one to three next steps as buttons in the editor. \
Pressing one sends its request to you as if they had typed it.";

/// What the built-in agent does in the editor beyond the shared guide.
const EDITOR_GUIDE: &str = "# In the editor\n\n\
The person works in Kerf's editor and answers with buttons more easily than \
by typing. End each turn by calling `suggest` with one to three next steps \
they are likely to want: a variation on what you just did (another effect or \
transition for that moment, stronger or subtler), the next part of the edit, \
or checking the result. Labels are a few words; requests are what the person \
would say. Do not repeat a step they just took.\n\n\
The timeline shows your proposal as it grows, so they watch you work. Build a \
longer edit in steps: stage the first part with `edit`, check it, then `extend` \
the proposal with the next part, rather than one large script at the end.\n\n\
Asked for a vertical or square cut, set the format first in the same proposal \
(`sequence set s1 size=1080x1920`) and frame footage of another shape to fill it; \
the picture takes the proposal's format as it grows.";

fn mcp_connection_status(line: &str) -> &'static str {
    if line.starts_with("mcp: connected ") {
        "MCP server connected"
    } else if line == inbound::TRACKED_PROJECT_CONFIG {
        "Skipped the project's .kerf/mcp.json: Git tracks it, so it came with the repository \
         rather than from you. Untrack it (git rm --cached .kerf/mcp.json) to use its servers."
    } else {
        "MCP server unavailable; skipped. Check its local command and configuration."
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mcp_connection_activity_omits_untrusted_details() {
        assert_eq!(
            super::mcp_connection_status("mcp: connected secret (2 tools)"),
            "MCP server connected"
        );
        let status = super::mcp_connection_status(
            "warning: mcp server 'secret' unavailable (token=secret), skipping",
        );
        assert!(status.contains("unavailable"));
        assert!(!status.contains("secret"));
        let skipped = super::mcp_connection_status(kerf_mcp::inbound::TRACKED_PROJECT_CONFIG);
        assert!(
            skipped.contains(".kerf/mcp.json") && skipped.contains("Git tracks it"),
            "{skipped}"
        );
    }
}
