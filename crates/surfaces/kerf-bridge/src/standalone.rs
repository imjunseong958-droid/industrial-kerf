//! What the editor calls without a bridge, all synchronous: about and updates, the FFmpeg
//! kerfd runs, Kerf's tools served over MCP, crash reports, the system keychain, the
//! project launcher, the command registry and the catalog.

use crate::event::{ViewFxDef, ViewParamDef};
use crate::tasks::{PresetCheck, ProjectInfo};
use crate::{secrets, tasks, updates, SERVE_MCP};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---- about ---------------------------------------------------------------------

/// Where Kerf's source and releases are.
pub const REPOSITORY: &str = "https://github.com/imjunseong958-droid/industrial-kerf";

/// `{name, version, license, repository}`: what Help > About Kerf shows.
pub fn about() -> Value {
    json!({
        "name": "Industrial Kerf",
        "version": env!("CARGO_PKG_VERSION"),
        "license": env!("CARGO_PKG_LICENSE"),
        "repository": REPOSITORY,
    })
}

/// Whether a newer Kerf is released on GitHub: `{current, latest?, newer, url?}` or `{error}`
/// (`KERF_UPDATE_CHECK=off` asks nothing). It asks the network and may block for up to
/// 10 s, so call it off the UI thread.
pub fn update_check() -> Value {
    updates::check()
}

// ---- FFmpeg -----------------------------------------------------------------------

/// The FFmpeg Kerf runs, as kerfd finds it (`kerf_engine::process::ffmpeg::resolve_ffmpeg_bin`): the
/// first that runs of `KERF_FFMPEG`, the one beside the program (where each package puts it,
/// the macOS app's Contents/MacOS among them), Homebrew's ffmpeg-full, then `ffmpeg` on the
/// PATH; `ffmpeg` when none runs, to say so as it fails. Found once, by running each with
/// `-version`, so call it off the UI thread the first time.
pub fn ffmpeg_bin() -> &'static str {
    kerf_engine::process::ffmpeg::resolve_ffmpeg_bin()
}

// ---- Kerf's tools, served by the editor ------------------------------------------

/// Serve Kerf's tools over MCP on stdin and stdout, as `kerf-mcp` does, when `args` (the
/// program's, without its name) begin with [`SERVE_MCP`]: `kerf-editor --serve-mcp --dir
/// <project>`. Claude Code is given the editor to run this way where no `kerf-mcp` is found.
/// Returns the status to exit with, or `None` when the arguments ask for no such thing and
/// the editor should start.
pub fn serve_mcp(args: &[String]) -> Option<i32> {
    let directory = served_directory(args)?;
    kerf_engine::process::sidecar::forward_termination_signals();
    let kerfd = std::env::var_os("KERFD_BIN").map(PathBuf::from);
    Some(
        match directory.and_then(|directory| kerf_mcp::serve_stdio(&directory, kerfd.as_deref())) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("kerf-editor {SERVE_MCP}: {error}");
                1
            }
        },
    )
}

/// The project `--serve-mcp --dir <project>` names; `None` when the arguments do not ask the
/// editor to serve.
fn served_directory(arguments: &[String]) -> Option<Result<PathBuf, String>> {
    let (first, rest) = arguments.split_first()?;
    if first != SERVE_MCP {
        return None;
    }
    Some(match rest {
        [option, directory] if option == "--dir" => Ok(directory.into()),
        _ => Err("expected --dir <project-directory>".into()),
    })
}

// ---- crash reports ---------------------------------------------------------------

/// Report every panic in the editor's process to the crash folder, as kerfd and the CLI do,
/// and every crash of its native code too: a bad memory access, or an abort (signals on
/// macOS and Linux; an unhandled-exception filter and `SIGABRT` on Windows). Call once,
/// before any other thread starts.
pub fn crash_install() {
    kerf_runtime::crash::install("editor");
}

/// The crash folder's reports: `{folder, reports: [{path, component (editor|kerfd|kerf),
/// atMs, message}]}`, newest first; `folder` is null when there is nowhere to keep them.
pub fn crash_reports() -> Value {
    crash_reports_json(kerf_runtime::crash::folder())
}

fn crash_reports_json(folder: Option<PathBuf>) -> Value {
    let reports: Vec<Value> = folder
        .as_deref()
        .map(kerf_runtime::crash::reports)
        .unwrap_or_default()
        .into_iter()
        .map(|report| {
            json!({
                "path": report.path,
                "component": report.component,
                // Milliseconds fit an f64 exactly until the year 287,396.
                "atMs": report.at_ms as f64,
                "message": report.message,
            })
        })
        .collect();
    json!({"folder": folder, "reports": reports})
}

/// The system keychain: `{op: get|set|delete, name, value?}` -> `{ok: true, value?}` (`value`
/// null when `get` finds none) or `{error}` when it cannot be used (`KERF_SECRETS=off` turns
/// it off).
pub fn secret(request: &Value) -> Value {
    secrets::request(request)
}

/// Keep this process out of the system keychain from here on, as `KERF_SECRETS=off` does, without
/// changing its environment: [`secret`] then refuses everything, and the agent and Kerf Cloud keep
/// their keys and token as they do where there is no keychain.
pub fn keychain_off() {
    secrets::turn_off();
}

// ---- project launcher ------------------------------------------------------------

/// What the launcher shows of the project folder at `path`, before any bridge exists;
/// synchronous and local.
pub fn project_inspect(path: &Path) -> ProjectInfo {
    tasks::inspect_project(path)
}

/// The preset file at `path`, its slots and texts, and how each of `files` would fill them;
/// or why it cannot be read. It probes the files with ffprobe, so it may take a moment.
pub fn preset_inspect(path: &Path, files: &[PathBuf]) -> Result<PresetCheck, String> {
    tasks::inspect_preset(path, files)
}

// ---- command registry ----------------------------------------------------------

fn command_context(value: &Value) -> kerf_commands::Context {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// The command registry (kerf-commands), with shortcuts in the keymap `keymap` names (such
/// as `premiere`; an unknown one is the default): `{keymap, keymaps, menus, commands}`.
/// Synchronous and pure, as the other registry functions are, so it can be called before a
/// bridge exists.
pub fn commands_catalog(keymap: &str) -> Value {
    kerf_commands::catalog(keymap)
}

/// `{commandId: reason or null}`: why each command is unavailable in the editor's JSON
/// `context`, or null when it is available.
pub fn commands_states(context: &Value) -> Value {
    kerf_commands::states(&command_context(context))
}

/// Resolve `{command, params, context}` to `{request}`, the command for
/// [`Bridge::send`](crate::Bridge::send) that carries it out, or `{error}`.
pub fn commands_run(invocation: &Value) -> Value {
    let id = invocation["command"].as_str().unwrap_or_default();
    let context = command_context(&invocation["context"]);
    match kerf_commands::run(id, &context, &invocation["params"]) {
        Ok(request) => json!({"request": request}),
        Err(error) => json!({"error": error}),
    }
}

/// Everything the catalog offers: picture effects and transitions, generators (`kind`
/// `generator`, such as titles and solids) and sound effects (`domain` `audio`).
pub fn fx_catalog_all() -> Vec<ViewFxDef> {
    kerf_runtime::fx_catalog_all()
}

/// The properties a clip has by its track's type, which [`fx_catalog_all`] does not list:
/// `video` and `audio`, as a timeline's `properties` carries them.
pub fn clip_properties() -> BTreeMap<String, Vec<ViewParamDef>> {
    kerf_runtime::clip_properties()
}
