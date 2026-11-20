//! The project the window has open: the bridge to kerfd, the state its
//! events leave, and the picture playback last drew. One entity, observed
//! by every region of the window, so what kerfd says shows everywhere at
//! once. Without a project (the launcher) there is no bridge.

use crate::model::Timeline;
use crate::prefs::{Prefs, Provider};
use crate::store::{Change, EditorState};
use crate::theme::motion;
use gpui::{App, AppContext, Context, Entity, EventEmitter, RenderImage, Task, Window};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Where Finish's deliveries go, inside the project.
pub const DELIVERIES: &str = "deliveries/";

/// What the system's keychain answered for each key, for every window of
/// this editor: asked once, written when a key is saved in any window, and
/// asked again when Settings opens (a key changed outside Kerf).
static KEYCHAIN: Mutex<BTreeMap<&'static str, Option<String>>> = Mutex::new(BTreeMap::new());

fn keychain() -> MutexGuard<'static, BTreeMap<&'static str, Option<String>>> {
    KEYCHAIN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a key was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKept {
    Keychain,
    /// The keychain could not be reached: in the editor's settings file.
    SettingsFile,
}

/// The newest picture playback drew, ready to draw.
pub struct Picture {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
    pub time: f64,
}

pub struct Session {
    pub project: Option<PathBuf>,
    bridge: Option<kerf_bridge::Bridge>,
    pub state: EditorState,
    pub prefs: Prefs,
    pub picture: Option<Picture>,
    /// The conversation the composer speaks in; None starts one.
    pub conversation: Option<String>,
    /// Clips chosen on the timeline.
    pub selection: Vec<String>,
    /// The media chosen in the band.
    pub chosen_media: Option<String>,
    _on_quit: gpui::Subscription,
    /// A kerf:// link followed to this project, carried out once it is connected.
    link: Option<crate::links::Link>,
    mark_in: Option<f64>,
    mark_out: Option<f64>,
    looping: bool,
    _poll: Option<Task<()>>,
}

impl EventEmitter<Change> for Session {}

/// Every event the bridge sends, as it sent it, for a region that keeps
/// state of its own (a check's findings, a making's plan) rather than in
/// [`EditorState`].
#[derive(Clone, Debug)]
pub struct BridgeEvent(pub Value);

impl EventEmitter<BridgeEvent> for Session {}

impl Session {
    /// A session on `project`, its bridge started without waiting on it.
    pub fn open(
        project: Option<PathBuf>,
        kerfd: Option<PathBuf>,
        prefs: Prefs,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let bridge = project.as_deref().map(|project| {
                let kerfd = kerfd.clone().unwrap_or_else(discover_kerfd);
                kerf_bridge::Bridge::open(project, &kerfd)
            });
            let poll = bridge.is_some().then(|| Self::poll_forever(window, cx));
            let mut session = Self {
                project,
                bridge,
                state: EditorState::default(),
                prefs,
                picture: None,
                conversation: None,
                selection: Vec::new(),
                chosen_media: None,
                // Quitting closes the bridge as closing the window does: the
                // render running stops, the queued ones are cancelled.
                _on_quit: cx.on_app_quit(|session: &mut Session, _| {
                    session.bridge.take();
                    async {}
                }),
                link: None,
                mark_in: None,
                mark_out: None,
                looping: false,
                _poll: poll,
            };
            session.send(json!({"type": "conversations", "action": "list"}));
            session.send(json!({"type": "render_queue"}));
            session
        })
    }

    fn poll_forever(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(motion::POLL).await;
            let alive = this.update_in(cx, |session, window, cx| session.drain(window, cx));
            if alive.is_err() {
                break;
            }
        })
    }

    /// Takes every event the bridge has, and its newest picture.
    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bridge) = self.bridge.as_mut() else {
            return;
        };
        let mut events = Vec::new();
        // Never more than a frame's worth at once, so the window keeps drawing.
        while events.len() < 256 {
            match bridge.poll() {
                Some(event) => events.push(event),
                None => break,
            }
        }
        let frame = bridge.frame();
        let heard = !events.is_empty();
        for event in events {
            if event["type"] != "clock" && event["type"] != "meter" {
                log::debug!("bridge event: {}", truncate(&event.to_string(), 400));
            }
            let change = self.state.apply(&event);
            cx.emit(BridgeEvent(event.clone()));
            match &change {
                Change::Snapshot => self.follow_link(),
                Change::Connection
                    if self.state.connection == crate::store::Connection::Connected =>
                {
                    // Live Sync as it was left for this project.
                    if self
                        .project
                        .as_ref()
                        .is_some_and(|project| self.prefs.live_sync.contains(project))
                    {
                        self.send(json!({"type": "collab_live", "enabled": true}));
                    }
                }
                Change::ViewerReset => self.drop_picture(window, cx),
                Change::Work if event["type"] == "agent_focus" => self.follow_kerf(),
                Change::Clock => self.keep_to_marks(),
                Change::TurnEnded => {
                    // The conversation as it is kept now, with Kerf's answer.
                    if let Some(id) = self.conversation.clone() {
                        self.send(json!({"type": "conversations", "action": "open", "id": id}));
                    }
                }
                _ => {}
            }
            // The conversation shown is the state's choice (a request sent
            // with none chosen starts one; a deleted one leaves none).
            self.conversation = self.state.chosen.clone();
            cx.emit(change);
        }
        let mut changed = heard;
        if let Some(frame) = frame {
            self.show_frame(frame, window, cx);
            cx.emit(Change::Clock);
            changed = true;
        }
        if self.state.expire_notices(6.0) {
            cx.emit(Change::Notice);
            changed = true;
        }
        // Nothing heard, nothing drawn: the window holds still between events.
        if changed {
            cx.notify();
        }
    }

    fn show_frame(
        &mut self,
        frame: kerf_bridge::Frame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(image) = render_image(&frame.pixels, frame.width, frame.height) else {
            return;
        };
        self.drop_picture(window, cx);
        self.picture = Some(Picture {
            image,
            width: frame.width,
            height: frame.height,
            time: frame.time,
        });
    }

    fn drop_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.picture.take() {
            cx.drop_image(old.image, Some(window));
        }
    }

    /// Follows `link` once the project is connected: seeks to its moment,
    /// and says so when it names another revision.
    pub fn follow(&mut self, link: crate::links::Link) {
        self.link = Some(link);
        if self.state.has_snapshot {
            self.follow_link();
        }
    }

    fn follow_link(&mut self) {
        let Some(link) = self.link.take() else {
            return;
        };
        let fps = self.timeline().rate();
        if let Some(seconds) = crate::links::seconds(&link, fps) {
            self.seek(seconds, false);
        }
        if let Some(revision) = link
            .revision
            .filter(|revision| *revision != self.revision())
        {
            let now = self.revision();
            self.state.notify(
                &format!("The link was to revision {revision}; the project is at revision {now}."),
                false,
            );
        }
    }

    /// Sends one command; false when the bridge refused it (its queue is
    /// full), which the window says rather than dropping an edit unseen.
    pub fn send(&mut self, command: Value) -> bool {
        let Some(bridge) = self.bridge.as_mut() else {
            return false;
        };
        let sent = bridge.send(&command);
        if !sent {
            self.state
                .notify("Kerf is busy; that was not sent. Try again.", true);
        }
        sent
    }

    pub fn has_project(&self) -> bool {
        self.bridge.is_some()
    }

    pub fn timeline(&self) -> &Timeline {
        &self.state.snapshot.timeline
    }

    pub fn revision(&self) -> u64 {
        self.state.snapshot.revision
    }

    /// The project's name: its document's, else its folder's.
    pub fn project_name(&self) -> String {
        let name = &self.state.snapshot.timeline.name;
        if !name.trim().is_empty() {
            return name.clone();
        }
        self.project
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Industrial Kerf".into())
    }

    /// Typed edits, in frames of the sequence.
    pub fn edit(&mut self, ops: Vec<Value>) -> bool {
        let revision = self.revision();
        self.send(json!({"type": "edit", "ops": ops, "revision": revision}))
    }

    pub fn undo(&mut self) {
        let revision = self.revision();
        self.send(json!({"type": "undo", "revision": revision}));
    }

    pub fn redo(&mut self) {
        let revision = self.revision();
        self.send(json!({"type": "redo", "revision": revision}));
    }

    pub fn seek(&mut self, seconds: f64, scrub: bool) {
        let seconds = seconds.clamp(0.0, self.state.program_length().max(0.0));
        self.state.position = seconds;
        self.send(json!({"type": "seek", "position": seconds, "scrub": scrub}));
    }

    pub fn toggle_playback(&mut self) {
        let command = if self.state.playing { "pause" } else { "play" };
        self.send(json!({"type": command}));
    }

    pub fn accept(&mut self, proposal: &str) {
        let revision = self.revision();
        self.send(json!({"type": "accept", "proposalId": proposal, "revision": revision}));
    }

    pub fn reject(&mut self, proposal: &str) {
        let revision = self.revision();
        self.send(json!({"type": "reject", "proposalId": proposal, "revision": revision}));
    }

    pub fn forward(&mut self, proposal: &str) {
        let revision = self.revision();
        self.send(json!({"type": "forward", "proposalId": proposal, "revision": revision}));
    }

    pub fn preview(&mut self, proposal: Option<&str>) {
        let mut command = json!({"type": "preview"});
        if let Some(proposal) = proposal {
            command["proposalId"] = json!(proposal);
        }
        self.send(command);
    }

    pub fn import(&mut self, path: &Path, media_only: bool) {
        self.send(
            json!({"type": "import", "path": path.to_string_lossy(), "mediaOnly": media_only}),
        );
    }

    pub fn export(&mut self, quality: &str) {
        self.send(json!({"type": "export", "quality": quality}));
    }

    /// Renders the master (`master`) or a review copy (`review`) into the
    /// project's deliveries/, named after its folder: `<folder>_master`,
    /// `<folder>_draft`. Without a folder name, as kerfd names it.
    pub fn deliver(&mut self, quality: &str) {
        let folder = self
            .project
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty());
        let Some(folder) = folder else {
            self.export(quality);
            return;
        };
        let command = if quality == "master" {
            json!({"type": "export", "quality": "master", "output": DELIVERIES, "name": format!("{folder}_master")})
        } else {
            json!({"type": "export", "quality": "review", "codec": "h264", "output": DELIVERIES,
                "name": format!("{folder}_draft")})
        };
        self.send(command);
    }

    /// Turns Live Sync on or off for this project, and remembers it for the
    /// next time the project opens.
    pub fn set_live_sync(&mut self, on: bool) {
        self.send(json!({"type": "collab_live", "enabled": on}));
        if let Some(project) = self.project.clone() {
            self.prefs.live_sync.retain(|known| known != &project);
            if on {
                self.prefs.live_sync.push(project);
            }
            self.prefs.save();
        }
    }

    pub fn cancel_export(&mut self, job: &str) {
        self.send(json!({"type": "export_cancel", "jobId": job}));
    }

    /// Opens a conversation kept beside the project; None starts a new one.
    pub fn open_conversation(&mut self, id: Option<String>) {
        self.state.choose(id.clone());
        self.conversation = id.clone();
        if let Some(id) = id {
            self.send(json!({"type": "conversations", "action": "open", "id": id}));
        }
    }

    /// Asks the bridge to reach kerfd again after it went away.
    pub fn reconnect(&mut self) {
        self.state.connection = crate::store::Connection::Connecting;
        self.send(json!({"type": "reconnect"}));
    }

    /// While Kerf works, the playhead goes where it is looking (Follow
    /// Kerf, View ▸ Follow Kerf), unless the program is playing.
    fn follow_kerf(&mut self) {
        if !self.prefs.follow_kerf || self.state.playing {
            return;
        }
        if let Some(at) = self.state.agent_focus.first().copied() {
            self.seek(at, false);
        }
    }

    /// The program's in and out marks, and whether playback loops between
    /// them (the whole program without marks).
    pub fn marks(&self) -> (Option<f64>, Option<f64>, bool) {
        (self.mark_in, self.mark_out, self.looping)
    }

    pub fn set_mark_in(&mut self, at: Option<f64>) {
        self.mark_in = at;
        if let (Some(start), Some(end)) = (self.mark_in, self.mark_out) {
            if end <= start {
                self.mark_out = None;
            }
        }
    }

    pub fn set_mark_out(&mut self, at: Option<f64>) {
        self.mark_out = at;
        if let (Some(start), Some(end)) = (self.mark_in, self.mark_out) {
            if end <= start {
                self.mark_in = None;
            }
        }
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
    }

    /// Looping, playback that passes the out mark (or the end) goes back
    /// to the in mark (or the start).
    fn keep_to_marks(&mut self) {
        if !self.looping || !self.state.playing {
            return;
        }
        let end = self.mark_out.unwrap_or_else(|| self.state.program_length());
        let start = self.mark_in.unwrap_or(0.0);
        let frame = 1.0 / self.timeline().rate();
        if end > start && self.state.position >= end - frame {
            self.seek(start, false);
        }
    }

    pub fn archive_conversation(&mut self, id: &str, archived: bool) {
        self.send(
            json!({"type": "conversations", "action": "archive", "id": id, "archived": archived}),
        );
        if archived && self.conversation.as_deref() == Some(id) {
            self.open_conversation(None);
        }
    }

    pub fn delete_conversation(&mut self, id: &str) {
        self.send(json!({"type": "conversations", "action": "delete", "id": id}));
        if self.conversation.as_deref() == Some(id) {
            self.open_conversation(None);
        }
    }

    pub fn rename_conversation(&mut self, id: &str, title: &str) {
        self.send(json!({"type": "conversations", "action": "rename", "id": id, "title": title}));
    }

    /// Asks Kerf, in the conversation open (or a new one), about what is
    /// chosen on the timeline if anything is.
    pub fn ask(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.state.work.busy() {
            return false;
        }
        // What another window chose since, as the person last left it.
        self.prefs.refresh();
        let mut command = json!({
            "type": "agent",
            "text": text,
            "permissions": self.prefs.permissions.wire(),
        });
        let settings = self.agent_settings();
        if let (Some(command), Value::Object(settings)) = (command.as_object_mut(), settings) {
            command.extend(settings);
        }
        if let Some(conversation) = &self.conversation {
            command["conversation"] = json!(conversation);
        }
        if self.prefs.send_playhead {
            command["at"] = json!(self.state.position);
        }
        if let Some(focus) = self.focus() {
            command["focus"] = focus;
        }
        let sent = self.send(command);
        if sent {
            self.state.work.request = Some(text.to_owned());
            self.state.work.conversation = self.conversation.clone();
            self.state
                .work
                .started
                .get_or_insert_with(std::time::Instant::now);
            self.state.work.status = "Working".into();
            self.state.work.answer.clear();
            self.state.work.steps.clear();
            self.state.next_steps.clear();
        }
        sent
    }

    pub fn stop(&mut self) {
        self.send(json!({"type": "abort"}));
    }

    /// What the command registry needs to know to say what is available
    /// and to resolve a command (kerf-commands' `Context`).
    pub fn command_context(&self) -> Value {
        let timeline = self.timeline();
        let count = |kind| {
            timeline
                .tracks
                .iter()
                .filter(|track| track.kind == kind)
                .count()
        };
        let mut context = json!({
            "connected": self.state.connection == crate::store::Connection::Connected,
            "revision": self.revision(),
            "canUndo": self.state.snapshot.can_undo,
            "canRedo": self.state.snapshot.can_redo,
            "playheadFrame": timeline.frame_at(self.state.position),
            "videoTracks": count(crate::model::TrackKind::Video),
            "audioTracks": count(crate::model::TrackKind::Audio),
            "agentBusy": self.state.work.busy(),
            "previewRequired": self.state.preview.status == "required",
            "projectionStatus": self.state.snapshot.projection_status,
            "previewRenderable": self.state.preview.status != "running",
        });
        if let Some((track, clip)) = self.selection.first().and_then(|id| timeline.clip(id)) {
            let kind = match track.kind {
                crate::model::TrackKind::Video => "video",
                crate::model::TrackKind::Audio => "audio",
                crate::model::TrackKind::Subtitle => "subtitle",
            };
            context["selection"] = json!({
                "clip": clip.id,
                "track": kind,
                "startFrame": clip.start_frame,
                "durationFrames": clip.duration_frames,
            });
        }
        if let Some(collab) = &self.state.collab {
            context["collab"] = collab.clone();
        }
        if let Some(proposal) = self.state.snapshot.pending() {
            context["proposal"] = json!({
                "id": proposal.proposal_id,
                "blocked": proposal.is_stale().then_some("The project changed since Kerf proposed this"),
            });
        }
        if let Some(source) = self
            .chosen_media
            .as_deref()
            .and_then(|id| timeline.source(id))
        {
            let frames = (source.duration * timeline.rate()).round() as i64;
            context["source"] = json!({"media": source.id, "inFrame": 0, "durationFrames": frames});
            let first = |kind| {
                timeline
                    .tracks
                    .iter()
                    .find(|track| track.kind == kind && !track.locked)
                    .map(|track| track.id.clone())
            };
            let mut targets = Vec::new();
            if source.width.is_some() {
                targets.extend(first(crate::model::TrackKind::Video));
            }
            if source.has_audio {
                targets.extend(first(crate::model::TrackKind::Audio));
            }
            context["sourceTargets"] = json!(targets);
        }
        context
    }

    /// Runs an engine command of the registry: the request it resolves to
    /// goes to the bridge; why it cannot run is said.
    pub fn run_command(&mut self, id: &str, params: Value) {
        let invocation =
            json!({"command": id, "context": self.command_context(), "params": params});
        let answer = kerf_bridge::commands_run(&invocation);
        if let Some(request) = answer.get("request").filter(|request| request.is_object()) {
            self.send(request.clone());
        } else if let Some(error) = answer["error"].as_str() {
            self.state.notify(error, true);
        }
    }

    /// What the request is about: the chosen clip, else nothing.
    fn focus(&self) -> Option<Value> {
        let clip = self.selection.first()?;
        let (_, clip) = self.timeline().clip(clip)?;
        Some(json!({
            "scope": "clip",
            "clip": clip.id,
            "fromFrame": clip.start_frame,
            "toFrame": clip.end_frame(),
        }))
    }

    /// The key for `provider`: the keychain's, else the one kept in the
    /// editor's settings where the keychain could not be reached, else its
    /// environment variable. The keychain is asked once for every window;
    /// the settings are this window's as last refreshed.
    pub fn api_key(&self, provider: Provider) -> Option<String> {
        let stored = provider.secret_name().and_then(|name| {
            let in_keychain = keychain()
                .entry(name)
                .or_insert_with(|| {
                    kerf_bridge::secret(&json!({"op": "get", "name": name}))["value"]
                        .as_str()
                        .map(str::to_owned)
                })
                .clone();
            in_keychain.or_else(|| self.prefs.kept_keys.get(name).cloned())
        });
        stored
            .filter(|key| !key.is_empty())
            .or_else(|| provider.key_env().and_then(|name| std::env::var(name).ok()))
            .filter(|key| !key.trim().is_empty())
    }

    /// Keeps `key` for `provider` (empty forgets it): in the system's
    /// keychain, or where that cannot be reached (Linux with no Secret
    /// Service, `KERF_SECRETS=off`), in the editor's settings file, as the
    /// answer says. Err only when it could be kept nowhere.
    pub fn set_api_key(&mut self, provider: Provider, key: &str) -> Result<KeyKept, String> {
        let Some(name) = provider.secret_name() else {
            return Ok(KeyKept::Keychain);
        };
        let key = key.trim();
        let request = if key.is_empty() {
            json!({"op": "delete", "name": name})
        } else {
            json!({"op": "set", "name": name, "value": key})
        };
        let answer = kerf_bridge::secret(&request);
        let kept = match answer["error"].as_str() {
            None => {
                // Kept in the keychain: no copy stays in the file.
                if self.prefs.kept_keys.remove(name).is_some() {
                    self.prefs.save();
                }
                KeyKept::Keychain
            }
            Some(error) => {
                if key.is_empty() {
                    self.prefs.kept_keys.remove(name);
                } else {
                    self.prefs.kept_keys.insert(name.to_owned(), key.to_owned());
                }
                self.prefs.save_checked().map_err(|written| {
                    format!("{error}; nor could it be written to the settings file: {written}")
                })?;
                KeyKept::SettingsFile
            }
        };
        // Every window's next request takes it.
        let in_keychain = (kept == KeyKept::Keychain && !key.is_empty()).then(|| key.to_owned());
        keychain().insert(name, in_keychain);
        Ok(kept)
    }

    pub fn save_prefs(&self) {
        self.prefs.save();
    }

    /// What a request runs on, as the agent command takes it: Kerf's
    /// hosted model (`cloud`), Claude Code and its model, or a provider
    /// with its key, model, base URL and (OpenAI) API shape. The MCP
    /// servers' switch and how many times their file was saved go with each.
    pub fn agent_settings(&self) -> Value {
        let prefs = &self.prefs;
        let provider = prefs.provider;
        let mut settings = json!({
            "mcpEnabled": prefs.mcp_enabled,
            "mcpConfigVersion": prefs.mcp_config_version,
        });
        if provider == Provider::Cloud {
            // The service picks the model.
            settings["cloud"] = json!(true);
            return settings;
        }
        settings["provider"] = json!(provider.wire());
        if !prefs.model().is_empty() {
            settings["model"] = json!(prefs.model());
        }
        if !prefs.base_url().is_empty() {
            settings["baseUrl"] = json!(prefs.base_url());
        }
        if provider == Provider::Openai {
            settings["apiMode"] = json!(prefs.openai_api_mode.wire());
        }
        if let Some(key) = self.api_key(provider) {
            settings["apiKey"] = json!(key);
        }
        settings
    }

    /// Why the connection chosen cannot take requests, as the command
    /// registry's `aiAccess` names it: `noKey` for a provider with no key
    /// kept and none in the environment, Kerf's account standing (`cloud`,
    /// the last `cloud` event's `access`, `unavailable` until one is
    /// heard). None when it can.
    pub fn ai_access(&self, cloud: Option<&Value>) -> Option<String> {
        match self.prefs.provider {
            Provider::ClaudeCode => None,
            Provider::Cloud => {
                let access = cloud
                    .and_then(|cloud| cloud["access"].as_str())
                    .unwrap_or("unavailable");
                (access != "ready").then(|| access.to_owned())
            }
            provider => self.api_key(provider).is_none().then(|| "noKey".into()),
        }
    }

    /// Each registry command's id with why it is unavailable now, or null
    /// when it is (`kerf_bridge::commands_states`). `cloud` is the last
    /// `cloud` event heard, for Ask's reason.
    pub fn command_states(&self, cloud: Option<&Value>) -> Value {
        let mut context = self.command_context();
        if let Some(access) = self.ai_access(cloud) {
            context["aiAccess"] = json!(access);
        }
        kerf_bridge::commands_states(&context)
    }

    /// Asks the bridge for what has to be known again: Kerf's account
    /// (`cloud_refresh`), the person's models (`models`). False without a
    /// project.
    pub fn ask_bridge(&mut self, kind: &str) -> bool {
        self.bridge.is_some() && self.send(json!({"type": kind}))
    }

    /// Signs in to Kerf with `token` (`cloud_sign_in`), or out with None
    /// (`cloud_sign_out`); a `cloud` event answers. False without a project.
    pub fn cloud_sign(&mut self, token: Option<&str>) -> bool {
        if self.bridge.is_none() {
            return false;
        }
        match token.map(str::trim).filter(|token| !token.is_empty()) {
            Some(token) => self.send(json!({"type": "cloud_sign_in", "token": token})),
            None => self.send(json!({"type": "cloud_sign_out"})),
        }
    }

    /// Forgets what the keychain answered, so the next request in any
    /// window asks it again (a key changed outside Kerf).
    pub fn forget_keys(&self) {
        keychain().clear();
    }
}

/// A playback picture (RGBA, `width` × `height`) as an image GPUI draws,
/// whose textures take BGRA.
pub fn render_image(pixels: &[u8], width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || pixels.len() != expected {
        return None;
    }
    let mut bgra = pixels.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// The kerfd beside the editor, else the checkout's build, else the PATH's.
pub fn discover_kerfd() -> PathBuf {
    if let Some(path) = std::env::var_os("KERFD_BIN").filter(|path| !path.is_empty()) {
        return path.into();
    }
    let name = if cfg!(windows) { "kerfd.exe" } else { "kerfd" };
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .filter(|path| path.is_file())
    {
        return beside;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for profile in ["debug", "release"] {
        let built = root.join("target").join(profile).join(name);
        if built.is_file() {
            return built;
        }
    }
    PathBuf::from(name)
}

/// At most `most` bytes of `text`, cut at a character boundary.
fn truncate(text: &str, most: usize) -> &str {
    if text.len() <= most {
        return text;
    }
    let mut end = most;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
