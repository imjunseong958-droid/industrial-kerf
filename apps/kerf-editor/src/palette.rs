//! The command palette: every command of the registry, found by typing a
//! few letters of it, with its shortcut. On macOS the menu bar holds the
//! same commands; elsewhere the palette is where they all are. One that
//! cannot run now is greyed, says why, and does nothing when chosen.

use crate::commands::{self, RunCommand};
use crate::input::{InputEvent, TextInput};
use crate::session::Session;
use crate::theme::{self, text};
use crate::ui;
use gpui::{
    actions, div, prelude::*, px, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, ScrollHandle, SharedString, Subscription, Window,
};
use serde_json::Value;

actions!(palette, [OpenPalette, Previous, Next]);

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
        let commands = commands::catalog(keymap);
        let cloud = crate::app::cloud(cx).cloned();
        let states = session.read(cx).command_states(cloud.as_ref());
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
            _subscriptions: subscriptions,
        }
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
        window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx);
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
        let found: Vec<(String, String, String, Vec<String>, Option<String>)> = self
            .found(cx)
            .into_iter()
            .map(|command| {
                (
                    command.id.clone(),
                    command.label.clone(),
                    command.menu.clone(),
                    command.keys.clone(),
                    unavailable(&self.states, &command.id).map(str::to_owned),
                )
            })
            .collect();
        let chosen = self.chosen.min(found.len().saturating_sub(1));
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .w(px(560.0))
            .max_h(px(460.0))
            .flex()
            .flex_col()
            .bg(theme::popup())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(14.0))
                    .h(px(46.0))
                    .border_b_1()
                    .border_color(theme::line())
                    .child(crate::icons::Icon::Search.view(16.0, theme::ink3()))
                    .child(self.query.clone()),
            )
            .child(
                div()
                    .id("palette-commands")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(6.0))
                    .when(found.is_empty(), |list| {
                        list.child(div().p(px(10.0)).child(ui::label(
                            "No command says that.",
                            text::MENU_QUIET,
                            theme::ink3(),
                        )))
                    })
                    .children(found.into_iter().enumerate().map(
                        |(index, (id, label, menu, keys, reason))| {
                            let shortcut: SharedString =
                                keys.first().cloned().unwrap_or_default().into();
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
