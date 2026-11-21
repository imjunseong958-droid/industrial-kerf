//! Export: the toolbar's split button and its menu. The button's main part
//! renders the quality chosen in one press (the master, or a draft, an
//! H.264 review copy into the project's deliveries/); File ▸
//! Export… renders that quality where kerfd puts it, at full size or as a
//! preview, as Qt's did. The chevron opens the menu under it: the two
//! qualities to choose between and Render preview; Preflight, the last QC
//! pass's checks, each failed one with Ask Kerf while the window takes it
//! ([`crate::finish::can_ask`]); Render master and the
//! captions; under them the queue, newest first: what renders, what waits,
//! and what is done. Esc or a press anywhere else puts it away.
//!
//! The master goes where the project keeps it to go ([`master_request`]):
//! deliveries/ as `<project>_master`, ProRes 422 HQ, unless the person's
//! settings name another folder, file name or codec for the project. The
//! menu says so: where one press puts the chosen quality, and what the
//! master is made as.

use crate::commands::RunCommand;
use crate::delivery::{
    export_of, export_request, master_makes, master_request, request, shown_folder, Quality,
};
use crate::finish::{AskKerf, Preflight};
use crate::icons::Icon;
use crate::model::{RenderJob, RenderJobShow};
use crate::prefs::MasterOutput;
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, height, radius, text};
use crate::ui;
use gpui::{
    actions, div, prelude::*, px, AnyElement, App, Context, ElementId, Entity, EventEmitter,
    FocusHandle, FontWeight, KeyBinding, Rgba, SharedString, Subscription, Window,
};
use kerf_bridge::{CaptionsRequest, Command, Event};
use serde_json::Value;
use std::path::Path;

actions!(export, [Close, ExportChosen]);

/// The key context the popover answers in.
const CONTEXT: &str = "ExportPopover";
/// The popover's width, and the most its queue takes before it scrolls.
pub const WIDTH: f32 = 340.0;
const QUEUE_MOST: f32 = 280.0;
/// The widest a row's aside (what it makes, or why it cannot) is before it is cut.
const ASIDE_MOST: f32 = 200.0;

/// What Export's menu says beside its title: where one press of the
/// toolbar's Export puts `quality` in `project` ("Into deliveries/", "Into
/// masters/" for a master the project keeps there). None where kerfd puts
/// renders.
pub fn export_into(
    quality: Quality,
    project: Option<&Path>,
    kept: Option<&MasterOutput>,
) -> Option<String> {
    let command = match quality {
        Quality::Master => master_request(project, kept),
        Quality::Draft => export_request(project, quality.wire()),
    };
    export_of(&command)
        .and_then(|request| request.output.as_deref())
        .filter(|folder| !folder.is_empty())
        .map(|folder| format!("Into {}", shown_folder(folder)))
}

/// Renders the project's master where it is kept to go ([`request`]),
/// leaving "Export started (Master)" in Activity. Export's Render master
/// and Finish's Export Master call it; false when it was not sent.
pub fn render_master(session: &mut Session) -> bool {
    let command = request(
        session.project.as_deref(),
        &session.prefs,
        Quality::Master.wire(),
    );
    let sent = session.send(command);
    if sent {
        session.activity.push("export", "Export started (Master)");
    }
    sent
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

/// What a press on a Preflight row does (Run QC, or Ask Kerf).
type PreflightPress = Box<dyn Fn(&mut ExportView, &mut Window, &mut Context<ExportView>)>;

impl ExportView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_close(cx);
        // Only what the queue and the checks say is drawn again, not every event.
        let subscriptions = vec![
            cx.subscribe(&session, |_, _, change: &Change, cx| {
                if matches!(change, Change::Renders | Change::Connection) {
                    cx.notify();
                }
            }),
            cx.subscribe(&session, |_, _, event: &BridgeEvent, cx| {
                if matches!(*event.0, Event::Check(_)) {
                    cx.notify();
                }
            }),
        ];
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
    // Each argument is a distinct part of the row.
    #[allow(clippy::too_many_arguments)]
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

    /// Master and Draft: the one chosen is what the button renders. The
    /// master says what it is made as (`master`: [`master_makes`]).
    fn qualities(&self, master: &str, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let chosen = self.session.read(cx).export_quality;
        Quality::ALL
            .into_iter()
            .enumerate()
            .map(|(index, quality)| {
                self.row(
                    ElementId::named_usize("export-quality", index),
                    quality.name(),
                    match quality {
                        Quality::Master => master.to_owned().into(),
                        Quality::Draft => quality.makes().into(),
                    },
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
                        session.send(Command::ExportCaptions(CaptionsRequest {
                            format: format.to_owned(),
                            ..CaptionsRequest::default()
                        }));
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

    /// One row of Preflight: its dot, its words, and at its end what a
    /// passed check allows or, accented, what a press does.
    // Each argument is a distinct part of the row.
    #[allow(clippy::too_many_arguments)]
    fn preflight_row(
        &self,
        id: impl Into<ElementId>,
        dot: Option<Rgba>,
        words: SharedString,
        ink: Rgba,
        aside: Option<(SharedString, bool)>,
        tip: Option<SharedString>,
        press: Option<PreflightPress>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let row = div()
            .id(id.into())
            .mx(px(theme::space::S6))
            .px(px(theme::space::S8))
            .h(px(height::H28))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .rounded(px(radius::CONTROL))
            .child(
                div()
                    .size(px(14.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(dot.map(|color| ui::dot(color, 7.0))),
            )
            .child(ui::line(words, text::CONTROL, ink).flex_1().min_w_0())
            .children(aside.map(|(words, accent)| {
                if accent {
                    ui::line(words, text::CONTROL_STRONG, theme::accent_ink()).flex_none()
                } else {
                    ui::line(words, text::META, theme::ink3())
                        .flex_none()
                        .max_w(px(ASIDE_MOST))
                }
            }));
        let row = match press {
            Some(press) => row
                .cursor_pointer()
                .hover(|style| style.bg(theme::popup_hover()))
                .on_click(cx.listener(move |this, _, window, cx| press(this, window, cx))),
            None => row,
        };
        ui::tip_on(row, tip.map(ui::Tip::new)).into_any_element()
    }

    /// Preflight: the last QC pass's checks, every failed one first (each
    /// with Ask Kerf while the window can fill the composer: `can_ask`),
    /// then passed ones with what they allow; or that none ran yet, that
    /// one runs, or why the last one failed. `qc` is why QC cannot run now.
    fn preflight(
        &self,
        preflight: &Preflight,
        qc: Option<SharedString>,
        can_ask: bool,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let run_qc = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            // Dispatched from here, where the window hears it, before the
            // keys go back.
            window.dispatch_action(Box::new(RunCommand::new("qc.run")), cx);
            this.dismiss(window, cx);
        };
        let mut rows = vec![div()
            .px(px(theme::space::S16 - 2.0))
            .h(px(height::H24))
            .flex()
            .items_center()
            .child(ui::label("Preflight", text::META_STRONG, theme::ink3()))
            .into_any_element()];
        if let Some(progress) = preflight.checking {
            rows.push(self.preflight_row(
                "export-preflight-running",
                None,
                format!("Checking… {}%", (progress.clamp(0.0, 1.0) * 100.0).floor()).into(),
                theme::dis(),
                None,
                None,
                None,
                cx,
            ));
        }
        if !preflight.checked && preflight.checking.is_none() {
            rows.push(self.preflight_row(
                "export-preflight-run",
                Some(theme::dis()),
                "Not checked yet".into(),
                if qc.is_none() {
                    theme::ink()
                } else {
                    theme::dis()
                },
                Some(("Run QC".into(), qc.is_none())),
                qc.clone(),
                qc.is_none().then(|| Box::new(run_qc) as Box<_>),
                cx,
            ));
        }
        if let Some(failed) = preflight.failed.as_deref() {
            let words = if failed.trim().is_empty() {
                "The last check failed".to_owned()
            } else {
                format!("The last check failed: {failed}")
            };
            rows.push(self.preflight_row(
                "export-preflight-failed",
                None,
                words.into(),
                theme::warn(),
                Some(("Run again".into(), qc.is_none())),
                qc.clone(),
                qc.is_none().then(|| Box::new(run_qc) as Box<_>),
                cx,
            ));
        }
        for (index, row) in preflight.rows.iter().enumerate() {
            let id = ElementId::named_usize("export-preflight-row", index);
            let dot = Some(if row.passed {
                theme::pos()
            } else {
                theme::warn()
            });
            let element = match row.ask.clone().filter(|_| can_ask) {
                Some(ask) => {
                    let words: SharedString = ask.into();
                    let asked = words.clone();
                    self.preflight_row(
                        id,
                        dot,
                        row.label.clone().into(),
                        theme::ink(),
                        Some(("Ask Kerf".into(), true)),
                        Some(words),
                        Some(Box::new(
                            move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                                // Put away first, so the keys it gives back
                                // are not taken from the composer after.
                                this.dismiss(window, cx);
                                window.dispatch_action(
                                    Box::new(AskKerf {
                                        words: asked.clone(),
                                    }),
                                    cx,
                                );
                            },
                        )),
                        cx,
                    )
                }
                None => self.preflight_row(
                    id,
                    dot,
                    row.label.clone().into(),
                    theme::ink(),
                    row.limit.clone().map(|limit| (limit.into(), false)),
                    None,
                    None,
                    cx,
                ),
            };
            rows.push(element);
        }
        if preflight.hidden > 0 {
            let words = if preflight.hidden == 1 {
                "1 more check failed".to_owned()
            } else {
                format!("{} more checks failed", preflight.hidden)
            };
            rows.push(self.preflight_row(
                "export-preflight-more",
                Some(theme::warn()),
                words.into(),
                theme::ink2(),
                None,
                None,
                None,
                cx,
            ));
        }
        rows
    }

    fn job(&self, index: usize, job: &RenderJob, cx: &Context<Self>) -> impl IntoElement {
        let (said, color) = job_status(job);
        let running = job.status == "running";
        let file = Path::new(job.output_path.as_deref().unwrap_or_default())
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
        } else if job.status == "completed"
            && job
                .output_path
                .as_deref()
                .is_some_and(|path| !path.is_empty())
        {
            let path = job.output_path.clone().unwrap_or_default();
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // What can be done now, as the registry says it.
        let states = {
            let cloud = crate::session::cloud(cx).cloned();
            self.session.read(cx).command_states(cloud.as_ref())
        };
        let reason = |id: &str| unavailable(&states, id);
        let preview_reason = reason("sequence.renderPreview");
        let master_reason = reason("export.master");
        let preflight = crate::finish::preflight(&self.session, cx);
        let can_ask = crate::finish::can_ask(window, cx);
        let preflight = self.preflight(&preflight, reason("qc.run"), can_ask, cx);
        let session = self.session.read(cx);
        let enabled = session.state.connection == crate::store::Connection::Connected;
        let jobs: Vec<RenderJob> = session.state.renders.iter().rev().cloned().collect();
        let active = jobs.iter().filter(|job| job.is_active()).count();
        // Where the master goes and what it is made as, as the project keeps it.
        let project = session.project.clone();
        let kept = project
            .as_deref()
            .and_then(|project| session.prefs.master_output(project))
            .cloned();
        let master = master_makes(kept.as_ref());
        let into = export_into(session.export_quality, project.as_deref(), kept.as_ref());
        let qualities = self.qualities(&master, cx);
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
                    .gap(px(theme::space::S8))
                    .child(ui::label("Export", text::TITLE, theme::ink()).flex_1())
                    .children(into.map(|into| {
                        ui::line(into, text::META, theme::ink3())
                            .flex_none()
                            .max_w(px(ASIDE_MOST))
                    })),
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
            .children(preflight)
            .child(div().my(px(theme::space::S6)).child(ui::hairline(true)))
            .child(
                self.row(
                    "export-render-master",
                    "Render master",
                    master_reason
                        .clone()
                        .unwrap_or_else(|| master.clone().into()),
                    None,
                    master_reason.is_none(),
                    // It stays open: the queue below shows the render come in.
                    |this, _, cx| {
                        this.session.update(cx, |session, cx| {
                            render_master(session);
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
    match job.quality.as_deref().unwrap_or_default() {
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
