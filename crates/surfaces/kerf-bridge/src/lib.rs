//! The editor's bridge to Kerf: [`Command`]s in, [`Event`]s out, over kerfd and Rust
//! workers.
//!
//! kerf-editor opens a project with [`Bridge::open`], sends it [`Command`]s with
//! [`Bridge::send`], and takes its [`Event`]s with [`Bridge::poll`] and its pictures with
//! [`Bridge::frame`] and [`Bridge::signal`], waiting for them on [`Bridge::wake`]. Every
//! call returns at once and never waits on
//! project I/O: canonical edits stay in kerfd, and runtime, media, agent, render and
//! collaboration work run on Rust workers. Media frames travel through a latest-frame slot
//! rather than an event or an accumulating queue. All text is UTF-8.
//!
//! Dropping the bridge closes it without waiting for its workers: the agent's turn, the
//! viewer preview rendering, the QC check, the makings it started and the render queue's
//! jobs are stopped. A making another program runs on the project goes on.
//!
//! # Commands and events
//!
//! [`Command`] and [`Event`] are the protocol: each variant says what it asks or tells,
//! what answers a command, and when one is refused. On the wire each is one JSON object
//! whose `type` names it, such as `{"type": "seek", "position": 2.5}`; [`commands_run`]
//! gives commands in that form. An event of a type this build does not know reads as
//! [`Event::Unknown`], so a newer bridge never breaks an older window.
//!
//! [`Bridge::send`] returns `false` when a command is refused: it cannot be taken as it is
//! (a seek before 0, an agent turn while one runs), or its bounded queue is full. Show an
//! error then; never silently drop edits.
//!
//! # Pictures
//!
//! [`Bridge::frame`] takes the latest playback frame only ([`Frame`]: RGBA8). [`Bridge::signal`]
//! takes the frame `scope_signal` drew ([`Signal`]: 16-bit little-endian RGBA in BT.2020, on
//! curve 1 (PQ) or 2 (HLG)). Neither copies the pixels.
//!
//! # Without a bridge
//!
//! These are synchronous, and can be called before a bridge exists:
//!
//! - The command registry (kerf-commands), pure: [`commands_catalog`], [`commands_states`],
//!   [`commands_run`].
//! - The catalog: [`fx_catalog`], [`fx_catalog_all`], [`clip_properties`].
//! - [`about`]; [`update_check`], which asks the network: call it off the UI thread.
//! - [`ffmpeg_bin`]: the FFmpeg kerfd runs, for the editor's own pictures (found once, by
//!   running the candidates: call it off the UI thread the first time).
//! - Crash reports: [`crash_install`], [`crash_fatal`], [`crash_reports`].
//! - The system keychain: [`secret`].
//! - The project launcher, local: [`project_inspect`], [`preset_inspect`] (which probes files
//!   with ffprobe and may take a moment).
//! - Serving Kerf's tools over MCP from the editor's own binary: [`serve_mcp`].
//!
//! ## Project tasks
//!
//! Project tasks may take a while: [`Task::start`] runs one on a thread of its own, and
//! [`Task::status`] is polled from the UI thread. Dropping a task leaves one still running to
//! finish on its own. The request is one of:
//!
//! - `{type: "create", name, location, template: standard|vertical|square, rate:
//!   "23.976"|"24"|"25"|"29.97"|"30"|"60", git, remote?, github?: {name?, private}, kerfd?,
//!   preset?: {path, bindings: {slotId: {file, from? (a time such as "2s" or "48f")}}, texts?:
//!   {clipId: text}}}` makes the folder and, with a preset, copies it into `presets/`, copies
//!   each file into `media/`, imports them with kerfd and applies the preset with those
//!   bindings as the project's first edit.
//! - `{type: "clone", url, location}`.
//! - `{type: "tools"}` -> `{git, gitLfs, gh, ghUser}`.
//!
//! The status is `{status: "running", message?}`, `{status: "done", path?, remote?,
//! warnings?, ...}` or `{status: "failed", error}`.

// No unsafe code in the library; the crate's empty `[lints]` table is for a Windows test.
#![forbid(unsafe_code)]

mod analysis;
mod claude_code;
mod cloud_session;
mod collab;
mod command;
mod conversations;
pub mod event;
mod looks;
mod media;
mod meter;
mod paperwork;
mod presets;
mod preview_turns;
mod previews;
mod qc;
mod queue;
mod recipes;
mod secrets;
mod sessions;
mod steps;
mod takes;
mod tasks;
#[cfg(test)]
mod tests_support;
mod tracks;
mod updates;
mod wake;
mod workflows;

pub use claude_code::SERVE_MCP;
pub use command::{AgentSettings, ChatProvider, Command};
pub use conversations::Request as ConversationsRequest;
pub use event::{Event, Snapshot};
pub use kerf_agent::{Focus, FocusScope, OpenAiApiMode};
pub use kerf_collab::scaffold::GitHubRepository;
pub use kerf_runtime::{EditOp, GeneratorSpec, MaskOutline, RateSpec, TrimSide};
pub use media::CastEntry;
pub use paperwork::CaptionsRequest;
pub use previews::{PreviewEffect, Request as EffectPreviewsRequest};
pub use queue::ExportRequest;
pub use recipes::Edit as RecipeEdit;
pub use tasks::Task;
pub use tracks::{Follow as ShotFollow, Of as ShotOf};
pub use wake::Wake;
pub use workflows::Edit as WorkflowEdit;

use event::{
    AgentState, AgentUpdate, AgentVersion, CloudAccess, ConnectionStatus, ErrorEvent, NextStep,
    Preview, PreviewStatus, ToolState, VersionState,
};
use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTool, ProviderName, ReasoningEffort,
};
use kerf_mcp::inbound;
use kerf_mcp::permissions::{Mode, Permissions};
use kerf_mcp::KerfMcpRouter;
use kerf_model::{
    AudioInfo, Clip, Content, Id, Media, Project, Ratio, Sequence, Time, Track, TrackKind,
    VideoInfo,
};
use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use kerf_runtime::{RuntimeClient, StateSnapshot};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use wake::Outbox;

/// The agent behind a chat: Kerf's own loop on a model API, with the router
/// its tools run on, or Claude Code.
enum Agent {
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
fn permission_mode(name: Option<&str>) -> Option<Mode> {
    name.map(|name| Mode::named(name).unwrap_or(Mode::Ask))
}

/// What the viewer plays: the project itself, or a rendered preview of it,
/// the file that preview plays, and the proposal it previews. Both have
/// their media paths resolved.
#[derive(Clone)]
enum MediaProject {
    Native(Project),
    Rendered(Project, PathBuf, Option<String>),
}

/// One render the runtime worker hands to an export thread.
struct RenderRequest {
    quality: String,
    /// Played in place of the program once rendered.
    preview: bool,
    proposal: Option<String>,
    range: Option<kerf_runtime::RenderRange>,
    sequence: Option<String>,
}

impl RenderRequest {
    /// A `preview` event about this render. Every render the bridge runs here is a viewer
    /// preview (`preview` is always true); the render queue's exports are `queue.rs`'s.
    fn event(&self, status: PreviewStatus) -> Preview {
        Preview {
            status,
            proposal_id: self.proposal.clone(),
            ..Preview::default()
        }
    }
}

struct PendingSeek {
    position: f64,
    exact: bool,
}

struct Shared {
    closed: AtomicBool,
    revision: AtomicU64,
    agent_busy: AtomicBool,
    /// A viewer preview is rendering; exports go through `renders`.
    preview_busy: AtomicBool,
    abort: Mutex<Option<Arc<AtomicBool>>>,
    /// The agents making versions of a request, while they run.
    version_aborts: Mutex<Vec<Arc<AtomicBool>>>,
    frame: Mutex<Option<PlaybackFrame>>,
    seek: Mutex<Option<PendingSeek>>,
    project: Mutex<Option<MediaProject>>,
    /// The project as edited, its media paths resolved.
    model: Mutex<Option<Project>>,
    /// The latest effect preview request; older ones stop.
    previews: AtomicU64,
    /// The project directory, which relative media paths are resolved
    /// against.
    directory: PathBuf,
    /// The media status last emitted.
    media_status: Mutex<media::MediaStatus>,
    /// Copies into `media/` still running; live sync waits for them.
    copies: AtomicUsize,
    /// The cancel marker of the preview render running, so closing the
    /// bridge stops it.
    preview_cancel: Mutex<Option<PathBuf>>,
    /// The order previews and their stops were sent in, which says which
    /// previews `preview_stop` and `preview_exit` asked to stop.
    preview_turns: preview_turns::PreviewTurns,
    /// Preview renders nothing shows any more whose files could not be
    /// removed yet: on Windows a playback worker still shutting down keeps
    /// its file open. The media worker tries them again.
    stale_previews: Mutex<Vec<PathBuf>>,
    /// The preview file the media worker plays, so closing the bridge can
    /// remove it even when the process exits before that worker stops.
    shown_preview: Mutex<Option<PathBuf>>,
    /// A QC check is running.
    check_busy: AtomicBool,
    /// Cancels the QC check running, so closing the bridge stops it.
    check_cancel: Mutex<Option<kerf_engine::CancellationToken>>,
    /// The makings running, by the media each makes: what cancels it.
    makings: Mutex<std::collections::BTreeMap<String, kerf_engine::CancellationToken>>,
    /// The makings another program runs on this project that are shown, by the media each
    /// makes: the take and the line last shown of it.
    others_makings: Mutex<std::collections::BTreeMap<String, (u32, String)>>,
    /// The render queue the export thread works through.
    renders: queue::Queue,
    /// Loudness, media info and media index work.
    analysis: analysis::Analysis,
    /// The workflow last said of each step a graph has open.
    workflows: workflows::Said,
}

/// One project open in the editor: commands in through [`Bridge::send`], events out through
/// [`Bridge::poll`], and pictures through [`Bridge::frame`] and [`Bridge::signal`]. Every
/// call returns at once; kerfd, playback, the agent and the other workers run on threads of
/// their own. Dropping it closes the project without waiting for them.
pub struct Bridge {
    /// The project folder, where its conversations are kept.
    directory: PathBuf,
    events_tx: Outbox,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    collab: SyncSender<Command>,
    prompts: SyncSender<AgentRequest>,
    /// The conversations' own worker: their files are read and written off
    /// the caller's thread, in the order asked.
    conversations: SyncSender<conversations::Request>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
}

struct AgentRequest {
    text: String,
    settings: AgentSettings,
    /// More than none: this many agents make their own version of the request.
    versions: usize,
    /// The person's own words, the conversation the turn goes on in (none
    /// starts one), and the playhead when it was asked: every turn of one
    /// agent is kept; one of several versions is not.
    turn: Option<(String, Option<String>, Option<f64>)>,
    /// What this turn may do without asking; none is the person's file's say.
    permissions: Option<Mode>,
}

fn emit(tx: &Outbox, event: Event) {
    let _ = tx.send(event);
}

fn error(tx: &Outbox, value: impl std::fmt::Display) {
    emit(
        tx,
        Event::Error(ErrorEvent {
            message: value.to_string(),
            ..ErrorEvent::default()
        }),
    );
}

fn key() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    format!(
        "editor-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn publish(state: StateSnapshot, shared: &Arc<Shared>, tx: &Outbox, previous: &mut Option<Value>) {
    publish_snapshot(state, shared, tx, previous, false);
}

fn publish_force(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &Outbox,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, true);
}

fn publish_snapshot(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &Outbox,
    previous: &mut Option<Value>,
    force: bool,
) {
    let snapshot = serde_json::to_value(&state).unwrap_or_default();
    if !force && previous.as_ref() == Some(&snapshot) {
        return;
    }

    // Proposal-only refreshes must not reopen decoders or interrupt playback.
    let project_changed = previous
        .as_ref()
        .and_then(|snapshot| snapshot.get("document"))
        .and_then(Value::as_str)
        != Some(state.document.as_str());
    if project_changed {
        // The document may use looks another process added to the project.
        let _ = kerf_model::look::refresh_project_looks(&shared.directory);
        match kerf_runtime::parse_document(&state.document) {
            Ok(mut project) => {
                media::publish_status(shared, tx, &project);
                kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
                // What new media are, for the bin and the inspector.
                analysis::media_changed(shared, tx, &project);
                if let Ok(mut model) = shared.model.lock() {
                    *model = Some(project.clone());
                }
                if let Ok(mut slot) = shared.project.lock() {
                    shared.revision.fetch_add(1, Ordering::SeqCst);
                    install_project(shared, &mut slot, MediaProject::Native(project));
                }
            }
            Err(err) => error(tx, err.message),
        }
    }
    *previous = Some(snapshot);
    emit(
        tx,
        Event::State {
            snapshot: Box::new(snapshot_of(&state)),
        },
    );
}

/// The `state` event's snapshot: the state, and each open proposal's timeline beside the
/// current one, so the editor can show proposals on the timeline before they are
/// accepted: `proposalTimelines` by proposal id (an open proposal's earlier versions too,
/// while they were made on this revision), and `proposalTimeline` for the pending
/// proposal. The lists a state leaves out while empty are always there.
fn snapshot_of(state: &StateSnapshot) -> Snapshot {
    let proposal_timeline = match &state.pending_proposal {
        Some(kerf_runtime::PendingProposal::Pending { script, .. }) => {
            kerf_runtime::proposal_timeline_view(&state.document, script).ok()
        }
        _ => None,
    };
    let mut proposal_timelines = std::collections::BTreeMap::new();
    for entry in state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
    {
        let earlier = entry.versions.iter().filter(|version| {
            version.proposal_id != entry.proposal_id && version.base_revision == state.revision
        });
        let scripts = std::iter::once((&entry.proposal_id, &entry.script))
            .chain(earlier.map(|version| (&version.proposal_id, &version.script)));
        for (id, script) in scripts {
            if let Ok(timeline) = kerf_runtime::proposal_timeline_view(&state.document, script) {
                proposal_timelines.insert(id.clone(), timeline);
            }
        }
    }
    Snapshot {
        document: state.document.clone(),
        // kerfd's timeline view, as its own TimelineView wrote it.
        timeline: serde_json::from_value(state.timeline.clone()).unwrap_or_default(),
        revision: state.revision,
        can_undo: state.can_undo,
        can_redo: state.can_redo,
        duration: state.duration,
        pending_proposal: state.pending_proposal.clone(),
        source_version: state.source_version,
        projection_status: state.projection_status.clone(),
        import_warnings: state.import_warnings.clone(),
        proposals: state.proposals.clone(),
        review_revision: state.review_revision,
        revision_time: state.revision_time.clone(),
        proposal_timeline,
        proposal_timelines,
    }
}

fn connection_event(tx: &Outbox, status: ConnectionStatus, message: Option<&str>) {
    emit(
        tx,
        Event::Connection {
            status,
            message: message.map(str::to_owned),
        },
    );
}

fn connect_runtime(
    directory: &std::path::Path,
    binary: &std::path::Path,
    tx: &Outbox,
    shared: &Arc<Shared>,
    previous: &mut Option<Value>,
) -> Option<RuntimeClient> {
    connection_event(tx, ConnectionStatus::Connecting, None);
    let bootstrap = match RuntimeClient::connect_or_spawn(directory, Some(binary)) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, ConnectionStatus::Disconnected, Some(&error.message));
            return None;
        }
    };
    let state = match bootstrap.get_state() {
        Ok(state) => state,
        Err(error) => {
            connection_event(tx, ConnectionStatus::Disconnected, Some(&error.message));
            return None;
        }
    };
    // Do not retain the binary on the steady-state client: a daemon that
    // disappears must be reported as disconnected until the user reconnects.
    let client = match RuntimeClient::new(directory) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, ConnectionStatus::Disconnected, Some(&error.message));
            return None;
        }
    };
    // Rebuild the viewer after a reconnect even when the canonical snapshot
    // did not change while the transport was down.
    *previous = None;
    publish(state, shared, tx, previous);
    connection_event(tx, ConnectionStatus::Ready, None);
    Some(client)
}

fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    tx: Outbox,
    shared: Arc<Shared>,
) {
    let mut previous = None;
    let mut client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
    // Hold kerfd for as long as the editor is open; it exits on its own
    // once the editor and every other client are gone.
    let _lease = RuntimeClient::new(&directory)
        .ok()
        .map(|lease| lease.hold());
    while !shared.closed.load(Ordering::Relaxed) {
        // What `kerf make` or an agent's own kerf-mcp is making of this project shows as the
        // editor's own makings do.
        media::follow_others_makings(&shared, std::process::id(), &tx);
        let command = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(active_client) = client.as_ref() {
                    match active_client.get_state() {
                        Ok(state) => publish(state, &shared, &tx, &mut previous),
                        Err(error) if error.is_transport() => {
                            let _ = media.try_send(Command::Pause);
                            connection_event(
                                &tx,
                                ConnectionStatus::Disconnected,
                                Some(&error.message),
                            );
                            client = None;
                        }
                        Err(_) => {}
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        // A preview's place among those sent, which a stop goes by.
        let preview_turn =
            matches!(command, Command::Preview { .. }).then(|| shared.preview_turns.take());
        if matches!(command, Command::Reconnect) {
            if client.is_some() {
                let _ = media.try_send(Command::Pause);
            }
            client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
            continue;
        }
        let Some(active_client) = client.clone() else {
            error(&tx, "Runtime is disconnected; reconnect to continue");
            if let (Some(turn), Command::Preview { proposal_id, .. }) = (preview_turn, command) {
                preview_unstarted(&shared, &tx, turn, proposal_id);
            }
            continue;
        };
        let operation = key();
        let force_refresh = matches!(&command, Command::Refresh);
        // Render queue, paperwork files and saved looks answer with their
        // own events, never a state.
        let side = match command {
            Command::Export(request) => {
                let renders: Arc<dyn queue::Renders> = Arc::new(active_client.clone());
                Err(("export", queue::submit(&shared, renders, &request, &tx)))
            }
            Command::ExportCancel { job_id } => Err((
                "export_cancel",
                queue::cancel(&shared, &active_client, &job_id, &tx),
            )),
            Command::RenderQueue => {
                Err(("render_queue", queue::publish(&shared, &active_client, &tx)))
            }
            Command::ExportCaptions(request) => Err((
                "export_captions",
                paperwork::export_captions(&active_client, &directory, &request, &tx),
            )),
            Command::LookSave {
                clip,
                name,
                replace,
            } => Err((
                "look_save",
                looks::save_from(&active_client, &directory, &clip, &name, replace, &tx),
            )),
            command => Ok(command),
        };
        let command = match side {
            Ok(command) => command,
            Err((name, result)) => {
                match result {
                    Ok(()) => {}
                    Err(error) if error.is_transport() => {
                        let _ = media.try_send(Command::Pause);
                        connection_event(&tx, ConnectionStatus::Disconnected, Some(&error.message));
                        client = None;
                    }
                    Err(error) => {
                        queue::refused(&tx, name, &error);
                    }
                }
                continue;
            }
        };
        // What follows a command's state, and the command a refusal names.
        let mut then: Option<Event> = None;
        let mut refusal: Option<&'static str> = None;
        let result = match command {
            Command::Refresh => active_client.get_state(),
            Command::Reload => {
                previous = None;
                active_client.get_state()
            }
            // A shared project copies the file into media/ on another
            // thread, then imports the copy.
            Command::Import { path, media_only }
                if media::needs_copy(&directory, Path::new(&path)) =>
            {
                media::copy_then(&shared, &path, &commands, &tx, media::probe, move |path| {
                    Command::Import { path, media_only }
                });
                continue;
            }
            Command::Import { path, media_only } => active_client.get_state().and_then(|state| {
                if media_only {
                    active_client.add_media_request("ui", &operation, &path, state.revision)
                } else {
                    active_client.import_media_request("ui", &operation, &path, state.revision)
                }
                .map(|(state, _, _)| state)
            }),
            Command::MediaReplace { media, path }
                if media::needs_copy(&directory, Path::new(&path)) =>
            {
                media::copy_then(&shared, &path, &commands, &tx, media::probe, move |path| {
                    Command::MediaReplace { media, path }
                });
                continue;
            }
            Command::MediaMake {
                media,
                confirmed,
                rolls,
            } => {
                let rolls = rolls.unwrap_or(1).clamp(1, 16);
                media::make(
                    &shared,
                    &active_client,
                    &media,
                    confirmed,
                    rolls,
                    &commands,
                    &tx,
                );
                continue;
            }
            Command::CastGet => {
                media::cast(&shared, &active_client, None, &tx);
                continue;
            }
            Command::CastSet { entries } => {
                media::cast(&shared, &active_client, Some(entries), &tx);
                continue;
            }
            Command::MediaCarry { media, confirmed } => {
                recipes::carry(&shared, &active_client, &media, confirmed, &tx);
                continue;
            }
            Command::RecipeGet { media } => {
                recipes::get(&shared, &active_client, &media, &tx);
                continue;
            }
            Command::RecipeEdit(edit) => {
                recipes::edit(&shared, &active_client, &edit, &tx);
                continue;
            }
            Command::WorkflowGet { media, line } => {
                workflows::get(&shared, &active_client, &media, line, &tx);
                continue;
            }
            Command::ShotPicture { clip, of } => {
                tracks::picture(&shared, &active_client, clip, of, &tx);
                continue;
            }
            Command::ShotFollow(asked) => {
                tracks::follow(&shared, &active_client, asked, &tx);
                continue;
            }
            Command::TakesRead { media } => {
                takes::read(&shared, &active_client, media, &tx);
                continue;
            }
            Command::WorkflowEdit(edit) => {
                workflows::edit(&shared, &active_client, &edit, &tx);
                continue;
            }
            Command::SignalFromSound { clip, of } => {
                active_client
                    .get_document(false)
                    .and_then(|(revision, text)| {
                        let project = kerf_runtime::parse_document(&text)?;
                        let read = kerf_shots::signals::from_sound(
                            &project,
                            &shared.directory,
                            &of,
                            &clip,
                            None,
                        )
                        .map_err(|why| kerf_runtime::RuntimeError::new("SIGNAL_FAILED", why))?;
                        active_client.apply_script_request(
                            "ui",
                            &operation,
                            &read.script(&clip),
                            revision,
                        )
                    })
            }
            Command::MediaReplace { media, path } => active_client.get_state().and_then(|state| {
                active_client.replace_media_request("ui", &operation, &media, &path, state.revision)
            }),
            Command::ProjectionImport => active_client.get_state().and_then(|state| {
                active_client.import_projection_request("ui", &operation, state.revision)
            }),
            Command::ProjectionRepair => active_client.repair_projection(),
            Command::LookApply { clip, path } => {
                let opened = active_client
                    .get_document(false)
                    .and_then(|(revision, text)| {
                        kerf_runtime::parse_document(&text).map(|project| (revision, project))
                    });
                match opened {
                    Ok((revision, project)) => {
                        match looks::apply(&directory, &project, &clip, &path) {
                            Ok((ops, event)) => {
                                then = Some(event);
                                refusal = Some("look_apply");
                                active_client.apply_ops_request("ui", &operation, ops, revision)
                            }
                            Err(message) => {
                                looks::refused(&tx, "look_apply", &message);
                                continue;
                            }
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Command::PresetSave { name, replace } => {
                let saved = active_client
                    .get_document(false)
                    .and_then(|(_, text)| kerf_runtime::parse_document(&text))
                    .map_err(|error| error.message)
                    .and_then(|project| presets::export_program(&project, Some(&name)))
                    .and_then(|document| presets::save(&directory, &name, &document, replace));
                match saved {
                    Ok(path) => emit(&tx, Event::PresetSaved { path }),
                    Err(message) => error(&tx, message),
                }
                continue;
            }
            Command::Edit { ops, revision } => {
                active_client.apply_ops_request("ui", &operation, ops, revision)
            }
            Command::Undo { revision } => active_client.undo_request("ui", &operation, revision),
            Command::Redo { revision } => active_client.redo_request("ui", &operation, revision),
            Command::Accept {
                proposal_id,
                revision,
            } => active_client.accept_proposal(&proposal_id, "ui", &operation, revision),
            Command::Reject {
                proposal_id,
                revision,
            } => active_client.reject_proposal_request(&proposal_id, "ui", &operation, revision),
            Command::Forward {
                proposal_id,
                revision,
            } => {
                // Its refusal is said as one, with the clip that stopped it.
                refusal = Some("forward");
                active_client.forward_proposal(&operation, revision, &proposal_id)
            }
            Command::PresetExport { path } => {
                match active_client
                    .get_document(false)
                    .and_then(|(_, text)| kerf_runtime::parse_document(&text))
                {
                    Ok(project) => match presets::export_program(&project, None) {
                        Ok(document) => emit(&tx, Event::PresetExport { path, document }),
                        Err(message) => error(&tx, message),
                    },
                    Err(err) => error(&tx, err.message),
                }
                continue;
            }
            Command::PresetInspect { path } => {
                match KerfMcpRouter::new(&directory, Some(&binary)) {
                    Ok(router) => {
                        let response = router.call("look", json!({"preset":path}));
                        if response["ok"] == true {
                            emit(
                                &tx,
                                Event::PresetInspect {
                                    path,
                                    preset: response["result"].clone(),
                                },
                            );
                        } else {
                            error(
                                &tx,
                                response["error"]["message"]
                                    .as_str()
                                    .unwrap_or("cannot read preset"),
                            );
                        }
                    }
                    Err(message) => error(&tx, message),
                }
                continue;
            }
            Command::PresetApply {
                path,
                bindings,
                texts,
                revision,
            } => {
                match active_client.get_state() {
                    Ok(state) if state.revision == revision => {
                        match KerfMcpRouter::new(&directory, Some(&binary)) {
                            Ok(router) => {
                                let response = router.call(
                                    "edit",
                                    json!({"preset":path,"bindings":bindings,"texts":texts}),
                                );
                                if response["ok"] == true {
                                    emit(
                                        &tx,
                                        Event::PresetApplied {
                                            path,
                                            proposal: response["result"]["proposal"].clone(),
                                        },
                                    );
                                } else {
                                    error(
                                        &tx,
                                        response["error"]["message"]
                                            .as_str()
                                            .unwrap_or("cannot apply preset"),
                                    );
                                }
                            }
                            Err(message) => error(&tx, message),
                        }
                    }
                    Ok(_) => error(
                        &tx,
                        "The project changed while choosing preset media. Apply the preset again.",
                    ),
                    Err(err) => error(&tx, err.message),
                }
                // Publish the same proposal state used by the native review UI.
                active_client.get_state()
            }
            Command::Preview {
                proposal_id,
                start,
                duration,
            } => {
                let turn = preview_turn.expect("taken with the command");
                let range = match (start, duration) {
                    (None, None) => Ok(None),
                    (Some(start), Some(duration)) => {
                        kerf_runtime::RenderRange::new(start, duration)
                            .map(Some)
                            .map_err(|err| err.message)
                    }
                    _ => Err("A preview range needs both start and duration".to_owned()),
                };
                let range = match range {
                    Ok(range) => range,
                    Err(message) => {
                        error(&tx, message);
                        preview_unstarted(&shared, &tx, turn, proposal_id);
                        continue;
                    }
                };
                spawn_render(
                    RenderRequest {
                        quality: "preview".into(),
                        preview: true,
                        proposal: proposal_id,
                        range,
                        sequence: None,
                    },
                    turn,
                    &active_client,
                    &tx,
                    &shared,
                );
                continue;
            }
            _ => continue,
        };
        match result {
            Ok(state) => {
                if force_refresh {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    publish(state, &shared, &tx, &mut previous);
                }
                if let Some(event) = then.take() {
                    emit(&tx, event);
                }
            }
            Err(error) if error.is_transport() => {
                let _ = media.try_send(Command::Pause);
                connection_event(&tx, ConnectionStatus::Disconnected, Some(&error.message));
                client = None;
            }
            Err(runtime_error) => {
                match refusal {
                    Some(name) => {
                        queue::refused(&tx, name, &runtime_error);
                    }
                    None => error(&tx, runtime_error.message),
                }
                if let Ok(state) = active_client.get_state() {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    let _ = media.try_send(Command::Pause);
                    connection_event(
                        &tx,
                        ConnectionStatus::Disconnected,
                        Some("runtime became unavailable"),
                    );
                    client = None;
                }
            }
        }
    }
}

/// A preview that ends before any render of it began: refused, or stopped
/// on its way here. One asked to stop says so, which answers the stop.
fn preview_unstarted(shared: &Shared, tx: &Outbox, turn: u64, proposal: Option<String>) {
    if shared.preview_turns.over(turn) {
        emit(
            tx,
            Event::Preview(Preview {
                status: PreviewStatus::Stopped,
                proposal_id: proposal,
                ..Preview::default()
            }),
        );
    }
}

/// Run the preview `request`, the one of `turn`, on its own thread, one at
/// a time. It plays once rendered, unless the program was edited, the
/// preview stopped, or the bridge closed meanwhile. A stop or an exit sent
/// after it and here before it keeps it from starting.
fn spawn_render(
    request: RenderRequest,
    turn: u64,
    client: &RuntimeClient,
    tx: &Outbox,
    shared: &Arc<Shared>,
) {
    if shared.preview_turns.stopped(turn) {
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    if shared.preview_busy.swap(true, Ordering::SeqCst) {
        error(tx, "A preview is already rendering");
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    let client = client.clone();
    let tx = tx.clone();
    let shared = shared.clone();
    thread::spawn(move || {
        let revision = shared.revision.load(Ordering::SeqCst);
        let result = export(&client, &request, turn, &tx, &shared);
        let rendered = result.as_ref().ok().and_then(|(output, document)| {
            let program = kerf_runtime::parse_document(document).ok()?;
            let project = rendered_preview(&program, output, request.range.as_ref())?;
            Some((project, PathBuf::from(output)))
        });
        // Whether it stopped is settled with the viewer's slot held, which
        // `preview_exit` takes after asking for the stop: an exit sent before
        // this keeps the preview out, and one sent after finds it waiting
        // there and puts the program back.
        let stopped = {
            let slot = shared.project.lock();
            let stopped = shared.preview_turns.over(turn);
            // A preview of a program edited since, one stopped, or one for a
            // closed bridge, is never shown.
            let shown = match (&rendered, slot) {
                (Some((project, file)), Ok(mut slot))
                    if !stopped
                        && !shared.closed.load(Ordering::SeqCst)
                        && shared.revision.load(Ordering::SeqCst) == revision =>
                {
                    let project = MediaProject::Rendered(
                        project.clone(),
                        file.clone(),
                        request.proposal.clone(),
                    );
                    install_project(&shared, &mut slot, project);
                    true
                }
                _ => false,
            };
            if !shown {
                if let Ok((output, _)) = &result {
                    discard_preview(&shared, PathBuf::from(output));
                }
            }
            stopped
        };
        match result {
            Ok(_) => {}
            Err(err) if stopped && err.code == "CANCELLED" => {}
            Err(err) => {
                error(&tx, err.message);
                emit(
                    &tx,
                    Event::Preview(Preview {
                        message: Some(
                            "Preview render failed. Retry after fixing the reported error."
                                .to_owned(),
                        ),
                        ..request.event(PreviewStatus::Required)
                    }),
                );
            }
        }
        // The answer to the stop, however the render ended.
        if stopped {
            emit(&tx, Event::Preview(request.event(PreviewStatus::Stopped)));
        }
        shared.preview_busy.store(false, Ordering::SeqCst);
    });
}

/// Render the program. Returns the output path and the document rendered.
/// Closing the bridge or stopping the preview cancels a preview, and a
/// preview that fails leaves no file behind.
fn export(
    client: &RuntimeClient,
    request: &RenderRequest,
    turn: u64,
    tx: &Outbox,
    shared: &Shared,
) -> Result<(String, String), kerf_runtime::RuntimeError> {
    let preview = request.preview;
    let (job, document, output) = client.start_render_range(
        &request.quality,
        request.range.clone(),
        request.proposal.as_deref(),
        request.sequence.as_deref(),
    )?;
    let cancel = kerf_runtime::render_cancel_path(client.directory(), &job.job_id);
    if preview {
        // The viewer's render, not an output.
        shared.renders.mark_viewer(&job.job_id);
        if let Ok(mut running) = shared.preview_cancel.lock() {
            *running = Some(cancel.clone());
        }
        // The bridge may have closed, or the preview stopped, before it
        // could see this render.
        if shared.closed.load(Ordering::SeqCst) || shared.preview_turns.stopped(turn) {
            cancel_preview(shared);
        }
    }
    let mut running = request.event(PreviewStatus::Running);
    running.progress = Some(0.0);
    emit(tx, Event::Preview(running.clone()));
    let result = kerf_runtime::run_render_job(
        client,
        &job,
        &document,
        &output,
        request.quality == "preview",
        |progress| {
            // Progress is replaceable; never stall rendering behind a busy GUI.
            running.progress = Some(progress);
            let _ = tx.try_send(Event::Preview(running.clone()));
        },
    );
    if preview {
        if let Ok(mut running) = shared.preview_cancel.lock() {
            *running = None;
            // A cancel that came as the render finished outlives the job.
            let _ = std::fs::remove_file(&cancel);
        }
    }
    match result {
        Ok(()) => {
            let path = output.to_string_lossy().into_owned();
            if !preview {
                emit(
                    tx,
                    Event::Preview(Preview {
                        path: Some(path.clone()),
                        progress: Some(1.0),
                        ..request.event(PreviewStatus::Completed)
                    }),
                );
            }
            Ok((path, document))
        }
        Err(err) => {
            if preview {
                discard_preview(shared, output);
            }
            if !(err.code == "CANCELLED" && shared.preview_turns.stopped(turn)) {
                emit(tx, Event::Preview(request.event(PreviewStatus::Failed)));
            }
            Err(err)
        }
    }
}

/// `preview_stop`: stop every preview sent before it, the one rendering
/// and one still on its way to its render alike. Each says `stopped` when
/// it is over; with none to stop the answer comes at once.
///
/// Which previews it stops is settled where the stop is sent, as that is
/// what orders it among them; the rest runs on a thread of its own, since
/// the events queue may be full while the editor is sending.
fn stop_preview(shared: &Arc<Shared>, tx: &Outbox) {
    let answered = shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || stopped_preview(&shared, answered, &tx));
}

/// What follows a stop: the render running is told, and a stop that no
/// preview answers for (`answered`) answers itself.
fn stopped_preview(shared: &Shared, answered: bool, tx: &Outbox) {
    cancel_preview(shared);
    if !answered {
        emit(
            tx,
            Event::Preview(Preview {
                status: PreviewStatus::Stopped,
                ..Preview::default()
            }),
        );
    }
}

/// `preview_exit`: stop every preview sent before it, as `preview_stop`
/// does, and play the program again in place of a rendered preview, which
/// the media worker announces with `viewer_reset`.
fn exit_preview(shared: &Arc<Shared>, tx: &Outbox) {
    shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || exited_preview(&shared, &tx));
}

/// What follows an exit, once its previews are asked to stop.
fn exited_preview(shared: &Shared, tx: &Outbox) {
    cancel_preview(shared);
    let model = shared.model.lock().ok().and_then(|model| model.clone());
    if let (Some(model), Ok(mut slot)) = (model, shared.project.lock()) {
        // A preview that was not stopped in time is waiting in the slot or
        // showing: both are looked at with the slot held, as a preview moves
        // from one to the other and into the slot with it held.
        let showing = shared
            .shown_preview
            .lock()
            .is_ok_and(|shown| shown.is_some());
        let waiting = matches!(*slot, Some(MediaProject::Rendered(..)));
        if showing || waiting {
            // A preview finishing now is of the program as it was.
            shared.revision.fetch_add(1, Ordering::SeqCst);
            install_project(shared, &mut slot, MediaProject::Native(model));
        }
    }
    emit(
        tx,
        Event::Preview(Preview {
            status: PreviewStatus::Exited,
            ..Preview::default()
        }),
    );
}

/// Stop the preview render running, if there is one.
fn cancel_preview(shared: &Shared) {
    if let Ok(running) = shared.preview_cancel.lock() {
        if let Some(marker) = &*running {
            let _ = std::fs::write(marker, b"");
        }
    }
}

/// Put `project` in the media slot. A rendered preview it replaces there was
/// never opened, so its file goes now.
fn install_project(shared: &Shared, slot: &mut Option<MediaProject>, project: MediaProject) {
    if let Some(MediaProject::Rendered(_, file, _)) = slot.replace(project) {
        discard_preview(shared, file);
    }
}

/// Remove the preview the media worker plays and one still waiting for it:
/// the bridge has closed, so nothing will show either.
fn discard_previews(shared: &Shared) {
    let shown = shared
        .shown_preview
        .lock()
        .ok()
        .and_then(|mut shown| shown.take());
    if let Some(file) = shown {
        discard_preview(shared, file);
    }
    let waiting = shared.project.lock().ok().and_then(|mut slot| {
        if matches!(*slot, Some(MediaProject::Rendered(..))) {
            slot.take()
        } else {
            None
        }
    });
    if let Some(MediaProject::Rendered(_, file, _)) = waiting {
        discard_preview(shared, file);
    }
}

/// Remove a preview render nothing shows any more, or keep it for the media
/// worker to try again while it is still open. Only previews come here;
/// exports are the person's.
fn discard_preview(shared: &Shared, file: PathBuf) {
    if !remove_preview(&file) {
        if let Ok(mut stale) = shared.stale_previews.lock() {
            stale.push(file);
        }
    }
}

/// Try the previews not removed yet again; `true` once none are left.
fn sweep_previews(shared: &Shared) -> bool {
    shared.stale_previews.lock().map_or(true, |mut stale| {
        stale.retain(|file| !remove_preview(file));
        stale.is_empty()
    })
}

/// Remove a preview file; `false` while something still holds it open.
fn remove_preview(file: &Path) -> bool {
    match std::fs::remove_file(file) {
        Ok(()) => true,
        Err(err) => err.kind() == std::io::ErrorKind::NotFound,
    }
}

/// A project that plays a rendered preview of `program` in its place: all
/// of it, or the `range` rendered, at its own place on the program.
fn rendered_preview(
    program: &Project,
    output: &str,
    range: Option<&kerf_runtime::RenderRange>,
) -> Option<Project> {
    let sequence = program.sequences.first()?;
    let (start, duration) = match range {
        Some(range) => (
            Time::from_seconds(range.start)?,
            Time::from_seconds(range.duration)?,
        ),
        None => (Time::ZERO, sequence.duration()),
    };
    let id = |text: &str| Id::new(text).ok();
    let media = id("preview")?;
    let clip = |name: &str| {
        Some(Clip::new(
            id(name)?,
            Content::Media {
                media: media.clone(),
                source_in: Time::ZERO,
                speed: Ratio::ONE.into(),
            },
            start,
            duration,
        ))
    };
    let mut picture = Track::new(id("V1")?, TrackKind::Video);
    picture.clips.push(clip("preview-picture")?);
    let mut sound = Track::new(id("A1")?, TrackKind::Audio);
    sound.clips.push(clip("preview-sound")?);
    Some(Project {
        name: program.name.clone(),
        media: vec![Media {
            id: media,
            path: output.to_owned(),
            rate: sequence.rate,
            drop_frame: false,
            start: Time::ZERO,
            duration,
            video: Some(VideoInfo {
                width: sequence.width,
                height: sequence.height,
                alpha: false,
                ..Default::default()
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48_000,
            }),
            reel: None,
            recipe: None,
            steps: Vec::new(),
            sync: None,
        }],
        sequences: vec![Sequence {
            id: id("program")?,
            name: String::new(),
            tracks: vec![picture, sound],
            markers: Vec::new(),
            start: Time::ZERO,
            ..sequence.clone()
        }],
        retired: Default::default(),
    })
}

fn dimension(value: f64) -> u32 {
    value.round().clamp(2.0, u32::MAX as f64) as u32
}

/// The viewer frame: the sequence canvas, with its longest edge at most
/// 1920 pixels.
fn preview_frame_size(project: &Project) -> (u32, u32) {
    let (width, height) = project.sequences.first().map_or((1920, 1080), |sequence| {
        (sequence.width.max(2), sequence.height.max(2))
    });
    let scale = (1920.0 / f64::from(width.max(height))).min(1.0);
    (
        dimension(f64::from(width) * scale),
        dimension(f64::from(height) * scale),
    )
}

/// With `KERF_PLAYBACK_STATS` set, a line on stderr every 5 seconds of playing: frames shown and
/// dropped, and the size they came at, for measuring playback in the real editor.
struct PlaybackStats {
    on: bool,
    since: Instant,
    shown: u64,
    dropped: u64,
    dropped_before: u64,
    size: (u32, u32),
}

impl PlaybackStats {
    fn new() -> Self {
        Self {
            on: std::env::var_os("KERF_PLAYBACK_STATS").is_some(),
            since: Instant::now(),
            shown: 0,
            dropped: 0,
            dropped_before: 0,
            size: (0, 0),
        }
    }

    fn frame(&mut self, width: u32, height: u32) {
        self.shown += 1;
        self.size = (width, height);
    }

    fn clock(&mut self, dropped: u64) {
        self.dropped = dropped;
        if !self.on || self.since.elapsed() < Duration::from_secs(5) {
            return;
        }
        let spent = self.since.elapsed().as_secs_f64();
        eprintln!(
            "playback: {:.1} shown/s, {:.1} dropped/s, {}x{}",
            self.shown as f64 / spent,
            dropped.saturating_sub(self.dropped_before) as f64 / spent,
            self.size.0,
            self.size.1
        );
        self.shown = 0;
        self.dropped_before = dropped;
        self.since = Instant::now();
    }
}

fn media_worker(rx: Receiver<Command>, tx: Outbox, shared: Arc<Shared>) {
    let mut stats = PlaybackStats::new();
    let mut session: Option<PlaybackSession> = None;
    let mut position = 0.0_f64;
    let mut playing = false;
    let mut volume = 1.0_f32;
    let mut swept = Instant::now();
    // The tracks heard alone, kept across sessions; the program's own
    // sessions only, as a rendered preview has none of its tracks.
    let mut solo: Vec<Id> = Vec::new();
    let mut native = false;
    let mut meter = meter::Meter::default();
    while !shared.closed.load(Ordering::Relaxed) {
        for command in rx.try_iter() {
            let handle = session.as_ref().map(PlaybackSession::handle);
            let result = match command {
                Command::Solo { tracks } => {
                    solo = tracks
                        .iter()
                        .filter_map(|track| Id::new(track).ok())
                        .collect();
                    handle
                        .filter(|_| native)
                        .map(|handle| handle.set_solo(solo.clone()))
                }
                Command::Play => {
                    playing = true;
                    handle.map(|handle| handle.play())
                }
                Command::Pause => {
                    playing = false;
                    handle.map(|handle| handle.pause())
                }
                Command::Volume { value } => {
                    volume = value;
                    handle.map(|handle| handle.set_volume(value))
                }
                _ => None,
            };
            if let Some(Err(err)) = result {
                error(&tx, err);
            }
        }
        // What waits in the slot becomes what is shown with the slot held,
        // so `preview_exit` finds a preview in one place or the other.
        let taken = shared.project.lock().ok().and_then(|mut slot| {
            // The proposal a rendered preview shows.
            let (project, file, shown_proposal) = match slot.take()? {
                MediaProject::Native(project) => (project, None, None),
                MediaProject::Rendered(project, file, proposal) => (project, Some(file), proposal),
            };
            let rendered = file.is_some();
            let old = shared
                .shown_preview
                .lock()
                .ok()
                .and_then(|mut shown| std::mem::replace(&mut *shown, file));
            Some((project, rendered, shown_proposal, old))
        });
        if let Some((project, rendered, shown_proposal, old)) = taken {
            native = !rendered;
            // Close the old session before the preview it played goes.
            drop(session.take());
            if let Some(old) = old {
                discard_preview(&shared, old);
            }
            if let Ok(mut frame) = shared.frame.lock() {
                *frame = None;
            }
            emit(&tx, Event::ViewerReset);
            emit(
                &tx,
                Event::Caption {
                    text: String::new(),
                },
            );
            let plan = kerf_playback::preview_plan(&project);
            let message = plan
                .reasons
                .iter()
                .map(|reason| reason.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let composited = !rendered && plan.composited();
            if !rendered && !plan.native_exact() && !composited {
                session = None;
                playing = false;
                emit(&tx, Event::Playback { playing: false });
                emit(
                    &tx,
                    Event::Preview(Preview {
                        status: if plan.unavailable() {
                            PreviewStatus::Unavailable
                        } else {
                            PreviewStatus::Required
                        },
                        message: Some(message),
                        ..Preview::default()
                    }),
                );
                thread::sleep(Duration::from_millis(4));
                continue;
            }
            // The compositor draws a program cuts cannot show, live; what it
            // or an export does not match yet comes along as a note.
            emit(
                &tx,
                Event::Preview(if composited {
                    Preview {
                        status: PreviewStatus::Live,
                        message: Some(message),
                        ..Preview::default()
                    }
                } else if let Some(proposal) = &shown_proposal {
                    Preview {
                        status: PreviewStatus::Ready,
                        proposal_id: Some(proposal.clone()),
                        ..Preview::default()
                    }
                } else {
                    Preview {
                        status: PreviewStatus::Ready,
                        ..Preview::default()
                    }
                }),
            );
            let size = preview_frame_size(&project);
            // A new worker prevents old-project frames from resurfacing after an edit.
            session = match PlaybackSession::open(
                project,
                PlaybackConfig::default()
                    .with_frame_size(size.0, size.1)
                    .with_composited(composited),
            ) {
                Ok(session) => {
                    let _ = session.handle().set_volume(volume);
                    if native && !solo.is_empty() {
                        let _ = session.handle().set_solo(solo.clone());
                    }
                    let _ = session.handle().seek(position);
                    if playing {
                        let _ = session.handle().play();
                    }
                    Some(session)
                }
                Err(err) => {
                    error(&tx, err);
                    None
                }
            };
        }
        if let Some(session) = &session {
            let handle = session.handle();
            let seek = shared.seek.lock().ok().and_then(|mut slot| slot.take());
            if let Some(seek) = seek {
                // Where the viewer was asked to be, before the session says
                // so itself: an edit that replaces this session first opens
                // the next one here, not where the last frame was.
                position = seek.position;
                // Keep the picture that is already on screen. The UI drops a
                // frame only when a newer one is closer to the pointer, so
                // clearing this slot just freezes the viewer during a scrub.
                let result = if seek.exact {
                    handle.seek(seek.position)
                } else {
                    handle.scrub(seek.position)
                };
                if let Err(err) = result {
                    error(&tx, err);
                }
            }
            for event in session.drain_events() {
                match event {
                    PlaybackEvent::Frame(frame) => {
                        stats.frame(frame.width, frame.height);
                        position = frame.timeline_time;
                        let caption = frame
                            .captions
                            .iter()
                            .map(|cue| cue.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let _ = tx.try_send(Event::Caption { text: caption });
                        if let Ok(mut slot) = shared.frame.lock() {
                            *slot = Some(frame);
                        }
                        tx.ring();
                    }
                    PlaybackEvent::Clock(clock) => {
                        stats.clock(clock.dropped);
                        position = clock.position;
                        let _ = tx.try_send(Event::Clock {
                            position: clock.position,
                            duration: clock.duration,
                            dropped: clock.dropped,
                        });
                    }
                    PlaybackEvent::StateChanged(state) => {
                        playing = state == PlaybackState::Playing;
                        emit(&tx, Event::Playback { playing });
                    }
                    PlaybackEvent::Error(err) => error(&tx, err),
                    PlaybackEvent::Ended => emit(&tx, Event::Playback { playing: false }),
                    PlaybackEvent::Opened { duration, .. } => {
                        position = position.min(duration);
                        emit(
                            &tx,
                            Event::Clock {
                                position,
                                duration,
                                dropped: 0,
                            },
                        );
                    }
                }
            }
        }
        meter.tick(
            Instant::now(),
            playing && session.is_some(),
            || session.as_ref().and_then(PlaybackSession::levels),
            &tx,
        );
        if swept.elapsed() >= Duration::from_millis(250) {
            sweep_previews(&shared);
            swept = Instant::now();
        }
        thread::sleep(if session.is_some() {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(4)
        });
    }
    drop(session);
    discard_previews(&shared);
    // A playback worker that is shutting down may hold its file open a
    // little longer.
    for _ in 0..40 {
        if sweep_previews(&shared) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
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

fn native_agent(
    shared: &Arc<Shared>,
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &Outbox,
) -> Result<(AgentSession, KerfMcpRouter), String> {
    // Kerf Cloud speaks the Anthropic Messages protocol with the account token.
    let cloud = if settings.cloud {
        let store = kerf_cloud::Store::default_path()
            .map(|path| cloud_session::load(&path, &secrets::SystemKeychain::new()))
            .unwrap_or_default();
        Some(kerf_cloud::model_credentials(&store).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let provider = if cloud.is_some() {
        ProviderName::Anthropic
    } else if let Some(provider) = settings.provider.and_then(ChatProvider::api) {
        provider
    } else {
        match std::env::var("KERF_PROVIDER").ok().as_deref() {
            Some(name) => {
                ProviderName::parse(name).ok_or("KERF_PROVIDER must be openai or anthropic")?
            }
            None if settings
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()) =>
            {
                // A key entered in the UI without an explicit provider defaults to
                // OpenAI, matching the existing environment fallback.
                ProviderName::Openai
            }
            None if std::env::var_os("OPENAI_API_KEY").is_some() => ProviderName::Openai,
            _ => ProviderName::Anthropic,
        }
    };
    let key_name = if provider == ProviderName::Openai {
        "OPENAI_API_KEY"
    } else {
        "ANTHROPIC_API_KEY"
    };
    let api_key = cloud
        .as_ref()
        .map(|(_, token)| token.clone())
        .or_else(|| settings.api_key.clone())
        .filter(|key| !key.trim().is_empty())
        .or_else(|| std::env::var(key_name).ok())
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Add an API key in Settings ▸ Chat or set {key_name}"))?;
    let api_mode = match settings.api_mode {
        Some(mode) => mode,
        None => match std::env::var("OPENAI_API_MODE") {
            Ok(name) => {
                OpenAiApiMode::parse(&name).ok_or("OPENAI_API_MODE must be chat or responses")?
            }
            Err(_) => OpenAiApiMode::Chat,
        },
    };
    let reasoning_effort = std::env::var("OPENAI_REASONING_EFFORT")
        .ok()
        .map(|value| ReasoningEffort::parse(&value).ok_or("Invalid OPENAI_REASONING_EFFORT"))
        .transpose()?;
    // The editor's own lease covers it too, but an agent session holds
    // kerfd for as long as it lives wherever it runs.
    let router = KerfMcpRouter::new(directory, Some(binary))?.held();
    // What it makes on its own, where its person lets it, shows as their own making does.
    router.watch_makings(media::agent_makings(shared.clone(), tx.clone()));
    let mut tools = router.agent_tools();
    inbound::add_agent_tools(&mut tools, directory, settings.mcp_enabled, |line| {
        // Connection errors may contain command arguments or server responses.
        // Never forward raw names, paths, or error details to chat activity.
        emit(
            tx,
            Event::Agent(AgentUpdate::Status {
                text: mcp_connection_status(&line).to_owned(),
            }),
        );
    })
    .map_err(|_| {
        "Could not load MCP configuration. Check ~/.kerf/mcp.json and project .kerf/mcp.json."
            .to_owned()
    })?;
    let next = tx.clone();
    tools.insert(
        "suggest".to_owned(),
        AgentTool::new(
            "suggest",
            SUGGEST_DOC,
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["steps"],
                "properties": {"steps": {
                    "type": "array", "minItems": 1, "maxItems": MAX_NEXT_STEPS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["label", "request"],
                        "properties": {
                            "label": {"type": "string", "maxLength": 40, "description": "The button: a few words, such as \"Stronger glow\"."},
                            "request": {"type": "string", "description": "What the person asks by pressing it, in their words."}
                        }
                    }
                }}
            }),
            move |input| {
                let steps: Vec<NextStep> = input["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|step| {
                        let label = step["label"].as_str()?.trim();
                        let request = step["request"].as_str()?.trim();
                        (!label.is_empty() && !request.is_empty()).then(|| NextStep {
                            label: label.chars().take(40).collect(),
                            request: request.to_owned(),
                        })
                    })
                    .take(MAX_NEXT_STEPS)
                    .collect();
                if steps.is_empty() {
                    return Err("give one to three steps, each with a label and a request".into());
                }
                let shown = steps.len();
                emit(&next, Event::AgentNext { steps });
                Ok(json!({"shown": shown}))
            },
        ),
    );
    let session = AgentSession::new(AgentSessionOptions {
        provider,
        api_mode,
        reasoning_effort,
        api_key: Some(api_key),
        model: settings
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .or_else(|| std::env::var("KERF_MODEL").ok()),
        base_url: cloud
            .map(|(url, _)| url)
            .or_else(|| settings.base_url.clone())
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                std::env::var(if provider == ProviderName::Openai {
                    "OPENAI_BASE_URL"
                } else {
                    "ANTHROPIC_BASE_URL"
                })
                .ok()
            }),
        system_prompt: Some(format!(
            "{}\n\n{EDITOR_GUIDE}",
            router.agent_system_prompt()
        )),
        tools,
        context: Some(router.agent_context()),
        ..Default::default()
    });
    Ok((session, router))
}

/// The models event: where the person's models file is, whether it is
/// there, and each model it names as `look` lists them. Nothing is run.
fn models_event() -> Event {
    use kerf_engine::recipe::model::Models;
    let file = Models::default_file();
    models_value(Models::read(&file), &file)
}

fn models_value(
    read: kerf_engine::error::Result<kerf_engine::recipe::model::Models>,
    file: &Path,
) -> Event {
    let (models, error) = match read {
        Ok(models) => (kerf_mcp::harness::models_listed(&models), None),
        // A file that does not read: said, and what to mend.
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Event::Models {
        file: file.display().to_string(),
        exists: file.is_file(),
        models,
        error,
    }
}

/// The most next steps the editor shows at once.
const MAX_NEXT_STEPS: usize = 3;

const SUGGEST_DOC: &str = "Offer the person one to three next steps as buttons in the editor. \
Pressing one sends its request to you as if they had typed it.";

/// What the built-in agent does in the editor beyond the shared guide.
const EDITOR_GUIDE: &str = "# In the editor\n\n\
The person works in Kerf's editor and answers with buttons more easily than \
by typing. End each turn by calling `suggest` with one to three next steps \
they are likely to want: a variation on what you just did (another effect or \
transition for that moment, stronger or subtler), the next part of the edit, \
or checking the result. Labels are a few words; requests are what the person \
would say. Do not repeat a step they just took.\n\n\
The timeline shows your proposal as it grows, so they watch you work. Build a \
longer edit in steps: stage the first part with `edit`, check it, then `extend` \
the proposal with the next part, rather than one large script at the end.\n\n\
Asked for a vertical or square cut, set the format first in the same proposal \
(`sequence set s1 size=1080x1920`) and frame footage of another shape to fill it; \
the picture takes the proposal's format as it grows.";

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

/// Carry out a Kerf Cloud command, if any, then report the account:
/// `{type: "cloud", access, account?, message?}`.
fn cloud_status(tx: &Outbox, command: Option<Command>) {
    let Some(path) = kerf_cloud::Store::default_path() else {
        emit(
            tx,
            Event::Cloud {
                access: CloudAccess::Unavailable,
                account: None,
                message: None,
            },
        );
        return;
    };
    // The token in the system's keychain, else in the file (cloud_session.rs).
    let keychain = secrets::SystemKeychain::new();
    let mut message = None;
    let kept = match command {
        Some(Command::CloudSignIn { token }) => {
            cloud_session::keep_token(&path, &keychain, Some(token.trim()))
        }
        Some(Command::CloudSignOut) => cloud_session::keep_token(&path, &keychain, None),
        _ => Ok(()),
    };
    if let Err(error) = kept {
        message = Some(error.to_string());
    }
    let store = cloud_session::load(&path, &keychain);
    let (access, account) = match (store.service_url(), store.token()) {
        (None, _) => (CloudAccess::Unavailable, None),
        (Some(_), None) => (CloudAccess::SignedOut, None),
        (Some(url), Some(token)) => match kerf_cloud::fetch_account(&url, token) {
            Ok(account) => (kerf_cloud::Access::of(&account), Some(account)),
            Err(kerf_cloud::CloudError::Unauthorized) => {
                let _ = cloud_session::keep_token(&path, &keychain, None);
                message = Some(kerf_cloud::CloudError::Unauthorized.to_string());
                (CloudAccess::SignedOut, None)
            }
            // Keep the session; the service may be briefly unreachable.
            Err(error) => {
                message = Some(error.to_string());
                (CloudAccess::Unreachable, None)
            }
        },
    };
    emit(
        tx,
        Event::Cloud {
            access,
            account,
            message,
        },
    );
}

fn mcp_connection_status(line: &str) -> &'static str {
    if line.starts_with("mcp: connected ") {
        "MCP server connected"
    } else if line == inbound::TRACKED_PROJECT_CONFIG {
        "Skipped the project's .kerf/mcp.json: Git tracks it, so it came with the repository \
         rather than from you. Untrack it (git rm --cached .kerf/mcp.json) to use its servers."
    } else {
        "MCP server unavailable; skipped. Check its local command and configuration."
    }
}

/// Make `request.versions` versions of a request, each agent on its own thread.
fn run_versions(
    directory: &std::path::Path,
    binary: &std::path::Path,
    request: &AgentRequest,
    tx: &Outbox,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let count = request.versions.min(MAX_VERSIONS);
    thread::scope(|scope| {
        for index in 0..count {
            scope.spawn(move || {
                run_version(
                    directory, binary, request, index, count, tx, commands, shared,
                )
            });
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
        if let Ok(mut aborts) = shared.version_aborts.lock() {
            aborts.push(session.abort_handle());
        }
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

fn agent_worker(
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
            if let Ok(mut aborts) = shared.version_aborts.lock() {
                aborts.clear();
            }
            emit(
                &tx,
                Event::AgentState {
                    state: AgentState::Completed,
                    message: None,
                },
            );
            shared.agent_busy.store(false, Ordering::SeqCst);
            emit(&tx, Event::AgentBusy { busy: false });
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
        if let (Err(err), Some(id)) = (&result, &conversation) {
            if err.starts_with(RESTORED_FAILED) && !turn_events.heard {
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
                    command: Some("agent".to_owned()),
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
        if let Ok(mut abort) = shared.abort.lock() {
            *abort = None;
        }
        shared.agent_busy.store(false, Ordering::SeqCst);
        emit(&tx, Event::AgentBusy { busy: false });
        let _ = commands.send(Command::Refresh);
    }
}

/// How a turn on a session gone on from what was kept fails, so that it is
/// tried once more on a fresh one.
const RESTORED_FAILED: &str = "\u{0}restored: ";

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
    if let Ok(mut abort) = shared.abort.lock() {
        *abort = Some(agent.abort_handle());
    }
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

impl Bridge {
    /// Open the project in `project_directory` and start the workers behind it, without
    /// waiting for project I/O: kerfd (`kerfd_binary`, started when none runs for the
    /// project yet) is reached on a worker, which says how that went with `connection` and
    /// `state` events. Never blocks.
    pub fn open(project_directory: &Path, kerfd_binary: &Path) -> Bridge {
        let directory = project_directory.to_path_buf();
        let project_folder = directory.clone();
        let binary = kerfd_binary.to_path_buf();
        // The project's own looks, for its previews; kerfd reports any that do
        // not read.
        let _ = kerf_model::look::install_project_looks(&directory);
        let (commands, rx) = mpsc::sync_channel(32);
        let (media, media_rx) = mpsc::sync_channel(32);
        let (prompts, prompt_rx) = mpsc::sync_channel(1);
        let (collab, collab_rx) = mpsc::sync_channel(16);
        let (conversations, conversations_rx) = mpsc::sync_channel(64);
        let (emit, events) = wake::outbox(256);
        let shared = Arc::new(Shared {
            directory: directory.clone(),
            media_status: Mutex::new(media::MediaStatus::default()),
            copies: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            agent_busy: AtomicBool::new(false),
            preview_busy: AtomicBool::new(false),
            abort: Mutex::new(None),
            version_aborts: Mutex::new(Vec::new()),
            frame: Mutex::new(None),
            seek: Mutex::new(None),
            project: Mutex::new(None),
            model: Mutex::new(None),
            previews: AtomicU64::new(0),
            preview_cancel: Mutex::new(None),
            preview_turns: Default::default(),
            stale_previews: Mutex::new(Vec::new()),
            shown_preview: Mutex::new(None),
            check_busy: AtomicBool::new(false),
            check_cancel: Mutex::new(None),
            makings: Mutex::new(std::collections::BTreeMap::new()),
            others_makings: Mutex::new(std::collections::BTreeMap::new()),
            renders: queue::Queue::default(),
            analysis: analysis::Analysis::default(),
            workflows: Default::default(),
        });
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let commands = commands.clone();
            let media = media.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || {
                runtime_worker(directory, binary, rx, commands, media, tx, shared)
            });
        }
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let runtime = commands.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || collab::run(directory, binary, collab_rx, runtime, tx, shared));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || media_worker(media_rx, tx, shared));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            thread::spawn(move || queue::worker(shared, tx));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            let commands = commands.clone();
            thread::spawn(move || agent_worker(directory, binary, prompt_rx, commands, tx, shared));
        }
        {
            let tx = emit.clone();
            thread::spawn(move || cloud_status(&tx, None));
        }
        {
            // Their events wait for room in the event queue here, never on
            // the caller's thread, which is the one that empties it.
            let tx = emit.clone();
            let store = conversations::Store::of(&project_folder);
            thread::spawn(move || {
                for request in conversations_rx {
                    conversations::command(&store, request, &tx);
                }
            });
        }
        Bridge {
            directory: project_folder,
            events_tx: emit,
            commands,
            media,
            collab,
            prompts,
            conversations,
            events,
            shared,
        }
    }

    /// Queue a command (see [`Command`] for every command). `false` means it was refused: it
    /// cannot be taken as it is, or its bounded queue is full. Show an error then; an edit is
    /// never dropped silently. Never blocks.
    pub fn send(&mut self, command: Command) -> bool {
        match command {
            Command::Seek { position, scrub } => {
                if !position.is_finite() || position < 0.0 {
                    return false;
                }
                // The worker holds this only to take the seek out, and a seek
                // refused for that moment would be lost.
                if let Ok(mut slot) = self.shared.seek.lock() {
                    *slot = Some(PendingSeek {
                        position,
                        exact: !scrub,
                    });
                    true
                } else {
                    false
                }
            }
            Command::Play | Command::Pause | Command::Volume { .. } | Command::Solo { .. } => {
                self.media.try_send(command).is_ok()
            }
            Command::Abort => {
                if let Ok(aborts) = self.shared.version_aborts.lock() {
                    for abort in aborts.iter() {
                        abort.store(true, Ordering::SeqCst);
                    }
                }
                if let Ok(abort) = self.shared.abort.try_lock() {
                    if let Some(abort) = &*abort {
                        abort.store(true, Ordering::SeqCst);
                    }
                    true
                } else {
                    false
                }
            }
            Command::EffectPreviews(request) => {
                let generation = self.shared.previews.fetch_add(1, Ordering::SeqCst) + 1;
                let shared = self.shared.clone();
                let tx = self.events_tx.clone();
                thread::spawn(move || {
                    if let Err(message) = previews::run(&shared, generation, &request, &tx) {
                        error(&tx, message);
                    }
                });
                true
            }
            Command::Check { sequence } => {
                qc::spawn(self.shared.clone(), self.events_tx.clone(), sequence);
                true
            }
            // Not queued behind the runtime worker: a making is stopped when it is asked.
            // What it says waits for room in the event queue on a thread of its own, never on
            // the caller's, which is the one that empties it.
            Command::MediaMakeCancel { media } => {
                let shared = self.shared.clone();
                let tx = self.events_tx.clone();
                thread::spawn(move || media::cancel_make(&shared, &media, &tx));
                true
            }
            Command::Models => {
                let tx = self.events_tx.clone();
                thread::spawn(move || emit(&tx, models_event()));
                true
            }
            Command::Loudness { sequence } => {
                analysis::loudness(self.shared.clone(), self.events_tx.clone(), sequence);
                true
            }
            Command::ScopeSignal { time, sequence } => {
                analysis::scope_signal(self.shared.clone(), self.events_tx.clone(), time, sequence);
                true
            }
            Command::MediaInfo { media } => {
                analysis::media_info(self.shared.clone(), self.events_tx.clone(), media);
                true
            }
            Command::MediaIndex { media } => {
                analysis::media_index(self.shared.clone(), self.events_tx.clone(), media);
                true
            }
            Command::CloudSignIn { .. } | Command::CloudSignOut | Command::CloudRefresh => {
                let tx = self.events_tx.clone();
                thread::spawn(move || cloud_status(&tx, Some(command)));
                true
            }
            // A preview is counted as it is sent, and a stop or an exit acts
            // here rather than behind the runtime worker's queue: it stops the
            // previews sent before it, wherever they have got to.
            Command::Preview { .. } => {
                let commands = &self.commands;
                self.shared
                    .preview_turns
                    .send(|| commands.try_send(command).is_ok())
            }
            Command::PreviewStop => {
                stop_preview(&self.shared, &self.events_tx);
                true
            }
            Command::PreviewExit => {
                exit_preview(&self.shared, &self.events_tx);
                true
            }
            Command::PresetLibrary => {
                let directory = self.shared.directory.clone();
                let tx = self.events_tx.clone();
                thread::spawn(move || emit(&tx, presets::library(&directory)));
                true
            }
            Command::CollabStatus
            | Command::CollabSync { .. }
            | Command::CollabLive { .. }
            | Command::CollabCommit { .. }
            | Command::CollabBranch { .. }
            | Command::CollabPullRequest { .. }
            | Command::CollabPublish { .. }
            | Command::CollabConsolidate => self.collab.try_send(command).is_ok(),
            Command::Agent {
                text,
                focus,
                conversation,
                at,
                permissions,
                settings,
            } => {
                if text.trim().is_empty() || self.shared.agent_busy.swap(true, Ordering::SeqCst) {
                    return false;
                }
                let turn = Some((text.clone(), conversation, at));
                let text = match &focus {
                    Some(focus) => kerf_agent::focused_request(&text, focus),
                    None => text,
                };
                if self
                    .prompts
                    .try_send(AgentRequest {
                        text,
                        settings,
                        versions: 0,
                        turn,
                        permissions: permission_mode(permissions.as_deref()),
                    })
                    .is_err()
                {
                    self.shared.agent_busy.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            }
            Command::AgentVersions {
                text,
                count,
                settings,
            } => {
                if text.trim().is_empty()
                    || count == 0
                    || self.shared.agent_busy.swap(true, Ordering::SeqCst)
                {
                    return false;
                }
                if self
                    .prompts
                    .try_send(AgentRequest {
                        text,
                        settings,
                        versions: count,
                        turn: None,
                        permissions: None,
                    })
                    .is_err()
                {
                    self.shared.agent_busy.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            }
            Command::Conversations(request) => self.conversations.try_send(request).is_ok(),
            _ => self.commands.try_send(command).is_ok(),
        }
    }

    /// The next event (see [`Event`] for every event), or `None` when none is waiting.
    /// Never blocks.
    pub fn poll(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// What rings when an event, a new playback frame or a new signal frame is waiting:
    /// await [`Wake::wait`], then take them with [`Bridge::poll`], [`Bridge::frame`] and
    /// [`Bridge::signal`].
    pub fn wake(&self) -> Wake {
        self.events_tx.wake()
    }

    /// The newest playback frame not taken yet, without copying its pixels; `None` when no
    /// newer one has come. Only the latest frame is kept: one not taken before the next
    /// arrives is never seen.
    pub fn frame(&mut self) -> Option<Frame> {
        let frame = self
            .shared
            .frame
            .try_lock()
            .ok()
            .and_then(|mut frame| frame.take())?;
        frame.has_valid_pixels().then_some(Frame {
            pixels: frame.pixels,
            width: frame.width,
            height: frame.height,
            time: frame.timeline_time,
        })
    }

    /// The newest signal frame `scope_signal` drew and that was not taken yet, for the
    /// Finish tray's scopes, without copying its pixels; `None` when there is no newer one.
    pub fn signal(&mut self) -> Option<Signal> {
        let frame = self
            .shared
            .analysis
            .signal_frame
            .try_lock()
            .ok()
            .and_then(|mut frame| frame.take())?;
        Some(Signal {
            pixels: frame.pixels,
            width: frame.width,
            height: frame.height,
            time: frame.time,
            curve: frame.curve,
        })
    }

    /// The project folder the bridge was opened on.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Bridge {
    /// Close the bridge and signal the workers to shut down, without blocking the editor.
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        // Nobody would see a preview still rendering, or the one playing. The
        // process may exit before the media worker stops, so the files go now;
        // one a playback worker still holds open is left for the media worker.
        cancel_preview(&self.shared);
        discard_previews(&self.shared);
        qc::cancel(&self.shared);
        // A making nobody would see the end of: its program is asked to end.
        media::cancel_makings(&self.shared);
        self.shared.renders.close(&self.shared.directory);
        self.shared.analysis.close();
        if let Ok(abort) = self.shared.abort.lock() {
            if let Some(abort) = &*abort {
                abort.store(true, Ordering::SeqCst);
            }
        };
        if let Ok(aborts) = self.shared.version_aborts.lock() {
            for abort in aborts.iter() {
                abort.store(true, Ordering::SeqCst);
            }
        };
    }
}

/// A playback frame: RGBA8, `width * height * 4` bytes.
#[derive(Clone)]
pub struct Frame {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Where it is on the program, in seconds.
    pub time: f64,
}

/// An HDR sequence's frame as its signal, for the Finish tray's scopes: 16-bit
/// little-endian RGBA in BT.2020, `width * height * 8` bytes.
#[derive(Clone)]
pub struct Signal {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Where it is on its sequence, in seconds.
    pub time: f64,
    /// 1 for PQ, 2 for HLG.
    pub curve: u32,
}

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

/// The FFmpeg Kerf runs, as kerfd finds it (`kerf_engine::ffmpeg::resolve_ffmpeg_bin`): the
/// first that runs of `KERF_FFMPEG`, the one beside the program (where each package puts it,
/// the macOS app's Contents/MacOS among them), Homebrew's ffmpeg-full, then `ffmpeg` on the
/// PATH; `ffmpeg` when none runs, to say so as it fails. Found once, by running each with
/// `-version`, so call it off the UI thread the first time.
pub fn ffmpeg_bin() -> &'static str {
    kerf_engine::ffmpeg::resolve_ffmpeg_bin()
}

// ---- Kerf's tools, served by the editor ------------------------------------------

/// Serve Kerf's tools over MCP on stdin and stdout, as `kerf-mcp` does, when `args` (the
/// program's, without its name) begin with [`SERVE_MCP`]: `kerf-editor --serve-mcp --dir
/// <project>`. Claude Code is given the editor to run this way where no `kerf-mcp` is found.
/// Returns the status to exit with, or `None` when the arguments ask for no such thing and
/// the editor should start.
pub fn serve_mcp(args: &[String]) -> Option<i32> {
    let directory = served_directory(args)?;
    kerf_engine::sidecar::forward_termination_signals();
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

/// Report `message` now as what ends the editor, before it aborts; the abort then adds no
/// second report. Nothing is written before [`crash_install`].
pub fn crash_fatal(message: &str) {
    let _ = kerf_runtime::crash::fatal(message);
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

// ---- project launcher ------------------------------------------------------------

/// What the launcher shows of a project folder, before any bridge exists; synchronous and
/// local: `{path}` -> `{path, exists, project (has project.kerf), name, repository, branch,
/// remote, modified (ISO 8601 of project.kerf), presets (how many)}`.
pub fn project_inspect(request: &Value) -> Value {
    tasks::inspect_project(request)
}

/// A preset file's slots, and how files would fill them: `{path, files?: [path]}` -> `{name,
/// slots: [{id, seconds, video, audio, clips}], texts: {clipId: text}, files: [{path,
/// seconds, video, audio, fits: [slotId], error?}]}` or `{error}`. It probes the files with
/// ffprobe, so it may take a moment.
pub fn preset_inspect(request: &Value) -> Value {
    tasks::inspect_preset(request)
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

/// Resolve `{command, params, context}` to `{request}`, the command for [`Bridge::send`]
/// that carries it out, or `{error}`.
pub fn commands_run(invocation: &Value) -> Value {
    let id = invocation["command"].as_str().unwrap_or_default();
    let context = command_context(&invocation["context"]);
    match kerf_commands::run(id, &context, &invocation["params"]) {
        Ok(request) => json!({"request": request}),
        Err(error) => json!({"error": error}),
    }
}

/// The picture effects and transitions the editor offers, as a JSON array of `{id, kind,
/// category, name, doc, engine, params}`.
pub fn fx_catalog() -> Value {
    kerf_runtime::fx_catalog_json()
}

/// Everything the catalog offers, as a JSON array of `{id, kind, domain, category, name,
/// doc, engine, params}`: picture effects and transitions as [`fx_catalog`] lists them, plus
/// generators (`kind: "generator"`, such as titles and solids) and sound effects (`domain:
/// "audio"`).
pub fn fx_catalog_all() -> Value {
    kerf_runtime::fx_catalog_all_json()
}

/// The properties a clip has by its track's type, which [`fx_catalog_all`] does not list:
/// `{video: [...], audio: [...]}`, as a timeline's `properties` carries them.
pub fn clip_properties() -> Value {
    kerf_runtime::properties_json()
}

#[cfg(test)]
mod tests {
    use super::{discard_previews, rendered_preview, Command, MediaProject, Shared};

    use serde_json::json;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    /// The registry (kerf-commands) and the bridge agree: each request the registry sends for
    /// an engine command reads as a bridge command, as `bridge.h` documented it once.
    #[test]
    fn every_engine_request_is_a_bridge_command() {
        let context = json!({
            "connected": true,
            "revision": 7,
            "playheadFrame": 48,
            "selection": {"clip": "c1", "track": "video", "startFrame": 24, "durationFrames": 48},
            "collab": {
                "repository": true,
                "branch": "main",
                "upstream": "origin/main",
                "remote": "git@github.com:jun/film.git",
                "changed": true,
                "branches": ["main", "color"],
                "externalMedia": 1
            }
        });
        let invocations = [
            ("file.refresh", serde_json::Value::Null),
            ("media.replace", json!({"media": "m1", "path": "a.mov"})),
            ("media.make", json!({"media": "m1"})),
            ("media.makeCancel", json!({"media": "m1"})),
            ("preset.save", json!({"name": "Intro"})),
            ("collab.status", serde_json::Value::Null),
            ("collab.sync", serde_json::Value::Null),
            ("collab.live", json!({"enabled": true})),
            ("collab.commit", json!({"message": "m"})),
            ("collab.newBranch", json!({"name": "b"})),
            ("collab.pullRequest", json!({"title": "t"})),
            (
                "collab.publish",
                json!({"remote": "git@example.com:film.git"}),
            ),
            ("collab.consolidate", json!({})),
            ("qc.run", serde_json::Value::Null),
            ("export.master", serde_json::Value::Null),
            ("proposal.forward", json!({"proposal": "p1"})),
        ];
        for (id, params) in invocations {
            let answer =
                super::commands_run(&json!({"command": id, "context": context, "params": params}));
            let request = &answer["request"];
            assert!(request.is_object(), "{id} sends no request: {answer}");
            assert!(
                <Command as serde::Deserialize>::deserialize(request).is_ok(),
                "the bridge does not take {request} for {id}"
            );
        }
        for documented in [
            json!({"type": "check"}),
            json!({"type": "check", "sequence": "s1"}),
            json!({"type": "projection_import"}),
            json!({"type": "projection_repair"}),
        ] {
            assert!(
                <Command as serde::Deserialize>::deserialize(&documented).is_ok(),
                "the bridge does not take {documented}"
            );
        }
    }

    fn shared() -> Shared {
        crate::tests_support::shared(&std::env::temp_dir())
    }

    #[test]
    fn mcp_connection_activity_omits_untrusted_details() {
        assert_eq!(
            super::mcp_connection_status("mcp: connected secret (2 tools)"),
            "MCP server connected"
        );
        let status = super::mcp_connection_status(
            "warning: mcp server 'secret' unavailable (token=secret), skipping",
        );
        assert!(status.contains("unavailable"));
        assert!(!status.contains("secret"));
        let skipped = super::mcp_connection_status(kerf_mcp::inbound::TRACKED_PROJECT_CONFIG);
        assert!(
            skipped.contains(".kerf/mcp.json") && skipped.contains("Git tracks it"),
            "{skipped}"
        );
    }

    /// A rendered preview of a one-cut program, playing `file`.
    fn rendered(file: &Path) -> MediaProject {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n",
        )
        .unwrap();
        let project = rendered_preview(&program, file.to_str().unwrap(), None).unwrap();
        MediaProject::Rendered(project, file.to_owned(), None)
    }

    #[test]
    fn closing_removes_the_previews_without_waiting_for_the_media_worker() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let playing = directory.path().join("render-1.mp4");
        let waiting = directory.path().join("render-2.mp4");
        std::fs::write(&playing, b"preview").unwrap();
        std::fs::write(&waiting, b"preview").unwrap();
        *shared.shown_preview.lock().unwrap() = Some(playing.clone());
        *shared.project.lock().unwrap() = Some(rendered(&waiting));

        discard_previews(&shared);
        assert!(!playing.exists() && !waiting.exists());
        assert!(shared.shown_preview.lock().unwrap().is_none());
        assert!(shared.project.lock().unwrap().is_none());
    }

    /// The next event as JSON, from a thread that may not have run yet.
    fn next(rx: &mpsc::Receiver<super::Event>) -> serde_json::Value {
        serde_json::to_value(rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap()
    }

    fn preview_request(proposal: &str) -> super::RenderRequest {
        super::RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: Some(proposal.into()),
            range: None,
            sequence: None,
        }
    }

    #[test]
    fn stopping_with_no_preview_running_answers_at_once() {
        let shared = Arc::new(shared());
        let (tx, rx) = crate::wake::outbox(4);
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));

        // With one running, the render answers once it has stopped.
        let directory = tempfile::tempdir().unwrap();
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        assert!(shared.preview_turns.send(|| true));
        let turn = shared.preview_turns.take();
        super::stop_preview(&shared, &tx);
        crate::tests_support::wait(Duration::from_secs(10), || marker.exists().then_some(()))
            .expect("the render was told to stop");
        assert!(shared.preview_turns.stopped(turn));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        // It is over, stopped, and the next stop has nothing to wait for.
        assert!(shared.preview_turns.over(turn));
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
    }

    /// The editor sends `preview` and then `preview_stop`; the stop acts
    /// where it is sent and the preview waits for the runtime worker, so the
    /// stop is first. The preview must not start, and answers the stop.
    #[test]
    fn a_stop_that_gets_ahead_of_its_preview_keeps_it_from_starting() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let client = kerf_runtime::RuntimeClient::new(directory.path()).unwrap();
        let (tx, rx) = crate::wake::outbox(8);
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-7"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-7"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "one stop, one answer"
        );

        // The same with an exit, which also answers for itself.
        assert!(shared.preview_turns.send(|| true));
        super::exit_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "exited"}));
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-8"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-8"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );

        // A preview the runtime worker refuses answers the stop too.
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::preview_unstarted(&shared, &tx, turn, None);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
