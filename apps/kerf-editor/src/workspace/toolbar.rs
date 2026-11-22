//! The toolbar over the main column: Sidebar and ⋯ at the window's leading
//! edge while the sidebar is away, the title, the format bank, the sync chip,
//! what renders, Undo, Redo, Finish, Export (a split button, with its menu
//! under its chevron), Follow Kerf, Media and the panel's button. Its own Tab
//! stops ([`toolbar_stop`]) and its tips, with the keys or why not.

use super::Workspace;
use editor_finish::export::{ExportEvent, ExportView};
use editor_session::commands::RunCommand;
use editor_state::delivery::Quality;
use editor_state::model::RenderJobShow;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui::{self, icon_button};
use gpui::{
    div, prelude::*, px, AnimationExt, ClickEvent, Context, Div, Entity, FocusHandle,
    MouseDownEvent, Stateful, Window,
};
use kerf_bridge::Command;

impl Workspace {
    fn toggle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.export.take().is_some() {
            cx.notify();
            return;
        }
        let export = cx.new(|cx| ExportView::new(self.session.clone(), window, cx));
        cx.subscribe(&export, |this, _, event: &ExportEvent, cx| match event {
            ExportEvent::Dismiss => {
                this.export = None;
                cx.notify();
            }
        })
        .detach();
        self.session.update(cx, |session, _| {
            session.send(Command::RenderQueue);
        });
        self.export = Some(export);
        cx.notify();
    }

    /// The toolbar over the main column: at the window's leading edge
    /// (`leading`, the sidebar away) Sidebar and ⋯ first; the title; the
    /// format bank and the sync chip; what renders; Undo, Redo, Finish and
    /// Export; Follow Kerf, Media and the panel's button.
    pub(super) fn toolbar(
        &self,
        leading: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.session.read(cx);
        let follow = session.prefs.follow_kerf;
        // What renders, queued exports too ("Draft export · 42%").
        let rendering = running_words(&session.state.renders, &session.state.preview);
        let quality = session.export_quality;
        // What can be done now, as the registry says it (none of these is
        // Ask, so the keychain is not asked).
        let states = kerf_bridge::commands_states(&session.command_context());
        let reason =
            |id: &str| editor_session::palette::unavailable(&states, id).map(str::to_owned);
        let (undo_reason, redo_reason, export_reason) = (
            reason("edit.undo"),
            reason("edit.redo"),
            reason("export.master"),
        );
        let tip = |words: &str, id: &str, why: Option<&String>| -> (String, Option<String>) {
            match why {
                Some(why) => (format!("{words} · {why}"), None),
                None => (words.to_owned(), self.shown_key(id)),
            }
        };
        let undo_tip = tip("Undo", "edit.undo", undo_reason.as_ref());
        let redo_tip = tip("Redo", "edit.redo", redo_reason.as_ref());
        // The master's key goes with it; a draft has none of its own.
        let export_tip = (
            editor_finish::export::export_tip(quality, export_reason.as_deref()),
            (quality == Quality::Master && export_reason.is_none())
                .then(|| self.shown_key("export.master"))
                .flatten(),
        );
        let follow_key = self.shown_key("view.followKerf");
        let media_key = self.shown_key("window.project");
        let export_open = self.export.is_some();
        // At the window's leading edge on macOS, the window buttons' room.
        let lead = toolbar_places(leading, editor_ui::chrome::window_buttons_room(window)).lead;
        div()
            .h(px(layout::TOOLBAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .pl(px(lead))
            .pr(px(theme::space::S8))
            .when(leading, |bar| self.leading_controls(bar, cx))
            .child(self.title_row(window, cx))
            // Its empty stretch: on macOS, where the toolbar is the title
            // bar, it moves the window and zooms it.
            .child(editor_ui::chrome::title_bar_stretch(
                div().flex_1().h_full(),
            ))
            .child(self.formats.clone())
            .child(self.collab.clone())
            .when_some(rendering, |bar, rendering| {
                bar.child(
                    ui::chip(rendering, theme::activity::gpu())
                        .child(ui::dot(theme::activity::gpu(), 6.0)),
                )
            })
            .child(
                kit_tip(
                    icon_button("undo", Icon::Undo),
                    undo_tip,
                    undo_reason.is_some(),
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("edit.undo")), cx)
                }),
            )
            .child(
                kit_tip(
                    icon_button("redo", Icon::Redo),
                    redo_tip,
                    redo_reason.is_some(),
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("edit.redo")), cx)
                }),
            )
            .child(
                ui::button("finish", "Finish")
                    .small()
                    .ghost()
                    .icon(Icon::Check)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(editor_finish::finish::OpenFinish { run_check: false }),
                            cx,
                        )
                    }),
            )
            .child(self.export_button(
                quality,
                export_tip,
                export_reason.is_none(),
                export_open,
                window,
                cx,
            ))
            .child(
                // While Kerf works, the playhead goes where it looks.
                with_key(
                    icon_button("follow-kerf", Icon::Eye).tooltip("Follow Kerf"),
                    follow_key,
                )
                .on(follow)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("view.followKerf")), cx)
                }),
            )
            .child(
                with_key(
                    icon_button("media", Icon::Media).tooltip("Media"),
                    media_key,
                )
                .on(self.media_open)
                .on_click(cx.listener(|this, _, _, cx| this.set_media(!this.media_open, cx))),
            )
            .child(
                icon_button("panel", Icon::Panel)
                    .tooltip("Picture and timeline")
                    .on(self.panel_open)
                    .on_click(cx.listener(|this, _, _, cx| this.set_panel(!this.panel_open, cx))),
            )
    }

    /// At the window's leading edge, with the sidebar away: Sidebar, and
    /// the ⋯ its account's menu opens from meanwhile.
    fn leading_controls(&self, bar: Div, cx: &Context<Self>) -> Div {
        let sidebar_key = self.shown_key("view.sidebar");
        let app_menu_open = self.app_menu.as_ref().is_some_and(|(_, account)| !account);
        bar.child(
            with_key(
                icon_button("show-sidebar", Icon::Sidebar).tooltip("Sidebar"),
                sidebar_key,
            )
            .on_click(cx.listener(|this, _, _, cx| this.set_sidebar(true, cx))),
        )
        // With the sidebar away, its account's menu opens from here.
        .child(
            ui::capture_trigger(
                "toolbar-app-menu",
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.toggle_app_menu(false, false, window, cx)
                }),
            )
            .child(
                // Its menu says all the tooltip would, so the tooltip waits while it is open.
                icon_button("toolbar-app-menu-button", Icon::More)
                    .when(!app_menu_open, |button| {
                        button.tooltip("Settings, account and more")
                    })
                    .on(app_menu_open)
                    // From the keys (the press is the trigger's).
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        if event.is_keyboard() {
                            this.toggle_app_menu(false, true, window, cx);
                        }
                    })),
            ),
        )
    }

    /// Export, a split button: its main part renders the quality chosen
    /// in one press ("Export Master"); its chevron opens the menu that
    /// chooses it, with the queue. Off, with the reason in its tooltip,
    /// while it cannot render.
    fn export_button(
        &self,
        quality: Quality,
        tip: (String, Option<String>),
        able: bool,
        open: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ink = if able { theme::ink() } else { theme::dis() };
        let main = toolbar_stop(
            div().id("export-main"),
            &self.export_focus,
            0.0,
            able,
            window,
        )
        .flex()
        .items_center()
        .gap(px(theme::space::S6))
        .h(px(EXPORT_INNER))
        .px(px(10.0))
        .child(Icon::Export.view(14.0, ink))
        .child(ui::line("Export", text::CONTROL_STRONG, ink))
        .child(ui::line(
            quality.name(),
            text::CONTROL,
            if able { theme::ink3() } else { theme::dis() },
        ))
        .when(able, |main| {
            main.cursor_pointer()
                .hover(|main| main.bg(theme::hover2()))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(editor_finish::export::ExportChosen), cx)
                })
        });
        let chevron = toolbar_stop(
            div().id("export-options"),
            &self.export_options_focus,
            0.0,
            true,
            window,
        )
        .flex()
        .items_center()
        .justify_center()
        .w(px(EXPORT_CHEVRON))
        .h(px(EXPORT_INNER))
        .border_l_1()
        .border_color(theme::line2())
        .cursor_pointer()
        .when(open, |chevron| chevron.bg(theme::on()))
        .hover(|chevron| chevron.bg(theme::hover2()))
        .child(Icon::ChevronDown.view(14.0, theme::ink2()))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_export(window, cx)));
        // Its menu says all the tip would: the tip waits while it is open.
        let chevron = if open {
            chevron.into_any_element()
        } else {
            ui::tipped(
                "export-options-tip",
                ui::Tip::new("Export options"),
                chevron,
            )
            .into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .h(px(theme::height::H28))
            .rounded(px(theme::radius::CONTROL))
            .overflow_hidden()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::line2())
            .child(ui::tipped(
                "export-tip",
                ui::Tip::new(tip.0).maybe_key(tip.1),
                main,
            ))
            .child(chevron)
    }
}

/// What the toolbar says renders: the newest export in the queue, a running one
/// before a queued one ("Draft export · 42%", "Master export · 0%" while it
/// waits, or the job's name), else the preview rendering ("Rendering preview ·
/// 42%"). None when nothing does.
pub fn running_words(
    jobs: &[editor_state::model::RenderJob],
    preview: &editor_state::store::Preview,
) -> Option<String> {
    let percent = |fraction: f64| (fraction.clamp(0.0, 1.0) * 100.0).floor() as i64;
    let latest = jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.is_active())
        .max_by_key(|(at, job)| (job.status == "running", *at))
        .map(|(_, job)| job);
    if let Some(job) = latest {
        let quality = job.quality.as_deref().unwrap_or_default();
        let label = if matches!(quality, "review" | "preview") {
            "Draft export".to_owned()
        } else if quality == "master" && job.name.is_empty() {
            "Master export".to_owned()
        } else if !job.name.is_empty() {
            job.name.clone()
        } else {
            "Export".to_owned()
        };
        return Some(format!("{label} · {}%", percent(job.progress)));
    }
    preview.rendering().then(|| {
        format!(
            "Rendering preview · {}%",
            percent(preview.progress.unwrap_or(0.0))
        )
    })
}

/// Export's chevron, and the height inside the button's outline.
const EXPORT_CHEVRON: f32 = 24.0;
const EXPORT_INNER: f32 = theme::height::H28 - 2.0;
/// How far Export's right edge is from the window's: the toolbar's end,
/// past Follow Kerf, Media and the panel's button. Its menu opens there.
const EXPORT_RIGHT: f32 = 8.0 + 3.0 * (theme::height::H28 + 4.0);

/// Where the toolbar's controls start, and the popups they open, from
/// the main column's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarPlaces {
    /// Its first control (its leading padding).
    pub lead: f32,
    /// The title's menu and Activity, under the title.
    pub title: f32,
    /// The app's menu, under the toolbar's ⋯ (the sidebar away).
    pub app_menu: f32,
}

/// [`ToolbarPlaces`] for a toolbar at the window's leading edge or not
/// (`leading`: the sidebar away), with `room` what macOS's window buttons
/// need there ([`editor_ui::chrome::window_buttons_room`]). At that edge the
/// toolbar starts past the buttons, Sidebar and ⋯ first, and its popups
/// move on with the controls that open them.
pub fn toolbar_places(leading: bool, room: f32) -> ToolbarPlaces {
    let pad = 8.0;
    if !leading {
        // The title first, past its row's own 8.
        return ToolbarPlaces {
            lead: pad,
            title: pad + 8.0,
            app_menu: pad + TOOLBAR_STEP,
        };
    }
    let lead = editor_ui::chrome::lead_past(room);
    ToolbarPlaces {
        lead,
        // Sidebar, then ⋯, then the title's row, 8 in, and 4 past it.
        title: lead + 2.0 * TOOLBAR_STEP + 8.0 + 4.0,
        app_menu: lead + TOOLBAR_STEP,
    }
}
/// A toolbar button and the gap after it.
const TOOLBAR_STEP: f32 = theme::height::H28 + 4.0;

/// `button` with its key after its name in its tip, when it has one.
fn with_key(button: ui::IconButton, key: Option<String>) -> ui::IconButton {
    match key {
        Some(key) => button.key(key),
        None => button,
    }
}

/// `button` with the tip the toolbar worked out for it (its name and key,
/// or its name and why not), greyed while it cannot be pressed.
fn kit_tip(
    button: ui::IconButton,
    (words, key): (String, Option<String>),
    disabled: bool,
) -> ui::IconButton {
    let button = button.disabled(disabled);
    if disabled {
        button.tooltip(words.clone()).reason(words)
    } else {
        with_key(button.tooltip(words), key)
    }
}

/// `element`, a Tab stop of the toolbar's own (not a kit control): its
/// keys tracked by `focus`, the kit's ring just inside its edge (`corner`
/// round) while it has them, and the pointer's press leaving them where
/// they were. Not a stop while it cannot be pressed (`able` false).
pub(super) fn toolbar_stop(
    element: Stateful<gpui::Div>,
    focus: &FocusHandle,
    corner: f32,
    able: bool,
    window: &Window,
) -> Stateful<gpui::Div> {
    if !able {
        return element;
    }
    element
        .relative()
        .track_focus(focus)
        .key_context(ui::CONTROL)
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
        .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
        .when(focus.is_focused(window), |element| {
            element.child(ui::ring(corner))
        })
}

/// Export's menu, under its chevron at the toolbar's end, fading and
/// rising into place.
pub(super) fn export_popover(export: Entity<ExportView>) -> impl IntoElement {
    div()
        .absolute()
        .top(px(layout::TOOLBAR))
        .right(px(EXPORT_RIGHT))
        .child(export)
        .with_animation(
            "export-pop",
            motion::animation(scene::FADE),
            |popover, t| popover.opacity(t).mt(px(scene::MENU_RISE * (1.0 - t))),
        )
}
