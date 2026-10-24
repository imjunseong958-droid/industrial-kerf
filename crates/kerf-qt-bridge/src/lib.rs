//! Qt's nonblocking, UTF-8 C ABI. Canonical edits stay in kerfd; media frames
//! travel through a latest-frame slot rather than JSON or an accumulating queue.
use kerf_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, AgentTool, OpenAiApiMode, ProviderName,
    ReasoningEffort,
};
use kerf_mcp::inbound;
use kerf_mcp::KerfMcpRouter;
use kerf_model::{
    AudioInfo, Catalog, Clip, Content, Effect, Engine, FxKind, Id, Media, Project, Ratio, Sequence,
    Time, Track, TrackKind, VideoInfo,
};
use kerf_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use kerf_runtime::{RuntimeClient, StateSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct AgentSettings {
    #[serde(default)]
    provider: Option<ProviderName>,
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Refresh,
    /// Read the project again even when its document is unchanged, as when
    /// analyses it draws from become ready.
    #[serde(skip)]
    Reload,
    Reconnect,
    Preview,
    Import {
        path: String,
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
    /// Let the live preview's compositor draw on the GPU when that is
    /// faster.
    Gpu {
        enabled: bool,
    },
    Export {
        quality: String,
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
    /// How `effects` would look added to `clip`, on its picture at sequence
    /// time `at`: an `effect_preview` event per raster effect, `width`
    /// pixels wide. A newer request supersedes it.
    EffectPreviews {
        clip: String,
        at: f64,
        effects: Vec<String>,
        width: u32,
    },
    /// Keep a Kerf Cloud token and read its account.
    CloudSignIn {
        token: String,
    },
    CloudSignOut,
    CloudRefresh,
}

/// What the viewer plays: the project itself, or a rendered preview of it.
/// Both have their media paths resolved.
#[derive(Clone)]
enum MediaProject {
    Native(Project),
    Rendered(Project),
}

struct PendingSeek {
    position: f64,
    exact: bool,
}

struct Shared {
    closed: AtomicBool,
    revision: AtomicU64,
    agent_busy: AtomicBool,
    export_busy: AtomicBool,
    abort: Mutex<Option<Arc<AtomicBool>>>,
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
    /// The runtime worker's commands, for work that finishes on its own.
    commands: SyncSender<Command>,
    /// Warp Stabilizer analyses are being made for the preview.
    stabilizing: AtomicBool,
}

struct Bridge {
    events_tx: SyncSender<Value>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    prompts: SyncSender<AgentRequest>,
    events: Receiver<Value>,
    shared: Arc<Shared>,
    displayed_frame: Option<PlaybackFrame>,
}

struct AgentRequest {
    text: String,
    settings: AgentSettings,
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
        match kerf_runtime::parse_document(&state.document) {
            Ok(mut project) => {
                kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
                let cache = kerf_engine::derivatives::DerivativeCache::new(&shared.directory);
                if !kerf_engine::attach_stabilizations(&mut project, &cache).is_empty() {
                    stabilize(project.clone(), shared, tx);
                }
                if let Ok(mut model) = shared.model.lock() {
                    *model = Some(project.clone());
                }
                if let Ok(mut slot) = shared.project.lock() {
                    shared.revision.fetch_add(1, Ordering::SeqCst);
                    *slot = Some(MediaProject::Native(project));
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

/// Make the Warp Stabilizer analyses `project` lacks, one batch at a time,
/// then read the project again so the preview draws from them. Until then
/// those clips play unstabilized.
fn stabilize(project: Project, shared: &Arc<Shared>, tx: &SyncSender<Value>) {
    if shared.stabilizing.swap(true, Ordering::SeqCst) {
        return;
    }
    emit(
        tx,
        json!({"type":"analysis","message":"Analysing shaky clips for Warp Stabilizer…"}),
    );
    let (shared, tx) = (shared.clone(), tx.clone());
    thread::spawn(move || {
        let cache = kerf_engine::derivatives::DerivativeCache::new(&shared.directory);
        let cancellation = kerf_engine::CancellationToken::new();
        let message = match kerf_engine::analyze_stabilizations(&project, &cache, &cancellation) {
            Ok(()) => "Warp Stabilizer analysis is ready".to_owned(),
            Err(error) => format!("Warp Stabilizer analysis failed: {error}"),
        };
        emit(&tx, json!({"type":"analysis","message":message}));
        shared.stabilizing.store(false, Ordering::SeqCst);
        if !shared.closed.load(Ordering::Relaxed) {
            let _ = shared.commands.try_send(Command::Reload);
        }
    });
}

/// A pending proposal's timeline beside the current one, so the editor can
/// show the proposal on the timeline before it is accepted.
fn with_proposal_timeline(mut snapshot: Value, state: &StateSnapshot) -> Value {
    if let Some(kerf_runtime::PendingProposal::Pending { script, .. }) = &state.pending_proposal {
        if let Ok(timeline) = kerf_runtime::proposal_timeline(&state.document, script) {
            snapshot["proposalTimeline"] = timeline;
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
    publish(state, shared, tx, previous);
    connection_event(tx, "ready", None);
    Some(client)
}

fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    media: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    let mut previous = None;
    let mut client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
    while !shared.closed.load(Ordering::Relaxed) {
        let command = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(active_client) = client.as_ref() {
                    match active_client.get_state() {
                        Ok(state) => publish(state, &shared, &tx, &mut previous),
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
        let force_refresh = matches!(&command, Command::Refresh | Command::Reload);
        let result = match command {
            Command::Refresh => active_client.get_state(),
            Command::Reload => {
                // Forgetting the last snapshot reads the project again.
                previous = None;
                active_client.get_state()
            }
            Command::Import { path } => active_client.get_state().and_then(|state| {
                active_client
                    .import_media_request("ui", &operation, &path, state.revision)
                    .map(|(state, _, _)| state)
            }),
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
                    Ok(project) => {
                        let result = project
                            .sequences
                            .first()
                            .ok_or_else(|| "no sequence to export".to_owned())
                            .and_then(|sequence| {
                                kerf_lang::preset::export(
                                    &project,
                                    kerf_model::Catalog::builtin(),
                                    &sequence.id,
                                    &sequence.name,
                                )
                            })
                            .and_then(|document| {
                                kerf_lang::preset::parse(
                                    &document,
                                    kerf_model::Catalog::builtin(),
                                )?;
                                if document.len() > 1024 * 1024 {
                                    return Err("preset exceeds 1 MiB".into());
                                }
                                Ok(document)
                            });
                        match result {
                            Ok(document) => emit(
                                &tx,
                                json!({"type":"preset_export", "path":path, "document":document}),
                            ),
                            Err(message) => error(&tx, message),
                        }
                    }
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
            Command::Preview | Command::Export { .. } => {
                let preview = matches!(command, Command::Preview);
                let quality = match command {
                    Command::Export { quality } => quality,
                    _ => "preview".into(),
                };
                if shared.export_busy.swap(true, Ordering::SeqCst) {
                    error(&tx, "An export is already running");
                    continue;
                }
                let client = active_client.clone();
                let tx = tx.clone();
                let shared = shared.clone();
                thread::spawn(move || {
                    let revision = shared.revision.load(Ordering::SeqCst);
                    let result = export(&client, &quality, preview, &tx);
                    match result {
                        Ok((output, document))
                            if preview && shared.revision.load(Ordering::SeqCst) == revision =>
                        {
                            let rendered = kerf_runtime::parse_document(&document)
                                .ok()
                                .and_then(|program| rendered_preview(&program, &output));
                            if let (Some(rendered), Ok(mut slot)) =
                                (rendered, shared.project.lock())
                            {
                                if shared.revision.load(Ordering::SeqCst) == revision {
                                    *slot = Some(MediaProject::Rendered(rendered));
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            error(&tx, err.message);
                            if preview {
                                emit(
                                    &tx,
                                    json!({"type":"preview","status":"required","message":"Preview render failed. Retry after fixing the reported error."}),
                                );
                            }
                        }
                    }
                    shared.export_busy.store(false, Ordering::SeqCst);
                });
                continue;
            }
            _ => continue,
        };
        match result {
            Ok(state) if force_refresh => publish_force(state, &shared, &tx, &mut previous),
            Ok(state) => publish(state, &shared, &tx, &mut previous),
            Err(error) if error.is_transport() => {
                let _ = media.try_send(Command::Pause);
                connection_event(&tx, "disconnected", Some(&error.message));
                client = None;
            }
            Err(runtime_error) => {
                error(&tx, runtime_error.message);
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

/// Render the program. Returns the output path and the document rendered.
fn export(
    client: &RuntimeClient,
    quality: &str,
    preview: bool,
    tx: &SyncSender<Value>,
) -> Result<(String, String), kerf_runtime::RuntimeError> {
    let event_type = if preview { "preview" } else { "export" };
    let (job, document, output) = client.start_render(quality)?;
    emit(
        tx,
        json!({"type":event_type,"status":"running","progress":0.0}),
    );
    let result = kerf_runtime::run_render_job(
        client,
        &job,
        &document,
        &output,
        quality == "preview",
        |progress| {
            // Progress is replaceable; never stall rendering behind a busy GUI.
            let _ = tx.try_send(json!({"type":event_type,"status":"running","progress":progress}));
        },
    );
    match result {
        Ok(()) => {
            let path = output.to_string_lossy().into_owned();
            if !preview {
                emit(
                    tx,
                    json!({"type":event_type,"status":"completed","path":path,"progress":1.0}),
                );
            }
            Ok((path, document))
        }
        Err(err) => {
            emit(tx, json!({"type":event_type,"status":"failed"}));
            Err(err)
        }
    }
}

/// A project that plays a rendered preview of `program` in its place.
fn rendered_preview(program: &Project, output: &str) -> Option<Project> {
    let sequence = program.sequences.first()?;
    let duration = sequence.duration();
    let id = |text: &str| Id::new(text).ok();
    let media = id("preview")?;
    let clip = |name: &str| {
        Some(Clip::new(
            id(name)?,
            Content::Media {
                media: media.clone(),
                source_in: Time::ZERO,
                speed: Ratio::ONE,
            },
            Time::ZERO,
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
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48_000,
            }),
            reel: None,
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

fn media_worker(rx: Receiver<Command>, tx: SyncSender<Value>, shared: Arc<Shared>) {
    let mut session: Option<PlaybackSession> = None;
    let mut position = 0.0_f64;
    let mut playing = false;
    let mut volume = 1.0_f32;
    // Whether the compositor may draw on the GPU, and the program it draws
    // live, to open again when that changes.
    let mut gpu = true;
    let mut live: Option<Project> = None;
    // The GPU adapter the editor was last told the live preview uses.
    let mut announced: Option<String> = None;
    while !shared.closed.load(Ordering::Relaxed) {
        for command in rx.try_iter() {
            let handle = session.as_ref().map(PlaybackSession::handle);
            let result = match command {
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
                Command::Gpu { enabled } if enabled != gpu => {
                    gpu = enabled;
                    if let (Some(program), Ok(mut slot)) = (live.take(), shared.project.lock()) {
                        // A newer program waiting to open wins.
                        slot.get_or_insert(MediaProject::Native(program));
                    }
                    None
                }
                _ => None,
            };
            if let Some(Err(err)) = result {
                error(&tx, err);
            }
        }
        let project = shared.project.lock().ok().and_then(|mut slot| slot.take());
        if let Some(project) = project {
            let (project, rendered) = match project {
                MediaProject::Native(project) => (project, false),
                MediaProject::Rendered(project) => (project, true),
            };
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
            live = composited.then(|| project.clone());
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
                    announced = gpu.then(kerf_playback::gpu_adapter).flatten();
                    json!({"type":"preview","status":"live","message":message,"gpu":announced})
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
                    .with_composited(composited)
                    .with_gpu(gpu),
            ) {
                Ok(session) => {
                    let _ = session.handle().set_volume(volume);
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
        // The GPU opens in the background; say when the live preview can use
        // it.
        if live.is_some() && gpu && announced.is_none() {
            announced = kerf_playback::gpu_adapter();
            if announced.is_some() {
                emit(&tx, json!({"type":"preview_device","gpu":announced}));
            }
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
                        position = clock.position;
                        let _ = tx.try_send(json!({"type":"clock","position":clock.position,"duration":clock.duration}));
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
                            json!({"type":"clock","position":position,"duration":duration}),
                        );
                    }
                }
            }
        }
        thread::sleep(if session.is_some() {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(4)
        });
    }
}

fn new_agent(
    directory: &std::path::Path,
    binary: &std::path::Path,
    settings: &AgentSettings,
    tx: &SyncSender<Value>,
) -> Result<AgentSession, String> {
    // Kerf Cloud speaks the Anthropic Messages protocol with the account token.
    let cloud = if settings.cloud {
        let store = kerf_cloud::Store::default_path()
            .map(|path| kerf_cloud::Store::load(&path))
            .unwrap_or_default();
        Some(kerf_cloud::model_credentials(&store).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let provider = if cloud.is_some() {
        ProviderName::Anthropic
    } else if let Some(provider) = settings.provider {
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
    let router = KerfMcpRouter::new(directory, Some(binary))?;
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
would say. Do not repeat a step they just took.";

/// Carry out a Kerf Cloud command, if any, then report the account:
/// `{type: "cloud", access, account?, message?}`.
fn cloud_status(tx: &SyncSender<Value>, command: Option<Command>) {
    let Some(path) = kerf_cloud::Store::default_path() else {
        emit(tx, json!({"type":"cloud","access":"unavailable"}));
        return;
    };
    let mut store = kerf_cloud::Store::load(&path);
    let mut message = None;
    match command {
        Some(Command::CloudSignIn { token }) => store.token = Some(token.trim().to_string()),
        Some(Command::CloudSignOut) => store.token = None,
        _ => {}
    }
    let changed = store != kerf_cloud::Store::load(&path);
    if changed {
        if let Err(error) = store.save(&path) {
            message = Some(error.to_string());
        }
    }
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
                store.token = None;
                let _ = store.save(&path);
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
    } else {
        "MCP server unavailable; skipped. Check its local command and configuration."
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
    let mut session: Option<(AgentSettings, AgentSession)> = None;
    while let Ok(request) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        emit(&tx, json!({"type":"agent_busy","busy":true}));
        emit(&tx, json!({"type":"agent_state","state":"running"}));
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
            current
                .run_with_events(&request.text, |event| {
                    if let AgentEvent::ToolCallFinished { summary, .. } = &event {
                        if summary.status.as_deref() == Some("awaiting_review") {
                            emit(&tx, json!({"type":"agent_state","state":"awaiting_review"}));
                        }
                    }
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
                    emit(&tx, json!({"type":"agent","kind":kind,"text":text}));
                })
                .map_err(|error| error.to_string())?;
            emit(&tx, json!({"type":"agent_state","state":"completed"}));
            Ok::<_, String>(())
        })();
        if let Err(err) = result {
            emit(
                &tx,
                json!({"type":"agent_state","state":"failed","message":err}),
            );
            error(&tx, err);
        }
        if let Ok(mut abort) = shared.abort.lock() {
            *abort = None;
        }
        shared.agent_busy.store(false, Ordering::SeqCst);
        emit(&tx, json!({"type":"agent_busy","busy":false}));
        let _ = commands.send(Command::Refresh);
    }
}

/// Emit how each of `effects` would look added to `clip`: its picture at
/// sequence time `at`, `width` pixels wide, through the clip's own raster
/// effects and then the effect, as RGBA. Graph effects have no preview.
/// Stops when a newer request than `request` arrives.
fn effect_previews(
    shared: &Shared,
    request: u64,
    clip: &str,
    at: f64,
    effects: &[String],
    width: u32,
    tx: &SyncSender<Value>,
) -> Result<(), String> {
    use base64::Engine as _;
    let project = shared
        .model
        .lock()
        .map_err(|_| "the project is unavailable".to_owned())?
        .clone()
        .ok_or_else(|| "no project is open".to_owned())?;
    let found = project
        .clip(clip)
        .ok_or_else(|| format!("no clip `{clip}`"))?;
    let Content::Media {
        media,
        source_in,
        speed,
    } = &found.content
    else {
        return Ok(());
    };
    let Some(media) = project.media(media.as_str()) else {
        return Ok(());
    };
    let Some(video) = media.video else {
        return Ok(());
    };
    let at = Time::from_seconds(at)
        .unwrap_or(Time::ZERO)
        .clamp(found.start, found.end() - media.rate.frame_duration());
    let clip_time = at - found.start;
    let source = if speed.is_reverse() {
        *source_in + speed.source_span(found.duration - clip_time)
    } else {
        *source_in + speed.source_span(clip_time)
    };
    let width = width.clamp(16, 640) & !1;
    let height = ((u64::from(width) * u64::from(video.height) / u64::from(video.width.max(1)))
        as u32)
        .max(2)
        & !1;
    let output = std::process::Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(kerf_engine::program::secs(source))
        .arg("-i")
        .arg(&media.path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={width}:{height},format=rgba"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .map_err(|err| err.to_string())?;
    let size = width as usize * height as usize * 4;
    if !output.status.success() || output.stdout.len() < size {
        return Err(format!("no picture of `{clip}` at that time"));
    }
    let picture = &output.stdout[..size];
    let own: Vec<Effect> = found
        .effects
        .iter()
        .filter(|effect| {
            effect.enabled
                && Catalog::builtin()
                    .fx(effect.fx.as_str())
                    .is_some_and(|fx| fx.engine == Engine::Raster)
        })
        .cloned()
        .collect();
    let seed = kerf_core::raster::seed(clip);
    for id in effects {
        if shared.previews.load(Ordering::SeqCst) != request {
            return Ok(());
        }
        let Some(fx) = Catalog::builtin()
            .fx(id)
            .filter(|fx| fx.engine == Engine::Raster && fx.kind == FxKind::Effect)
        else {
            continue;
        };
        let mut stack = own.clone();
        stack.push(Effect {
            fx: fx.id.clone(),
            enabled: true,
            params: Default::default(),
        });
        let Ok(stack) = kerf_fx::RasterStack::new(&stack, video.width, video.height, seed) else {
            continue;
        };
        let mut frame = picture.to_vec();
        stack.apply(&mut frame, width, height, clip_time, media.rate.fps());
        emit(
            tx,
            json!({
                "type": "effect_preview",
                "clip": clip,
                "fx": id,
                "width": width,
                "height": height,
                "rgba": base64::engine::general_purpose::STANDARD.encode(&frame),
            }),
        );
    }
    Ok(())
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
    let (commands, rx) = mpsc::sync_channel(32);
    let (media, media_rx) = mpsc::sync_channel(32);
    let (prompts, prompt_rx) = mpsc::sync_channel(1);
    let (emit, events) = mpsc::sync_channel(256);
    let shared = Arc::new(Shared {
        directory: directory.clone(),
        revision: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        agent_busy: AtomicBool::new(false),
        export_busy: AtomicBool::new(false),
        abort: Mutex::new(None),
        frame: Mutex::new(None),
        seek: Mutex::new(None),
        project: Mutex::new(None),
        model: Mutex::new(None),
        previews: AtomicU64::new(0),
        commands: commands.clone(),
        stabilizing: AtomicBool::new(false),
    });
    {
        let directory = directory.clone();
        let binary = binary.clone();
        let media = media.clone();
        let tx = emit.clone();
        let shared = shared.clone();
        thread::spawn(move || runtime_worker(directory, binary, rx, media, tx, shared));
    }
    {
        let tx = emit.clone();
        let shared = shared.clone();
        thread::spawn(move || media_worker(media_rx, tx, shared));
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
        prompts,
        events,
        shared,
        displayed_frame: None,
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
    if let Ok(abort) = bridge.shared.abort.lock() {
        if let Some(abort) = &*abort {
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
            if let Ok(mut slot) = bridge.shared.seek.try_lock() {
                *slot = Some(PendingSeek {
                    position,
                    exact: !scrub,
                });
                true
            } else {
                false
            }
        }
        Command::Play | Command::Pause | Command::Volume { .. } | Command::Gpu { .. } => {
            bridge.media.try_send(command).is_ok()
        }
        Command::Abort => {
            if let Ok(abort) = bridge.shared.abort.try_lock() {
                if let Some(abort) = &*abort {
                    abort.store(true, Ordering::SeqCst);
                }
                true
            } else {
                false
            }
        }
        Command::EffectPreviews {
            clip,
            at,
            effects,
            width,
        } => {
            let request = bridge.shared.previews.fetch_add(1, Ordering::SeqCst) + 1;
            let shared = bridge.shared.clone();
            let tx = bridge.events_tx.clone();
            thread::spawn(move || {
                if let Err(message) =
                    effect_previews(&shared, request, &clip, at, &effects, width, &tx)
                {
                    error(&tx, message);
                }
            });
            true
        }
        Command::CloudSignIn { .. } | Command::CloudSignOut | Command::CloudRefresh => {
            let tx = bridge.events_tx.clone();
            thread::spawn(move || cloud_status(&tx, Some(command)));
            true
        }
        Command::Agent {
            text,
            focus,
            settings,
        } => {
            if text.trim().is_empty() || bridge.shared.agent_busy.swap(true, Ordering::SeqCst) {
                return false;
            }
            let text = match &focus {
                Some(focus) => kerf_agent::focused_request(&text, focus),
                None => text,
            };
            if bridge
                .prompts
                .try_send(AgentRequest { text, settings })
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

// ---- command registry ----------------------------------------------------------

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
        preview_frame_size, publish, publish_force, rendered_preview, AgentSettings, Command,
        Shared,
    };
    use kerf_runtime::{EditOp, PendingProposal, ProposalFreshness, StateSnapshot};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    fn shared() -> Shared {
        Shared {
            directory: std::env::temp_dir(),
            revision: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            agent_busy: AtomicBool::new(false),
            export_busy: AtomicBool::new(false),
            abort: Mutex::new(None),
            frame: Mutex::new(None),
            seek: Mutex::new(None),
            project: Mutex::new(None),
            model: Mutex::new(None),
            previews: AtomicU64::new(0),
            commands: mpsc::sync_channel(1).0,
            stabilizing: AtomicBool::new(false),
        }
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

    fn snapshot(document: &str, pending_proposal: Option<PendingProposal>) -> StateSnapshot {
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
                provider: Some(kerf_agent::ProviderName::Openai),
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
        let rendered = rendered_preview(&program, "/renders/preview.mp4").unwrap();
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
}
