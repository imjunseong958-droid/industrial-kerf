//! Deliver, Finish's third section: the master and the review copy, where
//! each goes and how its newest render stands, the captions beside them,
//! and the render queue.

use super::{percent, plural, said, FinishView, Section};
use editor_state::delivery::DELIVERIES;
use editor_state::model::{RenderJob, RenderJobShow};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, AnyElement, App, Context, ElementId, Rgba};
use kerf_bridge::{CaptionsRequest, Command};
use std::path::Path;

/// Where the last captions went, or why they did not.
pub(super) struct Captions {
    pub(super) format: String,
    pub(super) outcome: Result<String, String>,
}

/// What Deliver says under its title: where the master and the review copy
/// go in `project`, the master where the project keeps it to go
/// ([`editor_state::delivery::master_request`], `kept`): "The master and a review
/// copy go to deliveries/ in the project, numbered when one is there
/// already.", or "The master goes to masters/ and a review copy to
/// deliveries/ in the project, each numbered when one is there already."
pub fn deliver_said(
    project: Option<&Path>,
    kept: Option<&editor_state::prefs::MasterOutput>,
) -> String {
    /// Where a request sends its file: the folder as people read it, and
    /// whether it is in the project; None where kerfd puts renders.
    fn place(command: &Command) -> Option<(String, bool)> {
        editor_state::delivery::export_of(command)
            .and_then(|request| request.output.as_deref())
            .map(|folder| {
                (
                    editor_state::delivery::shown_folder(folder),
                    !Path::new(folder).is_absolute(),
                )
            })
    }
    /// "to deliveries/", "to deliveries/ in the project" (`said_in`), or
    /// "where Kerf puts renders".
    fn goes(place: &Option<(String, bool)>, said_in: bool) -> String {
        match place {
            Some((folder, inside)) if *inside && said_in => {
                format!("to {folder} in the project")
            }
            Some((folder, _)) => format!("to {folder}"),
            None => "where Kerf puts renders".to_owned(),
        }
    }
    let master = place(&editor_state::delivery::master_request(project, kept));
    let review = place(&editor_state::delivery::export_request(project, "review"));
    if master == review {
        return format!(
            "The master and a review copy go {}, numbered when one is there already.",
            goes(&master, true)
        );
    }
    // Both in the project: said once, at the end.
    let both_inside = matches!((&master, &review), (Some((_, true)), Some((_, true))));
    format!(
        "The master goes {} and a review copy {}, each numbered when one is there already.",
        goes(&master, !both_inside),
        goes(&review, true)
    )
}

impl FinishView {
    /// The project's folder name, which names what it delivers.
    fn project_folder(&self, cx: &App) -> Option<String> {
        self.session
            .read(cx)
            .project
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty())
    }

    /// Renders the master (`master`) where the project keeps it to go
    /// (deliveries/ unless its settings say otherwise), or a review copy
    /// (`review`) into deliveries/, named after the project.
    fn deliver(&mut self, quality: &'static str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            if quality == "master" {
                crate::export::render_master(session);
            } else {
                session.deliver(quality);
            }
        });
    }

    fn captions(&mut self, format: &'static str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.send(Command::ExportCaptions(CaptionsRequest {
                format: format.to_owned(),
                ..CaptionsRequest::default()
            }));
        });
    }

    /// The section: where its renders go, what QC found that blocks
    /// delivery, the master, the review copy and the captions, and the
    /// render queue.
    pub(super) fn deliver_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let finishing = self.finishing.read(cx);
        let session = self.session.read(cx);
        let enabled = self.connected(cx);
        let project = self.project_folder(cx);
        let folder = session.project.clone();
        let jobs: Vec<RenderJob> = session.state.renders.iter().rev().cloned().collect();
        let named = |suffix: &str, extension: &str| match &project {
            Some(project) => format!("{DELIVERIES}{project}_{suffix}.{extension}"),
            None => format!("{suffix}.{extension}"),
        };
        let latest = |quality: &str| {
            jobs.iter()
                .find(|job| job.quality.as_deref().unwrap_or_default() == quality)
                .cloned()
        };
        let kept = folder
            .as_deref()
            .and_then(|project| session.prefs.master_output(project));
        let (master_goes, master_makes) =
            editor_state::delivery::master_goes(folder.as_deref(), kept);
        let heading = deliver_said(folder.as_deref(), kept);
        let mut out = vec![self.title("Deliver", heading, None)];
        if let Some(report) = finishing
            .report
            .as_ref()
            .filter(|report| report.summary.blocking > 0)
        {
            out.push(self.blocking(report.summary.blocking, cx));
        }
        let master = ui::button("finish-master", "Export Master")
            .primary()
            .icon(Icon::Export)
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.deliver("master", cx)))
            .into_any_element();
        let review = ui::button("finish-review", "Export Review Copy")
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.deliver("review", cx)))
            .into_any_element();
        let card = div()
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .child(
                Deliverable {
                    name: "Master",
                    makes: master_makes,
                    goes: master_goes,
                    newest: latest("master"),
                    problem: finishing.export_failed.clone(),
                }
                .row(master, folder.as_deref(), true),
            )
            .child(
                Deliverable {
                    name: "Review copy",
                    makes: "H.264 at 12 Mb/s, full size, to send round".into(),
                    goes: named("draft", "mp4"),
                    newest: latest("review"),
                    problem: None,
                }
                .row(review, folder.as_deref(), false),
            )
            .child(self.captions_row(enabled, folder.as_deref(), cx));
        out.push(card.into_any_element());
        out.push(self.queue(&jobs, folder.as_deref(), cx));
        out
    }

    /// What QC found that blocks delivery, `blocking` problems, with the
    /// way back to QC.
    fn blocking(&self, blocking: usize, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .px(px(theme::space::S12))
            .py(px(theme::space::S8))
            .rounded(px(radius::CARD))
            .bg(theme::raised())
            .child(ui::dot(theme::neg(), 8.0))
            .child(
                ui::label(
                    plural(
                        blocking,
                        "QC found 1 problem that blocks delivery",
                        "QC found {} problems that block delivery",
                    ),
                    text::CONTROL,
                    theme::ink(),
                )
                .flex_1(),
            )
            .child(
                ui::button("finish-to-qc", "Show QC")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| this.show(Section::Qc, cx))),
            )
            .into_any_element()
    }

    /// The captions' row: what they are, SRT and VTT, and where the last
    /// went or why they did not.
    fn captions_row(
        &self,
        enabled: bool,
        folder: Option<&Path>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let srt = ui::button("finish-srt", "SRT")
            .small()
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.captions("srt", cx)))
            .into_any_element();
        let vtt = ui::button("finish-vtt", "VTT")
            .small()
            .disabled(!enabled)
            .on_click(cx.listener(|this, _, _, cx| this.captions("vtt", cx)))
            .into_any_element();
        let captions_said =
            self.finishing
                .read(cx)
                .captions
                .as_ref()
                .map(|captions| match &captions.outcome {
                    Ok(path) => {
                        let shown = shown_path(folder, path);
                        let reveal = path.clone();
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S8))
                            .child(ui::dot(theme::pos(), 6.0))
                            .child(
                                ui::line(
                                    format!("Wrote {} · {shown}", captions.format.to_uppercase()),
                                    text::CONTROL,
                                    theme::ink2(),
                                )
                                .flex_1()
                                .min_w_0(),
                            )
                            .child(
                                ui::button("finish-captions-reveal", "Show in Folder")
                                    .small()
                                    .ghost()
                                    .on_click(move |_, _, cx| cx.reveal_path(Path::new(&reveal))),
                            )
                            .into_any_element()
                    }
                    Err(message) => said(message.clone(), theme::neg()),
                });
        div()
            .px(px(theme::space::S16))
            .py(px(theme::space::S12))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .border_t_1()
            .border_color(theme::line())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S16))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::space::S2))
                            .child(ui::label("Captions", text::BODY_NAME, theme::ink()))
                            .child(ui::label(
                                "Every enabled subtitle track, beside the newest master, else in deliveries/",
                                text::CONTROL,
                                theme::ink3(),
                            )),
                    )
                    .child(div().flex().gap(px(theme::space::S6)).child(srt).child(vtt)),
            )
            .children(captions_said)
    }

    /// The render queue: the newest five, and how many are to go.
    fn queue(&self, jobs: &[RenderJob], folder: Option<&Path>, cx: &Context<Self>) -> AnyElement {
        let active = jobs.iter().filter(|job| job.is_active()).count();
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(ui::label("Render queue", text::META_STRONG, theme::ink3()).flex_1())
                    .when(active > 0, |head| {
                        head.child(ui::label(
                            format!("{active} to go"),
                            text::META,
                            theme::activity::gpu(),
                        ))
                    }),
            )
            .when(jobs.is_empty(), |list| {
                list.child(ui::label(
                    "Nothing rendered yet",
                    text::CONTROL,
                    theme::ink3(),
                ))
            })
            .children(
                jobs.iter()
                    .take(5)
                    .enumerate()
                    .map(|(index, job)| self.queue_row(index, job, folder, cx)),
            )
            .into_any_element()
    }

    /// A render in the queue: its name, where it goes, how it stands, and
    /// Cancel while it is to go or Show in Folder once it is done.
    fn queue_row(
        &self,
        index: usize,
        job: &RenderJob,
        folder: Option<&Path>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let (status, color) = job_status(job);
        let running = job.status == "running";
        let action = if job.is_active() {
            let id = job.job_id.clone();
            Some(
                ui::button(ElementId::named_usize("finish-cancel", index), "Cancel")
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
                    ElementId::named_usize("finish-reveal", index),
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
            .py(px(theme::space::S6))
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
                    .min_h(px(height::H24))
                    .child(ui::line(job_name(job), text::BODY, theme::ink()).flex_none())
                    .child(
                        ui::line(
                            shown_path(folder, job.output_path.as_deref().unwrap_or_default()),
                            text::MONO_SMALL,
                            theme::ink3(),
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(ui::line(status, text::META, color).flex_none())
                    .children(action),
            )
            .when(running, |row| {
                row.child(ui::progress(job.progress, theme::activity::gpu()))
            })
            .when_some(
                job.error.clone().filter(|_| job.status == "failed"),
                |row, error| row.child(ui::label(error, text::META, theme::neg())),
            )
    }
}

/// What Deliver renders: its name, what it makes and where it goes, its
/// newest render, and why the last one did not go.
struct Deliverable {
    name: &'static str,
    makes: String,
    goes: String,
    newest: Option<RenderJob>,
    problem: Option<String>,
}

impl Deliverable {
    /// Its row: the words, `action` at the end, and how its newest render stands.
    fn row(self, action: AnyElement, folder: Option<&Path>, first: bool) -> AnyElement {
        let Self {
            name,
            makes,
            goes,
            newest,
            problem,
        } = self;
        let running = newest
            .as_ref()
            .filter(|job| job.status == "running")
            .map(|job| job.progress);
        let status = newest.as_ref().map(|job| {
            let (words, color) = job_status(job);
            (
                words,
                color,
                shown_path(folder, job.output_path.as_deref().unwrap_or_default()),
                job.is_active(),
            )
        });
        div()
            .px(px(theme::space::S16))
            .py(px(theme::space::S12))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .when(!first, |row| row.border_t_1().border_color(theme::line()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S16))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::space::S2))
                            .child(ui::label(name, text::BODY_NAME, theme::ink()))
                            .child(ui::label(makes, text::CONTROL, theme::ink3()))
                            .child(ui::line(goes, text::MONO_SMALL, theme::ink2())),
                    )
                    .child(action),
            )
            .when_some(status, |row, (words, color, file, active)| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S8))
                        .child(
                            ui::line(
                                if active { "Now" } else { "Last" },
                                text::META,
                                theme::ink3(),
                            )
                            .flex_none(),
                        )
                        .child(
                            ui::line(file, text::MONO_SMALL, theme::ink3())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(ui::line(words, text::META, color).flex_none()),
                )
            })
            .when_some(running, |row, progress| {
                row.child(ui::progress(progress, theme::activity::gpu()))
            })
            .when_some(problem, |row, problem| {
                row.child(said(problem, theme::neg()))
            })
            .into_any_element()
    }
}

/// What a job is called: its name, else what it renders.
fn job_name(job: &RenderJob) -> String {
    if !job.name.trim().is_empty() {
        return job.name.clone();
    }
    match job.quality.as_deref().unwrap_or_default() {
        "master" => "Master",
        "review" => "Review copy",
        "full" => "Full render",
        "preview" => "Preview",
        _ => "Render",
    }
    .to_owned()
}

/// What a job's status says, and in what colour: amber while it renders.
fn job_status(job: &RenderJob) -> (String, Rgba) {
    match job.status.as_str() {
        "queued" => ("Waiting".into(), theme::ink2()),
        "running" => (
            format!("Rendering {}", percent(job.progress)),
            theme::activity::gpu(),
        ),
        "completed" => ("Done".into(), theme::pos()),
        "failed" => ("Failed".into(), theme::neg()),
        "cancelled" => ("Cancelled".into(), theme::ink3()),
        other => (other.to_owned(), theme::ink3()),
    }
}

/// `path` from the project's folder when it is inside it.
fn shown_path(folder: Option<&Path>, path: &str) -> String {
    let full = Path::new(path);
    folder
        .and_then(|folder| full.strip_prefix(folder).ok())
        .map(|inside| inside.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}
