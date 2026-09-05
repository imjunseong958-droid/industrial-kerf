//! Qt's nonblocking, UTF-8 C ABI. Canonical edits stay in cutd; media frames
//! travel through a latest-frame slot rather than JSON or an accumulating queue.
use cut_agent::{AgentEvent, AgentSession, AgentSessionOptions, OpenAiApiMode, ProviderName};
use cut_mcp::{CutMcpRouter, McpProfile};
use cut_playback::{PlaybackConfig, PlaybackEvent, PlaybackFrame, PlaybackSession, PlaybackState};
use cut_runtime::{RuntimeClient, StateSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Refresh,
    Preview,
    Import {
        path: String,
    },
    Apply {
        command: Value,
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
    },
    Abort,
    Play,
    Pause,
    Seek {
        position: f64,
    },
    Volume {
        value: f32,
    },
    Export {
        quality: String,
    },
}

enum MediaProject {
    Native(Value),
    Rendered(Value),
}

struct Shared {
    closed: AtomicBool,
    revision: AtomicU64,
    agent_busy: AtomicBool,
    export_busy: AtomicBool,
    abort: Mutex<Option<Arc<AtomicBool>>>,
    frame: Mutex<Option<PlaybackFrame>>,
    seek: Mutex<Option<f64>>,
    project: Mutex<Option<MediaProject>>,
}

struct Bridge {
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    prompts: SyncSender<String>,
    events: Receiver<Value>,
    shared: Arc<Shared>,
    displayed_frame: Option<PlaybackFrame>,
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
    // Proposal-only refreshes must not reopen decoders or interrupt playback.
    if previous.as_ref() != Some(&state.project) {
        if let Ok(mut slot) = shared.project.lock() {
            shared.revision.fetch_add(1, Ordering::SeqCst);
            *slot = Some(MediaProject::Native(state.project.clone()));
        }
        *previous = Some(state.project.clone());
    }
    emit(tx, json!({"type":"state", "snapshot":state}));
}

fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    let client = match RuntimeClient::connect_or_spawn(&directory, Some(&binary)) {
        Ok(client) => client,
        Err(err) => {
            error(&tx, err.message);
            return;
        }
    };
    let mut previous = None;
    match client.get_state() {
        Ok(state) => publish(state, &shared, &tx, &mut previous),
        Err(err) => error(&tx, err.message),
    }
    while let Ok(command) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        let operation = key();
        let result = match command {
            Command::Refresh => client.get_state(),
            Command::Import { path } => client.get_state().and_then(|state| {
                client
                    .import_media_request("ui", &operation, &path, state.revision, None, None)
                    .map(|(state, _, _)| state)
            }),
            Command::Apply { command, revision } => {
                client.apply_request("ui", &operation, command, revision)
            }
            Command::Undo { revision } => client.undo_request("ui", &operation, revision),
            Command::Redo { revision } => client.redo_request("ui", &operation, revision),
            Command::Accept {
                proposal_id,
                revision,
            } => client.accept_proposal(&proposal_id, "ui", &operation, revision),
            Command::Reject {
                proposal_id,
                revision,
            } => client.reject_proposal_request(&proposal_id, "ui", &operation, revision),
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
                let client = client.clone();
                let tx = tx.clone();
                let shared = shared.clone();
                thread::spawn(move || {
                    let revision = shared.revision.load(Ordering::SeqCst);
                    let result = export(&client, &quality, preview, &tx);
                    match result {
                        Ok((output, project, duration))
                            if preview && shared.revision.load(Ordering::SeqCst) == revision =>
                        {
                            let rendered = json!({"version":2,"fps":project["fps"],"width":project["width"],"height":project["height"],"sources":[{"id":"preview","path":output,"duration":duration,"hasAudio":true}],"tracks":[{"id":"preview-track","type":"video","clips":[{"id":"preview-clip","sourceId":"preview","in":0,"out":duration,"timelineStart":0}]}]});
                            if let Ok(mut slot) = shared.project.lock() {
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
            Ok(state) => publish(state, &shared, &tx, &mut previous),
            Err(err) => {
                error(&tx, err.message);
                if let Ok(state) = client.get_state() {
                    publish(state, &shared, &tx, &mut previous);
                }
            }
        }
    }
}

fn export(
    client: &RuntimeClient,
    quality: &str,
    preview: bool,
    tx: &SyncSender<Value>,
) -> Result<(String, Value, f64), cut_runtime::RuntimeError> {
    let duration = client.get_state()?.duration;
    let event_type = if preview { "preview" } else { "export" };
    let (job, project, output) = client.start_render(quality)?;
    emit(
        tx,
        json!({"type":event_type,"status":"running","progress":0.0}),
    );
    let cancel = cut_runtime::render_cancel_path(client.directory(), &job.job_id);
    let result = cut_runtime::run_render_with_cancel_file_progress(
        job.job_id.clone(),
        project.clone(),
        output.clone(),
        quality == "preview",
        None,
        Some(cancel),
        |progress| {
            // Progress is replaceable; never stall rendering behind a busy GUI.
            let _ = tx.try_send(json!({"type":event_type,"status":"running","progress":progress}));
        },
    );
    match result {
        Ok(()) => {
            let path = output.to_string_lossy().into_owned();
            client.update_job_with_progress(
                &job.job_id,
                "completed",
                None,
                Some(path.clone()),
                Some(1.0),
            )?;
            if !preview {
                emit(
                    tx,
                    json!({"type":event_type,"status":"completed","path":path,"progress":1.0}),
                );
            }
            Ok((path, project, duration))
        }
        Err(err) => {
            let _ = client.update_job(&job.job_id, "failed", Some(err.message.clone()), None);
            emit(tx, json!({"type":event_type,"status":"failed"}));
            Err(err)
        }
    }
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
            let plan = cut_core::preview::analyze_project(&project, None, None);
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
            let width = project["width"].as_u64().unwrap_or(1920).clamp(2, 16384) as u32;
            let height = project["height"].as_u64().unwrap_or(1080).clamp(2, 16384) as u32;
            let scale = 1280.0 / f64::from(width.max(height)).max(1280.0);
            let size = (
                (f64::from(width) * scale).round() as u32,
                (f64::from(height) * scale).round() as u32,
            );
            // A new worker prevents old-project frames from resurfacing after an edit.
            session = match PlaybackSession::open_json(
                &project,
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
                    Command::Volume { value } => { volume = value; handle.set_volume(value) },
                    _ => continue,
                };
                if let Err(err) = result {
                    error(&tx, err);
                }
            }
            let seek = shared.seek.lock().ok().and_then(|mut slot| slot.take());
            if let Some(position) = seek {
                // A frame decoded before this seek belongs to the old target.
                // Clear the replaceable slot before asking the session to
                // decode so a hover skim cannot briefly show stale media.
                if let Ok(mut frame) = shared.frame.lock() {
                    *frame = None;
                }
                if let Err(err) = handle.seek(position) {
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
                    },
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
        thread::sleep(Duration::from_millis(4));
    }
}

fn new_agent(
    directory: &std::path::Path,
    binary: &std::path::Path,
) -> Result<AgentSession, String> {
    let provider = match std::env::var("CUT_PROVIDER").ok().as_deref() {
        Some("openai") => ProviderName::Openai,
        Some("anthropic") => ProviderName::Anthropic,
        Some(_) => return Err("CUT_PROVIDER must be openai or anthropic".into()),
        None if std::env::var_os("OPENAI_API_KEY").is_some() => ProviderName::Openai,
        _ => ProviderName::Anthropic,
    };
    let key_name = if provider == ProviderName::Openai {
        "OPENAI_API_KEY"
    } else {
        "ANTHROPIC_API_KEY"
    };
    let api_key = std::env::var(key_name)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Set {key_name} before sending an agent prompt"))?;
    let api_mode = match std::env::var("OPENAI_API_MODE").ok().as_deref() {
        Some("responses") => OpenAiApiMode::Responses,
        Some("chat") | None => OpenAiApiMode::Chat,
        _ => return Err("OPENAI_API_MODE must be chat or responses".into()),
    };
    let reasoning_effort = std::env::var("OPENAI_REASONING_EFFORT")
        .ok()
        .map(|value| {
            serde_json::from_value(Value::String(value))
                .map_err(|_| "Invalid OPENAI_REASONING_EFFORT".to_string())
        })
        .transpose()?;
    let router = CutMcpRouter::new(directory, McpProfile::Full, Some(binary))?;
    Ok(AgentSession::new(AgentSessionOptions {
        provider,
        api_mode,
        reasoning_effort,
        api_key: Some(api_key),
        model: std::env::var("CUT_MODEL").ok(),
        base_url: std::env::var(if provider == ProviderName::Openai {
            "OPENAI_BASE_URL"
        } else {
            "ANTHROPIC_BASE_URL"
        })
        .ok(),
        skills: router.skills().to_vec(),
        tools: router.agent_tools(),
        ..Default::default()
    }))
}

fn agent_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<String>,
    commands: SyncSender<Command>,
    tx: SyncSender<Value>,
    shared: Arc<Shared>,
) {
    // Keep one session for follow-up feedback, rather than forgetting every turn.
    let mut session = None;
    while let Ok(prompt) = rx.recv() {
        if shared.closed.load(Ordering::Relaxed) {
            break;
        }
        emit(&tx, json!({"type":"agent_busy","busy":true}));
        let result = (|| {
            if session.is_none() {
                session = Some(new_agent(&directory, &binary)?);
            }
            let current = session.as_mut().ok_or("Agent session unavailable")?;
            if let Ok(mut abort) = shared.abort.lock() {
                *abort = Some(current.abort_handle());
            }
            current
                .run_with_events(&prompt, |event| {
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
                                    .unwrap_or_default()
                            ),
                        ),
                        AgentEvent::TurnStarted { .. } => ("status", "Planning next step…".into()),
                        AgentEvent::Done { text } => ("done", text),
                        AgentEvent::Aborted => ("status", "Stopped".into()),
                    };
                    emit(&tx, json!({"type":"agent","kind":kind,"text":text}));
                })
                .map_err(|_| {
                    "Agent request failed. Check provider configuration and connection.".to_string()
                })?;
            Ok::<_, String>(())
        })();
        if let Err(err) = result {
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
        let tx = emit.clone();
        let shared = shared.clone();
        thread::spawn(move || runtime_worker(directory, binary, rx, tx, shared));
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
        Command::Seek { position } => {
            if !position.is_finite() || position < 0.0 {
                return false;
            }
            if let Ok(mut slot) = bridge.shared.seek.try_lock() {
                *slot = Some(position);
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
        Command::Agent { text } => {
            if text.trim().is_empty() || bridge.shared.agent_busy.swap(true, Ordering::SeqCst) {
                return false;
            }
            if bridge.prompts.try_send(text).is_err() {
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
