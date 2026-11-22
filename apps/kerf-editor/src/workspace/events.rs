//! What the window hears and does about it: its session (a snapshot, the
//! connection, Kerf's work, the bridge's errors and renders), the Git chip, its
//! own views (the sidebar, the timeline, the conversation, the title's field),
//! and the window itself (coming to the front, its size, its keys).

use super::bars::{
    bar_takes_error, conversation_error, kept_in_bar, readable_error, sync_failed, ErrorBar,
    DISCONNECTED,
};
use super::flights::placed_since;
use super::projects::{folder_media, open_conversation_in, window_title};
use super::Workspace;
use crate::chat::{ChatEvent, ChatView};
use crate::input::{InputEvent, TextInput};
use crate::session::{BridgeEvent, Session};
use crate::sidebar::{self, SidebarEvent, SidebarView};
use crate::stage::StageView;
use crate::store::Change;
use crate::theme::motion as scene;
use crate::timeline::TimelineView;
use gpui::{App, Context, Entity, FocusHandle, Subscription, Window};
use kerf_bridge::event::{AgentState, AgentUpdate, ConnectionStatus, Export, ExportStatus};
use kerf_bridge::{Command, ConversationsRequest, Event};
use std::collections::HashSet;

impl Workspace {
    /// What the window hears from its session and the Git chip: each
    /// change the session draws from, its changes, the bridge's events, a
    /// failed sync.
    pub(super) fn follow_session(
        session: &Entity<Session>,
        stage: &Entity<StageView>,
        collab: &Entity<crate::finish::Collab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            // The toolbar's tips name the keys of the keymap in use, and
            // what a proposal changed, watched, brings the picture in.
            cx.observe(session, |this, _, cx| {
                this.refresh_keys(cx);
                this.follow_reel(cx);
                cx.notify()
            }),
            // A mask tool taken out or put down (⇧R ⇧E ⇧P, the transport):
            // the menu bar ticks it as the window next draws.
            cx.observe(stage, |this, stage, cx| {
                let tool = stage.read(cx).mask_tool();
                if tool != this.mask_tool_seen {
                    this.mask_tool_seen = tool;
                    cx.notify();
                }
            }),
            cx.subscribe_in(session, window, Self::session_changed),
            cx.subscribe(session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
            // A failed sync is said in the error bar until it is dismissed
            // or a later sync starts or succeeds; the toolbar's line says
            // it meanwhile, and after a Dismiss until then, so a sync that
            // failed is not forgotten once its bar is put away.
            cx.subscribe(collab, |this, _, event: &crate::finish::CollabEvent, cx| {
                this.collab_event(event, cx)
            }),
        ]
    }

    /// What the window hears from its own views: the sidebar, the
    /// timeline, the conversation, the field the toolbar's title becomes.
    pub(super) fn follow_views(
        sidebar: &Entity<SidebarView>,
        timeline: &Entity<TimelineView>,
        chat: &Entity<ChatView>,
        title_field: &Entity<TextInput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(
                sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::Settings => this.show_settings(None, window, cx),
                    SidebarEvent::Hide => this.set_sidebar(false, cx),
                    SidebarEvent::Delete(id) => this.confirm_delete(id.clone(), window, cx),
                    SidebarEvent::Done => {
                        if this.modal.is_none() {
                            window.focus(&this.focus_handle);
                            this.settle_focus(window, cx);
                        }
                    }
                    SidebarEvent::MenuOpened => {
                        this.close_title_menu(window, cx);
                        this.close_app_menu(window, cx);
                        cx.notify();
                    }
                    SidebarEvent::AppMenu { from_keys } => {
                        this.toggle_app_menu(true, *from_keys, window, cx)
                    }
                    SidebarEvent::Chosen => {
                        if this.modal.is_none() {
                            this.chat
                                .update(cx, |chat, cx| chat.focus_composer(window, cx));
                        }
                    }
                    SidebarEvent::OpenConversationIn { folder, id } => {
                        open_conversation_in(folder.clone(), id.clone(), cx)
                    }
                },
            ),
            // What the timeline cannot reach itself: the picture follows its
            // skim, Insert and Overwrite its targets, and the project keeps
            // its tracks' heights.
            cx.subscribe(
                timeline,
                |this, _, event: &crate::timeline::TimelineEvent, cx| match event {
                    crate::timeline::TimelineEvent::Skim(time) => {
                        let time = *time;
                        this.stage.update(cx, |stage, cx| stage.skim_at(time, cx));
                    }
                    crate::timeline::TimelineEvent::Targets(targets) => {
                        let targets = targets.clone();
                        this.session
                            .update(cx, |session, _| session.source_targets = targets);
                    }
                    crate::timeline::TimelineEvent::Heights(heights) => {
                        let heights = heights.clone();
                        this.session.update(cx, |session, _| {
                            if let Some(project) = session.project.clone() {
                                session.prefs.keep_track_heights(&project, heights);
                                session.save_prefs();
                            }
                        });
                    }
                },
            ),
            // A cut's card pressed: its picture grows out of the card.
            cx.subscribe_in(
                chat,
                window,
                |this, _, event: &ChatEvent, window, cx| match event {
                    ChatEvent::GrowPicture { proposal } => {
                        this.grow_from_card(proposal, window, cx)
                    }
                },
            ),
            // The lanes draw the version of a proposal the pinned card shows.
            cx.observe(chat, |this, chat, cx| {
                let shown = chat.read(cx).pinned_proposal(cx);
                this.timeline
                    .update(cx, |timeline, cx| timeline.show_version(shown, cx));
            }),
            cx.subscribe_in(
                title_field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit | InputEvent::Cancel => {
                        this.finish_title_rename(*event == InputEvent::Submit, cx);
                        window.focus(&this.focus_handle);
                        this.settle_focus(window, cx);
                    }
                    InputEvent::Changed => {}
                },
            ),
        ]
    }

    /// What the window hears from itself: coming to the front, its size
    /// changing, the title's field left, nothing having the keys.
    pub(super) fn follow_window(
        title_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            // File ▸ Open Recent checks this window's project while it is in
            // front. The first time, the keys go where they belong; and the
            // conversation looked at again lets its dot go.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    let project = this.session.read(cx).project.clone();
                    crate::commands::refresh_menus(project.as_deref(), cx);
                    if !this.focus_settled {
                        this.focus_settled = true;
                        this.settle_focus(window, cx);
                    }
                    this.session.update(cx, |session, cx| {
                        if let Some(open) = session.conversation.clone() {
                            if session.state.settled.contains(&open) {
                                session.state.looked_at(&open);
                                cx.notify();
                            }
                        }
                    });
                }
            }),
            // In a window too narrow for it, the sidebar gives way, as the
            // window is resized.
            cx.observe_window_bounds(window, |this, window, cx| {
                this.follow_window_width(window, cx)
            }),
            // Leaving the title's field keeps what was typed.
            cx.on_blur(title_focus, window, |this, _, cx| {
                this.finish_title_rename(true, cx)
            }),
            // Nothing has the keys (Esc left the composer or a field, what
            // had them went away): they settle again, on the timeline while
            // it shows, else with the window, so the timeline's keys (Play,
            // the arrows, S, Delete) and the window's (the palette, the
            // mask tools) go on working.
            cx.on_focus_lost(window, |this, window, cx| {
                window.focus(&this.focus_handle);
                this.settle_focus(window, cx);
            }),
        ]
    }

    /// Watching changes, the picture comes in and the stage reaches over
    /// the conversation; done watching, it falls back.
    pub(super) fn follow_reel(&mut self, cx: &mut Context<Self>) {
        let watching = self.session.read(cx).reel().is_some();
        if watching != (self.reel_reach.target() > 0.5) {
            self.reel_reach
                .go(if watching { 1.0 } else { 0.0 }, scene::PANEL);
            if watching {
                self.reveal_picture(cx);
            }
        }
    }

    fn session_changed(
        &mut self,
        _: &Entity<Session>,
        change: &Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *change == Change::Snapshot {
            // Picture arrives: a project with clips on its timeline opens the
            // panel, the stage growing as the conversation gives way.
            let has_clips = self.session.read(cx).timeline().has_clips()
                || self
                    .session
                    .read(cx)
                    .state
                    .snapshot
                    .proposal_timeline
                    .is_some();
            // The window's title follows the project and its sequence.
            let title = {
                let session = self.session.read(cx);
                window_title(&session.project_name(), session.timeline())
            };
            if title != self.window_title {
                window.set_window_title(&title);
                self.window_title = title;
            }
            if has_clips && !self.panel_open && !self.panel_by_hand {
                // The picture grows out of what it came from.
                self.grow_for_cut(cx);
                window.refresh();
                // The timeline is in view: the keys go to it, unless
                // someone is typing.
                self.settle_focus(window, cx);
            }
            // What a cut being built placed flies to where it lands.
            self.follow_cut_build(cx);
            // The first snapshot puts the keys where they belong: on the
            // timeline with a cut, else in the composer to ask Kerf, unless
            // something holds them already.
            if !self.keys_found_their_place && self.modal.is_none() {
                self.keys_found_their_place = true;
                let free = window
                    .focused(cx)
                    .is_none_or(|focused| focused == self.focus_handle);
                if !has_clips && free {
                    self.chat
                        .update(cx, |chat, cx| chat.focus_composer(window, cx));
                }
            }
            // A folder that was no project yet: its media comes in now
            // that kerfd has the project.
            if let Some(folder) = self.start_media.take() {
                let files = folder_media(&folder);
                if !files.is_empty() {
                    self.session.update(cx, |session, cx| {
                        for file in &files {
                            session.import(file, true);
                        }
                        cx.notify();
                    });
                    if !self.media_open {
                        self.set_media(true, cx);
                    }
                }
            }
        }
        match change {
            // Back in touch with kerfd, the timeline answers its keys again.
            Change::Connection => self.settle_focus(window, cx),
            // Where Kerf works, for its dot when it ends unseen.
            Change::Work => {
                let session = self.session.read(cx);
                let work = &session.state.work;
                if work.busy() {
                    // A turn starts: what its cut has placed already is not new.
                    if self.working_in.is_none() {
                        self.live_seen = session
                            .live_cut()
                            .map(|(_, cut)| {
                                placed_since(session.timeline(), cut, &HashSet::new()).1
                            })
                            .unwrap_or_default();
                    }
                    self.working_in = work
                        .conversation
                        .clone()
                        .or_else(|| session.conversation.clone());
                }
            }
            // A turn that ends in the open conversation while the window is
            // in the back leaves its dot too, until the window comes to the
            // front on it.
            Change::TurnEnded => {
                self.live_seen.clear();
                if std::mem::take(&mut self.turn_rendered) {
                    self.go_home(cx);
                }
                let ended = self.working_in.take();
                if !window.is_window_active() {
                    if let Some(ended) = ended {
                        self.session.update(cx, |session, cx| {
                            if !session.state.settled.contains(&ended) {
                                session.state.settled.push(ended);
                                cx.notify();
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// Keeps the last error in its bar (what a view answers in place
    /// stays there); a failed turn's goes when the next turn starts.
    fn bridge_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            // The cut itself was rendered in this turn (not a range, a
            // draft or a proposal): it goes home when the turn ends.
            Event::Agent(AgentUpdate::Tool {
                rendered: Some(true),
                ..
            }) => {
                self.turn_rendered = true;
            }
            // When the project was last edited, for the title's tooltip.
            Event::State { snapshot } => {
                self.revision_time = snapshot
                    .revision_time
                    .as_deref()
                    .and_then(sidebar::parse_time);
            }
            Event::Error(error) => {
                let command = error.command.as_deref().unwrap_or_default();
                if !kept_in_bar(command) {
                    return;
                }
                // As the store said it: a message left out reads as empty.
                let raw = error.message.clone();
                // Saving over a preset of that name: the Save dialog asks
                // about it itself (the store kept the question).
                if crate::preset_file::existing_preset(&raw).is_some()
                    && self.session.read(cx).state.preset_exists.is_some()
                {
                    return;
                }
                // A file imported for a preset's slot, refused: said under
                // its slot in the dialog, not again here.
                if self.preset_importing(cx) && crate::presets::import_refusal(event).is_some() {
                    self.session.update(cx, |session, cx| {
                        session
                            .state
                            .notices
                            .retain(|notice| !(notice.error && notice.text == raw));
                        cx.notify();
                    });
                    return;
                }
                // Changing a conversation failed (renaming, archiving,
                // deleting, opening it): the notice says so, for
                // `scene::NOTICE_HOLD`, and the conversations are listed
                // again, so what was shown at once goes back to what is kept.
                if command == "conversation" {
                    self.session.update(cx, |session, cx| {
                        let said = session
                            .state
                            .notices
                            .iter_mut()
                            .rev()
                            .find(|notice| notice.error && notice.text == raw);
                        if let Some(notice) = said {
                            notice.text = conversation_error(&raw);
                            notice.hold = Some(scene::NOTICE_HOLD);
                        }
                        session.send(Command::Conversations(ConversationsRequest::List));
                        cx.notify();
                    });
                }
                let readable = readable_error(&raw);
                let text = if command == "forward" {
                    format!("Not brought forward · {readable}")
                } else {
                    readable
                };
                // A command sent while kerfd could not be reached: the
                // disconnected bar (or "Connecting…") says so already, so
                // no bar stacks under it, nor a notice over the conversation.
                if !bar_takes_error(&raw, &self.session.read(cx).state.connection) {
                    self.session.update(cx, |session, cx| {
                        session
                            .state
                            .notices
                            .retain(|notice| !(notice.error && notice.text == raw));
                        cx.notify();
                    });
                    return;
                }
                self.errors += 1;
                self.error = Some(ErrorBar {
                    raw,
                    text,
                    from_agent: command == "agent",
                    from_sync: false,
                    key: self.errors,
                });
                cx.notify();
            }
            // Connected again: what was refused while kerfd could not be
            // reached is not said over the new connection.
            Event::Connection {
                status: ConnectionStatus::Ready,
                ..
            } if self
                .error
                .as_ref()
                .is_some_and(|error| error.raw == DISCONNECTED) =>
            {
                self.error = None;
                cx.notify();
            }
            Event::AgentState {
                state: AgentState::Running,
                ..
            } if self.error.as_ref().is_some_and(|error| error.from_agent) => {
                self.error = None;
                cx.notify();
            }
            // A render this window queued is answered here first; once it
            // is done, a frame of the picture goes home to the project.
            // A captions file is no render, and names no job.
            Event::Export(Export::Render(render)) => {
                // A job id left out reads as empty: no job then.
                let job = render.job_id.as_str();
                if job.is_empty() {
                    return;
                }
                match render.status {
                    ExportStatus::Queued => {
                        self.queued_here.insert(job.to_owned());
                    }
                    ExportStatus::Completed => {
                        if self.queued_here.remove(job) {
                            self.fly_export_home(cx);
                        }
                    }
                    ExportStatus::Failed | ExportStatus::Cancelled => {
                        self.queued_here.remove(job);
                    }
                    ExportStatus::Running => {}
                }
            }
            _ => {}
        }
    }

    /// Whether Import Editing Preset's dialog is open over the window with
    /// a file for one of its slots coming in.
    fn preset_importing(&self, cx: &App) -> bool {
        self.modal
            .as_ref()
            .and_then(|(view, _)| view.clone().downcast::<crate::presets::ApplyPreset>().ok())
            .is_some_and(|dialog| dialog.read(cx).importing())
    }

    /// The Git chip's news: a failed sync goes into the error bar, which a
    /// later sync, started or done, takes away.
    fn collab_event(&mut self, event: &crate::finish::CollabEvent, cx: &mut Context<Self>) {
        match event {
            crate::finish::CollabEvent::SyncFailed { why, said: true } => {
                self.errors += 1;
                self.error = Some(ErrorBar {
                    raw: why.clone(),
                    text: sync_failed(why),
                    from_agent: false,
                    from_sync: true,
                    key: self.errors,
                });
            }
            crate::finish::CollabEvent::SyncFailed { said: false, .. } => {}
            crate::finish::CollabEvent::SyncCleared => {
                if self.error.as_ref().is_some_and(|error| error.from_sync) {
                    self.error = None;
                }
            }
        }
        // The toolbar's line follows either way.
        cx.notify();
    }
}
