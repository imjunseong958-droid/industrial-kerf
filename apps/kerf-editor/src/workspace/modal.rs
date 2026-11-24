//! What one goes into over the window, which dims: Settings, the palette, a
//! shot's graph, Finish, a dialog; one at a time, placed by
//! [`ui::modal_bounds`] when it has a size of its own, its body rising into
//! place. And the questions that open there: deleting a conversation, the
//! words a sync needs, Focus Region.

use super::asked::{sync_params, sync_question, AskWords, Confirm};
use super::Workspace;
use editor_launcher::settings::{Page, SettingsEvent, SettingsView};
use editor_state::model::SummaryShow;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, size, AnimationExt, AnyElement, AnyView, App, Context, DismissEvent,
    Entity, EventEmitter, Focusable, MouseButton, Pixels, SharedString, Window,
};

impl Workspace {
    /// Opens Settings over the window, on `page` (None: where Settings
    /// opens by itself, Chat). Open already, it goes to `page`; anything
    /// else open over the window gives way to it.
    pub fn show_settings(
        &mut self,
        page: Option<Page>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self
            .modal
            .as_ref()
            .and_then(|(view, _)| view.clone().downcast::<SettingsView>().ok());
        if let Some(settings) = open {
            if let Some(page) = page {
                settings.update(cx, |settings, cx| settings.show_page(page, cx));
            }
            return;
        }
        let session = self.session.clone();
        let settings = cx.new(|cx| match page {
            Some(page) => SettingsView::new_at(session, page, window, cx),
            None => SettingsView::new(session, window, cx),
        });
        cx.subscribe_in(
            &settings,
            window,
            |this, settings, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Dismiss => this.close_modal_of(settings.entity_id(), window, cx),
            },
        )
        .detach();
        window.focus(&settings.focus_handle(cx));
        self.put_modal(settings.into(), window, cx);
    }

    /// Goes into `view` over the window: the window dims at once and the
    /// view rises into its middle. It gives the window back by emitting
    /// [`DismissEvent`]. One is open at a time.
    pub fn open_modal<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + Focusable + EventEmitter<DismissEvent>,
    {
        cx.subscribe_in(&view, window, |this, view, _: &DismissEvent, window, cx| {
            this.close_modal_of(view.entity_id(), window, cx)
        })
        .detach();
        window.focus(&view.focus_handle(cx));
        self.put_modal(view.into(), window, cx);
    }

    fn put_modal(&mut self, view: AnyView, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menus(window, cx);
        self.modals_opened += 1;
        self.modal = Some((view, self.modals_opened));
        cx.notify();
    }

    /// Gives the window back from `view`, if it is what is open over the
    /// window still: one another has taken the place of (Settings opened
    /// from the palette) has nothing to give back.
    fn close_modal_of(
        &mut self,
        view: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .modal
            .as_ref()
            .is_some_and(|(open, _)| open.entity_id() == view)
        {
            self.modal = None;
            // What it held goes with it: the keys settle afresh. Settings
            // may have chosen another keymap, which the toolbar's tips
            // name from now on.
            self.refresh_keys(cx);
            window.focus(&self.focus_handle);
            self.settle_focus(window, cx);
            cx.notify();
        }
    }

    /// Whether a dialog may open over the window now: nothing is open over
    /// it, or the palette is, which a command chosen in it replaces (its
    /// command comes before it has gone).
    pub(super) fn modal_gives_way(&self) -> bool {
        self.modal.as_ref().is_none_or(|(view, _)| {
            view.clone()
                .downcast::<editor_session::palette::CommandPalette>()
                .is_ok()
        })
    }

    /// Asks, over the window, before deleting conversation `id`; not while
    /// Kerf works in it.
    pub(super) fn confirm_delete(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal.is_some() || self.conversation_working(&id, cx) {
            return;
        }
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.label().to_owned())
        else {
            return;
        };
        let session = self.session.clone();
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("Delete “{title}”?"),
                Some("Its messages are removed from this project. This can't be undone."),
                "Delete",
                true,
                false,
                move |_, cx| {
                    session.update(cx, |session, _| session.delete_conversation(&id));
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }

    /// Edit ▸ Focus Region…: its dialog over the window for the chosen
    /// picture clip; while a proposal waits, a word that it is to be
    /// reviewed first.
    pub(super) fn focus_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_gives_way() {
            return;
        }
        match crate::focus::target(self.session.read(cx)) {
            Ok(target) => {
                let session = self.session.clone();
                let dialog =
                    cx.new(|cx| crate::focus::FocusRegion::new(session, target, window, cx));
                self.open_modal(dialog, window, cx);
            }
            Err(words) => self.session.update(cx, |session, cx| {
                session.state.notify(words, false);
                cx.notify();
            }),
        }
    }

    /// Asks over the window for the words sync command `id` needs (a
    /// commit's message, a branch's name, a pull request's title, the
    /// GitHub repository's name), then runs it with them.
    pub(super) fn ask_sync_words(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_gives_way() {
            return;
        }
        let session = self.session.read(cx);
        let branch = session
            .state
            .collab
            .as_ref()
            .and_then(|collab| collab.git.branch.as_deref())
            .unwrap_or_default()
            .to_owned();
        let folder = session.project_name();
        let Some(asked) = sync_question(id, &branch, &folder) else {
            return;
        };
        let session = self.session.clone();
        let command = id.to_owned();
        let view = cx.new(|cx| {
            AskWords::new(asked, window, cx, move |words, cx| {
                let params = sync_params(&command, words);
                session.update(cx, |session, cx| {
                    session.run_command(&command, params);
                    cx.notify();
                });
            })
        });
        self.open_modal(view, window, cx);
    }

    /// Opens `media`'s shot graph over the window (Open Graph, a clip's
    /// double-click, `--graph`).
    pub fn open_shot_graph(
        &mut self,
        media: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let graph = cx.new(|cx| {
            editor_making::making::ShotGraph::new(self.session.clone(), media, window, cx)
        });
        self.open_modal(graph, window, cx);
    }

    /// What is open over the window, on the dim. A panel of a known size
    /// (Settings, Finish) is placed by [`ui::modal_bounds`]: at most that
    /// size, else the window less `MODAL_MARGIN` all round, with
    /// `MODAL_ABOVE` of the height to spare above it. Any other keeps its
    /// own size, held to the same room and sat as high.
    pub(super) fn modal(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (view, opened) = self.modal.as_ref()?;
        let key = *opened;
        let viewport = window.viewport_size();
        let most = modal_most(view, cx);
        let margin = layout::MODAL_MARGIN * 2.0;
        let panel = div()
            .relative()
            .rounded(px(theme::radius::PANEL))
            .overflow_hidden()
            .border_1()
            .border_color(theme::line2())
            .shadow_2xl()
            .child(view.clone());
        let panel = match most {
            Some(most) => {
                let placed = ui::modal_bounds(most, viewport);
                div()
                    .absolute()
                    .left(placed.origin.x)
                    .top(placed.origin.y)
                    .w(placed.size.width)
                    .h(placed.size.height)
                    .child(panel.size_full())
            }
            None => div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                // MODAL_ABOVE of the height to spare above it, the rest below.
                .child(spacer(layout::MODAL_ABOVE))
                .child(
                    panel
                        .flex_none()
                        .max_w(viewport.width - px(margin))
                        .max_h(viewport.height - px(margin)),
                )
                .child(spacer(1.0 - layout::MODAL_ABOVE)),
        };
        Some(
            div()
                .id("scrim")
                .absolute()
                .inset_0()
                // The window dims at once.
                .bg(theme::with_alpha(theme::scrim(), scene::SCRIM))
                // Nothing under the dim answers the pointer, not even its hover.
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    // Its body comes up into place.
                    panel.with_animation(
                        ("settings-rise", key),
                        motion::animation(scene::PAGE),
                        |body, t| {
                            body.mt(px(scene::PAGE_RISE * (1.0 - t)))
                                .opacity(0.2 + 0.8 * t)
                        },
                    ),
                )
                .into_any_element(),
        )
    }
}

/// The most a panel that fills what it is given is over the window:
/// Settings and Finish, at their largest (`settings::MOST`, `finish::MOST`).
fn modal_most(view: &AnyView, _cx: &App) -> Option<gpui::Size<Pixels>> {
    let view = view.clone();
    let most = if view.clone().downcast::<SettingsView>().is_ok() {
        editor_launcher::settings::MOST
    } else if view.downcast::<editor_finish::finish::FinishView>().is_ok() {
        editor_finish::finish::MOST
    } else {
        return None;
    };
    Some(size(px(most.0), px(most.1)))
}

/// Room that grows by `weight` against the others in its column.
fn spacer(weight: f32) -> gpui::Div {
    let mut spacer = div().flex_basis(px(0.0)).min_h_0();
    spacer.style().flex_grow = Some(weight);
    spacer
}
