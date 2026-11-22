//! The questions the chip's menu asks in its place before a row runs
//! (Commit…, New Branch…, Open Pull Request…, Publish to GitHub…): their
//! words and fields, Publish's three ways, and what each sends.

use super::{Collab, Menu, MENU_WIDTH};
use crate::finish::{Close, MENU_CONTEXT};
use editor_ui::input::TextInput;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, radius, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, App, Context, Entity, Focusable, FontWeight,
    Window,
};
use kerf_bridge::{Command, GitHubRepository};

/// A question that holds more than a line is wider than the menu.
const PROMPT_WIDTH: f32 = 380.0;
/// A pull request's description: its least height, and the most it grows to.
const BODY_LEAST: f32 = 72.0;
pub(super) const BODY_MOST: f32 = 144.0;

/// What the menu asks words for before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prompt {
    Publish,
    Commit,
    NewBranch,
    PullRequest,
}

impl Prompt {
    /// The one the registry's command `id` asks with.
    pub(super) fn of_command(id: &str) -> Option<Prompt> {
        match id {
            "collab.publish" => Some(Prompt::Publish),
            "collab.commit" => Some(Prompt::Commit),
            "collab.newBranch" => Some(Prompt::NewBranch),
            "collab.pullRequest" => Some(Prompt::PullRequest),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Prompt::Publish => "Publish to GitHub",
            Prompt::Commit => "Commit",
            Prompt::NewBranch => "New Branch",
            Prompt::PullRequest => "Open Pull Request",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Prompt::Publish => "Repository name",
            Prompt::Commit => "Describe the change",
            Prompt::NewBranch => "Name",
            Prompt::PullRequest => "Title",
        }
    }

    /// What it does, said over its fields; `branch` is the one checked out.
    fn says(self, branch: Option<&str>) -> String {
        match self {
            Prompt::Publish => "Makes the project a Git repository if it is not one yet, then pushes it so collaborators can clone it, sync, and open pull requests.".into(),
            Prompt::Commit => "Commits your edits without syncing. Left empty, the message describes them.".into(),
            Prompt::NewBranch => "Commits your edits, then starts a branch from them. The next sync publishes it.".into(),
            Prompt::PullRequest => format!(
                "Pushes {} and opens a pull request on GitHub. Reviewers read the change as Kerf code.",
                branch.filter(|branch| !branch.is_empty()).unwrap_or("this branch")
            ),
        }
    }

    fn action(self) -> &'static str {
        match self {
            Prompt::Publish => "Publish",
            Prompt::Commit => "Commit",
            Prompt::NewBranch => "Create Branch",
            Prompt::PullRequest => "Open Pull Request",
        }
    }

    /// Its panel is wider than the menu: it holds more than a line.
    fn wide(self) -> bool {
        matches!(self, Prompt::Publish | Prompt::PullRequest)
    }
}

/// How Publish shares the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishWay {
    /// A new repository on GitHub, made with the GitHub CLI.
    GitHub,
    /// A remote the person already has (GitHub, GitLab, their own server).
    Remote,
    /// A repository on this computer only.
    Local,
}

impl PublishWay {
    pub const ALL: [PublishWay; 3] = [PublishWay::GitHub, PublishWay::Remote, PublishWay::Local];

    pub fn label(self) -> &'static str {
        match self {
            PublishWay::GitHub => "GitHub",
            PublishWay::Remote => "Remote",
            PublishWay::Local => "This computer",
        }
    }

    /// What choosing it does.
    pub fn says(self) -> &'static str {
        match self {
            PublishWay::GitHub => "Create a GitHub repository",
            PublishWay::Remote => "Push to a remote I already have",
            PublishWay::Local => "Only make it a repository on this computer",
        }
    }
}

/// Why GitHub cannot be chosen without its command-line tool.
pub const NEEDS_GH: &str = "Install the GitHub CLI and sign in with `gh auth login`";

/// The repository name Publish starts with: the project's folder name,
/// each run of anything but letters, digits, `.`, `_` and `-` one `-`.
pub fn publish_name(folder: &str) -> String {
    let mut name = String::new();
    let mut gap = false;
    for ch in folder.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if gap {
                name.push('-');
            }
            gap = false;
            name.push(ch);
        } else {
            gap = true;
        }
    }
    if gap {
        name.push('-');
    }
    name
}

/// The title Open Pull Request starts with: the branch's name, its dashes
/// and underscores as spaces, its first letter a capital.
pub fn pull_request_title(branch: &str) -> String {
    let words = branch.replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What Publish sends for `way`: a GitHub repository named `name` (the
/// folder's when empty), private or not; a push to `remote`; or a
/// repository here only. None while the remote is not given.
pub fn publish_command(
    way: PublishWay,
    name: &str,
    private: bool,
    remote: &str,
) -> Option<Command> {
    match way {
        PublishWay::GitHub => {
            let name = name.trim();
            Some(Command::CollabPublish {
                remote: None,
                github: Some(GitHubRepository {
                    name: (!name.is_empty()).then(|| name.to_owned()),
                    private,
                }),
            })
        }
        PublishWay::Remote => {
            let remote = remote.trim();
            (!remote.is_empty()).then(|| Command::CollabPublish {
                remote: Some(remote.to_owned()),
                github: None,
            })
        }
        PublishWay::Local => Some(Command::CollabPublish {
            remote: None,
            github: None,
        }),
    }
}

/// What Open Pull Request sends: its title, and its description when one
/// is written (left out, the bridge lists the branch's commits). None
/// without a title.
pub fn pull_request_command(title: &str, body: &str) -> Option<Command> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    Some(Command::CollabPullRequest {
        title: title.to_owned(),
        body: (!body.trim().is_empty()).then(|| body.to_owned()),
    })
}

impl Collab {
    /// From words asked for in the menu's place, back to the menu; from
    /// the menu, or words asked for from the window's menus or the
    /// palette, closed.
    pub(super) fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.menu, Menu::Prompt(_)) && self.asked_over_menu {
            self.show_menu(false, window, cx);
        } else {
            self.close(window, cx);
        }
    }

    /// Asks for the words `prompt` needs, in the menu's place: from one of
    /// its rows (`over_menu`), or from the window's menus or the palette.
    pub(super) fn ask(
        &mut self,
        prompt: Prompt,
        over_menu: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = Menu::Prompt(prompt);
        self.asked_over_menu = over_menu;
        let status = self.status.clone().unwrap_or_default();
        let start = match prompt {
            Prompt::Publish => {
                // The folder's name; one with no letter GitHub takes (a
                // Hangul name) gives the project's title.
                let session = self.session.read(cx);
                let folder = session
                    .project
                    .as_deref()
                    .and_then(std::path::Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                [folder, session.project_name()]
                    .iter()
                    .map(|name| publish_name(name))
                    .find(|name| name.chars().any(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or_default()
            }
            Prompt::PullRequest => {
                pull_request_title(status.git.branch.as_deref().unwrap_or_default())
            }
            Prompt::Commit | Prompt::NewBranch => String::new(),
        };
        self.input.update(cx, |input, cx| {
            input.set_text(start, cx);
            input.set_placeholder(prompt.placeholder(), cx);
        });
        self.remote.update(cx, |input, cx| input.clear(cx));
        self.body.update(cx, |input, cx| input.clear(cx));
        self.publish_private = true;
        // GitHub when its command-line tool is there, else a remote of one's own.
        self.publish_way = if status.gh {
            PublishWay::GitHub
        } else {
            PublishWay::Remote
        };
        self.focus_field(window, cx);
        cx.notify();
    }

    /// The keys to the field the question is waiting on.
    fn focus_field(&self, window: &mut Window, cx: &mut Context<Self>) {
        let field = match (self.menu, self.publish_way) {
            (Menu::Prompt(Prompt::Publish), PublishWay::Remote) => &self.remote,
            (Menu::Prompt(Prompt::Publish), PublishWay::Local) => {
                window.focus(&self.focus_handle);
                return;
            }
            _ => &self.input,
        };
        field.read(cx).focus(window);
    }

    fn choose_way(&mut self, way: PublishWay, window: &mut Window, cx: &mut Context<Self>) {
        self.publish_way = way;
        self.focus_field(window, cx);
        cx.notify();
    }

    /// What the question sends as it stands; None while it cannot be sent.
    fn command(&self, prompt: Prompt, cx: &App) -> Option<Command> {
        let words = self.input.read(cx).text().trim().to_owned();
        match prompt {
            Prompt::Publish => publish_command(
                self.publish_way,
                &words,
                self.publish_private,
                self.remote.read(cx).text(),
            ),
            Prompt::Commit => Some(Command::CollabCommit { message: words }),
            Prompt::NewBranch => {
                let name = branch_name(&words);
                (!name.is_empty()).then_some(Command::CollabBranch { name, create: true })
            }
            Prompt::PullRequest => pull_request_command(&words, self.body.read(cx).text()),
        }
    }

    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Menu::Prompt(prompt) = self.menu else {
            return;
        };
        let Some(command) = self.command(prompt, cx) else {
            return;
        };
        self.send(command, cx);
        self.close(window, cx);
    }

    /// A question asked in the menu's place (Commit…, New Branch…, Pull
    /// Request…, Publish to GitHub…), rising into place as it fades in.
    pub(super) fn question(
        &self,
        prompt: Prompt,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let width = if prompt.wide() {
            PROMPT_WIDTH
        } else {
            MENU_WIDTH
        };
        let rise = scene::MENU_RISE;
        div()
            .key_context(MENU_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, window, cx| this.back(window, cx)))
            // A press anywhere else puts it away, and does nothing more: a
            // press on the chip closes it rather than opening it again.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .occlude()
            .relative()
            .w(px(width))
            .flex()
            .flex_col()
            .p(px(theme::space::S4))
            .rounded(px(radius::CARD))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(self.prompt(prompt, window, cx))
            .with_animation(
                ("collab-question", self.menus_opened),
                motion::animation(scene::FADE),
                move |menu, t| menu.opacity(t).top(px(rise * (1.0 - t))),
            )
            .into_any_element()
    }

    /// Publish's three ways, and the fields of the one chosen.
    fn publish_fields(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let status = self.status.clone().unwrap_or_default();
        let this = cx.entity().downgrade();
        let ways = PublishWay::ALL.map(|way| (way, way.label()));
        let mut chooser = ui::segmented(
            "collab-publish-way",
            &ways,
            self.publish_way,
            move |way, window, cx| {
                let _ = this.update(cx, |this, cx| this.choose_way(way, window, cx));
            },
        )
        .tips(&[
            if status.gh { "" } else { NEEDS_GH },
            "",
            if status.git.repository {
                "The project is a repository already"
            } else {
                ""
            },
        ]);
        if !status.gh {
            chooser = chooser.disabled(PublishWay::GitHub);
        }
        if status.git.repository {
            chooser = chooser.disabled(PublishWay::Local);
        }
        let this = cx.entity().downgrade();
        let fields = match self.publish_way {
            PublishWay::GitHub => div()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(
                    editor_launcher::launcher::field(&self.input, window, cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(ui::label("Private", text::CONTROL, theme::ink2()))
                .child(ui::switch(
                    "collab-publish-private",
                    self.publish_private,
                    move |on, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.publish_private = on;
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
            PublishWay::Remote => {
                editor_launcher::launcher::field(&self.remote, window, cx).into_any_element()
            }
            PublishWay::Local => div().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(div().flex().child(chooser))
            .child(ui::label(self.publish_way.says(), text::CONTROL, theme::ink2()))
            .child(fields)
            .when_some(status.enclosing.filter(|enclosing| !enclosing.as_os_str().is_empty()), |column, enclosing| {
                column.child(ui::label(
                    format!("This folder is inside another repository ({}); the project gets a repository of its own.", enclosing.display()),
                    text::META,
                    theme::ink3(),
                ))
            })
            .into_any_element()
    }

    fn prompt(&self, prompt: Prompt, window: &Window, cx: &Context<Self>) -> AnyElement {
        let ready = self.command(prompt, cx).is_some();
        let branch = self
            .status
            .as_ref()
            .and_then(|status| status.git.branch.clone());
        let fields = match prompt {
            Prompt::Publish => self.publish_fields(window, cx),
            Prompt::PullRequest => div()
                .flex()
                .flex_col()
                .gap(px(theme::space::S6))
                .child(editor_launcher::launcher::field(&self.input, window, cx))
                .child(ui::label("Description", text::META_STRONG, theme::ink3()))
                .child(area(&self.body, window, cx))
                .into_any_element(),
            Prompt::Commit | Prompt::NewBranch => {
                editor_launcher::launcher::field(&self.input, window, cx).into_any_element()
            }
        };
        div()
            .p(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(ui::label(prompt.title(), text::TITLE, theme::ink()))
            .child(ui::label(
                prompt.says(branch.as_deref()),
                text::META,
                theme::ink3(),
            ))
            .child(fields)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S6))
                    .child(
                        ui::button("collab-prompt-cancel", "Cancel")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(
                        ui::button("collab-prompt-go", prompt.action())
                            .small()
                            .primary()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------- Collab's pieces

/// A field of several lines (a pull request's description), as the
/// launcher's one-line field looks.
fn area(input: &Entity<TextInput>, window: &Window, cx: &App) -> gpui::Div {
    let handle = input.read(cx).focus_handle(cx);
    let focused = handle.is_focused(window);
    div()
        .min_h(px(BODY_LEAST))
        .px(px(theme::space::S8))
        .py(px(theme::space::S6))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
            window.focus(&handle);
            window.prevent_default();
        })
        .child(input.clone())
}

/// A branch name git takes: trimmed, its spaces as dashes.
fn branch_name(words: &str) -> String {
    words.split_whitespace().collect::<Vec<_>>().join("-")
}
