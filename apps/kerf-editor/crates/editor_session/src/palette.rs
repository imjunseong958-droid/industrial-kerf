//! The command palette: every command of the registry, found by typing a
//! few letters of it, with its shortcut. On macOS the menu bar holds the
//! same commands; elsewhere the palette is where they all are. One that
//! cannot run now is greyed, says why, and does nothing when chosen.

use crate::commands::{self, RunCommand};
use crate::session::Session;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::theme::{self, text};
use editor_ui::ui;
use gpui::{
    actions, div, prelude::*, px, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, ScrollHandle, SharedString, Subscription, Window,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

actions!(palette, [OpenPalette, Previous, Next]);

/// How a recent project's row's id starts in the palette; its path follows.
const RECENT: &str = "file.openRecent:";

/// What a recent project's row says it is under, where a command's menu is.
pub const RECENT_MENU: &str = "File ▸ Open Recent";

/// Why a recent project whose folder is gone cannot be opened.
pub const RECENT_GONE: &str = "Its folder is no longer there";

/// File ▸ Open Recent's rows as the palette lists them after the
/// registry's commands (off macOS the palette is where the menus are):
/// each recent project by its folder's name under [`RECENT_MENU`], newest
/// first, the one open here ticked as in the menu, with whether its folder
/// is gone. Chosen, one opens as Open Recent does ([`recent_path`]).
pub fn recent_commands(
    recent: &[PathBuf],
    current: Option<&Path>,
) -> Vec<(commands::Command, bool)> {
    commands::recent_rows(recent, current)
        .into_iter()
        .map(|row| {
            let label = if row.current {
                format!("✓ {}", row.name)
            } else {
                row.name.clone()
            };
            (
                commands::Command {
                    id: format!("{RECENT}{}", row.path.to_string_lossy()),
                    label,
                    menu: RECENT_MENU.to_owned(),
                    group: 0,
                    scope: "window".to_owned(),
                    keys: Vec::new(),
                    checkable: false,
                },
                row.missing,
            )
        })
        .collect()
}

/// The project a row of [`recent_commands`] opens, by its id; None for a
/// command of the registry.
pub fn recent_path(id: &str) -> Option<&str> {
    id.strip_prefix(RECENT)
}

/// The widest a command's reason (or menu) is before it is cut.
const REASON_MOST: f32 = 280.0;

/// The key context the palette's list keys answer in.
pub const CONTEXT: &str = "CommandPalette";

pub struct CommandPalette {
    query: Entity<TextInput>,
    commands: Vec<commands::Command>,
    /// Each command's id with why it cannot run now, or null when it can,
    /// as the palette opened.
    states: Value,
    chosen: usize,
    scroll: ScrollHandle,
    /// The switches on, and the tool in use, as the window has them: their
    /// rows are ticked ([`CommandPalette::with_checked`]).
    checked: BTreeSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for CommandPalette {}

/// Whether `command` answers `query`: every word of the query is in its
/// label, its menu or its id, in any case.
pub fn matches(command: &commands::Command, query: &str) -> bool {
    let haystack = format!("{} {} {}", command.label, command.menu, command.id).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

impl CommandPalette {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // What another window chose since (a provider, a key kept in the
        // settings file), so Ask is said to be available as it is.
        session.update(cx, |session, _| session.prefs.refresh());
        let keymap = session.read(cx).prefs.keymap.wire();
        let mut commands = commands::catalog(keymap);
        let cloud = crate::session::cloud(cx).cloned();
        let mut states = session.read(cx).command_states(cloud.as_ref());
        // The recent projects, as File ▸ Open Recent lists them; one whose
        // folder is gone says so.
        let recent = {
            let session = session.read(cx);
            recent_commands(&session.prefs.recent, session.project.as_deref())
        };
        for (command, missing) in recent {
            if missing {
                if let Some(states) = states.as_object_mut() {
                    states.insert(command.id.clone(), Value::from(RECENT_GONE));
                }
            }
            commands.push(command);
        }
        let query = cx.new(|cx| TextInput::new(cx, "Find a command…").with_style(text::MENU_ITEM));
        query.read(cx).focus(window);
        let subscriptions =
            vec![
                cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Changed => {
                            this.chosen = 0;
                            cx.notify();
                        }
                        InputEvent::Submit => this.run(window, cx),
                        InputEvent::Cancel => cx.emit(DismissEvent),
                    }
                }),
            ];
        Self {
            query,
            commands,
            states,
            chosen: 0,
            scroll: ScrollHandle::new(),
            checked: BTreeSet::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Ticks the rows of `checked`, the registry's checkable commands that
    /// are on in the window (Safe Zones, Follow Kerf, the tool in use). The
    /// workspace gives them as it opens the palette.
    pub fn with_checked(mut self, checked: BTreeSet<String>) -> Self {
        self.checked = checked;
        self
    }

    fn found(&self, cx: &gpui::App) -> Vec<&commands::Command> {
        let query = self.query.read(cx).text();
        self.commands
            .iter()
            .filter(|command| matches(command, query))
            .collect()
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self
            .found(cx)
            .get(self.chosen)
            .map(|command| command.id.clone())
        else {
            return;
        };
        self.run_id(id, window, cx);
    }

    fn run_id(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // One that cannot run now says why in its row, and stays.
        if unavailable(&self.states, &id).is_some() {
            return;
        }
        // Dispatched from here, where the workspace hears it, before giving the window back.
        match recent_path(&id) {
            Some(path) => window.dispatch_action(
                Box::new(commands::OpenRecent {
                    path: path.to_owned().into(),
                }),
                cx,
            ),
            None => window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx),
        }
        cx.emit(DismissEvent);
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.found(cx).len();
        if count == 0 {
            return;
        }
        self.chosen = (self.chosen as isize + by).rem_euclid(count as isize) as usize;
        self.scroll.scroll_to_item(self.chosen);
        cx.notify();
    }
}

/// The shortcut `command`'s row shows: its first, as the system writes it
/// and the window binds it ("⌘E" on a Mac; "Ctrl+R" elsewhere for a
/// keymap's "Meta+R"). Empty when it has none.
pub fn shortcut(command: &commands::Command) -> String {
    command
        .keys
        .first()
        .map(|keys| editor_state::shortcuts::shown_keys(keys))
        .unwrap_or_default()
}

/// Why `id` cannot run, as `states` (`kerf_bridge::commands_states`) says;
/// None when it can, or when the registry says nothing of it.
pub fn unavailable<'a>(states: &'a Value, id: &str) -> Option<&'a str> {
    states.get(id).and_then(Value::as_str)
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let found: Vec<(String, String, String, String, Option<String>, bool)> = self
            .found(cx)
            .into_iter()
            .map(|command| {
                (
                    command.id.clone(),
                    command.label.clone(),
                    command.menu.clone(),
                    shortcut(command),
                    unavailable(&self.states, &command.id).map(str::to_owned),
                    command.checkable && self.checked.contains(&command.id),
                )
            })
            .collect();
        let chosen = self.chosen.min(found.len().saturating_sub(1));
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            // A press on the dim around it gives the window back, and does
            // nothing to what lies under the dim.
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(DismissEvent);
            }))
            .w(px(560.0))
            .max_h(px(460.0))
            .flex()
            .flex_col()
            .bg(theme::popup())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .px(px(14.0))
                    .h(px(46.0))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(editor_ui::icons::Icon::Search.view(16.0, theme::ink3()))
                    .child(self.query.clone()),
            )
            .child(
                div()
                    .id("palette-commands")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(theme::space::S6))
                    .when(found.is_empty(), |list| {
                        list.child(div().p(px(10.0)).child(ui::label(
                            "No command says that.",
                            text::MENU_QUIET,
                            theme::ink3(),
                        )))
                    })
                    .children(found.into_iter().enumerate().map(
                        |(index, (id, label, menu, shortcut, reason, checked))| {
                            let shortcut: SharedString = shortcut.into();
                            let able = reason.is_none();
                            div()
                                .id(ElementId::Name(format!("palette-{id}").into()))
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .h(px(32.0))
                                .px(px(10.0))
                                .rounded(px(theme::radius::CONTROL))
                                .when(able, |row| row.cursor_pointer())
                                .when(index == chosen, |row| row.bg(theme::popup_hover()))
                                .when(able, |row| row.hover(|row| row.bg(theme::popup_hover())))
                                .child(
                                    ui::line(
                                        label,
                                        text::MENU_ITEM,
                                        if able { theme::ink() } else { theme::dis() },
                                    )
                                    .flex_1()
                                    .min_w_0(),
                                )
                                // On: ticked, as its menu row is.
                                .when(checked, |row| {
                                    row.child(editor_ui::icons::Icon::Check.view(
                                        14.0,
                                        if able { theme::ink2() } else { theme::dis() },
                                    ))
                                })
                                .child(
                                    // Why it cannot run, where its menu is said otherwise.
                                    ui::line(
                                        reason.unwrap_or(menu),
                                        text::MENU_QUIET,
                                        theme::ink3(),
                                    )
                                    .flex_none()
                                    .max_w(px(REASON_MOST)),
                                )
                                .child(ui::label(
                                    shortcut,
                                    text::MONO_SMALL,
                                    if able { theme::ink2() } else { theme::dis() },
                                ))
                                .when(able, |row| {
                                    row.on_click(cx.listener(move |this, _, window, cx| {
                                        this.run_id(id.clone(), window, cx)
                                    }))
                                })
                        },
                    )),
            )
    }
}
