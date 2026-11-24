//! `kerf` with nothing that asks for the terminal: the native editor, found and started on a
//! project or on its launcher.

use crate::llm::{resolve_llm, LlmConfig};
use crate::media::import_media;
use crate::{binary_name, checkout, discover_kerfd, Args};
use anyhow::{anyhow, bail, Context, Result};
use kerf_agent::ProviderName;
use kerf_collab::git;
use std::env;
use std::path::{Path, PathBuf};

/// The project the editor opens: the requested one, else the current
/// directory when media is to be imported into it, else none, and the
/// editor shows its project launcher.
pub(crate) fn editor_project(args: &Args, requested: Option<PathBuf>) -> Option<PathBuf> {
    requested.or_else(|| args.media.is_some().then(|| PathBuf::from(".")))
}

pub(crate) fn run_gui(args: &Args, directory: Option<&Path>) -> Result<()> {
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

fn discover_editor() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KERF_EDITOR_BIN").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        bail!("configured editor does not exist: {}", path.display());
    }
    let root = checkout().context("could not resolve repository root")?;
    let current = env::current_exe().ok();
    let home = kerf_paths::home();
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
