//! QC, Finish's first section: Kerf's check of the whole programme, its
//! verdict, each check a delivery needs with what it found (Ask Kerf on one
//! that failed), and the findings; a row with a time goes to it.

use super::preflight::{ask_text, AskKerf};
use super::{empty, plural, said, sentence, working, FinishView};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, AnyElement, Context, ElementId, Rgba, SharedString};
use kerf_bridge::event::{CheckSummary, Finding, Row, Severity};
use kerf_bridge::Command;

/// The most findings listed; the rest are counted.
const FINDINGS_MOST: usize = 300;

/// The last check's report, as `check {status: done}` sent it.
pub(super) struct Report {
    /// The document revision checked.
    revision: u64,
    valid: bool,
    pub(super) summary: CheckSummary,
    findings: Vec<Finding>,
}

/// How a check of the summary shows.
trait RowShow {
    fn name(&self) -> &str;
    fn color(&self) -> Rgba;
}

impl RowShow for Row {
    fn name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.code
        } else {
            &self.name
        }
    }

    /// Passed, failed and blocking (brick), or failed and only wanting a look (ochre).
    fn color(&self) -> Rgba {
        if self.passed {
            theme::pos()
        } else if self.blocking > 0 {
            theme::neg()
        } else {
            theme::warn()
        }
    }
}

/// How a finding shows.
trait FindingShow {
    fn rank(&self) -> u8;
    fn color(&self) -> Rgba;
}

impl FindingShow for Finding {
    /// Errors block delivery; warnings want a look; information is only said.
    fn rank(&self) -> u8 {
        match self.severity {
            Severity::Error => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        }
    }

    fn color(&self) -> Rgba {
        match self.rank() {
            0 => theme::neg(),
            1 => theme::warn(),
            _ => theme::ink3(),
        }
    }
}

impl Report {
    pub(super) fn new(
        valid: bool,
        findings: &[Finding],
        summary: &CheckSummary,
        revision: u64,
    ) -> Self {
        let mut findings = findings.to_vec();
        // What blocks first, then what wants a look; each in the order found.
        findings.sort_by_key(|finding| finding.rank());
        Self {
            revision,
            valid,
            summary: summary.clone(),
            findings,
        }
    }
}

impl FinishView {
    /// Starts Kerf's QC pass, unless one runs.
    pub(super) fn run_check(&mut self, cx: &mut Context<Self>) {
        if self.finishing.read(cx).checking.is_some() {
            return;
        }
        let revision = self.session.read(cx).revision();
        let sent = self.session.update(cx, |session, _| {
            session.send(Command::Check { sequence: None })
        });
        if sent {
            self.finishing.update(cx, |finishing, cx| {
                finishing.checking = Some(0.0);
                finishing.check_revision = Some(revision);
                finishing.check_failed = None;
                cx.notify();
            });
        }
    }

    /// Goes to `at` on the programme and gives the window back, to look.
    fn seek(&mut self, at: f64, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.seek(at, false);
            cx.notify();
        });
        self.dismiss(cx);
    }

    /// The section: its title and Run QC, how a check runs or failed, and
    /// the last one's report; a failed check offers Ask Kerf where the
    /// window takes it (`can_ask`).
    pub(super) fn qc(&self, can_ask: bool, cx: &Context<Self>) -> Vec<AnyElement> {
        let finishing = self.finishing.read(cx);
        let running = finishing.checking;
        let revision = self.session.read(cx).revision();
        // Greyed, it says why: what the registry says of Run QC (kerfd
        // gone), else that a check runs.
        let reason = if running.is_some() {
            Some("A check is running".to_owned())
        } else if !self.connected(cx) {
            let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
            Some(
                editor_session::palette::unavailable(&states, "qc.run")
                    .unwrap_or("Kerf is not connected")
                    .to_owned(),
            )
        } else {
            None
        };
        let mut run = ui::button(
            "finish-run-qc",
            if finishing.report.is_some() {
                "Run QC Again"
            } else {
                "Run QC"
            },
        )
        .primary()
        .disabled(reason.is_some())
        .on_click(cx.listener(|this, _, _, cx| this.run_check(cx)));
        if let Some(reason) = reason {
            run = run.tooltip("Run QC").reason(reason);
        }
        let run = run.into_any_element();
        let mut out = vec![self.title(
            "QC",
            "The checks a delivery needs: loudness, true peak, gamut, title safe, caption overlap and timing, \
             flash, black, and a picture and its sound more than a frame out of step.",
            Some(run),
        )];
        if let Some(progress) = running {
            out.push(working("Checking…", progress));
        }
        if let Some(failed) = &finishing.check_failed {
            out.push(said(
                format!("The last check failed: {failed}"),
                theme::neg(),
            ));
        }
        let Some(report) = &finishing.report else {
            if running.is_none() && finishing.check_failed.is_none() {
                out.push(empty(
                    "Not checked yet",
                    "Run QC reads the whole programme, about as long as a draft render.",
                ));
            }
            return out;
        };
        out.push(self.qc_summary(report, revision));
        out.push(self.qc_rows(report, can_ask, cx));
        out.push(self.qc_findings(report, cx));
        out
    }

    fn qc_summary(&self, report: &Report, revision: u64) -> AnyElement {
        let summary = &report.summary;
        let (verdict, color) = if summary.blocking > 0 {
            (
                plural(summary.blocking, "1 blocks delivery", "{} block delivery"),
                theme::neg(),
            )
        } else if report.valid && summary.passed == summary.total {
            ("Ready to deliver".to_owned(), theme::pos())
        } else {
            ("Nothing blocks delivery".to_owned(), theme::pos())
        };
        let checked = format!("Checked at revision {}", report.revision);
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .child(ui::label(
                format!("{} of {} pass", summary.passed, summary.total),
                text::TITLE,
                theme::ink(),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S6))
                    .child(ui::dot(color, 8.0))
                    .child(ui::label(verdict, text::CONTROL, theme::ink2())),
            )
            .child(div().flex_1())
            .child(ui::label(checked, text::META, theme::ink3()))
            .when(report.revision != revision, |row| {
                row.child(ui::label(
                    "· the project changed since",
                    text::META,
                    theme::warn(),
                ))
            })
            .into_any_element()
    }

    /// Each check a delivery needs, a row each.
    fn qc_rows(&self, report: &Report, can_ask: bool, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .overflow_hidden()
            .children(
                report
                    .summary
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| self.qc_row(index, row, can_ask, cx)),
            )
            .into_any_element()
    }

    /// A check's row: its mark, name, what it found and what it is held to;
    /// failed, Ask Kerf (where the window takes it) and a press to its time.
    fn qc_row(
        &self,
        index: usize,
        row: &Row,
        can_ask: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let color = row.color();
        let mark = if row.passed {
            Icon::Check.view(14.0, color).into_any_element()
        } else {
            div()
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .child(ui::dot(color, 8.0))
                .into_any_element()
        };
        let seek = row.at.filter(|_| !row.passed);
        let ask = (!row.passed && can_ask).then(|| {
            // Named as the row shows it: by its code when it has no name.
            let words: SharedString = ask_text(&Row {
                name: row.name().to_owned(),
                ..row.clone()
            })
            .into();
            ui::button(ElementId::named_usize("finish-row-ask", index), "Ask Kerf")
                .small()
                .ghost()
                .tooltip(words.clone())
                .on_click(cx.listener(move |this, _, window, cx| {
                    // The row's own press (to its time) is not this one.
                    cx.stop_propagation();
                    // Put away first, so the window takes the keys
                    // back before the composer has them.
                    this.dismiss(cx);
                    window.dispatch_action(
                        Box::new(AskKerf {
                            words: words.clone(),
                        }),
                        cx,
                    );
                }))
        });
        div()
            .id(ElementId::named_usize("finish-row", index))
            .h(px(height::H32))
            .px(px(theme::space::S12))
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .when(index > 0, |line| {
                line.border_t_1().border_color(theme::line())
            })
            .child(mark)
            .child(
                ui::line(row.name().to_owned(), text::BODY, theme::ink())
                    .flex_1()
                    .min_w_0(),
            )
            .child(ui::line(row.value.clone(), text::CONTROL, theme::ink2()).flex_none())
            .when_some(row.limit.clone(), |line, limit| {
                line.child(
                    ui::line(limit, text::META, theme::ink3())
                        .flex_none()
                        .min_w(px(72.0)),
                )
            })
            .children(ask)
            .when_some(seek, |line, at| {
                line.cursor_pointer()
                    .hover(|style| style.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _, _, cx| this.seek(at, cx)))
            })
    }

    fn qc_findings(&self, report: &Report, cx: &Context<Self>) -> AnyElement {
        let count = report.findings.len();
        let hidden = count.saturating_sub(FINDINGS_MOST);
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(ui::label("Findings", text::META_STRONG, theme::ink3()))
                    .child(ui::label(count.to_string(), text::META, theme::ink3())),
            )
            .when(count == 0, |list| {
                list.child(ui::label("Nothing found", text::CONTROL, theme::ink3()))
            })
            .children(report.findings.iter().take(FINDINGS_MOST).enumerate().map(
                |(index, finding)| {
                    let at = finding.at;
                    div()
                        .id(ElementId::named_usize("finish-finding", index))
                        .px(px(theme::space::S8))
                        .py(px(theme::space::S6))
                        .flex()
                        .items_start()
                        .gap(px(theme::space::S8))
                        .rounded(px(radius::CONTROL))
                        .child(
                            div()
                                .h(px(20.0))
                                .flex()
                                .items_center()
                                .child(ui::dot(finding.color(), 7.0)),
                        )
                        .child(
                            ui::label(sentence(finding.message.clone()), text::BODY, theme::ink())
                                .flex_1()
                                .min_w_0(),
                        )
                        .when_some(
                            finding.timecode.clone().filter(|_| at.is_some()),
                            |line, timecode| {
                                line.child(
                                    ui::label(timecode, text::TIMECODE, theme::accent_ink())
                                        .flex_none()
                                        .pt(px(theme::space::S2)),
                                )
                            },
                        )
                        .when_some(at, |line, at| {
                            line.cursor_pointer()
                                .hover(|style| style.bg(theme::raised()))
                                .on_click(cx.listener(move |this, _, _, cx| this.seek(at, cx)))
                        })
                },
            ))
            .when(hidden > 0, |list| {
                list.child(ui::label(
                    plural(hidden, "and 1 more", "and {} more"),
                    text::META,
                    theme::ink3(),
                ))
            })
            .into_any_element()
    }
}
