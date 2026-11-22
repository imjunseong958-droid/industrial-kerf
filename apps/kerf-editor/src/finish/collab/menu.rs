//! The chip's menu, the kit's: where the project stands over its rows
//! (Sync Now, Live Sync, Commit…, Switch Branch ▸ and the rest), each greyed
//! with why while it cannot run, opened and closed with the keys going back
//! where they were.

use super::question::Prompt;
use super::said::stage_words;
use super::{Collab, Menu, MENU_WIDTH};
use crate::finish::plural;
use crate::theme::{self, text};
use crate::ui;
use gpui::{div, prelude::*, px, AnyElement, App, Context, DismissEvent, SharedString, Window};
use kerf_bridge::event::CollabStatus;
use kerf_bridge::Command;

impl Collab {
    pub(super) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu != Menu::Closed {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menus_opened += 1;
        // The keys go back here once it goes.
        self.previous_focus = window.focused(cx);
        // What it says is as git has it now.
        self.session
            .update(cx, |session, _| session.send(Command::CollabStatus));
        self.show_menu(false, window, cx);
    }

    /// The menu's rows, the kit's: it takes the keys (the arrows, Return,
    /// a letter, Esc are its own) and gives them back to what had them
    /// before it opened. `branches`: opened on Switch Branch's submenu.
    pub(super) fn show_menu(
        &mut self,
        branches: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // From a question, the keys go back where they were first.
        if let Some(previous) = self.previous_focus.clone() {
            window.focus(&previous);
        }
        let items = self.items(cx);
        let switch_at = items
            .iter()
            .position(|item| item.words().as_ref() == "Switch Branch");
        let head_of = cx.entity().downgrade();
        let kit = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx)
                .width(MENU_WIDTH)
                .head(move |_, cx| match head_of.upgrade() {
                    Some(collab) => collab.read(cx).head_and_line(cx),
                    None => div().into_any_element(),
                });
            match switch_at.filter(|_| branches) {
                Some(at) => menu.opened(at),
                None => menu,
            }
        });
        cx.subscribe_in(&kit, window, |this, kit, _: &DismissEvent, _, cx| {
            if this.kit.as_ref() == Some(kit) {
                this.kit = None;
                // A row that asks for words has the menu's place.
                if matches!(this.menu, Menu::Open | Menu::Branches) {
                    this.menu = Menu::Closed;
                }
                cx.notify();
            }
        })
        .detach();
        self.kit = Some(kit);
        self.menu = if branches { Menu::Branches } else { Menu::Open };
        cx.notify();
    }

    /// The rows of the menu open as things stand now (git answered, a sync
    /// began or ended, the window's commands changed).
    pub(super) fn refresh_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(kit) = self.kit.clone() {
            let items = self.items(cx);
            kit.update(cx, |menu, cx| menu.set_items(items, cx));
        }
    }

    pub(super) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Menu::Closed;
        // The kit's menu may be what chose this: it goes once that is done.
        if let Some(kit) = self.kit.take() {
            cx.defer_in(window, move |_, window, cx| {
                kit.update(cx, |menu, cx| menu.close(window, cx));
            });
        }
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous);
        }
        cx.notify();
    }

    /// Runs `command` from the menu and closes it.
    fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        self.send(command, cx);
        self.close(window, cx);
    }

    /// The menu's head, and the line under it.
    fn head_and_line(&self, cx: &App) -> AnyElement {
        let status = self.status.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .child(self.head(&status, cx))
            .child(ui::hairline(true).mb(px(theme::space::S4)))
            .into_any_element()
    }

    /// The menu's head: where the project stands, and what went wrong.
    fn head(&self, status: &CollabStatus, cx: &App) -> impl IntoElement {
        let revision = self.session.read(cx).revision();
        let (title, standing) = if status.git.repository {
            let mut parts = vec![format!("Revision {revision}")];
            if status.git.behind > 0 {
                parts.push(plural(
                    status.git.behind as usize,
                    "1 commit to pull",
                    "{} commits to pull",
                ));
            }
            if status.git.ahead > 0 {
                parts.push(plural(
                    status.git.ahead as usize,
                    "1 commit to push",
                    "{} commits to push",
                ));
            }
            if status.git.behind == 0 && status.git.ahead == 0 {
                parts.push(if status.git.remote.is_some() {
                    "up to date".into()
                } else {
                    "on this computer only".into()
                });
            }
            (
                status
                    .git
                    .branch
                    .clone()
                    .unwrap_or_else(|| "Detached".into()),
                parts.join(" · "),
            )
        } else {
            ("Not shared yet".to_owned(), format!("Revision {revision}"))
        };
        let problem = if status.git.conflicted {
            Some("A merge is unfinished; finish it with git".to_owned())
        } else if let Some(enclosing) = status.enclosing.as_ref().filter(|_| !status.git.repository)
        {
            Some(format!(
                "Inside the repository at {}; publish it as its own repository to share it",
                enclosing.display()
            ))
        } else {
            self.failure
                .last()
                .map(|failure| format!("Sync failed: {failure}"))
        };
        let note = if self.busy() {
            Some(stage_words(self.stage.as_ref()).to_owned())
        } else {
            self.report.clone()
        };
        let who = status
            .git
            .user
            .as_ref()
            .map(|user| format!("Commits as {}", user.name))
            .filter(|_| status.git.repository);
        div()
            .px(px(theme::space::S8))
            .pt(px(theme::space::S6))
            .pb(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .child(ui::line(title, text::TITLE, theme::ink()))
            .child(ui::label(standing, text::META, theme::ink3()))
            .when_some(who, |head, who| {
                head.child(ui::line(who, text::META, theme::ink3()))
            })
            .when_some(problem.clone(), |head, problem| {
                head.child(
                    div()
                        .pt(px(theme::space::S4))
                        .flex()
                        .items_start()
                        .gap(px(theme::space::S6))
                        .child(
                            div()
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .child(ui::dot(theme::neg(), 6.0)),
                        )
                        .child(
                            ui::label(problem, text::META, theme::ink2())
                                .flex_1()
                                .min_w_0(),
                        ),
                )
            })
            .when_some(note.filter(|_| problem.is_none()), |head, note| {
                head.child(ui::label(note, text::META, theme::ink2()).pt(px(theme::space::S2)))
            })
    }

    /// Why each of the menu's commands cannot run now, as the registry says
    /// it for the whole window (a project.kerf changed on disk holds back
    /// Sync and the branches); while a sync runs, that.
    pub(super) fn reasons(&self, cx: &App) -> impl Fn(&str) -> Result<(), SharedString> {
        let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
        let busy = self.busy();
        move |id: &str| {
            if busy && id != "collab.live" {
                return Err("A sync is running".into());
            }
            match crate::palette::unavailable(&states, id) {
                Some(reason) => Err(reason.to_owned().into()),
                None => Ok(()),
            }
        }
    }

    /// The menu's rows as things stand: Sync Now, Live Sync (ticked while
    /// on), Commit…; Switch Branch ▸ (the other branches, or No Other
    /// Branches), New Branch…, Pull Request…, Publish to GitHub… while
    /// there is no remote; Consolidate Media and Refresh. A project that is
    /// no repository yet has Publish to GitHub… and Make a Repository Here.
    /// A row that cannot run is greyed, and says why in its tip.
    fn items(&self, cx: &App) -> Vec<ui::MenuItem> {
        let status = self.status.clone().unwrap_or_default();
        let reason = self.reasons(cx);
        let this = self.this.clone();
        let row = |words: &'static str, enabled: Result<(), SharedString>| {
            let item = ui::MenuItem::new(words);
            match enabled {
                Ok(()) => item,
                Err(why) => item.disabled(true).tip(why),
            }
        };
        let asks = |prompt: Prompt| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |collab, cx| collab.ask(prompt, true, window, cx));
            }
        };
        let runs = |command: Command| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |collab, cx| collab.run(command.clone(), window, cx));
            }
        };
        let mut items: Vec<ui::MenuItem> = Vec::new();
        if !status.git.repository {
            items.push(
                row("Publish to GitHub…", reason("collab.publish"))
                    .on_choose(asks(Prompt::Publish)),
            );
            items.push(
                row("Make a Repository Here", reason("collab.publish")).on_choose(runs(
                    Command::CollabPublish {
                        remote: None,
                        github: None,
                    },
                )),
            );
        } else {
            let live = status.live;
            let set_live = this.clone();
            items.push(
                row("Sync Now", reason("collab.sync"))
                    .on_choose(runs(Command::CollabSync { message: None })),
            );
            items.push(
                row("Live Sync", reason("collab.live"))
                    .checked(live)
                    .on_choose(move |_, cx| {
                        let _ = set_live.update(cx, |collab, cx| collab.set_live(!live, cx));
                    }),
            );
            items.push(row("Commit…", reason("collab.commit")).on_choose(asks(Prompt::Commit)));
            // Switch Branch opens on the other branches; with none it says
            // so, and holds back only for what holds back a sync.
            let others = other_branches(status.git.branch.as_deref(), &status.git.branches);
            let switch = if others.is_empty() {
                reason("collab.sync")
            } else {
                reason("collab.switchBranch")
            };
            let rows = if others.is_empty() {
                vec![ui::MenuItem::new(NO_OTHER_BRANCHES).disabled(true)]
            } else {
                others
                    .into_iter()
                    .map(|name| {
                        ui::MenuItem::new(name.clone()).on_choose(runs(Command::CollabBranch {
                            name,
                            create: false,
                        }))
                    })
                    .collect()
            };
            items.push(row("Switch Branch", switch).separated().submenu(rows));
            items.push(
                row("New Branch…", reason("collab.newBranch")).on_choose(asks(Prompt::NewBranch)),
            );
            items.push(
                row("Pull Request…", reason("collab.pullRequest"))
                    .on_choose(asks(Prompt::PullRequest)),
            );
            if status.git.remote.is_none() {
                items.push(
                    row("Publish to GitHub…", reason("collab.publish"))
                        .on_choose(asks(Prompt::Publish)),
                );
            }
        }
        items.push(
            row("Consolidate Media", reason("collab.consolidate"))
                .separated()
                .on_choose(runs(Command::CollabConsolidate)),
        );
        let refresh = this.clone();
        items.push(row("Refresh", Ok(())).on_choose(move |window, cx| {
            let _ = refresh.update(cx, |collab, cx| {
                collab
                    .session
                    .update(cx, |session, _| session.send(Command::CollabStatus));
                collab.close(window, cx);
            });
        }));
        items
    }

    pub(super) fn set_live(&mut self, on: bool, cx: &mut Context<Self>) {
        // The switch moves at once; the bridge's status follows.
        if let Some(status) = self.status.as_mut() {
            status.live = on;
        }
        self.session
            .update(cx, |session, _| session.set_live_sync(on));
        cx.notify();
    }
}

/// What Switch Branch's submenu says with no branch but this one.
pub const NO_OTHER_BRANCHES: &str = "No Other Branches";

/// The branches to switch to from `current`: the others, each once, in
/// the order git lists them.
pub fn other_branches(current: Option<&str>, branches: &[String]) -> Vec<String> {
    let mut others: Vec<String> = Vec::new();
    for branch in branches {
        if !branch.is_empty() && Some(branch.as_str()) != current && !others.contains(branch) {
            others.push(branch.clone());
        }
    }
    others
}
