//! The agent behind the chat: Kerf's own loop on a model API, or Claude Code, run a turn
//! at a time on the session kept for its conversation, and what its events tell the editor.

mod native;
mod versions;

use crate::event::{AgentState, AgentUpdate, ErrorEvent, ToolState};
use crate::wake::Outbox;
use crate::CommandName;
use crate::{
    claude_code, conversations, emit, error, sessions, steps, AgentRequest, AgentSettings,
    ChatProvider, Command, Event, Shared,
};
use kerf_agent::{AgentEvent, AgentSession};
use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use kerf_runtime::RuntimeClient;
use native::native_agent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use versions::run_versions;

/// The agent behind a chat: Kerf's own loop on a model API, with the router
/// its tools run on, or Claude Code.
pub(crate) enum Agent {
    Native(Box<AgentSession>, KerfMcpRouter),
    ClaudeCode(claude_code::ClaudeCodeSession),
}

impl Agent {
    fn abort_handle(&self) -> Arc<AtomicBool> {
        match self {
            Self::Native(session, _) => session.abort_handle(),
            Self::ClaudeCode(session) => session.abort_handle(),
        }
    }

    /// What its context is counted in: the model's tokens on Claude Code,
    /// which says how many a request read, or Kerf's own measure of its
    /// input allowance.
    fn context_unit(&self) -> &'static str {
        match self {
            Self::Native(..) => "units",
            Self::ClaudeCode(_) => "tokens",
        }
    }

    /// What the next turn may do without asking: the mode the person chose
    /// in the editor, or their own file's with none. The session goes on, so
    /// a change of mode keeps what the agent knows of the conversation.
    fn set_permissions(&mut self, mode: Option<Mode>) {
        match self {
            Self::Native(_, router) => router.set_permissions(mode.map(chosen_permissions)),
            Self::ClaudeCode(session) => session.set_permissions(mode),
        }
    }

    fn run_with_events(&mut self, text: &str, emit: impl FnMut(AgentEvent)) -> Result<(), String> {
        match self {
            Self::Native(session, _) => session
                .run_with_events(text, emit)
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::ClaudeCode(session) => session.run_with_events(text, emit),
        }
    }
}

/// The person's permissions with the mode they chose in the editor in place
/// of their file's: how many makings run on their own stays the file's say.
fn chosen_permissions(mode: Mode) -> Permissions {
    Permissions {
        mode,
        ..Permissions::load().unwrap_or_default()
    }
}

/// The mode a request names: `ask`, `accept-edits` or `auto`. A name that is
/// none of them asks, never lets more through.
pub(crate) fn permission_mode(name: Option<&str>) -> Option<Mode> {
    name.map(|name| Mode::named(name).unwrap_or(Mode::Ask))
}

fn new_agent(
    shared: &Arc<Shared>,
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &Outbox,
) -> Result<Agent, String> {
    if !settings.cloud && settings.provider == Some(ChatProvider::ClaudeCode) {
        return claude_code::ClaudeCodeSession::new(directory, binary, settings.model.clone())
            .map(Agent::ClaudeCode);
    }
    native_agent(shared, directory, binary, settings, tx)
        .map(|(session, router)| Agent::Native(Box::new(session), router))
}

/// Where a tool call is working, for the timeline and picture to follow.
fn focus_event(name: &str, targets: &kerf_agent::AgentTargetSummary) -> Option<Event> {
    (!targets.times.is_empty() || !targets.clips.is_empty()).then(|| Event::AgentFocus {
        tool: name.to_owned(),
        times: targets.times.clone(),
        clips: targets.clips.clone(),
    })
}

/// Whether a finished tool call changed what the timeline shows.
fn staged_proposal(summary: &kerf_agent::ToolResultSummary) -> bool {
    summary.proposal.is_some() || summary.status.as_deref() == Some("awaiting_review")
}

pub(crate) fn agent_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<AgentRequest>,
    commands: SyncSender<Command>,
    tx: Outbox,
    shared: Arc<Shared>,
) {
    // A session for each conversation, kept beside it as each turn ends, so
    // going back to one goes on with what its agent knew (sessions.rs).
    let mut sessions = sessions::Sessions::of(&directory);
    // A turn whose request could not be kept still has a session of its own.
    let mut loose: Option<(AgentSettings, Agent)> = None;
    let store = conversations::Store::of(&directory);
    while let Ok(request) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        emit(&tx, Event::AgentBusy { busy: true });
        emit(
            &tx,
            Event::AgentState {
                state: AgentState::Running,
                message: None,
            },
        );
        if request.versions > 0 {
            run_versions(&directory, &binary, &request, &tx, &commands, &shared);
            shared.agent.versions_are_over();
            emit(
                &tx,
                Event::AgentState {
                    state: AgentState::Completed,
                    message: None,
                },
            );
            shared.agent.end();
            emit(&tx, Event::AgentBusy { busy: false });
            // The runtime worker is gone only as the bridge closes, with nothing to refresh.
            let _ = commands.send(Command::Refresh);
            continue;
        }
        // The turn in its conversation: the person's request first, and the
        // open proposals, to find the one this turn stages.
        let mut kept = request
            .turn
            .as_ref()
            .and_then(|(prompt, conversation, at)| {
                let client = RuntimeClient::new(&directory).ok()?;
                let before = client
                    .get_state()
                    .map(|state| conversations::open_proposals(&state))
                    .unwrap_or_default();
                match conversations::Turn::begin(&store, conversation.as_deref(), prompt, *at, &tx)
                {
                    Ok(turn) => Some((client, turn, before)),
                    Err(err) => {
                        error(&tx, format!("The request was not kept: {err}"));
                        None
                    }
                }
            });
        let conversation = kept.as_ref().map(|(_, turn, _)| turn.id().to_owned());
        let earlier = kept.as_ref().and_then(|(_, turn, _)| turn.earlier());
        let mut turn_events = TurnEvents {
            tx: &tx,
            commands: &commands,
            conversation: conversation.clone(),
            turn: kept.as_mut().map(|(_, turn, _)| turn),
            running: std::collections::HashMap::new(),
            heard: false,
        };
        let open = || new_agent(&shared, &directory, &binary, &request.settings, &tx);
        let mut result = (|| {
            let (current, opened) = match &conversation {
                Some(id) => sessions.get(id, &request.settings, open)?,
                None => {
                    let fresh = loose
                        .as_ref()
                        .is_none_or(|(settings, _)| settings != &request.settings);
                    if fresh {
                        loose = Some((request.settings.clone(), open()?));
                    }
                    let (_, agent) = loose.as_mut().ok_or("Agent session unavailable")?;
                    let opened = if fresh {
                        sessions::Opened::New
                    } else {
                        sessions::Opened::Continued
                    };
                    (agent, opened)
                }
            };
            run_turn(
                current,
                opened,
                &request,
                earlier.as_deref(),
                &shared,
                &mut turn_events,
            )
            .map_err(|err| {
                if opened == sessions::Opened::Restored {
                    format!("{RESTORED_FAILED}{err}")
                } else {
                    err
                }
            })
        })();
        // A kept session that no longer goes on (Claude Code forgot it, or its
        // transcript would not run): the conversation starts its agent afresh,
        // hearing what was said, once, if nothing of the turn was heard yet.
        if let (Err(err), Some(id)) = (&result, &conversation)
            && err.starts_with(RESTORED_FAILED)
            && !turn_events.heard
        {
            sessions.forget(id);
            result = sessions
                .get(id, &request.settings, open)
                .and_then(|(current, opened)| {
                    run_turn(
                        current,
                        opened,
                        &request,
                        earlier.as_deref(),
                        &shared,
                        &mut turn_events,
                    )
                });
        }
        let result = result.map_err(|err| {
            err.strip_prefix(RESTORED_FAILED)
                .map(str::to_owned)
                .unwrap_or(err)
        });
        drop(turn_events);
        let outcome = match &result {
            Ok(()) => conversations::Outcome::Done(String::new()),
            Err(err) => conversations::Outcome::Failed(err.clone()),
        };
        if let Err(err) = result {
            emit(
                &tx,
                Event::AgentState {
                    state: AgentState::Failed,
                    message: Some(err.clone()),
                },
            );
            // Named as the turn's, so the editor clears it when the next turn starts.
            emit(
                &tx,
                Event::Error(ErrorEvent {
                    message: err,
                    command: Some(CommandName::Agent),
                    ..ErrorEvent::default()
                }),
            );
        }
        // What the session knows, kept for the next launch.
        let claude_session = conversation.as_deref().and_then(|id| sessions.kept(id));
        if let Some((client, turn, before)) = kept {
            let proposal = client
                .get_state()
                .ok()
                .and_then(|state| conversations::staged_proposal(&before, &state));
            if let Err(err) = turn.finish(&store, outcome, proposal, claude_session, &tx) {
                error(&tx, format!("Kerf's reply was not kept: {err}"));
            }
        }
        shared.agent.turn_is_over();
        shared.agent.end();
        emit(&tx, Event::AgentBusy { busy: false });
        // The runtime worker is gone only as the bridge closes, with nothing to refresh.
        let _ = commands.send(Command::Refresh);
    }
}

/// How a turn on a session gone on from what was kept fails, so that it is
/// tried once more on a fresh one.
const RESTORED_FAILED: &str = "\u{0}restored: ";

/// Kerf's agent, as its thread and the bridge's callers share it: whether a request (a
/// turn, or a request's versions) has it, and what stops each.
#[derive(Default)]
pub(crate) struct Turns {
    busy: AtomicBool,
    /// What stops the turn running.
    abort: Mutex<Option<Arc<AtomicBool>>>,
    /// What stops each agent making a version of a request, while they run.
    versions: Mutex<Vec<Arc<AtomicBool>>>,
}

impl Turns {
    /// Take the agent for a request. False when one has it already.
    pub(crate) fn begin(&self) -> bool {
        !self.busy.swap(true, Ordering::SeqCst)
    }

    /// The request is over, or never reached the agent's thread.
    pub(crate) fn end(&self) {
        self.busy.store(false, Ordering::SeqCst);
    }

    /// The turn's agent has started: `abort` stops it.
    fn turn_runs(&self, abort: Arc<AtomicBool>) {
        if let Ok(mut running) = self.abort.lock() {
            *running = Some(abort);
        }
    }

    fn turn_is_over(&self) {
        if let Ok(mut running) = self.abort.lock() {
            *running = None;
        }
    }

    /// One more agent is making a version: `abort` stops it.
    fn version_runs(&self, abort: Arc<AtomicBool>) {
        if let Ok(mut running) = self.versions.lock() {
            running.push(abort);
        }
    }

    fn versions_are_over(&self) {
        if let Ok(mut running) = self.versions.lock() {
            running.clear();
        }
    }

    /// Stop what is running, for the person who asked: without waiting for the turn's
    /// handle, and false when it could not be had this moment, so they may ask again.
    pub(crate) fn try_stop(&self) -> bool {
        self.stop_versions();
        match self.abort.try_lock() {
            Ok(abort) => {
                if let Some(abort) = &*abort {
                    abort.store(true, Ordering::SeqCst);
                }
                true
            }
            _ => false,
        }
    }

    /// Stop what is running, as the bridge is let go.
    pub(crate) fn stop(&self) {
        if let Ok(abort) = self.abort.lock()
            && let Some(abort) = &*abort
        {
            abort.store(true, Ordering::SeqCst);
        }
        self.stop_versions();
    }

    fn stop_versions(&self) {
        if let Ok(aborts) = self.versions.lock() {
            for abort in aborts.iter() {
                abort.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// What a turn's events go to: the editor, the conversation that keeps the
/// turn, and the tool calls under way, by id.
struct TurnEvents<'a> {
    tx: &'a Outbox,
    commands: &'a SyncSender<Command>,
    conversation: Option<String>,
    turn: Option<&'a mut conversations::Turn>,
    running: std::collections::HashMap<String, kerf_agent::AgentTargetSummary>,
    /// Whether anything of the turn has been heard yet.
    heard: bool,
}

impl TurnEvents<'_> {
    fn hear(&mut self, event: AgentEvent, unit: &str) {
        let tx = self.tx;
        if !matches!(event, AgentEvent::Context { .. }) {
            self.heard = true;
        }
        match event {
            // How full Kerf's context is, for the composer's meter.
            AgentEvent::Context { used, limit } => emit(
                tx,
                Event::AgentContext {
                    used,
                    limit,
                    unit: unit.to_owned(),
                    conversation: self.conversation.clone(),
                },
            ),
            AgentEvent::TextDelta(text) => {
                emit(tx, Event::Agent(AgentUpdate::Delta { text }));
            }
            AgentEvent::TurnStarted { .. } => emit(
                tx,
                Event::Agent(AgentUpdate::Status {
                    text: "Planning next step…".to_owned(),
                }),
            ),
            // A tool call is one row of the work log: said as it starts, and
            // again in its place once it ends.
            AgentEvent::ToolCallStarted { id, name, targets } => {
                if let Some(focus) = focus_event(&name, &targets) {
                    emit(tx, focus);
                }
                let text = steps::running(&name, &targets);
                self.running.insert(id.clone(), targets);
                emit(
                    tx,
                    Event::Agent(AgentUpdate::Tool {
                        id,
                        state: ToolState::Running,
                        text,
                        rendered: None,
                    }),
                );
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                ok,
                summary,
            } => {
                if summary.status.as_deref() == Some("awaiting_review") {
                    emit(
                        tx,
                        Event::AgentState {
                            state: AgentState::AwaitingReview,
                            message: None,
                        },
                    );
                }
                // The proposal shows on the timeline as it grows.
                if staged_proposal(&summary) {
                    // A full queue refreshes anyway: the runtime worker republishes a changed
                    // state on its own soon.
                    let _ = self.commands.try_send(Command::Refresh);
                }
                let targets = self.running.remove(&id).unwrap_or_default();
                let text = steps::finished(&name, &targets, ok, &summary);
                // The cut itself was rendered: what Kerf was asked to make is made.
                let rendered = ok && steps::tool_name(&name) == "render" && summary.deliverable;
                emit(
                    tx,
                    Event::Agent(AgentUpdate::Tool {
                        id,
                        state: if ok {
                            ToolState::Done
                        } else {
                            ToolState::Failed
                        },
                        text: text.clone(),
                        rendered: Some(rendered),
                    }),
                );
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.step(&text);
                }
                // A making the person is asked to run: a card with Make.
                if let (Some("asked"), Some(media), Some(ask)) =
                    (summary.status.as_deref(), &summary.media, &summary.ask)
                {
                    let ask = conversations::Ask {
                        media: media.clone(),
                        rolls: summary.rolls.filter(|rolls| *rolls > 1),
                        take: summary.take,
                        text: ask.clone(),
                    };
                    emit(
                        tx,
                        Event::AgentAsk {
                            media: ask.media.clone(),
                            rolls: ask.rolls,
                            take: ask.take,
                            text: ask.text.clone(),
                            conversation: self.conversation.clone(),
                        },
                    );
                    if let Some(turn) = self.turn.as_deref_mut() {
                        turn.ask(ask);
                    }
                }
            }
            AgentEvent::Done { text } => {
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.done(&text);
                }
                emit(tx, Event::Agent(AgentUpdate::Done { text }));
            }
            AgentEvent::Aborted => {
                if let Some(turn) = self.turn.as_deref_mut() {
                    turn.stopped();
                }
                emit(
                    tx,
                    Event::AgentState {
                        state: AgentState::Aborted,
                        message: None,
                    },
                );
                emit(
                    tx,
                    Event::Agent(AgentUpdate::Status {
                        text: "Stopped".to_owned(),
                    }),
                );
            }
        }
    }
}

/// One turn on `agent`. A session that has nothing of the conversation hears
/// what was said in it first.
fn run_turn(
    agent: &mut Agent,
    opened: sessions::Opened,
    request: &AgentRequest,
    earlier: Option<&str>,
    shared: &Shared,
    events: &mut TurnEvents<'_>,
) -> Result<(), String> {
    let text = match (opened, earlier) {
        (sessions::Opened::New, Some(earlier)) => format!("{earlier}\nNow: {}", request.text),
        _ => request.text.clone(),
    };
    agent.set_permissions(request.permissions);
    shared.agent.turn_runs(agent.abort_handle());
    let unit = agent.context_unit();
    agent.run_with_events(&text, |event| events.hear(event, unit))?;
    emit(
        events.tx,
        Event::AgentState {
            state: AgentState::Completed,
            message: None,
        },
    );
    Ok(())
}
