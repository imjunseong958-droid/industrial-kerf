//! Qt's nonblocking, UTF-8 C ABI. Canonical edits stay in cutd; media frames
//! travel through a latest-frame slot rather than JSON or an accumulating queue.
use cut_agent::{
    AgentEvent, AgentSession, AgentSessionOptions, OpenAiApiMode, ProviderName, ReasoningEffort,
};
use cut_mcp::inbound;
use cut_mcp::CutMcpRouter;
use cut_model::{
    AudioInfo, Clip, Content, Id, Media, Project, Ratio, Sequence, Time, Track, TrackKind,
    VideoInfo,
};
use cut_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use cut_runtime::{RuntimeClient, StateSnapshot};
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
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Refresh,
    Reconnect,
    Preview,
    Import {
        path: String,
    },
    /// Typed edits on the model, applied atomically.
    Edit {
        ops: Vec<cut_runtime::EditOp>,
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
    Export {
        quality: String,
    },
}

/// What the viewer plays: the project itself, or a rendered preview of it.
/// Both have their media paths resolved.
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
    /// The project directory, which relative media paths are resolved
    /// against.
    directory: PathBuf,
}

struct Bridge {
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
    shared: &Shared,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, false);
}

fn publish_force(
    state: StateSnapshot,
    shared: &Shared,
    tx: &SyncSender<Value>,
    previous: &mut Option<Value>,
) {
    publish_snapshot(state, shared, tx, previous, true);
}

fn publish_snapshot(
    state: StateSnapshot,
    shared: &Shared,
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
        match cut_runtime::parse_document(&state.document) {
            Ok(mut project) => {
                cut_runtime::resolve_media_paths(&mut project, &shared.directory);
                if let Ok(mut slot) = shared.project.lock() {
                    shared.revision.fetch_add(1, Ordering::SeqCst);
                    *slot = Some(MediaProject::Native(project));
                }
            }
            Err(err) => error(tx, err.message),
        }
    }
    *previous = Some(snapshot);
    emit(tx, json!({"type":"state", "snapshot":state}));
}

fn connection_event(tx: &SyncSender<Value>, status: &str, message: Option<&str>) {
    let mut event = json!({"type":"connection", "status":status});
    if let Some(message) = message {
        event["message"] = Value::String(message.to_owned());
    }
    emit(tx, event);
}

fn is_transport_error(error: &cut_runtime::RuntimeError) -> bool {
    matches!(
        error.code.as_str(),
        "IPC_UNSUPPORTED"
            | "IPC_CONNECT_ERROR"
            | "IPC_PERMISSION_DENIED"
            | "IPC_TIMEOUT"
            | "IPC_ERROR"
            | "RUNTIME_UNAVAILABLE"
    )
}

fn connect_runtime(
    directory: &std::path::Path,
    binary: &std::path::Path,
    tx: &SyncSender<Value>,
    shared: &Shared,
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
                        Err(error) if is_transport_error(&error) => {
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
        let result = match command {
            Command::Refresh => active_client.get_state(),
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
                            let rendered = cut_runtime::parse_document(&document)
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
            Err(error) if is_transport_error(&error) => {
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
) -> Result<(String, String), cut_runtime::RuntimeError> {
    let event_type = if preview { "preview" } else { "export" };
    let (job, document, output) = client.start_render(quality)?;
    emit(
        tx,
        json!({"type":event_type,"status":"running","progress":0.0}),
    );
    let result = cut_runtime::run_render_job(
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
    while !shared.closed.load(Ordering::Relaxed) {
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
            let plan = cut_playback::preview_plan(&project);
            if !rendered && !plan.native_exact() {
                session = None;
                playing = false;
                emit(&tx, json!({"type":"playback","playing":false}));
                emit(
                    &tx,
                    json!({"type":"preview","status":if plan.unavailable() { "unavailable" } else { "required" }, "message":plan.reasons.iter().map(|reason| reason.message.as_str()).collect::<Vec<_>>().join("; ")}),
                );
                thread::sleep(Duration::from_millis(4));
                continue;
            }
            emit(&tx, json!({"type":"preview","status":"ready"}));
            let size = preview_frame_size(&project);
            // A new worker prevents old-project frames from resurfacing after an edit.
            session = match PlaybackSession::open(
                project,
                PlaybackConfig::default().with_frame_size(size.0, size.1),
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
        if let Some(session) = &session {
            let handle = session.handle();
            for command in rx.try_iter() {
                let result = match command {
                    Command::Play => {
                        playing = true;
                        handle.play()
                    }
                    Command::Pause => {
                        playing = false;
                        handle.pause()
                    }
                    Command::Volume { value } => {
                        volume = value;
                        handle.set_volume(value)
                    }
                    _ => continue,
                };
                if let Err(err) = result {
                    error(&tx, err);
                }
            }
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
    let provider = if let Some(provider) = settings.provider {
        provider
    } else {
        match std::env::var("CUT_PROVIDER").ok().as_deref() {
            Some(name) => {
                ProviderName::parse(name).ok_or("CUT_PROVIDER must be openai or anthropic")?
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
    let api_key = settings
        .api_key
        .clone()
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
    let router = CutMcpRouter::new(directory, Some(binary))?;
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
        "Could not load MCP configuration. Check ~/.cut/mcp.json and project .cut/mcp.json."
            .to_owned()
    })?;
    Ok(AgentSession::new(AgentSessionOptions {
        provider,
        api_mode,
        reasoning_effort,
        api_key: Some(api_key),
        model: settings
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .or_else(|| std::env::var("CUT_MODEL").ok()),
        base_url: settings
            .base_url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| {
                std::env::var(if provider == ProviderName::Openai {
                    "OPENAI_BASE_URL"
                } else {
                    "ANTHROPIC_BASE_URL"
                })
                .ok()
            }),
        system_prompt: Some(router.agent_system_prompt()),
        tools,
        context: Some(router.agent_context()),
        ..Default::default()
    }))
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

/// Start workers without waiting for project I/O.
/// # Safety
/// Both arguments must be live, NUL-terminated UTF-8 strings for this call.
#[no_mangle]
pub unsafe extern "C" fn cut_bridge_new(
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
    Box::into_raw(Box::new(Bridge {
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
/// `bridge` must be a live pointer returned by `cut_bridge_new`, freed only once.
#[no_mangle]
pub unsafe extern "C" fn cut_bridge_free(bridge: *mut c_void) {
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
pub unsafe extern "C" fn cut_bridge_send(bridge: *mut c_void, message: *const c_char) -> bool {
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
        Command::Play | Command::Pause | Command::Volume { .. } => {
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
        Command::Agent { text, settings } => {
            if text.trim().is_empty() || bridge.shared.agent_busy.swap(true, Ordering::SeqCst) {
                return false;
            }
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
pub unsafe extern "C" fn cut_bridge_poll(bridge: *mut c_void) -> *mut c_char {
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

/// Release an event string returned by `cut_bridge_poll`.
/// # Safety
/// `value` must be null or an unfreed pointer returned by `cut_bridge_poll`.
#[no_mangle]
pub unsafe extern "C" fn cut_bridge_string_free(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: Ownership is returned exactly once by the caller.
        drop(unsafe { CString::from_raw(value) });
    }
}

/// A borrowed RGBA frame; bytes live until the next frame poll or bridge free.
#[repr(C)]
pub struct CutFrame {
    pub pixels: *const u8,
    pub length: usize,
    pub width: u32,
    pub height: u32,
    pub time: f64,
}

/// Take the newest frame without copying its pixels.
/// # Safety
/// `bridge` must be live and exclusively accessed. `output` must point to a
/// writable `CutFrame`. Previously returned pixels expire on this call.
#[no_mangle]
pub unsafe extern "C" fn cut_bridge_frame(bridge: *mut c_void, output: *mut CutFrame) -> bool {
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
        *output = CutFrame {
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
    use cut_runtime::{PendingProposal, ProposalFreshness, StateSnapshot};
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
        }
    }

    const DOCUMENT: &str = "cut 1\nproject name=\"Bridge\"\n\
        sequence s1 rate=24 size=1920x1080 audio=48000\n";

    fn project(width: u32, height: u32) -> cut_model::Project {
        cut_runtime::parse_document(&format!(
            "cut 1\nproject name=\"Bridge\"\n\
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
        let Command::Agent { text, settings } = command else {
            panic!("expected agent command");
        };
        assert_eq!(text, "Trim the opening");
        assert_eq!(
            settings,
            AgentSettings {
                provider: Some(cut_agent::ProviderName::Openai),
                api_mode: Some(cut_agent::OpenAiApiMode::Responses),
                api_key: Some("test-key".into()),
                base_url: Some("http://localhost:9000/v1".into()),
                model: Some("local-model".into()),
                mcp_enabled: true,
                mcp_config_version: 7,
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
            preview_frame_size(&cut_model::Project::default()),
            (1920, 1080)
        );
    }

    #[test]
    fn preview_frame_size_limits_only_the_longest_edge_for_large_canvases() {
        assert_eq!(preview_frame_size(&project(8000, 4000)), (1920, 960));
    }

    #[test]
    fn a_rendered_preview_plays_in_place_of_the_program() {
        let program = cut_runtime::parse_document(
            "cut 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n\
             \x20     set crop_l=0.25\n",
        )
        .unwrap();
        assert_eq!(
            cut_playback::preview_plan(&program).mode,
            cut_core::preview::PreviewMode::RenderedFallback
        );
        let rendered = rendered_preview(&program, "/renders/preview.mp4").unwrap();
        assert!(cut_model::validate::errors(&rendered, cut_model::Catalog::builtin()).is_empty());
        assert_eq!(
            rendered.sequences[0].duration(),
            program.sequences[0].duration()
        );
        let timeline = cut_playback::PlaybackTimeline::from_project(&rendered).unwrap();
        assert_eq!(
            timeline.video_at(1.0).unwrap().source.path.to_str(),
            Some("/renders/preview.mp4")
        );
        assert_eq!(timeline.audio_at(1.0).len(), 1);
    }
}
