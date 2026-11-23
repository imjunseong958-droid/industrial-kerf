//! The runtime worker: kerfd reached and held, the editor's commands carried out on it in
//! the order they come, and the state each leaves published as a `state` event, the
//! viewer's project with it.

use crate::event::{ConnectionStatus, Snapshot};
use crate::viewer::preview::{self, install_project, preview_unstarted};
use crate::wake::Outbox;
use crate::{
    analysis, emit, error, key, looks, media, paperwork, presets, queue, recipes, takes, tracks,
    workflows, Command, Event, MediaProject, Shared,
};
use kerf_mcp::KerfMcpRouter;
use kerf_runtime::{RuntimeClient, StateSnapshot};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

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
        // The document may use looks another process added to the project. One that does not
        // read leaves the catalog as it was, and kerfd reports it.
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
    for entry in state.proposals.iter().filter(|entry| entry.is_pending()) {
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

/// Reach kerfd for the project in `directory` (starting `binary` when none runs for it yet),
/// then carry out the editor's commands on it in the order they come, publishing the state
/// each leaves; with none for a while, publish the state kerfd has now.
pub(crate) fn runtime_worker(
    directory: PathBuf,
    binary: PathBuf,
    rx: Receiver<Command>,
    commands: SyncSender<Command>,
    media: SyncSender<Command>,
    tx: Outbox,
    shared: Arc<Shared>,
) {
    let mut previous = None;
    let client = connect_runtime(&directory, &binary, &tx, &shared, &mut previous);
    let mut worker = Worker {
        directory,
        binary,
        commands,
        media,
        tx,
        shared,
        previous,
        client,
    };
    // Hold kerfd for as long as the editor is open; it exits on its own
    // once the editor and every other client are gone.
    let _lease = RuntimeClient::new(&worker.directory)
        .ok()
        .map(|lease| lease.hold());
    while !worker.shared.closed.load(Ordering::Relaxed) {
        // What `kerf make` or an agent's own kerf-mcp is making of this project shows as the
        // editor's own makings do.
        media::follow_others_makings(&worker.shared, std::process::id(), &worker.tx);
        let command = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                worker.poll();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if worker.shared.closed.load(Ordering::Relaxed) {
            break;
        }
        worker.command(command);
    }
}

/// The runtime worker's own: what it was started with, the snapshot it published last, and
/// kerfd while it is reached.
///
/// The fields drop in the order they are declared, which is the order the worker's own
/// locals and arguments dropped in before they were gathered here: kerfd's client first.
struct Worker {
    client: Option<RuntimeClient>,
    previous: Option<Value>,
    shared: Arc<Shared>,
    tx: Outbox,
    /// The media worker's queue: playback pauses when kerfd goes.
    media: SyncSender<Command>,
    /// The runtime worker's own queue, for the commands a copy into `media/` or a making
    /// sends once it is done.
    commands: SyncSender<Command>,
    binary: PathBuf,
    directory: PathBuf,
}

/// What a command carried out on kerfd comes to.
struct Answer {
    /// The state it leaves, or why it left none.
    result: Result<StateSnapshot, kerf_runtime::RuntimeError>,
    /// What follows that state.
    then: Option<Event>,
    /// The command a refusal of it names; with none, a refusal is a plain error.
    refusal: Option<&'static str>,
}

impl Worker {
    /// With no command for a while: the state kerfd has now, published when it changed, or
    /// kerfd said to be gone.
    fn poll(&mut self) {
        if let Some(active_client) = self.client.as_ref() {
            match active_client.get_state() {
                Ok(state) => publish(state, &self.shared, &self.tx, &mut self.previous),
                Err(error) if error.is_transport() => self.disconnect(&error.message),
                Err(_) => {}
            }
        }
    }

    /// kerfd is gone (`message` says how): playback pauses, the editor is told, and commands
    /// wait for a reconnect.
    fn disconnect(&mut self, message: &str) {
        // Never waits on the media worker: it is gone only as the bridge closes, and a pause
        // lost to its full queue leaves playback going until the person stops it.
        let _ = self.media.try_send(Command::Pause);
        connection_event(&self.tx, ConnectionStatus::Disconnected, Some(message));
        self.client = None;
    }

    /// `reconnect`: reach kerfd again, pausing playback first while it was reached.
    fn reconnect(&mut self) {
        if self.client.is_some() {
            // As in `disconnect`: never waits on the media worker.
            let _ = self.media.try_send(Command::Pause);
        }
        self.client = connect_runtime(
            &self.directory,
            &self.binary,
            &self.tx,
            &self.shared,
            &mut self.previous,
        );
    }

    /// Carry out one command from the editor, and publish the state it leaves.
    fn command(&mut self, command: Command) {
        // A preview's place among those sent, which a stop goes by.
        let preview_turn =
            matches!(command, Command::Preview { .. }).then(|| self.shared.preview_turns.take());
        if matches!(command, Command::Reconnect) {
            self.reconnect();
            return;
        }
        let Some(active_client) = self.client.clone() else {
            error(&self.tx, "Runtime is disconnected; reconnect to continue");
            if let (Some(turn), Command::Preview { proposal_id, .. }) = (preview_turn, command) {
                preview_unstarted(&self.shared, &self.tx, turn, proposal_id);
            }
            return;
        };
        let operation = key();
        let force_refresh = matches!(&command, Command::Refresh);
        // Render queue, paperwork files and saved looks answer with their
        // own events, never a state.
        let Some(command) = self.side(command, &active_client) else {
            return;
        };
        let Some(answer) = self.carry_out(command, &active_client, &operation, preview_turn) else {
            return;
        };
        self.answered(answer, force_refresh, &active_client);
    }

    /// Carry out a command for the render queue, a paperwork file or a saved look, which
    /// answers with its own events, never a state. Any other command is given back.
    fn side(&mut self, command: Command, active_client: &RuntimeClient) -> Option<Command> {
        let (name, result) = match command {
            Command::Export(request) => {
                let renders: Arc<dyn queue::Renders> = Arc::new(active_client.clone());
                (
                    "export",
                    queue::submit(&self.shared, renders, &request, &self.tx),
                )
            }
            Command::ExportCancel { job_id } => (
                "export_cancel",
                queue::cancel(&self.shared, active_client, &job_id, &self.tx),
            ),
            Command::RenderQueue => (
                "render_queue",
                queue::publish(&self.shared, active_client, &self.tx),
            ),
            Command::ExportCaptions(request) => (
                "export_captions",
                paperwork::export_captions(active_client, &self.directory, &request, &self.tx),
            ),
            Command::LookSave {
                clip,
                name,
                replace,
            } => (
                "look_save",
                looks::save_from(
                    active_client,
                    &self.directory,
                    &clip,
                    &name,
                    replace,
                    &self.tx,
                ),
            ),
            command => return Some(command),
        };
        match result {
            Ok(()) => {}
            Err(error) if error.is_transport() => self.disconnect(&error.message),
            Err(error) => {
                queue::refused(&self.tx, name, &error);
            }
        }
        None
    }

    /// Carry out a command that leaves a state on kerfd: what it comes to, or `None` when it
    /// said what it had to with events of its own.
    fn carry_out(
        &mut self,
        command: Command,
        active_client: &RuntimeClient,
        operation: &str,
        preview_turn: Option<u64>,
    ) -> Option<Answer> {
        // The command a refusal names.
        let mut refusal: Option<&'static str> = None;
        let result = match command {
            Command::Refresh => active_client.get_state(),
            Command::Reload => {
                self.previous = None;
                active_client.get_state()
            }
            // A shared project copies the file into media/ on another
            // thread, then imports the copy.
            Command::Import { path, media_only }
                if media::needs_copy(&self.directory, Path::new(&path)) =>
            {
                media::copy_then(
                    &self.shared,
                    &path,
                    &self.commands,
                    &self.tx,
                    media::probe,
                    move |path| Command::Import { path, media_only },
                );
                return None;
            }
            Command::Import { path, media_only } => active_client.get_state().and_then(|state| {
                if media_only {
                    active_client.add_media_request("ui", operation, &path, state.revision)
                } else {
                    active_client.import_media_request("ui", operation, &path, state.revision)
                }
                .map(|(state, _, _)| state)
            }),
            Command::MediaReplace { media, path }
                if media::needs_copy(&self.directory, Path::new(&path)) =>
            {
                media::copy_then(
                    &self.shared,
                    &path,
                    &self.commands,
                    &self.tx,
                    media::probe,
                    move |path| Command::MediaReplace { media, path },
                );
                return None;
            }
            Command::SignalFromSound { clip, of } => {
                self.signal_from_sound(active_client, operation, clip, of)?
            }
            Command::MediaReplace { media, path } => active_client.get_state().and_then(|state| {
                active_client.replace_media_request("ui", operation, &media, &path, state.revision)
            }),
            Command::ProjectionImport => active_client.get_state().and_then(|state| {
                active_client.import_projection_request("ui", operation, state.revision)
            }),
            Command::ProjectionRepair => active_client.repair_projection(),
            Command::LookApply { clip, path } => {
                return self.look_apply(active_client, operation, clip, path);
            }
            Command::Edit { ops, revision } => {
                active_client.apply_ops_request("ui", operation, ops, revision)
            }
            Command::Undo { revision } => active_client.undo_request("ui", operation, revision),
            Command::Redo { revision } => active_client.redo_request("ui", operation, revision),
            Command::Accept {
                proposal_id,
                revision,
            } => active_client.accept_proposal(&proposal_id, "ui", operation, revision),
            Command::Reject {
                proposal_id,
                revision,
            } => active_client.reject_proposal_request(&proposal_id, "ui", operation, revision),
            Command::Forward {
                proposal_id,
                revision,
            } => {
                // Its refusal is said as one, with the clip that stopped it.
                refusal = Some("forward");
                active_client.forward_proposal(operation, revision, &proposal_id)
            }
            Command::PresetApply {
                path,
                bindings,
                texts,
                revision,
            } => self.preset_apply(active_client, path, bindings, texts, revision),
            Command::Preview {
                proposal_id,
                start,
                duration,
            } => {
                let turn = preview_turn.expect(
                    "`command` takes a turn for every Command::Preview, and `side` hands the \
                     command on unchanged",
                );
                preview::start(
                    turn,
                    proposal_id,
                    start,
                    duration,
                    active_client,
                    &self.tx,
                    &self.shared,
                );
                return None;
            }
            command => {
                self.on_its_own(command, active_client);
                return None;
            }
        };
        Some(Answer {
            result,
            then: None,
            refusal,
        })
    }

    /// `look_apply`: the look at `path` put on `clip` as one edit, with the event that says
    /// so to follow its state; `None` once a look that does not apply is refused.
    fn look_apply(
        &self,
        active_client: &RuntimeClient,
        operation: &str,
        clip: String,
        path: String,
    ) -> Option<Answer> {
        // What follows the state, and the command a refusal names.
        let mut then: Option<Event> = None;
        let mut refusal: Option<&'static str> = None;
        let opened = active_client
            .get_document(false)
            .and_then(|(revision, text)| {
                kerf_runtime::parse_document(&text).map(|project| (revision, project))
            });
        let result = match opened {
            Ok((revision, project)) => {
                match looks::apply(&self.directory, &project, &clip, &path) {
                    Ok((ops, event)) => {
                        then = Some(event);
                        refusal = Some("look_apply");
                        active_client.apply_ops_request("ui", operation, ops, revision)
                    }
                    Err(message) => {
                        looks::refused(&self.tx, "look_apply", &message);
                        return None;
                    }
                }
            }
            Err(error) => Err(error),
        };
        Some(Answer {
            result,
            then,
            refusal,
        })
    }

    /// Carry out a command that answers with events of its own, never a state: makings, the
    /// cast, recipes, workflows, shots and takes, and presets saved, exported or read. Any
    /// other command is let go.
    fn on_its_own(&self, command: Command, active_client: &RuntimeClient) {
        match command {
            Command::MediaMake {
                media,
                confirmed,
                rolls,
            } => {
                let rolls = rolls.unwrap_or(1).clamp(1, 16);
                media::make(
                    &self.shared,
                    active_client,
                    &media,
                    confirmed,
                    rolls,
                    &self.commands,
                    &self.tx,
                );
            }
            Command::CastGet => {
                media::cast(&self.shared, active_client, None, &self.tx);
            }
            Command::CastSet { entries } => {
                media::cast(&self.shared, active_client, Some(entries), &self.tx);
            }
            Command::MediaCarry { media, confirmed } => {
                recipes::carry(&self.shared, active_client, &media, confirmed, &self.tx);
            }
            Command::RecipeGet { media } => {
                recipes::get(&self.shared, active_client, &media, &self.tx);
            }
            Command::RecipeEdit(edit) => {
                recipes::edit(&self.shared, active_client, &edit, &self.tx);
            }
            Command::WorkflowGet { media, line } => {
                workflows::get(&self.shared, active_client, &media, line, &self.tx);
            }
            Command::ShotPicture { clip, of } => {
                tracks::picture(&self.shared, active_client, clip, of, &self.tx);
            }
            Command::ShotFollow(asked) => {
                tracks::follow(&self.shared, active_client, asked, &self.tx);
            }
            Command::TakesRead { media } => {
                takes::read(&self.shared, active_client, media, &self.tx);
            }
            Command::WorkflowEdit(edit) => {
                workflows::edit(&self.shared, active_client, &edit, &self.tx);
            }
            Command::PresetSave { name, replace } => {
                self.preset_save(active_client, name, replace);
            }
            Command::PresetExport { path } => {
                self.preset_export(active_client, path);
            }
            Command::PresetInspect { path } => {
                self.preset_inspect(path);
            }
            _ => {}
        }
    }

    /// `signal_from_sound`: the keys of the signal clip `clip` set from the onsets of the
    /// sound `of` plays, as one edit; `None` once keys that cannot be read (the clips are not
    /// what it takes, or the sound has no onsets) are said here, with no edit asked of kerfd.
    fn signal_from_sound(
        &mut self,
        active_client: &RuntimeClient,
        operation: &str,
        clip: String,
        of: String,
    ) -> Option<Result<StateSnapshot, kerf_runtime::RuntimeError>> {
        let opened = active_client
            .get_document(false)
            .and_then(|(revision, text)| {
                kerf_runtime::parse_document(&text).map(|project| (revision, project))
            });
        let (revision, project) = match opened {
            Ok(opened) => opened,
            Err(error) => return Some(Err(error)),
        };
        match kerf_shots::signals::from_sound(&project, &self.shared.directory, &of, &clip, None) {
            Ok(read) => Some(active_client.apply_script_request(
                "ui",
                operation,
                &read.script(&clip),
                revision,
            )),
            Err(why) => {
                error(&self.tx, why);
                self.publish_as_it_is(active_client);
                None
            }
        }
    }

    /// `preset_save`: the program saved as the project's preset `name`.
    fn preset_save(&self, active_client: &RuntimeClient, name: String, replace: bool) {
        let saved = active_client
            .get_document(false)
            .and_then(|(_, text)| kerf_runtime::parse_document(&text))
            .map_err(|error| error.message)
            .and_then(|project| presets::export_program(&project, Some(&name)))
            .and_then(|document| presets::save(&self.directory, &name, &document, replace));
        match saved {
            Ok(path) => emit(&self.tx, Event::PresetSaved { path }),
            Err(message) => error(&self.tx, message),
        }
    }

    /// `preset_export`: the program as a preset document, for the editor to write to `path`.
    fn preset_export(&self, active_client: &RuntimeClient, path: String) {
        match active_client
            .get_document(false)
            .and_then(|(_, text)| kerf_runtime::parse_document(&text))
        {
            Ok(project) => match presets::export_program(&project, None) {
                Ok(document) => emit(&self.tx, Event::PresetExport { path, document }),
                Err(message) => error(&self.tx, message),
            },
            Err(err) => error(&self.tx, err.message),
        }
    }

    /// `preset_inspect`: what the preset at `path` holds, as kerf-mcp's `look` reads it.
    fn preset_inspect(&self, path: String) {
        match KerfMcpRouter::new(&self.directory, Some(&self.binary)) {
            Ok(router) => {
                crate::tools::begin_on_drawers(&router, &self.shared);
                let response = router.call("look", json!({"preset":path}));
                if response["ok"] == true {
                    emit(
                        &self.tx,
                        Event::PresetInspect {
                            path,
                            preset: response["result"].clone(),
                        },
                    );
                } else {
                    error(
                        &self.tx,
                        response["error"]["message"]
                            .as_str()
                            .unwrap_or("cannot read preset"),
                    );
                }
            }
            Err(message) => error(&self.tx, message),
        }
    }

    /// `preset_apply`: the preset at `path` staged as a proposal with kerf-mcp's `edit`,
    /// while the project is still at `revision`. The state to publish either way.
    fn preset_apply(
        &self,
        active_client: &RuntimeClient,
        path: String,
        bindings: Value,
        texts: Value,
        revision: u64,
    ) -> Result<StateSnapshot, kerf_runtime::RuntimeError> {
        match active_client.get_state() {
            Ok(state) if state.revision == revision => {
                match KerfMcpRouter::new(&self.directory, Some(&self.binary)) {
                    Ok(router) => {
                        crate::tools::begin_on_drawers(&router, &self.shared);
                        let response = router.call(
                            "edit",
                            json!({"preset":path,"bindings":bindings,"texts":texts}),
                        );
                        if response["ok"] == true {
                            emit(
                                &self.tx,
                                Event::PresetApplied {
                                    path,
                                    proposal: response["result"]["proposal"].clone(),
                                },
                            );
                        } else {
                            error(
                                &self.tx,
                                response["error"]["message"]
                                    .as_str()
                                    .unwrap_or("cannot apply preset"),
                            );
                        }
                    }
                    Err(message) => error(&self.tx, message),
                }
            }
            Ok(_) => error(
                &self.tx,
                "The project changed while choosing preset media. Apply the preset again.",
            ),
            Err(err) => error(&self.tx, err.message),
        }
        // Publish the same proposal state used by the native review UI.
        active_client.get_state()
    }

    /// Publish the state a command left, then what follows it; or say why it left none, and
    /// publish the state as it is.
    fn answered(&mut self, answer: Answer, force_refresh: bool, active_client: &RuntimeClient) {
        let Answer {
            result,
            mut then,
            refusal,
        } = answer;
        match result {
            Ok(state) => {
                if force_refresh {
                    publish_force(state, &self.shared, &self.tx, &mut self.previous);
                } else {
                    publish(state, &self.shared, &self.tx, &mut self.previous);
                }
                if let Some(event) = then.take() {
                    emit(&self.tx, event);
                }
            }
            Err(error) if error.is_transport() => self.disconnect(&error.message),
            Err(runtime_error) => {
                match refusal {
                    Some(name) => {
                        queue::refused(&self.tx, name, &runtime_error);
                    }
                    None => error(&self.tx, runtime_error.message),
                }
                self.publish_as_it_is(active_client);
            }
        }
    }

    /// Publish the state as it is, after a command that left none; or say kerfd is gone.
    fn publish_as_it_is(&mut self, active_client: &RuntimeClient) {
        if let Ok(state) = active_client.get_state() {
            publish_force(state, &self.shared, &self.tx, &mut self.previous);
        } else {
            self.disconnect("runtime became unavailable");
        }
    }
}
