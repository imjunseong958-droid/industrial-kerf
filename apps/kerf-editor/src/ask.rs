//! The Ask popover (Ask Kerf…, ⌘J): a card over the chosen clip, or over
//! the timeline at the playhead, to ask Kerf about it without the
//! conversation in view.
//!
//! ```text
//!   Sequence at 00:00:17:08
//!   Scope  [This clip · A019_C001 ⌄]
//!   ┌ Tell Kerf what to change ───────────┐
//!   └──────────────────────────────────────┘
//!   ↩ Send · ⇧↩ New line                [Ask]
//! ```
//!
//! Return sends; Shift-Return starts a line; Esc or a press elsewhere
//! closes it. With a clip chosen the request can stay on the clip or take
//! in the whole sequence around it; without one it is about the sequence
//! at the playhead. The request is stamped at the start of what it is
//! about ([`Session::ask_about`]). It fades in over `FADE`, as a menu does.
//!
//! Scope is the kit's menu ([`ui::Menu`]): Up and Down, a letter to its
//! row, Return or Space to choose; Esc closes only the menu, and what was
//! typed stays. Its button is a Tab stop (Tab goes round the field, Scope
//! and Ask only) that opens it with Return or Space.

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::ClipShow;
use crate::session::{AskScope, Session};
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnimationExt, App, Bounds, ClickEvent, Context,
    DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseDownEvent,
    Pixels, Point, Size, Subscription, Window,
};

/// How wide the card is, and about how tall: it opens above what it is
/// anchored to while that leaves it room, else below.
/// How far it keeps from what it is anchored to, and from the window's
/// edges. Where the Scope menu opens in the card: under its button.
pub use crate::theme::layout::{
    ASK_GAP as GAP, ASK_HEIGHT as HEIGHT, ASK_SCOPE_MENU_AT as SCOPE_MENU_AT, ASK_WIDTH as WIDTH,
};

/// Where the card's top-left goes for `anchor` (in the window, the chosen
/// clip or the timeline at the playhead) in a window of `window` size: over
/// the anchor's middle, `GAP` above it, kept `GAP` inside the window's
/// sides; below it where there is no room above.
pub fn place(anchor: Bounds<Pixels>, window: Size<Pixels>) -> Point<Pixels> {
    let width = f32::from(window.width);
    let middle = f32::from(anchor.origin.x) + f32::from(anchor.size.width) / 2.0;
    let most = (width - WIDTH - GAP).max(GAP);
    let x = (middle - WIDTH / 2.0).clamp(GAP, most);
    let top = f32::from(anchor.origin.y);
    let above = top - GAP - HEIGHT;
    let y = if above >= GAP {
        above
    } else {
        top + f32::from(anchor.size.height).min(16.0) + GAP
    };
    point(px(x), px(y))
}

/// How wide the Scope menu is.
pub use crate::theme::layout::ASK_SCOPE_MENU_WIDTH as SCOPE_MENU_WIDTH;

/// What the Scope button says: "This clip · <name>" or "Whole sequence".
pub fn scope_words(scope: AskScope, clip: Option<&str>) -> String {
    match (scope, clip) {
        (AskScope::Clip, Some(clip)) => format!("This clip · {clip}"),
        _ => "Whole sequence".to_owned(),
    }
}

/// The Scope menu's rows: "This clip · <name>" while a clip is chosen,
/// then "Whole sequence".
pub fn scope_choices(clip: Option<&str>) -> Vec<(AskScope, String)> {
    let mut choices = Vec::new();
    if let Some(clip) = clip {
        choices.push((AskScope::Clip, scope_words(AskScope::Clip, Some(clip))));
    }
    choices.push((AskScope::Sequence, scope_words(AskScope::Sequence, clip)));
    choices
}

pub struct AskView {
    session: Entity<Session>,
    anchor: Bounds<Pixels>,
    /// "Sequence at 00:00:17:08".
    title: String,
    /// The chosen clip's name, when one is chosen.
    clip: Option<String>,
    scope: AskScope,
    /// The Scope menu, while it is open.
    scope_menu: Option<Entity<ui::Menu>>,
    /// The Scope button's, a Tab stop.
    scope_focus: FocusHandle,
    /// Return or Space went down on the Scope button: as it comes up, the
    /// menu opens.
    scope_armed: bool,
    input: Entity<TextInput>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for AskView {}

impl Focusable for AskView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AskView {
    /// The popover over `anchor`, about the clip chosen now (if any) at the
    /// playhead's time now.
    pub fn new(
        session: Entity<Session>,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (title, clip) = {
            let session = session.read(cx);
            let timeline = session.timeline();
            let title = format!(
                "Sequence at {}",
                crate::model::timecode(session.state.position, timeline.rate())
            );
            let clip = session
                .selection
                .first()
                .and_then(|id| timeline.clip(id))
                .map(|(_, clip)| clip.label().to_owned());
            (title, clip)
        };
        let input = cx.new(|cx| {
            TextInput::new(cx, "Tell Kerf what to change")
                .multiline()
                .max_height(px(56.0 - 12.0))
        });
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Submit => this.submit(cx),
                InputEvent::Cancel => cx.emit(DismissEvent),
                InputEvent::Changed => cx.notify(),
            },
        )];
        Self {
            session,
            anchor,
            title,
            scope: if clip.is_some() {
                AskScope::Clip
            } else {
                AskScope::Sequence
            },
            clip,
            scope_menu: None,
            scope_focus: cx.focus_handle().tab_stop(true),
            scope_armed: false,
            input,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Gives the field the keys.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.input.read(cx).focus(window);
    }

    pub fn scope(&self) -> AskScope {
        self.scope
    }

    /// The request goes, about what the scope says, and the popover closes;
    /// nothing written, or Kerf at work, sends nothing.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let scope = self.scope;
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.ask_about(&text, scope);
            cx.notify();
            sent
        });
        if sent {
            cx.emit(DismissEvent);
        }
    }

    /// The scope chosen; the menu has gone, and the keys are back where
    /// they were before it opened (the field, or the Scope button).
    fn set_scope(&mut self, scope: AskScope, cx: &mut Context<Self>) {
        self.scope = if self.clip.is_some() {
            scope
        } else {
            AskScope::Sequence
        };
        cx.notify();
    }

    /// Opens the Scope menu (from the keys: its first row lit), or puts it
    /// away when it is open. Without a clip there is nothing to choose.
    fn toggle_scope_menu(&mut self, keys: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.scope_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            cx.notify();
            return;
        }
        if self.clip.is_none() {
            return;
        }
        let this = cx.weak_entity();
        let items: Vec<ui::MenuItem> = scope_choices(self.clip.as_deref())
            .into_iter()
            .map(|(scope, words)| {
                let this = this.clone();
                ui::MenuItem::new(words)
                    .checked(scope == self.scope)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |this, cx| this.set_scope(scope, cx));
                    })
            })
            .collect();
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(SCOPE_MENU_WIDTH);
            if keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this.scope_menu.as_ref() == Some(&menu) {
                this.scope_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.scope_menu = Some(menu);
        cx.notify();
    }

    fn scope_row(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = scope_words(self.scope, self.clip.as_deref());
        let choosable = self.clip.is_some();
        let open = self.scope_menu.is_some();
        let button = if choosable {
            // The kit's trigger: pressed again, its own menu goes rather
            // than opening anew. A Tab stop of its own, ringed while it has
            // the keys; Return or Space opens the menu; a press leaves the
            // keys where they were.
            ui::trigger(
                "ask-scope",
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.toggle_scope_menu(false, window, cx)
                }),
            )
            .relative()
            .track_focus(&self.scope_focus)
            .key_context(ui::CONTROL)
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
            .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                let keystroke = &event.keystroke;
                if matches!(keystroke.key.as_str(), "enter" | "space")
                    && !keystroke.modifiers.modified()
                {
                    this.scope_armed = true;
                }
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.is_keyboard() && std::mem::take(&mut this.scope_armed) {
                    this.toggle_scope_menu(true, window, cx);
                }
            }))
            .cursor_pointer()
            .when(open, |button| button.bg(theme::on()))
            .when(!open, |button| {
                button.hover(|button| button.bg(theme::hover2()))
            })
            .when(self.scope_focus.is_focused(window), |button| {
                button.child(ui::ring(theme::radius::CONTROL))
            })
        } else {
            div().id("ask-scope")
        };
        let button = theme::type_style(button, text::CONTROL)
            .flex()
            .items_center()
            .gap(px(theme::space::S2))
            .h(px(24.0))
            .pl(px(theme::space::S8))
            .pr(px(theme::space::S4))
            .rounded(px(theme::radius::CONTROL))
            .text_color(if choosable {
                theme::ink()
            } else {
                theme::ink2()
            })
            .child(words)
            .when(choosable, |button| {
                button.child(Icon::ChevronDown.view(12.0, theme::ink3()))
            });
        // Its menu says all the tip would, so the tip waits while it is open.
        let tip = (!open).then(|| ui::Tip::new("What Kerf looks at"));
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .child(ui::label("Scope", text::CONTROL, theme::ink2()))
            .child(ui::tip_on(button, tip))
    }

    /// The Scope menu, laid over the card under its button: inside the
    /// card, so a press on it is no press elsewhere, and over the field.
    /// The kit's menu fades in over `FADE`, rising `MENU_RISE`.
    fn scope_menu(&self) -> Option<impl IntoElement> {
        let menu = self.scope_menu.clone()?;
        Some(
            div()
                .absolute()
                .left(px(SCOPE_MENU_AT.0))
                .top(px(SCOPE_MENU_AT.1))
                .child(menu),
        )
    }
}

impl Render for AskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let at = place(self.anchor, window.viewport_size());
        let typing = self.input.read(cx).focus_handle(cx).is_focused(window);
        let empty = self.input.read(cx).text().trim().is_empty();
        let busy = self.session.read(cx).state.work.busy();
        let menu = self.scope_menu();
        let card = div()
            .id("ask-popover")
            .track_focus(&self.focus_handle)
            // Tab goes round what is in it only: the field, Scope, Ask.
            .key_context(ui::PANEL)
            // Esc on Scope or Ask closes it, as it does from the field (an
            // open Scope menu takes Esc itself, and only it goes).
            .on_action(cx.listener(|_, _: &crate::commands::Escape, _, cx| cx.emit(DismissEvent)))
            .relative()
            .w(px(WIDTH))
            .p(px(theme::space::S12))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .rounded(px(theme::radius::CARD))
            .bg(theme::card())
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            .occlude()
            // A press anywhere on it leaves the keys in the field.
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                this.input.read(cx).focus(window);
            }))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .child(ui::label(self.title.clone(), text::BODY_NAME, theme::ink()))
            .child(self.scope_row(window, cx))
            .child(
                div()
                    .h(px(56.0))
                    .px(px(theme::space::S8))
                    .py(px(theme::space::S6))
                    .rounded(px(theme::radius::CONTROL))
                    .bg(theme::raised())
                    .border_1()
                    .border_color(if typing {
                        theme::accent()
                    } else {
                        theme::line2()
                    })
                    .overflow_hidden()
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::label("↩ Send · ⇧↩ New line", text::META, theme::ink3())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(
                        ui::button("ask-send", "Ask")
                            .primary()
                            .disabled(empty || busy)
                            .reason(if busy {
                                "Kerf is working; ask when it is done"
                            } else {
                                "Write what to change"
                            })
                            .tooltip("Ask")
                            .key("↩")
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
            .children(menu)
            // It rises into place as it fades in, as the kit's menus do.
            .with_animation(
                "ask-popover-open",
                crate::motion::animation(scene::FADE),
                |card, t| card.opacity(t).top(px(scene::MENU_RISE * (1.0 - t))),
            );
        deferred(
            anchored()
                .position(at)
                .snap_to_window_with_margin(px(GAP))
                .child(card),
        )
        .with_priority(3)
    }
}
