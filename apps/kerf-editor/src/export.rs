//! Export: the render queue's popover under the toolbar's Export. What to
//! render, from the master to a draft, and the captions; under them the
//! queue, newest first: what renders, what waits, and what is done. Esc or
//! a press anywhere else puts it away.

use crate::commands::RunCommand;
use crate::model::RenderJob;
use crate::session::Session;
use crate::store::Change;
use crate::theme::{self, height, radius, text};
use crate::ui;
use gpui::{
    actions, div, prelude::*, px, App, Context, ElementId, Entity, EventEmitter, FocusHandle,
    FontWeight, KeyBinding, Rgba, SharedString, Subscription, Window,
};
use serde_json::json;
use std::path::Path;

actions!(export, [Close]);

/// The key context the popover answers in.
const CONTEXT: &str = "ExportPopover";
/// The popover's width, and the most its queue takes before it scrolls.
const WIDTH: f32 = 340.0;
const QUEUE_MOST: f32 = 280.0;

/// A quality the queue renders: the bridge's name for it, the popover's,
/// and what it makes.
struct Quality {
    id: &'static str,
    name: &'static str,
    makes: &'static str,
}

const QUALITIES: [Quality; 4] = [
    Quality {
        id: "master",
        name: "Master",
        makes: "ProRes 422 HQ · .mov",
    },
    Quality {
        id: "review",
        name: "Review copy",
        makes: "H.264 · 12 Mb/s",
    },
    Quality {
        id: "full",
        name: "Full render",
        makes: "H.264 · full size",
    },
    Quality {
        id: "preview",
        name: "Preview",
        makes: "Draft from proxies",
    },
];

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

    /// Export's own command puts it away, as its button would; the rest go
    /// on to the window.
    fn command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        if action.id.as_ref() == "file.export" {
            self.dismiss(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn quality(
        &self,
        index: usize,
        quality: &Quality,
        enabled: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = quality.id;
        div()
            .id(ElementId::named_usize("export-quality", index))
            .mx(px(theme::space::S6))
            .px(px(theme::space::S8))
            .h(px(height::H32))
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .rounded(px(radius::CONTROL))
            .child(
                ui::line(
                    quality.name,
                    text::MENU_ITEM,
                    if enabled { theme::ink() } else { theme::dis() },
                )
                .flex_1(),
            )
            .child(ui::line(quality.makes, text::META, theme::ink3()).flex_none())
            .when(enabled, |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(theme::popup_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session.update(cx, |session, _| session.export(id));
                    }))
            })
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
        let session = self.session.read(cx);
        let enabled = session.has_project();
        let jobs: Vec<RenderJob> = session.state.renders.iter().rev().cloned().collect();
        let active = jobs.iter().filter(|job| job.is_active()).count();
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(Self::command))
            // A press anywhere else puts it away, and does nothing more: a
            // press on Export closes it rather than opening it again.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.dismiss(window, cx);
            }))
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
                    .child(ui::label("Beside project.kerf", text::META, theme::ink3())),
            )
            .children(
                QUALITIES
                    .iter()
                    .enumerate()
                    .map(|(index, quality)| self.quality(index, quality, enabled, cx)),
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

/// What a job is called: its name, else what it renders.
fn job_name(job: &RenderJob) -> SharedString {
    if !job.name.trim().is_empty() {
        return job.name.clone().into();
    }
    QUALITIES
        .iter()
        .find(|quality| quality.id == job.quality)
        .map(|quality| quality.name)
        .unwrap_or("Render")
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
