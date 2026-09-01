//! Native terminal entry point for Cut.
//!
//! The CLI is deliberately a thin adapter: project state is read and mutated
//! through `cutd`, while media probing/fetching and the agent are Rust
//! libraries shared with the native app and MCP server.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use cut_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, OpenAiApiMode, ProviderName, ReasoningEffort,
};
use cut_mcp::{inbound::InboundMcpRegistry, CutMcpRouter, McpProfile};
use cut_runtime::RuntimeClient;
use cut_skills::{load_skills, Skill};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(name = "lcut", version, about = "Cut native terminal agent")]
struct Args {
    /// Local media path or a URL to fetch before opening the session.
    media: Option<String>,

    /// LLM API key. Prefer the provider-specific environment variable.
    #[arg(short = 'k', long = "key")]
    key: Option<String>,

    /// anthropic or openai.
    #[arg(long)]
    provider: Option<String>,

    /// Provider model id.
    #[arg(long)]
    model: Option<String>,

    /// OpenAI transport: chat or responses.
    #[arg(long = "api-mode")]
    api_mode: Option<String>,

    /// OpenAI Responses reasoning effort.
    #[arg(long = "reasoning-effort")]
    reasoning_effort: Option<String>,

    /// OpenAI-compatible API root.
    #[arg(long = "base-url")]
    base_url: Option<String>,

    /// Project directory owned by cutd.
    #[arg(long, default_value = ".")]
    dir: PathBuf,

    /// Explicit cutd sidecar path.
    #[arg(long)]
    cutd: Option<PathBuf>,

    /// yt-dlp cookie file.
    #[arg(long)]
    cookies: Option<PathBuf>,

    /// Execute one instruction and exit.
    #[arg(short = 'p', long = "prompt")]
    prompt: Option<String>,

    /// Accepted for parity with the pre-migration launcher.
    #[arg(long)]
    terminal: bool,

    /// List loaded skills and exit.
    #[arg(long)]
    skills: bool,

    /// List native and configured inbound MCP tools and exit.
    #[arg(long)]
    mcp: bool,

    /// Attach configured inbound MCP servers through mcp_search/mcp_call.
    #[arg(long = "mcp-tools")]
    mcp_tools: bool,

    /// Keep the flag accepted for scripts that used the GUI launcher.
    #[arg(long)]
    no_open: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("lcut: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let directory = args
        .dir
        .canonicalize()
        .or_else(|_| {
            fs::create_dir_all(&args.dir)?;
            args.dir.canonicalize()
        })
        .context("resolve project directory")?;
    let skills = project_skills(&directory)?;

    if args.skills {
        print_skills(&skills);
        return Ok(());
    }

    let inbound_mcp_enabled =
        args.mcp_tools || env::var("CUT_ENABLE_MCP").ok().as_deref() == Some("1");
    if args.mcp {
        let router = open_router(&directory, args.cutd.as_deref(), McpProfile::Full)?;
        let inbound = InboundMcpRegistry::connect(&directory, |line| eprintln!("{line}"))
            .map_err(|error| anyhow!(error))?;
        let inbound_servers = inbound.describe();
        if !inbound_servers.is_empty() {
            println!("configured inbound:");
        }
        for server in &inbound_servers {
            println!("{}:", server.server);
            if server.tools.is_empty() {
                println!("  (no tools)");
            }
            for tool in &server.tools {
                println!(
                    "  mcp_{}_{}{}",
                    server.server,
                    tool.name,
                    tool.description
                        .as_deref()
                        .map(|description| format!(" · {description}"))
                        .unwrap_or_default()
                );
            }
        }
        if inbound_servers.is_empty() {
            println!("no configured inbound MCP tools");
        }
        println!("native:");
        for tool in router.list_tools() {
            println!("{}\t{}", tool.name, tool.description);
        }
        return Ok(());
    }

    if let Some(media) = args.media.as_deref() {
        let path = import_media(
            &directory,
            args.cutd.as_deref(),
            media,
            args.cookies.as_deref(),
        )?;
        println!("imported {}", path.display());
    }

    let router = open_router(&directory, args.cutd.as_deref(), McpProfile::Full)?;
    let llm = resolve_llm(&args)?;
    if llm.api_key.is_none() {
        if args.prompt.is_some() || args.terminal || inbound_mcp_enabled {
            bail!("no API key configured; set ANTHROPIC_API_KEY or OPENAI_API_KEY, or pass --key");
        }
        print_snapshot(router.runtime(), "session ready");
        return Ok(());
    }

    let mut tools = router.agent_tools();
    if inbound_mcp_enabled {
        let inbound = InboundMcpRegistry::connect(&directory, |line| eprintln!("{line}"))
            .map_err(|error| anyhow!(error))?;
        tools.extend(inbound.meta_tools());
    }
    let mut session = AgentSession::new(AgentSessionOptions {
        provider: llm.provider,
        api_mode: llm.api_mode,
        reasoning_effort: llm.reasoning_effort,
        api_key: llm.api_key,
        base_url: llm.base_url,
        model: llm.model,
        skills,
        tools,
        ..AgentSessionOptions::default()
    });

    if let Some(prompt) = args.prompt.as_deref() {
        let text = run_prompt(&mut session, prompt)?;
        if !text.is_empty() {
            println!("\n{text}");
        }
        return Ok(());
    }

    run_repl(&mut session, &directory)
}

#[derive(Debug)]
struct LlmConfig {
    provider: ProviderName,
    api_key: Option<String>,
    model: Option<String>,
    api_mode: OpenAiApiMode,
    reasoning_effort: Option<ReasoningEffort>,
    base_url: Option<String>,
}

fn resolve_llm(args: &Args) -> Result<LlmConfig> {
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
    let provider = match provider_name {
        "anthropic" => ProviderName::Anthropic,
        "openai" => ProviderName::Openai,
        other => bail!("unknown provider {other:?}; expected anthropic or openai"),
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
    let api_mode = match api_mode_raw.as_str() {
        "chat" => OpenAiApiMode::Chat,
        "responses" => OpenAiApiMode::Responses,
        other => bail!("unknown OpenAI API mode {other:?}; expected chat or responses"),
    };
    let reasoning_effort = args
        .reasoning_effort
        .as_deref()
        .or_else(|| file_config.get("reasoningEffort").and_then(Value::as_str))
        .map(parse_reasoning_effort)
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

fn parse_reasoning_effort(value: &str) -> Result<ReasoningEffort> {
    Ok(match value {
        "none" => ReasoningEffort::None,
        "minimal" => ReasoningEffort::Minimal,
        "low" => ReasoningEffort::Low,
        "medium" => ReasoningEffort::Medium,
        "high" => ReasoningEffort::High,
        "xhigh" => ReasoningEffort::Xhigh,
        "max" => ReasoningEffort::Max,
        other => bail!("unknown reasoning effort {other:?}"),
    })
}

fn read_user_config() -> Value {
    let Some(home) = env::var_os("HOME") else {
        return json!({});
    };
    fs::read_to_string(PathBuf::from(home).join(".cut/config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn project_skills(directory: &Path) -> Result<Vec<Skill>> {
    let repo_skills = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|root| root.join("skills"))
        .context("resolve bundled skills directory")?;
    let user_skills = env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".cut/skills"))
        .unwrap_or_default();
    let project_skills = directory.join(".cut/skills");
    Ok(load_skills(&[repo_skills, user_skills, project_skills])?)
}

fn print_skills(skills: &[Skill]) {
    if skills.is_empty() {
        println!("no skills loaded");
        return;
    }
    for skill in skills {
        println!("{} · {}", skill.name, skill.description);
    }
}

fn open_router(directory: &Path, cutd: Option<&Path>, profile: McpProfile) -> Result<CutMcpRouter> {
    let cutd = cutd.map(Path::to_path_buf).or_else(discover_cutd);
    CutMcpRouter::new(directory, profile, cutd.as_deref()).map_err(|error| anyhow!(error))
}

fn discover_cutd() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CUTD_BIN") {
        return Some(path.into());
    }
    let current = env::current_exe().ok();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    let mut candidates = Vec::new();
    if let Some(parent) = current.as_deref().and_then(Path::parent) {
        candidates.push(parent.join(binary_name("cutd")));
    }
    candidates.push(root.join("target/debug").join(binary_name("cutd")));
    candidates.push(root.join("target/release").join(binary_name("cutd")));
    candidates.into_iter().find(|path| path.is_file())
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn import_media(
    directory: &Path,
    cutd: Option<&Path>,
    media: &str,
    cookies: Option<&Path>,
) -> Result<PathBuf> {
    let path = if media.starts_with("http://") || media.starts_with("https://") {
        let result = cut_fetcher::fetch_video(
            media,
            directory,
            &cut_fetcher::FetchOptions {
                cookies: cookies.map(Path::to_path_buf),
                ..cut_fetcher::FetchOptions::default()
            },
        )?;
        result.path
    } else {
        let path = PathBuf::from(media);
        if path.is_absolute() {
            path
        } else {
            directory.join(path)
        }
    };
    if !path.is_file() {
        bail!("media file does not exist: {}", path.display());
    }
    let router = open_router(directory, cutd, McpProfile::Core)?;
    let snapshot = router.runtime().get_state().map_err(runtime_error)?;
    let metadata = serde_json::to_value(cut_engine::info(&path.to_string_lossy())?)?;
    let key = operation_key("cli-import");
    router
        .runtime()
        .import_media_request(
            "agent",
            &key,
            &path.to_string_lossy(),
            snapshot.revision,
            Some(metadata),
            None,
        )
        .map_err(runtime_error)?;
    Ok(path)
}

fn print_snapshot(runtime: &RuntimeClient, label: &str) {
    match runtime.get_state() {
        Ok(snapshot) => println!(
            "{label}: revision {}, duration {:.1}s, {} source(s)",
            snapshot.revision,
            snapshot.duration,
            snapshot.sources.len()
        ),
        Err(error) => eprintln!("{label}: {}: {}", error.code, error.message),
    }
}

fn runtime_error(error: cut_runtime::RuntimeError) -> anyhow::Error {
    anyhow!("{}: {}", error.code, error.message)
}

fn run_prompt(session: &mut AgentSession, prompt: &str) -> Result<String> {
    let mut text = String::new();
    let result = session.run_with_events(prompt, |event| match event {
        AgentEvent::TextDelta(delta) => {
            print!("{delta}");
            let _ = io::stdout().flush();
            text.push_str(&delta);
        }
        AgentEvent::ToolCallStarted { name, input, .. } => {
            eprintln!("\n▸ tool {name} {}", compact_json(&input));
        }
        AgentEvent::ToolCallFinished {
            name, ok, detail, ..
        } => {
            if !ok {
                eprintln!("✗ tool {name} failed: {detail}");
            }
        }
        AgentEvent::Aborted => eprintln!("\nagent aborted"),
        AgentEvent::TurnStarted { .. } | AgentEvent::Done { .. } => {}
    })?;
    if text.is_empty() {
        text = result;
    }
    Ok(text)
}

fn run_repl(session: &mut AgentSession, directory: &Path) -> Result<()> {
    println!("lcut, session dir: {}", directory.display());
    println!("Type /quit to exit, /timeline to inspect the current revision.");
    let stdin = io::stdin();
    loop {
        print!("❯ ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        match line {
            "" => continue,
            "/quit" | "/q" | "exit" => break,
            "/timeline" => {
                let router = open_router(directory, None, McpProfile::Core)?;
                print_snapshot(router.runtime(), "timeline");
            }
            "/save" => {
                session.save(&directory.join("conversation.json"))?;
                println!("saved conversation.json");
            }
            prompt => {
                let _ = run_prompt(session, prompt)?;
                println!();
            }
        }
    }
    println!("bye");
    Ok(())
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".into())
        .chars()
        .take(160)
        .collect()
}

fn operation_key(kind: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{kind}-{}-{timestamp}", std::process::id())
}
