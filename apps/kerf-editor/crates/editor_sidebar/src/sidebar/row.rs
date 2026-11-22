//! A conversation's row: its title (a band of light passing over it while
//! Kerf works, or the field it is renamed in), the dot that says how it
//! stands, its ⋯ and right-click menu, its keys, and how it fades in.

use super::ages::{parse_time, relative_time};
use super::keys::{DeleteRow, OpenMenu, RenameRow, ROW_KEYS};
use super::list::{matches, RowState};
use super::{SidebarEvent, SidebarMenu, SidebarView};
use editor_state::model::{ConversationSummary, SummaryShow};
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui::{self, icon_button};
use gpui::{
    div, linear_color_stop, linear_gradient, prelude::*, px, AnimationExt, AnyElement, Context,
    Div, ElementId, MouseButton, MouseDownEvent, Rgba, SharedString, Stateful, Window,
};

/// How many times the light over a working title is drawn in a pass:
/// 30 a second, smooth to the eye without the sidebar drawn every frame.
pub use editor_ui::theme::motion::TITLE_SHIMMER_STEPS;

/// `phase` (0..1 of a pass) held to the step it is in.
pub fn shimmer_step(phase: f32) -> f32 {
    let steps = TITLE_SHIMMER_STEPS as f32;
    (phase.clamp(0.0, 1.0) * steps).floor().min(steps - 1.0) / steps
}

/// The title a rename keeps: what was typed, its spaces run together and
/// trimmed. None when it is empty or the same as before.
pub fn renamed(old: &str, typed: &str) -> Option<String> {
    let title = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty() && title != old).then_some(title)
}

/// The dot at a row's end: Kerf's while it works there, the accent's ink
/// for a proposal waiting or a turn done while you looked elsewhere, brick
/// for a last answer that failed; none while it rests.
fn row_dot(state: RowState) -> Option<Rgba> {
    match state {
        RowState::Working => Some(theme::activity::kerf()),
        RowState::Review | RowState::Done => Some(theme::accent_ink()),
        RowState::Failed => Some(theme::neg()),
        RowState::Idle => None,
    }
}

impl SidebarView {
    /// The light over a working title steps on from one timer the sidebar
    /// holds while Kerf works, the sidebar shows and motion is on: it draws
    /// the sidebar again `TITLE_SHIMMER_STEPS` times a pass. Followed as the
    /// session changes, as the sidebar is put away or comes back, and as it
    /// draws, which is when Reduce Motion is read (Settings turns it on or
    /// off with only the window drawn again to say so).
    pub(super) fn follow_shimmer(&mut self, cx: &mut Context<Self>) {
        let shimmering =
            self.shown && self.session.read(cx).state.work.busy() && !motion::reduced();
        if !shimmering {
            self.shimmer_tick = None;
        } else if self.shimmer_tick.is_none() {
            self.shimmer_tick = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(scene::TITLE_SHIMMER / TITLE_SHIMMER_STEPS)
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }));
        }
    }

    /// Edits the title of conversation `id` where it is.
    pub fn begin_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| {
                (
                    conversation.title.clone(),
                    conversation.label().to_owned(),
                    conversation.archived,
                )
            })
        else {
            return;
        };
        let (title, label, archived) = title;
        if self.renaming.is_some() {
            self.finish_rename(true, false, cx);
        }
        // Its row comes into sight to be renamed in: unfolded, unfiltered.
        if archived && !self.archived_open {
            self.archived_open = true;
            self.unfoldings += 1;
        }
        if !matches(&label, &self.query) {
            self.search.update(cx, |field, cx| field.clear(cx));
            self.query.clear();
        }
        self.menu = None;
        self.menu_return = None;
        self.renaming = Some(id.to_owned());
        // The whole title is chosen, to type over.
        self.rename.update(cx, |field, cx| {
            field.set_text(title, cx);
            field.choose_all(cx);
        });
        self.rename.read(cx).focus(window);
        cx.notify();
    }

    /// Ends a rename: `keep` sends the new title (an empty or unchanged one
    /// is not sent); `give_back` hands the keys back to the window.
    pub(super) fn finish_rename(&mut self, keep: bool, give_back: bool, cx: &mut Context<Self>) {
        let Some(id) = self.renaming.take() else {
            return;
        };
        if keep {
            let typed = self.rename.read(cx).text().to_owned();
            let old = self
                .session
                .read(cx)
                .state
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
                .map(|conversation| conversation.title.clone());
            if let Some(title) = old.and_then(|old| renamed(&old, &typed)) {
                self.session
                    .update(cx, |session, _| session.rename_conversation(&id, &title));
            }
        }
        if give_back {
            cx.emit(SidebarEvent::Done);
        }
        cx.notify();
    }

    /// A conversation's row, `index` among those listed with it, standing
    /// as `state` says, its tip saying how long ago it changed at `now`.
    pub(super) fn row(
        &self,
        conversation: &ConversationSummary,
        index: usize,
        state: RowState,
        now: i64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let session = self.session.read(cx);
        let chosen = session.conversation.as_deref() == Some(conversation.id.as_str());
        let working = state == RowState::Working;
        let id = conversation.id.clone();
        let menu = SidebarMenu::Row(id.clone());
        let menu_open = self.menu_is(&menu);
        let editing = self.renaming.as_deref() == Some(conversation.id.as_str());
        let title: SharedString = conversation.label().to_owned().into();
        // The whole title, and when it last changed, a hover away.
        let tip: SharedString = match parse_time(&conversation.updated) {
            Some(when) => format!("{title} · {}", relative_time(when, now)).into(),
            None => title.clone(),
        };
        let words = self.title_words(title, chosen, working, editing);
        let dot = row_dot(state);

        let row = Self::keyed(
            div().id(ElementId::Name(format!("conversation-{id}").into())),
            ROW_KEYS,
        )
        .group("row")
        .relative()
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .h(px(theme::height::H32))
        .px(px(10.0))
        .mx(px(theme::space::S8))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .when(chosen, |row| row.bg(theme::on()))
        .when(!chosen, |row| row.hover(|row| row.bg(theme::raised())))
        .child(words)
        .when_some(dot.filter(|_| !editing), |row, color| {
            row.child(ui::dot(color, 6.0))
        })
        // The whole title and when it last changed, the kit's tip, under
        // what the row holds (its ⋯ keeps its press).
        .child(ui::tip_zone(
            ElementId::Name(format!("conversation-tip-{id}").into()),
            (!editing && !menu_open).then(|| ui::Tip::new(tip.clone())),
        ))
        .when(!editing, |row| {
            row.child(Self::more_button(&id, &menu, menu_open, cx))
        });
        let row = Self::answering(row, &id, &menu, working, cx);
        let with_menu = div().relative().child(row).when(menu_open, |wrapper| {
            // Over the rows after it, and past the list's edge.
            wrapper.child(self.placed_menu(&menu, 30.0, 180.0, cx))
        });
        self.appearing(with_menu, conversation, index)
    }

    /// A row's title: in ink while it is the one open, with a band of light
    /// passing over it while Kerf works there (simply in ink under Reduce
    /// Motion or while the sidebar is put away); being renamed, the field
    /// in its place.
    fn title_words(
        &self,
        title: SharedString,
        chosen: bool,
        working: bool,
        editing: bool,
    ) -> AnyElement {
        let style = if chosen { text::SIDE_NAME } else { text::SIDE };
        let mut words = ui::line(
            title,
            style,
            if chosen { theme::ink() } else { theme::ink2() },
        )
        .flex_1()
        .min_w_0();
        if working && !motion::reduced() && self.shown {
            // A band of light passing over the title, then resting; drawn
            // again TITLE_SHIMMER_STEPS times a pass, not every frame.
            let phase = shimmer_step(motion::phase(self.shimmer_since, scene::TITLE_SHIMMER));
            if let Some(at) = motion::shimmer_at(phase, scene::TITLE_CROSS) {
                let light = theme::ink();
                let rest = theme::with_alpha(theme::ink2(), 0.0);
                words = words.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui::relative(at - 0.15))
                        .w(gpui::relative(0.3))
                        .bg(linear_gradient(
                            90.0,
                            linear_color_stop(rest, 0.0),
                            linear_color_stop(theme::with_alpha(light, 0.22), 0.5),
                        ))
                        .opacity(0.9),
                );
                words = words.relative();
            }
        } else if working {
            words = words.text_color(theme::ink());
        }
        if editing {
            // The field takes the title's place, its text where the title was.
            div()
                .flex_1()
                .min_w_0()
                .h(px(26.0))
                .flex()
                .items_center()
                .ml(px(-5.0))
                .px(px(theme::space::S4))
                .rounded(px(theme::radius::SMALL))
                .bg(theme::card())
                .border_1()
                .border_color(theme::accent())
                .child(self.rename.clone())
                .into_any_element()
        } else {
            words.into_any_element()
        }
    }

    /// The row's ⋯, seen while the pointer is over the row or its menu is
    /// open: pressed, the menu opens, or closes.
    fn more_button(id: &str, menu: &SidebarMenu, menu_open: bool, cx: &Context<Self>) -> Div {
        div()
            .invisible()
            .group_hover("row", |more| more.visible())
            .when(menu_open, |more| more.visible())
            .child(
                ui::capture_trigger(
                    ElementId::Name(format!("more-{id}").into()),
                    cx.listener({
                        let menu = menu.clone();
                        move |this, _: &MouseDownEvent, window, cx| {
                            this.toggle_menu(menu.clone(), false, window, cx)
                        }
                    }),
                )
                .child(
                    icon_button(
                        ElementId::Name(format!("more-button-{id}").into()),
                        Icon::More,
                    )
                    .size(22.0),
                ),
            )
    }

    /// What a conversation's row answers: a right-click opens its menu at
    /// the pointer, and Shift+F10 or the Menu key with the row holding the
    /// keys opens it under the row; F2 renames it; Delete asks to delete it,
    /// not while Kerf works in it (`working`); a press opens it.
    fn answering(
        row: Stateful<Div>,
        id: &str,
        menu: &SidebarMenu,
        working: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        row
            // A right-click opens its menu too; Shift+F10 or the Menu key with
            // the row holding the keys.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let menu = menu.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(menu.clone(), false, window, cx);
                        // At the pointer, kept inside the window.
                        this.menu_at = Some(event.position);
                    }
                }),
            )
            .on_action(cx.listener({
                let menu = menu.clone();
                move |this, _: &OpenMenu, window, cx| this.open_menu(menu.clone(), true, window, cx)
            }))
            .on_action(cx.listener({
                let id = id.to_owned();
                move |this, _: &RenameRow, window, cx| this.begin_rename(&id, window, cx)
            }))
            .on_action(cx.listener({
                let id = id.to_owned();
                move |_, _: &DeleteRow, _, cx| {
                    // Not while Kerf works in it; asked first, over the window.
                    if !working {
                        cx.emit(SidebarEvent::Delete(id.clone()));
                    }
                }
            }))
            .on_click(cx.listener({
                let id = id.to_owned();
                move |this, _, _, cx| {
                    if this.renaming.as_deref() == Some(id.as_str()) {
                        return;
                    }
                    this.menu_pressed_out(cx);
                    this.session.update(cx, |session, cx| {
                        session.open_conversation(Some(id.clone()));
                        cx.notify();
                    });
                    cx.emit(SidebarEvent::Chosen);
                }
            }))
    }

    /// A row and its menu as they come into the list: an archived one fades
    /// in where it is, one after another, as they unfold; one new to the
    /// list fades in where it is; the rest are there at once.
    fn appearing(
        &self,
        with_menu: Div,
        conversation: &ConversationSummary,
        index: usize,
    ) -> AnyElement {
        let id = &conversation.id;
        let arrived = self
            .arrivals
            .get(id)
            .filter(|at| at.elapsed() < scene::APPEAR)
            .copied();
        if conversation.archived {
            // Unfolding, each fades in where it is, one after another.
            let delay = scene::UNFOLD_STAGGER.as_secs_f32() * index as f32;
            let total = scene::APPEAR.as_secs_f32() + delay;
            let key = self.unfoldings;
            with_menu
                .with_animation(
                    ElementId::Name(format!("unfold-{id}-{key}").into()),
                    motion::animation(std::time::Duration::from_secs_f32(total)),
                    move |row, t| {
                        let start = if total > 0.0 { delay / total } else { 0.0 };
                        let local = ((t - start) / (1.0 - start).max(0.001)).clamp(0.0, 1.0);
                        row.opacity(local)
                    },
                )
                .into_any_element()
        } else if arrived.is_some() {
            // New to the list: it fades in where it is.
            with_menu
                .with_animation(
                    ElementId::Name(format!("arrive-{id}").into()),
                    motion::animation(scene::APPEAR),
                    |row, t| row.opacity(t),
                )
                .into_any_element()
        } else {
            with_menu.into_any_element()
        }
    }
}
