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
//!
//! A command that cannot run now says why, as a notice, and the toolbar's
//! buttons are greyed with the reason (and their keys) in their tooltips.
//! Another project, new, chosen or recent, opens in place of this window.

use crate::chat::ChatView;
use crate::commands::{ClearRecent, OpenRecent, RunCommand, WORKSPACE};
use crate::export::{ExportEvent, ExportView, Quality};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::media::MediaBand;
use crate::motion::{self, Tween};
use crate::session::{BridgeEvent, Session};
use crate::settings::{Page, SettingsEvent, SettingsView};
use crate::sidebar::{self, SidebarEvent, SidebarView};
use crate::stage::StageView;
use crate::store::{Change, Connection};
use crate::theme::{self, layout, motion as scene, text};
use crate::timeline::TimelineView;
use crate::ui::{self, icon_button};
use gpui::{
    canvas, div, img, point, prelude::*, px, size, AnimationExt, AnyElement, AnyView, App, Bounds,
    ClickEvent, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, PathPromptOptions, Pixels,
    RenderImage, Rgba, SharedString, Stateful, Subscription, Window,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// The window is too narrow for the sidebar (as last drawn): it gives
    /// way, whatever its switch says.
    sidebar_gives_way: bool,
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
    /// Activity, from the title's menu: what the window and Kerf did.
    activity_open: bool,
    /// The renders this window queued, by job: one that finishes flies home.
    queued_here: HashSet<String>,
    /// A frame of the picture on its way to the project's row (Manual export).
    flight: Option<Flight>,
    /// Where the stage was laid out last, in the window.
    stage_slot: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Each command's keys in the keymap they were read for, as the system
    /// writes them, for the toolbar's tooltips.
    keys: (String, HashMap<String, Vec<String>>),
    _subscriptions: Vec<Subscription>,
}

/// A frame of the picture flying from the stage to the project's row in
/// the sidebar once an export this window started is done.
struct Flight {
    image: Arc<RenderImage>,
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    started: Instant,
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
                    SidebarEvent::Settings => this.show_settings(None, window, cx),
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
            // File ▸ Open Recent checks this window's project while it is in front.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    let project = this.session.read(cx).project.clone();
                    crate::commands::refresh_menus(project.as_deref(), cx);
                }
            }),
            // Leaving the title's field keeps what was typed.
            cx.on_blur(&title_focus, window, |this, _, cx| {
                this.finish_title_rename(true, cx)
            }),
            // Nothing has the keys (Esc left the composer or a field, what
            // had them went away): the window has them again, so its keys
            // (the palette, Play, the mask tools) go on working.
            cx.on_focus_lost(window, |this, window, _| {
                window.focus(&this.focus_handle);
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
            sidebar_gives_way: false,
            title_menu: false,
            app_menu: false,
            title_field,
            renaming_title: None,
            error: None,
            errors: 0,
            start_media: None,
            window_title: String::new(),
            activity_open: false,
            queued_here: HashSet::new(),
            flight: None,
            stage_slot: Rc::new(Cell::new(None)),
            keys: (String::new(), HashMap::new()),
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
                // Saving over a preset of that name: the Save dialog asks
                // about it itself (the store kept the question).
                if crate::presets::existing_preset(&raw).is_some()
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
            // A render this window queued is answered here first; once it
            // is done, a frame of the picture goes home to the project.
            "export" => {
                let Some(job) = event["jobId"].as_str() else {
                    return;
                };
                match event["status"].as_str().unwrap_or_default() {
                    "queued" => {
                        self.queued_here.insert(job.to_owned());
                    }
                    "completed" | "complete" | "ready" => {
                        if self.queued_here.remove(job) && event["kind"] != "captions" {
                            self.fly_export_home(cx);
                        }
                    }
                    "failed" | "cancelled" | "canceled" => {
                        self.queued_here.remove(job);
                    }
                    _ => {}
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

    /// Manual export: a frame of the picture flies from the stage to the
    /// project's row in the sidebar, fading as it goes. Not under Reduce
    /// Motion, while Kerf works, with no picture showing, or with the
    /// sidebar away (the row is not there to land on).
    fn fly_export_home(&mut self, cx: &mut Context<Self>) {
        if motion::reduced()
            || !self.sidebar_open
            || self.sidebar_gives_way
            || !self.panel_open
            || self.picture_shown.target() < 0.5
        {
            return;
        }
        let session = self.session.read(cx);
        if session.state.work.busy() {
            return;
        }
        let Some(picture) = session.picture.as_ref() else {
            return;
        };
        // From the picture where the stage placed it (else where it would
        // be in its slot), to the project's row where the sidebar laid it.
        let Some(from) = self.stage.read(cx).picture_bounds().or_else(|| {
            self.stage_slot
                .get()
                .map(|slot| picture_within(slot, picture.width, picture.height))
        }) else {
            return;
        };
        let to = self
            .sidebar
            .read(cx)
            .project_row_bounds()
            .map(landing_in)
            .unwrap_or_else(project_row);
        self.flight = Some(Flight {
            image: picture.image.clone(),
            from,
            to,
            started: Instant::now(),
        });
        cx.notify();
    }

    /// Where the flight is this frame, and how clear: None once it has
    /// landed and faded, when what it carried leaves the atlas (unless the
    /// stage still shows it).
    fn flight_now(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(Bounds<Pixels>, f32, Arc<RenderImage>)> {
        let flight = self.flight.as_ref()?;
        match export_flight_at(flight.started.elapsed()) {
            Some((along, opacity)) => {
                window.request_animation_frame();
                Some((
                    lerp_bounds(flight.from, flight.to, along),
                    opacity,
                    flight.image.clone(),
                ))
            }
            None => {
                let flight = self.flight.take()?;
                let shown = self
                    .session
                    .read(cx)
                    .picture
                    .as_ref()
                    .is_some_and(|picture| Arc::ptr_eq(&picture.image, &flight.image));
                if !shown {
                    cx.drop_image(flight.image, Some(window));
                }
                None
            }
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
        // The whole title is chosen, to type over.
        self.title_field.update(cx, |field, cx| {
            field.set_text(title, cx);
            field.choose_all(cx);
        });
        self.title_field.read(cx).focus(window);
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
            Connection::Connected => self
                .error
                .as_ref()
                .map(|error| first_line(&error.text).to_owned()),
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

    /// Looking at something opens the picture, as by hand.
    fn reveal_picture(&mut self, cx: &mut Context<Self>) {
        if !self.panel_open {
            self.set_panel(true, cx);
        } else if self.picture_shown.target() < 0.5 {
            self.toggle_pane(true, cx);
        }
    }

    /// Takes out mask tool `tool` on the picture, or puts it down; the
    /// picture comes in for it. Not while something is open over the window.
    fn toggle_mask_tool(
        &mut self,
        tool: crate::stage::MaskTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal.is_some() {
            return;
        }
        let taken = self
            .stage
            .update(cx, |stage, cx| stage.toggle_tool(tool, window, cx));
        if taken && self.stage.read(cx).mask_tool().is_some() {
            self.reveal_picture(cx);
        }
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

    /// Opens Settings over the window, on `page` (None: where Settings
    /// opens by itself, Chat). Open already, it goes to `page`; anything
    /// else open over the window gives way to it.
    pub fn show_settings(
        &mut self,
        page: Option<Page>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self
            .modal
            .as_ref()
            .and_then(|(view, _)| view.clone().downcast::<SettingsView>().ok());
        if let Some(settings) = open {
            if let Some(page) = page {
                settings.update(cx, |settings, cx| settings.show_page(page, cx));
            }
            return;
        }
        let session = self.session.clone();
        let settings = cx.new(|cx| match page {
            Some(page) => SettingsView::new_at(session, page, window, cx),
            None => SettingsView::new(session, window, cx),
        });
        cx.subscribe_in(
            &settings,
            window,
            |this, settings, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Dismiss => this.close_modal_of(settings.entity_id(), window, cx),
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
        cx.subscribe_in(&view, window, |this, view, _: &DismissEvent, window, cx| {
            this.close_modal_of(view.entity_id(), window, cx)
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

    /// Gives the window back from `view`, if it is what is open over the
    /// window still: one another has taken the place of (Settings opened
    /// from the palette) has nothing to give back.
    fn close_modal_of(
        &mut self,
        view: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .modal
            .as_ref()
            .is_some_and(|(open, _)| open.entity_id() == view)
        {
            self.modal = None;
            window.focus(&self.focus_handle);
            cx.notify();
        }
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

    /// File ▸ Import Media…: the files chosen come into the project's media.
    /// No chooser to ask (no portal), or a name kerfd cannot be sent, says so.
    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Import".into()),
            },
        );
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                if let Some(paths) = chosen_paths(answer, session) {
                    for path in &paths {
                        session.import(path, false);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a folder in place of this project. One that is not a project
    /// yet is asked about first; this project is open already.
    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open".into()),
            },
        );
        let current = self.session.read(cx).project.clone();
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let Ok(Some(paths)) = session.update(cx, |session, cx| {
                cx.notify();
                chosen_paths(answer, session)
            }) else {
                return;
            };
            let Some(project) = paths.into_iter().next() else {
                return;
            };
            if current
                .as_deref()
                .is_some_and(|current| crate::app::same_folder(current, &project))
            {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.open_chosen(project, window, cx));
        })
        .detach();
    }

    /// Opens `project` in place of this window: one that is not a project
    /// yet is asked about first; this window's own project stays as it is.
    fn open_chosen(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if needs_start(&project) {
            self.ask_to_start(project, window, cx);
        } else {
            self.replace_with(project, false, window, cx);
        }
    }

    /// Opens `project` in a window of its own with this window's kerfd,
    /// and closes this one: another project takes its place, as in Qt.
    /// This window's own project stays open.
    fn replace_with(
        &mut self,
        project: PathBuf,
        starts: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        if session
            .project
            .as_deref()
            .is_some_and(|current| crate::app::same_folder(current, &project))
        {
            window.activate_window();
            return;
        }
        let kerfd = session.kerfd().map(Path::to_path_buf);
        open_folder_with(project, starts, kerfd, cx);
        window.remove_window();
    }

    /// New Project: the launcher, in a window of its own. What it opens
    /// takes this window's place, with this window's kerfd; closed with
    /// nothing chosen, this window stays.
    fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let kerfd = session.kerfd().map(Path::to_path_buf);
        let current = session.project.clone();
        let replaced = window.window_handle();
        let prefs = crate::prefs::Prefs::load();
        let options = crate::app::options("Industrial Kerf", cx);
        let opened = cx.open_window(options, move |launcher_window, cx| {
            // What it makes from a preset brings its media in with this kerfd too.
            let launcher = cx.new(|cx| {
                crate::launcher::LauncherView::new(prefs, launcher_window, cx)
                    .with_kerfd(kerfd.clone())
            });
            let handle = launcher_window.window_handle();
            cx.subscribe(
                &launcher,
                move |_, event: &crate::launcher::LauncherEvent, cx| match event {
                    crate::launcher::LauncherEvent::Open { project, starts } => {
                        let here = current
                            .as_deref()
                            .is_some_and(|current| crate::app::same_folder(current, project));
                        if !here {
                            open_folder_with(project.clone(), *starts, kerfd.clone(), cx);
                            let _ = replaced.update(cx, |_, window, _| window.remove_window());
                        } else {
                            let _ = replaced.update(cx, |_, window, _| window.activate_window());
                        }
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                },
            )
            .detach();
            launcher
        });
        if let Err(error) = opened {
            log::error!("could not open the launcher: {error}");
        }
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
        let kerfd = self.session.read(cx).kerfd().map(Path::to_path_buf);
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("“{name}” is not a project yet."),
                Some("Start one here, with its media?"),
                "Start",
                false,
                true,
                move |window, cx| {
                    open_folder_with(project, true, kerfd, cx);
                    window.remove_window();
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// The pinned proposal (the card the conversation pins) is what the
    /// registry's proposal commands act on and are said to be available for.
    fn pin_proposal(&mut self, cx: &mut Context<Self>) {
        let pinned = self.chat.read(cx).proposal_target("proposal.reject", cx);
        self.session
            .update(cx, |session, _| session.pinned_proposal = pinned);
    }

    /// Why command `id` cannot run now, as the registry says; None when it
    /// can. Only Ask's reason asks after the connection's key (the keychain).
    fn unavailable(&mut self, id: &str, cx: &mut Context<Self>) -> Option<String> {
        self.pin_proposal(cx);
        let session = self.session.read(cx);
        let states = if id == "ai.ask" {
            let cloud = crate::app::cloud(cx).cloned();
            session.command_states(cloud.as_ref())
        } else {
            kerf_bridge::commands_states(&session.command_context())
        };
        crate::palette::unavailable(&states, id).map(str::to_owned)
    }

    /// Carries out a command by its id: what the window does itself, else
    /// the request the registry resolves it to. One that cannot run now
    /// says why, and its key goes on to whatever else it means there.
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        self.refresh_keys(cx);
        if let Some(reason) = self.unavailable(id, cx) {
            // Esc puts things away everywhere: saying on each press that
            // there is no proposal to reject would be noise.
            if !self.bound_to_escape(id) {
                self.session.update(cx, |session, cx| {
                    session.state.notify(&reason, true);
                    cx.notify();
                });
            }
            cx.propagate();
            return;
        }
        let session = self.session.clone();
        match id {
            "app.quit" => cx.quit(),
            "app.preferences" => self.show_settings(None, window, cx),
            "app.account" => self.show_settings(Some(Page::Account), window, cx),
            "view.sidebar" => self.set_sidebar(!self.sidebar_open, cx),
            "view.picture" => self.toggle_pane(true, cx),
            "view.timeline" => self.toggle_pane(false, cx),
            "window.project" => self.set_media(!self.media_open, cx),
            "window.assistant" | "ai.ask" => self
                .chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx)),
            "file.import" => self.import(window, cx),
            "file.openProject" => self.open_project(window, cx),
            // The launcher, in a window of its own; what it opens takes this one's place.
            "file.newProject" => self.new_project(window, cx),
            // The quality chosen in Export's menu, full size or a preview,
            // where kerfd puts it (Qt's File ▸ Export…); the toolbar's
            // Export renders it into deliveries/ (`ExportChosen`).
            "file.export" => session.update(cx, |session, _| session.export_file()),
            "export.master" => session.update(cx, |session, _| {
                session.deliver(Quality::Master.wire());
            }),
            "file.refresh" => session.update(cx, |session, _| {
                session.send(serde_json::json!({"type": "refresh"}));
            }),
            "edit.undo" => session.update(cx, |session, _| session.undo()),
            "edit.redo" => session.update(cx, |session, _| session.redo()),
            "playback.toggle" => {
                // A skim ends first, so kerfd is back at the playhead before it plays.
                self.stage.update(cx, |stage, cx| stage.end_skim(cx));
                session.update(cx, |session, _| session.toggle_playback());
            }
            "playback.goToStart" => session.update(cx, |session, _| session.seek(0.0, false)),
            "playback.goToEnd" => session.update(cx, |session, _| {
                let end = session.state.program_length();
                session.seek(end, false);
            }),
            "playback.previousFrame" | "playback.nextFrame" => session.update(cx, |session, _| {
                let frames = if id == "playback.nextFrame" { 1 } else { -1 };
                let at = frame_step(session.state.position, session.timeline().rate(), frames);
                session.seek(at, false);
            }),
            // As the conversation's Stop: its work row says "Stopping…" at once.
            "ai.stop" => self.chat.update(cx, |chat, cx| chat.stop(cx)),
            // As the pinned card's own buttons: Accept takes the version it
            // shows, Reject and Bring Forward the proposal.
            "proposal.accept" | "proposal.reject" | "proposal.forward" => {
                let Some(proposal) = self.chat.read(cx).proposal_target(id, cx) else {
                    return;
                };
                session.update(cx, |session, _| match id {
                    "proposal.accept" => session.accept(&proposal),
                    "proposal.reject" => session.reject(&proposal),
                    _ => session.forward(&proposal),
                })
            }
            "clip.startPath" => {
                let started = self.stage.update(cx, |stage, cx| stage.start_path(cx));
                if !started {
                    session.update(cx, |session, _| {
                        session
                            .state
                            .notify("Choose a picture clip with no motion path", true)
                    });
                }
            }
            // Editing presets: Import asks for the file, then its dialog
            // comes over the window (through this entity: the window is
            // being updated now); Export asks where; Save asks the name.
            "file.applyPreset" => {
                let workspace = cx.entity().downgrade();
                crate::presets::import_preset(session, window, cx, move |view, window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.open_modal(view, window, cx));
                });
            }
            "file.exportPreset" => crate::presets::export_preset(session, window, cx),
            "preset.save" => {
                let view = crate::presets::save_preset(session, window, cx);
                self.open_modal(view, window, cx);
            }
            // The zoom's menu over the transport (⇧Z, View ▸ Viewer Zoom…).
            "viewer.zoom" => {
                self.reveal_picture(cx);
                self.stage
                    .update(cx, |stage, cx| stage.toggle_zoom_menu(window, cx));
            }
            "help.guide" => cx.open_url(&user_guide()),
            "help.about" => self.show_settings(Some(Page::About), window, cx),
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
                session.run_command(id, serde_json::json!({}));
            }),
        }
        cx.notify();
    }

    /// The toolbar's Export (its main part): renders the quality chosen in
    /// its menu into deliveries/. While Kerf cannot export it says why, as
    /// Export Master would.
    fn export_chosen(&mut self, cx: &mut Context<Self>) {
        if let Some(reason) = self.unavailable("export.master", cx) {
            self.session.update(cx, |session, cx| {
                session.state.notify(&reason, true);
                cx.notify();
            });
            return;
        }
        self.session.update(cx, |session, cx| {
            let quality = session.export_quality;
            session.export(quality.wire());
            cx.notify();
        });
    }

    /// Whether `id` answers plain Esc in the keymap in use.
    fn bound_to_escape(&self, id: &str) -> bool {
        self.keys.1.get(id).is_some_and(|keys| {
            keys.iter()
                .any(|key| crate::commands::keystroke(key).as_deref() == Some("escape"))
        })
    }

    /// Reads each command's keys again when the keymap has changed.
    fn refresh_keys(&mut self, cx: &App) {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 == keymap {
            return;
        }
        let keys = crate::commands::catalog(keymap)
            .into_iter()
            .map(|command| (command.id, command.keys))
            .collect();
        self.keys = (keymap.to_owned(), keys);
    }

    /// Command `id`'s first key as the system writes it ("⌘E", "Ctrl+E").
    fn shown_key(&self, id: &str) -> Option<String> {
        self.keys
            .1
            .get(id)
            .and_then(|keys| keys.first())
            .map(|key| crate::commands::shown_keys(key))
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

    /// Puts away whichever menu is open: the title's, Activity, the
    /// toolbar's app menu, the sidebar's. Whether one was.
    fn close_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let mine = self.title_menu || self.app_menu || self.activity_open;
        if mine {
            self.title_menu = false;
            self.app_menu = false;
            self.activity_open = false;
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
        let rendering = session.state.rendering();
        let quality = session.export_quality;
        // What can be done now, as the registry says it (none of these is
        // Ask, so the keychain is not asked).
        let states = kerf_bridge::commands_states(&session.command_context());
        let reason = |id: &str| crate::palette::unavailable(&states, id).map(str::to_owned);
        let (undo_reason, redo_reason, export_reason) = (
            reason("edit.undo"),
            reason("edit.redo"),
            reason("export.master"),
        );
        let tip = |words: &str, id: &str, why: Option<&String>| -> (String, Option<String>) {
            match why {
                Some(why) => (format!("{words} · {why}"), None),
                None => (words.to_owned(), self.shown_key(id)),
            }
        };
        let undo_tip = tip("Undo", "edit.undo", undo_reason.as_ref());
        let redo_tip = tip("Redo", "edit.redo", redo_reason.as_ref());
        // The master's key goes with it; a draft has none of its own.
        let export_tip = (
            crate::export::export_tip(quality, export_reason.as_deref()),
            (quality == Quality::Master && export_reason.is_none())
                .then(|| self.shown_key("export.master"))
                .flatten(),
        );
        let sidebar_key = self.shown_key("view.sidebar");
        let follow_key = self.shown_key("view.followKerf");
        let media_key = self.shown_key("window.project");
        let export_open = self.export.is_some();
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
                bar.child(key_tip(
                    "show-sidebar-tip",
                    ("Sidebar".into(), sidebar_key),
                    icon_button("show-sidebar", Icon::Sidebar)
                        .on_click(cx.listener(|this, _, _, cx| this.set_sidebar(true, cx))),
                ))
                // With the sidebar away, its account's menu opens from here.
                .child(
                    sidebar::trigger(
                        "toolbar-app-menu",
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.app_menu = !this.app_menu;
                            this.title_menu = false;
                            this.activity_open = false;
                            this.sidebar
                                .update(cx, |sidebar, cx| sidebar.close_menus(cx));
                            cx.notify();
                        }),
                    )
                    .child(
                        // Its menu says all the tooltip would, so the tooltip waits while it is open.
                        icon_button("toolbar-app-menu-button", Icon::More)
                            .when(!self.app_menu, |button| {
                                button.tooltip("Settings, account and more")
                            })
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
                            .child(self.title_field.clone())
                            .into_any_element()
                    } else {
                        // Pressed, what can be done with the conversation;
                        // pressed twice, its name becomes a field.
                        sidebar::trigger(
                            "toolbar-title",
                            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                this.app_menu = false;
                                this.activity_open = false;
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
            .child(key_tip(
                "undo-tip",
                undo_tip,
                icon_button("undo", Icon::Undo)
                    .disabled(undo_reason.is_some())
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(RunCommand::new("edit.undo")), cx)
                    }),
            ))
            .child(key_tip(
                "redo-tip",
                redo_tip,
                icon_button("redo", Icon::Redo)
                    .disabled(redo_reason.is_some())
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(RunCommand::new("edit.redo")), cx)
                    }),
            ))
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
            .child(self.export_button(
                quality,
                export_tip,
                export_reason.is_none(),
                export_open,
                cx,
            ))
            .child(key_tip(
                "follow-kerf-tip",
                ("Follow Kerf".into(), follow_key),
                // While Kerf works, the playhead goes where it looks.
                icon_button("follow-kerf", Icon::Eye)
                    .on(follow)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(RunCommand::new("view.followKerf")), cx)
                    }),
            ))
            .child(key_tip(
                "media-tip",
                ("Media".into(), media_key),
                icon_button("media", Icon::Media)
                    .on(self.media_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_media(!this.media_open, cx))),
            ))
            .child(
                icon_button("panel", Icon::Panel)
                    .tooltip("Picture and timeline")
                    .on(self.panel_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_panel(!this.panel_open, cx))),
            )
    }

    /// Export, a split button: its main part renders the quality chosen
    /// in one press ("Export Master"); its chevron opens the menu that
    /// chooses it, with the queue. Off, with the reason in its tooltip,
    /// while it cannot render.
    fn export_button(
        &self,
        quality: Quality,
        tip: (String, Option<String>),
        able: bool,
        open: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ink = if able { theme::ink() } else { theme::dis() };
        let main = div()
            .id("export-main")
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(EXPORT_INNER))
            .px(px(10.0))
            .child(Icon::Export.view(14.0, ink))
            .child(ui::line("Export", text::CONTROL_STRONG, ink))
            .child(ui::line(
                quality.name(),
                text::CONTROL,
                if able { theme::ink3() } else { theme::dis() },
            ))
            .when(able, |main| {
                main.cursor_pointer()
                    .hover(|main| main.bg(theme::hover2()))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::export::ExportChosen), cx)
                    })
            });
        let chevron = div()
            .id("export-options")
            .flex()
            .items_center()
            .justify_center()
            .w(px(EXPORT_CHEVRON))
            .h(px(EXPORT_INNER))
            .border_l_1()
            .border_color(theme::line2())
            .cursor_pointer()
            .when(open, |chevron| chevron.bg(theme::on()))
            .hover(|chevron| chevron.bg(theme::hover2()))
            .child(Icon::ChevronDown.view(14.0, theme::ink2()))
            .tooltip(|_, cx| cx.new(|_| ui::Tooltip("Export options".into())).into())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_export(window, cx)));
        div()
            .flex()
            .flex_none()
            .h(px(theme::height::H28))
            .rounded(px(theme::radius::CONTROL))
            .overflow_hidden()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::line2())
            .child(key_tip("export-tip", tip, main))
            .child(chevron)
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
        // The models the connection offers to choose between (Claude
        // Code's), and the one in use.
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let model = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured.clone()
        };
        let choices = crate::chat::model_choices(provider, &configured);
        let choosable = choices.len() > 1 && provider == crate::prefs::Provider::ClaudeCode;
        // A request carries the playhead's time while the picture is open.
        let stamp_offered = self.panel_open;
        let stamped = session.prefs.send_playhead;
        sidebar::menu_panel()
            .w(px(TITLE_MENU_WIDTH))
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
                // The model in use, where a menu writes a key.
                .child(div().flex_1())
                .child(ui::line(
                    crate::chat::model_label(&model),
                    text::META,
                    theme::ink3(),
                ))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.title_menu = false;
                    this.show_settings(Some(Page::Chat), window, cx);
                })),
            )
            .when(choosable, |menu| {
                menu.child(div().px(px(10.0)).pt(px(6.0)).pb(px(2.0)).child(ui::line(
                    "Model",
                    text::META,
                    theme::ink3(),
                )))
                .children(choices.into_iter().map(|choice| {
                    let chosen = choice == model;
                    sidebar::menu_item(
                        ElementId::Name(format!("title-model-{choice}").into()),
                        crate::chat::model_label(&choice),
                        chosen.then_some(Icon::Check),
                        false,
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.title_menu = false;
                        this.session.update(cx, |session, cx| {
                            session.prefs.claude_code_model = choice.clone();
                            session.save_prefs();
                            cx.notify();
                        });
                        cx.notify();
                    }))
                }))
            })
            .when(stamp_offered, |menu| {
                menu.child(
                    sidebar::menu_item(
                        "title-stamp",
                        "Send playhead time",
                        stamped.then_some(Icon::Check),
                        false,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.title_menu = false;
                        this.session.update(cx, |session, cx| {
                            session.prefs.send_playhead = !session.prefs.send_playhead;
                            session.save_prefs();
                            cx.notify();
                        });
                        cx.notify();
                    })),
                )
            })
            .child(
                sidebar::menu_item("title-activity", "Activity", None, false, false).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.title_menu = false;
                        this.activity_open = true;
                        cx.notify();
                    }),
                ),
            )
            .with_animation("title-menu", motion::animation(scene::FADE), |menu, t| {
                menu.opacity(t)
            })
    }

    /// Activity, under the title: the newest of what the window and Kerf
    /// did, a line each. A press outside or Esc puts it away.
    fn activity_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines: Vec<(String, String)> = self
            .session
            .read(cx)
            .activity
            .latest(crate::session::ACTIVITY_SHOWN)
            .map(|(kind, text)| (kind.to_uppercase(), text.to_owned()))
            .collect();
        sidebar::menu_panel()
            .w(px(ACTIVITY_WIDTH))
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.activity_open = false;
                cx.notify();
            }))
            .child(ui::line("Activity", text::META_STRONG, theme::ink3()))
            .when(lines.is_empty(), |panel| {
                panel.child(ui::label(
                    "Nothing yet: imports, exports and previews are listed here.",
                    text::CONTROL,
                    theme::ink3(),
                ))
            })
            .children(lines.into_iter().map(|(kind, words)| {
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        ui::line(kind, text::MONO_SMALL, theme::ink3())
                            .w(px(ACTIVITY_KIND))
                            .flex_none(),
                    )
                    .child(
                        ui::label(words, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
            }))
            .with_animation("activity", motion::animation(scene::FADE), |panel, t| {
                panel.opacity(t)
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
                None,
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
                None,
            ));
        }
        if let Some(error) = &self.error {
            // Its first line; the whole of a longer one (an import's ffmpeg
            // arguments and stderr) in its tooltip.
            let line = first_line(&error.text);
            let whole = (line != error.text.trim()).then(|| error.text.clone().into());
            bars.push(bar(
                format!("bar-error-{}", error.key).into(),
                theme::neg(),
                line.to_owned().into(),
                None,
                vec![ui::button("bar-dismiss", "Dismiss")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element()],
                whole,
            ));
        }
        bars
    }

    fn panel(
        &self,
        width: f32,
        height: f32,
        inspecting: bool,
        aspect: f32,
        window: &Window,
    ) -> AnyElement {
        let picture = self.picture_shown.sample(window);
        let timeline = self.timeline_shown.sample(window);
        let media = self.media_shown.sample(window);
        let band = layout::MEDIA_BAND * media;
        // The timeline's share follows the picture's shape; the picture
        // takes the rest. One of the two alone has all of it.
        let timeline_full = (height - band).max(0.0);
        let stage_width = width - if inspecting { layout::INSPECTOR } else { 0.0 };
        let timeline_tall = timeline_split(height, band, stage_width, aspect);
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(self.stage.clone())
                            // Where the stage is, for a frame to fly from.
                            .child({
                                let slot = self.stage_slot.clone();
                                canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
                                    .absolute()
                                    .inset_0()
                            }),
                    )
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
                    .max_w(px(layout::NOTICE_MOST))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(theme::radius::CARD))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .text_color(theme::ink())
                    .child(div().flex_none().child(ui::dot(color, 6.0)))
                    // Long words (a failed push's ssh line) wrap inside it.
                    .child(div().flex_1().min_w_0().child(notice.text.clone()))
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
        self.refresh_keys(cx);
        let flight = self.flight_now(window, cx);
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width), f32::from(viewport.height));
        let sidebar_width = self.sidebar_width(self.session.read(cx));
        // In a window too narrow for it, the sidebar gives way (Qt's
        // sidebarWindowLeast), and the toolbar has its menu.
        self.sidebar_gives_way = window_width < layout::SIDEBAR_WINDOW_LEAST;
        let sidebar = if self.sidebar_gives_way {
            0.0
        } else {
            self.sidebar_shown.sample(window)
        };
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
        // The picture's shape, which splits the panel.
        let aspect = crate::stage::shown_aspect(self.session.read(cx));
        // Where the title starts, for its menu to open under it.
        let title_left = main_left + if leading { 84.0 } else { 16.0 };
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
            .on_action(
                cx.listener(|this, _: &crate::export::ExportChosen, _, cx| this.export_chosen(cx)),
            )
            .on_action(cx.listener(|this, _: &crate::commands::Escape, _, cx| this.escape(cx)))
            .on_action(
                cx.listener(|this, _: &crate::commands::RevealPicture, _, cx| {
                    this.reveal_picture(cx)
                }),
            )
            // ⇧R ⇧E ⇧P wherever the window has the keys, as Qt's View menu
            // had them: a mask tool on the chosen picture clip.
            .on_action(
                cx.listener(|this, _: &crate::stage::RectangleMask, window, cx| {
                    this.toggle_mask_tool(crate::stage::MaskTool::Rectangle, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::stage::EllipseMask, window, cx| {
                    this.toggle_mask_tool(crate::stage::MaskTool::Ellipse, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &crate::stage::PenMask, window, cx| {
                this.toggle_mask_tool(crate::stage::MaskTool::Pen, window, cx)
            }))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.open_chosen(PathBuf::from(action.path.as_ref()), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearRecent, _, cx| {
                this.session.update(cx, |session, _| {
                    session.prefs.recent.clear();
                    session.save_prefs();
                });
                let project = this.session.read(cx).project.clone();
                crate::commands::refresh_menus(project.as_deref(), cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::palette::OpenPalette, window, cx| {
                    if this.modal.is_none() {
                        // What the palette says of a proposal is of the pinned one.
                        this.pin_proposal(cx);
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
                    // Its edge (drag, press, double-click) is the sidebar's own.
                    .child(self.sidebar.clone()),
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
                                            aspect,
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
                        .right(px(EXPORT_RIGHT))
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
            .when(self.activity_open, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(title_left))
                        .child(self.activity_view(cx)),
                )
            })
            .when(self.app_menu && leading, |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(layout::TOOLBAR - 2.0))
                        .left(px(main_left + 40.0))
                        .child(
                            sidebar::account_menu(&self.session, close_app_menu.clone(), cx)
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
            // Manual export: a frame of the picture on its way to the project's row.
            .when_some(flight, |root, (bounds, opacity, image)| {
                root.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .opacity(opacity)
                        // Covering its box, and cut to it.
                        .overflow_hidden()
                        .rounded(px(theme::radius::CARD))
                        .child(
                            img(image)
                                .size_full()
                                .rounded(px(theme::radius::CARD))
                                .object_fit(ObjectFit::Cover),
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
/// Export's chevron, and the height inside the button's outline.
const EXPORT_CHEVRON: f32 = 24.0;
const EXPORT_INNER: f32 = theme::height::H28 - 2.0;
/// How far Export's right edge is from the window's: the toolbar's end,
/// past Follow Kerf, Media and the panel's button. Its menu opens there.
const EXPORT_RIGHT: f32 = 8.0 + 3.0 * (theme::height::H28 + 4.0);
/// The title's menu: room for Agent settings… and the model after it.
const TITLE_MENU_WIDTH: f32 = 260.0;
/// Activity's width, and its column of kinds.
const ACTIVITY_WIDTH: f32 = 380.0;
const ACTIVITY_KIND: f32 = 64.0;
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
    tip: Option<SharedString>,
) -> AnyElement {
    div()
        .id(id.clone())
        .when_some(tip, |bar, tip| {
            bar.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone())).into())
        })
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

/// The paths a file chooser gave; None, with a notice in `session`'s window
/// saying why where there is something to say (no chooser could open, a
/// name kerfd cannot be sent), when there are none to use.
fn chosen_paths(answer: crate::presets::Picked, session: &mut Session) -> Option<Vec<PathBuf>> {
    use crate::presets::Picked;
    match answer {
        Picked::Paths(paths) => Some(paths),
        Picked::Failed(error) => {
            session
                .state
                .notify(&format!("No file chooser could open: {error}"), true);
            None
        }
        Picked::Unusable(words) => {
            session.state.notify(&words, true);
            None
        }
        Picked::Nothing => None,
    }
}

/// The first line of what `text` says, trimmed (the whole of it when it
/// has one): what the error bar and the toolbar's line show of an error.
pub fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// One frame on from `position`, or `frames` back, on the frame grid at
/// `rate` frames a second: the playhead rounded to its frame first (after a
/// skim or the clock it can sit between frames), never before the start.
pub fn frame_step(position: f64, rate: f64, frames: i64) -> f64 {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        30.0
    };
    (((position * rate).round() + frames as f64) / rate).max(0.0)
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

/// Opens `project` in a window of its own. `starts` when it was no project
/// yet: once kerfd has it, its folder's media comes in (the launcher's
/// `LauncherEvent::Open { starts }` means the same).
pub fn open_folder(project: PathBuf, starts: bool, cx: &mut App) {
    open_folder_with(project, starts, None, cx);
}

/// [`open_folder`] with the kerfd the project is opened with (None finds
/// one as at launch): a window opening another project keeps its own.
pub fn open_folder_with(project: PathBuf, starts: bool, kerfd: Option<PathBuf>, cx: &mut App) {
    crate::app::open(Some(project.clone()), kerfd, cx);
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
                .is_some_and(|open| crate::app::same_folder(open, &project));
            if fresh && here && workspace.start_media.is_none() {
                workspace.start_media = Some(project.clone());
            }
        });
    }
}

/// `child` with a tooltip of words and, dimmer after them, its keys.
fn key_tip(
    id: &'static str,
    (words, keys): (String, Option<String>),
    child: impl IntoElement,
) -> Stateful<gpui::Div> {
    let words: SharedString = words.into();
    let keys: Option<SharedString> = keys.map(Into::into);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .tooltip(move |_, cx| {
            cx.new(|_| KeyTip {
                words: words.clone(),
                keys: keys.clone(),
            })
            .into()
        })
        .child(child)
}

/// What a toolbar button says when the pointer rests on it: what it does
/// (or why it cannot now), and its keys.
struct KeyTip {
    words: SharedString,
    keys: Option<SharedString>,
}

impl Render for KeyTip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        theme::type_style(div(), text::CONTROL)
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .text_color(theme::ink())
            .shadow_md()
            .child(self.words.clone())
            .when_some(self.keys.clone(), |tip, keys| {
                tip.child(div().text_color(theme::ink3()).child(keys))
            })
    }
}

/// Where a manual export's frame is at `elapsed`: how far along (eased,
/// 0..1) and how clear. It crosses over [`scene::FLIGHT`] fading from 0.9
/// to 0.2, then fades out where it landed over [`scene::FLIGHT_LAND`].
/// None once it has gone.
pub fn export_flight_at(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= scene::FLIGHT + scene::FLIGHT_LAND {
        return None;
    }
    let flight = scene::FLIGHT.as_secs_f32();
    let land = scene::FLIGHT_LAND.as_secs_f32();
    let elapsed = elapsed.as_secs_f32();
    let t = (elapsed / flight).clamp(0.0, 1.0);
    let opacity = FLIGHT_FROM + (FLIGHT_TO - FLIGHT_FROM) * t;
    let fading = ((elapsed - flight) / land).clamp(0.0, 1.0);
    Some((motion::ease(t), opacity * (1.0 - fading)))
}

/// How clear a manual export's frame is as it leaves the picture and as it lands.
const FLIGHT_FROM: f32 = 0.9;
const FLIGHT_TO: f32 = 0.2;

/// The rect `along` (0..1) of the way from `from` to `to`.
pub fn lerp_bounds(from: Bounds<Pixels>, to: Bounds<Pixels>, along: f32) -> Bounds<Pixels> {
    let mix = |a: Pixels, b: Pixels| a + (b - a) * along;
    Bounds::new(
        point(
            mix(from.origin.x, to.origin.x),
            mix(from.origin.y, to.origin.y),
        ),
        size(
            mix(from.size.width, to.size.width),
            mix(from.size.height, to.size.height),
        ),
    )
}

/// Where the stage puts a `width` × `height` picture within `slot` (the
/// stage, its transport under it): in its room, inset as the stage insets
/// its device, at its own aspect, centred.
pub fn picture_within(slot: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    let area_height = (f32::from(slot.size.height) - layout::TRANSPORT).max(1.0);
    let inset = STAGE_MARGIN + STAGE_BEZEL;
    let room_width = (f32::from(slot.size.width) - inset * 2.0).max(1.0);
    let room_height = (area_height - inset * 2.0).max(1.0);
    let aspect = if width == 0 || height == 0 {
        16.0 / 9.0
    } else {
        width as f32 / height as f32
    };
    let (shown_width, shown_height) = if room_width / room_height > aspect {
        (room_height * aspect, room_height)
    } else {
        (room_width, room_width / aspect)
    };
    Bounds::new(
        point(
            slot.origin.x + px((f32::from(slot.size.width) - shown_width) / 2.0),
            slot.origin.y + px((area_height - shown_height) / 2.0),
        ),
        size(px(shown_width), px(shown_height)),
    )
}

/// How tall the timeline is under the picture in a panel `height` tall,
/// with the Media band `band` tall between them and the stage `width` wide,
/// split as Qt split it by the picture's shape (`aspect`, width over
/// height): under a wide picture [`layout::TIMELINE_WIDE_SHARE`] of the
/// height, at least [`layout::TIMELINE_WIDE_LEAST`], and what the picture
/// cannot use at that width; under a vertical or square one
/// [`layout::TIMELINE_UPRIGHT_SHARE`], at least
/// [`layout::TIMELINE_UPRIGHT_LEAST`]. The band takes the timeline's top
/// until that would leave it under [`layout::MEDIA_BAND_TIMELINE_LEAST`];
/// then the picture gives way. The transport, which Qt's timeline carried,
/// is under the picture here, and counts as the timeline's.
pub fn timeline_split(height: f32, band: f32, width: f32, aspect: f32) -> f32 {
    let height = height.max(0.0);
    let band = band.clamp(0.0, height);
    let wide = aspect >= layout::WIDE;
    let (share, least) = if wide {
        (layout::TIMELINE_WIDE_SHARE, layout::TIMELINE_WIDE_LEAST)
    } else {
        (
            layout::TIMELINE_UPRIGHT_SHARE,
            layout::TIMELINE_UPRIGHT_LEAST,
        )
    };
    // Qt's slot: the picture with its air, under it the timeline and its transport.
    let mut slot = height - (height * share).max(least).min(height);
    if wide {
        let air = (STAGE_MARGIN + STAGE_BEZEL) * 2.0;
        slot = slot.min((width - air).max(0.0) / aspect + air);
    }
    if band > 0.0 {
        slot = slot.min(height - band - layout::MEDIA_BAND_TIMELINE_LEAST);
    }
    let picture = (slot.max(0.0) + layout::TRANSPORT).min(height - band);
    (height - band - picture).max(0.0)
}

/// The stage's air round its device, and the device's bezel.
const STAGE_MARGIN: f32 = 24.0;
const STAGE_BEZEL: f32 = 8.0;

/// Where a flight lands on `row` (the project's row in the sidebar), as
/// Qt landed it: a small frame at the row's start, inside its height.
pub fn landing_in(row: Bounds<Pixels>) -> Bounds<Pixels> {
    let (left, top) = scene::LANDING_INSET;
    let height = (f32::from(row.size.height) - top * 2.0).max(1.0);
    let width = scene::LANDING_WIDTH.min(f32::from(row.size.width)).max(1.0);
    Bounds::new(
        point(row.origin.x + px(left), row.origin.y + px(top)),
        size(px(width), px(height)),
    )
}

/// Where a manual export's frame lands when the sidebar has not laid the
/// project's row out yet: at its folder, under the toolbar, New
/// conversation and Search.
fn project_row() -> Bounds<Pixels> {
    let top = layout::TOOLBAR + theme::height::H32 * 2.0 + 2.0 + 8.0 + 4.0;
    Bounds::new(point(px(12.0), px(top)), size(px(44.0), px(26.0)))
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
