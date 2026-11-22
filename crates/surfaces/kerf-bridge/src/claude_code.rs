//! Kerf chat on the person's own Claude Code.
//!
//! Each turn runs the installed, unmodified `claude` CLI headless with this
//! project's `kerf-mcp` as its only MCP server, passed on the command line:
//! nothing is written to Claude Code's own configuration, so the server is
//! there only for the turns Kerf starts. Claude Code signs in through its own
//! flow and Kerf never sees the account's credentials; the usage counts
//! against that account as any other Claude Code session does.
//! Where no `kerf-mcp` program is found the editor is the server instead,
//! run again with `--serve-mcp`.

use crate::collab::program_path;
use kerf_agent::{summarize_targets, summarize_tool_result, AgentEvent, ToolResultSummary};
use kerf_mcp::permissions::{Mode, MODE_VARIABLE};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// The name the server goes by in Claude Code; its tools are `mcp__kerf__*`.
const SERVER: &str = "kerf";
const TOOL_PREFIX: &str = "mcp__kerf__";
/// The most of Claude Code's error output kept for a failed turn.
const MAX_STDERR_BYTES: usize = 4 * 1024;
/// The context window assumed, in tokens, until a turn's result names the model's.
const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

/// What Claude Code is told about working in the editor, beyond the
/// instructions `kerf-mcp` gives every client.
const EDITOR_GUIDE: &str = "You are Kerf's chat, working in the person's open Kerf \
editor through the kerf tools. Edits you stage are proposals: the timeline shows them \
as they grow and the person accepts or rejects them in Kerf. Build a longer edit in \
steps: stage the first part with `edit`, check it, then extend the proposal. Keep \
replies short: say what you changed and where.";

pub(crate) struct ClaudeCodeSession {
    claude: PathBuf,
    directory: PathBuf,
    server: McpProgram,
    kerfd: PathBuf,
    /// What the next turn may do without asking; none is the person's file's say.
    permissions: Option<Mode>,
    model: Option<String>,
    /// Claude Code's own session, resumed for each follow-up turn.
    session: Option<String>,
    /// The model's context window in tokens, once a turn's result has said.
    context_window: u64,
    aborted: Arc<AtomicBool>,
}

impl ClaudeCodeSession {
    pub(crate) fn new(
        directory: &Path,
        kerfd: &Path,
        model: Option<String>,
    ) -> Result<Self, String> {
        let claude = find_claude().ok_or(
            "Claude Code isn't installed. Install it (https://code.claude.com), sign in by \
             running `claude` once in a terminal, then try again.",
        )?;
        let server = find_kerf_mcp(kerfd).ok_or(
            "Kerf's tools (kerf-mcp) weren't found. Build them (cargo build -p kerf-mcp) or \
             set KERF_MCP_BIN.",
        )?;
        Ok(Self {
            claude,
            directory: directory.to_path_buf(),
            server,
            kerfd: kerfd.to_path_buf(),
            permissions: None,
            model: model.filter(|model| !model.trim().is_empty() && model != "default"),
            session: None,
            context_window: DEFAULT_CONTEXT_WINDOW,
            aborted: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn abort_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.aborted)
    }

    /// Go on in Claude Code's session `id`, kept from an earlier launch.
    pub(crate) fn resume(&mut self, id: String) {
        self.session = Some(id).filter(|id| !id.trim().is_empty());
    }

    /// Claude Code's own session, once a turn has named it.
    pub(crate) fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// The mode the turns from here on run in: each starts its own
    /// `kerf-mcp`, which is told it as it starts.
    pub(crate) fn set_permissions(&mut self, mode: Option<Mode>) {
        self.permissions = mode;
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.claude);
        command
            .current_dir(&self.directory)
            .args(arguments(
                &mcp_config(&self.server, &self.directory, &self.kerfd, self.permissions),
                self.model.as_deref(),
                self.session.as_deref(),
            ))
            // The account Claude Code is signed in to pays for the turn, not
            // a key that happens to be in Kerf's environment.
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// One turn: `text` goes to Claude Code and its stream comes back as
    /// the built-in agent's events.
    pub(crate) fn run_with_events(
        &mut self,
        text: &str,
        mut emit: impl FnMut(AgentEvent),
    ) -> Result<(), String> {
        self.aborted.store(false, Ordering::SeqCst);
        let mut child = self
            .command()
            .spawn()
            .map_err(|error| format!("Could not start Claude Code: {error}"))?;
        // The request goes on stdin: list options would take it as a value.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|error| format!("Could not send the request to Claude Code: {error}"))?;
        }
        let stdout = child.stdout.take().ok_or("Claude Code has no output")?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or("Claude Code has no error output")?;
        let errors = thread::spawn(move || {
            let mut text = String::new();
            // What it said before a read failed is all there is to say.
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let watcher = {
            let child = Arc::clone(&child);
            let finished = Arc::clone(&finished);
            let aborted = Arc::clone(&self.aborted);
            thread::spawn(move || {
                while !finished.load(Ordering::SeqCst) {
                    if aborted.load(Ordering::SeqCst) {
                        if let Ok(mut child) = child.lock() {
                            // It may have ended by itself already.
                            let _ = child.kill();
                        }
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            })
        };
        let mut stream = Stream {
            context_window: self.context_window,
            ..Stream::default()
        };
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            stream.read(&message, &mut emit);
        }
        finished.store(true, Ordering::SeqCst);
        // The watcher only waits and kills; there is nothing it could fail with.
        let _ = watcher.join();
        let status = child
            .lock()
            .map_err(|_| "Claude Code's process was lost")?
            .wait()
            .map_err(|error| error.to_string())?;
        let errors = errors.join().unwrap_or_default();
        if let Some(session) = stream.session.take() {
            self.session = Some(session);
        }
        self.context_window = stream.context_window;
        if self.aborted.load(Ordering::SeqCst) {
            emit(AgentEvent::Aborted);
            return Ok(());
        }
        if let Some(failure) = stream.failure {
            return Err(failure);
        }
        match stream.done {
            Some(text) => {
                emit(AgentEvent::Done { text });
                Ok(())
            }
            None => Err(failed_turn(status.code(), &errors)),
        }
    }
}

/// Claude Code's command line for one turn.
fn arguments(mcp_config: &str, model: Option<&str>, session: Option<&str>) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--mcp-config",
        mcp_config,
        "--strict-mcp-config",
        // None of Claude Code's own tools: no shell, no file edits; the
        // project changes only through Kerf's proposals.
        "--tools",
        "",
        "--allowedTools",
        "mcp__kerf",
        "--append-system-prompt",
        EDITOR_GUIDE,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if let Some(model) = model {
        arguments.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(session) = session {
        arguments.extend(["--resume".to_owned(), session.to_owned()]);
    }
    arguments
}

fn mcp_config(
    server: &McpProgram,
    directory: &Path,
    kerfd: &Path,
    permissions: Option<Mode>,
) -> String {
    let mut args: Vec<String> = server.args.iter().map(|arg| (*arg).to_owned()).collect();
    args.extend(["--dir".to_owned(), directory.to_string_lossy().into_owned()]);
    let mut env = json!({"KERFD_BIN": kerfd.to_string_lossy()});
    // The mode chosen beside the composer, over the person's file's.
    if let Some(mode) = permissions {
        env[MODE_VARIABLE] = json!(mode.name());
    }
    json!({"mcpServers": {SERVER: {
        "type": "stdio",
        "command": server.program.to_string_lossy(),
        "args": args,
        "env": env,
    }}})
    .to_string()
}

/// What a turn's stream has said so far.
#[derive(Default)]
struct Stream {
    session: Option<String>,
    /// Tool names by call id, for the results that name only the id.
    tools: HashMap<String, String>,
    done: Option<String>,
    failure: Option<String>,
    /// The tokens the latest request read, and the window they are out of.
    context_used: u64,
    context_window: u64,
}

impl Stream {
    /// Says how full the context is, once there is something to say.
    fn say_context(&self, emit: &mut impl FnMut(AgentEvent)) {
        if self.context_used > 0 && self.context_window > 0 {
            emit(AgentEvent::Context {
                used: self.context_used,
                limit: self.context_window,
            });
        }
    }

    fn read(&mut self, message: &Value, emit: &mut impl FnMut(AgentEvent)) {
        if let Some(session) = message["session_id"].as_str() {
            self.session = Some(session.to_owned());
        }
        match message["type"].as_str() {
            Some("system") if message["subtype"] == "init" => {
                let kerf = message["mcp_servers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|server| server["name"] == SERVER);
                if let Some(server) = kerf
                    && server["status"] != "connected"
                {
                    self.failure = Some(
                        "Claude Code could not start Kerf's tools (kerf-mcp). Check that \
                             kerf-mcp and kerfd are built."
                            .to_owned(),
                    );
                }
            }
            Some("stream_event") => {
                let event = &message["event"];
                match event["type"].as_str() {
                    Some("message_start") => emit(AgentEvent::TurnStarted {
                        stop_reason: String::new(),
                    }),
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        if let Some(text) = event["delta"]["text"].as_str() {
                            emit(AgentEvent::TextDelta(text.to_owned()));
                        }
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                // What this request read: its new input and what it took from the cache.
                let usage = &message["message"]["usage"];
                let used = [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ]
                .iter()
                .filter_map(|key| usage[*key].as_u64())
                .sum::<u64>();
                if used > 0 && used != self.context_used {
                    self.context_used = used;
                    self.say_context(emit);
                }
                for block in blocks(message) {
                    if block["type"] != "tool_use" {
                        continue;
                    }
                    let id = block["id"].as_str().unwrap_or_default().to_owned();
                    let name = tool_name(block["name"].as_str().unwrap_or_default());
                    self.tools.insert(id.clone(), name.clone());
                    emit(AgentEvent::ToolCallStarted {
                        id,
                        name,
                        targets: summarize_targets(&block["input"]),
                    });
                }
            }
            Some("user") => {
                for block in blocks(message) {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let id = block["tool_use_id"].as_str().unwrap_or_default().to_owned();
                    let name = self.tools.get(&id).cloned().unwrap_or_default();
                    let ok = block["is_error"].as_bool() != Some(true);
                    emit(AgentEvent::ToolCallFinished {
                        id,
                        name,
                        ok,
                        summary: Box::new(result_summary(&block["content"], ok)),
                    });
                }
            }
            Some("result") => {
                // The window of the model that answered: the largest, when it used helpers.
                let window = message["modelUsage"]
                    .as_object()
                    .into_iter()
                    .flat_map(|models| models.values())
                    .filter_map(|model| model["contextWindow"].as_u64())
                    .max();
                if let Some(window) = window.filter(|window| *window != self.context_window) {
                    self.context_window = window;
                    self.say_context(emit);
                }
                let text = message["result"].as_str().unwrap_or_default().to_owned();
                if message["is_error"].as_bool() == Some(true) || message["subtype"] != "success" {
                    if self.failure.is_none() {
                        self.failure = Some(if text.is_empty() {
                            format!(
                                "Claude Code stopped: {}",
                                message["subtype"].as_str().unwrap_or("error")
                            )
                        } else {
                            text
                        });
                    }
                } else {
                    self.done = Some(text);
                }
            }
            _ => {}
        }
    }
}

fn blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
}

/// The tool as the built-in agent names it, without Claude Code's namespace.
fn tool_name(name: &str) -> String {
    name.strip_prefix(TOOL_PREFIX).unwrap_or(name).to_owned()
}

/// Kerf's tools answer in JSON text; read it for the proposal and status the
/// editor follows. Raw output never reaches an event.
fn result_summary(content: &Value, ok: bool) -> ToolResultSummary {
    let texts: Vec<&str> = match content {
        Value::String(text) => vec![text.as_str()],
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect(),
        _ => Vec::new(),
    };
    let values: Vec<Value> = texts
        .iter()
        .map(|text| {
            serde_json::from_str(text).unwrap_or_else(|_| {
                if ok {
                    Value::Null
                } else {
                    json!({"error": text})
                }
            })
        })
        .collect();
    summarize_tool_result(&Value::Array(values))
}

fn failed_turn(code: Option<i32>, errors: &str) -> String {
    let errors = errors.trim();
    let lower = errors.to_lowercase();
    if lower.contains("not logged in") || lower.contains("/login") || lower.contains("log in") {
        return "Claude Code isn't signed in. Run `claude` once in a terminal and sign in, then \
                try again."
            .to_owned();
    }
    let mut end = errors.len().min(MAX_STDERR_BYTES);
    while !errors.is_char_boundary(end) {
        end -= 1;
    }
    match (code, &errors[..end]) {
        (_, "") => format!(
            "Claude Code stopped without an answer (exit {}).",
            code.map_or_else(|| "signal".to_owned(), |code| code.to_string())
        ),
        (_, text) => format!("Claude Code stopped: {text}"),
    }
}

/// `KERF_CLAUDE_BIN`, then `PATH`, then where Claude Code's installers put
/// it: an app started from the Dock does not see the shell's `PATH`.
fn find_claude() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KERF_CLAUDE_BIN") {
        return Some(path.into());
    }
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .collect()
        })
        .unwrap_or_default();
    if let Some(home) = kerf_paths::home() {
        candidates.push(home.join(".local/bin").join(name));
        candidates.push(home.join(".claude/local").join(name));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin").join(name));
    candidates.push(PathBuf::from("/usr/local/bin").join(name));
    candidates.into_iter().find(|path| path.is_file())
}

/// The program that serves Kerf's tools over MCP, and what it is given before `--dir`.
struct McpProgram {
    program: PathBuf,
    args: &'static [&'static str],
}

/// The editor's argument that has it serve Kerf's tools as `kerf-mcp` does, without a window:
/// `kerf-editor --serve-mcp --dir <project>`, which [`serve_mcp`](crate::serve_mcp) answers.
pub const SERVE_MCP: &str = "--serve-mcp";

/// `KERF_MCP_BIN`, then `kerf-mcp` beside kerfd, then beside this program, then the editor
/// itself, which serves the same tools: a `kerf-mcp` that was never built, or that is not
/// where kerfd is, does not keep Claude Code from them.
fn find_kerf_mcp(kerfd: &Path) -> Option<McpProgram> {
    let program = std::env::current_exe().ok();
    mcp_program(
        std::env::var_os("KERF_MCP_BIN").map(PathBuf::from),
        program_path(kerfd, std::env::var_os("PATH").as_deref()).as_deref(),
        program.as_deref(),
    )
}

fn mcp_program(
    named: Option<PathBuf>,
    kerfd: Option<&Path>,
    program: Option<&Path>,
) -> Option<McpProgram> {
    if let Some(named) = named.filter(|path| !path.as_os_str().is_empty()) {
        return Some(McpProgram {
            program: named,
            args: &[],
        });
    }
    let name = if cfg!(windows) {
        "kerf-mcp.exe"
    } else {
        "kerf-mcp"
    };
    let beside = [kerfd, program]
        .into_iter()
        .flatten()
        .filter_map(|path| Some(path.parent()?.join(name)))
        .find(|path| path.is_file());
    if let Some(beside) = beside {
        return Some(McpProgram {
            program: beside,
            args: &[],
        });
    }
    // Only the editor reads the argument: a test's program, or another that loads this
    // library, is not one to run as a server.
    let editor =
        program.filter(|path| path.file_stem().is_some_and(|stem| stem == "kerf-editor"))?;
    Some(McpProgram {
        program: editor.to_owned(),
        args: &[SERVE_MCP],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_carries_only_kerfs_server_and_no_built_in_tools() {
        let arguments = arguments("{}", Some("opus"), Some("s1"));
        let after = |flag: &str| {
            let index = arguments.iter().position(|value| value == flag).unwrap();
            arguments[index + 1].clone()
        };
        assert!(arguments.contains(&"--strict-mcp-config".to_owned()));
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--allowedTools"), "mcp__kerf");
        assert_eq!(after("--model"), "opus");
        assert_eq!(after("--resume"), "s1");
        let fresh = super::arguments("{}", None, None);
        assert!(!fresh.contains(&"--resume".to_owned()) && !fresh.contains(&"--model".to_owned()));
    }

    #[test]
    fn a_chosen_mode_reaches_the_server_it_starts() {
        let config: Value = serde_json::from_str(&mcp_config(
            &McpProgram {
                program: "/bin/kerf-mcp".into(),
                args: &[],
            },
            Path::new("/work/film"),
            Path::new("/bin/kerfd"),
            Some(Mode::AcceptEdits),
        ))
        .unwrap();
        assert_eq!(
            config["mcpServers"]["kerf"]["env"][MODE_VARIABLE],
            "accept-edits"
        );
    }
}
