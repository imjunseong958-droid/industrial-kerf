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
use crate::event::as_said;
use kerf_agent::{summarize_targets, summarize_tool_result, AgentEvent, ToolResultSummary};
use kerf_engine::process::sidecar::{Sidecar, Stopper, GRACE};
use kerf_mcp::permissions::{Mode, MODE_VARIABLE};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

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
    abort: Abort,
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
            abort: Abort::default(),
        })
    }

    pub(crate) fn abort_handle(&self) -> Abort {
        self.abort.clone()
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
    ///
    /// Claude Code runs as a sidecar: on Unix it leads a process group of its
    /// own, and on Windows it is in a job of its own, with the `kerf-mcp` it
    /// starts and whatever that starts. Stopping the turn stops all of them,
    /// and what is left when Claude Code ends ends with it. On Unix Claude
    /// Code starts the shells its tools run in sessions of their own, out of
    /// its group's reach: it ends them itself when it is asked to end, which
    /// is why it is asked before it is killed.
    pub(crate) fn run_with_events(
        &mut self,
        text: &str,
        mut emit: impl FnMut(AgentEvent),
    ) -> Result<(), String> {
        self.abort.begin();
        let mut claude = Sidecar::start(&mut self.command())
            .map_err(|error| format!("Could not start Claude Code: {error}"))?;
        self.abort.runs(claude.stopper());
        let read = self.read(&mut claude, text, &mut emit);
        if read.is_err() {
            claude.kill();
        }
        // Its output has ended: it has, or is ending. What it left running is ended as it is
        // waited for.
        let status = claude.wait().map_err(|error| error.to_string());
        let aborted = self.abort.over();
        let (mut stream, errors) = match read {
            Ok(read) => read,
            // The request could not be sent to a Claude Code that was being stopped.
            Err(_) if aborted => {
                emit(AgentEvent::Aborted);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let status = status?;
        let errors = errors.join().unwrap_or_default();
        if let Some(session) = stream.session.take() {
            self.session = Some(session);
        }
        self.context_window = stream.context_window;
        if aborted {
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

    /// Send `text` to `claude` and read its stream to the end, as `emit`ted
    /// events; with what it said on stderr, once that has ended too.
    fn read(
        &self,
        claude: &mut Sidecar,
        text: &str,
        emit: &mut impl FnMut(AgentEvent),
    ) -> Result<(Stream, thread::JoinHandle<String>), String> {
        let stdout = claude.stdout.take().ok_or("Claude Code has no output")?;
        let mut stderr = claude
            .stderr
            .take()
            .ok_or("Claude Code has no error output")?;
        let errors = thread::spawn(move || {
            let mut text = String::new();
            // What it said before a read failed is all there is to say.
            let _ = stderr.read_to_string(&mut text);
            text
        });
        // The request goes on stdin: list options would take it as a value.
        if let Some(mut stdin) = claude.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|error| format!("Could not send the request to Claude Code: {error}"))?;
        }
        let mut stream = Stream {
            context_window: self.context_window,
            ..Stream::default()
        };
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Some(line) = Line::read(&line) else {
                continue;
            };
            stream.read(line, emit);
        }
        Ok((stream, errors))
    }
}

/// What stops a session's turns. The turn running is stopped as soon as it
/// is asked, as a cancelled sidecar is: Claude Code and what it started are
/// asked to end (SIGTERM to their process group on Unix), and what is left
/// of them after [`GRACE`] is killed.
#[derive(Clone, Default)]
pub(crate) struct Abort(Arc<Mutex<Stopping>>);

#[derive(Default)]
struct Stopping {
    /// The turn was asked to stop.
    asked: bool,
    /// Claude Code, while it runs the turn.
    running: Option<Stopper>,
}

impl Abort {
    /// Stop the turn running, without waiting for it to end.
    pub(crate) fn abort(&self) {
        let mut stopping = self.stopping();
        stopping.asked = true;
        if let Some(claude) = stopping.running.take() {
            stop(claude);
        }
    }

    fn stopping(&self) -> MutexGuard<'_, Stopping> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A turn begins: what was asked of the one before is over.
    fn begin(&self) {
        *self.stopping() = Stopping::default();
    }

    /// Claude Code has started the turn: stopped at once if it was asked to
    /// stop while it started.
    fn runs(&self, claude: Stopper) {
        let mut stopping = self.stopping();
        if stopping.asked {
            stop(claude);
        } else {
            stopping.running = Some(claude);
        }
    }

    /// The turn is over; whether it was asked to stop.
    fn over(&self) -> bool {
        let mut stopping = self.stopping();
        stopping.running = None;
        stopping.asked
    }
}

/// Ask Claude Code and what it started to end, and kill what is left of
/// them after [`GRACE`], on a thread of its own: whoever asked does not wait.
fn stop(claude: Stopper) {
    claude.ask_to_end();
    let waiting = thread::Builder::new()
        .name("kerf-claude-code-stop".into())
        .spawn(move || claude.kill_after(GRACE));
    if waiting.is_err() {
        claude.kill();
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

/// One line of Claude Code's `--output-format stream-json`: the session it is of, which any
/// line may name, and what it says.
#[derive(Deserialize)]
struct Line {
    #[serde(default, deserialize_with = "as_said")]
    session_id: Option<String>,
    #[serde(flatten, deserialize_with = "as_said")]
    message: Message,
}

impl Line {
    /// What the line `text` says, or nothing when it is not a JSON object. It is read as JSON
    /// first, so that a key said twice is the last one said, as it is read by key.
    fn read(text: &str) -> Option<Self> {
        serde_json::from_value(serde_json::from_str(text).ok()?).ok()
    }
}

/// What a line says, by its `type`: those a turn is followed by, and [`Message::Other`] for
/// any other, or a line that says none. Each field reads on its own ([`as_said`], and
/// [`object_said`] where an object belongs): one said otherwise than it is read here is its
/// default, and the rest of the line still says what it does.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    /// `init` begins a turn, with the MCP servers Claude Code started for it.
    System {
        #[serde(default, deserialize_with = "as_said")]
        subtype: Option<String>,
        #[serde(default, deserialize_with = "each_object_said")]
        mcp_servers: Vec<McpServer>,
    },
    /// One of the API's own streaming events (`--include-partial-messages`).
    StreamEvent {
        #[serde(default, deserialize_with = "object_said")]
        event: ApiEvent,
    },
    /// What the model answered a request with: its tool calls, and what the request read.
    Assistant {
        #[serde(default, deserialize_with = "object_said")]
        message: ApiMessage,
    },
    /// What went back to the model: its tool calls' results.
    User {
        #[serde(default, deserialize_with = "object_said")]
        message: ApiMessage,
    },
    /// The turn's end: its answer, or why it failed.
    #[serde(rename = "result")]
    Finished {
        #[serde(default, deserialize_with = "as_said")]
        subtype: Option<String>,
        #[serde(default, deserialize_with = "as_said")]
        is_error: bool,
        /// The answer's text; when the turn failed, why, if it says.
        #[serde(default, deserialize_with = "as_said")]
        result: String,
        /// What each model the turn used read, by the model's name.
        #[serde(rename = "modelUsage", default, deserialize_with = "each_value_said")]
        models: Vec<ModelUsage>,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// An MCP server Claude Code started for a turn, and how it stands: `connected`, or not.
#[derive(Default, Deserialize)]
struct McpServer {
    #[serde(default, deserialize_with = "as_said")]
    name: Option<String>,
    #[serde(default, deserialize_with = "as_said")]
    status: Option<String>,
}

/// The API's streaming events that are followed: a request's answer begins, and its text
/// grows.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiEvent {
    MessageStart,
    ContentBlockDelta {
        #[serde(default, deserialize_with = "object_said")]
        delta: Delta,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// How a block of the answer grew: by text, or otherwise.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Delta {
    TextDelta {
        #[serde(default, deserialize_with = "as_said")]
        text: Option<String>,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// A message of the API as Claude Code passes it on: its content, and, for the model's, what
/// its request read.
#[derive(Default, Deserialize)]
struct ApiMessage {
    #[serde(default, deserialize_with = "each_object_said")]
    content: Vec<Block>,
    #[serde(default, deserialize_with = "object_said")]
    usage: Usage,
}

/// The tokens a request read: its new input, and what it wrote to the cache and read from it.
#[derive(Default, Deserialize)]
struct Usage {
    #[serde(default, deserialize_with = "as_said")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "as_said")]
    cache_read_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "as_said")]
    cache_creation_input_tokens: Option<u64>,
}

impl Usage {
    /// All the context the request read.
    fn read(&self) -> u64 {
        [
            self.input_tokens,
            self.cache_read_input_tokens,
            self.cache_creation_input_tokens,
        ]
        .into_iter()
        .flatten()
        .sum()
    }
}

/// A block of a message's content, of the kinds that are followed.
#[derive(Default, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        #[serde(default, deserialize_with = "as_said")]
        text: Option<String>,
    },
    ToolUse {
        #[serde(default, deserialize_with = "as_said")]
        id: String,
        #[serde(default, deserialize_with = "as_said")]
        name: String,
        /// The call's arguments. Their shape is the tool's own, so they stay JSON:
        /// `summarize_targets` finds the ids and times in any.
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        #[serde(default, deserialize_with = "as_said")]
        tool_use_id: String,
        #[serde(default, deserialize_with = "as_said")]
        is_error: bool,
        #[serde(default, deserialize_with = "as_said")]
        content: ResultContent,
    },
    #[default]
    #[serde(other)]
    Other,
}

/// What a tool's result says: text, or blocks, of which the text ones are read.
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum ResultContent {
    Text(String),
    Blocks(#[serde(deserialize_with = "each_object_said")] Vec<Block>),
    #[default]
    Nothing,
}

/// The context a model the turn used has room for.
#[derive(Default, Deserialize)]
struct ModelUsage {
    #[serde(rename = "contextWindow", default, deserialize_with = "as_said")]
    context_window: Option<u64>,
}

/// What is said where an object belongs, read as `T` when it is one and as `T`'s default
/// otherwise, as a key looked up in anything else finds nothing. ([`as_said`] would read a
/// list as a `T` too, field by field.)
fn object<T: DeserializeOwned + Default>(said: Value) -> T {
    match said {
        Value::Object(_) => serde_json::from_value(said).unwrap_or_default(),
        _ => T::default(),
    }
}

/// A field where an object belongs, read as [`object`] reads it.
fn object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(object(Value::deserialize(deserializer)?))
}

/// A list of objects, each read on its own as [`object`] reads it; what is not a list has
/// none.
fn each_object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => items.into_iter().map(object).collect(),
        _ => Vec::new(),
    })
}

/// A map's values, each read on its own as [`object`] reads it; what is not a map has none.
fn each_value_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Object(values) => values.into_iter().map(|(_, value)| object(value)).collect(),
        _ => Vec::new(),
    })
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

    fn read(&mut self, line: Line, emit: &mut impl FnMut(AgentEvent)) {
        if let Some(session) = line.session_id {
            self.session = Some(session);
        }
        match line.message {
            Message::System {
                subtype,
                mcp_servers,
            } if subtype.as_deref() == Some("init") => {
                let kerf = mcp_servers
                    .iter()
                    .find(|server| server.name.as_deref() == Some(SERVER));
                if let Some(server) = kerf
                    && server.status.as_deref() != Some("connected")
                {
                    self.failure = Some(
                        "Claude Code could not start Kerf's tools (kerf-mcp). Check that \
                             kerf-mcp and kerfd are built."
                            .to_owned(),
                    );
                }
            }
            Message::StreamEvent { event } => match event {
                ApiEvent::MessageStart => emit(AgentEvent::TurnStarted {
                    stop_reason: String::new(),
                }),
                ApiEvent::ContentBlockDelta {
                    delta: Delta::TextDelta { text: Some(text) },
                } => emit(AgentEvent::TextDelta(text)),
                _ => {}
            },
            Message::Assistant { message } => {
                let used = message.usage.read();
                if used > 0 && used != self.context_used {
                    self.context_used = used;
                    self.say_context(emit);
                }
                for block in message.content {
                    let Block::ToolUse { id, name, input } = block else {
                        continue;
                    };
                    let name = tool_name(&name);
                    self.tools.insert(id.clone(), name.clone());
                    emit(AgentEvent::ToolCallStarted {
                        id,
                        name,
                        targets: summarize_targets(&input),
                    });
                }
            }
            Message::User { message } => {
                for block in message.content {
                    let Block::ToolResult {
                        tool_use_id: id,
                        is_error,
                        content,
                    } = block
                    else {
                        continue;
                    };
                    let name = self.tools.get(&id).cloned().unwrap_or_default();
                    let ok = !is_error;
                    emit(AgentEvent::ToolCallFinished {
                        id,
                        name,
                        ok,
                        summary: Box::new(result_summary(&content, ok)),
                    });
                }
            }
            Message::Finished {
                subtype,
                is_error,
                result,
                models,
            } => {
                // The window of the model that answered: the largest, when it used helpers.
                let window = models.iter().filter_map(|model| model.context_window).max();
                if let Some(window) = window.filter(|window| *window != self.context_window) {
                    self.context_window = window;
                    self.say_context(emit);
                }
                if is_error || subtype.as_deref() != Some("success") {
                    if self.failure.is_none() {
                        self.failure = Some(if result.is_empty() {
                            format!(
                                "Claude Code stopped: {}",
                                subtype.as_deref().unwrap_or("error")
                            )
                        } else {
                            result
                        });
                    }
                } else {
                    self.done = Some(result);
                }
            }
            _ => {}
        }
    }
}

/// The tool as the built-in agent names it, without Claude Code's namespace.
fn tool_name(name: &str) -> String {
    name.strip_prefix(TOOL_PREFIX).unwrap_or(name).to_owned()
}

/// Kerf's tools answer in JSON text; read it for the proposal and status the
/// editor follows. Raw output never reaches an event. What each text says stays JSON: it is
/// whatever the tool answered, and `summarize_tool_result` finds what it knows in any.
fn result_summary(content: &ResultContent, ok: bool) -> ToolResultSummary {
    let texts: Vec<&str> = match content {
        ResultContent::Text(text) => vec![text.as_str()],
        ResultContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text { text: Some(text) } => Some(text.as_str()),
                _ => None,
            })
            .collect(),
        ResultContent::Nothing => Vec::new(),
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

    /// A stream that has read `lines`, and the events they made.
    fn read(lines: &[&str]) -> (Stream, Vec<AgentEvent>) {
        let mut stream = Stream::default();
        let mut events = Vec::new();
        for line in lines.iter().filter_map(|line| Line::read(line)) {
            stream.read(line, &mut |event| events.push(event));
        }
        (stream, events)
    }

    #[test]
    fn a_line_said_askew_reads_as_its_keys_did() {
        // A list where an object belongs says nothing, as a key looked up in it found nothing.
        let (stream, events) = read(&[
            r#"{"type":"system","subtype":"init","mcp_servers":[["kerf","failed"]]}"#,
            r#"{"type":"stream_event","event":["message_start"]}"#,
            r#"{"type":"assistant","message":{"content":[["tool_use","t1","mcp__kerf__edit",{}]],"usage":[7,0,0]}}"#,
            r#"{"type":"result","subtype":"success","result":"Done.","modelUsage":{"m":[1000000]}}"#,
        ]);
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(stream.failure, None);
        assert_eq!(stream.context_window, 0);
        assert_eq!(stream.done.as_deref(), Some("Done."));
        // Of a key said twice, the last is read, and the line with it.
        let (stream, _) = read(&[
            r#"{"session_id":"a","session_id":"b","type":"result","subtype":"success","result":"x"}"#,
        ]);
        assert_eq!(stream.session.as_deref(), Some("b"));
        assert_eq!(stream.done.as_deref(), Some("x"));
    }

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
