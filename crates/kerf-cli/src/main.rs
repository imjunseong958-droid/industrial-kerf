//! Native Kerf launcher and terminal entry point.
//!
//! `kerf` opens the native editor by default. `--terminal` keeps the
//! project-scoped terminal agent available for scripts and interactive shells.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTargetSummary, OpenAiApiMode, ProviderName,
    ReasoningEffort,
};
use kerf_mcp::{docs::add_agents_pointer, inbound, load_project_skills, KerfMcpRouter};
use kerf_runtime::RuntimeClient;
use kerf_skills::Skill;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(
    name = "kerf",
    version,
    about = "Industrial Kerf native editor and terminal agent"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// Local media path or a URL to import before opening the editor/session.
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

    /// Reasoning effort: OpenAI Responses reasoning, or Anthropic effort.
    #[arg(long = "reasoning-effort")]
    reasoning_effort: Option<String>,

    /// OpenAI-compatible API root.
    #[arg(long = "base-url")]
    base_url: Option<String>,

    /// Project directory owned by kerfd.
    #[arg(long, default_value = ".", global = true)]
    dir: PathBuf,

    /// Explicit kerfd sidecar path.
    #[arg(long, global = true)]
    kerfd: Option<PathBuf>,

    /// yt-dlp cookie file.
    #[arg(long)]
    cookies: Option<PathBuf>,

    /// Execute one instruction and exit.
    #[arg(short = 'p', long = "prompt")]
    prompt: Option<String>,

    /// Use the terminal agent instead of opening the native editor.
    #[arg(long)]
    terminal: bool,

    /// List loaded skills and exit.
    #[arg(long)]
    skills: bool,

    /// List native and configured inbound MCP tools and exit.
    #[arg(long)]
    mcp: bool,

    /// Generate the project-local agent documentation bundle and exit.
    #[arg(long = "generate-docs", alias = "docs")]
    generate_docs: bool,

    /// Explicitly append a pointer to the generated docs in AGENTS.md.
    #[arg(long = "add-agents-pointer")]
    add_agents_pointer: bool,

    /// Attach configured inbound MCP servers through mcp_search/mcp_call.
    #[arg(long = "mcp-tools")]
    mcp_tools: bool,

    /// Keep the flag accepted for launcher-script compatibility.
    #[arg(long)]
    no_open: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Save, inspect, share and apply portable editing patterns. No API key needed.
    Preset {
        #[command(subcommand)]
        action: PresetAction,
    },
}

#[derive(Debug, Subcommand)]
enum PresetAction {
    /// List presets in the project root and .kerf/presets.
    List,
    /// Save the accepted sequence to a new .kerfpreset file.
    Save {
        file: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        sequence: Option<String>,
    },
    /// Inspect the pattern, minimum source lengths, and text slots.
    Show { file: String },
    /// Stage a proposal replacing a sequence; accept it in the editor.
    Apply {
        file: String,
        /// SLOT=MEDIA or SLOT=MEDIA@FROM (e.g. slot1=m2@1.5s).
        #[arg(long = "bind")]
        bindings: Vec<String>,
        /// TEMPLATE_CLIP=TEXT; quote values containing spaces.
        #[arg(long = "text")]
        texts: Vec<String>,
        #[arg(long)]
        sequence: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("kerf: {error:#}");
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
    let home = env::var_os("HOME").map(PathBuf::from);
    for note in kerf_runtime::migrate_from_cut(home.as_deref(), Some(&directory)) {
        eprintln!("kerf: {note}");
    }

    if let Some(Command::Preset { action }) = &args.command {
        return run_preset(action, &directory, args.kerfd.as_deref());
    }

    if !uses_terminal(&args) {
        return run_gui(&args, &directory);
    }

    let skills = load_project_skills(&directory)?;

    if args.skills {
        print_skills(&skills);
        return Ok(());
    }

    if args.generate_docs || args.add_agents_pointer {
        let router = open_router(&directory, args.kerfd.as_deref())?;
        let generated = router
            .generate_agent_docs()
            .map_err(|error| anyhow!(error))?;
        println!(
            "generated {} agent doc(s) at {} (revision {})",
            generated.files.len(),
            generated.directory.display(),
            generated.revision
        );
        if args.add_agents_pointer {
            let path = add_agents_pointer(&directory).map_err(|error| anyhow!(error))?;
            println!("ensured generated-docs pointer at {}", path.display());
        }
        return Ok(());
    }

    let inbound_mcp_enabled = inbound::enabled(args.mcp_tools);
    if args.mcp {
        let router = open_router(&directory, args.kerfd.as_deref())?;
        let inbound = inbound::InboundMcpRegistry::connect(&directory, |line| eprintln!("{line}"))
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
            args.kerfd.as_deref(),
            media,
            args.cookies.as_deref(),
        )?;
        println!("imported {}", path.display());
    }

    let router = open_router(&directory, args.kerfd.as_deref())?;
    let llm = resolve_llm(&args)?;
    if llm.api_key.is_none() {
        if args.prompt.is_some() || args.terminal || inbound_mcp_enabled {
            bail!("no API key configured; set ANTHROPIC_API_KEY or OPENAI_API_KEY, or pass --key");
        }
        print_snapshot(router.runtime(), "session ready");
        return Ok(());
    }

    let mut tools = router.agent_tools();
    inbound::add_agent_tools(&mut tools, &directory, args.mcp_tools, |line| {
        eprintln!("{line}")
    })
    .map_err(|error| anyhow!(error))?;
    let mut session = AgentSession::new(AgentSessionOptions {
        provider: llm.provider,
        api_mode: llm.api_mode,
        reasoning_effort: llm.reasoning_effort,
        api_key: llm.api_key,
        base_url: llm.base_url,
        model: llm.model,
        system_prompt: Some(router.agent_system_prompt()),
        tools,
        context: Some(router.agent_context()),
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

// Optional MCP configuration belongs to the agent, not the UI selection.
fn uses_terminal(args: &Args) -> bool {
    args.command.is_some()
        || args.terminal
        || args.prompt.is_some()
        || args.skills
        || args.mcp
        || args.generate_docs
        || args.add_agents_pointer
        || args.mcp_tools
}

fn run_preset(action: &PresetAction, directory: &Path, kerfd: Option<&Path>) -> Result<()> {
    let (tool, arguments) = preset_request(action)?;
    let router = open_router(directory, kerfd)?;
    let response = router.call(tool, arguments);
    if response["ok"] != true {
        bail!(
            "{}: {}",
            response["error"]["code"].as_str().unwrap_or("PRESET_ERROR"),
            response["error"]["message"]
                .as_str()
                .unwrap_or("preset operation failed")
        );
    }
    let mut result = response["result"].clone();
    // Images remain available in the editor and MCP response; the terminal
    // prints their paths, without megabytes of inline image data.
    fn omit_images(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                fields.remove("imageBase64");
                for value in fields.values_mut() {
                    omit_images(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    omit_images(value);
                }
            }
            _ => {}
        }
    }
    omit_images(&mut result);
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn preset_request(action: &PresetAction) -> Result<(&'static str, Value)> {
    Ok(match action {
        PresetAction::List => ("look", json!({"presets": true})),
        PresetAction::Show { file } => ("look", json!({"preset": file})),
        PresetAction::Save {
            file,
            name,
            sequence,
        } => (
            "edit",
            json!({"savePreset": file, "name": name, "sequence": sequence}),
        ),
        PresetAction::Apply {
            file,
            bindings,
            texts,
            sequence,
        } => {
            let mut media = serde_json::Map::new();
            for binding in bindings {
                let (slot, source) = binding
                    .split_once('=')
                    .context("--bind needs SLOT=MEDIA or SLOT=MEDIA@FROM")?;
                let (id, from) = source.split_once('@').unwrap_or((source, "0f"));
                if slot.is_empty() || id.is_empty() || from.is_empty() {
                    bail!("empty slot, media or source start in --bind");
                }
                if media
                    .insert(slot.into(), json!({"media": id, "from": from}))
                    .is_some()
                {
                    bail!("duplicate binding for `{slot}`");
                }
            }
            let mut overrides = serde_json::Map::new();
            for text in texts {
                let (clip, value) = text
                    .split_once('=')
                    .context("--text needs TEMPLATE_CLIP=TEXT")?;
                if clip.is_empty() {
                    bail!("empty text slot");
                }
                if overrides.insert(clip.into(), json!(value)).is_some() {
                    bail!("duplicate text for `{clip}`");
                }
            }
            (
                "edit",
                json!({"preset": file, "bindings": media, "texts": overrides, "sequence": sequence}),
            )
        }
    })
}

fn run_gui(args: &Args, directory: &Path) -> Result<()> {
    let llm = resolve_llm(args)?;
    configure_gui_agent(&llm);

    if let Some(media) = args.media.as_deref() {
        let path = import_media(
            directory,
            args.kerfd.as_deref(),
            media,
            args.cookies.as_deref(),
        )?;
        println!("imported {}", path.display());
    }

    let qt = discover_kerf_qt()?;
    let kerfd = args
        .kerfd
        .clone()
        .or_else(discover_kerfd)
        .unwrap_or_else(|| PathBuf::from(binary_name("kerfd")));
    let status = std::process::Command::new(&qt)
        .arg(directory)
        .arg(&kerfd)
        .status()
        .with_context(|| format!("could not launch {}", qt.display()))?;
    if !status.success() {
        bail!("Qt editor exited with {status}");
    }
    Ok(())
}

fn configure_gui_agent(llm: &LlmConfig) {
    env::set_var("KERF_PROVIDER", llm.provider.name());

    if let Some(api_key) = llm.api_key.as_deref() {
        let key_name = match llm.provider {
            ProviderName::Anthropic => "ANTHROPIC_API_KEY",
            ProviderName::Openai => "OPENAI_API_KEY",
        };
        env::set_var(key_name, api_key);
    }
    if let Some(model) = llm.model.as_deref() {
        env::set_var("KERF_MODEL", model);
    }
    if let Some(base_url) = llm.base_url.as_deref() {
        env::set_var("OPENAI_BASE_URL", base_url);
    }
    env::set_var("OPENAI_API_MODE", llm.api_mode.name());
    if let Some(reasoning_effort) = llm.reasoning_effort {
        env::set_var("OPENAI_REASONING_EFFORT", reasoning_effort.name());
    }
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
    let Some(home) = env::var_os("HOME") else {
        return json!({});
    };
    fs::read_to_string(PathBuf::from(home).join(".kerf/config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
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

fn open_router(directory: &Path, kerfd: Option<&Path>) -> Result<KerfMcpRouter> {
    let kerfd = kerfd.map(Path::to_path_buf).or_else(discover_kerfd);
    KerfMcpRouter::new(directory, kerfd.as_deref()).map_err(|error| anyhow!(error))
}

fn discover_kerfd() -> Option<PathBuf> {
    if let Some(path) = env::var_os("KERFD_BIN") {
        return Some(path.into());
    }
    let current = env::current_exe().ok();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    let mut candidates = Vec::new();
    if let Some(parent) = current.as_deref().and_then(Path::parent) {
        candidates.push(parent.join(binary_name("kerfd")));
    }
    candidates.push(root.join("target/debug").join(binary_name("kerfd")));
    candidates.push(root.join("target/release").join(binary_name("kerfd")));
    candidates.into_iter().find(|path| path.is_file())
}

fn discover_kerf_qt() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KERF_QT_BIN").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        bail!("configured Qt editor does not exist: {}", path.display());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("could not resolve repository root")?;
    let mut candidates = Vec::new();
    if let Some(parent) = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        candidates.push(parent.join(binary_name("kerf-qt")));
    }
    candidates.push(root.join("target/qt").join(binary_name("kerf-qt")));
    candidates.push(root.join("target/debug").join(binary_name("kerf-qt")));
    candidates.push(root.join("target/release").join(binary_name("kerf-qt")));
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!("Qt editor not found; build it with scripts/run-kerf-qt.sh or set KERF_QT_BIN")
        })
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
    kerfd: Option<&Path>,
    media: &str,
    cookies: Option<&Path>,
) -> Result<PathBuf> {
    let path = if media.starts_with("http://") || media.starts_with("https://") {
        let result = kerf_fetcher::fetch_video(
            media,
            directory,
            &kerf_fetcher::FetchOptions {
                cookies: cookies.map(Path::to_path_buf),
                ..kerf_fetcher::FetchOptions::default()
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
    let router = open_router(directory, kerfd)?;
    let snapshot = router.runtime().get_state().map_err(runtime_error)?;
    let key = operation_key("cli-import");
    router
        .runtime()
        .import_media_request("agent", &key, &path.to_string_lossy(), snapshot.revision)
        .map_err(runtime_error)?;
    Ok(path)
}

fn print_snapshot(runtime: &RuntimeClient, label: &str) {
    match runtime.get_state() {
        Ok(snapshot) => println!(
            "{label}: revision {}, duration {:.1}s, {} media",
            snapshot.revision,
            snapshot.duration,
            snapshot.timeline["sources"].as_array().map_or(0, Vec::len)
        ),
        Err(error) => eprintln!("{label}: {}: {}", error.code, error.message),
    }
}

fn runtime_error(error: kerf_runtime::RuntimeError) -> anyhow::Error {
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
        AgentEvent::ToolCallStarted { name, targets, .. } => {
            eprintln!("\n▸ tool {name} {}", target_summary(&targets));
        }
        AgentEvent::ToolCallFinished {
            name, ok, summary, ..
        } => {
            if !ok {
                eprintln!(
                    "✗ tool {name} failed: {}",
                    summary.error.as_deref().unwrap_or("unknown tool error")
                );
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
    println!("kerf, session dir: {}", directory.display());
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
                let router = open_router(directory, None)?;
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

fn target_summary(targets: &AgentTargetSummary) -> String {
    let mut parts = Vec::new();
    add_target_summary(&mut parts, "source", &targets.sources);
    add_target_summary(&mut parts, "clip", &targets.clips);
    add_target_summary(&mut parts, "track", &targets.tracks);
    add_target_summary(&mut parts, "job", &targets.jobs);
    if parts.is_empty() {
        "(no targets)".into()
    } else {
        parts.join("; ")
    }
}

fn add_target_summary(parts: &mut Vec<String>, label: &str, values: &[String]) {
    if !values.is_empty() {
        parts.push(format!("{label}: {}", values.join(", ")));
    }
}

fn operation_key(kind: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{kind}-{}-{timestamp}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::{preset_request, uses_terminal, Args, Command, PresetAction};
    use clap::Parser;

    #[test]
    fn kerf_opens_the_native_editor_by_default() {
        let args = Args::try_parse_from(["kerf"]).expect("parse default launcher args");

        assert!(!uses_terminal(&args));
    }

    #[test]
    fn preset_commands_need_no_agent_and_keep_source_offsets_and_text_literal() {
        let args = Args::try_parse_from([
            "kerf",
            "--dir",
            "/tmp/project",
            "preset",
            "apply",
            "style.kerfpreset",
            "--bind",
            "slot1=m2@1.5s",
            "--text",
            "title1=New = title",
        ])
        .unwrap();
        assert!(uses_terminal(&args));
        let Some(Command::Preset { action }) = args.command else {
            panic!("preset command");
        };
        let (tool, request) = preset_request(&action).unwrap();
        assert_eq!(tool, "edit");
        assert_eq!(request["bindings"]["slot1"]["from"], "1.5s");
        assert_eq!(request["texts"]["title1"], "New = title");
        assert!(preset_request(&PresetAction::Apply {
            file: "style.kerfpreset".into(),
            bindings: vec!["slot1=m1".into(), "slot1=m2".into()],
            texts: vec![],
            sequence: None
        })
        .is_err());
        let media = Args::try_parse_from(["kerf", "video.mp4"]).unwrap();
        assert_eq!(media.media.as_deref(), Some("video.mp4"));
    }

    #[test]
    fn terminal_commands_keep_the_terminal_agent_path() {
        for argv in [
            vec!["kerf", "--terminal"],
            vec!["kerf", "-p", "trim the first clip"],
            vec!["kerf", "--skills"],
            vec!["kerf", "--mcp"],
            vec!["kerf", "--generate-docs"],
        ] {
            let args = Args::try_parse_from(argv).expect("parse terminal launcher args");
            assert!(uses_terminal(&args));
        }
    }
}
