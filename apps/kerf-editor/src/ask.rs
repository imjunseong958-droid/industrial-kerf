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

use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::session::{AskScope, Session};
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnimationExt, App, Bounds, Context,
    DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Pixels, Point, Size, Subscription,
    Window,
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

/// What the Scope button says: "This clip · <name>" or "Whole sequence".
pub fn scope_words(scope: AskScope, clip: Option<&str>) -> String {
    match (scope, clip) {
        (AskScope::Clip, Some(clip)) => format!("This clip · {clip}"),
        _ => "Whole sequence".to_owned(),
    }
}

pub struct AskView {
    session: Entity<Session>,
    anchor: Bounds<Pixels>,
    /// "Sequence at 00:00:17:08".
    title: String,
    /// The chosen clip's name, when one is chosen.
    clip: Option<String>,
    scope: AskScope,
    scope_menu: bool,
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
            scope_menu: false,
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

    fn set_scope(&mut self, scope: AskScope, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = if self.clip.is_some() {
            scope
        } else {
            AskScope::Sequence
        };
        self.scope_menu = false;
        // Back to what is being written.
        self.input.read(cx).focus(window);
        cx.notify();
    }

    fn scope_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let words = scope_words(self.scope, self.clip.as_deref());
        let choosable = self.clip.is_some();
        let button = theme::type_style(div(), text::CONTROL)
            .id("ask-scope")
            .flex()
            .items_center()
            .gap(px(2.0))
            .h(px(24.0))
            .pl(px(8.0))
            .pr(px(4.0))
            .rounded(px(theme::radius::CONTROL))
            .text_color(if choosable {
                theme::ink()
            } else {
                theme::ink2()
            })
            .child(words)
            .when(choosable, |button| {
                button
                    .cursor_pointer()
                    .hover(|button| button.bg(theme::hover2()))
                    .child(Icon::ChevronDown.view(12.0, theme::ink3()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scope_menu = !this.scope_menu;
                        cx.notify();
                    }))
            })
            .tooltip(ui::tooltip(ui::Tip::new("What Kerf looks at")));
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(ui::label("Scope", text::CONTROL, theme::ink2()))
            .child(button)
    }

    /// The Scope menu, laid over the card under its button: inside the
    /// card, so a press on it is no press elsewhere, and over the field.
    fn scope_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.scope_menu {
            return None;
        }
        let clip = self.clip.clone().unwrap_or_default();
        let choices = [
            (AskScope::Clip, format!("This clip · {clip}")),
            (AskScope::Sequence, "Whole sequence".to_owned()),
        ];
        let rows = choices.into_iter().map(|(scope, words)| {
            let chosen = scope == self.scope;
            div()
                .id(gpui::ElementId::Name(
                    format!("ask-scope-{}", scope.wire()).into(),
                ))
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(30.0))
                .px(px(10.0))
                .rounded(px(theme::radius::CONTROL))
                .cursor_pointer()
                .hover(|row| row.bg(theme::popup_hover()))
                .child(ui::label(words, text::MENU_ITEM, theme::ink()).flex_1())
                .child(if chosen {
                    Icon::Check.view(14.0, theme::ink()).into_any_element()
                } else {
                    crate::icons::spacer(14.0).into_any_element()
                })
                .on_click(cx.listener(move |this, _, window, cx| this.set_scope(scope, window, cx)))
        });
        Some(
            div()
                .absolute()
                .left(px(SCOPE_MENU_AT.0))
                .top(px(SCOPE_MENU_AT.1))
                .w(px(280.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .rounded(px(theme::radius::CARD))
                .bg(theme::popup())
                .border_1()
                .border_color(theme::line2())
                .shadow_lg()
                .occlude()
                .children(rows)
                .with_animation(
                    "ask-scope-menu",
                    crate::motion::animation(scene::FADE),
                    |menu, t| menu.opacity(t),
                ),
        )
    }
}

impl Render for AskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let at = place(self.anchor, window.viewport_size());
        let typing = self.input.read(cx).focus_handle(cx).is_focused(window);
        let empty = self.input.read(cx).text().trim().is_empty();
        let busy = self.session.read(cx).state.work.busy();
        let menu = self.scope_menu(cx);
        let card = div()
            .id("ask-popover")
            .track_focus(&self.focus_handle)
            .relative()
            .w(px(WIDTH))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
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
            .child(self.scope_row(cx))
            .child(
                div()
                    .h(px(56.0))
                    .px(px(8.0))
                    .py(px(6.0))
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
                    .gap(px(8.0))
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
            .with_animation(
                "ask-popover-open",
                crate::motion::animation(scene::FADE),
                |card, t| card.opacity(t),
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
