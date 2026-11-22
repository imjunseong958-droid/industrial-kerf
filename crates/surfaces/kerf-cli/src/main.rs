//! Native Kerf launcher and terminal entry point.
//!
//! `kerf` opens the native editor by default: on the project `--dir` names,
//! or, without one, on its project launcher. `--terminal` keeps the
//! project-scoped terminal agent available for scripts and interactive
//! shells. `kerf new` makes a project folder and `kerf sync` shares a
//! project's edits through Git. `kerf load` takes a `project.kerf` changed
//! outside Kerf as the project, and `kerf proposals`, `kerf accept` and
//! `kerf reject` answer what an agent proposed, so a project is worked on
//! from a terminal with no editor open. `kerf export` writes a sequence as a
//! timeline file another editor opens, and says what the file leaves out.

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
    /// Make a take of a media by running the steps under its line in the document.
    Make {
        /// The media's id, as its line in the document names it.
        media: String,
        /// Run the steps without asking first.
        #[arg(long)]
        yes: bool,
        /// Make this many takes, one after another, each with its seeds one on from the
        /// last: several of the same shot to choose among.
        #[arg(long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
        rolls: u32,
    },
    /// List the takes a media's steps have made, or have the media be another of them.
    Takes {
        /// The media's id, as its line in the document names it.
        media: String,
        /// The take the media is to be, by its number.
        #[arg(long = "use", value_name = "TAKE")]
        choose: Option<u32>,
    },
    /// List every media its steps make: its take, its cast, and whether it wants making.
    Shots,
    /// Carry a media's recipe file into the document: its steps are written under the
    /// media's line, where an edit changes them, and its takes stay as they are.
    Carry {
        /// The media to carry, by their ids.
        media: Vec<String>,
        /// Every media that names a recipe file.
        #[arg(long)]
        all: bool,
        /// Carry them without asking first.
        #[arg(long)]
        yes: bool,
    },
    /// Take `project.kerf` as it is on disk, changed by hand or by another program, as the project.
    Load {
        /// Load it without asking first.
        #[arg(long)]
        yes: bool,
    },
    /// List the proposals waiting for an answer, or show what one would change.
    Proposals {
        /// A waiting proposal's id.
        proposal: Option<String>,
    },
    /// Accept a proposal: the one waiting, or the one named. The project becomes what it proposed.
    Accept {
        /// The proposal's id, where more than one waits.
        proposal: Option<String>,
        /// Accept it without asking first.
        #[arg(long)]
        yes: bool,
    },
    /// Reject a proposal: the one waiting, or the one named. The project stays as it is.
    Reject {
        /// The proposal's id, where more than one waits.
        proposal: Option<String>,
    },
    /// Write a sequence as a timeline file another editor opens, and list what the file
    /// does not carry: .otio for DaVinci Resolve and Premiere Pro, .fcpxml for Final Cut
    /// Pro and DaVinci Resolve. No Python needed.
    Export {
        /// The file to write. Its extension says the format: .otio or .fcpxml.
        file: PathBuf,
        /// otio or fcpxml, for a file whose extension does not say.
        #[arg(long)]
        format: Option<String>,
        /// The sequence to write, by its id; the first when absent.
        #[arg(long)]
        sequence: Option<String>,
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
    /// Inspect the pattern, minimum source lengths, text slots, and the media it makes from steps.
    Show { file: String },
    /// Stage a proposal replacing a sequence; accept it with `kerf accept` or in the editor.
    Apply {
        file: String,
        /// SLOT=MEDIA or SLOT=MEDIA@FROM (e.g. slot1=m2@1.5s). A media the preset makes takes none.
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
    kerf_runtime::crash::install("kerf");
    kerf_engine::process::sidecar::forward_termination_signals();
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
        Some(Command::Make { media, yes, rolls }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_make(&directory, args.kerfd.as_deref(), media, *yes, *rolls);
        }
        Some(Command::Takes { media, choose }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_takes(&directory, args.kerfd.as_deref(), media, *choose);
        }
        Some(Command::Shots) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_shots(&directory, args.kerfd.as_deref());
        }
        Some(Command::Carry { media, all, yes }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_carry(&directory, args.kerfd.as_deref(), media, *all, *yes);
        }
        Some(Command::Load { yes }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_load(&directory, args.kerfd.as_deref(), *yes);
        }
        Some(Command::Proposals { proposal }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_proposals(&directory, args.kerfd.as_deref(), proposal.as_deref());
        }
        Some(Command::Accept { proposal, yes }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            let answer = if *yes {
                Answer::Accept
            } else {
                Answer::AcceptAsked
            };
            return run_answer(
                &directory,
                args.kerfd.as_deref(),
                proposal.as_deref(),
                answer,
            );
        }
        Some(Command::Reject { proposal }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_answer(
                &directory,
                args.kerfd.as_deref(),
                proposal.as_deref(),
                Answer::Reject,
            );
        }
        Some(Command::Preset { action }) => {
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_preset(action, &directory, args.kerfd.as_deref());
        }
        Some(Command::Export {
            file,
            format,
            sequence,
        }) => {
            // Refused before kerfd is started for it.
            let format = export_format(file, format.as_deref())?;
            let directory = resolve_directory(&requested.unwrap_or_else(current))?;
            return run_export(
                &directory,
                args.kerfd.as_deref(),
                file,
                format,
                sequence.as_deref(),
            );
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
    // A making the agent started goes on until it is made; Ctrl-C stops it, and its model's
    // program takes back what it asked a server for.
    let making = router.running_makings();
    if making > 0 {
        eprintln!("waiting for {making} making(s) to finish; Ctrl-C to stop");
        let stop = kerf_engine::CancellationToken::new();
        let _held = kerf_engine::process::sidecar::stop_on_termination(&stop);
        while router.running_makings() > 0 && !stop.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        router.finish_makings(stop.is_cancelled());
    }
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
    git::set_env(git::INTERACTIVE_ENV, "1");
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
    git::set_env(git::INTERACTIVE_ENV, "1");
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
                "project.kerf was changed outside Kerf; `kerf load` takes it as the project, or open the project in the editor and choose Load from Disk or Keep Editor Version, then sync"
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

/// Whether the person at the terminal says yes to `question`; no where
/// there is no terminal to ask.
fn confirmed(question: &str) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return false;
    }
    print!("{question} [y/N] ");
    // Should the question not show, the answer is still read.
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

/// The format `kerf export` writes `file` as: the one `--format` names, else the one its
/// extension does.
fn export_format(file: &Path, format: Option<&str>) -> Result<kerf_interchange::Format> {
    use kerf_interchange::Format;
    match format {
        Some(name) => Format::parse(name)
            .ok_or_else(|| anyhow!("`{name}` is not a format kerf export writes: otio or fcpxml")),
        None => Format::of_path(file).ok_or_else(|| {
            anyhow!(
                "{} does not say its format: name it .otio or .fcpxml, or pass --format",
                file.display()
            )
        }),
    }
}

/// Write a sequence of the project as a timeline file another editor opens, then say what
/// the file does not carry. The project is not changed.
fn run_export(
    directory: &Path,
    kerfd: Option<&Path>,
    file: &Path,
    format: kerf_interchange::Format,
    sequence: Option<&str>,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let mut project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    // Another editor finds a file by where it is, not by where the project is.
    kerf_runtime::resolve_media_paths(&mut project, runtime.directory());
    let output =
        std::path::absolute(file).with_context(|| format!("resolve {}", file.display()))?;
    let written = kerf_interchange::export(&project, sequence, &output, format)
        .map_err(|error| anyhow!(error))?;
    print!("{}", export_report(&written));
    Ok(())
}

/// What `kerf export` says of the file it wrote: where it is, then everything it does not
/// carry and everything that arrives changed, a line each.
fn export_report(written: &kerf_interchange::ExportResult) -> String {
    let report = &written.report;
    let mut text = format!(
        "wrote {} ({})\n",
        written.output_path.display(),
        written.adapter
    );
    let mut list = |heading: &str, issues: &[kerf_interchange::InterchangeIssue]| {
        if issues.is_empty() {
            return;
        }
        text.push_str(&format!("{heading} ({}):\n", issues.len()));
        for issue in issues {
            let at = issue.path.as_deref().unwrap_or("-");
            text.push_str(&format!("  {at}\t{}\n", issue.message));
        }
    };
    list("not carried", &report.losses);
    list("check", &report.warnings);
    if report.issues.is_empty() {
        text.push_str("everything in the sequence is in the file\n");
    }
    text
}

/// A connection to the project's kerfd, started if it is not running.
fn runtime_for(directory: &Path, kerfd: Option<&Path>) -> Result<RuntimeClient> {
    let kerfd = kerfd
        .map(Path::to_path_buf)
        .or_else(discover_kerfd)
        .unwrap_or_else(|| PathBuf::from(binary_name("kerfd")));
    Ok(RuntimeClient::connect_or_spawn(directory, Some(&kerfd))
        .map_err(runtime_error)?
        .with_request_timeout(Duration::from_secs(60)))
}

/// Refuse to go on while `project.kerf` is not what Kerf has: Kerf reads its own state, not
/// the file, and writes nothing to the document until one of the two is chosen. Said before
/// anything runs, so a making is not paid for and then lost.
fn in_step(state: &kerf_runtime::StateSnapshot) -> Result<()> {
    if state.projection_status == "diverged" {
        bail!(
            "project.kerf was changed outside Kerf, so the project Kerf has is not the one the file says; `kerf load` takes the file as the project (the editor's Load from Disk), and Keep Editor Version there puts back what Kerf has"
        );
    }
    Ok(())
}

fn run_make(directory: &Path, kerfd: Option<&Path>, id: &str, yes: bool, rolls: u32) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let models = kerf_engine::recipe::model::Models::load().map_err(|error| anyhow!(error))?;
    // One making is the recipe as it is written. Several are rolls of it, each with its seeds
    // one on from the last, from the first roll no take of this recipe has had.
    let first = if rolls > 1 {
        kerf_engine::recipe::next_roll(&project, directory, id).map_err(|error| anyhow!(error))?
    } else {
        0
    };
    let planned = |roll: u32| {
        kerf_engine::recipe::plan_rolled(&project, directory, id, &models, first + roll)
            .map_err(|error| anyhow!(error))
    };
    let plan = planned(0)?;
    println!("steps   under `{id}` in the document");
    for (number, step) in plan.steps.iter().enumerate() {
        println!("step {}  {step}", number + 1);
    }
    if rolls > 1 {
        println!(
            "makes   {} (take {}), and {} more after it, takes {} to {}: the same steps, each seed one on from the last",
            plan.file.display(),
            plan.take,
            rolls - 1,
            plan.take + 1,
            plan.take + rolls - 1
        );
    } else {
        println!("makes   {} (take {})", plan.file.display(), plan.take);
    }
    // What its model has said it does not take: worth mending before it is paid for.
    for note in &plan.notes {
        println!("note    {note}");
    }
    // A recipe is commands someone wrote, perhaps someone else.
    if !yes && !confirmed(if rolls > 1 { "Run them?" } else { "Run it?" }) {
        bail!("not run; `kerf make {id} --yes` runs it without asking");
    }
    // Ctrl-C stops it as the editor's Stop Making does: its step is asked to end, a model's
    // program takes back what it asked a server for and says so, and nothing of the take is
    // left. A second Ctrl-C ends it at once.
    let cancellation = kerf_engine::CancellationToken::new();
    let stop = kerf_engine::process::sidecar::stop_on_termination(&cancellation);
    let mut made = Vec::new();
    let mut ended = None;
    for roll in 0..rolls {
        let plan = match roll {
            0 => Ok(plan.clone()),
            roll => planned(roll),
        };
        let take = plan.and_then(|plan| {
            if rolls > 1 {
                eprintln!("take {} ({} of {rolls})", plan.take, roll + 1);
            }
            kerf_engine::recipe::make(&project, directory, &plan, &models, &cancellation, |line| {
                eprintln!("{line}")
            })
            .map_err(|error| match error {
                kerf_engine::EngineError::Cancelled => anyhow!("stopped"),
                error => anyhow!(error),
            })
        });
        match take {
            Ok(take) => made.push(take),
            Err(error) => {
                ended = Some(error);
                break;
            }
        }
    }
    drop(stop);
    let Some(last) = made.last() else {
        let error =
            ended.expect("clap takes --rolls from 1, so only a take that failed leaves none");
        if error.to_string() == "stopped" {
            bail!("stopped: no take was made, and {id} is as it was");
        }
        return Err(error);
    };
    // The media becomes the new take, the last where there are several: the document takes its
    // path, length and streams, and clips keep their timing.
    let state = runtime
        .replace_media_request(
            "ui",
            &operation_key("make"),
            id,
            &last.stored,
            state.revision,
        )
        .map_err(runtime_error)?;
    if let [only] = made.as_slice() {
        println!(
            "made take {} of {id} (revision {})",
            only.take, state.revision
        );
    } else {
        println!(
            "made takes {} of {id}, which is take {} (revision {}); `kerf takes {id} --use <take>` has it be another",
            made.iter()
                .map(|take| take.take.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            last.take,
            state.revision
        );
    }
    match ended {
        Some(error) => Err(anyhow!(
            "{} of the {rolls} takes were made, and the next was not: {error}",
            made.len()
        )),
        None => Ok(()),
    }
}

fn run_takes(directory: &Path, kerfd: Option<&Path>, id: &str, choose: Option<u32>) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let media = project
        .media(id)
        .ok_or_else(|| anyhow!("no media `{id}` in this project"))?;
    if let Some(number) = choose {
        let take = kerf_engine::recipe::take(&project, directory, media, number)
            .map_err(|error| anyhow!(error))?;
        if take.chosen {
            println!("{id} is take {number} already");
            return Ok(());
        }
        let state = runtime
            .replace_media_request(
                "ui",
                &operation_key("take"),
                id,
                &take.stored,
                state.revision,
            )
            .map_err(runtime_error)?;
        println!("{id} is take {number} (revision {})", state.revision);
        return Ok(());
    }
    let takes =
        kerf_engine::recipe::takes(&project, directory, media).map_err(|error| anyhow!(error))?;
    if takes.is_empty() {
        println!("{id} has no takes yet; `kerf make {id}` makes one");
    }
    for take in takes {
        println!("{}", take_line(&take));
    }
    Ok(())
}

/// Every shot its steps make, a line each, and what they come to. Nothing is run.
fn run_shots(directory: &Path, kerfd: Option<&Path>) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let project = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let shots = kerf_engine::recipe::shots(&project, directory);
    if shots.is_empty() {
        println!(
            "no media has steps that make it; `step <media> add …` in an edit writes them under its line"
        );
        return Ok(());
    }
    for shot in &shots {
        println!("{}", shot.line());
    }
    println!("{}", kerf_engine::recipe::shots::summary(&shots));
    Ok(())
}

/// Carry the recipes of `ids`, or with `all` of every media that names one, into the
/// document: what would be written is shown first, and a person says yes. It is one edit,
/// which undo takes back, and the takes each recipe made are then held to the steps it
/// became, so that none is asked for again for being carried.
fn run_carry(
    directory: &Path,
    kerfd: Option<&Path>,
    ids: &[String],
    all: bool,
    yes: bool,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    in_step(&state)?;
    let before = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    let named: Vec<String> = if all {
        before
            .media
            .iter()
            .filter(|media| media.recipe.is_some())
            .map(|media| media.id.to_string())
            .collect()
    } else {
        ids.to_vec()
    };
    if named.is_empty() {
        if all {
            println!("no media names a recipe file: there is nothing to carry");
            return Ok(());
        }
        bail!("name the media to carry, or pass --all for every media that names a recipe file");
    }
    let mut script = Vec::new();
    let mut carried = Vec::new();
    for id in &named {
        match kerf_engine::recipe::carry(&before, directory, id) {
            Ok(carry) => {
                println!("{id}, from {}:", carry.recipe);
                for line in &carry.script {
                    println!("  {line}");
                }
                for note in &carry.notes {
                    println!("  note  {note}");
                }
                script.extend(carry.script);
                carried.push((id.clone(), carry.recipe));
            }
            // With --all, one that cannot be carried is said and left as it is.
            Err(error) if all => println!("{id} is left as it is: {error}"),
            Err(error) => return Err(anyhow!(error)),
        }
    }
    if carried.is_empty() {
        return Ok(());
    }
    if !yes && !confirmed("Write these steps into the document?") {
        bail!("not carried: say yes at the terminal, or pass --yes");
    }
    let state = runtime
        .apply_script_request(
            "ui",
            &operation_key("carry"),
            &script.join("\n"),
            state.revision,
        )
        .map_err(runtime_error)?;
    let after = kerf_runtime::parse_document(&state.document).map_err(runtime_error)?;
    for (id, recipe) in &carried {
        let held = kerf_engine::recipe::takes::mark_carried(&before, &after, directory, id)
            .map_err(|error| anyhow!(error))?;
        println!(
            "carried {id} (revision {}): {held} of its takes held to its steps; {recipe} is left where it is, and nothing names it now",
            state.revision
        );
    }
    Ok(())
}

/// Take `project.kerf` as it is on disk as the project, as the editor's Load from Disk does:
/// what would change is shown first, and a person says yes. It becomes one revision that undo
/// does not go back past, and the proposals waiting, made for the project before it, are
/// dropped.
fn run_load(directory: &Path, kerfd: Option<&Path>, yes: bool) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let file = directory.join("project.kerf");
    let text =
        fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
    // Read here first, so what is wrong with it is said with its lines and nothing is asked.
    let project = kerf_runtime::parse_document(&text)
        .map_err(|error| anyhow!("{} does not read: {}", file.display(), error.message))?;
    let printed = kerf_lang::print_document(
        &project,
        Catalog::builtin(),
        kerf_lang::PrintOptions::default(),
    );
    let changes = kerf_lang::diff_text(&state.document, &printed);
    if changes.is_empty() && state.projection_status == "current" {
        println!(
            "project.kerf is the project as Kerf has it (revision {}); nothing to load",
            state.revision
        );
        return Ok(());
    }
    if changes.is_empty() {
        println!(
            "project.kerf says what Kerf has, in other words; loading it writes it as Kerf does"
        );
    } else {
        print!("{changes}");
        let waiting = waiting(&state).len();
        println!(
            "loading it makes this the project, as one revision that undo does not go back past{}",
            match waiting {
                0 => String::new(),
                1 => ", and drops the proposal that waits".to_owned(),
                several => format!(", and drops the {several} proposals that wait"),
            }
        );
    }
    if !yes && !confirmed("Load it?") {
        bail!("not loaded; `kerf load --yes` loads it without asking");
    }
    let loaded = runtime
        .import_projection_request("ui", &operation_key("load"), state.revision)
        .map_err(runtime_error)?;
    println!("loaded project.kerf (revision {})", loaded.revision);
    Ok(())
}

/// The proposals of `state` that wait for an answer, oldest first.
fn waiting(state: &kerf_runtime::StateSnapshot) -> Vec<&kerf_runtime::ProposalEntry> {
    let mut waiting: Vec<_> = state
        .proposals
        .iter()
        .filter(|proposal| proposal.status == "pending")
        .collect();
    // A snapshot lists the newest first.
    waiting.reverse();
    waiting
}

/// A waiting proposal as a line of `kerf proposals`: its id and version, who made it and
/// when, and whether the project has changed since it was made.
fn proposal_line(proposal: &kerf_runtime::ProposalEntry, revision: u64) -> String {
    let mut line = format!(
        "{}  v{}  by {}",
        proposal.proposal_id, proposal.version, proposal.author.name
    );
    if let Some(staged) = &proposal.staged {
        line.push_str(&format!("  {staged}"));
    }
    if proposal.base_revision != revision {
        line.push_str(&format!(
            "  (made for revision {}; the project is at {revision})",
            proposal.base_revision
        ));
    }
    line
}

/// What a proposal would change, as a person reads it before answering: its line, the
/// script it runs, and the document's lines before and after.
fn proposal_shown(proposal: &kerf_runtime::ProposalEntry, revision: u64) -> String {
    let script = if proposal.source.is_empty() {
        &proposal.script
    } else {
        &proposal.source
    };
    format!(
        "{}\n\n{}\n\n{}",
        proposal_line(proposal, revision),
        script.trim_end(),
        proposal.document_diff.trim_end()
    )
}

/// The proposal `id` names among those `waiting`, or the one that waits when none is named.
/// An id is long, so enough of its start or its end to be one proposal's alone names it
/// too. What is said otherwise ends with how to `verb` one.
fn chosen<'a>(
    waiting: &[&'a kerf_runtime::ProposalEntry],
    id: Option<&str>,
    revision: u64,
    verb: &str,
) -> Result<&'a kerf_runtime::ProposalEntry> {
    let listed = || {
        waiting
            .iter()
            .map(|proposal| proposal_line(proposal, revision))
            .collect::<Vec<_>>()
            .join("\n")
    };
    match (id, waiting) {
        (_, []) => bail!("no proposal is waiting"),
        (None, [only]) => Ok(only),
        (None, _) => bail!(
            "{} proposals wait; name one, as `kerf {verb} {}`:\n{}",
            waiting.len(),
            waiting[0].proposal_id,
            listed()
        ),
        (Some(id), _) => {
            let exact = waiting
                .iter()
                .copied()
                .find(|proposal| proposal.proposal_id == id);
            let part: Vec<_> = waiting
                .iter()
                .copied()
                .filter(|proposal| {
                    proposal.proposal_id.starts_with(id) || proposal.proposal_id.ends_with(id)
                })
                .collect();
            match (exact, part.as_slice()) {
                (Some(proposal), _) | (None, &[proposal]) => Ok(proposal),
                (None, []) => bail!("no proposal `{id}` is waiting; these are:\n{}", listed()),
                (None, _) => bail!(
                    "`{id}` is part of {} proposals' ids; these wait:\n{}",
                    part.len(),
                    listed()
                ),
            }
        }
    }
}

fn run_proposals(directory: &Path, kerfd: Option<&Path>, id: Option<&str>) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let waiting = waiting(&state);
    if let Some(id) = id {
        let proposal = chosen(&waiting, Some(id), state.revision, "proposals")?;
        println!("{}", proposal_shown(proposal, state.revision));
        return Ok(());
    }
    if waiting.is_empty() {
        println!("no proposal is waiting");
    }
    for proposal in waiting {
        println!("{}", proposal_line(proposal, state.revision));
    }
    Ok(())
}

/// What a person says to a proposal from a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    /// Accept it, once the person has seen it and said yes.
    AcceptAsked,
    /// Accept it without asking.
    Accept,
    Reject,
}

/// Answer a waiting proposal as the editor's Accept and Reject do. An accept shows what the
/// proposal would change and asks first.
fn run_answer(
    directory: &Path,
    kerfd: Option<&Path>,
    id: Option<&str>,
    answer: Answer,
) -> Result<()> {
    let runtime = runtime_for(directory, kerfd)?;
    let state = runtime.get_state().map_err(runtime_error)?;
    let verb = if answer == Answer::Reject {
        "reject"
    } else {
        "accept"
    };
    in_step(&state)?;
    let waiting = waiting(&state);
    let proposal = chosen(&waiting, id, state.revision, verb)?;
    let id = proposal.proposal_id.clone();
    if answer == Answer::Reject {
        runtime
            .reject_proposal_request(&id, "ui", &operation_key("reject"), state.revision)
            .map_err(runtime_error)?;
        println!(
            "rejected {id}; the project is as it was (revision {})",
            state.revision
        );
        return Ok(());
    }
    println!("{}", proposal_shown(proposal, state.revision));
    if answer == Answer::AcceptAsked && !confirmed("Accept it?") {
        bail!("not accepted; `kerf accept {id} --yes` accepts it without asking");
    }
    let accepted = runtime
        .accept_proposal(&id, "ui", &operation_key("accept"), state.revision)
        .map_err(runtime_error)?;
    println!("accepted {id} (revision {})", accepted.revision);
    Ok(())
}

/// A take as a line of `kerf takes`: its number, a mark on the one the media is, its file,
/// and what has changed since it was made.
fn take_line(take: &kerf_engine::recipe::Take) -> String {
    let mut line = format!(
        "{} take {}  {}",
        if take.chosen { "*" } else { " " },
        take.number,
        take.stored
    );
    if take.rolled > 0 {
        line.push_str(&format!("  roll {}", take.rolled));
    }
    if !take.changed.is_empty() {
        line.push_str(&format!("  ({} changed since)", take.changed.join(", ")));
    }
    line
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
    let agent = gui_agent_env(&resolve_llm(args)?);

    if let (Some(media), Some(directory)) = (args.media.as_deref(), directory) {
        let path = import_media(
            directory,
            args.kerfd.as_deref(),
            media,
            args.cookies.as_deref(),
        )?;
        println!("imported {}", path.display());
    }

    let editor = discover_editor()?;
    let kerfd = args
        .kerfd
        .clone()
        .or_else(discover_kerfd)
        .unwrap_or_else(|| PathBuf::from(binary_name("kerfd")));
    let status = editor_command(&editor, directory, &kerfd)
        .envs(agent)
        .status()
        .with_context(|| format!("could not launch {}", editor.display()))?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

/// The editor's command line: the project, when there is one, as its only
/// argument, and kerfd through `KERFD_BIN`, so that without a project the
/// kerfd path is never read as one and the editor shows its launcher.
fn editor_command(editor: &Path, project: Option<&Path>, kerfd: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(editor);
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

/// What the editor's own agent reads from its environment, as `llm` has it:
/// the editor is started with these set.
fn gui_agent_env(llm: &LlmConfig) -> Vec<(&'static str, String)> {
    let mut env = vec![("KERF_PROVIDER", llm.provider.name().to_owned())];
    if let Some(api_key) = llm.api_key.as_deref() {
        let key_name = match llm.provider {
            ProviderName::Anthropic => "ANTHROPIC_API_KEY",
            ProviderName::Openai => "OPENAI_API_KEY",
        };
        env.push((key_name, api_key.to_owned()));
    }
    if let Some(model) = llm.model.as_deref() {
        env.push(("KERF_MODEL", model.to_owned()));
    }
    if let Some(base_url) = llm.base_url.as_deref() {
        env.push(("OPENAI_BASE_URL", base_url.to_owned()));
    }
    env.push(("OPENAI_API_MODE", llm.api_mode.name().to_owned()));
    if let Some(reasoning_effort) = llm.reasoning_effort {
        env.push((
            "OPENAI_REASONING_EFFORT",
            reasoning_effort.name().to_owned(),
        ));
    }
    env
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

fn discover_editor() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KERF_EDITOR_BIN").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        bail!("configured editor does not exist: {}", path.display());
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
            anyhow!(
                "editor not found; build it with `cargo build --manifest-path apps/kerf-editor/Cargo.toml` or set KERF_EDITOR_BIN"
            )
        })
}

/// Where the editor may be, in the order tried: beside `kerf`; on macOS the
/// installed app (the install's only editor there); and the build in the
/// checkout (`target/debug`, then `target/release`). A `kerf` run from the
/// checkout itself tries the checkout's build before the installed app.
fn editor_candidates(current: Option<&Path>, root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut beside = Vec::new();
    if let Some(parent) = current.and_then(Path::parent) {
        beside.push(parent.join(binary_name("kerf-editor")));
    }
    let mut installed = Vec::new();
    if cfg!(target_os = "macos") {
        const APP: &str = "Industrial Kerf.app/Contents/MacOS/kerf-editor";
        if let Some(home) = home {
            installed.push(home.join("Applications").join(APP));
        }
        installed.push(Path::new("/Applications").join(APP));
    }
    let built = [
        root.join("target/debug").join(binary_name("kerf-editor")),
        root.join("target/release").join(binary_name("kerf-editor")),
    ];
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
        // What stays in the incoming folder is swept as stale later.
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
            // Left in the buffer, it shows with the next line.
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
        AgentEvent::TurnStarted { .. } | AgentEvent::Done { .. } | AgentEvent::Context { .. } => {}
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
                // Its answer was printed as it streamed in.
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
    use super::{create_project, git, in_step, run_sync, Args, Command};
    use clap::Parser;

    use serde_json::json;

    use std::fs;
    use std::path::Path;
    use std::sync::Once;

    /// A snapshot at `revision` whose proposals are `proposals`, newest first as kerfd lists
    /// them: each an id, its status, and the revision it was made for.
    fn snapshot(
        revision: u64,
        status: &str,
        proposals: &[(&str, &str, u64)],
    ) -> kerf_runtime::StateSnapshot {
        serde_json::from_value(json!({
            "document": "kerf 1\n",
            "timeline": {},
            "revision": revision,
            "canUndo": false,
            "canRedo": false,
            "duration": 0.0,
            "pendingProposal": null,
            "projectionStatus": status,
            "proposals": proposals.iter().map(|(id, status, base)| json!({
                "proposalId": id,
                "status": status,
                "freshness": "current",
                "baseRevision": base,
                "script": "set c1 opacity=0.5\n",
                "source": "",
                "documentDiff": "--- before\n+++ after\n+      set opacity=0.5\n",
                "author": {"kind": "agent", "name": "Claude"},
                "version": 2,
                "staged": "2026-11-11T15:12:56Z",
            })).collect::<Vec<_>>(),
        }))
        .expect("a snapshot as kerfd sends one")
    }

    #[test]
    fn nothing_is_made_or_answered_while_the_file_is_not_what_kerf_has() {
        assert!(in_step(&snapshot(1, "current", &[])).is_ok());
        let refused = in_step(&snapshot(1, "diverged", &[]))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("`kerf load` takes the file as the project"),
            "{refused}"
        );
    }

    /// Keep the person's git configuration (signing, hooks, templates) out
    /// of the tests.
    fn hermetic() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let empty = std::env::temp_dir().join("kerf-cli-empty-gitconfig");
            fs::write(&empty, "").unwrap();
            git::set_env("GIT_CONFIG_GLOBAL", &empty);
            git::set_env("GIT_CONFIG_NOSYSTEM", "1");
            git::set_env("GIT_AUTHOR_NAME", "Test");
            git::set_env("GIT_AUTHOR_EMAIL", "test@example.com");
            git::set_env("GIT_COMMITTER_NAME", "Test");
            git::set_env("GIT_COMMITTER_EMAIL", "test@example.com");
        });
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
    fn sync_refuses_a_folder_that_is_not_a_repository() {
        hermetic();
        let root = tempfile::tempdir().unwrap();
        let error = run_sync(root.path(), Some(Path::new("/nonexistent/kerfd")), None).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a Git repository"),
            "{error:#}"
        );
    }
}
