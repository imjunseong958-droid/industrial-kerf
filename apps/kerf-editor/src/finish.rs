//! Finishing: Kerf's QC pass, the programme's loudness, where the master
//! goes and the captions; and the project shared through Git.
//!
//! Finish is a panel over the dimmed window (the workspace draws the dim and
//! the panel's rise), its sections named down its side: QC, Loudness and
//! Deliver. A section chosen fades in. What the bridge said of the last
//! check and the last measurement is kept for as long as the project is
//! open, so closing Finish loses nothing and a check goes on behind it.
//! Esc, its close, or a press on the dim gives the window back.
//!
//! [`Collab`] is the toolbar's chip for the project's Git repository: the
//! branch and how far it is from the remote, with Sync and the rest of
//! sharing in its menu.

mod collab;
mod deliver;
mod finishing;
mod loudness;
mod preflight;
mod qc;

pub use collab::{
    other_branches, publish_command, publish_name, pull_request_command, pull_request_said,
    pull_request_title, sync_lines, sync_said, Collab, CollabEvent, PublishWay, SyncFailure,
    NEEDS_GH, NO_OTHER_BRANCHES,
};
pub use deliver::deliver_said;
pub use preflight::{
    ask_text, can_ask, preflight, preflight_hidden, preflight_rows, AskKerf, Preflight,
    PreflightRow, PREFLIGHT_LEAST, PREFLIGHT_MOST,
};

use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::model::RenderJobShow;
use crate::motion;
use crate::session::Session;
use crate::store::Change;
use crate::theme::{self, height, motion as scene, radius, text};
use crate::ui::{self, icon_button};
use finishing::{finishing, Finishing};
use gpui::{
    actions, div, prelude::*, px, AnimationExt, AnyElement, App, Bounds, Context, DismissEvent,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Pixels, Rgba,
    SharedString, Subscription, Window,
};
use loudness::{meter_glyph, TOLERANCE};
use std::cell::Cell;
use std::rc::Rc;

actions!(finish, [Close]);

/// The key contexts Finish and the chip's menu answer Esc in.
const CONTEXT: &str = "Finish";
const MENU_CONTEXT: &str = "CollabMenu";

/// The panel at its largest; the workspace places it ([`crate::ui::modal_bounds`])
/// and rounds its corners, and it fills that.
pub const MOST: (f32, f32) = (860.0, 600.0);
/// Its title row, and the column of sections down its side.
const HEAD: f32 = 48.0;
const NAV: f32 = 196.0;
/// A section's edges.
const PAGE_SIDE: f32 = 24.0;
const PAGE_TOP: f32 = 20.0;

// ------------------------------------------------------------------ Finish

/// Open Finish (and run QC when asked to).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct OpenFinish {
    pub run_check: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Qc,
    Loudness,
    Deliver,
}

impl Section {
    const ALL: [Section; 3] = [Section::Qc, Section::Loudness, Section::Deliver];

    fn label(self) -> &'static str {
        match self {
            Section::Qc => "QC",
            Section::Loudness => "Loudness",
            Section::Deliver => "Deliver",
        }
    }

    fn index(self) -> usize {
        Section::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0)
    }

    fn icon(self, color: Rgba) -> AnyElement {
        match self {
            Section::Qc => Icon::Check.view(16.0, color).into_any_element(),
            Section::Loudness => meter_glyph(color).into_any_element(),
            Section::Deliver => Icon::Export.view(16.0, color).into_any_element(),
        }
    }
}

/// Finish, over the window: QC, loudness, the master and the captions.
pub struct FinishView {
    session: Entity<Session>,
    finishing: Entity<Finishing>,
    focus_handle: FocusHandle,
    section: Section,
    /// Where the chart was drawn, to read the pointer against.
    chart_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The share of the way across the chart the pointer is.
    hover: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl FinishView {
    pub fn new(
        session: Entity<Session>,
        run_check: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_close(cx);
        let finishing = finishing(&session, cx);
        let subscriptions = vec![
            cx.observe(&finishing, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |_, _, change: &Change, cx| {
                if matches!(
                    change,
                    Change::Renders | Change::Snapshot | Change::Connection
                ) {
                    cx.notify();
                }
            }),
        ];
        let mut view = Self {
            session,
            finishing,
            focus_handle: cx.focus_handle(),
            section: Section::Qc,
            chart_bounds: Rc::new(Cell::new(None)),
            hover: None,
            _subscriptions: subscriptions,
        };
        if run_check {
            view.run_check(cx);
        }
        view
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// The window's commands wait while Finish is over it; Run QC runs it
    /// here, and Quit still quits.
    fn command(&mut self, action: &RunCommand, _: &mut Window, cx: &mut Context<Self>) {
        match action.id.as_ref() {
            "app.quit" => cx.propagate(),
            "qc.run" => {
                self.show(Section::Qc, cx);
                self.run_check(cx);
            }
            _ => {}
        }
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.section != section {
            self.section = section;
            self.hover = None;
            cx.notify();
        }
    }

    fn connected(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        session.has_project() && session.state.connection == crate::store::Connection::Connected
    }

    // ---------------------------------------------------------- drawing

    fn head(&self, cx: &Context<Self>) -> impl IntoElement {
        let project = self.session.read(cx).project_name();
        div()
            .h(px(HEAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .pl(px(theme::space::S16))
            .pr(px(theme::space::S12))
            .child(ui::label("Finish", text::SIDE_NAME, theme::ink()))
            .child(
                ui::line(project, text::BODY, theme::ink3())
                    .flex_1()
                    .min_w_0(),
            )
            .child(
                icon_button("finish-close", Icon::Close)
                    .tooltip("Close")
                    .key("Esc")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            )
    }

    /// What a section's row says at its end: how its work stands.
    fn nav_note(&self, section: Section, cx: &App) -> Option<(SharedString, Rgba)> {
        let finishing = self.finishing.read(cx);
        match section {
            Section::Qc => {
                if let Some(progress) = finishing.checking {
                    return Some((percent(progress).into(), theme::activity::gpu()));
                }
                if finishing.check_failed.is_some() {
                    return Some(("Failed".into(), theme::neg()));
                }
                finishing.report.as_ref().map(|report| {
                    let color = if report.summary.blocking > 0 {
                        theme::neg()
                    } else if report.summary.passed < report.summary.total {
                        theme::warn()
                    } else {
                        theme::pos()
                    };
                    (
                        format!("{}/{}", report.summary.passed, report.summary.total).into(),
                        color,
                    )
                })
            }
            Section::Loudness => {
                if let Some(progress) = finishing.measuring {
                    return Some((percent(progress).into(), theme::activity::gpu()));
                }
                let loudness = finishing.loudness.as_ref()?;
                let integrated = loudness.integrated?;
                let color = if (integrated - loudness.target).abs() <= TOLERANCE {
                    theme::pos()
                } else {
                    theme::neg()
                };
                Some((number(integrated, 1).into(), color))
            }
            Section::Deliver => {
                let active = self
                    .session
                    .read(cx)
                    .state
                    .renders
                    .iter()
                    .filter(|job| job.is_active())
                    .count();
                (active > 0).then(|| (format!("{active} to go").into(), theme::activity::gpu()))
            }
        }
    }

    fn nav(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .w(px(NAV))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .px(px(theme::space::S8))
            .py(px(theme::space::S12))
            .border_r_1()
            .border_color(theme::line())
            .children(Section::ALL.iter().map(|section| {
                let section = *section;
                let on = section == self.section;
                let ink = if on { theme::ink() } else { theme::ink2() };
                let note = self.nav_note(section, cx);
                div()
                    .id(ElementId::named_usize("finish-nav", section.index()))
                    .h(px(height::H32))
                    .px(px(theme::space::S8))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer()
                    .when(on, |row| row.bg(theme::on()))
                    .when(!on, |row| row.hover(|style| style.bg(theme::raised())))
                    .child(section.icon(ink))
                    .child(
                        ui::label(
                            section.label(),
                            if on { text::BODY_NAME } else { text::BODY },
                            ink,
                        )
                        .flex_1(),
                    )
                    .when_some(note, |row, (words, color)| {
                        row.child(ui::dot(color, 6.0)).child(ui::label(
                            words,
                            text::META,
                            theme::ink2(),
                        ))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
            }))
    }

    /// The section chosen; QC offers a failed check's Ask Kerf where the
    /// window takes it (`can_ask`).
    fn page(&self, can_ask: bool, cx: &Context<Self>) -> impl IntoElement {
        let section = self.section;
        let content = match section {
            Section::Qc => self.qc(can_ask, cx),
            Section::Loudness => self.loudness(cx),
            Section::Deliver => self.deliver_page(cx),
        };
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        div()
                            .id(ElementId::named_usize("finish-scroll", section.index()))
                            .size_full()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(theme::space::S16))
                                    .px(px(PAGE_SIDE))
                                    .pt(px(PAGE_TOP))
                                    .pb(px(PAGE_SIDE))
                                    .children(content),
                            ),
                    )
                    // A section chosen fades in where it is.
                    .with_animation(
                        ElementId::named_usize("finish-section", section.index()),
                        motion::animation(scene::APPEAR),
                        |page, t| page.opacity(t),
                    ),
            )
    }

    /// A section's title, what it does, and its one action.
    fn title(
        &self,
        words: &'static str,
        says: impl Into<SharedString>,
        action: Option<AnyElement>,
    ) -> AnyElement {
        div()
            .flex()
            .items_start()
            .gap(px(theme::space::S16))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S2))
                    .child(ui::label(words, text::SIDE_NAME, theme::ink()))
                    .child(ui::label(says.into(), text::CONTROL, theme::ink3())),
            )
            .children(action)
            .into_any_element()
    }
}

impl EventEmitter<DismissEvent> for FinishView {}

impl Focusable for FinishView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FinishView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Whether the window fills the composer from a failed check's Ask
        // Kerf, as things stand as it is drawn.
        let can_ask = can_ask(window, cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(Self::command))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(self.head(cx))
            .child(ui::hairline(true))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(cx))
                    .child(self.page(can_ask, cx)),
            )
    }
}

// ---------------------------------------------------------- Finish's pieces

/// What runs, its share done, and an amber bar: the computer is at work.
fn working(words: &str, progress: f64) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::space::S6))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(ui::label(words.to_owned(), text::CONTROL, theme::ink2()).flex_1())
                .child(ui::label(
                    percent(progress),
                    text::MONO_SMALL,
                    theme::activity::gpu(),
                )),
        )
        .child(ui::progress(progress, theme::activity::gpu()))
        .into_any_element()
}

/// One line said with a dot of its colour; the words stay in ink.
fn said(words: impl Into<String>, color: Rgba) -> AnyElement {
    let words = sentence(words.into());
    div()
        .flex()
        .items_start()
        .gap(px(theme::space::S8))
        .child(
            div()
                .h(px(16.0))
                .flex()
                .items_center()
                .child(ui::dot(color, 6.0)),
        )
        .child(
            ui::label(words, text::CONTROL, theme::ink2())
                .flex_1()
                .min_w_0(),
        )
        .into_any_element()
}

/// `words` beginning with a capital, as the bridge's own messages may not.
fn sentence(words: String) -> String {
    let mut letters = words.chars();
    match letters.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().chain(letters).collect(),
        _ => words,
    }
}

/// What stands in for a result there is not yet.
fn empty(words: &'static str, more: &'static str) -> AnyElement {
    div()
        .px(px(theme::space::S16))
        .py(px(theme::space::S16))
        .flex()
        .flex_col()
        .gap(px(theme::space::S4))
        .rounded(px(radius::CARD))
        .border_1()
        .border_color(theme::line())
        .child(ui::label(words, text::BODY_NAME, theme::ink2()))
        .child(ui::label(more, text::CONTROL, theme::ink3()))
        .into_any_element()
}

/// `value` with `places` decimals and a true minus sign.
fn number(value: f64, places: usize) -> String {
    let shown = format!("{:.*}", places, value.abs());
    let zero = shown.chars().all(|c| c == '0' || c == '.');
    if value < 0.0 && !zero {
        format!("−{shown}")
    } else {
        shown
    }
}

/// A share, 0..1, in whole percents.
fn percent(share: f64) -> String {
    format!("{:.0}%", (share.clamp(0.0, 1.0) * 100.0).floor())
}

/// `one` for 1, else `many` with `{}` replaced by the count.
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        one.to_owned()
    } else {
        many.replace("{}", &count.to_string())
    }
}

/// Esc closes Finish and the chip's menu, before the window's own Esc
/// (Reject Proposal) has it. Binding a keymap clears this, so it is bound
/// again whenever it is missing.
fn bind_close(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &Close) {
        cx.bind_keys([
            KeyBinding::new("escape", Close, Some(CONTEXT)),
            KeyBinding::new("escape", Close, Some(MENU_CONTEXT)),
        ]);
    }
}
