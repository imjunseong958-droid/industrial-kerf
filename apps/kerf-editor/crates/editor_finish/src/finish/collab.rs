//! The toolbar's chip for the project's Git repository: the branch and how
//! far it is from the remote, with Sync and the rest of sharing in its menu,
//! and the questions its rows ask in the menu's place. The window's menus
//! and the palette reach the same questions ([`Collab::open_command`]).

mod chip;
mod menu;
mod question;
mod said;

pub use menu::{other_branches, NO_OTHER_BRANCHES};
pub use question::{
    publish_command, publish_name, pull_request_command, pull_request_title, PublishWay, NEEDS_GH,
};
pub use said::{pull_request_said, sync_lines, sync_said, SyncFailure};

use super::bind_close;
use super::finishing::finishing;
use editor_session::session::{BridgeEvent, Session};
use editor_state::store::Change;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::theme::{self, motion as scene};
use editor_ui::ui;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, App, Context, Corner, Edges, Entity,
    EventEmitter, FocusHandle, Subscription, Window,
};
use kerf_bridge::event::{CollabStatus, CollabSync};
use kerf_bridge::{Command, Event};
use question::{Prompt, BODY_MOST};
use said::summary;
use std::time::Instant;

/// The chip's menu, and a question asked in its place that holds no more
/// than a line.
const MENU_WIDTH: f32 = 284.0;

/// What stands under the chip: nothing, its menu (opened on Switch
/// Branch's branches), or a question in the menu's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Closed,
    Open,
    /// Open, with the branches to switch to unfolded.
    Branches,
    Prompt(Prompt),
}

/// What the Git chip tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollabEvent {
    /// A sync failed: why, and whether it is to be said (the window's
    /// error bar, "Sync failed: <why>"), by [`SyncFailure::failed`].
    SyncFailed { why: String, said: bool },
    /// A sync started or succeeded: the failure stands no longer.
    SyncCleared,
}

/// The project's standing against its Git repository, in the toolbar, with
/// Sync and the rest of sharing in its menu. The window's menus and the
/// palette reach its questions too ([`Collab::open_command`]).
pub struct Collab {
    session: Entity<Session>,
    /// Where the project stands against its repository (`collab {status}`).
    status: Option<CollabStatus>,
    /// The step a sync is at (committing, fetching…) while it runs.
    stage: Option<CollabSync>,
    /// Why the last sync failed, until one succeeds, and whether it stands.
    failure: SyncFailure,
    /// What the last sync did.
    report: Option<String>,
    menu: Menu,
    /// The kit's menu of its rows, while it shows them (not a question).
    kit: Option<Entity<ui::Menu>>,
    /// How many times the menu has opened, so each opening fades in.
    menus_opened: u64,
    /// Itself, for what its menu's rows do.
    this: gpui::WeakEntity<Self>,
    /// A question's line: the message, the name, the title.
    input: Entity<TextInput>,
    /// Publish's remote.
    remote: Entity<TextInput>,
    /// A pull request's description.
    body: Entity<TextInput>,
    publish_way: PublishWay,
    publish_private: bool,
    focus_handle: FocusHandle,
    /// The chip's own Tab stop (between the format bank and Undo).
    chip_focus: FocusHandle,
    /// Enter or Space went down on the chip: as it comes up it opens the
    /// menu. An Enter that chose in the menu, coming up once the keys are
    /// back on the chip, does not.
    chip_armed: bool,
    /// What had the keys before the menu took them, to give them back.
    previous_focus: Option<FocusHandle>,
    /// Whether the question open was asked from a row of the menu, in its
    /// place: Esc and Cancel go back to the menu. One asked from the
    /// window's menus or the palette has no menu to go back to, and just
    /// closes.
    asked_over_menu: bool,
    /// When the work under way began, for the spinner.
    busy_since: Instant,
    /// The command a person asked for (its bridge type), so its end is said.
    asked: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Collab {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_close(cx);
        let input = cx.new(|cx| TextInput::new(cx, ""));
        let remote = cx.new(|cx| TextInput::new(cx, "git@github.com:you/film.git"));
        let body = cx.new(|cx| {
            TextInput::new(cx, "Leave empty to list this branch's commits")
                .multiline()
                .max_height(px(BODY_MOST))
        });
        let mut subscriptions = vec![
            cx.subscribe_in(
                &session,
                window,
                |this, _, event: &BridgeEvent, window, cx| this.apply(&event.0, window, cx),
            ),
            cx.subscribe_in(&session, window, |this, _, change: &Change, window, cx| {
                if matches!(change, Change::Snapshot | Change::Connection) {
                    // What the menu's rows can do follows the window's commands.
                    this.refresh_menu(cx);
                    this.offer_branches(window, cx);
                    cx.notify();
                }
            }),
            // Come to the front, the menu bar offers this window's branches.
            cx.observe_window_activation(window, |this, window, cx| {
                this.offer_branches(window, cx)
            }),
        ];
        for field in [&input, &remote, &body] {
            subscriptions.push(cx.subscribe_in(
                field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.submit(window, cx),
                    InputEvent::Cancel => this.back(window, cx),
                    InputEvent::Changed => cx.notify(),
                },
            ));
        }
        // What Finish keeps listens from the start, so a check started with
        // Finish closed is not missed.
        finishing(&session, cx);
        session.update(cx, |session, _| session.send(Command::CollabStatus));
        Self {
            session,
            status: None,
            stage: None,
            failure: SyncFailure::default(),
            report: None,
            menu: Menu::Closed,
            kit: None,
            menus_opened: 0,
            this: cx.weak_entity(),
            input,
            remote,
            body,
            publish_way: PublishWay::GitHub,
            publish_private: true,
            focus_handle: cx.focus_handle(),
            chip_focus: cx.focus_handle().tab_stop(true),
            chip_armed: false,
            previous_focus: None,
            asked_over_menu: false,
            busy_since: Instant::now(),
            asked: None,
            _subscriptions: subscriptions,
        }
    }

    /// Carries out the registry's collaboration command `id` from the
    /// window's menus or the palette, as the chip's menu does: Commit…, New
    /// Branch…, Open Pull Request… and Publish to GitHub… open the menu on
    /// their question; Switch Branch opens it on the branches. False for a
    /// command it does not ask for, or while the project's Git standing is
    /// not known yet. The workspace calls it (its `run_command`).
    pub fn open_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.status.is_none() || !self.session.read(cx).has_project() {
            return false;
        }
        let prompt = Prompt::of_command(id);
        if prompt.is_none() && id != "collab.switchBranch" {
            return false;
        }
        // What asked (the palette) gives the keys back as it goes, after
        // this: the menu or its question opens once it has, and takes them.
        cx.defer_in(window, move |this, window, cx| {
            if this.menu == Menu::Closed {
                this.menus_opened += 1;
                this.previous_focus = window.focused(cx);
                this.session
                    .update(cx, |session, _| session.send(Command::CollabStatus));
            }
            // The menu open gives the question, or itself opened anew, its place.
            if let Some(kit) = this.kit.take() {
                kit.update(cx, |menu, cx| menu.close(window, cx));
            }
            match prompt {
                Some(prompt) => this.ask(prompt, false, window, cx),
                None => this.show_menu(true, window, cx),
            }
        });
        true
    }

    /// Sync Now from the window's menus or the palette: as the chip's, its
    /// end is said. The workspace calls it for `collab.sync`.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.send(Command::CollabSync { message: None }, cx);
        cx.notify();
    }

    /// Why the last sync failed, until one succeeds (the app's menu's
    /// View ▸ Sync says it, as the chip's own menu does).
    pub fn failure(&self) -> Option<&str> {
        self.failure.last()
    }

    /// Why the last sync failed, while no sync has started since: the
    /// toolbar's red line says it.
    pub fn standing_failure(&self) -> Option<&str> {
        self.failure.standing()
    }

    /// Whether a merge is unfinished, to be finished with git.
    pub fn conflicted(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.git.conflicted)
    }

    /// Whether one of its questions (Commit…, New Branch…, Pull Request…,
    /// Publish to GitHub…) shows, its keys in it or not: it counts as a
    /// dialog, so Esc closes it, by its own Close, and not what stands
    /// behind it (`Workspace`'s Esc intercept).
    pub fn asking(&self, cx: &App) -> bool {
        matches!(self.menu, Menu::Prompt(_))
            && self.status.is_some()
            && self.session.read(cx).has_project()
    }

    /// Live Sync from the window's menus or the palette: turned the other
    /// way and remembered for the project. The workspace calls it for
    /// `collab.live`.
    pub fn toggle_live(&mut self, cx: &mut Context<Self>) {
        let live = self.status.as_ref().is_some_and(|status| status.live);
        self.set_live(!live, cx);
    }

    fn apply(&mut self, event: &Event, window: &Window, cx: &mut Context<Self>) {
        match event {
            Event::Collab { status } => {
                let was_busy = self.busy();
                if !status.busy {
                    self.stage = None;
                }
                self.status = Some(status.clone());
                if self.busy() && !was_busy {
                    self.busy_since = Instant::now();
                }
            }
            Event::CollabSync(sync) => match sync {
                CollabSync::Done { report } => {
                    self.stage = None;
                    if self.failure.last().is_some() {
                        cx.emit(CollabEvent::SyncCleared);
                    }
                    self.failure.done();
                    let lines = sync_lines(report);
                    if !lines.is_empty() {
                        self.session.update(cx, |session, _| {
                            for line in &lines {
                                session.activity.push("sync", line);
                            }
                        });
                    }
                    // A sync is said by what it brought and sent; what else
                    // a person asked for (a commit, a branch) as it went.
                    let said = match self.asked.take().as_deref() {
                        Some("collab_sync") => sync_said(report, true),
                        Some(_) => Some(summary(report)),
                        None => sync_said(report, false),
                    };
                    // Held longer than most notices, there being more to read.
                    if let Some(said) = said {
                        self.notify(&said, false, Some(scene::NOTICE_LONG_HOLD), cx);
                    }
                    self.report = Some(summary(report));
                }
                CollabSync::Failed { message } => {
                    self.stage = None;
                    let message = message.clone();
                    // Live sync says a failure once; a person asking hears it
                    // again. It is said in the window's error bar, which
                    // stays until it is dismissed.
                    let asked = self.asked.take().is_some();
                    let said = self.failure.failed(&message, asked);
                    if said {
                        self.session
                            .update(cx, |session, _| session.activity.push("sync", &message));
                    }
                    cx.emit(CollabEvent::SyncFailed { why: message, said });
                }
                stage => {
                    if self.stage.is_none() {
                        self.busy_since = Instant::now();
                    }
                    self.stage = Some(stage.clone());
                    if self.failure.standing().is_some() {
                        cx.emit(CollabEvent::SyncCleared);
                    }
                    self.failure.started();
                }
            },
            Event::CollabPullRequest { url, created } => {
                self.asked = None;
                let url = url.trim().to_owned();
                let created = *created;
                // Held longer than most notices, for its link to be read.
                self.notify(
                    &pull_request_said(&url, created),
                    false,
                    Some(scene::NOTICE_LONG_HOLD),
                    cx,
                );
                if !url.is_empty() {
                    self.session
                        .update(cx, |session, _| session.activity.push("sync", &url));
                    cx.open_url(&url);
                }
            }
            _ => return,
        }
        // The menu open says what git says now, and so does the menu bar.
        self.refresh_menu(cx);
        self.offer_branches(window, cx);
        cx.notify();
    }

    /// The menu bar (macOS) offers the window in front's branches
    /// (Collaborate ▸ Switch Branch), held back while a sync would be: set
    /// again from this window, while it is in front, as git answers, a sync
    /// begins or ends, or the window's commands change, and as it comes to
    /// the front.
    fn offer_branches(&self, window: &Window, cx: &mut App) {
        if !cfg!(target_os = "macos") || !window.is_window_active() {
            return;
        }
        if !self.session.read(cx).has_project() {
            return;
        }
        let Some(status) = &self.status else {
            return;
        };
        let others = other_branches(status.git.branch.as_deref(), &status.git.branches);
        let blocked = self.reasons(cx)("collab.sync").is_err() || !status.git.repository;
        editor_session::commands::set_menu_branches(
            editor_session::commands::MenuBranches { others, blocked },
            cx,
        );
    }

    fn busy(&self) -> bool {
        self.stage.is_some() || self.status.as_ref().is_some_and(|status| status.busy)
    }

    /// Says `words` at the foot of the window for a while: `hold`, else
    /// as long as a notice (or an error) stays.
    fn notify(
        &self,
        words: &str,
        error: bool,
        hold: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, cx| {
            session.state.notify_held(words, error, hold);
            cx.emit(Change::Notice);
            cx.notify();
        });
    }

    /// Sends one collaboration command a person asked for.
    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        let kind = command.kind().to_owned();
        if self.session.update(cx, |session, _| session.send(command)) {
            self.asked = Some(kind);
            self.busy_since = Instant::now();
        }
    }
}

impl EventEmitter<CollabEvent> for Collab {}

impl Render for Collab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        if !session.has_project() || self.status.is_none() {
            return div().into_any_element();
        }
        let shown = match self.menu {
            Menu::Closed => None,
            Menu::Prompt(prompt) => Some(self.question(prompt, window, cx)),
            Menu::Open | Menu::Branches => self.kit.clone().map(|kit| kit.into_any_element()),
        };
        div()
            .relative()
            .flex_none()
            .child(self.chip(window, cx))
            .when_some(shown, |wrapper, shown| {
                wrapper.child(
                    // Under the chip, its right edge on the chip's, over everything.
                    div().absolute().top_full().right_0().child(
                        deferred(
                            anchored()
                                .anchor(Corner::TopRight)
                                .offset(point(px(0.0), px(theme::space::S4)))
                                .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                                .child(shown),
                        )
                        .with_priority(1),
                    ),
                )
            })
            .into_any_element()
    }
}
