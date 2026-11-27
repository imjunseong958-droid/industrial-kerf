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

mod agent;
mod editor;
mod files;
mod llm;
mod making;
mod media;
mod proposals;
mod sharing;

use agent::{finish_renders, print_skills, run_prompt, run_repl};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use editor::{editor_project, run_gui};
use files::{export_format, run_export, run_preset};
use kerf_agent::{AgentSession, AgentSessionOptions};
use kerf_collab::scaffold::{GitHubRepository, NewProject, Template};
use kerf_mcp::{docs::add_agents_pointer, inbound, load_project_skills, KerfMcpRouter};
use kerf_runtime::{ProjectionStatus, RuntimeClient};
use llm::resolve_llm;
use making::{run_carry, run_make, run_shots, run_takes};
use media::import_media;
use proposals::{run_answer, run_load, run_proposals, Answer};
use sharing::{run_new, run_sync};
use std::env;
use std::ffi::OsString;
use std::fs;
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
    /// Local media file to import before opening the editor/session.
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
    kerf_runtime::logging::to_stderr();
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
            let directory = kerf_paths::canonicalize(&directory)
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
        let path = import_media(&directory, args.kerfd.as_deref(), media)?;
        println!("imported {}", path.display());
    }

    let router = open_router(&directory, args.kerfd.as_deref())?;
    let llm = resolve_llm(&args)?;
    if llm.api_key.is_none() {
        // The media is in the project: with no instruction to carry out, that is done.
        if args.media.is_some() && args.prompt.is_none() {
            eprintln!(
                "no agent session started: no API key configured; set ANTHROPIC_API_KEY or OPENAI_API_KEY, or pass --key"
            );
            return Ok(());
        }
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

/// The project `--dir` names, else the one `KERF_PROJECT_DIR` (`environment`)
/// names.
fn requested_project(args: &Args, environment: Option<OsString>) -> Option<PathBuf> {
    args.dir.clone().or_else(|| {
        environment
            .filter(|directory| !directory.is_empty())
            .map(PathBuf::from)
    })
}

/// `directory` as an absolute path, made first when it does not exist.
fn resolve_directory(directory: &Path) -> Result<PathBuf> {
    kerf_paths::canonicalize(directory)
        .or_else(|_| {
            fs::create_dir_all(directory)?;
            kerf_paths::canonicalize(directory)
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
    if state.projection_status == ProjectionStatus::Diverged {
        bail!(
            "project.kerf was changed outside Kerf, so the project Kerf has is not the one the file says; `kerf load` takes the file as the project (the editor's Load from Disk), and Keep Editor Version there puts back what Kerf has"
        );
    }
    Ok(())
}

fn open_router(directory: &Path, kerfd: Option<&Path>) -> Result<KerfMcpRouter> {
    let kerfd = kerfd.map(Path::to_path_buf).or_else(discover_kerfd);
    KerfMcpRouter::new(directory, kerfd.as_deref()).map_err(|error| anyhow!(error))
}

/// The checkout this `kerf` was built in, whose `target` holds the kerfd and the editor
/// built with it: the folder that holds the workspace, three above this crate
/// (`crates/surfaces/kerf-cli`).
fn checkout() -> Option<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3)
}

fn discover_kerfd() -> Option<PathBuf> {
    if let Some(path) = env::var_os("KERFD_BIN") {
        return Some(path.into());
    }
    let current = env::current_exe().ok();
    let root = checkout()?;
    let mut candidates = Vec::new();
    if let Some(parent) = current.as_deref().and_then(Path::parent) {
        candidates.push(parent.join(binary_name("kerfd")));
    }
    candidates.push(root.join("target/debug").join(binary_name("kerfd")));
    candidates.push(root.join("target/release").join(binary_name("kerfd")));
    candidates.into_iter().find(|path| path.is_file())
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
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

fn operation_key(kind: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("cli-{kind}-{}-{timestamp}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::{checkout, in_step};
    use serde_json::json;

    /// `kerf` looks in the checkout it was built in for the kerfd and the editor built
    /// with it (`target/debug`, `target/release`): that is the folder that holds the
    /// workspace, not one of the folders the crates are sorted into.
    #[test]
    fn the_checkout_is_the_folder_that_holds_the_workspace() {
        let checkout = checkout().expect("this crate is three folders into its checkout");
        assert!(
            checkout.join("rust-toolchain.toml").is_file(),
            "{}",
            checkout.display()
        );
        assert!(checkout
            .join("crates/surfaces/kerf-cli/Cargo.toml")
            .is_file());
    }

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
}
