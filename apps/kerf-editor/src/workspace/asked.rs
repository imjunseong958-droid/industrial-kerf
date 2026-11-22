//! The questions over the window, asked first: [`Confirm`], before what
//! cannot be undone or what changes the window (Cancel, or what it asks), and
//! [`AskWords`], for the line of words a sync command needs
//! ([`sync_question`]). Esc and Enter answer them before the window's own keys.

use crate::input::{InputEvent, TextInput};
use crate::theme::{self, text};
use crate::ui;
use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, KeyBinding, SharedString, Subscription, Window,
};

gpui::actions!(confirm, [CloseConfirm, ConfirmDefault]);

/// The key context a question over the window answers in.
pub(super) const CONFIRM: &str = "Confirm";

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

/// What a [`Confirm`] does, once, when what it asks is chosen.
type ConfirmHandler = Box<dyn FnOnce(&mut Window, &mut App)>;

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
    on_confirm: Option<ConfirmHandler>,
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

/// What a sync command asks before it runs ([`AskWords`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordsAsked {
    pub title: String,
    pub detail: String,
    /// What the field says while empty, and what it holds at first.
    pub placeholder: String,
    pub text: String,
    /// Its button's words.
    pub action: String,
}

/// What sync command `id` asks for: Commit's message, New Branch's name, a pull
/// request's title (suggested from `branch`), the GitHub repository's name
/// (from the project's `folder`). None for a command that needs no words.
pub fn sync_question(id: &str, branch: &str, folder: &str) -> Option<WordsAsked> {
    let asked =
        |title: &str, detail: String, placeholder: &str, text: String, action: &str| WordsAsked {
            title: title.into(),
            detail,
            placeholder: placeholder.into(),
            text,
            action: action.into(),
        };
    Some(match id {
        "collab.commit" => asked(
            "Commit",
            "Describe the change".into(),
            "Message",
            String::new(),
            "Commit",
        ),
        "collab.newBranch" => asked(
            "New Branch",
            "Commits your edits, then starts a branch from them.".into(),
            "Name",
            String::new(),
            "Create",
        ),
        "collab.pullRequest" => {
            let mut title: String = branch.replace(['-', '_'], " ");
            if let Some(first) = title.chars().next() {
                title.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
            }
            let which = if branch.is_empty() {
                "this branch"
            } else {
                branch
            };
            asked(
                "Open Pull Request",
                format!(
                    "Pushes {which} and opens a pull request on GitHub. Reviewers read the change as Kerf code."
                ),
                "Title",
                title,
                "Open",
            )
        }
        "collab.publish" => {
            let name: String = folder
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                        ch
                    } else {
                        '-'
                    }
                })
                .collect();
            asked(
                "Publish to GitHub",
                "Makes the project a Git repository if it is not one yet, then pushes it so collaborators can clone it, sync, and open pull requests. The repository is private."
                    .into(),
                "Repository name",
                name,
                "Publish",
            )
        }
        _ => return None,
    })
}

/// The parameters sync command `id` runs with, given the `words` asked for.
pub fn sync_params(id: &str, words: &str) -> serde_json::Value {
    let words = words.trim();
    match id {
        "collab.commit" => serde_json::json!({"message": words}),
        "collab.newBranch" => serde_json::json!({"name": words}),
        "collab.pullRequest" => serde_json::json!({"title": words}),
        "collab.publish" => serde_json::json!({"github": {"name": words, "private": true}}),
        _ => serde_json::json!({}),
    }
}

/// What an [`AskWords`] does, once, with the field's words when it is run.
type AcceptHandler = Box<dyn FnOnce(&str, &mut App)>;

/// Asks over the window for a line of words a command needs: its title, what it
/// is for, the field, Cancel and the button that runs it (not while the field
/// is empty). Esc or a press on the dim cancels; Return runs it.
pub struct AskWords {
    focus_handle: FocusHandle,
    asked: WordsAsked,
    field: Entity<TextInput>,
    on_accept: Option<AcceptHandler>,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for AskWords {}

impl AskWords {
    pub fn new(
        asked: WordsAsked,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_accept: impl FnOnce(&str, &mut App) + 'static,
    ) -> Self {
        bind_confirm(cx);
        let field = cx.new(|cx| {
            let mut field = TextInput::new(cx, asked.placeholder.clone()).with_style(text::CONTROL);
            field.set_text(asked.text.clone(), cx);
            field.choose_all(cx);
            field
        });
        let subscription =
            cx.subscribe_in(
                &field,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.accept(cx),
                    InputEvent::Cancel => cx.emit(DismissEvent),
                    InputEvent::Changed => cx.notify(),
                },
            );
        Self {
            focus_handle: cx.focus_handle(),
            asked,
            field,
            on_accept: Some(Box::new(on_accept)),
            _subscription: subscription,
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let words = self.field.read(cx).text().trim().to_owned();
        if words.is_empty() {
            return;
        }
        if let Some(on_accept) = self.on_accept.take() {
            cx.emit(DismissEvent);
            on_accept(&words, cx);
        }
    }
}

impl Focusable for AskWords {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Typing goes into the field at once.
        self.field.read(cx).focus_handle(cx)
    }
}

impl Render for AskWords {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = !self.field.read(cx).text().trim().is_empty();
        div()
            .key_context(CONFIRM)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &CloseConfirm, _, cx| cx.emit(DismissEvent)))
            .on_action(cx.listener(|this, _: &ConfirmDefault, _, cx| this.accept(cx)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(DismissEvent);
            }))
            .w(px(CONFIRM_WIDTH))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(theme::popup())
            .child(ui::label(
                self.asked.title.clone(),
                text::TITLE,
                theme::ink(),
            ))
            .child(ui::label(
                self.asked.detail.clone(),
                text::BODY,
                theme::ink2(),
            ))
            .child(
                div()
                    .mt(px(8.0))
                    .h(px(theme::height::H28))
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .rounded(px(theme::radius::CONTROL))
                    .bg(theme::card())
                    .border_1()
                    .border_color(theme::line2())
                    .child(self.field.clone()),
            )
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        ui::button("words-cancel", "Cancel").on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(DismissEvent)),
                        ),
                    )
                    .child(
                        ui::button("words-action", self.asked.action.clone())
                            .primary()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.accept(cx))),
                    ),
            )
    }
}
