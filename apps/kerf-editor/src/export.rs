//! Export: the toolbar's split button and its menu. The button's main part
//! renders the quality chosen in one press (the master, ProRes 422 HQ, or a
//! draft, an H.264 review copy, both into the project's deliveries/); File ▸
//! Export… renders that quality where kerfd puts it, at full size or as a
//! preview, as Qt's did. The chevron opens the menu under it: the two
//! qualities to choose between, Render preview, Render master and the
//! captions; under them the queue, newest first: what renders, what waits,
//! and what is done. Esc or a press anywhere else puts it away.

use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::model::RenderJob;
use crate::session::Session;
use crate::store::Change;
use crate::theme::{self, height, radius, text};
use crate::ui;
use gpui::{
    actions, div, prelude::*, px, App, Context, ElementId, Entity, EventEmitter, FocusHandle,
    FontWeight, KeyBinding, Rgba, SharedString, Subscription, Window,
};
use serde_json::{json, Value};
use std::path::Path;

actions!(export, [Close, ExportChosen]);

/// The key context the popover answers in.
const CONTEXT: &str = "ExportPopover";
/// The popover's width, and the most its queue takes before it scrolls.
pub const WIDTH: f32 = 340.0;
const QUEUE_MOST: f32 = 280.0;
/// The widest a row's aside (what it makes, or why it cannot) is before it is cut.
const ASIDE_MOST: f32 = 200.0;

/// What the toolbar's Export renders in one press, as its menu chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// The finished file: ProRes 422 HQ.
    #[default]
    Master,
    /// A review copy: H.264.
    Draft,
}

impl Quality {
    pub const ALL: [Quality; 2] = [Quality::Master, Quality::Draft];

    /// As the bridge's `export` takes it.
    pub fn wire(self) -> &'static str {
        match self {
            Quality::Master => "master",
            Quality::Draft => "review",
        }
    }

    /// As File ▸ Export… asks the registry's `file.export` for it: the
    /// full-size render (`full`) for Master, `preview` for Draft, both
    /// where kerfd puts them rather than into deliveries/.
    pub fn file_wire(self) -> &'static str {
        match self {
            Quality::Master => "full",
            Quality::Draft => "preview",
        }
    }

    /// As people read it, after "Export" on the button.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Master => "Master",
            Quality::Draft => "Draft",
        }
    }

    /// What it makes, as its row in the menu says.
    pub fn makes(self) -> &'static str {
        match self {
            Quality::Master => "ProRes 422 HQ",
            Quality::Draft => "H.264 review",
        }
    }

    /// What the button does with it, as its tooltip says.
    pub fn action(self) -> &'static str {
        match self {
            Quality::Master => "Export master",
            Quality::Draft => "Export draft",
        }
    }
}

/// What the toolbar's Export says when the pointer rests on it: what one
/// press renders, and why it cannot now ("Export master · Kerf is offline").
pub fn export_tip(quality: Quality, reason: Option<&str>) -> String {
    match reason.filter(|reason| !reason.trim().is_empty()) {
        Some(reason) => format!("{} · {reason}", quality.action()),
        None => quality.action().to_owned(),
    }
}

pub enum ExportEvent {
    Dismiss,
}

pub struct ExportView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    /// What had the keys before the popover took them, to give them back.
    previous_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ExportEvent> for ExportView {}

impl ExportView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_close(cx);
        // Only what the queue says is drawn again, not every poll.
        let subscriptions = vec![cx.subscribe(&session, |_, _, change: &Change, cx| {
            if matches!(change, Change::Renders | Change::Connection) {
                cx.notify();
            }
        })];
        // It takes the keys, so Esc is its own and not the window's.
        let focus_handle = cx.focus_handle();
        let previous_focus = window.focused(cx);
        window.focus(&focus_handle);
        Self {
            session,
            focus_handle,
            previous_focus,
            _subscriptions: subscriptions,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous);
        }
        cx.emit(ExportEvent::Dismiss);
    }

    /// File ▸ Export… renders and puts the menu away; the rest go on to
    /// the window.
    fn command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        if action.id.as_ref() == "file.export" {
            self.dismiss(window, cx);
        }
        cx.propagate();
    }

    /// The button's main part renders the chosen quality and puts the
    /// menu away; the window renders it.
    fn export_chosen(&mut self, _: &ExportChosen, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
        cx.propagate();
    }

    /// One row of the menu: its words, what it makes or why it cannot be
    /// done now, a check where it is the one chosen. `press` runs on a
    /// press while it can be done.
    fn row(
        &self,
        id: impl Into<ElementId>,
        words: &'static str,
        aside: SharedString,
        checked: Option<bool>,
        able: bool,
        press: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id.into())
            .mx(px(theme::space::S6))
            .px(px(theme::space::S8))
            .h(px(height::H32))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .rounded(px(radius::CONTROL))
            .when_some(checked, |row, checked| {
                row.child(if checked {
                    Icon::Check.view(14.0, theme::ink()).into_any_element()
                } else {
                    div().size(px(14.0)).flex_none().into_any_element()
                })
            })
            .child(
                ui::line(
                    words,
                    text::MENU_ITEM,
                    if able { theme::ink() } else { theme::dis() },
                )
                .flex_1(),
            )
            .child(
                ui::line(aside, text::META, theme::ink3())
                    .flex_none()
                    .max_w(px(ASIDE_MOST)),
            )
            .when(able, |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(theme::popup_hover()))
                    .on_click(cx.listener(move |this, _, window, cx| press(this, window, cx)))
            })
    }

    /// Master and Draft: the one chosen is what the button renders.
    fn qualities(&self, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let chosen = self.session.read(cx).export_quality;
        Quality::ALL
            .into_iter()
            .enumerate()
            .map(|(index, quality)| {
                self.row(
                    ElementId::named_usize("export-quality", index),
                    quality.name(),
                    quality.makes().into(),
                    Some(quality == chosen),
                    true,
                    move |this, window, cx| {
                        this.session.update(cx, |session, cx| {
                            session.export_quality = quality;
                            cx.notify();
                        });
                        this.dismiss(window, cx);
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect()
    }

    fn captions(&self, enabled: bool, cx: &Context<Self>) -> impl IntoElement {
        let format = |id: &'static str, label: &'static str, format: &'static str| {
            ui::button(id, label)
                .small()
                .disabled(!enabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |session, _| {
                        session.send(json!({"type": "export_captions", "format": format}));
                    });
                }))
        };
        div()
            .mx(px(theme::space::S6))
            .px(px(theme::space::S8))
            .h(px(height::H32))
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .child(
                ui::line(
                    "Captions",
                    text::MENU_ITEM,
                    if enabled { theme::ink() } else { theme::dis() },
                )
                .flex_1(),
            )
            .child(format("export-srt", "SRT", "srt"))
            .child(format("export-vtt", "VTT", "vtt"))
    }

    fn job(&self, index: usize, job: &RenderJob, cx: &Context<Self>) -> impl IntoElement {
        let (said, color) = job_status(job);
        let running = job.status == "running";
        let file = Path::new(&job.output_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let action = if job.is_active() {
            let id = job.job_id.clone();
            Some(
                ui::button(ElementId::named_usize("export-cancel", index), "Cancel")
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session
                            .update(cx, |session, _| session.cancel_export(&id));
                    }))
                    .into_any_element(),
            )
        } else if job.status == "completed" && !job.output_path.is_empty() {
            let path = job.output_path.clone();
            Some(
                ui::button(
                    ElementId::named_usize("export-reveal", index),
                    "Show in Folder",
                )
                .small()
                .ghost()
                .on_click(move |_, _, cx| cx.reveal_path(Path::new(&path)))
                .into_any_element(),
            )
        } else {
            None
        };
        div()
            .px(px(theme::space::S16 - 2.0))
            .py(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S4))
            .when(index > 0, |row| {
                row.border_t_1().border_color(theme::line())
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::line(job_name(job), text::BODY, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(ui::line(said, text::META, color).flex_none()),
            )
            .when(running, |row| {
                row.child(ui::progress(job.progress, theme::activity::gpu()))
            })
            .when_some(
                job.error.clone().filter(|_| job.status == "failed"),
                |row, error| row.child(ui::label(error, text::META, theme::neg())),
            )
            .when(!file.is_empty() || action.is_some(), |row| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S8))
                        .min_h(px(height::H24))
                        .child(
                            ui::line(file, text::MONO_SMALL, theme::ink3())
                                .flex_1()
                                .min_w_0(),
                        )
                        .children(action),
                )
            })
    }
}

impl Render for ExportView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // What can be done now, as the registry says it.
        let states = {
            let cloud = crate::app::cloud(cx).cloned();
            self.session.read(cx).command_states(cloud.as_ref())
        };
        let reason = |id: &str| unavailable(&states, id);
        let preview_reason = reason("sequence.renderPreview");
        let master_reason = reason("export.master");
        let session = self.session.read(cx);
        let enabled = session.state.connection == crate::store::Connection::Connected;
        let jobs: Vec<RenderJob> = session.state.renders.iter().rev().cloned().collect();
        let active = jobs.iter().filter(|job| job.is_active()).count();
        let qualities = self.qualities(cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(Self::command))
            .on_action(cx.listener(Self::export_chosen))
            // A press anywhere else puts it away, and does nothing more: a
            // press on Export closes it rather than opening it again.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.dismiss(window, cx);
            }))
            // What lies under it (the picture, which plays on a press) has
            // none of its presses.
            .occlude()
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .py(px(theme::space::S6))
            .rounded(px(radius::CARD))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(
                div()
                    .px(px(theme::space::S16 - 2.0))
                    .h(px(height::H28))
                    .flex()
                    .items_center()
                    .child(ui::label("Export", text::TITLE, theme::ink()).flex_1())
                    .child(ui::label("Into deliveries/", text::META, theme::ink3())),
            )
            .children(qualities)
            .child(
                self.row(
                    "export-render-preview",
                    "Render preview",
                    preview_reason
                        .clone()
                        .unwrap_or_else(|| "Draft from proxies".into()),
                    Some(false),
                    preview_reason.is_none(),
                    |this, window, cx| {
                        // Dispatched from here, where the window hears it, before
                        // the keys go back.
                        window.dispatch_action(
                            Box::new(RunCommand::new("sequence.renderPreview")),
                            cx,
                        );
                        this.dismiss(window, cx);
                    },
                    cx,
                ),
            )
            .child(div().my(px(theme::space::S6)).child(ui::hairline(true)))
            .child(
                self.row(
                    "export-render-master",
                    "Render master",
                    master_reason
                        .clone()
                        .unwrap_or_else(|| Quality::Master.makes().into()),
                    None,
                    master_reason.is_none(),
                    // It stays open: the queue below shows the render come in.
                    |this, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.deliver(Quality::Master.wire());
                            cx.notify();
                        });
                    },
                    cx,
                ),
            )
            .child(self.captions(enabled, cx))
            .child(div().my(px(theme::space::S6)).child(ui::hairline(true)))
            .child(
                div()
                    .px(px(theme::space::S16 - 2.0))
                    .h(px(height::H24))
                    .flex()
                    .items_center()
                    .child(ui::label("Queue", text::META_STRONG, theme::ink3()).flex_1())
                    .when(active > 0, |head| {
                        head.child(ui::label(
                            if active == 1 {
                                "1 to go".to_owned()
                            } else {
                                format!("{active} to go")
                            },
                            text::META,
                            theme::activity::gpu(),
                        ))
                    }),
            )
            .when(jobs.is_empty(), |popover| {
                popover.child(
                    ui::label("Nothing rendered yet", text::CONTROL, theme::ink3())
                        .px(px(theme::space::S16 - 2.0))
                        .py(px(theme::space::S6)),
                )
            })
            .when(!jobs.is_empty(), |popover| {
                popover.child(
                    div()
                        .id("export-queue")
                        .max_h(px(QUEUE_MOST))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .children(
                            jobs.iter()
                                .enumerate()
                                .map(|(index, job)| self.job(index, job, cx)),
                        ),
                )
            })
    }
}

/// Why command `id` cannot run, as `states` (`Session::command_states`)
/// says; None when it can.
fn unavailable(states: &Value, id: &str) -> Option<SharedString> {
    states
        .get(id)
        .and_then(Value::as_str)
        .map(|reason| SharedString::from(reason.to_owned()))
}

/// What a job is called: its name, else what it renders.
fn job_name(job: &RenderJob) -> SharedString {
    if !job.name.trim().is_empty() {
        return job.name.clone().into();
    }
    match job.quality.as_str() {
        "master" => "Master",
        "review" => "Review copy",
        "full" => "Full render",
        "preview" => "Preview",
        _ => "Render",
    }
    .into()
}

/// What a job's status says, and in what colour: amber while it renders.
fn job_status(job: &RenderJob) -> (SharedString, Rgba) {
    match job.status.as_str() {
        "queued" => ("Waiting".into(), theme::ink2()),
        "running" => (
            format!("Rendering {:.0}%", job.progress.clamp(0.0, 1.0) * 100.0).into(),
            theme::activity::gpu(),
        ),
        "completed" => ("Done".into(), theme::pos()),
        "failed" => ("Failed".into(), theme::neg()),
        "cancelled" => ("Cancelled".into(), theme::ink3()),
        other => (other.to_owned().into(), theme::ink3()),
    }
}

/// Esc puts the popover away, before the window's own Esc (Reject
/// Proposal) has it. Binding a keymap clears this, so it is bound again
/// whenever it is missing.
pub(crate) fn bind_close(cx: &mut App) {
    if !crate::settings::escape_bound(cx, &Close) {
        cx.bind_keys([KeyBinding::new("escape", Close, Some(CONTEXT))]);
    }
}
