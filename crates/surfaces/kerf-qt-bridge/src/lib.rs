//! Qt's nonblocking, UTF-8 C ABI. Canonical edits stay in kerfd; media frames
//! travel through a latest-frame slot rather than JSON or an accumulating queue.
mod analysis;
mod claude_code;
mod cloud_session;
mod collab;
mod looks;
mod media;
mod meter;
mod paperwork;
mod presets;
mod previews;
mod qc;
mod secrets;
mod updates;
mod queue;
mod review;
mod tasks;
#[cfg(test)]
mod tests_support;

use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTool, OpenAiApiMode, ProviderName,
    ReasoningEffort,
};
use kerf_mcp::inbound;
use kerf_mcp::KerfMcpRouter;
use kerf_model::{
    AudioInfo, Clip, Content, Id, Media, Project, Ratio, Sequence, Time, Track, TrackKind,
    VideoInfo,
};
use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use kerf_runtime::{RuntimeClient, StateSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct AgentSettings {
    #[serde(default)]
    provider: Option<ChatProvider>,
    #[serde(rename = "apiMode", default)]
    api_mode: Option<OpenAiApiMode>,
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
    #[serde(rename = "baseUrl", default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "mcpEnabled", default)]
    mcp_enabled: bool,
    #[serde(rename = "mcpConfigVersion", default)]
    mcp_config_version: u64,
    /// Use Kerf Cloud, the hosted model, instead of a key of one's own.
    #[serde(default)]
    cloud: bool,
}

/// Who answers in Kerf chat: a model API with a key, or the person's own
/// Claude Code, signed in to their Claude account.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ChatProvider {
    Anthropic,
    Openai,
    ClaudeCode,
}

impl ChatProvider {
    fn api(self) -> Option<ProviderName> {
        match self {
            Self::Anthropic => Some(ProviderName::Anthropic),
            Self::Openai => Some(ProviderName::Openai),
            Self::ClaudeCode => None,
        }
    }
}

/// The agent behind a chat: Kerf's own loop on a model API, or Claude Code.
enum Agent {
    Native(Box<AgentSession>),
    ClaudeCode(claude_code::ClaudeCodeSession),
}

impl Agent {
    fn abort_handle(&self) -> Arc<AtomicBool> {
        match self {
            Self::Native(session) => session.abort_handle(),
            Self::ClaudeCode(session) => session.abort_handle(),
        }
    }

    fn run_with_events(&mut self, text: &str, emit: impl FnMut(AgentEvent)) -> Result<(), String> {
        match self {
            Self::Native(session) => session
                .run_with_events(text, emit)
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::ClaudeCode(session) => session.run_with_events(text, emit),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Refresh,
    Reconnect,
    /// Render a preview of the program, or of what the pending proposal
    /// `proposalId` would make of it, over `start`..`start + duration` when
    /// given, and play it in place of the program.
    Preview {
        #[serde(rename = "proposalId", default)]
        proposal_id: Option<String>,
        #[serde(default)]
        start: Option<f64>,
        #[serde(default)]
        duration: Option<f64>,
    },
    /// Stop the preview render running: `preview{status:"stopped"}`.
    PreviewStop,
    /// Stop any preview render and play the program again:
    /// `preview{status:"exited"}`, then `viewer_reset`.
    PreviewExit,
    Import {
        path: String,
        /// Add it to the project's media without cutting it into the program.
        #[serde(default, rename = "mediaOnly")]
        media_only: bool,
    },
    /// Typed edits on the model, applied atomically.
    Edit {
        ops: Vec<kerf_runtime::EditOp>,
        revision: u64,
    },
    Undo {
        revision: u64,
    },
    Redo {
        revision: u64,
    },
    Accept {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Reject {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        revision: u64,
    },
    Agent {
        text: String,
        /// The clip, range, or playhead the request is about.
        #[serde(default)]
        focus: Option<kerf_agent::Focus>,
        /// Keep the turn in the review log: the request as the person's
        /// comment, and Kerf's answer as a reply.
        #[serde(default)]
        comment: Option<review::TurnComment>,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    /// The same request made by several agents at once, each staging its own
    /// proposal in its own thread: `agent_version` events follow each one.
    AgentVersions {
        text: String,
        #[serde(default = "default_versions")]
        count: usize,
        #[serde(flatten)]
        settings: AgentSettings,
    },
    Abort,
    Play,
    Pause,
    Seek {
        position: f64,
        /// Pointer is still moving. The playback worker shows the nearest
        /// picture immediately; a later seek without this flag is exact.
        #[serde(default)]
        scrub: bool,
    },
    Volume {
        value: f32,
    },
    /// Queue a render: `export {status: queued, jobId, …}`, then its
    /// turn on the render queue's thread.
    Export(queue::ExportRequest),
    /// Take a queued render out of the queue, or stop the one rendering.
    ExportCancel {
        #[serde(rename = "jobId")]
        job_id: String,
    },
    /// The render jobs kerfd keeps: `render_queue`.
    RenderQueue,
    /// The review log as a CSV sheet, EDL locators, or sequence markers.
    ExportComments(paperwork::CommentsRequest),
    /// The captions as SRT or WebVTT.
    ExportCaptions(paperwork::CaptionsRequest),
    /// The programme measured to EBU R128: `loudness`.
    Loudness {
        #[serde(default)]
        sequence: Option<String>,
    },
    /// An HDR sequence's frame for the Finish tray's scopes: `scope_signal`.
    ScopeSignal {
        time: f64,
        #[serde(default)]
        sequence: Option<String>,
    },
    /// What the media files say of themselves: `media_info`.
    MediaInfo {
        #[serde(default)]
        media: Option<String>,
    },
    /// Where a media file cuts, pauses and is loudest: `media_index`.
    MediaIndex {
        media: String,
    },
    /// Keep a clip's picture effects as an effects preset: `look_saved`.
    LookSave {
        clip: String,
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Put an effects preset on a clip, as one edit.
    LookApply {
        clip: String,
        path: String,
    },
    PresetExport {
        path: String,
    },
    PresetInspect {
        path: String,
    },
    PresetApply {
        path: String,
        bindings: Value,
        texts: Value,
        revision: u64,
    },
    /// List the project's preset library: `preset_library`.
    PresetLibrary,
    /// Save the program as `presets/<name>.kerfpreset`: `preset_saved`.
    PresetSave {
        name: String,
        #[serde(default)]
        replace: bool,
    },
    /// Swap the file behind a media id, keeping every clip's timing. A
    /// shared project copies the file into `media/` first.
    MediaReplace {
        media: String,
        path: String,
    },
    /// Make a media's file from its recipe: `media_make_plan` and nothing
    /// run unless `confirmed`, which a person's yes to that plan sends.
    MediaMake {
        media: String,
        #[serde(default)]
        confirmed: bool,
    },
    /// Load a `project.kerf` changed outside Kerf as one revision.
    ProjectionImport,
    /// Write the editor's project over a `project.kerf` changed outside Kerf.
    ProjectionRepair,
    // Collaboration over Git, run by the collaboration worker.
    CollabStatus,
    CollabSync {
        #[serde(default)]
        message: Option<String>,
    },
    CollabLive {
        enabled: bool,
        /// Seconds between syncs.
        #[serde(default)]
        interval: Option<f64>,
    },
    CollabCommit {
        #[serde(default)]
        message: String,
    },
    CollabBranch {
        name: String,
        #[serde(default)]
        create: bool,
    },
    CollabPullRequest {
        title: String,
        #[serde(default)]
        body: Option<String>,
    },
    CollabPublish {
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        github: Option<kerf_collab::scaffold::GitHubRepository>,
    },
    CollabConsolidate,
    /// Republish the state with the viewer's media reopened: files changed
    /// on disk under an unchanged document. Sent by the bridge's own
    /// workers only.
    #[serde(skip)]
    Reload,
    /// How `effects` would look on `clip`, on its picture at sequence time
    /// `at`, or a generator alone on black: an `effect_preview` event per
    /// entry that has a picture, `width` pixels wide. A newer request
    /// supersedes it.
    EffectPreviews(previews::Request),
    /// The review log's comments: `comments`.
    CommentList,
    /// Add a note by the person using the editor: `comments`.
    CommentAdd(review::AddRequest),
    /// Resolve, reopen, edit, tag or delete a note: `comments`.
    CommentSet(review::SetRequest),
    /// Kerf's QC pass, its failing findings kept as `qc` comments:
    /// `check`.
    Check {
        #[serde(default)]
        sequence: Option<String>,
    },
    /// Hear only these audio tracks; empty hears them all. Monitoring
    /// only: the project never changes.
    Solo {
        #[serde(default)]
        tracks: Vec<String>,
    },
    /// Keep a Kerf Cloud token and read its account.
    CloudSignIn {
        token: String,
    },
    CloudSignOut,
    CloudRefresh,
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
    /// A `preview` or `export` event about this render.
    fn event(&self, status: &str) -> Value {
        let mut event = json!({
            "type": if self.preview { "preview" } else { "export" },
            "status": status,
        });
        if let Some(proposal) = &self.proposal {
            event["proposalId"] = json!(proposal);
        }
        event
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
    /// A preview render is running, from its request to its end.
    preview_running: AtomicBool,
    /// `preview_stop` or `preview_exit` asked the running preview to stop.
    preview_stopped: AtomicBool,
    /// Preview renders nothing shows any more whose files could not be
    /// removed yet: on Windows a playback worker still shutting down keeps
    /// its file open. The media worker tries them again.
    stale_previews: Mutex<Vec<PathBuf>>,
    /// The preview file the media worker plays, so closing the bridge can
    /// remove it even when the process exits before that worker stops.
    shown_preview: Mutex<Option<PathBuf>>,
    /// The review log revision of the comments last sent to the editor.
    review_seen: Mutex<Option<u64>>,
    /// A QC check is running.
    check_busy: AtomicBool,
    /// Cancels the QC check running, so closing the bridge stops it.
    check_cancel: Mutex<Option<kerf_engine::CancellationToken>>,
    /// The render queue the export thread works through.
    renders: queue::Queue,
    /// Loudness, media info and media index work.
    analysis: analysis::Analysis,
}

struct Bridge {
    events_tx: SyncSender<Value>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    collab: SyncSender<Command>,
    prompts: SyncSender<AgentRequest>,
    events: Receiver<Value>,
    shared: Arc<Shared>,
    displayed_frame: Option<PlaybackFrame>,
    displayed_signal: Option<analysis::SignalFrame>,
}

struct AgentRequest {
    text: String,
    settings: AgentSettings,
    /// More than none: this many agents make their own version of the request.
    versions: usize,
    /// The person's own words and where the turn goes in the review log,
    /// when it is kept there.
    note: Option<(String, review::TurnComment)>,
}

fn emit(tx: &SyncSender<Value>, value: Value) {
    let _ = tx.send(value);
}

fn error(tx: &SyncSender<Value>, value: impl std::fmt::Display) {
    emit(tx, json!({"type":"error", "message":value.to_string()}));
}

fn key() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    format!(
        "qt-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn publish(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, false);
}

fn publish_force(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, true);
}

fn publish_snapshot(
    state: StateSnapshot,
    shared: &Arc<Shared>,
    tx: &SyncSender<Value>,
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
    *previous = Some(snapshot.clone());
    emit(
        tx,
        json!({"type":"state", "snapshot":with_proposal_timeline(snapshot, &state)}),
    );
}

/// Each open proposal's timeline beside the current one, so the editor can
/// show proposals on the timeline before they are accepted:
/// `proposalTimelines` by proposal id (an open proposal's earlier versions
/// too, while they were made on this revision), and `proposalTimeline` for
/// the pending proposal. The list fields a snapshot leaves out while empty
/// are filled in, so the editor always reads the same shape.
fn with_proposal_timeline(mut snapshot: Value, state: &StateSnapshot) -> Value {
    if let Some(kerf_runtime::PendingProposal::Pending { script, .. }) = &state.pending_proposal {
        if let Ok(timeline) = kerf_runtime::proposal_timeline(&state.document, script) {
            snapshot["proposalTimeline"] = timeline;
        }
    }
    let mut timelines = serde_json::Map::new();
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
            if let Ok(timeline) = kerf_runtime::proposal_timeline(&state.document, script) {
                timelines.insert(id.clone(), timeline);
            }
        }
    }
    snapshot["proposalTimelines"] = Value::Object(timelines);
    for (field, empty) in [("proposals", json!([])), ("reviewRevision", json!(0))] {
        if snapshot.get(field).is_none() {
            snapshot[field] = empty;
        }
    }
    snapshot
}

fn connection_event(tx: &SyncSender<Value>, status: &str, message: Option<&str>) {
    let mut event = json!({"type":"connection", "status":status});
    if let Some(message) = message {
        event["message"] = Value::String(message.to_owned());
    }
    emit(tx, event);
}

fn connect_runtime(
    directory: &std::path::Path,
    binary: &std::path::Path,
    tx: &SyncSender<Value>,
    shared: &Arc<Shared>,
    previous: &mut Option<Value>,
) -> Option<RuntimeClient> {
    connection_event(tx, "connecting", None);
    let bootstrap = match RuntimeClient::connect_or_spawn(directory, Some(binary)) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    let state = match bootstrap.get_state() {
        Ok(state) => state,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    // Do not retain the binary on the steady-state client: a daemon that
    // disappears must be reported as disconnected until the user reconnects.
    let client = match RuntimeClient::new(directory) {
        Ok(client) => client,
        Err(error) => {
            connection_event(tx, "disconnected", Some(&error.message));
            return None;
        }
    };
    // Rebuild the viewer after a reconnect even when the canonical snapshot
    // did not change while the transport was down.
    *previous = None;
    let review_revision = state.review_revision;
    publish(state, shared, tx, previous);
    connection_event(tx, "ready", None);
    // The daemon may be a new one whose review counter starts again.
    review::reset(shared);
    let _ = review::follow(&client, review_revision, shared, tx);
    Some(client)
}

fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    tx: SyncSender<Value>,
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
        let command = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(active_client) = client.as_ref() {
                    match active_client.get_state() {
                        Ok(state) => {
                            let review_revision = state.review_revision;
                            publish(state, &shared, &tx, &mut previous);
                            // Notes MCP agents and merges write show up here.
                            let _ = review::follow(active_client, review_revision, &shared, &tx);
                        }
                        Err(error) if error.is_transport() => {
                            let _ = media.try_send(Command::Pause);
                            connection_event(&tx, "disconnected", Some(&error.message));
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
        if matches!(command, Command::Reconnect) {
            if client.is_some() {
                let _ = media.try_send(Command::Pause);
            }
            client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
            continue;
        }
        let Some(active_client) = client.clone() else {
            error(&tx, "Runtime is disconnected; reconnect to continue");
            continue;
        };
        let operation = key();
        let force_refresh = matches!(&command, Command::Refresh);
        // Comment commands answer with `comments`, never a state.
        let command = match command {
            Command::CommentList | Command::CommentAdd(_) | Command::CommentSet(_) => {
                let (name, id, result) =
                    review::command(command, &active_client, &directory, &shared, &tx);
                match result {
                    Ok(()) => {}
                    Err(error) if error.is_transport() => {
                        let _ = media.try_send(Command::Pause);
                        connection_event(&tx, "disconnected", Some(&error.message));
                        client = None;
                    }
                    Err(error) => review::refused(&tx, name, id.as_deref(), &error),
                }
                continue;
            }
            command => command,
        };
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
            Command::ExportComments(request) if request.format != "markers" => Err((
                "export_comments",
                paperwork::export_comments(&active_client, &directory, &request, &tx),
            )),
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
                        connection_event(&tx, "disconnected", Some(&error.message));
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
        let mut then: Option<Value> = None;
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
            Command::MediaMake { media, confirmed } => {
                media::make(&active_client, &directory, &media, confirmed, &commands, &tx);
                continue;
            }
            Command::MediaReplace { media, path } => active_client.get_state().and_then(|state| {
                active_client.replace_media_request("ui", &operation, &media, &path, state.revision)
            }),
            Command::ProjectionImport => active_client.get_state().and_then(|state| {
                active_client.import_projection_request("ui", &operation, state.revision)
            }),
            Command::ProjectionRepair => active_client.repair_projection(),
            Command::ExportComments(request) => {
                match paperwork::markers(&active_client, &request, &tx) {
                    Ok(Some((ops, revision, count))) => {
                        then = Some(paperwork::markers_done(count));
                        refusal = Some("markers");
                        active_client.apply_ops_request("ui", &operation, ops, revision)
                    }
                    Ok(None) => continue,
                    Err(error) => Err(error),
                }
            }
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
                    Ok(path) => emit(&tx, json!({"type":"preset_saved", "path":path})),
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
            Command::PresetExport { path } => {
                match active_client
                    .get_document(false)
                    .and_then(|(_, text)| kerf_runtime::parse_document(&text))
                {
                    Ok(project) => match presets::export_program(&project, None) {
                        Ok(document) => emit(
                            &tx,
                            json!({"type":"preset_export", "path":path, "document":document}),
                        ),
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
                                json!({"type":"preset_inspect", "path":path, "preset":response["result"]}),
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
                                        json!({"type":"preset_applied", "path":path, "proposal":response["result"]["proposal"]}),
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
                let range = match (start, duration) {
                    (None, None) => None,
                    (Some(start), Some(duration)) => {
                        match kerf_runtime::RenderRange::new(start, duration) {
                            Ok(range) => Some(range),
                            Err(err) => {
                                error(&tx, err.message);
                                continue;
                            }
                        }
                    }
                    _ => {
                        error(&tx, "A preview range needs both start and duration");
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
                let review_revision = state.review_revision;
                if force_refresh {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    publish(state, &shared, &tx, &mut previous);
                }
                if let Some(event) = then.take() {
                    emit(&tx, event);
                }
                let _ = review::follow(&active_client, review_revision, &shared, &tx);
            }
            Err(error) if error.is_transport() => {
                let _ = media.try_send(Command::Pause);
                connection_event(&tx, "disconnected", Some(&error.message));
                client = None;
            }
            Err(runtime_error) => {
                match refusal {
                    Some("markers") => paperwork::markers_failed(&tx, &runtime_error.message),
                    Some(name) => {
                        queue::refused(&tx, name, &runtime_error);
                    }
                    None => error(&tx, runtime_error.message),
                }
                if let Ok(state) = active_client.get_state() {
                    publish_force(state, &shared, &tx, &mut previous);
                } else {
                    let _ = media.try_send(Command::Pause);
                    connection_event(&tx, "disconnected", Some("runtime became unavailable"));
                    client = None;
                }
            }
        }
    }
}

/// Run the preview `request` on its own thread, one at a time. It plays
/// once rendered, unless the program was edited, the preview stopped, or
/// the bridge closed meanwhile.
fn spawn_render(
    request: RenderRequest,
    client: &RuntimeClient,
    tx: &SyncSender<Value>,
    shared: &Arc<Shared>,
) {
    if shared.preview_busy.swap(true, Ordering::SeqCst) {
        error(tx, "A preview is already rendering");
        return;
    }
    if request.preview {
        shared.preview_stopped.store(false, Ordering::SeqCst);
        shared.preview_running.store(true, Ordering::SeqCst);
    }
    let client = client.clone();
    let tx = tx.clone();
    let shared = shared.clone();
    thread::spawn(move || {
        let revision = shared.revision.load(Ordering::SeqCst);
        let result = export(&client, &request, &tx, &shared);
        let stopped = request.preview && shared.preview_stopped.swap(false, Ordering::SeqCst);
        match result {
            Ok((output, document)) if request.preview => {
                let rendered = kerf_runtime::parse_document(&document)
                    .ok()
                    .and_then(|program| {
                        rendered_preview(&program, &output, request.range.as_ref())
                    });
                let file = PathBuf::from(output);
                // A preview of a program edited since, one stopped, or one
                // for a closed bridge, is never shown.
                let shown = match (rendered, shared.project.lock()) {
                    (Some(rendered), Ok(mut slot))
                        if !stopped
                            && !shared.closed.load(Ordering::SeqCst)
                            && shared.revision.load(Ordering::SeqCst) == revision =>
                    {
                        let project = MediaProject::Rendered(
                            rendered,
                            file.clone(),
                            request.proposal.clone(),
                        );
                        install_project(&shared, &mut slot, project);
                        true
                    }
                    _ => false,
                };
                if !shown {
                    discard_preview(&shared, file);
                }
                if stopped {
                    emit(&tx, request.event("stopped"));
                }
            }
            Ok(_) => {}
            Err(err) if stopped && err.code == "CANCELLED" => emit(&tx, request.event("stopped")),
            Err(err) => {
                error(&tx, err.message);
                if request.preview {
                    let mut event = request.event("required");
                    event["message"] =
                        json!("Preview render failed. Retry after fixing the reported error.");
                    emit(&tx, event);
                }
            }
        }
        if request.preview {
            shared.preview_running.store(false, Ordering::SeqCst);
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
    tx: &SyncSender<Value>,
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
        if shared.closed.load(Ordering::SeqCst) || shared.preview_stopped.load(Ordering::SeqCst) {
            cancel_preview(shared);
        }
    }
    let mut running = request.event("running");
    running["progress"] = json!(0.0);
    emit(tx, running.clone());
    let result = kerf_runtime::run_render_job(
        client,
        &job,
        &document,
        &output,
        request.quality == "preview",
        |progress| {
            // Progress is replaceable; never stall rendering behind a busy GUI.
            running["progress"] = json!(progress);
            let _ = tx.try_send(running.clone());
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
                let mut event = request.event("completed");
                event["path"] = json!(path);
                event["progress"] = json!(1.0);
                emit(tx, event);
            }
            Ok((path, document))
        }
        Err(err) => {
            if preview {
                discard_preview(shared, output);
            }
            if !(err.code == "CANCELLED" && shared.preview_stopped.load(Ordering::SeqCst)) {
                emit(tx, request.event("failed"));
            }
            Err(err)
        }
    }
}

/// `preview_stop`: stop the preview render running. With none running the
/// answer comes at once.
fn stop_preview(shared: &Shared, tx: &SyncSender<Value>) {
    shared.preview_stopped.store(true, Ordering::SeqCst);
    cancel_preview(shared);
    if !shared.preview_running.load(Ordering::SeqCst) {
        shared.preview_stopped.store(false, Ordering::SeqCst);
        emit(tx, json!({"type":"preview","status":"stopped"}));
    }
}

/// `preview_exit`: stop any preview render, and play the program again in
/// place of a rendered preview, which the media worker announces with
/// `viewer_reset`.
fn exit_preview(shared: &Shared, tx: &SyncSender<Value>) {
    if shared.preview_running.load(Ordering::SeqCst) {
        shared.preview_stopped.store(true, Ordering::SeqCst);
        cancel_preview(shared);
    }
    let showing = shared
        .shown_preview
        .lock()
        .is_ok_and(|shown| shown.is_some());
    let model = shared.model.lock().ok().and_then(|model| model.clone());
    if let (Some(model), Ok(mut slot)) = (model, shared.project.lock()) {
        let waiting = matches!(*slot, Some(MediaProject::Rendered(..)));
        if showing || waiting {
            // A preview finishing now is of the program as it was.
            shared.revision.fetch_add(1, Ordering::SeqCst);
            install_project(shared, &mut slot, MediaProject::Native(model));
        }
    }
    emit(tx, json!({"type":"preview","status":"exited"}));
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

fn media_worker(rx: Receiver<Command>, tx: SyncSender<Value>, shared: Arc<Shared>) {
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
        let project = shared.project.lock().ok().and_then(|mut slot| slot.take());
        if let Some(project) = project {
            // The proposal a rendered preview shows.
            let (project, file, shown_proposal) = match project {
                MediaProject::Native(project) => (project, None, None),
                MediaProject::Rendered(project, file, proposal) => (project, Some(file), proposal),
            };
            let rendered = file.is_some();
            native = !rendered;
            // Close the old session before the preview it played goes.
            drop(session.take());
            let old = shared
                .shown_preview
                .lock()
                .ok()
                .and_then(|mut shown| std::mem::replace(&mut *shown, file));
            if let Some(old) = old {
                discard_preview(&shared, old);
            }
            if let Ok(mut frame) = shared.frame.lock() {
                *frame = None;
            }
            emit(&tx, json!({"type":"viewer_reset"}));
            emit(&tx, json!({"type":"caption","text":""}));
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
                emit(&tx, json!({"type":"playback","playing":false}));
                emit(
                    &tx,
                    json!({"type":"preview","status":if plan.unavailable() { "unavailable" } else { "required" }, "message":message}),
                );
                thread::sleep(Duration::from_millis(4));
                continue;
            }
            // The compositor draws a program cuts cannot show, live; what it
            // or an export does not match yet comes along as a note.
            emit(
                &tx,
                if composited {
                    json!({"type":"preview","status":"live","message":message})
                } else if let Some(proposal) = &shown_proposal {
                    json!({"type":"preview","status":"ready","proposalId":proposal})
                } else {
                    json!({"type":"preview","status":"ready"})
                },
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
                        let _ = tx.try_send(json!({"type":"caption","text":caption}));
                        if let Ok(mut slot) = shared.frame.lock() {
                            *slot = Some(frame);
                        }
                    }
                    PlaybackEvent::Clock(clock) => {
                        stats.clock(clock.dropped);
                        position = clock.position;
                        let _ = tx.try_send(json!({
                            "type": "clock",
                            "position": clock.position,
                            "duration": clock.duration,
                            "dropped": clock.dropped,
                        }));
                    }
                    PlaybackEvent::StateChanged(state) => {
                        playing = state == PlaybackState::Playing;
                        emit(&tx, json!({"type":"playback","playing":playing}));
                    }
                    PlaybackEvent::Error(err) => error(&tx, err),
                    PlaybackEvent::Ended => emit(&tx, json!({"type":"playback","playing":false})),
                    PlaybackEvent::Opened { duration, .. } => {
                        position = position.min(duration);
                        emit(
                            &tx,
                            json!({"type":"clock","position":position,"duration":duration,"dropped":0}),
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
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &SyncSender<Value>,
) -> Result<Agent, String> {
    if !settings.cloud && settings.provider == Some(ChatProvider::ClaudeCode) {
        return claude_code::ClaudeCodeSession::new(directory, binary, settings.model.clone())
            .map(Agent::ClaudeCode);
    }
    native_agent(directory, binary, settings, tx).map(|session| Agent::Native(Box::new(session)))
}

fn native_agent(
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &SyncSender<Value>,
) -> Result<AgentSession, String> {
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
        .ok_or_else(|| format!("Add an API key in Agent Settings or set {key_name}"))?;
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
    let mut tools = router.agent_tools();
    inbound::add_agent_tools(&mut tools, directory, settings.mcp_enabled, |line| {
        // Connection errors may contain command arguments or server responses.
        // Never forward raw names, paths, or error details to chat activity.
        emit(
            tx,
            json!({"type":"agent","kind":"status","text":mcp_connection_status(&line)}),
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
                let steps: Vec<Value> = input["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|step| {
                        let label = step["label"].as_str()?.trim();
                        let request = step["request"].as_str()?.trim();
                        (!label.is_empty() && !request.is_empty()).then(|| {
                            json!({"label": label.chars().take(40).collect::<String>(), "request": request})
                        })
                    })
                    .take(MAX_NEXT_STEPS)
                    .collect();
                if steps.is_empty() {
                    return Err("give one to three steps, each with a label and a request".into());
                }
                emit(&next, json!({"type": "agent_next", "steps": steps}));
                Ok(json!({"shown": steps.len()}))
            },
        ),
    );
    Ok(AgentSession::new(AgentSessionOptions {
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
    }))
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

fn default_versions() -> usize {
    3
}

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
fn focus_event(name: &str, targets: &kerf_agent::AgentTargetSummary) -> Option<Value> {
    (!targets.times.is_empty() || !targets.clips.is_empty()).then(|| {
        json!({
            "type": "agent_focus",
            "tool": name,
            "times": targets.times,
            "clips": targets.clips,
        })
    })
}

/// Whether a finished tool call changed what the timeline shows.
fn staged_proposal(summary: &kerf_agent::ToolResultSummary) -> bool {
    summary.proposal.is_some() || summary.status.as_deref() == Some("awaiting_review")
}

/// Carry out a Kerf Cloud command, if any, then report the account:
/// `{type: "cloud", access, account?, message?}`.
fn cloud_status(tx: &SyncSender<Value>, command: Option<Command>) {
    let Some(path) = kerf_cloud::Store::default_path() else {
        emit(tx, json!({"type":"cloud","access":"unavailable"}));
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
    let event = match (store.service_url(), store.token()) {
        (None, _) => json!({"type":"cloud","access":"unavailable"}),
        (Some(_), None) => json!({"type":"cloud","access":"signedOut"}),
        (Some(url), Some(token)) => match kerf_cloud::fetch_account(&url, token) {
            Ok(account) => json!({
                "type":"cloud",
                "access": kerf_cloud::Access::of(&account),
                "account": account,
            }),
            Err(kerf_cloud::CloudError::Unauthorized) => {
                let _ = cloud_session::keep_token(&path, &keychain, None);
                message = Some(kerf_cloud::CloudError::Unauthorized.to_string());
                json!({"type":"cloud","access":"signedOut"})
            }
            // Keep the session; the service may be briefly unreachable.
            Err(error) => {
                message = Some(error.to_string());
                json!({"type":"cloud","access":"unreachable"})
            }
        },
    };
    let mut event = event;
    if let Some(message) = message {
        event["message"] = json!(message);
    }
    emit(tx, event);
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
    tx: &SyncSender<Value>,
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
    tx: &SyncSender<Value>,
    commands: &SyncSender<Command>,
    shared: &Arc<Shared>,
) {
    let key = VERSION_KEYS[index];
    let title = format!("Version {key}: {}", request.text.trim());
    emit(
        tx,
        json!({"type":"agent_version","key":key,"state":"running","title":title}),
    );
    let finish = |state: &str, proposal: Option<&str>, message: Option<&str>| {
        emit(
            tx,
            json!({
                "type": "agent_version", "key": key, "state": state,
                "proposalId": proposal, "message": message,
            }),
        );
    };
    let client = match RuntimeClient::new(directory) {
        Ok(client) => client,
        Err(err) => {
            finish("failed", None, Some(&err.message));
            return;
        }
    };
    let person = review::person(directory);
    let mut turn = match review::Turn::begin(
        &client,
        person,
        &title,
        review::TurnComment::default(),
        shared,
        tx,
    ) {
        Ok(turn) => turn,
        Err(err) => {
            finish("failed", None, Some(&err.message));
            return;
        }
    };
    emit(
        tx,
        json!({"type":"agent_version","key":key,"state":"running","thread":turn.root()}),
    );
    let mut proposal: Option<String> = None;
    let mut stopped = false;
    let result = (|| {
        let mut session = new_agent(directory, binary, &request.settings, tx)?;
        if let Ok(mut aborts) = shared.version_aborts.lock() {
            aborts.push(session.abort_handle());
        }
        session.run_with_events(
            &version_prompt(&request.text, index, count),
            |event| match event {
                AgentEvent::ToolCallStarted { name, .. } => emit(
                    tx,
                    json!({"type":"agent_version","key":key,"state":"running","status":name}),
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
                AgentEvent::TextDelta(_) | AgentEvent::TurnStarted { .. } => {}
            },
        )?;
        Ok::<_, String>(())
    })();
    let outcome = match &result {
        Ok(()) => review::Outcome::Done(String::new()),
        Err(err) => review::Outcome::Failed(err.clone()),
    };
    if let Err(err) = turn.finish(&client, outcome, proposal.clone(), shared, tx) {
        error(
            tx,
            format!("Kerf's reply was not kept as a note: {}", err.message),
        );
    }
    match result {
        Err(err) => finish("failed", proposal.as_deref(), Some(&err)),
        Ok(()) if stopped => finish("stopped", proposal.as_deref(), None),
        Ok(()) => finish("done", proposal.as_deref(), None),
    }
}

fn agent_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<AgentRequest>,
    commands: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    // Keep one session for follow-up feedback, rather than forgetting every turn.
    let mut session: Option<(AgentSettings, Agent)> = None;
    while let Ok(request) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        emit(&tx, json!({"type":"agent_busy","busy":true}));
        emit(&tx, json!({"type":"agent_state","state":"running"}));
        if request.versions > 0 {
            run_versions(&directory, &binary, &request, &tx, &commands, &shared);
            if let Ok(mut aborts) = shared.version_aborts.lock() {
                aborts.clear();
            }
            emit(&tx, json!({"type":"agent_state","state":"completed"}));
            shared.agent_busy.store(false, Ordering::SeqCst);
            emit(&tx, json!({"type":"agent_busy","busy":false}));
            let _ = commands.send(Command::Refresh);
            continue;
        }
        // A turn kept in the review log: the person's comment first, and
        // the open proposals, to find the one this turn stages.
        let mut kept = request.note.as_ref().and_then(|(prompt, comment)| {
            let client = RuntimeClient::new(&directory).ok()?;
            let before = client
                .get_state()
                .map(|state| review::open_proposals(&state))
                .unwrap_or_default();
            match review::Turn::begin(
                &client,
                review::person(&directory),
                prompt,
                comment.clone(),
                &shared,
                &tx,
            ) {
                Ok(turn) => Some((client, turn, before)),
                Err(err) => {
                    error(
                        &tx,
                        format!("The request was not kept as a note: {}", err.message),
                    );
                    None
                }
            }
        });
        let result = (|| {
            if session
                .as_ref()
                .is_none_or(|(active_settings, _)| active_settings != &request.settings)
            {
                session = Some((
                    request.settings.clone(),
                    new_agent(&directory, &binary, &request.settings, &tx)?,
                ));
            }
            let current = &mut session.as_mut().ok_or("Agent session unavailable")?.1;
            if let Ok(mut abort) = shared.abort.lock() {
                *abort = Some(current.abort_handle());
            }
            current.run_with_events(&request.text, |event| {
                if let AgentEvent::ToolCallStarted { name, targets, .. } = &event {
                    if let Some(focus) = focus_event(name, targets) {
                        emit(&tx, focus);
                    }
                }
                if let AgentEvent::ToolCallFinished { summary, .. } = &event {
                    if summary.status.as_deref() == Some("awaiting_review") {
                        emit(&tx, json!({"type":"agent_state","state":"awaiting_review"}));
                    }
                    // The proposal shows on the timeline as it grows.
                    if staged_proposal(summary) {
                        let _ = commands.try_send(Command::Refresh);
                    }
                }
                // The reply keeps finished tool calls and statuses as
                // its steps, not the planning line every turn opens.
                let step = matches!(
                    event,
                    AgentEvent::ToolCallFinished { .. } | AgentEvent::Aborted
                );
                let (kind, text) = match event {
                    AgentEvent::TextDelta(text) => ("delta", text),
                    AgentEvent::ToolCallStarted { name, targets, .. } => {
                        ("tool", format!("{name} · {}", targets.clips.join(", ")))
                    }
                    AgentEvent::ToolCallFinished {
                        name, ok, summary, ..
                    } => (
                        "tool",
                        format!(
                            "{name} · {}{}",
                            if ok { "done" } else { "failed" },
                            summary
                                .status
                                .map(|status| format!(" · {status}"))
                                .unwrap_or_default(),
                        ),
                    ),
                    AgentEvent::TurnStarted { .. } => ("status", "Planning next step…".into()),
                    AgentEvent::Done { text } => ("done", text),
                    AgentEvent::Aborted => {
                        emit(&tx, json!({"type":"agent_state","state":"aborted"}));
                        ("status", "Stopped".into())
                    }
                };
                if let Some((_, turn, _)) = kept.as_mut() {
                    if step {
                        turn.step(&text);
                    }
                    match kind {
                        "done" => turn.done(&text),
                        "status" if text == "Stopped" => turn.stopped(),
                        _ => {}
                    }
                }
                emit(&tx, json!({"type":"agent","kind":kind,"text":text}));
            })?;
            emit(&tx, json!({"type":"agent_state","state":"completed"}));
            Ok::<_, String>(())
        })();
        let outcome = match &result {
            Ok(()) => review::Outcome::Done(String::new()),
            Err(err) => review::Outcome::Failed(err.clone()),
        };
        if let Err(err) = result {
            emit(
                &tx,
                json!({"type":"agent_state","state":"failed","message":err}),
            );
            error(&tx, err);
        }
        if let Some((client, turn, before)) = kept {
            let proposal = client
                .get_state()
                .ok()
                .and_then(|state| review::staged_proposal(&before, &state));
            if let Err(err) = turn.finish(&client, outcome, proposal, &shared, &tx) {
                error(
                    &tx,
                    format!("Kerf's reply was not kept as a note: {}", err.message),
                );
            }
        }
        if let Ok(mut abort) = shared.abort.lock() {
            *abort = None;
        }
        shared.agent_busy.store(false, Ordering::SeqCst);
        emit(&tx, json!({"type":"agent_busy","busy":false}));
        let _ = commands.send(Command::Refresh);
    }
}

/// Start workers without waiting for project I/O.
/// # Safety
/// Both arguments must be live, NUL-terminated UTF-8 strings for this call.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_new(
    directory: *const c_char,
    binary: *const c_char,
) -> *mut c_void {
    if directory.is_null() || binary.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: The caller supplies the two live C strings as documented above.
    let (Ok(directory), Ok(binary)) = (
        unsafe { CStr::from_ptr(directory) }.to_str(),
        unsafe { CStr::from_ptr(binary) }.to_str(),
    ) else {
        return std::ptr::null_mut();
    };
    let directory = PathBuf::from(directory);
    let binary = PathBuf::from(binary);
    // The project's own looks, for its previews; kerfd reports any that do
    // not read.
    let _ = kerf_model::look::install_project_looks(&directory);
    let (commands, rx) = mpsc::sync_channel(32);
    let (media, media_rx) = mpsc::sync_channel(32);
    let (prompts, prompt_rx) = mpsc::sync_channel(1);
    let (collab, collab_rx) = mpsc::sync_channel(16);
    let (emit, events) = mpsc::sync_channel(256);
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
        preview_running: AtomicBool::new(false),
        preview_stopped: AtomicBool::new(false),
        stale_previews: Mutex::new(Vec::new()),
        shown_preview: Mutex::new(None),
        review_seen: Mutex::new(None),
        check_busy: AtomicBool::new(false),
        check_cancel: Mutex::new(None),
        renders: queue::Queue::default(),
        analysis: analysis::Analysis::default(),
    });
    {
        let directory = directory.clone();
        let binary = binary.clone();
        let commands = commands.clone();
        let media = media.clone();
        let tx = emit.clone();
        let shared = shared.clone();
        thread::spawn(move || runtime_worker(directory, binary, rx, commands, media, tx, shared));
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
    Box::into_raw(Box::new(Bridge {
        events_tx: emit,
        commands,
        media,
        collab,
        prompts,
        events,
        shared,
        displayed_frame: None,
        displayed_signal: None,
    }))
    .cast()
}

/// Release the bridge and signal worker shutdown without blocking the GUI.
/// # Safety
/// `bridge` must be a live pointer returned by `kerf_bridge_new`, freed only once.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_free(bridge: *mut c_void) {
    if bridge.is_null() {
        return;
    }
    // SAFETY: Ownership is transferred back exactly once by the caller.
    let bridge = unsafe { Box::from_raw(bridge.cast::<Bridge>()) };
    bridge.shared.closed.store(true, Ordering::SeqCst);
    // Nobody would see a preview still rendering, or the one playing. The
    // process may exit before the media worker stops, so the files go now;
    // one a playback worker still holds open is left for the media worker.
    cancel_preview(&bridge.shared);
    discard_previews(&bridge.shared);
    qc::cancel(&bridge.shared);
    bridge.shared.renders.close(&bridge.shared.directory);
    bridge.shared.analysis.close();
    if let Ok(abort) = bridge.shared.abort.lock() {
        if let Some(abort) = &*abort {
            abort.store(true, Ordering::SeqCst);
        }
    };
    if let Ok(aborts) = bridge.shared.version_aborts.lock() {
        for abort in aborts.iter() {
            abort.store(true, Ordering::SeqCst);
        }
    };
}

/// Queue an edit or control message; `false` means it was not accepted.
/// # Safety
/// `bridge` must be live and exclusively accessed by the GUI thread. `message`
/// must be a live NUL-terminated string. No bridge calls may overlap.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_send(bridge: *mut c_void, message: *const c_char) -> bool {
    if bridge.is_null() || message.is_null() {
        return false;
    }
    // SAFETY: Pointers and exclusive access are guaranteed by the caller.
    let bridge = unsafe { &mut *bridge.cast::<Bridge>() };
    // SAFETY: message is a live C string for this call.
    let parsed = unsafe { CStr::from_ptr(message) }
        .to_str()
        .ok()
        .and_then(|text| serde_json::from_str::<Command>(text).ok());
    let Some(command) = parsed else {
        return false;
    };
    match command {
        Command::Seek { position, scrub } => {
            if !position.is_finite() || position < 0.0 {
                return false;
            }
            // The worker holds this only to take the seek out, and a seek
            // refused for that moment would be lost.
            if let Ok(mut slot) = bridge.shared.seek.lock() {
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
            bridge.media.try_send(command).is_ok()
        }
        Command::Abort => {
            if let Ok(aborts) = bridge.shared.version_aborts.lock() {
                for abort in aborts.iter() {
                    abort.store(true, Ordering::SeqCst);
                }
            }
            if let Ok(abort) = bridge.shared.abort.try_lock() {
                if let Some(abort) = &*abort {
                    abort.store(true, Ordering::SeqCst);
                }
                true
            } else {
                false
            }
        }
        Command::EffectPreviews(request) => {
            let generation = bridge.shared.previews.fetch_add(1, Ordering::SeqCst) + 1;
            let shared = bridge.shared.clone();
            let tx = bridge.events_tx.clone();
            thread::spawn(move || {
                if let Err(message) = previews::run(&shared, generation, &request, &tx) {
                    error(&tx, message);
                }
            });
            true
        }
        Command::Check { sequence } => {
            qc::spawn(bridge.shared.clone(), bridge.events_tx.clone(), sequence);
            true
        }
        Command::Loudness { sequence } => {
            analysis::loudness(bridge.shared.clone(), bridge.events_tx.clone(), sequence);
            true
        }
        Command::ScopeSignal { time, sequence } => {
            analysis::scope_signal(
                bridge.shared.clone(),
                bridge.events_tx.clone(),
                time,
                sequence,
            );
            true
        }
        Command::MediaInfo { media } => {
            analysis::media_info(bridge.shared.clone(), bridge.events_tx.clone(), media);
            true
        }
        Command::MediaIndex { media } => {
            analysis::media_index(bridge.shared.clone(), bridge.events_tx.clone(), media);
            true
        }
        Command::CloudSignIn { .. } | Command::CloudSignOut | Command::CloudRefresh => {
            let tx = bridge.events_tx.clone();
            thread::spawn(move || cloud_status(&tx, Some(command)));
            true
        }
        Command::PreviewStop => {
            let shared = bridge.shared.clone();
            let tx = bridge.events_tx.clone();
            thread::spawn(move || stop_preview(&shared, &tx));
            true
        }
        Command::PreviewExit => {
            let shared = bridge.shared.clone();
            let tx = bridge.events_tx.clone();
            thread::spawn(move || exit_preview(&shared, &tx));
            true
        }
        Command::PresetLibrary => {
            let directory = bridge.shared.directory.clone();
            let tx = bridge.events_tx.clone();
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
        | Command::CollabConsolidate => bridge.collab.try_send(command).is_ok(),
        Command::Agent {
            text,
            focus,
            comment,
            settings,
        } => {
            if text.trim().is_empty() || bridge.shared.agent_busy.swap(true, Ordering::SeqCst) {
                return false;
            }
            let note = comment.map(|comment| (text.clone(), comment));
            let text = match &focus {
                Some(focus) => kerf_agent::focused_request(&text, focus),
                None => text,
            };
            if bridge
                .prompts
                .try_send(AgentRequest {
                    text,
                    settings,
                    versions: 0,
                    note,
                })
                .is_err()
            {
                bridge.shared.agent_busy.store(false, Ordering::SeqCst);
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
                || bridge.shared.agent_busy.swap(true, Ordering::SeqCst)
            {
                return false;
            }
            if bridge
                .prompts
                .try_send(AgentRequest {
                    text,
                    settings,
                    versions: count,
                    note: None,
                })
                .is_err()
            {
                bridge.shared.agent_busy.store(false, Ordering::SeqCst);
                false
            } else {
                true
            }
        }
        _ => bridge.commands.try_send(command).is_ok(),
    }
}

/// Return one owned event string, or null when no event is pending.
/// # Safety
/// `bridge` must be live and exclusively accessed by the GUI thread.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_poll(bridge: *mut c_void) -> *mut c_char {
    if bridge.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: The caller guarantees a live, exclusively accessed bridge.
    let bridge = unsafe { &mut *bridge.cast::<Bridge>() };
    bridge
        .events
        .try_recv()
        .ok()
        .and_then(|value| CString::new(value.to_string()).ok())
        .map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Release an event string returned by `kerf_bridge_poll`.
/// # Safety
/// `value` must be null or an unfreed pointer returned by `kerf_bridge_poll`.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: Ownership is returned exactly once by the caller.
        drop(unsafe { CString::from_raw(value) });
    }
}

// ---- about ---------------------------------------------------------------------

/// Where Kerf's source and releases are.
pub const REPOSITORY: &str = "https://github.com/imjunseong958-droid/industrial-kerf";

/// `{name, version, license, repository}`: what Help > About Kerf shows.
#[no_mangle]
pub extern "C" fn kerf_about() -> *mut c_char {
    owned_json(about())
}

fn about() -> Value {
    json!({
        "name": "Industrial Kerf",
        "version": env!("CARGO_PKG_VERSION"),
        "license": env!("CARGO_PKG_LICENSE"),
        "repository": REPOSITORY,
    })
}

/// Whether a newer Kerf is released on GitHub: `{current, latest?, newer, url?}` or `{error}`. It
/// asks the network and may take seconds, so call it off the GUI thread.
#[no_mangle]
pub extern "C" fn kerf_update_check() -> *mut c_char {
    owned_json(updates::check())
}

// ---- crash reports ---------------------------------------------------------------

/// Report every panic in the editor's process to the crash folder, as kerfd and the CLI do.
#[no_mangle]
pub extern "C" fn kerf_crash_install() {
    kerf_runtime::crash::install("editor");
}

/// Report `message` now as what ends the editor: Qt's fatal message, before Qt aborts
/// (`crash_notice.cpp`). Nothing is written before `kerf_crash_install`.
/// # Safety
/// `message` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_crash_fatal(message: *const c_char) {
    if message.is_null() {
        return;
    }
    let message = CStr::from_ptr(message).to_string_lossy();
    let _ = kerf_runtime::crash::fatal(&message);
}

/// `{folder, reports: [{path, component, atMs, message}]}`, newest first; `folder` is null when
/// there is nowhere to keep them.
#[no_mangle]
pub extern "C" fn kerf_crash_reports() -> *mut c_char {
    owned_json(crash_reports_json(kerf_runtime::crash::folder()))
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

/// The system keychain: `{op: get|set|delete, name, value?}` -> `{ok: true, value?}` or `{error}`.
/// # Safety
/// `request` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_secret(request: *const c_char) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract.
    let request = unsafe { json_argument(request) };
    owned_json(secrets::request(&request))
}

// ---- command registry ----------------------------------------------------------

// ---- project launcher ------------------------------------------------------------

/// What the launcher shows of a project folder.
/// # Safety
/// `request` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_project_inspect(request: *const c_char) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract.
    let request = unsafe { json_argument(request) };
    owned_json(tasks::inspect_project(&request))
}

/// A preset file's slots and how files would fill them.
/// # Safety
/// `request` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_preset_inspect(request: *const c_char) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract.
    let request = unsafe { json_argument(request) };
    owned_json(tasks::inspect_preset(&request))
}

/// Start a project task; poll it with `kerf_task_poll`.
/// # Safety
/// `request` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_task_start(request: *const c_char) -> *mut c_void {
    // SAFETY: forwarded from the caller's contract.
    let request = unsafe { json_argument(request) };
    Box::into_raw(Box::new(tasks::Task::start(request))).cast()
}

/// The task's latest status as an owned string.
/// # Safety
/// `task` must be null or a live pointer from `kerf_task_start`.
#[no_mangle]
pub unsafe extern "C" fn kerf_task_poll(task: *mut c_void) -> *mut c_char {
    if task.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a live task from kerf_task_start.
    let task = unsafe { &*task.cast::<tasks::Task>() };
    owned_json(task.status())
}

/// Release a task; one still running finishes on its own.
/// # Safety
/// `task` must be null or a live pointer from `kerf_task_start`, freed once.
#[no_mangle]
pub unsafe extern "C" fn kerf_task_free(task: *mut c_void) {
    if !task.is_null() {
        // SAFETY: ownership returns exactly once.
        drop(unsafe { Box::from_raw(task.cast::<tasks::Task>()) });
    }
}

/// Read a UTF-8 JSON argument; a null pointer or invalid text reads as null.
///
/// # Safety
/// `value` must be null or a NUL-terminated string.
unsafe fn json_argument(value: *const c_char) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    // SAFETY: The caller guarantees a NUL-terminated string.
    let text = unsafe { CStr::from_ptr(value) };
    text.to_str()
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null)
}

fn owned_json(value: Value) -> *mut c_char {
    CString::new(value.to_string()).map_or(std::ptr::null_mut(), CString::into_raw)
}

fn command_context(value: &Value) -> kerf_commands::Context {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// The command registry with shortcuts in `keymap` (a JSON string such as
/// `"premiere"`, or plain text). Free the result with `kerf_bridge_string_free`.
/// # Safety
/// `keymap` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_commands_catalog(keymap: *const c_char) -> *mut c_char {
    let keymap = if keymap.is_null() {
        String::new()
    } else {
        // SAFETY: The caller guarantees a NUL-terminated string.
        unsafe { CStr::from_ptr(keymap) }
            .to_string_lossy()
            .into_owned()
    };
    owned_json(kerf_commands::catalog(&keymap))
}

/// The picture effects and transitions the editor offers, as a JSON array
/// of `{id, kind, category, name, doc, engine, params}`. Free the result
/// with `kerf_bridge_string_free`.
#[no_mangle]
pub extern "C" fn kerf_fx_catalog() -> *mut c_char {
    owned_json(kerf_runtime::fx_catalog_json())
}

/// Everything the catalog offers, as a JSON array of `{id, kind, domain,
/// category, name, doc, engine, params}`: picture effects and transitions
/// as `kerf_fx_catalog` lists them, plus generators (`kind: "generator"`,
/// such as titles) and sound effects (`domain: "audio"`). Free the result
/// with `kerf_bridge_string_free`.
#[no_mangle]
pub extern "C" fn kerf_fx_catalog_all() -> *mut c_char {
    owned_json(kerf_runtime::fx_catalog_all_json())
}

/// The properties a clip has by its track's type, which `kerf_fx_catalog_all`
/// does not list: `{video: [...], audio: [...]}`, as a timeline's `properties`
/// carries them. Free the result with `kerf_bridge_string_free`.
#[no_mangle]
pub extern "C" fn kerf_clip_properties() -> *mut c_char {
    owned_json(kerf_runtime::properties_json())
}

/// Why each command is unavailable in a JSON context, or null when it is
/// available. Free the result with `kerf_bridge_string_free`.
/// # Safety
/// `context` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_commands_states(context: *const c_char) -> *mut c_char {
    // SAFETY: Forwarded from the caller's guarantee.
    let context = unsafe { json_argument(context) };
    owned_json(kerf_commands::states(&command_context(&context)))
}

/// Resolve `{command, params, context}` to `{request}`, the bridge command
/// that carries it out, or `{error}`. Free the result with
/// `kerf_bridge_string_free`.
/// # Safety
/// `invocation` must be null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn kerf_commands_run(invocation: *const c_char) -> *mut c_char {
    // SAFETY: Forwarded from the caller's guarantee.
    let invocation = unsafe { json_argument(invocation) };
    owned_json(run_command(&invocation))
}

fn run_command(invocation: &Value) -> Value {
    let id = invocation["command"].as_str().unwrap_or_default();
    let context = command_context(&invocation["context"]);
    match kerf_commands::run(id, &context, &invocation["params"]) {
        Ok(request) => json!({"request": request}),
        Err(error) => json!({"error": error}),
    }
}

/// A borrowed RGBA frame; bytes live until the next frame poll or bridge free.
#[repr(C)]
pub struct KerfFrame {
    pub pixels: *const u8,
    pub length: usize,
    pub width: u32,
    pub height: u32,
    pub time: f64,
}

/// An HDR sequence's frame as its signal, for the Finish tray's scopes.
#[repr(C)]
pub struct KerfSignal {
    /// 16-bit little-endian RGBA, in BT.2020.
    pub pixels: *const u8,
    pub length: usize,
    pub width: u32,
    pub height: u32,
    pub time: f64,
    /// 1 for PQ, 2 for HLG.
    pub curve: u32,
}

/// Take the newest signal frame that `scope_signal` drew, without copying its pixels.
/// # Safety
/// `bridge` must be live and exclusively accessed. `output` must point to a
/// writable `KerfSignal`. Previously returned pixels expire on this call.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_signal(bridge: *mut c_void, output: *mut KerfSignal) -> bool {
    if bridge.is_null() || output.is_null() {
        return false;
    }
    // SAFETY: Both pointers and exclusive access are guaranteed by the caller.
    let bridge = unsafe { &mut *bridge.cast::<Bridge>() };
    let Some(frame) = bridge
        .shared
        .analysis
        .signal_frame
        .try_lock()
        .ok()
        .and_then(|mut frame| frame.take())
    else {
        return false;
    };
    let frame = bridge.displayed_signal.insert(frame);
    // SAFETY: output is writable and pixels remain owned by this bridge.
    unsafe {
        *output = KerfSignal {
            pixels: frame.pixels.as_ptr(),
            length: frame.pixels.len(),
            width: frame.width,
            height: frame.height,
            time: frame.time,
            curve: frame.curve,
        };
    }
    true
}

/// Take the newest frame without copying its pixels.
/// # Safety
/// `bridge` must be live and exclusively accessed. `output` must point to a
/// writable `KerfFrame`. Previously returned pixels expire on this call.
#[no_mangle]
pub unsafe extern "C" fn kerf_bridge_frame(bridge: *mut c_void, output: *mut KerfFrame) -> bool {
    if bridge.is_null() || output.is_null() {
        return false;
    }
    // SAFETY: Both pointers and exclusive access are guaranteed by the caller.
    let bridge = unsafe { &mut *bridge.cast::<Bridge>() };
    let frame = bridge
        .shared
        .frame
        .try_lock()
        .ok()
        .and_then(|mut frame| frame.take());
    let Some(frame) = frame.filter(PlaybackFrame::has_valid_pixels) else {
        return false;
    };
    bridge.displayed_frame = Some(frame);
    let frame = bridge
        .displayed_frame
        .as_ref()
        .expect("frame was just stored");
    // SAFETY: output is writable and pixels remain owned by this bridge.
    unsafe {
        *output = KerfFrame {
            pixels: frame.pixels.as_ptr(),
            length: frame.pixels.len(),
            width: frame.width,
            height: frame.height,
            time: frame.timeline_time,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{
        cancel_preview, discard_preview, discard_previews, media_worker, preview_frame_size,
        publish, publish_force, queue, rendered_preview, sweep_previews, AgentSettings, Command,
        MediaProject, Shared,
    };
    use kerf_runtime::{EditOp, PendingProposal, ProposalEntry, ProposalFreshness, StateSnapshot};
    use serde_json::json;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_editor_has_the_workspaces_version() {
        let cmake = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/kerf-qt/CMakeLists.txt"),
        )
        .expect("the editor's CMakeLists.txt");
        let declared = cmake
            .lines()
            .find_map(|line| line.trim().strip_prefix("project(kerf_qt VERSION "))
            .and_then(|rest| rest.split_whitespace().next())
            .expect("project(kerf_qt VERSION ...)");
        assert_eq!(declared, env!("CARGO_PKG_VERSION"));
        assert_eq!(super::about()["version"], json!(env!("CARGO_PKG_VERSION")));
        assert_eq!(super::about()["license"], json!("MIT"));
    }

    #[test]
    fn crash_reports_reach_the_editor_newest_first() {
        let dir = tempfile::tempdir().expect("a folder");
        for (at, message) in [(5u128, "older"), (9, "newer")] {
            let text = kerf_runtime::crash::report_text("kerfd", at, "main", message, "here", "");
            std::fs::write(dir.path().join(format!("kerfd-{at}-1.txt")), text).expect("written");
        }
        let listed = super::crash_reports_json(Some(dir.path().to_owned()));
        assert_eq!(listed["folder"], json!(dir.path()));
        let messages: Vec<&str> = listed["reports"]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| report["message"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(messages, ["newer", "older"]);
        assert_eq!(listed["reports"][0]["atMs"], json!(9.0));
        assert_eq!(
            super::crash_reports_json(None),
            json!({"folder": null, "reports": []})
        );
    }

    fn shared() -> Shared {
        crate::tests_support::shared(&std::env::temp_dir())
    }

    const DOCUMENT: &str = "kerf 1\nproject name=\"Bridge\"\n\
        sequence s1 rate=24 size=1920x1080 audio=48000\n";

    fn project(width: u32, height: u32) -> kerf_model::Project {
        kerf_runtime::parse_document(&format!(
            "kerf 1\nproject name=\"Bridge\"\n\
             sequence s1 rate=24 size={width}x{height} audio=48000\n"
        ))
        .unwrap()
    }

    pub(crate) fn snapshot(
        document: &str,
        pending_proposal: Option<PendingProposal>,
    ) -> StateSnapshot {
        StateSnapshot {
            document: document.to_owned(),
            timeline: json!({}),
            revision: 1,
            can_undo: true,
            can_redo: false,
            duration: 0.0,
            pending_proposal,
            source_version: 3,
            projection_status: "current".into(),
            import_warnings: Vec::new(),
            proposals: Vec::new(),
            review_revision: 0,
            revision_time: None,
        }
    }

    #[test]
    fn proposal_only_snapshot_is_published_without_restarting_media() {
        let (tx, rx) = mpsc::sync_channel(4);
        let shared = Arc::new(shared());
        let project = DOCUMENT;
        let mut previous = None;

        publish(snapshot(project, None), &shared, &tx, &mut previous);
        let first = rx.try_recv().expect("initial snapshot should publish");
        assert_eq!(first["type"], "state");
        let media_revision = shared.revision.load(std::sync::atomic::Ordering::SeqCst);
        publish_force(snapshot(project, None), &shared, &tx, &mut previous);
        assert_eq!(
            rx.try_recv().expect("forced refresh should publish")["type"],
            "state"
        );
        assert_eq!(
            shared.revision.load(std::sync::atomic::Ordering::SeqCst),
            media_revision,
            "forced refresh must not restart unchanged media"
        );

        let proposal = PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 1,
            script: "track add T1 video\n".into(),
            source: String::new(),
            document_diff: "+  track T1 video\n".into(),
            freshness: ProposalFreshness::Current,
        };
        let proposal_for_repeat = proposal.clone();
        publish(
            snapshot(project, Some(proposal)),
            &shared,
            &tx,
            &mut previous,
        );
        let proposal_event = rx.try_recv().expect("proposal snapshot should publish");
        assert_eq!(
            proposal_event["snapshot"]["pendingProposal"]["proposalId"],
            "proposal-1"
        );
        assert_eq!(
            shared.revision.load(std::sync::atomic::Ordering::SeqCst),
            media_revision,
            "proposal-only refresh must keep the media generation stable"
        );
        publish(
            snapshot(DOCUMENT, Some(proposal_for_repeat)),
            &shared,
            &tx,
            &mut previous,
        );
        assert!(
            rx.try_recv().is_err(),
            "unchanged snapshots must be coalesced"
        );
    }

    #[test]
    fn command_requests_are_bridge_commands() {
        let context = json!({
            "connected": true, "revision": 4, "canUndo": true, "playheadFrame": 30, "agentBusy": true,
            "selection": {"clip": "c1", "track": "video", "startFrame": 0, "durationFrames": 48},
        });
        for (command, params) in [
            ("file.import", json!({"path": "/tmp/a.mov"})),
            ("file.export", json!({"quality": "full"})),
            ("file.exportPreset", json!({"path":"/tmp/style.kerfpreset"})),
            (
                "file.applyPreset",
                json!({"path":"/tmp/style.kerfpreset", "bindings":{}, "texts":{}}),
            ),
            ("file.refresh", json!(null)),
            ("edit.undo", json!(null)),
            ("clip.split", json!(null)),
            ("clip.rippleDelete", json!(null)),
            ("sequence.addVideoTrack", json!(null)),
            (
                "clip.trim",
                json!({"clip": "c1", "edge": "in", "frames": 2}),
            ),
            ("proposal.reject", json!({"proposal": "p1"})),
            ("ai.stop", json!(null)),
        ] {
            let result = super::run_command(
                &json!({"command": command, "params": params, "context": context}),
            );
            let request = result["request"].clone();
            assert!(
                serde_json::from_value::<Command>(request).is_ok(),
                "{command} resolved to {result}"
            );
        }
        let refused =
            super::run_command(&json!({"command": "clip.delete", "context": {"connected": true}}));
        assert_eq!(refused["error"], "Select a clip first");
    }

    #[test]
    fn cloud_commands_and_settings_are_read() {
        for command in [
            json!({"type": "cloud_sign_in", "token": "tok"}),
            json!({"type": "cloud_sign_out"}),
            json!({"type": "cloud_refresh"}),
        ] {
            assert!(
                serde_json::from_value::<Command>(command.clone()).is_ok(),
                "{command}"
            );
        }
        let Command::Agent { settings, .. } =
            serde_json::from_value(json!({"type": "agent", "text": "x", "cloud": true})).unwrap()
        else {
            panic!("expected agent command");
        };
        assert!(settings.cloud);
    }

    #[test]
    fn versions_are_asked_for_three_by_default_and_differ() {
        use crate::{version_prompt, MAX_VERSIONS};
        let Command::AgentVersions { text, count, .. } =
            serde_json::from_value(json!({"type": "agent_versions", "text": "Open stronger"}))
                .unwrap()
        else {
            panic!("expected versions");
        };
        assert_eq!((text.as_str(), count), ("Open stronger", 3));
        let prompts: Vec<String> = (0..MAX_VERSIONS)
            .map(|index| version_prompt("Open stronger", index, MAX_VERSIONS))
            .collect();
        assert!(prompts[0].starts_with("Open stronger"));
        assert!(prompts[1].contains("version B of 3"));
        assert_ne!(prompts[0], prompts[2]);
    }

    #[test]
    fn a_tool_call_says_where_it_works() {
        use crate::focus_event;
        let targets = kerf_agent::AgentTargetSummary {
            times: vec!["4.5s".into()],
            ..Default::default()
        };
        let focus = focus_event("look", &targets).unwrap();
        assert_eq!(focus["type"], "agent_focus");
        assert_eq!(focus["times"], json!(["4.5s"]));
        assert!(focus_event("look", &Default::default()).is_none());
    }

    #[test]
    fn agent_requests_carry_their_focus() {
        let command: Command = serde_json::from_value(json!({
            "type": "agent", "text": "Tighten this",
            "focus": {"scope": "clip", "clip": "c3", "fromFrame": 24, "toFrame": 72},
        }))
        .expect("focused agent command");
        let Command::Agent { focus, .. } = command else {
            panic!("expected agent command");
        };
        let focus = focus.expect("focus");
        assert!(kerf_agent::focused_request("Tighten this", &focus)
            .starts_with("Focus: clip c3 (24f–72f)."));
    }

    #[test]
    fn pending_proposals_carry_the_timeline_they_would_make() {
        let state = snapshot(
            DOCUMENT,
            Some(PendingProposal::Pending {
                proposal_id: "p1".into(),
                base_revision: 0,
                script: "track add T9 video\n".into(),
                source: String::new(),
                document_diff: String::new(),
                freshness: ProposalFreshness::Current,
            }),
        );
        let value = super::with_proposal_timeline(serde_json::to_value(&state).unwrap(), &state);
        let tracks = value["proposalTimeline"]["tracks"]
            .as_array()
            .expect("proposal tracks");
        assert!(tracks.iter().any(|track| track["id"] == "T9"), "{tracks:?}");
    }

    #[test]
    fn edit_command_carries_typed_operations() {
        let command: Command = serde_json::from_value(json!({
            "type": "edit",
            "revision": 4,
            "ops": [
                {"op": "move", "clip": "c1", "frames": 12},
                {"op": "trim", "clip": "c1", "edge": "out", "frames": -6},
                {"op": "set", "clip": "c1", "props": {"crop_l": 0.25, "opacity": null}}
            ]
        }))
        .expect("edit command should deserialize");
        let Command::Edit { ops, revision } = command else {
            panic!("expected an edit command");
        };
        assert_eq!((ops.len(), revision), (3, 4));
    }

    #[test]
    fn mask_edits_from_the_viewer_are_typed_operations() {
        let command: Command = serde_json::from_value(json!({
            "type": "edit",
            "revision": 2,
            "ops": [
                {"op": "mask-add", "clip": "c1", "name": "k1", "rect": [0.25, 0.25, 0.75, 0.75]},
                {"op": "mask-set", "clip": "c1", "name": "k1",
                 "points": [[0.1, 0.1], [0.9, 0.1], [0.5, 0.9]], "params": {"feather": null}},
                {"op": "mask-remove", "clip": "c1", "name": "k1"}
            ]
        }))
        .expect("mask edits should deserialize");
        let Command::Edit { ops, .. } = command else {
            panic!("expected an edit command");
        };
        let [EditOp::MaskAdd { name, outline, .. }, EditOp::MaskSet {
            outline: set,
            params,
            ..
        }, EditOp::MaskRemove { .. }] = ops.as_slice()
        else {
            panic!("expected mask edits, got {ops:?}");
        };
        assert_eq!(name.as_deref(), Some("k1"));
        assert_eq!(outline.rect, Some([0.25, 0.25, 0.75, 0.75]));
        assert_eq!(set.points.as_ref().map(Vec::len), Some(3));
        assert_eq!(params.get("feather"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn agent_command_accepts_per_request_provider_settings() {
        let command: Command = serde_json::from_value(json!({
            "type": "agent",
            "text": "Trim the opening",
            "provider": "openai",
            "apiMode": "responses",
            "apiKey": "test-key",
            "baseUrl": "http://localhost:9000/v1",
            "model": "local-model",
            "mcpEnabled": true,
            "mcpConfigVersion": 7
        }))
        .expect("agent command should deserialize");
        let Command::Agent { text, settings, .. } = command else {
            panic!("expected agent command");
        };
        assert_eq!(text, "Trim the opening");
        assert_eq!(
            settings,
            AgentSettings {
                provider: Some(crate::ChatProvider::Openai),
                api_mode: Some(kerf_agent::OpenAiApiMode::Responses),
                api_key: Some("test-key".into()),
                base_url: Some("http://localhost:9000/v1".into()),
                model: Some("local-model".into()),
                mcp_enabled: true,
                mcp_config_version: 7,
                cloud: false,
            }
        );
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

    #[test]
    fn preview_frame_follows_the_sequence_canvas() {
        assert_eq!(preview_frame_size(&project(1080, 1920)), (1080, 1920));
        assert_eq!(preview_frame_size(&project(1920, 1080)), (1920, 1080));
        assert_eq!(
            preview_frame_size(&kerf_model::Project::default()),
            (1920, 1080)
        );
    }

    #[test]
    fn preview_frame_size_limits_only_the_longest_edge_for_large_canvases() {
        assert_eq!(preview_frame_size(&project(8000, 4000)), (1920, 960));
    }

    #[test]
    fn a_rendered_preview_plays_in_place_of_the_program() {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n\
             \x20     set crop_l=0.25\n",
        )
        .unwrap();
        // The compositor draws a crop live; a rendered preview still plays
        // exactly.
        assert_eq!(
            kerf_playback::preview_plan(&program).mode,
            kerf_core::preview::PreviewMode::Composited
        );
        let rendered = rendered_preview(&program, "/renders/preview.mp4", None).unwrap();
        assert!(kerf_model::validate::errors(&rendered, kerf_model::Catalog::builtin()).is_empty());
        assert_eq!(
            rendered.sequences[0].duration(),
            program.sequences[0].duration()
        );
        let timeline = kerf_playback::PlaybackTimeline::from_project(&rendered).unwrap();
        assert_eq!(
            timeline.video_at(1.0).unwrap().source.path.to_str(),
            Some("/renders/preview.mp4")
        );
        assert_eq!(timeline.audio_at(1.0).len(), 1);
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
    fn an_edit_removes_a_preview_that_never_played() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let file = directory.path().join("render-1.mp4");
        std::fs::write(&file, b"preview").unwrap();
        *shared.project.lock().unwrap() = Some(rendered(&file));

        let (tx, _rx) = mpsc::sync_channel(8);
        publish(snapshot(DOCUMENT, None), &shared, &tx, &mut None);
        assert!(matches!(
            *shared.project.lock().unwrap(),
            Some(MediaProject::Native(_))
        ));
        assert!(!file.exists(), "the superseded preview must be removed");
    }

    #[test]
    fn a_preview_still_open_is_removed_later() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        // A directory stands in for a file something still holds open:
        // removing it as a file fails.
        let held = directory.path().join("render-held.mp4");
        std::fs::create_dir(&held).unwrap();
        discard_preview(&shared, held.clone());
        assert!(!sweep_previews(&shared));
        assert_eq!(shared.stale_previews.lock().unwrap().len(), 1);

        std::fs::remove_dir(&held).unwrap();
        assert!(sweep_previews(&shared));
        assert!(shared.stale_previews.lock().unwrap().is_empty());
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

    #[test]
    fn closing_cancels_only_a_running_preview() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        cancel_preview(&shared);
        assert!(!marker.exists(), "no preview is running");

        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        cancel_preview(&shared);
        assert!(marker.exists());
    }

    #[test]
    fn the_media_worker_removes_previews_it_stops_playing() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let first = directory.path().join("render-1.mp4");
        let second = directory.path().join("render-2.mp4");
        std::fs::write(&first, b"preview").unwrap();
        std::fs::write(&second, b"preview").unwrap();
        *shared.project.lock().unwrap() = Some(rendered(&first));

        let (_commands, rx) = mpsc::sync_channel(4);
        let (tx, events) = mpsc::sync_channel(4);
        drop(events);
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || media_worker(rx, tx, shared))
        };
        let taken = || shared.project.lock().unwrap().is_none().then_some(());
        assert!(crate::tests_support::wait(Duration::from_secs(5), taken).is_some());
        assert!(first.exists(), "the preview playing stays");

        *shared.project.lock().unwrap() = Some(rendered(&second));
        let removed = || (!first.exists()).then_some(());
        assert!(crate::tests_support::wait(Duration::from_secs(5), removed).is_some());
        assert!(crate::tests_support::wait(Duration::from_secs(5), taken).is_some());
        assert!(second.exists());

        shared.closed.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert!(!second.exists(), "closing removes the preview it played");
    }

    #[test]
    fn preview_and_export_commands_carry_their_new_fields() {
        let Command::Preview {
            proposal_id,
            start,
            duration,
        } = serde_json::from_value(json!({
            "type": "preview", "proposalId": "proposal-2", "start": 12.5, "duration": 4.0
        }))
        .unwrap()
        else {
            panic!("expected preview");
        };
        assert_eq!(proposal_id.as_deref(), Some("proposal-2"));
        assert_eq!((start, duration), (Some(12.5), Some(4.0)));
        // The plain preview the command registry sends still reads.
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview"})).unwrap(),
            Command::Preview {
                proposal_id: None,
                start: None,
                duration: None
            }
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview_stop"})).unwrap(),
            Command::PreviewStop
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "preview_exit"})).unwrap(),
            Command::PreviewExit
        ));
        let Command::Export(export) = serde_json::from_value(json!({
            "type": "export", "quality": "full", "sequence": "vertical"
        }))
        .unwrap() else {
            panic!("expected export");
        };
        assert_eq!(export.quality, "full");
        assert_eq!(export.sequence.as_deref(), Some("vertical"));
        assert!(matches!(
            serde_json::from_value(json!({"type": "export", "quality": "preview"})).unwrap(),
            Command::Export(queue::ExportRequest { sequence: None, .. })
        ));
    }

    #[test]
    fn render_events_name_the_proposal_they_preview() {
        let request = super::RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: Some("proposal-7".into()),
            range: None,
            sequence: None,
        };
        assert_eq!(
            request.event("running"),
            json!({"type": "preview", "status": "running", "proposalId": "proposal-7"})
        );
        let export = super::RenderRequest {
            quality: "full".into(),
            preview: false,
            proposal: None,
            range: None,
            sequence: Some("s2".into()),
        };
        assert_eq!(
            export.event("completed"),
            json!({"type": "export", "status": "completed"})
        );
    }

    #[test]
    fn every_open_proposal_carries_its_timeline() {
        let entry = |id: &str, script: &str, status: &str| ProposalEntry {
            proposal_id: id.into(),
            status: status.into(),
            freshness: ProposalFreshness::Current,
            base_revision: 1,
            applied_revision: None,
            script: script.into(),
            source: String::new(),
            document_diff: String::new(),
            author: kerf_runtime::Author::kerf(),
            version: 1,
            supersedes: None,
            versions: vec![kerf_runtime::ProposalVersion {
                version: 1,
                proposal_id: id.into(),
                base_revision: 1,
                script: script.into(),
                source: String::new(),
                document_diff: String::new(),
            }],
            staged: None,
        };
        let mut revised = entry("p2", "track add T2 video\n", "pending");
        revised.version = 2;
        revised.versions.insert(
            0,
            kerf_runtime::ProposalVersion {
                version: 1,
                proposal_id: "p2-v1".into(),
                base_revision: 1,
                script: "track add T5 video\n".into(),
                source: String::new(),
                document_diff: String::new(),
            },
        );
        revised.versions[1].version = 2;
        let mut state = snapshot(DOCUMENT, None);
        state.proposals = vec![
            revised,
            entry("p1", "track add T1 video\n", "pending"),
            entry("p0", "track add T0 video\n", "accepted"),
        ];
        let value = super::with_proposal_timeline(serde_json::to_value(&state).unwrap(), &state);
        let timelines = value["proposalTimelines"].as_object().unwrap();
        let mut ids: Vec<&str> = timelines.keys().map(String::as_str).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            ["p1", "p2", "p2-v1"],
            "open proposals and their versions only"
        );
        let has_track = |id: &str, track: &str| {
            timelines[id]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["id"] == track)
        };
        assert!(has_track("p2", "T2") && has_track("p2-v1", "T5") && has_track("p1", "T1"));
        assert_eq!(value["proposals"].as_array().unwrap().len(), 3);

        // A snapshot without proposals or comments still has both fields.
        let empty = snapshot(DOCUMENT, None);
        let value = super::with_proposal_timeline(serde_json::to_value(&empty).unwrap(), &empty);
        assert_eq!(value["proposals"], json!([]));
        assert_eq!(value["reviewRevision"], json!(0));
        assert_eq!(value["proposalTimelines"], json!({}));
    }

    #[test]
    fn a_range_preview_plays_at_its_place_on_the_program() {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=120f\n",
        )
        .unwrap();
        let range = kerf_runtime::RenderRange::new(2.0, 1.5).unwrap();
        let rendered = rendered_preview(&program, "/renders/range.mp4", Some(&range)).unwrap();
        let clip = &rendered.sequences[0].tracks[0].clips[0];
        assert_eq!(clip.start.seconds(), 2.0);
        assert_eq!(clip.duration.seconds(), 1.5);
        let timeline = kerf_playback::PlaybackTimeline::from_project(&rendered).unwrap();
        assert!(timeline.video_at(1.0).is_none());
        assert!(timeline.video_at(2.5).is_some());
    }

    #[test]
    fn stopping_with_no_preview_running_answers_at_once() {
        let shared = shared();
        let (tx, rx) = mpsc::sync_channel(4);
        super::stop_preview(&shared, &tx);
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({"type": "preview", "status": "stopped"})
        );
        assert!(!shared.preview_stopped.load(Ordering::SeqCst));

        // With one running, the render answers once it has stopped.
        let directory = tempfile::tempdir().unwrap();
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        shared.preview_running.store(true, Ordering::SeqCst);
        super::stop_preview(&shared, &tx);
        assert!(marker.exists(), "the render was told to stop");
        assert!(shared.preview_stopped.load(Ordering::SeqCst));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn exiting_a_preview_plays_the_program_again() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(4);
        // Nothing to leave: the program keeps playing as it is.
        super::exit_preview(&shared, &tx);
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({"type": "preview", "status": "exited"})
        );
        assert!(shared.project.lock().unwrap().is_none());

        let file = directory.path().join("render-1.mp4");
        std::fs::write(&file, b"preview").unwrap();
        *shared.model.lock().unwrap() = Some(project(1920, 1080));
        *shared.project.lock().unwrap() = Some(rendered(&file));
        let generation = shared.revision.load(Ordering::SeqCst);
        super::exit_preview(&shared, &tx);
        assert!(matches!(
            *shared.project.lock().unwrap(),
            Some(MediaProject::Native(_))
        ));
        assert!(!file.exists(), "the preview never played, so it goes");
        assert!(shared.revision.load(Ordering::SeqCst) > generation);
        assert_eq!(rx.try_recv().unwrap()["status"], "exited");
    }
}
