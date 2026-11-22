//! The conversation's menus ([`Menu`]): the composer chips' (the folder,
//! the model, what Kerf does without asking) and a card's versions as the
//! kit's menu, the context meter's card, and the menu of the words a
//! right-click opened; Esc puts them away first. And what Kerf does
//! without asking: each mode's words and look, and choosing one, from the
//! chip's menu or with Shift-Tab, which goes round them.

use super::ChatView;
use editor_session::commands::RunCommand;
use editor_state::prefs::{model_label, Permissions, Provider};
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, App, ClipboardItem, Context, DismissEvent, ElementId, Rgba,
    SharedString, Subscription, Window,
};
use std::time::{Duration, Instant};

/// Which of the conversation's menus is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    Permissions,
    Model,
    /// The folder chip's: the folders, Open Folder… and New Project….
    Folder,
    /// A card's versions, by the proposal's id.
    Versions(String),
    /// The context meter's card.
    Context,
}

/// The mode after `permissions`, as Shift-Tab goes round them.
pub fn next_permissions(permissions: Permissions) -> Permissions {
    match permissions {
        Permissions::Ask => Permissions::AcceptEdits,
        Permissions::AcceptEdits => Permissions::Auto,
        Permissions::Auto => Permissions::Ask,
    }
}

/// How long a mode chosen stays said: 3 s, long enough to read its line.
pub use editor_ui::theme::motion::NOTICE_BRIEF_HOLD as MODE_HOLD;

/// Says the mode `permissions` and what it means ("Auto · Edits apply, and
/// Kerf makes its shots") in place of the mode said last, for [`MODE_HOLD`]:
/// choosing again and again leaves one line, the newest.
pub fn say_permissions(state: &mut editor_state::store::EditorState, permissions: Permissions) {
    let (label, line) = permission_words(permissions);
    state.say_in_place(
        editor_state::store::StatusLine::Permissions,
        &format!("{label} · {line}"),
        false,
        Some(MODE_HOLD),
    );
}

/// What the permission chip says, and its line in the chip's menu.
pub fn permission_words(permissions: Permissions) -> (&'static str, &'static str) {
    match permissions {
        Permissions::Ask => ("Ask before edits", "Each edit waits for you"),
        Permissions::AcceptEdits => ("Accept edits", "Edits apply as Kerf makes them"),
        Permissions::Auto => ("Auto", "Edits apply, and Kerf makes its shots"),
    }
}

/// How the permission chip looks: a lock while each edit waits, a check
/// while edits apply, Kerf's own mark under Auto; drawn in the accent's ink
/// for Accept edits and the GPU's colour for Auto, so the modes where Kerf
/// does more on its own stand out. None draws it as the other chips are
/// drawn.
pub fn permission_look(permissions: Permissions) -> (Icon, Option<Rgba>) {
    match permissions {
        Permissions::Ask => (Icon::Lock, None),
        Permissions::AcceptEdits => (Icon::Check, Some(theme::accent_ink())),
        Permissions::Auto => (Icon::Kerf, Some(theme::activity::gpu())),
    }
}

/// The models the chip offers for `provider`: Claude Code's own aliases
/// ("default" leaves the choice to it), else the one typed in Settings, or
/// the service's default.
pub fn model_choices(provider: Provider, configured: &str) -> Vec<String> {
    match provider {
        Provider::ClaudeCode => ["default", "opus", "sonnet", "haiku"]
            .map(str::to_owned)
            .to_vec(),
        _ if configured.trim().is_empty() => vec!["default".to_owned()],
        _ => vec![configured.trim().to_owned()],
    }
}

impl ChatView {
    /// Whether a menu of the conversation is open (a composer chip's, a
    /// card's versions, the context card).
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The menu of the conversation's words at `at`, from a right-click on
    /// run `run`: Copy what is chosen (greyed with nothing chosen) and
    /// Select All of the run.
    pub(super) fn open_words_menu(
        &mut self,
        run: SharedString,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copied = self.selections.borrow().copied();
        let shared = self.selections.clone();
        let items = vec![
            ui::MenuItem::new("Copy")
                .key(editor_state::shortcuts::shown_keys("Ctrl+C"))
                .disabled(copied.is_none())
                .on_choose(move |_, cx| {
                    if let Some(words) = copied.clone() {
                        cx.write_to_clipboard(ClipboardItem::new_string(words));
                    }
                }),
            ui::MenuItem::new("Select All").on_choose(move |window, _| {
                if shared.borrow_mut().choose_all(&run) {
                    window.refresh();
                }
            }),
        ];
        let menu = cx.new(|cx| ui::Menu::new(items, window, cx));
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .words_menu
                .as_ref()
                .is_some_and(|(open, _)| *open == menu)
            {
                this.words_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.words_menu = Some((menu, at));
        cx.notify();
    }

    /// Whether the Esc being handled put a menu of the conversation away
    /// (its own keystroke intercept runs before the window's), once.
    pub fn take_escape(&mut self) -> bool {
        std::mem::take(&mut self.escape_took_menu)
    }

    /// What Kerf does without asking, chosen here (the chip's menu or
    /// Shift-Tab): kept, and said with what it means in place of the mode
    /// said last, for [`MODE_HOLD`].
    pub(super) fn set_permissions(&mut self, permissions: Permissions, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.prefs.permissions = permissions;
            session.save_prefs();
            say_permissions(&mut session.state, permissions);
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        let closed_by_this_press = self.menu_closed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(500)
        });
        if self.menu.as_ref() == Some(&menu) {
            self.menu = None;
            if let Some(kit) = self.kit_menu.take() {
                kit.update(cx, |kit, cx| kit.close(window, cx));
            }
        } else if !closed_by_this_press {
            if let Some(kit) = self.kit_menu.take() {
                kit.update(cx, |kit, cx| kit.close(window, cx));
            }
            self.open_kit_menu(&menu, window, cx);
            self.menu = Some(menu);
        }
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            self.menu_closed = Some((menu, Instant::now()));
        }
        self.kit_menu = None;
        cx.notify();
    }

    /// The kit's menu for `menu` (all but the context card, which is a
    /// card): its keys, its letters, Esc and a press outside close it.
    fn open_kit_menu(&mut self, menu: &Menu, window: &mut Window, cx: &mut Context<Self>) {
        let (items, header, width): (Vec<ui::MenuItem>, Option<&'static str>, f32) = match menu {
            Menu::Context => return,
            Menu::Folder => (self.folder_items(cx), None, 240.0),
            Menu::Permissions => (
                self.permissions_items(cx),
                Some("What Kerf does without asking"),
                280.0,
            ),
            Menu::Model => {
                let provider = self.session.read(cx).prefs.provider;
                (self.model_items(cx), Some(provider.label()), 240.0)
            }
            Menu::Versions(id) => (self.version_items(id, cx), None, 260.0),
        };
        let kit = cx.new(|cx| {
            let kit = ui::Menu::new(items, window, cx).width(width);
            match header {
                Some(header) => kit.header(header),
                None => kit,
            }
        });
        let open = menu.clone();
        cx.subscribe(&kit, move |this, kit, _: &DismissEvent, cx| {
            if this.kit_menu.as_ref() == Some(&kit) {
                this.kit_menu = None;
                if this.menu.as_ref() == Some(&open) {
                    this.close_menu(cx);
                }
            }
        })
        .detach();
        self.kit_menu = Some(kit);
    }

    /// The folder chip's menu: the folders (the open one ticked), Open
    /// Folder… and New Project…. Another folder opens in place of this one.
    fn folder_items(&self, cx: &App) -> Vec<ui::MenuItem> {
        use editor_sidebar::sidebar::Entry;
        let session = self.session.read(cx);
        let entries = editor_sidebar::sidebar::folder_entries(
            &session.prefs.recent,
            session.project.as_deref(),
        );
        entries
            .into_iter()
            .filter(|entry| matches!(entry.entry, Entry::Open(_) | Entry::Command(_)))
            .map(|entry| {
                let chosen = entry.checked;
                let run = entry.entry.clone();
                let mut item =
                    ui::MenuItem::new(entry.words)
                        .checked(chosen)
                        .on_choose(move |window, cx| match &run {
                            Entry::Open(folder) if !chosen => window.dispatch_action(
                                Box::new(editor_session::commands::OpenRecent {
                                    path: folder.to_string_lossy().into_owned().into(),
                                }),
                                cx,
                            ),
                            Entry::Command(id) => {
                                window.dispatch_action(Box::new(RunCommand::new(id)), cx)
                            }
                            _ => {}
                        });
                if entry.separated {
                    item = item.separated();
                }
                if let Some(tip) = entry.tip {
                    item = item.tip(tip);
                }
                item
            })
            .collect()
    }

    /// "What Kerf does without asking": the three modes, each with what it
    /// means in its tip.
    fn permissions_items(&self, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let current = self.session.read(cx).prefs.permissions;
        let this = cx.entity().downgrade();
        Permissions::ALL
            .iter()
            .map(|permissions| {
                let permissions = *permissions;
                let (label, line) = permission_words(permissions);
                let this = this.clone();
                ui::MenuItem::new(label)
                    .checked(permissions == current)
                    .tip(line)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |this, cx| this.set_permissions(permissions, cx));
                    })
            })
            .collect()
    }

    /// The models that can be chosen, Agent settings…, and whether a request
    /// carries the playhead time.
    fn model_items(&self, cx: &App) -> Vec<ui::MenuItem> {
        let session = self.session.read(cx);
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let current = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured.clone()
        };
        // Claude Code's aliases are chosen here. A key's model is the one
        // typed in Settings: it is said, and not offered as a choice.
        let choosable = provider == Provider::ClaudeCode;
        let stamped = session.prefs.send_playhead;
        let mut items: Vec<ui::MenuItem> = model_choices(provider, &configured)
            .into_iter()
            .filter(|model| choosable || *model == current)
            .map(|model| {
                let chosen = model == current;
                let session = self.session.clone();
                ui::MenuItem::new(model_label(&model))
                    .checked(chosen)
                    .disabled(!choosable)
                    .on_choose(move |_, cx| {
                        let model = model.clone();
                        session.update(cx, |session, cx| {
                            session.prefs.claude_code_model = if model == "default" {
                                String::new()
                            } else {
                                model
                            };
                            session.save_prefs();
                            cx.notify();
                        });
                    })
            })
            .collect();
        items.push(
            ui::MenuItem::new("Agent settings…")
                .separated()
                .on_choose(|window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("app.preferences")), cx)
                }),
        );
        let session = self.session.clone();
        items.push(
            ui::MenuItem::new("Send playhead time")
                .checked(stamped)
                .tip("A request says where the playhead is")
                .on_choose(move |_, cx| {
                    session.update(cx, |session, cx| {
                        session.prefs.send_playhead = !session.prefs.send_playhead;
                        session.save_prefs();
                        cx.notify();
                    });
                }),
        );
        items
    }
}

/// Esc puts a menu of the conversation away wherever the keys are (a press
/// on its chip leaves them where they were), before the box or the window
/// has it.
pub(super) fn escape_keys(window: &Window, cx: &mut Context<ChatView>) -> Subscription {
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
        let closed = this
            .update(cx, |this, cx| {
                let open = this.menu.take().is_some();
                if open {
                    this.escape_took_menu = true;
                    if let Some(menu) = this.kit_menu.take() {
                        menu.update(cx, |menu, cx| menu.close(window, cx));
                    }
                    cx.notify();
                }
                open
            })
            .unwrap_or(false);
        if closed {
            cx.stop_propagation();
        }
    })
}

/// A menu's box: a step up from what it opens over; a press outside closes
/// it at once.
pub(super) fn menu_frame(width: gpui::Pixels, cx: &Context<ChatView>) -> gpui::Div {
    div()
        .w(width)
        .p(px(theme::space::S4))
        .flex()
        .flex_col()
        .rounded(px(theme::radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))
}

/// What opens from a press, fading in over `FADE` as it rises
/// `MENU_RISE` into place, as the kit's menus do.
pub(super) fn fade_in(body: impl IntoElement, id: String) -> impl IntoElement {
    div().relative().child(body).with_animation(
        ElementId::Name(id.into()),
        motion::animation(scene::FADE),
        |menu, t| menu.opacity(t).top(px(scene::MENU_RISE * (1.0 - t))),
    )
}
