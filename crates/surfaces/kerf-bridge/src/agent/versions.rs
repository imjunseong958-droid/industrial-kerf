//! Versions of one request: several agents at once, each making its own version of it as a
//! proposal in a conversation of its own, to compare.

use super::{new_agent, staged_proposal};
use crate::event::{AgentVersion, VersionState};
use crate::wake::Outbox;
use crate::{conversations, emit, error, AgentRequest, Command, Event, Shared};
use kerf_agent::AgentEvent;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread;

/// The most versions of a request made at once.
const MAX_VERSIONS: usize = 3;
const VERSION_KEYS: [&str; MAX_VERSIONS] = ["A", "B", "C"];
const VERSION_ANGLES: [&str; MAX_VERSIONS] = [
    "Stay closest to what was asked, in its most direct form.",
    "Take a bolder approach: a different opening, structure, or pace.",
    "Take the simplest approach: shorter, with fewer cuts and effects.",
];

/// What one agent is asked to make: its version of the person's request.
fn version_prompt(text: &str, index: usize, count: usize) -> String {
    format!(
        "{text}\n\nMake version {key} of {count}; other agents are making the others at the same \
         time, to compare. {angle} Do not ask questions. Stage it as one proposal with `edit` \
         and do not change any other proposal. End with one line saying what makes this version \
         different.",
        key = VERSION_KEYS[index],
        angle = VERSION_ANGLES[index],
    )
}

/// Make `request.versions` versions of a request, each agent on its own thread.
pub(super) fn run_versions(
    directory: &std::path::Path,
    binary: &std::path::Path,
    request: &AgentRequest,
    tx: &Outbox,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let count = request.versions.min(MAX_VERSIONS);
    // Counted with the agent's own thread, which waits here for them all.
    thread::scope(|scope| {
        for index in 0..count {
            thread::Builder::new()
                .name("bridge:version".to_owned())
                .spawn_scoped(scope, move || {
                    run_version(
                        directory, binary, request, index, count, tx, commands, shared,
                    )
                })
                // As `Scope::spawn` does when the system has no thread to give.
                .expect("failed to spawn thread");
        }
    });
}

/// One agent's version: its request and reply go in a thread of their own,
/// and `agent_version` events say how it is going.
#[allow(clippy::too_many_arguments)]
fn run_version(
    directory: &std::path::Path,
    binary: &std::path::Path,
    request: &AgentRequest,
    index: usize,
    count: usize,
    tx: &Outbox,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let key = VERSION_KEYS[index];
    let title = format!("Version {key}: {}", request.text.trim());
    emit(
        tx,
        Event::AgentVersion(AgentVersion {
            key: key.to_owned(),
            state: VersionState::Running {
                title: Some(title.clone()),
                conversation: None,
                status: None,
            },
        }),
    );
    let finish = |state: VersionState| {
        emit(
            tx,
            Event::AgentVersion(AgentVersion {
                key: key.to_owned(),
                state,
            }),
        );
    };
    // Each version is a conversation of its own.
    let store = conversations::Store::of(directory);
    let mut turn = match conversations::Turn::begin(&store, None, &title, None, tx) {
        Ok(turn) => turn,
        Err(err) => {
            finish(VersionState::Failed {
                proposal_id: None,
                message: Some(err),
            });
            return;
        }
    };
    emit(
        tx,
        Event::AgentVersion(AgentVersion {
            key: key.to_owned(),
            state: VersionState::Running {
                title: None,
                conversation: Some(turn.id().to_owned()),
                status: None,
            },
        }),
    );
    let mut proposal: Option<String> = None;
    let mut stopped = false;
    let result = (|| {
        let mut session = new_agent(shared, directory, binary, &request.settings, tx)?;
        shared.agent.version_runs(session.abort_handle());
        session.run_with_events(
            &version_prompt(&request.text, index, count),
            |event| match event {
                AgentEvent::ToolCallStarted { name, .. } => emit(
                    tx,
                    Event::AgentVersion(AgentVersion {
                        key: key.to_owned(),
                        state: VersionState::Running {
                            title: None,
                            conversation: None,
                            status: Some(name),
                        },
                    }),
                ),
                AgentEvent::ToolCallFinished {
                    name, ok, summary, ..
                } => {
                    turn.step(&format!(
                        "{name} · {}{}",
                        if ok { "done" } else { "failed" },
                        summary
                            .status
                            .as_ref()
                            .map(|status| format!(" · {status}"))
                            .unwrap_or_default(),
                    ));
                    if staged_proposal(&summary) {
                        if let Some(id) = &summary.proposal {
                            proposal = Some(id.clone());
                        }
                        // A full queue refreshes anyway: the runtime worker republishes a
                        // changed state on its own soon.
                        let _ = commands.try_send(Command::Refresh);
                    }
                }
                AgentEvent::Done { text } => turn.done(&text),
                AgentEvent::Aborted => {
                    stopped = true;
                    turn.stopped();
                }
                AgentEvent::TextDelta(_)
                | AgentEvent::TurnStarted { .. }
                | AgentEvent::Context { .. } => {}
            },
        )?;
        Ok::<_, String>(())
    })();
    let outcome = match &result {
        Ok(()) => conversations::Outcome::Done(String::new()),
        Err(err) => conversations::Outcome::Failed(err.clone()),
    };
    if let Err(err) = turn.finish(&store, outcome, proposal.clone(), None, tx) {
        error(tx, format!("Kerf's reply was not kept: {err}"));
    }
    match result {
        Err(err) => finish(VersionState::Failed {
            proposal_id: proposal,
            message: Some(err),
        }),
        Ok(()) if stopped => finish(VersionState::Stopped {
            proposal_id: proposal,
            message: None,
        }),
        Ok(()) => finish(VersionState::Done {
            proposal_id: proposal,
            message: None,
        }),
    }
}
