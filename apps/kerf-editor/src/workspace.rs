//! The editor's window: one room. The folders and conversations at the
//! left, the conversation with Kerf beside them, and at the right a panel
//! with the picture, the Media band and the timeline once there is a cut
//! to show. What one goes into (Settings) is over the window, which dims.
//!
//! Every region moves by a [`Tween`] of its own: the sidebar slides in from
//! the window's edge, the panel from the right with its left edge against
//! the conversation's right as it narrows, the Media band down from under
//! the picture. State changes at once; motion follows.
//!
//! Under the toolbar, bars say what stands in the way until it is dealt
//! with: kerfd gone (Reconnect), project.kerf changed on disk (keep the
//! editor's version or load the file's), the last error (Dismiss). What
//! cannot be undone, and a folder that is not a project yet, are asked
//! first, over the window ([`Confirm`]).

use crate::chat::ChatView;
use crate::commands::{RunCommand, WORKSPACE};
use crate::export::{ExportEvent, ExportView};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::media::MediaBand;
use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::settings::{SettingsEvent, SettingsView};
use crate::sidebar::{self, SidebarEvent, SidebarView};
use crate::stage::StageView;
use crate::store::{Change, Connection};
use crate::theme::{self, layout, motion as scene, text};
use crate::timeline::TimelineView;
use crate::ui::{self, icon_button};
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, AnyView, App, ClickEvent, Context, DismissEvent,
    Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Rgba, SharedString, Subscription,
    Window,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The last error, kept in a bar under the toolbar until it is dismissed.
struct ErrorBar {
    /// As the bridge said it, to tell its notice from the rest.
    raw: String,
    /// As a person reads it.
    text: String,
    /// A failed turn's: the next turn takes it away.
    from_agent: bool,
    /// Its place among the errors, so a new one fades in afresh.
    key: u64,
}

pub struct Workspace {
    session: Entity<Session>,
    sidebar: Entity<SidebarView>,
    chat: Entity<ChatView>,
    stage: Entity<StageView>,
    timeline: Entity<TimelineView>,
    media: Entity<MediaBand>,
    inspector: Entity<crate::inspector::InspectorView>,
    formats: Entity<crate::inspector::FormatBank>,
    collab: Entity<crate::finish::Collab>,
    /// Settings while open, and how many times it has been opened.
    /// What one has gone into (Settings, a shot's graph, Finish), over the
    /// window, and how many times one has been opened.
    modal: Option<(AnyView, u64)>,
    modals_opened: u64,
    export: Option<Entity<ExportView>>,
    focus_handle: FocusHandle,
    sidebar_open: bool,
    sidebar_shown: Tween,
    panel_open: bool,
    panel_shown: Tween,
    /// The person closed the panel: a cut arriving does not open it again.
    panel_by_hand: bool,
    media_open: bool,
    media_shown: Tween,
    picture_shown: Tween,
    timeline_shown: Tween,
    dragging_sidebar: bool,
    /// The conversation's menu, from the chevron after its title.
    title_menu: bool,
    /// The app's menu, from the toolbar's ⋯ while the sidebar is away.
    app_menu: bool,
    title_field: Entity<TextInput>,
    /// The conversation whose title the toolbar edits in place.
    renaming_title: Option<String>,
    error: Option<ErrorBar>,
    errors: u64,
    /// A folder that was no project yet: its media comes in once kerfd has it.
    start_media: Option<PathBuf>,
    window_title: String,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|cx| SidebarView::new(session.clone(), window, cx));
        let chat = cx.new(|cx| ChatView::new(session.clone(), window, cx));
        let stage = cx.new(|cx| StageView::new(session.clone(), window, cx));
        let timeline = cx.new(|cx| TimelineView::new(session.clone(), window, cx));
        let media = cx.new(|cx| MediaBand::new(session.clone(), window, cx));
        let inspector =
            cx.new(|cx| crate::inspector::InspectorView::new(session.clone(), window, cx));
        let formats = cx.new(|cx| crate::inspector::FormatBank::new(session.clone(), window, cx));
        let collab = cx.new(|cx| crate::finish::Collab::new(session.clone(), window, cx));
        let sidebar_open = session.read(cx).prefs.sidebar_open;
        sidebar.update(cx, |sidebar, cx| sidebar.set_shown(sidebar_open, cx));
        let title_field =
            cx.new(|cx| TextInput::new(cx, "Conversation title").with_style(text::TITLE));
        let title_focus = title_field.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(&session, window, Self::session_changed),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::Settings => this.show_settings(window, cx),
                    SidebarEvent::Hide => this.set_sidebar(false, cx),
                    SidebarEvent::Delete(id) => this.confirm_delete(id.clone(), window, cx),
                    SidebarEvent::Done => {
                        if this.modal.is_none() {
                            window.focus(&this.focus_handle);
                        }
                    }
                    SidebarEvent::MenuOpened => {
                        this.title_menu = false;
                        this.app_menu = false;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &title_field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit | InputEvent::Cancel => {
                        this.finish_title_rename(*event == InputEvent::Submit, cx);
                        window.focus(&this.focus_handle);
                    }
                    InputEvent::Changed => {}
                },
            ),
            // Leaving the title's field keeps what was typed.
            cx.on_blur(&title_focus, window, |this, _, cx| {
                this.finish_title_rename(true, cx)
            }),
            // Esc puts an open menu away wherever the keys are: before the
            // timeline's Esc (Reject Proposal) or a field's has it. A menu's
            // press leaves the keys where they were.
            {
                let this = cx.weak_entity();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    if window.window_handle() != handle
                        || keystroke.key != "escape"
                        || keystroke.modifiers.modified()
                    {
                        return;
                    }
                    if this
                        .update(cx, |this, cx| this.close_menus(cx))
                        .unwrap_or(false)
                    {
                        cx.stop_propagation();
                    }
                })
            },
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            session,
            sidebar,
            chat,
            stage,
            timeline,
            media,
            inspector,
            formats,
            collab,
            modal: None,
            modals_opened: 0,
            export: None,
            focus_handle,
            sidebar_open,
            sidebar_shown: Tween::at(if sidebar_open { 1.0 } else { 0.0 }),
            panel_open: false,
            panel_shown: Tween::at(0.0),
            panel_by_hand: false,
            media_open: false,
            media_shown: Tween::at(0.0),
            picture_shown: Tween::at(1.0),
            timeline_shown: Tween::at(1.0),
            dragging_sidebar: false,
            title_menu: false,
            app_menu: false,
            title_field,
            renaming_title: None,
            error: None,
            errors: 0,
            start_media: None,
            window_title: String::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn session(&self) -> &Entity<Session> {
        &self.session
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
                self.panel_open = true;
                self.panel_shown.go(1.0, scene::STAGE);
                window.refresh();
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
    }

    /// Keeps the last error in its bar (what a view answers in place
    /// stays there); a failed turn's goes when the next turn starts.
    fn bridge_event(&mut self, event: &serde_json::Value, cx: &mut Context<Self>) {
        match event["type"].as_str().unwrap_or_default() {
            "error" => {
                let command = event["command"].as_str().unwrap_or_default();
                if !kept_in_bar(command) {
                    return;
                }
                let raw = event["message"]
                    .as_str()
                    .unwrap_or("Something went wrong")
                    .to_owned();
                let readable = readable_error(&raw);
                let text = if command == "forward" {
                    format!("Not brought forward · {readable}")
                } else {
                    readable
                };
                self.errors += 1;
                self.error = Some(ErrorBar {
                    raw,
                    text,
                    from_agent: command == "agent",
                    key: self.errors,
                });
                cx.notify();
            }
            "agent_state"
                if event["state"] == "running"
                    && self.error.as_ref().is_some_and(|error| error.from_agent) =>
            {
                self.error = None;
                cx.notify();
            }
            _ => {}
        }
    }

    /// Whether Kerf works in conversation `id` now.
    fn conversation_working(&self, id: &str, cx: &App) -> bool {
        let session = self.session.read(cx);
        let work = &session.state.work;
        work.busy()
            && (work.conversation.as_deref() == Some(id)
                || (work.conversation.is_none() && session.conversation.as_deref() == Some(id)))
    }

    /// Asks, over the window, before deleting conversation `id`; not while
    /// Kerf works in it.
    fn confirm_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.conversation_working(&id, cx) {
            return;
        }
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.label().to_owned())
        else {
            return;
        };
        let session = self.session.clone();
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("Delete “{title}”?"),
                Some("Its messages are removed from this project. This can't be undone."),
                "Delete",
                true,
                false,
                move |_, cx| {
                    session.update(cx, |session, _| session.delete_conversation(&id));
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// Renames the open conversation where it is named: in the sidebar
    /// while it shows, else in the toolbar.
    fn rename_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.read(cx).conversation.clone() else {
            return;
        };
        if self.sidebar_open {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.begin_rename(&id, window, cx));
        } else {
            self.begin_title_rename(window, cx);
        }
    }

    /// The toolbar's title becomes a field holding it.
    fn begin_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let Some(id) = session.conversation.clone() else {
            return;
        };
        let Some(title) = session
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
        else {
            return;
        };
        self.title_menu = false;
        self.renaming_title = Some(id);
        self.title_field
            .update(cx, |field, cx| field.set_text(title, cx));
        self.title_field.read(cx).focus(window);
        // The whole title is chosen, to type over, once the field is drawn.
        cx.on_next_frame(window, |_, window, cx| {
            cx.on_next_frame(window, |this, window, cx| {
                if this.renaming_title.is_some() {
                    window.dispatch_action(Box::new(crate::input::SelectAll), cx);
                }
            });
        });
        cx.notify();
    }

    /// Ends the toolbar's rename: `keep` sends the new title (an empty or
    /// unchanged one is not sent).
    fn finish_title_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(id) = self.renaming_title.take() else {
            return;
        };
        if keep {
            let typed = self.title_field.read(cx).text().to_owned();
            let old = self
                .session
                .read(cx)
                .state
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
                .map(|conversation| conversation.title.clone());
            if let Some(title) = old.and_then(|old| sidebar::renamed(&old, &typed)) {
                self.session
                    .update(cx, |session, _| session.rename_conversation(&id, &title));
            }
        }
        cx.notify();
    }

    /// What stands in the way, as the toolbar's line says it in red.
    fn standing_problem(&self, session: &Session) -> Option<String> {
        if !session.has_project() {
            return None;
        }
        match &session.state.connection {
            Connection::Failed(_) => Some("Offline".into()),
            Connection::Connecting => Some("Connecting…".into()),
            Connection::Connected => self.error.as_ref().map(|error| error.text.clone()),
        }
    }

    fn set_sidebar(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sidebar_open = open;
        self.app_menu = false;
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_shown(open, cx));
        self.sidebar_shown
            .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        self.session.update(cx, |session, _| {
            session.prefs.sidebar_open = open;
            session.save_prefs();
        });
        cx.notify();
    }

    pub fn set_panel(&mut self, open: bool, cx: &mut Context<Self>) {
        self.panel_open = open;
        self.panel_by_hand = !open;
        self.panel_shown
            .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        if open {
            if self.picture_shown.target() == 0.0 && self.timeline_shown.target() == 0.0 {
                self.picture_shown.set(1.0);
                self.timeline_shown.set(1.0);
            }
        } else if self.media_open {
            self.media_open = false;
            self.media_shown.go(0.0, scene::PANEL);
        }
        cx.notify();
    }

    fn set_media(&mut self, open: bool, cx: &mut Context<Self>) {
        self.media_open = open;
        if open && !self.panel_open {
            // With the panel closed, the panel comes in with the band in it.
            self.panel_open = true;
            self.panel_by_hand = false;
            self.panel_shown.go(1.0, scene::PANEL);
            self.media_shown.set(1.0);
        } else {
            self.media_shown
                .go(if open { 1.0 } else { 0.0 }, scene::PANEL);
        }
        cx.notify();
    }

    /// One of the panel's two, the picture or the timeline, on or off.
    fn toggle_pane(&mut self, picture: bool, cx: &mut Context<Self>) {
        let (this, other) = if picture {
            (&mut self.picture_shown, &self.timeline_shown)
        } else {
            (&mut self.timeline_shown, &self.picture_shown)
        };
        if !self.panel_open {
            this.set(1.0);
            self.set_panel(true, cx);
            return;
        }
        let showing = this.target() > 0.5;
        if showing && other.target() < 0.5 {
            // The last of the two going closes the panel.
            self.set_panel(false, cx);
            return;
        }
        this.go(if showing { 0.0 } else { 1.0 }, scene::PANEL);
        cx.notify();
    }

    pub fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        let settings = cx.new(|cx| SettingsView::new(self.session.clone(), window, cx));
        cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Dismiss => this.close_modal(window, cx),
            },
        )
        .detach();
        window.focus(&settings.focus_handle(cx));
        self.put_modal(settings.into(), cx);
    }

    /// Goes into `view` over the window: the window dims at once and the
    /// view rises into its middle. It gives the window back by emitting
    /// [`DismissEvent`]. One is open at a time.
    pub fn open_modal<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + Focusable + EventEmitter<DismissEvent>,
    {
        cx.subscribe_in(&view, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_modal(window, cx)
        })
        .detach();
        window.focus(&view.focus_handle(cx));
        self.put_modal(view.into(), cx);
    }

    fn put_modal(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.close_menus(cx);
        self.modals_opened += 1;
        self.modal = Some((view, self.modals_opened));
        cx.notify();
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn toggle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.export.take().is_some() {
            cx.notify();
            return;
        }
        let export = cx.new(|cx| ExportView::new(self.session.clone(), window, cx));
        cx.subscribe(&export, |this, _, event: &ExportEvent, cx| match event {
            ExportEvent::Dismiss => {
                this.export = None;
                cx.notify();
            }
        })
        .detach();
        self.session.update(cx, |session, _| {
            session.send(serde_json::json!({"type": "render_queue"}));
        });
        self.export = Some(export);
        cx.notify();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = session.update(cx, |session, cx| {
                for path in &paths {
                    session.import(path, false);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a folder in place of this project. One that is not a project
    /// yet is asked about first; this project is open already.
    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        let current = self.session.read(cx).project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(project) = paths.into_iter().next() else {
                return;
            };
            if current
                .as_deref()
                .is_some_and(|current| same_folder(current, &project))
            {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if needs_start(&project) {
                    this.ask_to_start(project, window, cx);
                } else {
                    open_folder(project, false, cx);
                    window.remove_window();
                }
            });
        })
        .detach();
    }

    /// "“X” is not a project yet. Start one here, with its media?"
    fn ask_to_start(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        let name = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.display().to_string());
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("“{name}” is not a project yet."),
                Some("Start one here, with its media?"),
                "Start",
                false,
                true,
                move |window, cx| {
                    open_folder(project, true, cx);
                    window.remove_window();
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// Carries out a command by its id: what the window does itself, else
    /// the request the registry resolves it to.
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        let session = self.session.clone();
        match id {
            "app.quit" => cx.quit(),
            "app.preferences" | "app.account" => self.show_settings(window, cx),
            "view.sidebar" => self.set_sidebar(!self.sidebar_open, cx),
            "view.picture" => self.toggle_pane(true, cx),
            "view.timeline" => self.toggle_pane(false, cx),
            "window.project" => self.set_media(!self.media_open, cx),
            "window.assistant" | "ai.ask" => self
                .chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx)),
            "file.import" => self.import(window, cx),
            "file.openProject" => self.open_project(window, cx),
            // The launcher, in a window of its own: a new project, a clone, or a recent one.
            "file.newProject" => crate::app::open(None, None, cx),
            "file.export" => self.toggle_export(window, cx),
            "export.master" => session.update(cx, |session, _| session.deliver("master")),
            "file.refresh" => session.update(cx, |session, _| {
                session.send(serde_json::json!({"type": "refresh"}));
            }),
            "edit.undo" => session.update(cx, |session, _| session.undo()),
            "edit.redo" => session.update(cx, |session, _| session.redo()),
            "playback.toggle" => session.update(cx, |session, _| session.toggle_playback()),
            "playback.goToStart" => session.update(cx, |session, _| session.seek(0.0, false)),
            "playback.goToEnd" => session.update(cx, |session, _| {
                let end = session.state.program_length();
                session.seek(end, false);
            }),
            "playback.previousFrame" | "playback.nextFrame" => session.update(cx, |session, _| {
                let step = 1.0 / session.timeline().rate();
                let step = if id == "playback.nextFrame" {
                    step
                } else {
                    -step
                };
                let at = session.state.position + step;
                session.seek(at.max(0.0), false);
            }),
            "ai.stop" => session.update(cx, |session, _| session.stop()),
            "proposal.accept" | "proposal.reject" | "proposal.forward" => {
                session.update(cx, |session, _| {
                    let Some(proposal) = session
                        .state
                        .snapshot
                        .pending()
                        .map(|p| p.proposal_id.clone())
                    else {
                        return;
                    };
                    match id {
                        "proposal.accept" => session.accept(&proposal),
                        "proposal.reject" => session.reject(&proposal),
                        _ => session.forward(&proposal),
                    }
                })
            }
            "help.guide" => cx.open_url(&user_guide()),
            "help.about" => self.show_settings(window, cx),
            // Where crash reports are kept, in the system's file manager.
            "help.crashReports" => match kerf_bridge::crash_reports()["folder"].as_str() {
                Some(folder) => {
                    let folder = PathBuf::from(folder);
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                }
                None => session.update(cx, |session, _| {
                    session.state.notify(
                        "There is nowhere to keep crash reports on this computer",
                        true,
                    )
                }),
            },
            "mark.in" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_in(Some(at));
            }),
            "mark.out" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_out(Some(at));
            }),
            "mark.clear" => session.update(cx, |session, _| {
                session.set_mark_in(None);
                session.set_mark_out(None);
            }),
            "view.safeZones" => session.update(cx, |session, _| {
                session.prefs.safe_zones = !session.prefs.safe_zones;
                session.save_prefs();
            }),
            "playback.loop" => session.update(cx, |session, _| session.toggle_loop()),
            "view.followKerf" => session.update(cx, |session, _| {
                session.prefs.follow_kerf = !session.prefs.follow_kerf;
                session.save_prefs();
            }),
            "qc.run" => {
                window.dispatch_action(Box::new(crate::finish::OpenFinish { run_check: true }), cx)
            }
            _ => session.update(cx, |session, _| {
                session.run_command(id, serde_json::json!({}))
            }),
        }
        cx.notify();
    }

    /// Esc in the window (outside a field, the timeline, and whatever is
    /// open over the window): stops a preview rendering, else puts the
    /// Media band away.
    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.close_menus(cx) {
            return;
        }
        let rendering = self.session.read(cx).state.preview.rendering();
        if rendering {
            self.session.update(cx, |session, _| {
                session.send(serde_json::json!({"type": "preview_stop"}));
            });
        } else if self.media_open {
            self.set_media(false, cx);
        }
    }

    /// Puts away whichever menu is open: the title's, the toolbar's app
    /// menu, the sidebar's. Whether one was.
    fn close_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let mine = self.title_menu || self.app_menu;
        if mine {
            self.title_menu = false;
            self.app_menu = false;
            cx.notify();
        }
        let sidebar = self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.close_menus(cx));
        mine || sidebar
    }

    fn sidebar_width(&self, session: &Session) -> f32 {
        session
            .prefs
            .sidebar_width
            .clamp(layout::SIDEBAR_LEAST, 900.0)
    }

    fn on_drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging_sidebar {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_drag(cx);
            return;
        }
        let most = layout::sidebar_most(f32::from(window.viewport_size().width));
        let width = f32::from(event.position.x).clamp(layout::SIDEBAR_LEAST, most);
        self.session
            .update(cx, |session, _| session.prefs.sidebar_width = width);
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragging_sidebar {
            self.dragging_sidebar = false;
            self.session.read(cx).save_prefs();
            cx.notify();
        }
    }

    fn toolbar(&self, leading: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let project = session.project_name();
        // The title is the open conversation's name, the project after it;
        // with none open, the project's.
        let conversation = session.conversation.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
                .map(|conversation| conversation.label().to_owned())
                .or_else(|| {
                    session
                        .state
                        .open_conversation
                        .as_ref()
                        .filter(|conversation| &conversation.id == id)
                        .map(|conversation| conversation.title.trim().to_owned())
                        .filter(|title| !title.is_empty())
                })
        });
        let title = conversation.clone().unwrap_or_else(|| project.clone());
        let subtitle: Option<(String, Rgba)> = match self.standing_problem(session) {
            Some(problem) => Some((problem, theme::neg())),
            None => conversation
                .is_some()
                .then(|| (project.clone(), theme::ink3())),
        };
        let follow = session.prefs.follow_kerf;
        let can_undo = session.state.snapshot.can_undo;
        let can_redo = session.state.snapshot.can_redo;
        let rendering = session.state.rendering();
        let exporting = session.state.renders.iter().any(|job| job.is_active());
        let renaming = self.renaming_title.is_some();
        let title_menu = self.title_menu;
        div()
            .h(px(layout::TOOLBAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .when(leading, |bar| {
                bar.child(
                    icon_button("show-sidebar", Icon::Sidebar)
                        .tooltip("Sidebar")
                        .on_click(cx.listener(|this, _, _, cx| this.set_sidebar(true, cx))),
                )
                // With the sidebar away, its account's menu opens from here.
                .child(
                    sidebar::trigger(
                        "toolbar-app-menu",
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.app_menu = !this.app_menu;
                            this.title_menu = false;
                            this.sidebar
                                .update(cx, |sidebar, cx| sidebar.close_menus(cx));
                            cx.notify();
                        }),
                    )
                    .child(
                        icon_button("toolbar-app-menu-button", Icon::More)
                            .tooltip("Settings, account and more")
                            .on(self.app_menu),
                    ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .pl(px(8.0))
                    .min_w_0()
                    .child(if renaming {
                        // Renamed where it is named: the field takes the title's place.
                        div()
                            .w(px(TITLE_FIELD))
                            .h(px(theme::height::H28))
                            .flex()
                            .items_center()
                            .ml(px(-5.0))
                            .px(px(4.0))
                            .rounded(px(theme::radius::SMALL))
                            .bg(theme::card())
                            .border_1()
                            .border_color(theme::accent())
                            // Esc reaches the field's holder as a key, whatever
                            // the keymap leaves bound in fields.
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    this.finish_title_rename(false, cx);
                                    window.focus(&this.focus_handle);
                                }
                            }))
                            .child(self.title_field.clone())
                            .into_any_element()
                    } else {
                        // Pressed, what can be done with the conversation;
                        // pressed twice, its name becomes a field.
                        sidebar::trigger(
                            "toolbar-title",
                            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                this.app_menu = false;
                                this.sidebar
                                    .update(cx, |sidebar, cx| sidebar.close_menus(cx));
                                if event.click_count >= 2 {
                                    this.title_menu = false;
                                    this.begin_title_rename(window, cx);
                                } else {
                                    this.title_menu = !this.title_menu;
                                }
                                cx.notify();
                            }),
                        )
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .min_w_0()
                        .h(px(theme::height::H28))
                        .pl(px(4.0))
                        .pr(px(2.0))
                        .ml(px(-4.0))
                        .rounded(px(theme::radius::CONTROL))
                        .cursor_pointer()
                        .when(title_menu, |title| title.bg(theme::raised()))
                        .hover(|title| title.bg(theme::raised()))
                        .when(!title_menu, |title| {
                            title.tooltip(|_, cx| {
                                cx.new(|_| ui::Tooltip("Rename, archive, more".into()))
                                    .into()
                            })
                        })
                        .child(ui::line(title, text::TITLE, theme::ink()))
                        .child(Icon::ChevronDown.view(14.0, theme::ink3()))
                        .into_any_element()
                    })
                    .when_some(subtitle, |row, (words, color)| {
                        row.child(ui::line(words, text::CONTROL, color).min_w_0())
                    }),
            )
            .child(div().flex_1())
            .child(self.formats.clone())
            .child(self.collab.clone())
            .when_some(rendering, |bar, rendering| {
                bar.child(
                    ui::chip(rendering, theme::activity::gpu())
                        .child(ui::dot(theme::activity::gpu(), 6.0)),
                )
            })
            .child(
                icon_button("undo", Icon::Undo)
                    .tooltip("Undo")
                    .disabled(!can_undo)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, _| session.undo())
                    })),
            )
            .child(
                icon_button("redo", Icon::Redo)
                    .tooltip("Redo")
                    .disabled(!can_redo)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, _| session.redo())
                    })),
            )
            .child(
                ui::button("finish", "Finish")
                    .small()
                    .ghost()
                    .icon(Icon::Check)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::finish::OpenFinish { run_check: false }),
                            cx,
                        )
                    }),
            )
            .child(
                ui::button("export", "Export")
                    .icon(Icon::Export)
                    .small()
                    .kind(if exporting {
                        ui::Kind::Primary
                    } else {
                        ui::Kind::Secondary
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_export(window, cx))),
            )
            .child(
                // While Kerf works, the playhead goes where it looks.
                icon_button("follow-kerf", Icon::Eye)
                    .tooltip("Follow Kerf")
                    .on(follow)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(RunCommand::new("view.followKerf")), cx)
                    }),
            )
            .child(
                icon_button("media", Icon::Media)
                    .tooltip("Media")
                    .on(self.media_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_media(!this.media_open, cx))),
            )
            .child(
                icon_button("panel", Icon::Panel)
                    .tooltip("Panel")
                    .on(self.panel_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_panel(!this.panel_open, cx))),
            )
    }

    /// What can be done with the open conversation, under its title.
    fn title_menu_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let id = session.conversation.clone();
        let connected = session.state.connection == Connection::Connected;
        let summary = id.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
        });
        let kept = summary.is_some() && connected;
        let archived = summary.is_some_and(|summary| summary.archived);
        let working = id
            .as_deref()
            .is_some_and(|id| self.conversation_working(id, cx));
        let can_delete = kept && !working;
        sidebar::menu_panel()
            .w(px(200.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.title_menu = false;
                cx.notify();
            }))
            .child(
                sidebar::menu_item("title-rename", "Rename", None, false, !kept).when(
                    kept,
                    |item| {
                        item.on_click(
                            cx.listener(|this, _, window, cx| this.begin_title_rename(window, cx)),
                        )
                    },
                ),
            )
            .child(
                sidebar::menu_item(
                    "title-archive",
                    if archived { "Unarchive" } else { "Archive" },
                    Some(Icon::Archive),
                    false,
                    !kept,
                )
                .when_some(id.clone().filter(|_| kept), |item, id| {
                    item.on_click(cx.listener(move |this, _, _, cx| {
                        this.title_menu = false;
                        this.session.update(cx, |session, _| {
                            session.archive_conversation(&id, !archived)
                        });
                        cx.notify();
                    }))
                }),
            )
            .child(
                sidebar::menu_item(
                    "title-delete",
                    "Delete…",
                    Some(Icon::Trash),
                    true,
                    !can_delete,
                )
                .when_some(id.filter(|_| can_delete), |item, id| {
                    item.on_click(cx.listener(move |this, _, window, cx| {
                        this.title_menu = false;
                        this.confirm_delete(id.clone(), window, cx);
                    }))
                }),
            )
            .child(sidebar::menu_separator())
            .child(
                sidebar::menu_item(
                    "title-agent",
                    "Agent settings…",
                    Some(Icon::Sparkle),
                    false,
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.title_menu = false;
                    this.show_settings(window, cx);
                })),
            )
            .with_animation("title-menu", motion::animation(scene::FADE), |menu, t| {
                menu.opacity(t)
            })
    }

    /// The bars under the toolbar, each until what it says is dealt with:
    /// kerfd gone, project.kerf changed on disk, the last error.
    fn bars(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let mut bars = Vec::new();
        if let (true, Connection::Failed(reason)) =
            (session.has_project(), &session.state.connection)
        {
            bars.push(bar(
                "bar-disconnected".into(),
                theme::neg(),
                format!("Project disconnected. {reason}").into(),
                None,
                vec![ui::button("bar-reconnect", "Reconnect")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.reconnect();
                            cx.notify();
                        })
                    }))
                    .into_any_element()],
            ));
        }
        if session.state.snapshot.projection_status == "diverged" {
            bars.push(bar(
                "bar-diverged".into(),
                theme::warn(),
                "project.kerf changed on disk.".into(),
                Some(
                    "Load it to bring in edits made outside Kerf (a git pull, a text editor), \
                     or keep the editor's version and write it over the file."
                        .into(),
                ),
                vec![
                    ui::button("bar-keep-editor", "Keep Editor Version")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(serde_json::json!({"type": "projection_repair"}));
                            })
                        }))
                        .into_any_element(),
                    ui::button("bar-load-disk", "Load from Disk")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(serde_json::json!({"type": "projection_import"}));
                            })
                        }))
                        .into_any_element(),
                ],
            ));
        }
        if let Some(error) = &self.error {
            bars.push(bar(
                format!("bar-error-{}", error.key).into(),
                theme::neg(),
                error.text.clone().into(),
                None,
                vec![ui::button("bar-dismiss", "Dismiss")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element()],
            ));
        }
        bars
    }

    fn panel(&self, width: f32, height: f32, inspecting: bool, window: &Window) -> AnyElement {
        let picture = self.picture_shown.sample(window);
        let timeline = self.timeline_shown.sample(window);
        let media = self.media_shown.sample(window);
        let band = layout::MEDIA_BAND * media;
        // The timeline keeps its height; the picture takes the rest. One of
        // the two alone has all of it.
        let timeline_full = (height - band).max(0.0);
        let timeline_tall = layout::TIMELINE.min(timeline_full * 0.55);
        let timeline_height = timeline_tall + (timeline_full - timeline_tall) * (1.0 - picture);
        let timeline_height = timeline_height * timeline;
        let picture_height = (height - band - timeline_height).max(0.0);
        div()
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(picture_height))
                    .w_full()
                    .flex_none()
                    .flex()
                    .overflow_hidden()
                    .child(div().flex_1().min_w_0().h_full().child(self.stage.clone()))
                    .when(inspecting, |row| {
                        row.child(
                            div()
                                .w(px(layout::INSPECTOR))
                                .flex_none()
                                .h_full()
                                .border_l_1()
                                .border_color(theme::line())
                                .child(self.inspector.clone()),
                        )
                    }),
            )
            .child(
                // The band comes down out from under the picture.
                div()
                    .h(px(band))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(layout::MEDIA_BAND))
                            .child(self.media.clone()),
                    ),
            )
            .child(
                div()
                    .h(px(timeline_height))
                    .w_full()
                    .flex_none()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(theme::line())
                    .child(self.timeline.clone()),
            )
            .into_any_element()
    }

    fn notices(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut notices = self.session.read(cx).state.notices.clone();
        // What the error bar holds is said there, not again here.
        if let Some(error) = &self.error {
            notices.retain(|notice| !(notice.error && notice.text == error.raw));
        }
        div()
            .absolute()
            .bottom(px(120.0))
            .left_0()
            .right_0()
            .px(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.0))
            .children(notices.into_iter().map(|notice| {
                let color = if notice.error {
                    theme::neg()
                } else {
                    theme::pos()
                };
                let key = notice.id;
                theme::type_style(div(), text::CONTROL)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .max_w(px(560.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(theme::radius::CARD))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .text_color(theme::ink())
                    .child(ui::dot(color, 6.0))
                    .child(notice.text.clone())
                    .with_animation(
                        ("notice", key),
                        motion::animation(scene::APPEAR),
                        |notice, t| notice.opacity(t),
                    )
            }))
    }

    fn modal(&self, window: &Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let (settings, opened) = self.modal.as_ref()?;
        let key = *opened;
        let _ = window;
        Some(
            div()
                .id("scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                // The window dims at once.
                .bg(theme::with_alpha(gpui::rgb(0x000000), scene::SCRIM))
                // Nothing under the dim answers the pointer, not even its hover.
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .relative()
                        .rounded(px(theme::radius::PANEL))
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme::line2())
                        .shadow_2xl()
                        .child(settings.clone())
                        // Its body comes up into place.
                        .with_animation(
                            ("settings-rise", key),
                            motion::animation(scene::PAGE),
                            |body, t| {
                                body.top(px(scene::PAGE_RISE * (1.0 - t)))
                                    .opacity(0.2 + 0.8 * t)
                            },
                        ),
                )
                .into_any_element(),
        )
    }
}

/// The user guide the package carries (beside the editor on Windows, in the
/// app's Resources on macOS, in share/doc on Linux), else the one on GitHub.
fn user_guide() -> String {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    let candidates = beside.into_iter().flat_map(|folder| {
        [
            folder.join("user-guide.html"),
            folder.join("../Resources/user-guide.html"),
            folder.join("../share/doc/industrial-kerf/user-guide.html"),
        ]
    });
    for candidate in candidates {
        if let Ok(path) = candidate.canonicalize() {
            // A Windows path as a URL: forward slashes, after a third.
            let text = path.display().to_string();
            let text = text.trim_start_matches(r"\\?\").replace('\\', "/");
            return if text.starts_with('/') {
                format!("file://{text}")
            } else {
                format!("file:///{text}")
            };
        }
    }
    format!("{}/blob/main/docs/user-guide.md", kerf_bridge::REPOSITORY)
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width), f32::from(viewport.height));
        let sidebar_width = self.sidebar_width(self.session.read(cx));
        let sidebar = self.sidebar_shown.sample(window);
        let panel = self.panel_shown.sample(window);
        let main_left = sidebar_width * sidebar;
        let main_width = (window_width - main_left).max(0.0);
        let bars = self.bars(cx);
        let bars_height = BAR * bars.len() as f32;
        let content_height = (window_height - layout::TOOLBAR - bars_height).max(0.0);
        // The conversation keeps a rail beside an open panel.
        let rail = layout::RAIL
            .min((main_width * 0.45).max(layout::RAIL_LEAST))
            .min(main_width);
        let panel_width = (main_width - rail).max(0.0);
        let conversation_width = main_width - panel_width * panel;
        let leading = sidebar < 0.02;
        // The chosen clip's inspector stands beside the picture when there is room.
        let inspecting =
            !self.session.read(cx).selection.is_empty() && panel_width > layout::INSPECTOR * 2.2;
        // Where the title starts, for its menu to open under it.
        let title_left = main_left + if leading { 84.0 } else { 16.0 };
        let provider: SharedString = self.session.read(cx).prefs.provider.label().into();
        let close_app_menu: Rc<dyn Fn(&mut Window, &mut App)> = {
            let this = cx.entity().downgrade();
            Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.app_menu = false;
                    cx.notify();
                });
            })
        };

        div()
            .key_context(WORKSPACE)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::run_command))
            .on_action(cx.listener(|this, _: &crate::commands::Escape, _, cx| this.escape(cx)))
            .on_action(
                cx.listener(|this, _: &crate::commands::RevealPicture, _, cx| {
                    // Looking at something opens the picture, as by hand.
                    if !this.panel_open {
                        this.set_panel(true, cx);
                    } else if this.picture_shown.target() < 0.5 {
                        this.toggle_pane(true, cx);
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::palette::OpenPalette, window, cx| {
                    if this.modal.is_none() {
                        let palette = cx.new(|cx| {
                            crate::palette::CommandPalette::new(this.session.clone(), window, cx)
                        });
                        this.open_modal(palette, window, cx);
                    }
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::making::OpenShotGraph, window, cx| {
                    let graph = cx.new(|cx| {
                        crate::making::ShotGraph::new(
                            this.session.clone(),
                            action.media.clone(),
                            window,
                            cx,
                        )
                    });
                    this.open_modal(graph, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::making::MakeMedia, window, cx| {
                    let plan = cx.new(|cx| {
                        crate::making::MakePlan::new(
                            this.session.clone(),
                            action.media.clone(),
                            action.rolls,
                            window,
                            cx,
                        )
                    });
                    this.open_modal(plan, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::finish::OpenFinish, window, cx| {
                    let finish = cx.new(|cx| {
                        crate::finish::FinishView::new(
                            this.session.clone(),
                            action.run_check,
                            window,
                            cx,
                        )
                    });
                    this.open_modal(finish, window, cx);
                }),
            )
            // F2 renames the open conversation where it is named.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "f2"
                    && !event.keystroke.modifiers.modified()
                    && this.modal.is_none()
                {
                    cx.stop_propagation();
                    this.rename_open(window, cx);
                }
            }))
            .on_mouse_move(cx.listener(Self::on_drag))
            // Files dropped on the window come into the project's media.
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.session.update(cx, |session, cx| {
                    for path in paths.paths() {
                        session.import(path, true);
                    }
                    cx.notify();
                });
                if !this.media_open {
                    this.set_media(true, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_drag(cx)),
            )
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(theme::surround())
            .font(theme::ui_font(gpui::FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(
                // The sidebar slides in from the window's edge.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-sidebar_width * (1.0 - sidebar)))
                    .w(px(sidebar_width))
                    .child(self.sidebar.clone())
                    .child(
                        // Its edge takes a drag, and shows a line under the pointer.
                        div()
                            .id("sidebar-grip")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(-layout::SIDEBAR_GRIP / 2.0))
                            .w(px(layout::SIDEBAR_GRIP))
                            .cursor_col_resize()
                            .when(self.dragging_sidebar, |grip| {
                                grip.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(px(3.0))
                                        .w(px(2.0))
                                        .bg(theme::lineh()),
                                )
                            })
                            .hover(|grip| grip.bg(theme::with_alpha(theme::lineh(), 0.25)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.dragging_sidebar = true;
                                    cx.notify();
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(main_left))
                    .w(px(main_width))
                    .flex()
                    .flex_col()
                    .child(self.toolbar(leading, cx))
                    .children(bars)
                    .child(
                        div()
                            .relative()
                            .h(px(content_height))
                            .w_full()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(px(conversation_width))
                                    .child(self.chat.clone()),
                            )
                            .when(panel > 0.001, |content| {
                                content.child(
                                    // The panel comes in from the right, its left edge
                                    // against the conversation's right.
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(px(conversation_width))
                                        .w(px(panel_width))
                                        .pr(px(8.0))
                                        .pb(px(8.0))
                                        .child(self.panel(
                                            panel_width - 8.0,
                                            content_height - 8.0,
                                            inspecting,
                                            window,
                                        )),
                                )
                            }),
                    ),
            )
            .when_some(self.export.clone(), |root, export| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR))
                        .right(px(80.0))
                        .child(export)
                        .with_animation(
                            "export-pop",
                            motion::animation(scene::FADE),
                            |popover, t| popover.opacity(t).mt(px(4.0 * (1.0 - t))),
                        ),
                )
            })
            .when(self.title_menu, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(title_left))
                        .child(self.title_menu_view(cx)),
                )
            })
            .when(self.app_menu && leading, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(main_left + 40.0))
                        .child(
                            sidebar::app_menu(provider.clone(), close_app_menu.clone())
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.app_menu = false;
                                    cx.notify();
                                })),
                        )
                        .with_animation(
                            "toolbar-app-menu",
                            motion::animation(scene::FADE),
                            |menu, t| menu.opacity(t),
                        ),
                )
            })
            .children(self.modal(window, cx))
            // Notices sit over the conversation, above what is open over the window.
            .child(
                div()
                    .absolute()
                    .top(px(layout::TOOLBAR + bars_height))
                    .bottom_0()
                    .left(px(main_left))
                    .w(px(conversation_width))
                    .child(self.notices(cx)),
            )
    }
}

/// Pixels as points.
pub fn points(value: Pixels) -> f32 {
    f32::from(value)
}

/// A bar's height under the toolbar.
const BAR: f32 = theme::height::H40;
/// The toolbar's title field, while a conversation is renamed there.
const TITLE_FIELD: f32 = 280.0;

/// A bar under the toolbar: its colour in the dot only, the bar and its
/// words neutral; what it says, then its buttons. It fades in where it is.
fn bar(
    id: SharedString,
    color: Rgba,
    title: SharedString,
    detail: Option<SharedString>,
    buttons: Vec<AnyElement>,
) -> AnyElement {
    div()
        .id(id.clone())
        .h(px(BAR))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .bg(theme::chrome())
        .border_b_1()
        .border_color(theme::line())
        .child(ui::dot(color, 7.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_1()
                .min_w_0()
                .child(
                    ui::line(title, text::CONTROL_STRONG, theme::ink())
                        .min_w_0()
                        .when(detail.is_some(), |title| title.flex_none()),
                )
                .when_some(detail, |row, detail| {
                    row.child(
                        ui::line(detail, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
                }),
        )
        .children(buttons)
        .with_animation(id, motion::animation(scene::FADE), |bar, t| bar.opacity(t))
        .into_any_element()
}

/// Whether an error the bridge sends stays in the error bar: not what a
/// view answers in place (a making, a shot's graph, a recipe, a workflow).
pub fn kept_in_bar(command: &str) -> bool {
    !(matches!(command, "media_make" | "media_carry" | "takes_read")
        || command.starts_with("recipe")
        || command.starts_with("shot_")
        || command.starts_with("workflow"))
}

/// An error as a person reads it: an engine diagnostic's severity and code
/// ("error[ripple.blocked]: ", "warning[x.y] 3:14: ") are for tools.
pub fn readable_error(message: &str) -> String {
    let Some(rest) = message
        .strip_prefix("error[")
        .or_else(|| message.strip_prefix("warning["))
    else {
        return message.to_owned();
    };
    let Some(close) = rest.find(']') else {
        return message.to_owned();
    };
    let code = &rest[..close];
    if code.is_empty()
        || !code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return message.to_owned();
    }
    let after = &rest[close + 1..];
    // An optional " line:column".
    let position = |text: &str| -> Option<usize> {
        let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
        let tail = text.strip_prefix(' ')?;
        let line = digits(tail);
        let tail = tail.get(line..)?.strip_prefix(':').filter(|_| line > 0)?;
        let column = digits(tail);
        (column > 0).then_some(1 + line + 1 + column)
    };
    let after = match position(after) {
        Some(skip) if after[skip..].starts_with(": ") => &after[skip..],
        _ => after,
    };
    match after.strip_prefix(": ") {
        Some(text) if !text.trim().is_empty() => text.to_owned(),
        _ => message.to_owned(),
    }
}

/// The media files a folder that becomes a project brings in.
pub const FOLDER_MEDIA: [&str; 7] = ["mov", "mp4", "m4v", "mkv", "webm", "wav", "mp3"];

/// The media files directly in `folder`, by name.
pub fn folder_media(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    FOLDER_MEDIA.contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .collect();
    files.sort();
    files
}

/// The window's title: the project, and its sequence when that is named
/// otherwise ("Atlas · 30 s spot v7").
pub fn window_title(project: &str, timeline: &crate::model::Timeline) -> String {
    let sequence = timeline
        .sequence
        .as_deref()
        .and_then(|id| timeline.sequences.iter().find(|sequence| sequence.id == id))
        .map(|sequence| sequence.name.trim())
        .unwrap_or_default();
    if sequence.is_empty() || sequence == project {
        project.to_owned()
    } else if project.is_empty() {
        sequence.to_owned()
    } else {
        format!("{project} · {sequence}")
    }
}

/// Whether `folder` is no project yet: it has no project.kerf.
pub fn needs_start(folder: &Path) -> bool {
    !folder.join("project.kerf").exists()
}

/// Whether two paths name the same folder.
fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Opens `project` in a window of its own. `starts` when it was no project
/// yet: once kerfd has it, its folder's media comes in (the launcher's
/// `LauncherEvent::Open { starts }` means the same).
pub fn open_folder(project: PathBuf, starts: bool, cx: &mut App) {
    crate::app::open(Some(project.clone()), None, cx);
    if !starts {
        return;
    }
    for handle in cx.windows() {
        let Some(workspace) = handle.downcast::<Workspace>() else {
            continue;
        };
        // The window being updated (this one) answers Err, and is left be.
        let _ = workspace.update(cx, |workspace, _, cx| {
            let session = workspace.session.read(cx);
            let fresh = !session.state.has_snapshot;
            let here = session
                .project
                .as_deref()
                .is_some_and(|open| same_folder(open, &project));
            if fresh && here && workspace.start_media.is_none() {
                workspace.start_media = Some(project.clone());
            }
        });
    }
}

gpui::actions!(confirm, [CloseConfirm, ConfirmDefault]);

/// The key context a question over the window answers in.
const CONFIRM: &str = "Confirm";

/// Esc and Enter answer the question before the window's own keys have
/// them. Binding a keymap clears these, so they are bound again whenever a
/// question opens and finds them missing.
fn bind_confirm(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &CloseConfirm) {
        cx.bind_keys([
            KeyBinding::new("escape", CloseConfirm, Some(CONFIRM)),
            KeyBinding::new("enter", ConfirmDefault, Some(CONFIRM)),
        ]);
    }
}

/// A question over the window, before what cannot be undone or what
/// changes the window: Cancel, or what it asks. Esc, or a press on the dim
/// around it, cancels; Enter does the default (Cancel, unless said).
pub struct Confirm {
    focus_handle: FocusHandle,
    title: SharedString,
    detail: Option<SharedString>,
    action: SharedString,
    /// What it asks cannot be undone: its button is in brick.
    danger: bool,
    /// Enter does what it asks rather than cancelling.
    act_by_default: bool,
    on_confirm: Option<Box<dyn FnOnce(&mut Window, &mut App)>>,
}

impl EventEmitter<DismissEvent> for Confirm {}

impl Confirm {
    pub fn new(
        cx: &mut Context<Self>,
        title: impl Into<SharedString>,
        detail: Option<&str>,
        action: impl Into<SharedString>,
        danger: bool,
        act_by_default: bool,
        on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
    ) -> Self {
        bind_confirm(cx);
        Self {
            focus_handle: cx.focus_handle(),
            title: title.into(),
            detail: detail.map(|detail| detail.to_owned().into()),
            action: action.into(),
            danger,
            act_by_default,
            on_confirm: Some(Box::new(on_confirm)),
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.on_confirm = None;
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_confirm) = self.on_confirm.take() {
            cx.emit(DismissEvent);
            on_confirm(window, cx);
        }
    }
}

impl Focusable for Confirm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Confirm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONFIRM)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &CloseConfirm, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &ConfirmDefault, window, cx| {
                if this.act_by_default {
                    this.confirm(window, cx);
                } else {
                    this.cancel(cx);
                }
            }))
            // A press on the dim around it cancels, and does nothing to
            // what lies under the dim.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.cancel(cx);
            }))
            .w(px(CONFIRM_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(theme::popup())
            .child(ui::label(self.title.clone(), text::TITLE, theme::ink()))
            .when_some(self.detail.clone(), |body, detail| {
                body.child(ui::label(detail, text::BODY, theme::ink2()))
            })
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        ui::button("confirm-cancel", "Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx))),
                    )
                    .child(
                        ui::button("confirm-action", self.action.clone())
                            .kind(if self.danger {
                                ui::Kind::Danger
                            } else {
                                ui::Kind::Primary
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm(window, cx)
                            })),
                    ),
            )
    }
}

/// A question's width over the window.
const CONFIRM_WIDTH: f32 = 420.0;
