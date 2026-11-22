//! Details under a new project's name and place: the frame rate, and how
//! the project is shared (with Git, as a private GitHub repository, or
//! pushed to a remote) as the tools found to share with allow; folded, a
//! line that says what Create makes.

use super::form::{field, form_row, FORM_LABEL};
use super::keys::Stop;
use super::plan::home_relative;
use super::{LauncherView, Mode};
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, AnimationExt, AnyElement, App, Context, ElementId, Window};
use serde_json::Value;

/// The frame rates a new project can have, as documents write them.
pub(super) const RATES: [(&str, &str); 6] = [
    ("23.976", "23.976"),
    ("24", "24"),
    ("25", "25"),
    ("29.97", "29.97"),
    ("30", "30"),
    ("60", "60"),
];

/// What the Sharing rows say of the tools the bridge found (`{git,
/// gitLfs, gh, ghUser}`), before they are found (None), or why they could
/// not be (`error`).
pub fn tools_hint(tools: Option<&Value>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return format!("Kerf could not look for Git and the GitHub CLI: {error}");
    }
    let Some(tools) = tools else {
        return "Looking for Git and the GitHub CLI…".into();
    };
    if tools["git"] != true {
        return if cfg!(target_os = "macos") {
            "Install Git to share projects (xcode-select --install).".into()
        } else {
            "Install Git to share projects.".into()
        };
    }
    let mut hints = vec![match github_account(tools) {
        Some(user) => format!("Signed in to GitHub as @{user}."),
        None if tools["gh"] == true => {
            "Sign in with `gh auth login` to create a GitHub repository here.".to_owned()
        }
        None if cfg!(target_os = "macos") => {
            "Install the GitHub CLI (brew install gh) to create a GitHub repository here."
                .to_owned()
        }
        None => "Install the GitHub CLI (gh) to create a GitHub repository here.".to_owned(),
    }];
    if tools["gitLfs"] != true {
        hints.push("Git LFS is not installed: media of 100 MB or more cannot be shared.".into());
    }
    hints.join(" ")
}

/// The GitHub account the GitHub CLI is signed in to, if it is.
pub fn github_account(tools: &Value) -> Option<&str> {
    tools["ghUser"]
        .as_str()
        .filter(|user| tools["gh"] == true && !user.is_empty())
}

/// One of the Sharing switches: the stop the keys reach it at, its id, how
/// it is set, whether it can be turned on, its words, and what turns it.
struct Switch {
    stop: Stop,
    id: &'static str,
    on: bool,
    enabled: bool,
    words: String,
    toggle: fn(&mut LauncherView, bool, &mut Context<LauncherView>),
}

impl LauncherView {
    /// What the bridge found to share with (`{git, gitLfs, gh, ghUser}`),
    /// or why it could not look; a switch with nothing to share with goes
    /// off.
    pub(super) fn found_tools(&mut self, status: Value, cx: &mut Context<Self>) {
        if status["status"] == "done" {
            if status["git"] != true {
                self.git = false;
            }
            if github_account(&status).is_none() {
                self.github = false;
            }
            self.tools = Some(status);
        } else {
            // Sharing still works when Git is there; the task says if not.
            self.tools_error = Some(
                status["error"]
                    .as_str()
                    .unwrap_or("the task stopped")
                    .to_owned(),
            );
        }
        cx.notify();
    }

    /// Git is there, or not known yet not to be.
    pub(super) fn git_found(&self) -> bool {
        self.tools.as_ref().is_none_or(|tools| tools["git"] == true)
    }

    /// Share with Git, or not; off, nothing is published either.
    pub(super) fn set_git(&mut self, on: bool, cx: &mut Context<Self>) {
        if on && !self.git_found() {
            return;
        }
        self.git = on;
        // Without Git there is nothing to publish.
        if !on {
            self.github = false;
            self.remote.update(cx, |remote, cx| remote.clear(cx));
            self.git = false;
        }
        cx.notify();
    }

    /// A private GitHub repository, or not, while the GitHub CLI is signed
    /// in; on, Git is too.
    pub(super) fn set_github(&mut self, on: bool, cx: &mut Context<Self>) {
        let signed_in = self.tools.as_ref().and_then(github_account).is_some();
        if on && !signed_in {
            return;
        }
        self.github = on;
        if on {
            self.git = true;
        }
        cx.notify();
    }

    /// Details unfolded or folded, kept for the next time.
    pub(super) fn set_details(&mut self, open: bool, cx: &mut Context<Self>) {
        self.details = open;
        self.prefs.refresh();
        self.prefs.launcher_details = open;
        self.prefs.save();
        cx.notify();
    }

    /// What Create makes, in one line: the name, what it starts from, the
    /// frame rate, where, and how it is shared.
    fn summary(&self, cx: &App) -> String {
        let name = self.name.read(cx).text().trim().to_owned();
        let mut summary = vec![if name.is_empty() {
            "Create an unnamed project".to_owned()
        } else {
            format!("Create “{name}”")
        }];
        if self.mode == Some(Mode::Preset) {
            summary.push(match self.preset.name() {
                Some(preset) => format!("from {preset}"),
                None => "from a preset".into(),
            });
        } else {
            summary.push(self.format.label().to_owned());
        }
        summary.push(format!("{} fps", self.rate));
        summary.push(home_relative(&self.location(cx)));
        if self.github {
            summary.push("private GitHub repository".into());
        } else if !self.remote.read(cx).text().trim().is_empty() {
            summary.push("Git, pushed to the remote".into());
        } else if self.git {
            summary.push("Git".into());
        }
        summary.join(" · ")
    }

    /// The Details disclosure: the frame rate and how the project is shared.
    pub(super) fn details_rows(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![self.details_toggle(window, cx)];
        if !self.details {
            return rows;
        }
        rows.push(self.rate_row(window, cx));
        rows.push(self.sharing_row(window, cx));
        rows.into_iter()
            .enumerate()
            .map(|(index, row)| {
                if index == 0 {
                    row
                } else {
                    div()
                        .child(row)
                        .with_animation(
                            ElementId::named_usize("launcher-details-in", index),
                            motion::animation(theme::motion::APPEAR),
                            |row, t| row.opacity(t),
                        )
                        .into_any_element()
                }
            })
            .collect()
    }

    /// Details' own row: its chevron and its name; folded, a line beside it
    /// that says what Create makes.
    fn details_toggle(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let toggle = div()
            .id("launcher-details")
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .cursor_pointer()
            .child(
                if self.details {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .view(12.0, theme::ink2()),
            )
            .child(ui::label("Details", text::CONTROL, theme::ink2()))
            .on_click(cx.listener(|this, _, _, cx| this.set_details(!this.details, cx)));
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .child(
                self.ring(Stop::Details, window, toggle)
                    .w(px(FORM_LABEL))
                    .flex_none(),
            )
            .when(!self.details, |row| {
                row.child(
                    ui::line(self.summary(cx), text::CONTROL, theme::ink3())
                        .flex_1()
                        .min_w_0(),
                )
            })
            .into_any_element()
    }

    /// The frame rate, one of those documents write.
    fn rate_row(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        form_row(
            "Frame rate",
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .child(self.ring(
                    Stop::Rate,
                    window,
                    ui::segmented(
                        "launcher-rate",
                        &RATES,
                        self.rate,
                        Self::chooser(cx, |this, rate, _, cx| {
                            this.rate = rate;
                            cx.notify();
                        }),
                    ),
                ))
                .child(ui::label("fps", text::CONTROL, theme::ink3())),
        )
    }

    /// How the project is shared: with Git, as a private GitHub repository,
    /// or pushed to a remote, as the tools found allow, and what they say.
    fn sharing_row(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let signed_in = self
            .tools
            .as_ref()
            .and_then(github_account)
            .map(str::to_owned);
        let git_found = self.git_found();
        let remote_open = git_found && !self.github;
        let remote = if remote_open {
            field(&self.remote, window, cx).flex_1().into_any_element()
        } else {
            div()
                .flex_1()
                .h(px(height::H28))
                .flex()
                .items_center()
                .px(px(theme::space::S8))
                .rounded(px(radius::CONTROL))
                .border_1()
                .border_color(theme::line())
                .child(ui::line(
                    "Or push to a remote: git@github.com:you/film.git",
                    text::BODY,
                    theme::dis(),
                ))
                .into_any_element()
        };
        form_row(
            "Sharing",
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::space::S6))
                .child(
                    self.switch_row(
                        Switch {
                            stop: Stop::Git,
                            id: "launcher-git",
                            on: self.git,
                            enabled: git_found,
                            words:
                                "Share with Git, so collaborators pull, merge and push its edits"
                                    .into(),
                            toggle: |this, on, cx| this.set_git(on, cx),
                        },
                        window,
                        cx,
                    ),
                )
                .child(self.switch_row(
                    Switch {
                        stop: Stop::GitHub,
                        id: "launcher-github",
                        on: self.github,
                        enabled: signed_in.is_some(),
                        words: match &signed_in {
                            Some(user) => format!("Create a private GitHub repository as @{user}"),
                            None => "Create a private GitHub repository".into(),
                        },
                        toggle: |this, on, cx| this.set_github(on, cx),
                    },
                    window,
                    cx,
                ))
                .child(div().flex().child(remote))
                .child(ui::label(
                    tools_hint(self.tools.as_ref(), self.tools_error.as_deref()),
                    text::CONTROL,
                    theme::ink3(),
                )),
        )
    }

    /// A Sharing switch with the ring the keys show on it, greyed while it
    /// cannot be turned on, and its words.
    fn switch_row(&self, switch: Switch, window: &Window, cx: &Context<Self>) -> gpui::Div {
        let Switch {
            stop,
            id,
            on,
            enabled,
            words,
            toggle,
        } = switch;
        let this = cx.entity().downgrade();
        let control = ui::switch(id, on, move |on, _, cx| {
            if enabled {
                let _ = this.update(cx, |this, cx| toggle(this, on, cx));
            }
        });
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .child(
                self.ring(stop, window, control)
                    .when(!enabled, |ring| ring.opacity(0.4)),
            )
            .child(ui::label(
                words,
                text::CONTROL,
                if enabled { theme::ink() } else { theme::dis() },
            ))
    }
}
