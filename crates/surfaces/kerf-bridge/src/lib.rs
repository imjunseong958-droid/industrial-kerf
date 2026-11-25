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
//! viewer preview rendering, the QC check, the makings it started, the render queue's jobs
//! and the renders its agent's tools started are stopped. A making another program runs on
//! the project goes on.
//! The threads it starts itself are named for what they do and counted
//! ([`Bridge::workers`]), so that whoever let it go can find out that they are done.
//!
//! One thing an editor waits for, just before its process exits, with [`drawing_settled`]:
//! the frames a closed bridge was still drawing, and the GPU a frame had begun to bring up.
//! A process that exits under a thread at work in the GPU's libraries now and then dies as
//! it ends. The frames are its playback's, its effect previews', its scope signal's, its
//! shot pictures', its take readings', its renders' (the viewer's preview and the render
//! queue's jobs), its QC check's, its makings' and those of the tools its agent runs in this
//! process: a closed bridge begins none of them, and a render, a check or a making it or a
//! tool was on is cancelled, so the wait is for the frames under way and for what was
//! cancelled to stop. A tool call under way is the one thing with nothing to stop it: it is
//! waited for to its end, for as long as the editor will wait.
//!
//! # Commands and events
//!
//! [`Command`] and [`Event`] are the protocol: each variant says what it asks or tells,
//! what answers a command, and when one is refused. On the wire each is one JSON object
//! whose `type` names it, such as `{"type": "seek", "position": 2.5}`; [`commands_run`]
//! gives commands in that form. An event of a type this build does not know reads as
//! [`Event::Unknown`].
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
//! - The catalog: [`fx_catalog_all`], [`clip_properties`].
//! - [`about`]; [`update_check`], which asks the network: call it off the UI thread.
//! - [`ffmpeg_bin`]: the FFmpeg kerfd runs, for the editor's own pictures (found once, by
//!   running the candidates: call it off the UI thread the first time).
//! - Crash reports: [`crash_install`], [`crash_reports`].
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
//! - `{type: "tools"}`, which finds what there is to share a project with.
//!
//! Its [`TaskStatus`] is running (with what it is doing, where it says), done (the project
//! made or cloned, or the tools found: a [`TaskDone`]), or failed, and why.

// No unsafe code in the library; the workspace's `deny` would let an item allow it, this does not.
#![forbid(unsafe_code)]

mod agent;
mod analysis;
mod claude_code;
mod cloud_session;
mod collab;
mod command;
mod conversations;
mod error;
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
mod runtime;
mod secrets;
mod sessions;
mod standalone;
mod steps;
mod takes;
mod tasks;
#[cfg(test)]
mod tests_support;
mod tools;
mod tracks;
mod updates;
mod viewer;
mod wake;
mod workers;
mod workflows;

pub use claude_code::SERVE_MCP;
pub use command::{AgentSettings, ChatProvider, Command, CommandName, UnnamedCommand};
pub use conversations::Request as ConversationsRequest;
pub use event::{Event, Snapshot};
pub use kerf_agent::{Focus, FocusScope, OpenAiApiMode};
pub use kerf_collab::git::Tools as GitTools;
pub use kerf_collab::scaffold::Created as CreatedProject;
pub use kerf_collab::scaffold::GitHubRepository;
pub use kerf_playback::drawing_settled;
pub use kerf_runtime::{
    EditKey, EditOp, EditParam, GeneratorSpec, MaskOutline, RateSpec, TrimSide,
};
pub use media::CastEntry;
pub use paperwork::CaptionsRequest;
pub use previews::{PreviewEffect, Request as EffectPreviewsRequest};
pub use queue::ExportRequest;
pub use recipes::Edit as RecipeEdit;
pub use standalone::{
    about, clip_properties, commands_catalog, commands_run, commands_states, crash_install,
    crash_reports, ffmpeg_bin, fx_catalog_all, keychain_off, preset_inspect, project_inspect,
    secret, serve_mcp, update_check, About, CrashReport, CrashReports, REPOSITORY,
};
pub use tasks::{CheckedFile, PresetCheck, ProjectInfo, Task, TaskDone, TaskStatus};
pub use tracks::{Follow as ShotFollow, Of as ShotOf};
pub use updates::UpdateCheck;
pub use wake::Wake;
pub use workers::Workers;
pub use workflows::Edit as WorkflowEdit;

use agent::{agent_worker, permission_mode, Agent};
use event::{CloudAccess, CloudStatus, ErrorEvent};
use kerf_mcp::permissions::Mode;
use kerf_model::Project;
use runtime::runtime_worker;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use viewer::media_worker;
use viewer::preview::{cancel_preview, discard_previews, exit_preview, stop_preview};
use wake::Outbox;

/// What the viewer plays: the project itself, or a rendered preview of it,
/// the file that preview plays, and the proposal it previews. Both have
/// their media paths resolved.
#[derive(Clone)]
enum MediaProject {
    Native(Project),
    Rendered(Project, PathBuf, Option<String>),
}

struct PendingSeek {
    position: f64,
    exact: bool,
}

/// What a bridge's workers share. What only one kind of work touches is that work's own,
/// in a group its module keeps; what is left here is the bridge's as a whole.
struct Shared {
    closed: AtomicBool,
    revision: AtomicU64,
    /// The project directory, which relative media paths are resolved
    /// against.
    directory: PathBuf,
    /// The project as edited, its media paths resolved.
    model: Mutex<Option<Project>>,
    /// Kerf's agent: whether a turn runs, and what stops it.
    agent: agent::Turns,
    /// What the viewer plays, shows and was asked to seek to.
    viewer: viewer::Showing,
    /// The viewer's preview renders.
    preview: viewer::preview::Previews,
    /// The latest effect preview request; older ones stop.
    effect_previews: AtomicU64,
    /// The media files, as last seen, and the copies of them under way.
    media: media::Files,
    /// The QC check.
    check: qc::Running,
    /// The makings running.
    makings: media::Makings,
    /// The render queue the export thread works through.
    renders: queue::Queue,
    /// The renders Kerf's own tools started in this process and are still on.
    tool_renders: tools::Renders,
    /// The threads the bridge started itself and whose work is not over.
    workers: Workers,
    /// Loudness, media info and media index work.
    analysis: analysis::Analysis,
    /// The workflow last said of each step a graph has open.
    workflows: workflows::Said,
    /// What lets the bridge draw a frame with the compositor in this process, for its
    /// playback, an effect preview, the scope signal, a shot picture or follow, a take
    /// reading, a render, a QC check, a making, and what Kerf's own tools do here for its
    /// agent ([`tools`]). Closed with the bridge, so that [`drawing_settled`] can wait for
    /// the ones under way before the process exits.
    drawers: kerf_playback::Drawers,
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
    // Nobody takes events any more only once the editor has closed the bridge.
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

/// Carry out a Kerf Cloud command, if any, then report the account:
/// `{type: "cloud", access, account?, message?}`.
fn cloud_status(tx: &Outbox, command: Option<Command>) {
    let Some(path) = kerf_cloud::Store::default_path() else {
        emit(
            tx,
            Event::Cloud(CloudStatus {
                access: CloudAccess::Unavailable,
                account: None,
                message: None,
            }),
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
                if let Err(error) = cloud_session::keep_token(&path, &keychain, None) {
                    log::warn!("the token Kerf Cloud refused was not forgotten: {error}");
                }
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
        Event::Cloud(CloudStatus {
            access,
            account,
            message,
        }),
    );
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
            closed: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            directory: directory.clone(),
            model: Mutex::new(None),
            agent: Default::default(),
            viewer: Default::default(),
            preview: Default::default(),
            effect_previews: AtomicU64::new(0),
            media: Default::default(),
            check: Default::default(),
            makings: Default::default(),
            renders: queue::Queue::default(),
            tool_renders: Default::default(),
            workers: Workers::new(),
            analysis: analysis::Analysis::default(),
            workflows: Default::default(),
            drawers: Default::default(),
        });
        let workers = shared.workers.clone();
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let commands = commands.clone();
            let media = media.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            workers.spawn("bridge:runtime", move || {
                runtime_worker(directory, binary, rx, commands, media, tx, shared)
            });
        }
        {
            let directory = directory.clone();
            let binary = binary.clone();
            let runtime = commands.clone();
            let tx = emit.clone();
            let shared = shared.clone();
            workers.spawn("bridge:collab", move || {
                collab::run(directory, binary, collab_rx, runtime, tx, shared)
            });
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            workers.spawn("bridge:media", move || media_worker(media_rx, tx, shared));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            workers.spawn("bridge:renders", move || queue::worker(shared, tx));
        }
        {
            let tx = emit.clone();
            let shared = shared.clone();
            let commands = commands.clone();
            workers.spawn("bridge:agent", move || {
                agent_worker(directory, binary, prompt_rx, commands, tx, shared)
            });
        }
        {
            let tx = emit.clone();
            workers.spawn("bridge:cloud", move || cloud_status(&tx, None));
        }
        {
            // Their events wait for room in the event queue here, never on
            // the caller's thread, which is the one that empties it.
            let tx = emit.clone();
            let store = conversations::Store::of(&project_folder);
            workers.spawn("bridge:chats", move || {
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
                match self.shared.viewer.seek.lock() {
                    Ok(mut slot) => {
                        *slot = Some(PendingSeek {
                            position,
                            exact: !scrub,
                        });
                        true
                    }
                    _ => false,
                }
            }
            Command::Play | Command::Pause | Command::Volume { .. } | Command::Solo { .. } => {
                self.media.try_send(command).is_ok()
            }
            Command::Abort => self.shared.agent.try_stop(),
            Command::EffectPreviews(request) => {
                let generation = self.shared.effect_previews.fetch_add(1, Ordering::SeqCst) + 1;
                let shared = self.shared.clone();
                let tx = self.events_tx.clone();
                self.shared.workers.spawn("bridge:effects", move || {
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
                self.shared.workers.spawn("bridge:unmake", move || {
                    media::cancel_make(&shared, &media, &tx)
                });
                true
            }
            Command::Models => {
                let tx = self.events_tx.clone();
                self.shared
                    .workers
                    .spawn("bridge:models", move || emit(&tx, models_event()));
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
                self.shared
                    .workers
                    .spawn("bridge:cloud", move || cloud_status(&tx, Some(command)));
                true
            }
            // A preview is counted as it is sent, and a stop or an exit acts
            // here rather than behind the runtime worker's queue: it stops the
            // previews sent before it, wherever they have got to.
            Command::Preview { .. } => {
                let commands = &self.commands;
                self.shared
                    .preview
                    .turns
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
                self.shared.workers.spawn("bridge:presets", move || {
                    emit(&tx, presets::library(&directory))
                });
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
                if text.trim().is_empty() || !self.shared.agent.begin() {
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
                    self.shared.agent.end();
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
                if text.trim().is_empty() || count == 0 || !self.shared.agent.begin() {
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
                    self.shared.agent.end();
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
            .viewer
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

    /// The threads the bridge started itself, which go on being counted once it is let go:
    /// for finding out that they are done.
    pub fn workers(&self) -> Workers {
        self.shared.workers.clone()
    }
}

impl Drop for Bridge {
    /// Close the bridge and signal the workers to shut down, without blocking the editor.
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        // Nothing that draws through them begins a frame from here on; [`drawing_settled`]
        // waits for those that had, and for the renders, the check and the makings
        // cancelled below to stop, the bridge's own and its tools'.
        self.shared.drawers.close();
        // Nobody would see a preview still rendering, or the one playing. The
        // process may exit before the media worker stops, so the files go now;
        // one a playback worker still holds open is left for the media worker.
        cancel_preview(&self.shared);
        discard_previews(&self.shared);
        qc::cancel(&self.shared);
        // A making nobody would see the end of: its program is asked to end.
        media::cancel_makings(&self.shared);
        self.shared.renders.close(&self.shared.directory);
        tools::stop_renders(&self.shared);
        self.shared.analysis.close();
        self.shared.agent.stop();
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

#[cfg(test)]
mod tests {
    use super::Command;

    use serde_json::json;

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
}
