//! Native Kerf launcher and terminal entry point.
//!
//! `kerf` opens the native editor by default: on the project `--dir` names,
//! or, without one, on its project launcher. `--terminal` keeps the
//! project-scoped terminal agent available for scripts and interactive
//! shells. `kerf new` makes a project folder and `kerf sync` shares a
//! project's edits through Git.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTargetSummary, OpenAiApiMode, ProviderName,
    ReasoningEffort,
};
use kerf_collab::git::{self, Pushed};
use kerf_collab::scaffold::{self, Created, GitHubRepository, NewProject, Template};
use kerf_mcp::{docs::add_agents_pointer, inbound, load_project_skills, KerfMcpRouter};
use kerf_model::Catalog;
use kerf_runtime::RuntimeClient;
use kerf_skills::Skill;
use serde_json::{json, Value};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    /// Project directory owned by kerfd. Defaults to KERF_PROJECT_DIR, then
    /// the current directory; without either, the editor opens its project
    /// launcher.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,

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
    /// Make a project folder, optionally shared through Git or GitHub.
    New(NewArgs),
    /// Commit local edits, merge what collaborators pushed, and push.
    Sync {
        /// Commit message for local edits; described from the edits when absent.
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
}

#[derive(Debug, clap::Args)]
struct NewArgs {
    /// Project name; the folder is named after it.
    name: String,
    /// Folder to make the project folder in.
    #[arg(long, default_value = ".")]
    location: PathBuf,
    /// 1080×1920 (9:16) instead of 1920×1080 (16:9).
    #[arg(long, conflicts_with = "square")]
    vertical: bool,
    /// 1080×1080 (1:1) instead of 1920×1080 (16:9).
    #[arg(long)]
    square: bool,
    /// Frames per second: 23.976, 24, 25, 29.97, 30 (the default), or 60.
    #[arg(long)]
    rate: Option<String>,
    /// Make the project a Git repository with a first commit.
    #[arg(long)]
    git: bool,
    /// Publish the repository to this remote (implies --git).
    #[arg(long, conflicts_with = "github")]
    remote: Option<String>,
    /// Create a private GitHub repository with gh and publish to it (implies --git).
    #[arg(long)]
    github: bool,
    /// Make the GitHub repository public.
    #[arg(long, requires = "github")]
    public: bool,
}

impl NewArgs {
    /// The project these arguments describe, made at an absolute location.
    fn project(&self) -> Result<NewProject> {
        let template = if self.vertical {
            Template::Vertical
        } else if self.square {
            Template::Square
        } else {
            Template::Standard
        };
        Ok(NewProject {
            name: self.name.clone(),
            location: std::path::absolute(&self.location).context("resolve --location")?,
            template,
            rate: self.rate.clone(),
            git: self.git,
            remote: self.remote.clone(),
            github: self.github.then_some(GitHubRepository {
                name: None,
                private: !self.public,
            }),
        })
    }
}

#[derive(Debug, Subcommand)]
enum PresetAction {
    /// List presets in presets/, .kerf/presets, and the project folder.
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
        /// SLOT of music: move the markers over its clips to its beats.
        #[arg(long = "beats")]
        beats: Vec<String>,
        #[arg(long)]
        sequence: Option<String>,
    },
}

fn main() {
    kerf_engine::sidecar::forward_termination_signals();
    if let Err(error) = run() {
        eprintln!("kerf: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let requested = requested_project(&args, env::var_os("KERF_PROJECT_DIR"));
    let current = || PathBuf::from(".");

    match &args.command {
        Some(Command::New(new)) => return run_new(new),
        Some(Command::Sync { message }) => {
            let directory = requested.unwrap_or_else(current);
            let directory = directory
                .canonicalize()
                .with_context(|| format!("no project at {}", directory.display()))?;
            return run_sync(&directory, args.kerfd.as_deref(), message.as_deref());
        }
        Some(Command::Preset { action }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_preset(action, &directory, args.kerfd.as_deref());
        }
        None => {}
    }

    if !uses_terminal(&args) {
        let directory = editor_project(&args, requested)
            .map(|directory| resolve_directory(&directory))
            .transpose()?;
        return run_gui(&args, directory.as_deref());
    }

    let directory = resolve_directory(&requested.unwrap_or_else(current))?;
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

    // The agent holds kerfd for the whole session.
    let router = router.held();
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

    let result = match args.prompt.as_deref() {
        Some(prompt) => run_prompt(&mut session, prompt).map(|text| {
            if !text.is_empty() {
                println!("\n{text}");
            }
        }),
        None => run_repl(&mut session, &directory),
    };
    finish_renders(&router);
    result
}

/// Wait for the renders the agent started: they run on this process's
/// threads and die with it. Ctrl-C stops them with the process.
fn finish_renders(router: &KerfMcpRouter) {
    let running = router.running_renders();
    if running > 0 {
        eprintln!("waiting for {running} render job(s) to finish; Ctrl-C to stop");
    }
    router.finish_renders(false);
}

/// The project `--dir` names, else the one `KERF_PROJECT_DIR` (`environment`)
/// names.
fn requested_project(args: &Args, environment: Option<OsString>) -> Option<PathBuf> {
    args.dir.clone().or_else(|| {
        environment
            .filter(|directory| !directory.is_empty())
            .map(PathBuf::from)
    })
}

/// The project the editor opens: the requested one, else the current
/// directory when media is to be imported into it, else none, and the
/// editor shows its project launcher.
fn editor_project(args: &Args, requested: Option<PathBuf>) -> Option<PathBuf> {
    requested.or_else(|| args.media.is_some().then(|| PathBuf::from(".")))
}

/// `directory` as an absolute path, made first when it does not exist.
fn resolve_directory(directory: &Path) -> Result<PathBuf> {
    directory
        .canonicalize()
        .or_else(|_| {
            fs::create_dir_all(directory)?;
            directory.canonicalize()
        })
        .context("resolve project directory")
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
            beats,
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
            for slot in beats {
                let binding = media
                    .get_mut(slot)
                    .with_context(|| format!("--beats {slot}: bind `{slot}` with --bind"))?;
                binding["beats"] = json!(true);
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

fn run_new(new: &NewArgs) -> Result<()> {
    // A terminal can answer git's prompts (an ssh passphrase, say).
    env::set_var(git::INTERACTIVE_ENV, "1");
    let created = create_project(new)?;
    println!("created {}", created.path.display());
    if let Some(remote) = &created.remote {
        println!("published to {remote}");
    }
    for warning in &created.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn create_project(new: &NewArgs) -> Result<Created> {
    Ok(scaffold::create(&new.project()?, Catalog::builtin())?)
}

/// Commit local edits, merge what collaborators pushed, and push, as the
/// editor's Sync Now does: staging and the network run here, and kerfd,
/// which owns `project.kerf`, commits and merges.
fn run_sync(directory: &Path, kerfd: Option<&Path>, message: Option<&str>) -> Result<()> {
    env::set_var(git::INTERACTIVE_ENV, "1");
    if !git::is_repository(directory) {
        if let Some(top) = git::enclosing_repository(directory) {
            bail!(
                "{} is a folder inside the repository {}, not a shared project of its own; run `git init` in the project to share it",
                directory.display(),
                top.display()
            );
        }
        bail!(
            "{} is not a Git repository; make a shared project with `kerf new --git`, or publish this one from the editor (Collaborate → Publish to GitHub…)",
            directory.display()
        );
    }
    // Hashing new media is the slow part of a commit; staged first, kerfd's
    // commit is quick.
    git::stage(directory).context("stage")?;
    let fetched = git::fetch(directory).context("fetch")?;
    let kerfd = kerfd
        .map(Path::to_path_buf)
        .or_else(discover_kerfd)
        .unwrap_or_else(|| PathBuf::from(binary_name("kerfd")));
    // A merge runs inside kerfd; give it longer than an edit.
    let runtime = RuntimeClient::connect_or_spawn(directory, Some(&kerfd))
        .map_err(runtime_error)?
        .with_request_timeout(Duration::from_secs(60));
    let (_, report) = runtime
        .collab_integrate("ui", &operation_key("sync"), message)
        .map_err(|error| match error.code.as_str() {
            "PROJECTION_DIVERGED" => anyhow!(
                "project.kerf was changed outside Kerf; open the project in the editor and choose Load from Disk or Keep Editor Version, then sync"
            ),
            _ => runtime_error(error),
        })?;
    let pushed = git::push(directory).context("push")?;
    let downloaded = fetched && git::lfs_pull(directory).context("download Git LFS media")?;
    let upstream = git::upstream(directory);
    for line in sync_report(&report, &pushed, upstream.as_deref(), downloaded) {
        println!("{line}");
    }
    Ok(())
}

/// What a sync did, as lines for the terminal: kerfd's `report` (the commit,
/// incoming commits, conflicts the merge settled, renamed ids, and how the
/// project changed), then the push and the Git LFS download.
fn sync_report(
    report: &Value,
    pushed: &Pushed,
    upstream: Option<&str>,
    downloaded: bool,
) -> Vec<String> {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    if let Some(commit) = report["committed"].as_str() {
        lines.push(format!("committed {commit}"));
    }
    let incoming = list(&report["incoming"]);
    if !incoming.is_empty() {
        let how = if report["fastForward"] == true {
            " (fast-forward)"
        } else {
            ""
        };
        lines.push(format!(
            "merged {} incoming commit(s){how}:",
            incoming.len()
        ));
        for commit in &incoming {
            lines.push(format!(
                "  {} {} · {}",
                text(&commit["commit"]),
                text(&commit["author"]),
                text(&commit["subject"])
            ));
        }
    }
    for conflict in list(&report["conflicts"]) {
        let kept = if conflict["kept"] == "theirs" {
            "theirs"
        } else {
            "yours"
        };
        lines.push(format!(
            "conflict: {}: {} (kept {kept})",
            text(&conflict["path"]),
            text(&conflict["what"])
        ));
    }
    for pair in list(&report["renamed"]) {
        lines.push(format!(
            "renamed incoming {} to {}, as both sides added {}",
            text(&pair[0]),
            text(&pair[1]),
            text(&pair[0])
        ));
    }
    let summary = list(&report["summary"]);
    if !summary.is_empty() {
        lines.push("changed in the project:".into());
        lines.extend(summary.iter().map(|line| format!("  {}", text(line))));
    }
    if pushed.pushed {
        let to = upstream.unwrap_or("the remote");
        lines.push(if pushed.set_upstream {
            format!("published the branch to {to}")
        } else {
            format!("pushed to {to}")
        });
    }
    if downloaded {
        lines.push("downloaded Git LFS media".into());
    }
    if lines.is_empty() {
        lines.push("up to date".into());
    }
    lines
}

fn run_gui(args: &Args, directory: Option<&Path>) -> Result<()> {
    let llm = resolve_llm(args)?;
    configure_gui_agent(&llm);

    if let (Some(media), Some(directory)) = (args.media.as_deref(), directory) {
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
    let status = editor_command(&qt, directory, &kerfd)
        .status()
        .with_context(|| format!("could not launch {}", qt.display()))?;
    if !status.success() {
        bail!("Qt editor exited with {status}");
    }
    Ok(())
}

/// The editor's command line: the project, when there is one, as its only
/// argument, and kerfd through `KERFD_BIN`, so that without a project the
/// kerfd path is never read as one and the editor shows its launcher.
fn editor_command(qt: &Path, project: Option<&Path>, kerfd: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(qt);
    // The editor has no terminal for git to prompt in.
    command
        .env("KERFD_BIN", kerfd)
        .env_remove(git::INTERACTIVE_ENV);
    match project {
        Some(project) => {
            command.arg(project);
        }
        // An empty KERF_PROJECT_DIR names no project here; neither may it there.
        None => {
            command.env_remove("KERF_PROJECT_DIR");
        }
    }
    command
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
    let current = env::current_exe().ok();
    let home = env::var_os("HOME").map(PathBuf::from);
    editor_candidates(current.as_deref(), root, home.as_deref())
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow!("Qt editor not found; build it with scripts/run-kerf-qt.sh or set KERF_QT_BIN")
        })
}

/// Where the editor may be, in the order tried: beside `kerf`; on macOS the
/// installed app (the install's only editor there); and the build in the
/// checkout (on macOS its bundle, before a plain `target/qt/kerf-qt` an
/// older build may have left). A `kerf` run from the checkout itself tries
/// the checkout's build before the installed app.
fn editor_candidates(current: Option<&Path>, root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut beside = Vec::new();
    if let Some(parent) = current.and_then(Path::parent) {
        beside.push(parent.join(binary_name("kerf-qt")));
    }
    let mut installed = Vec::new();
    if cfg!(target_os = "macos") {
        const APP: &str = "Industrial Kerf.app/Contents/MacOS/kerf-qt";
        if let Some(home) = home {
            installed.push(home.join("Applications").join(APP));
        }
        installed.push(Path::new("/Applications").join(APP));
    }
    let mut built = Vec::new();
    if cfg!(target_os = "macos") {
        built.push(root.join("target/qt/kerf-qt.app/Contents/MacOS/kerf-qt"));
    }
    built.push(root.join("target/qt").join(binary_name("kerf-qt")));
    built.push(root.join("target/debug").join(binary_name("kerf-qt")));
    built.push(root.join("target/release").join(binary_name("kerf-qt")));
    let from_checkout = current.is_some_and(|path| path.starts_with(root));
    if from_checkout {
        beside.into_iter().chain(built).chain(installed).collect()
    } else {
        beside.into_iter().chain(installed).chain(built).collect()
    }
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
    let options = kerf_fetcher::FetchOptions {
        cookies: cookies.map(Path::to_path_buf),
        ..kerf_fetcher::FetchOptions::default()
    };
    let path = media_file(directory, media, &options)?;
    let router = open_router(directory, kerfd)?;
    let snapshot = router.runtime().get_state().map_err(runtime_error)?;
    let key = operation_key("cli-import");
    router
        .runtime()
        .import_media_request("agent", &key, &path.to_string_lossy(), snapshot.revision)
        .map_err(runtime_error)?;
    Ok(path)
}

/// The file `kerf <media>` imports into the project in `directory`: a
/// download of a URL, or a local file. A project that keeps its media in
/// `media/` gets a copy there (named by its content) of a download and of a
/// file its repository would not carry, as the editor's import does.
fn media_file(
    directory: &Path,
    media: &str,
    options: &kerf_fetcher::FetchOptions,
) -> Result<PathBuf> {
    if media.starts_with("http://") || media.starts_with("https://") {
        if !kerf_collab::media::shares_media(directory) {
            return Ok(kerf_fetcher::fetch_video(media, directory, options)?.path);
        }
        // Downloaded where Git does not look, so a sync meanwhile never
        // commits half of it, then copied in.
        let folder = kerf_collab::media::incoming_path(directory, "-download")
            .context("make a download folder")?;
        let copied = kerf_fetcher::fetch_video(media, &folder, options)
            .map_err(anyhow::Error::from)
            .and_then(|fetched| {
                kerf_collab::media::copy_into_media(directory, &fetched.path)
                    .with_context(|| format!("copy {} into media/", fetched.path.display()))
            });
        let _ = fs::remove_dir_all(&folder);
        return Ok(directory.join(copied?));
    }
    let path = kerf_collab::media::absolute(directory, Path::new(media));
    if !path.is_file() {
        bail!("media file does not exist: {}", path.display());
    }
    if kerf_collab::media::needs_copy(directory, &path) {
        let copy = kerf_collab::media::copy_into_media(directory, &path)
            .with_context(|| format!("copy {} into media/", path.display()))?;
        return Ok(directory.join(copy));
    }
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
    use super::{
        binary_name, create_project, editor_candidates, editor_command, editor_project, media_file,
        preset_request, requested_project, run_sync, sync_report, uses_terminal, Args, Command,
        PresetAction,
    };
    use clap::Parser;
    use kerf_collab::git::Pushed;
    use kerf_collab::scaffold::Template;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Once;

    #[test]
    fn kerf_finds_the_installed_app_or_the_checkout_build_first() {
        let root = Path::new("/src/kerf");
        let home = Path::new("/Users/me");
        let app = home.join("Applications/Industrial Kerf.app/Contents/MacOS/kerf-qt");
        let bundle = root.join("target/qt/kerf-qt.app/Contents/MacOS/kerf-qt");
        let position = |candidates: &[PathBuf], path: &Path| {
            candidates.iter().position(|candidate| candidate == path)
        };
        // An installed kerf: beside it first, then the app, then the checkout.
        let installed = editor_candidates(
            Some(Path::new("/Users/me/.local/bin/kerf")),
            root,
            Some(home),
        );
        assert_eq!(
            installed[0],
            Path::new("/Users/me/.local/bin").join(binary_name("kerf-qt"))
        );
        let plain = root.join("target/qt").join(binary_name("kerf-qt"));
        if cfg!(target_os = "macos") {
            assert!(position(&installed, &app) < position(&installed, &bundle));
            assert!(position(&installed, &bundle) < position(&installed, &plain));
            // The checkout's own kerf: its build before the app.
            let checkout =
                editor_candidates(Some(&root.join("target/release/kerf")), root, Some(home));
            assert!(position(&checkout, &bundle) < position(&checkout, &app));
        } else {
            assert_eq!(position(&installed, &app), None);
            assert!(position(&installed, &plain).is_some());
        }
    }

    #[test]
    fn kerf_opens_the_native_editor_by_default() {
        let args = Args::try_parse_from(["kerf"]).expect("parse default launcher args");

        assert!(!uses_terminal(&args));
    }

    #[test]
    fn kerf_without_a_project_opens_the_launcher_and_dir_still_opens_one() {
        let launcher = Args::try_parse_from(["kerf"]).unwrap();
        assert_eq!(launcher.dir, None);
        assert_eq!(
            editor_project(&launcher, requested_project(&launcher, None)),
            None
        );
        let empty = Some(OsString::new());
        assert_eq!(
            editor_project(&launcher, requested_project(&launcher, empty)),
            None
        );
        let from_environment = requested_project(&launcher, Some("/films/a".into()));
        assert_eq!(
            editor_project(&launcher, from_environment),
            Some(PathBuf::from("/films/a"))
        );

        let project = Args::try_parse_from(["kerf", "--dir", "./my-project"]).unwrap();
        assert!(!uses_terminal(&project));
        let requested = requested_project(&project, Some("/films/a".into()));
        assert_eq!(requested, Some(PathBuf::from("./my-project")), "--dir wins");
        assert_eq!(
            editor_project(&project, requested),
            Some(PathBuf::from("./my-project"))
        );

        // Media is imported into a project: the current directory by default.
        let media = Args::try_parse_from(["kerf", "video.mp4"]).unwrap();
        assert_eq!(
            editor_project(&media, requested_project(&media, None)),
            Some(PathBuf::from("."))
        );
    }

    #[test]
    fn the_editor_gets_kerfd_from_the_environment_and_a_project_only_when_there_is_one() {
        let kerfd = Path::new("/opt/kerf/kerfd");
        let launcher = editor_command(Path::new("kerf-qt"), None, kerfd);
        assert_eq!(launcher.get_args().count(), 0);
        let environment: Vec<_> = launcher.get_envs().collect();
        assert!(environment.contains(&("KERFD_BIN".as_ref(), Some(kerfd.as_os_str()))));
        assert!(environment.contains(&("KERF_PROJECT_DIR".as_ref(), None)));

        let project = editor_command(Path::new("kerf-qt"), Some(Path::new("/films/a")), kerfd);
        let arguments: Vec<_> = project.get_args().collect();
        assert_eq!(arguments, ["/films/a"]);
        assert!(project
            .get_envs()
            .any(|pair| pair == ("KERFD_BIN".as_ref(), Some(kerfd.as_os_str()))));
    }

    #[test]
    fn project_commands_parse_in_their_old_and_new_forms() {
        // `--dir` is global: before or after the subcommand.
        for argv in [
            vec!["kerf", "--dir", "p", "preset", "list"],
            vec!["kerf", "preset", "list", "--dir", "p"],
        ] {
            let args = Args::try_parse_from(argv).unwrap();
            assert!(matches!(
                args.command,
                Some(Command::Preset {
                    action: PresetAction::List
                })
            ));
            assert_eq!(args.dir, Some(PathBuf::from("p")));
        }
        let terminal = Args::try_parse_from(["kerf", "--terminal", "video.mp4"]).unwrap();
        assert!(uses_terminal(&terminal));
        assert_eq!(terminal.dir, None);
        assert_eq!(terminal.media.as_deref(), Some("video.mp4"));

        let sync =
            Args::try_parse_from(["kerf", "sync", "--dir", "p", "-m", "Trim the intro"]).unwrap();
        assert!(uses_terminal(&sync));
        let Some(Command::Sync { message }) = sync.command else {
            panic!("sync command");
        };
        assert_eq!(message.as_deref(), Some("Trim the intro"));
        let Some(Command::Sync { message }) =
            Args::try_parse_from(["kerf", "sync"]).unwrap().command
        else {
            panic!("sync command");
        };
        assert_eq!(message, None);
    }

    #[test]
    fn new_takes_a_shape_a_rate_and_where_to_share_it() {
        let args = Args::try_parse_from([
            "kerf",
            "new",
            "Short Film",
            "--location",
            "/films",
            "--vertical",
            "--rate",
            "23.976",
            "--github",
            "--public",
        ])
        .unwrap();
        let Some(Command::New(new)) = args.command else {
            panic!("new command");
        };
        let project = new.project().unwrap();
        assert_eq!(project.name, "Short Film");
        assert_eq!(project.location, PathBuf::from("/films"));
        assert_eq!(project.template, Template::Vertical);
        assert_eq!(project.rate.as_deref(), Some("23.976"));
        let github = project.github.expect("a GitHub repository");
        assert!(!github.private);
        assert_eq!(github.name, None);

        let Some(Command::New(new)) = Args::try_parse_from(["kerf", "new", "Plain"])
            .unwrap()
            .command
        else {
            panic!("new command");
        };
        let project = new.project().unwrap();
        assert!(project.location.is_absolute());
        assert_eq!(project.template, Template::Standard);
        assert!(!project.git && project.remote.is_none() && project.github.is_none());

        let Some(Command::New(new)) =
            Args::try_parse_from(["kerf", "new", "Square", "--square", "--github"])
                .unwrap()
                .command
        else {
            panic!("new command");
        };
        let project = new.project().unwrap();
        assert_eq!(project.template, Template::Square);
        assert!(project.github.unwrap().private, "private unless --public");

        for argv in [
            vec!["kerf", "new"],
            vec!["kerf", "new", "A", "--vertical", "--square"],
            vec![
                "kerf",
                "new",
                "A",
                "--github",
                "--remote",
                "git@example.com:a.git",
            ],
            vec!["kerf", "new", "A", "--public"],
        ] {
            assert!(Args::try_parse_from(&argv).is_err(), "{argv:?}");
        }
    }

    /// Keep the person's git configuration (signing, hooks, templates) out
    /// of the tests.
    fn hermetic() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let empty = std::env::temp_dir().join("kerf-cli-empty-gitconfig");
            fs::write(&empty, "").unwrap();
            std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            std::env::set_var("GIT_AUTHOR_NAME", "Test");
            std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
            std::env::set_var("GIT_COMMITTER_NAME", "Test");
            std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
        });
    }

    fn git(args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[test]
    fn new_makes_a_shared_project_and_publishes_it_to_its_remote() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        let remote_text = remote.to_str().unwrap();
        git(&["init", "-q", "--bare", "-b", "main", remote_text]);
        let location = root.path().join("projects");
        let args = Args::try_parse_from([
            "kerf",
            "new",
            "Short Film",
            "--location",
            location.to_str().unwrap(),
            "--vertical",
            "--rate",
            "24",
            "--remote",
            remote_text,
        ])
        .unwrap();
        let Some(Command::New(new)) = args.command else {
            panic!("new command");
        };
        let created = create_project(&new).unwrap();

        assert_eq!(created.path, location.join("Short Film"));
        assert_eq!(created.remote.as_deref(), Some(remote_text));
        let document = fs::read_to_string(created.path.join("project.kerf")).unwrap();
        assert!(
            document.contains("project name=\"Short Film\""),
            "{document}"
        );
        assert!(document.contains("rate=24 size=1080x1920"), "{document}");
        for folder in ["media", "presets"] {
            assert!(created.path.join(folder).is_dir(), "{folder}/");
        }
        let attributes = fs::read_to_string(created.path.join(".gitattributes")).unwrap();
        assert!(attributes.contains("*.kerf merge=kerf"), "{attributes}");
        assert!(created.path.join(".gitignore").is_file());

        let project = created.path.to_str().unwrap();
        assert_eq!(
            git(&["-C", project, "log", "-1", "--format=%s"]),
            "Create Short Film"
        );
        assert_eq!(
            git(&["-C", project, "rev-parse", "HEAD"]),
            git(&["--git-dir", remote_text, "rev-parse", "main"]),
            "the first commit is on the remote"
        );
        assert_eq!(
            git(&["-C", project, "rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/main"
        );
        assert!(
            created
                .warnings
                .iter()
                .all(|warning| !warning.contains("not pushed")),
            "{:?}",
            created.warnings
        );
    }

    #[test]
    fn new_without_git_makes_only_the_folder_and_never_overwrites_one() {
        let root = tempfile::tempdir().unwrap();
        let location = root.path().to_str().unwrap();
        let args = Args::try_parse_from(["kerf", "new", "Plain", "--location", location]).unwrap();
        let Some(Command::New(new)) = args.command else {
            panic!("new command");
        };
        let created = create_project(&new).unwrap();
        assert!(created.path.join("project.kerf").is_file());
        assert!(!created.path.join(".git").exists());
        assert_eq!(created.remote, None);

        let again = create_project(&new).unwrap_err();
        assert!(format!("{again:#}").contains("already exists"), "{again:#}");
    }

    #[test]
    fn imported_files_go_into_media_in_a_project_that_keeps_it() {
        let root = tempfile::tempdir().unwrap();
        let downloads = root.path().join("Downloads");
        fs::create_dir_all(&downloads).unwrap();
        let clip = downloads.join("a.mov");
        fs::write(&clip, b"picture").unwrap();
        let options = kerf_fetcher::FetchOptions::default();
        let clip_text = clip.to_str().unwrap();

        // A plain folder imports the file where it is.
        let plain = root.path().join("Plain");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(media_file(&plain, clip_text, &options).unwrap(), clip);

        // A shared one copies it into media/, named by its content.
        let shared = root.path().join("Shared");
        fs::create_dir_all(shared.join("media")).unwrap();
        let copy = media_file(&shared, clip_text, &options).unwrap();
        let name = copy.strip_prefix(shared.join("media")).unwrap();
        let name = name.to_str().unwrap();
        assert!(name.starts_with("a-") && name.ends_with(".mov"), "{name}");
        assert_eq!(fs::read(&copy).unwrap(), b"picture");
        // A file already in media/ is imported as it is.
        assert_eq!(
            media_file(&shared, &format!("media/{name}"), &options).unwrap(),
            copy
        );
        assert!(media_file(&shared, "missing.mov", &options).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn downloads_into_a_shared_project_land_in_media() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        // A yt-dlp that writes a video where it runs.
        let yt_dlp = root.path().join("yt-dlp");
        fs::write(
            &yt_dlp,
            "#!/bin/sh\n[ \"$1\" = --version ] && { echo 2025.01.01; exit 0; }\n\
             printf video > Clip.mp4\necho '[download] Destination: Clip.mp4'\n",
        )
        .unwrap();
        fs::set_permissions(&yt_dlp, fs::Permissions::from_mode(0o755)).unwrap();
        let options = kerf_fetcher::FetchOptions {
            yt_dlp_binary: Some(yt_dlp),
            ..kerf_fetcher::FetchOptions::default()
        };
        let url = "https://example.com/watch?v=1";

        let shared = root.path().join("Shared");
        fs::create_dir_all(shared.join("media")).unwrap();
        let copy = media_file(&shared, url, &options).unwrap();
        assert_eq!(copy.parent(), Some(shared.join("media").as_path()));
        let name = copy.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("Clip-") && name.ends_with(".mp4"),
            "{name}"
        );
        assert_eq!(fs::read(&copy).unwrap(), b"video");
        // Nothing is left beside project.kerf, where `/*.mp4` is ignored,
        // or in the download folder.
        assert!(!shared.join("Clip.mp4").exists());
        let incoming = shared.join(kerf_collab::media::INCOMING_DIR);
        assert_eq!(fs::read_dir(incoming).unwrap().count(), 0);

        let plain = root.path().join("Plain");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(
            media_file(&plain, url, &options).unwrap(),
            plain.join("Clip.mp4")
        );
    }

    #[test]
    fn sync_refuses_a_folder_that_is_not_a_repository() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let error = run_sync(root.path(), Some(Path::new("/nonexistent/kerfd")), None).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a Git repository"),
            "{error:#}"
        );
    }

    #[test]
    fn a_sync_report_reads_as_what_happened() {
        let report = json!({
            "committed": "1a2b3c4",
            "incoming": [{"commit": "9f8e7d6", "author": "jun", "subject": "Trimmed c3"}],
            "fastForward": false,
            "conflicts": [
                {"path": "s1/V1/c3", "what": "duration changed on both sides", "kept": "ours"},
                {"path": "s1/V1/c9", "what": "overlapped c5 on V1; moved to V2", "kept": "theirs"}
            ],
            "renamed": [["c7", "c9"]],
            "summary": ["Changed clip c3: duration", "Added clip c9 on V2"],
        });
        let pushed = Pushed {
            pushed: true,
            set_upstream: false,
        };
        assert_eq!(
            sync_report(&report, &pushed, Some("origin/main"), true),
            [
                "committed 1a2b3c4",
                "merged 1 incoming commit(s):",
                "  9f8e7d6 jun · Trimmed c3",
                "conflict: s1/V1/c3: duration changed on both sides (kept yours)",
                "conflict: s1/V1/c9: overlapped c5 on V1; moved to V2 (kept theirs)",
                "renamed incoming c7 to c9, as both sides added c7",
                "changed in the project:",
                "  Changed clip c3: duration",
                "  Added clip c9 on V2",
                "pushed to origin/main",
                "downloaded Git LFS media",
            ]
        );
        let idle = json!({"committed": null, "incoming": [], "fastForward": false});
        assert_eq!(
            sync_report(&idle, &Pushed::default(), None, false),
            ["up to date"]
        );
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
            beats: vec![],
            sequence: None
        })
        .is_err());
        let music = Args::try_parse_from([
            "kerf",
            "preset",
            "apply",
            "style.kerfpreset",
            "--bind",
            "slot2=song",
            "--beats",
            "slot2",
        ])
        .unwrap();
        let Some(Command::Preset { action }) = music.command else {
            panic!("preset command");
        };
        let (_, request) = preset_request(&action).unwrap();
        assert_eq!(request["bindings"]["slot2"]["beats"], true);
        assert!(preset_request(&PresetAction::Apply {
            file: "style.kerfpreset".into(),
            bindings: vec![],
            texts: vec![],
            beats: vec!["slot2".into()],
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
