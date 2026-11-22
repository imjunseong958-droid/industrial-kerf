//! Making and cloning: Create and Clone run as the bridge's project tasks,
//! on a thread of their own, followed until they end; and what the
//! launcher says under the form, what a task is doing or why the last
//! thing asked could not be done.

use super::plan::{folder_name, repository_name, Plan};
use super::{LauncherView, Mode};
use crate::theme::{self, text};
use crate::ui;
use gpui::{div, prelude::*, px, App, Context};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// How often a running project task is asked how it is getting on.
pub(super) const TASK_POLL: Duration = Duration::from_millis(100);

impl LauncherView {
    /// What Create asks for, as the form says it.
    fn plan(&self, cx: &App) -> Plan {
        Plan {
            name: self.name.read(cx).text().trim().to_owned(),
            location: self.location(cx),
            template: self.format.wire().to_owned(),
            rate: self.rate.to_owned(),
            git: self.git,
            github: self.github,
            remote: if self.github {
                String::new()
            } else {
                self.remote.read(cx).text().trim().to_owned()
            },
            kerfd: self.kerfd.clone(),
            preset: (self.mode == Some(Mode::Preset))
                .then(|| self.preset.request(cx))
                .flatten(),
        }
    }

    /// Create Project: the new project made, from the preset chosen on From
    /// Preset, once the form can.
    pub(super) fn create(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.form_problem(cx).is_some() {
            return;
        }
        let plan = self.plan(cx);
        let folder = folder_name(&plan.name);
        let message = if plan.preset.is_some() {
            format!("Making {folder} from the preset…")
        } else {
            format!("Making {folder}…")
        };
        self.remember(self.mode);
        self.run(plan.request(), message, true, cx);
    }

    /// Clone: the repository cloned, once the form can.
    pub(super) fn clone_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.form_problem(cx).is_some() {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let folder = repository_name(&url).unwrap_or_default();
        let request =
            json!({"type": "clone", "url": url, "location": self.location(cx).to_string_lossy()});
        self.remember(Some(Mode::Clone));
        self.run(request, format!("Cloning {folder}…"), false, cx);
    }

    /// Starts a project task and follows it until it ends; `made` when it
    /// makes a project from nothing.
    fn run(&mut self, request: Value, message: String, made: bool, cx: &mut Context<Self>) {
        self.busy = Some(message.into());
        self.problem = None;
        let task = kerf_bridge::Task::start(request);
        self._task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TASK_POLL).await;
            let status = task.status();
            let running = status["status"] == "running";
            let followed = this.update(cx, |this, cx| this.task_status(&status, made, cx));
            if followed.is_err() || !running {
                break;
            }
        }));
        cx.notify();
    }

    /// How a running task stands: its words while it runs; done, its
    /// project opens; failed, why.
    fn task_status(&mut self, status: &Value, made: bool, cx: &mut Context<Self>) {
        match status["status"].as_str() {
            Some("running") => {
                if let Some(message) = status["message"]
                    .as_str()
                    .filter(|message| !message.is_empty())
                {
                    self.busy = Some(message.to_owned().into());
                }
            }
            Some("done") => {
                self.busy = None;
                // Said once the project is open: this window closes at once.
                let warnings: Vec<String> = status["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                match status["path"].as_str().filter(|path| !path.is_empty()) {
                    Some(path) => {
                        self.open(PathBuf::from(path), made.then_some(true), warnings, cx)
                    }
                    None => {
                        self.problem = Some("The task finished without a project folder".into())
                    }
                }
            }
            _ => {
                self.busy = None;
                let error = status["error"]
                    .as_str()
                    .unwrap_or("Kerf could not make the project");
                self.problem = Some(error.to_owned().into());
            }
        }
        cx.notify();
    }

    /// Under the form: what a running task is doing, else why the last
    /// thing asked could not be done.
    pub(super) fn status(&self) -> Option<impl IntoElement> {
        if let Some(busy) = &self.busy {
            let dot = ui::dot(theme::activity::gpu(), 6.0);
            return Some(
                div()
                    .mt(px(theme::space::S12))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(dot)
                    .child(
                        ui::label(busy.clone(), text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    ),
            );
        }
        self.problem.as_ref().map(|problem| {
            div()
                .mt(px(theme::space::S12))
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(ui::dot(theme::neg(), 6.0))
                .child(
                    ui::label(problem.clone(), text::CONTROL, theme::neg())
                        .flex_1()
                        .min_w_0(),
                )
        })
    }
}
